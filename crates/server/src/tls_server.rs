// Copyright © 2026 Kirky.X
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! TLS 服务器模块
//! 提供 HTTPS 和 gRPC TLS 支持

use nebula_core::config::TlsConfig;
use rustls::pki_types::PrivateKeyDer;
use rustls::ServerConfig;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::sync::Arc;
use thiserror::Error;
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
    config: TlsConfig,
    http_acceptor: Option<TlsAcceptor>,
    grpc_tls_config: Option<Arc<ServerTlsConfig>>,
}

impl TlsManager {
    pub fn new(config: TlsConfig) -> Self {
        Self {
            config,
            http_acceptor: None,
            grpc_tls_config: None,
        }
    }

    pub fn is_http_enabled(&self) -> bool {
        self.config.http_enabled && self.http_acceptor.is_some()
    }

    pub fn is_grpc_enabled(&self) -> bool {
        self.config.grpc_enabled && self.grpc_tls_config.is_some()
    }

    pub fn http_acceptor(&self) -> Option<&TlsAcceptor> {
        self.http_acceptor.as_ref()
    }

    pub fn grpc_tls_config(&self) -> Option<&Arc<ServerTlsConfig>> {
        self.grpc_tls_config.as_ref()
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

        // 读取 PEM 文件
        let cert_file =
            File::open(cert_path).map_err(|e| TlsError::CertificateLoadError(e.to_string()))?;
        let key_file =
            File::open(key_path).map_err(|e| TlsError::PrivateKeyLoadError(e.to_string()))?;

        let mut cert_reader = BufReader::new(cert_file);
        let mut key_reader = BufReader::new(key_file);

        // 读取证书 - rustls-pemfile 2.x API
        let mut cert_chain = Vec::new();
        loop {
            match rustls_pemfile::read_one(&mut cert_reader) {
                Ok(Some(rustls_pemfile::Item::X509Certificate(cert))) => {
                    cert_chain.push(cert);
                    break; // 只取第一个证书
                }
                Ok(Some(_)) => continue, // 跳过非证书项
                Ok(None) => break,
                Err(e) => return Err(TlsError::CertificateLoadError(e.to_string())),
            }
        }

        let cert_der = cert_chain
            .into_iter()
            .next()
            .ok_or_else(|| TlsError::CertificateLoadError("Empty certificate chain".to_string()))?;

        // 读取密钥 - rustls-pemfile 2.x API
        let mut private_key_der: Option<PrivateKeyDer<'static>> = None;
        loop {
            match rustls_pemfile::read_one(&mut key_reader) {
                Ok(Some(rustls_pemfile::Item::Pkcs1Key(key))) => {
                    private_key_der = Some(PrivateKeyDer::from(key));
                    break;
                }
                Ok(Some(rustls_pemfile::Item::Pkcs8Key(key))) => {
                    private_key_der = Some(PrivateKeyDer::from(key));
                    break;
                }
                Ok(Some(rustls_pemfile::Item::Sec1Key(key))) => {
                    private_key_der = Some(PrivateKeyDer::from(key));
                    break;
                }
                Ok(Some(_)) => continue, // 跳过非密钥项
                Ok(None) => break,
                Err(e) => return Err(TlsError::PrivateKeyLoadError(e.to_string())),
            }
        }

        let private_key_der = private_key_der
            .ok_or_else(|| TlsError::PrivateKeyLoadError("Empty private key".to_string()))?;

        // 获取 PEM 字节用于 gRPC Identity
        let cert_pem = cert_der.as_ref();
        let key_pem: &[u8] = match &private_key_der {
            PrivateKeyDer::Pkcs1(k) => k.secret_pkcs1_der(),
            PrivateKeyDer::Pkcs8(k) => k.secret_pkcs8_der(),
            PrivateKeyDer::Sec1(k) => k.secret_sec1_der(),
            _ => unreachable!(),
        };

        // 创建 Identity 用于 gRPC
        let identity = Identity::from_pem(cert_pem, key_pem);

        // 为 HTTP 配置 TLS
        if self.config.http_enabled {
            let rustls_config = ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(vec![cert_der], private_key_der)
                .map_err(|e| TlsError::InvalidConfig(e.to_string()))?;

            // 配置 ALPN 协议 (HTTP/2 支持)
            let mut config_with_alpn = rustls_config;
            if !self.config.alpn_protocols.is_empty() {
                let alpn_protocols: Vec<Vec<u8>> = self
                    .config
                    .alpn_protocols
                    .iter()
                    .map(|s| s.as_bytes().to_vec())
                    .collect();
                config_with_alpn.alpn_protocols = alpn_protocols;
            }
            self.http_acceptor = Some(TlsAcceptor::from(Arc::new(config_with_alpn)));
        }

        // 为 gRPC 配置 TLS
        if self.config.grpc_enabled {
            let mut grpc_config = ServerTlsConfig::new().identity(identity);

            if let Some(ref ca_path) = self.config.ca_path {
                let ca_file = File::open(ca_path)
                    .map_err(|e| TlsError::CertificateLoadError(e.to_string()))?;
                let mut ca_reader = BufReader::new(ca_file);

                // 读取 CA 证书
                let mut ca_certs = Vec::new();
                loop {
                    match rustls_pemfile::read_one(&mut ca_reader) {
                        Ok(Some(rustls_pemfile::Item::X509Certificate(cert))) => {
                            ca_certs.push(cert);
                            break;
                        }
                        Ok(Some(_)) => continue,
                        Ok(None) => break,
                        Err(e) => return Err(TlsError::CertificateLoadError(e.to_string())),
                    }
                }

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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tls_config_default() {
        let config = TlsConfig::default();
        assert!(!config.enabled);
        assert!(config.cert_path.is_empty());
        assert!(config.key_path.is_empty());
        assert!(config.ca_path.is_none());
        assert!(!config.http_enabled);
        assert!(!config.grpc_enabled);
        assert_eq!(
            config.min_tls_version,
            nebula_core::config::TlsVersion::Tls13
        );
        assert!(!config.alpn_protocols.is_empty());
    }
}
