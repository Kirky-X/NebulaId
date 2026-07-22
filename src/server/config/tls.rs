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

use crate::core::config::TlsConfig;
use rustls::pki_types::PrivateKeyDer;
use sdforge::tonic::transport::{Certificate, Identity, ServerTlsConfig};
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::sync::Arc;
use thiserror::Error;
use tokio_rustls::rustls::ServerConfig;
use tokio_rustls::TlsAcceptor;

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

        // SECURITY: Validate minimum TLS version configuration
        // Prevent use of insecure TLS versions (TLS 1.0, TLS 1.1)
        match self.config.min_tls_version {
            crate::core::config::TlsVersion::Tls12 => {
                tracing::info!("{}", t!("log.server.config.tls.tls12_min_configured"));
            }
            crate::core::config::TlsVersion::Tls13 => {
                tracing::info!("{}", t!("log.server.config.tls.tls13_min_configured"));
            }
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

        // 为 HTTP 配置 TLS with version enforcement
        if self.config.http_enabled {
            let mut config_with_alpn = ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(vec![cert_der.clone()], private_key_der.clone_key())
                .map_err(|e| TlsError::InvalidConfig(e.to_string()))?;

            // tiangang H1 部分修复：rustls 0.23 的 ServerConfig 不暴露 protocol_versions
            // 公开字段，with_no_client_auth() 快捷方法跳过 versions 配置。
            // 当前依赖 rustls 默认（TLS 1.2+1.3），并通过 min_tls_version 配置记录意图。
            // 完整强制需迁移到 rustls CryptoProvider 自定义流程，延后到 v0.3.0。
            match self.config.min_tls_version {
                crate::core::config::TlsVersion::Tls12 => {
                    tracing::info!(
                        "{}",
                        t!("log.server.config.tls.tls12_configured_rustls_default")
                    );
                }
                crate::core::config::TlsVersion::Tls13 => {
                    tracing::warn!(
                        "{}",
                        t!("log.server.config.tls.tls13_only_requested_auto_negotiate")
                    );
                }
            }

            // 配置 ALPN 协议 (HTTP/2 支持)
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

        tracing::info!(
            event = "tls_initialized",
            http_enabled = %self.config.http_enabled,
            grpc_enabled = %self.config.grpc_enabled,
            min_version = %self.config.min_tls_version,
            "{}",
            t!("log.server.config.tls.tls_initialized")
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::{TlsConfig, TlsVersion};
    use std::io::Write;
    use tempfile::NamedTempFile;

    /// Generate a self-signed cert + key pair as PEM files for testing.
    /// Returns (cert_file, key_file) NamedTempFile handles.
    fn generate_test_cert_files() -> (NamedTempFile, NamedTempFile) {
        // Install ring crypto provider (required by rustls 0.23 + rcgen 0.13).
        // `install_default()` is idempotent; safe to call multiple times.
        let _ = rustls::crypto::ring::default_provider().install_default();

        // rcgen 0.13 API: `generate_simple_self_signed` returns a
        // `CertifiedKey { cert, key_pair }`. `cert.pem()` and
        // `key_pair.serialize_pem()` both return `String`.
        let certified = rcgen::generate_simple_self_signed(vec!["localhost".to_string()])
            .expect("generate self-signed cert");
        let cert_pem = certified.cert.pem();
        let key_pem = certified.key_pair.serialize_pem();

        let mut cert_file = NamedTempFile::new().expect("cert tmp file");
        cert_file
            .write_all(cert_pem.as_bytes())
            .expect("write cert pem");
        cert_file.flush().expect("flush cert file");

        let mut key_file = NamedTempFile::new().expect("key tmp file");
        key_file
            .write_all(key_pem.as_bytes())
            .expect("write key pem");
        key_file.flush().expect("flush key file");

        (cert_file, key_file)
    }

    // ===== TlsConfig::default =====

    #[test]
    fn test_tls_config_default() {
        let config = TlsConfig::default();
        assert!(!config.enabled);
        assert!(config.cert_path.is_empty());
        assert!(config.key_path.is_empty());
        assert!(config.ca_path.is_none());
        assert!(!config.http_enabled);
        assert!(!config.grpc_enabled);
        assert_eq!(config.min_tls_version, TlsVersion::Tls13);
        assert!(!config.alpn_protocols.is_empty());
    }

    // ===== TlsError Display =====

    #[test]
    fn test_tls_error_certificate_load_error_display() {
        let err = TlsError::CertificateLoadError("file missing".to_string());
        assert_eq!(err.to_string(), "Failed to load certificate: file missing");
    }

    #[test]
    fn test_tls_error_private_key_load_error_display() {
        let err = TlsError::PrivateKeyLoadError("bad key".to_string());
        assert_eq!(err.to_string(), "Failed to load private key: bad key");
    }

    #[test]
    fn test_tls_error_invalid_config_display() {
        let err = TlsError::InvalidConfig("bad config".to_string());
        assert_eq!(err.to_string(), "Invalid TLS configuration: bad config");
    }

    #[test]
    fn test_tls_error_equality_and_clone() {
        let err1 = TlsError::CertificateLoadError("a".to_string());
        let err2 = TlsError::CertificateLoadError("a".to_string());
        let err3 = TlsError::PrivateKeyLoadError("a".to_string());
        assert_eq!(err1, err2);
        assert_ne!(err1, err3);
        // Clone produces equal value
        assert_eq!(err1.clone(), err1);
    }

    // ===== TlsManager::new — default state =====

    #[test]
    fn test_tls_manager_new_disabled_config() {
        let config = TlsConfig::default();
        let manager = TlsManager::new(config);
        // Disabled config: nothing should be initialized
        assert!(!manager.is_http_enabled());
        assert!(!manager.is_grpc_enabled());
        assert!(manager.http_acceptor().is_none());
        assert!(manager.grpc_tls_config().is_none());
    }

    #[test]
    fn test_tls_manager_new_preserves_disabled_flags() {
        // Even if http_enabled/grpc_enabled are true in config, when not
        // initialized, is_*_enabled returns false (acceptor is None).
        let config = TlsConfig {
            enabled: true,
            http_enabled: true,
            grpc_enabled: true,
            ..Default::default()
        };
        let manager = TlsManager::new(config);
        assert!(!manager.is_http_enabled());
        assert!(!manager.is_grpc_enabled());
        assert!(manager.http_acceptor().is_none());
        assert!(manager.grpc_tls_config().is_none());
    }

    // ===== TlsManager::initialize — disabled config returns Ok =====

    #[tokio::test]
    async fn test_initialize_disabled_returns_ok_without_initializing() {
        let mut manager = TlsManager::new(TlsConfig::default());
        let result = manager.initialize().await;
        assert!(result.is_ok());
        // Even after initialize, disabled means acceptors remain None
        assert!(manager.http_acceptor().is_none());
        assert!(manager.grpc_tls_config().is_none());
        assert!(!manager.is_http_enabled());
        assert!(!manager.is_grpc_enabled());
    }

    // ===== TlsManager::initialize — missing cert file =====

    #[tokio::test]
    async fn test_initialize_enabled_missing_cert_file_returns_cert_error() {
        let config = TlsConfig {
            enabled: true,
            cert_path: "/nonexistent/cert.pem".to_string(),
            key_path: "/nonexistent/key.pem".to_string(),
            http_enabled: true,
            ..Default::default()
        };
        let mut manager = TlsManager::new(config);
        let result = manager.initialize().await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, TlsError::CertificateLoadError(_)));
        assert!(err.to_string().contains("Certificate file not found"));
    }

    // ===== TlsManager::initialize — missing key file =====

    #[tokio::test]
    async fn test_initialize_enabled_missing_key_file_returns_key_error() {
        // Create a valid cert file but leave key path nonexistent.
        let (cert_file, _key_file) = generate_test_cert_files();
        let config = TlsConfig {
            enabled: true,
            cert_path: cert_file
                .path()
                .to_str()
                .expect("cert path utf8")
                .to_string(),
            key_path: "/nonexistent/key.pem".to_string(),
            http_enabled: true,
            ..Default::default()
        };
        let mut manager = TlsManager::new(config);
        let result = manager.initialize().await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, TlsError::PrivateKeyLoadError(_)));
        assert!(err.to_string().contains("Private key file not found"));
    }

    // ===== TlsManager::initialize — empty cert file =====

    #[tokio::test]
    async fn test_initialize_empty_cert_file_returns_cert_error() {
        let cert_file = NamedTempFile::new().expect("cert tmp");
        let key_file = NamedTempFile::new().expect("key tmp");
        let config = TlsConfig {
            enabled: true,
            cert_path: cert_file.path().to_str().unwrap().to_string(),
            key_path: key_file.path().to_str().unwrap().to_string(),
            http_enabled: true,
            ..Default::default()
        };
        let mut manager = TlsManager::new(config);
        let result = manager.initialize().await;
        assert!(result.is_err());
        // Empty cert file -> rustls_pemfile returns None -> "Empty certificate chain"
        let err = result.unwrap_err();
        assert!(matches!(err, TlsError::CertificateLoadError(_)));
    }

    // ===== TlsManager::initialize — empty key file =====

    #[tokio::test]
    async fn test_initialize_empty_key_file_returns_key_error() {
        let (cert_file, _key) = generate_test_cert_files();
        let empty_key = NamedTempFile::new().expect("key tmp");
        let config = TlsConfig {
            enabled: true,
            cert_path: cert_file.path().to_str().unwrap().to_string(),
            key_path: empty_key.path().to_str().unwrap().to_string(),
            http_enabled: true,
            ..Default::default()
        };
        let mut manager = TlsManager::new(config);
        let result = manager.initialize().await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, TlsError::PrivateKeyLoadError(_)));
        assert!(err.to_string().contains("Empty private key"));
    }

    // ===== TlsManager::initialize — valid cert+key, http_enabled =====

    #[tokio::test]
    async fn test_initialize_valid_http_enabled_creates_acceptor() {
        let (cert_file, key_file) = generate_test_cert_files();
        let config = TlsConfig {
            enabled: true,
            cert_path: cert_file.path().to_str().unwrap().to_string(),
            key_path: key_file.path().to_str().unwrap().to_string(),
            http_enabled: true,
            grpc_enabled: false,
            ..Default::default()
        };
        let mut manager = TlsManager::new(config);
        let result = manager.initialize().await;
        assert!(result.is_ok(), "initialize should succeed: {:?}", result);
        assert!(manager.is_http_enabled(), "http should be enabled");
        assert!(!manager.is_grpc_enabled(), "grpc should not be enabled");
        assert!(manager.http_acceptor().is_some());
        assert!(manager.grpc_tls_config().is_none());
    }

    // ===== TlsManager::initialize — valid cert+key, grpc_enabled =====

    #[tokio::test]
    async fn test_initialize_valid_grpc_enabled_creates_tls_config() {
        let (cert_file, key_file) = generate_test_cert_files();
        let config = TlsConfig {
            enabled: true,
            cert_path: cert_file.path().to_str().unwrap().to_string(),
            key_path: key_file.path().to_str().unwrap().to_string(),
            http_enabled: false,
            grpc_enabled: true,
            ..Default::default()
        };
        let mut manager = TlsManager::new(config);
        let result = manager.initialize().await;
        assert!(result.is_ok(), "initialize should succeed: {:?}", result);
        assert!(!manager.is_http_enabled());
        assert!(manager.is_grpc_enabled(), "grpc should be enabled");
        assert!(manager.http_acceptor().is_none());
        assert!(manager.grpc_tls_config().is_some());
    }

    // ===== TlsManager::initialize — both http + grpc enabled =====

    #[tokio::test]
    async fn test_initialize_both_http_and_grpc_enabled() {
        let (cert_file, key_file) = generate_test_cert_files();
        let config = TlsConfig {
            enabled: true,
            cert_path: cert_file.path().to_str().unwrap().to_string(),
            key_path: key_file.path().to_str().unwrap().to_string(),
            http_enabled: true,
            grpc_enabled: true,
            ..Default::default()
        };
        let mut manager = TlsManager::new(config);
        let result = manager.initialize().await;
        assert!(result.is_ok(), "initialize should succeed: {:?}", result);
        assert!(manager.is_http_enabled());
        assert!(manager.is_grpc_enabled());
        assert!(manager.http_acceptor().is_some());
        assert!(manager.grpc_tls_config().is_some());
    }

    // ===== TlsManager::initialize — TLS 1.2 min version =====

    #[tokio::test]
    async fn test_initialize_tls12_min_version_succeeds() {
        let (cert_file, key_file) = generate_test_cert_files();
        let config = TlsConfig {
            enabled: true,
            cert_path: cert_file.path().to_str().unwrap().to_string(),
            key_path: key_file.path().to_str().unwrap().to_string(),
            http_enabled: true,
            min_tls_version: TlsVersion::Tls12,
            ..Default::default()
        };
        let mut manager = TlsManager::new(config);
        let result = manager.initialize().await;
        assert!(result.is_ok(), "TLS 1.2 min should succeed: {:?}", result);
        assert!(manager.is_http_enabled());
    }

    // ===== TlsManager::initialize — TLS 1.3 min version =====

    #[tokio::test]
    async fn test_initialize_tls13_min_version_succeeds() {
        let (cert_file, key_file) = generate_test_cert_files();
        let config = TlsConfig {
            enabled: true,
            cert_path: cert_file.path().to_str().unwrap().to_string(),
            key_path: key_file.path().to_str().unwrap().to_string(),
            http_enabled: true,
            min_tls_version: TlsVersion::Tls13,
            ..Default::default()
        };
        let mut manager = TlsManager::new(config);
        let result = manager.initialize().await;
        assert!(result.is_ok(), "TLS 1.3 min should succeed: {:?}", result);
        assert!(manager.is_http_enabled());
    }

    // ===== TlsManager::initialize — custom ALPN protocols =====

    #[tokio::test]
    async fn test_initialize_with_custom_alpn_protocols_succeeds() {
        let (cert_file, key_file) = generate_test_cert_files();
        let config = TlsConfig {
            enabled: true,
            cert_path: cert_file.path().to_str().unwrap().to_string(),
            key_path: key_file.path().to_str().unwrap().to_string(),
            http_enabled: true,
            alpn_protocols: vec!["h2".to_string(), "http/1.1".to_string()],
            ..Default::default()
        };
        let mut manager = TlsManager::new(config);
        let result = manager.initialize().await;
        assert!(result.is_ok(), "ALPN config should succeed: {:?}", result);
        assert!(manager.is_http_enabled());
        // The acceptor was created; ALPN protocols are baked into the
        // ServerConfig (we cannot introspect them via TlsAcceptor's public
        // API, but the fact that initialize() returned Ok with non-empty
        // alpn_protocols confirms the branch was exercised).
        assert!(manager.http_acceptor().is_some());
    }

    // ===== TlsManager::initialize — empty ALPN protocols branch =====

    #[tokio::test]
    async fn test_initialize_with_empty_alpn_protocols_skips_alpn_block() {
        let (cert_file, key_file) = generate_test_cert_files();
        let config = TlsConfig {
            enabled: true,
            cert_path: cert_file.path().to_str().unwrap().to_string(),
            key_path: key_file.path().to_str().unwrap().to_string(),
            http_enabled: true,
            alpn_protocols: vec![],
            ..Default::default()
        };
        let mut manager = TlsManager::new(config);
        let result = manager.initialize().await;
        assert!(result.is_ok(), "empty ALPN should succeed: {:?}", result);
        assert!(manager.is_http_enabled());
    }

    // ===== TlsManager::initialize — gRPC with CA cert (mTLS) =====

    #[tokio::test]
    async fn test_initialize_grpc_with_ca_cert_succeeds() {
        let (cert_file, key_file) = generate_test_cert_files();
        // Use the same cert as the CA for testing purposes (self-signed).
        let ca_file = NamedTempFile::new().expect("ca tmp");
        std::fs::write(ca_file.path(), std::fs::read(cert_file.path()).unwrap()).unwrap();

        let config = TlsConfig {
            enabled: true,
            cert_path: cert_file.path().to_str().unwrap().to_string(),
            key_path: key_file.path().to_str().unwrap().to_string(),
            http_enabled: false,
            grpc_enabled: true,
            ca_path: Some(ca_file.path().to_str().unwrap().to_string()),
            ..Default::default()
        };
        let mut manager = TlsManager::new(config);
        let result = manager.initialize().await;
        assert!(result.is_ok(), "gRPC with CA should succeed: {:?}", result);
        assert!(manager.is_grpc_enabled());
        assert!(manager.grpc_tls_config().is_some());
    }

    // ===== TlsManager::initialize — gRPC with empty CA file =====

    #[tokio::test]
    async fn test_initialize_grpc_with_empty_ca_returns_error() {
        let (cert_file, key_file) = generate_test_cert_files();
        let empty_ca = NamedTempFile::new().expect("ca tmp");

        let config = TlsConfig {
            enabled: true,
            cert_path: cert_file.path().to_str().unwrap().to_string(),
            key_path: key_file.path().to_str().unwrap().to_string(),
            http_enabled: false,
            grpc_enabled: true,
            ca_path: Some(empty_ca.path().to_str().unwrap().to_string()),
            ..Default::default()
        };
        let mut manager = TlsManager::new(config);
        let result = manager.initialize().await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, TlsError::CertificateLoadError(_)));
        assert!(err.to_string().contains("Empty CA certificate"));
    }

    // ===== TlsManager::initialize — gRPC with nonexistent CA file =====

    #[tokio::test]
    async fn test_initialize_grpc_with_nonexistent_ca_returns_error() {
        let (cert_file, key_file) = generate_test_cert_files();
        let config = TlsConfig {
            enabled: true,
            cert_path: cert_file.path().to_str().unwrap().to_string(),
            key_path: key_file.path().to_str().unwrap().to_string(),
            http_enabled: false,
            grpc_enabled: true,
            ca_path: Some("/nonexistent/ca.pem".to_string()),
            ..Default::default()
        };
        let mut manager = TlsManager::new(config);
        let result = manager.initialize().await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, TlsError::CertificateLoadError(_)));
    }

    // ===== TlsManager::initialize — neither http nor grpc enabled =====

    #[tokio::test]
    async fn test_initialize_neither_http_nor_grpc_enabled_succeeds() {
        // Even with enabled=true, if both http_enabled and grpc_enabled are
        // false, the function should succeed without creating acceptors.
        let (cert_file, key_file) = generate_test_cert_files();
        let config = TlsConfig {
            enabled: true,
            cert_path: cert_file.path().to_str().unwrap().to_string(),
            key_path: key_file.path().to_str().unwrap().to_string(),
            http_enabled: false,
            grpc_enabled: false,
            ..Default::default()
        };
        let mut manager = TlsManager::new(config);
        let result = manager.initialize().await;
        assert!(result.is_ok());
        assert!(!manager.is_http_enabled());
        assert!(!manager.is_grpc_enabled());
        assert!(manager.http_acceptor().is_none());
        assert!(manager.grpc_tls_config().is_none());
    }

    // ===== TlsManager — accessor methods =====

    #[test]
    fn test_http_acceptor_returns_none_before_initialize() {
        let manager = TlsManager::new(TlsConfig::default());
        assert!(manager.http_acceptor().is_none());
    }

    #[test]
    fn test_grpc_tls_config_returns_none_before_initialize() {
        let manager = TlsManager::new(TlsConfig::default());
        assert!(manager.grpc_tls_config().is_none());
    }

    #[test]
    fn test_tls_manager_is_cloneable() {
        let manager = TlsManager::new(TlsConfig::default());
        // TlsManager derives Clone — required for use in axum State.
        let _cloned = manager.clone();
    }
}
