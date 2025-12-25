use rustls::ServerConfig;
use rustls_pemfile::{certs, pkcs8_private_keys};
use std::fs::File;
use std::io::BufReader;
use std::net::SocketAddr;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;
use tonic::transport::{Certificate, Identity, ServerTlsConfig};

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum TlsError {
    #[error("Failed to load certificate: {}", _0)]
    CertificateLoadError(String),

    #[error("Failed to load private key: {}", _0)]
    PrivateKeyLoadError(String),

    #[error("Invalid TLS configuration: {}", _0)]
    InvalidConfig(String),
}

pub type TlsResult<T> = std::result::Result<T, TlsError>;

#[derive(Clone)]
pub struct TlsManager {
    config: nebula_core::config::TlsConfig,
    http_acceptor: Option<TlsAcceptor>,
    grpc_tls_config: Option<Arc<ServerTlsConfig>>,
}

impl TlsManager {
    pub fn new(config: nebula_core::config::TlsConfig) -> Self {
        Self {
            config,
            http_acceptor: None,
            grpc_tls_config: None,
        }
    }

    pub async fn initialize(&mut self) -> TlsResult<()> {
        if !self.config.enabled {
            return Ok(());
        }

        let cert_path = Path::new(&self.config.cert_path);
        let key_path = Path::new(&self.config.key_path);

        if !cert_path.exists() {
            return Err(TlsError::CertificateLoadError(format!(
                "Certificate file not found: {:?}",
                cert_path
            )));
        }

        if !key_path.exists() {
            return Err(TlsError::PrivateKeyLoadError(format!(
                "Private key file not found: {:?}",
                key_path
            )));
        }

        let cert_file =
            File::open(cert_path).map_err(|e| TlsError::CertificateLoadError(e.to_string()))?;
        let key_file =
            File::open(key_path).map_err(|e| TlsError::PrivateKeyLoadError(e.to_string()))?;

        let mut cert_reader = BufReader::new(cert_file);
        let mut key_reader = BufReader::new(key_file);

        let cert_chain_result = certs(&mut cert_reader);
        let cert_chain: Vec<_> = cert_chain_result
            .collect::<Result<_, _>>()
            .map_err(|e| TlsError::CertificateLoadError(e.to_string()))?;
        let cert_chain = cert_chain
            .into_iter()
            .next()
            .ok_or_else(|| TlsError::CertificateLoadError("Empty certificate chain".to_string()))?;

        let pkcs8_key_result = pkcs8_private_keys(&mut key_reader);
        let pkcs8_keys: Vec<_> = pkcs8_key_result
            .collect::<Result<_, _>>()
            .map_err(|e| TlsError::PrivateKeyLoadError(e.to_string()))?;
        let pkcs8_key = pkcs8_keys
            .into_iter()
            .next()
            .ok_or_else(|| TlsError::PrivateKeyLoadError("Empty private key".to_string()))?;

        let private_key = rustls::pki_types::PrivateKeyDer::Pkcs8(pkcs8_key);

        let private_key_ref: &[u8] = private_key.secret_der();
        let identity = Identity::from_pem(cert_chain.as_ref(), private_key_ref);

        if self.config.http_enabled {
            let rustls_config = ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(vec![cert_chain], private_key)
                .map_err(|e| TlsError::InvalidConfig(e.to_string()))?;

            self.http_acceptor = Some(TlsAcceptor::from(Arc::new(rustls_config)));
        }

        if self.config.grpc_enabled {
            let mut grpc_config = ServerTlsConfig::new().identity(identity.clone());

            if let Some(ref ca_path) = self.config.ca_path {
                let ca_file = File::open(ca_path)
                    .map_err(|e| TlsError::CertificateLoadError(e.to_string()))?;
                let mut ca_reader = BufReader::new(ca_file);
                let ca_cert_result = certs(&mut ca_reader);
                let ca_certs: Vec<_> = ca_cert_result
                    .collect::<Result<_, _>>()
                    .map_err(|e| TlsError::CertificateLoadError(e.to_string()))?;
                let ca_cert = ca_certs.into_iter().next().ok_or_else(|| {
                    TlsError::CertificateLoadError("Empty CA certificate".to_string())
                })?;

                let ca_cert_bytes = ca_cert.as_ref().to_vec();
                let ca_cert_tonic = Certificate::from_pem(ca_cert_bytes);
                grpc_config = grpc_config.client_ca_root(ca_cert_tonic);
            }

            self.grpc_tls_config = Some(Arc::new(grpc_config));
        }

        Ok(())
    }

    pub fn is_http_enabled(&self) -> bool {
        self.config.enabled && self.config.http_enabled && self.http_acceptor.is_some()
    }

    pub fn is_grpc_enabled(&self) -> bool {
        self.config.enabled && self.config.grpc_enabled && self.grpc_tls_config.is_some()
    }

    pub fn http_acceptor(&self) -> Option<&TlsAcceptor> {
        self.http_acceptor.as_ref()
    }

    pub fn grpc_tls_config(&self) -> Option<Arc<ServerTlsConfig>> {
        self.grpc_tls_config.clone()
    }

    pub fn config(&self) -> &nebula_core::config::TlsConfig {
        &self.config
    }
}

pub struct TlsIncoming {
    listener: TcpListener,
    acceptor: TlsAcceptor,
}

impl TlsIncoming {
    pub fn new(listener: TcpListener, acceptor: TlsAcceptor) -> Self {
        Self { listener, acceptor }
    }

    pub async fn accept(
        &mut self,
    ) -> std::io::Result<(tokio_rustls::server::TlsStream<TcpStream>, SocketAddr)> {
        let (stream, addr) = self.listener.accept().await?;
        let tls_stream = self.acceptor.accept(stream).await.map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("TLS handshake failed: {}", e),
            )
        })?;
        Ok((tls_stream, addr))
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.listener
            .local_addr()
            .unwrap_or_else(|_| SocketAddr::from(([0, 0, 0, 0], 0)))
    }

    pub fn into_inner(self) -> (TcpListener, TlsAcceptor) {
        (self.listener, self.acceptor)
    }
}

pub struct TlsStream(pub tokio_rustls::server::TlsStream<TcpStream>);

impl TlsStream {
    pub fn inner_mut(&mut self) -> &mut tokio_rustls::server::TlsStream<TcpStream> {
        &mut self.0
    }
}

impl AsyncRead for TlsStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.0).poll_read(cx, buf)
    }
}

impl AsyncWrite for TlsStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.0).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.0).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.0).poll_shutdown(cx)
    }
}

impl std::fmt::Display for TlsIncoming {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TlsIncoming({})", self.local_addr())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tls_config_default() {
        let config = nebula_core::config::TlsConfig::default();
        assert!(!config.enabled);
        assert!(config.cert_path.is_empty());
        assert!(config.key_path.is_empty());
        assert!(config.ca_path.is_none());
        assert!(!config.http_enabled);
        assert!(!config.grpc_enabled);
    }
}
