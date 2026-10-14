// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! TLS 服务器模块：提供 HTTPS 与 gRPC TLS 支持。

use crate::core::config::TlsConfig;
use axum::extract::connect_info::Connected;
use axum::serve::IncomingStream;
use rustls::pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer};
use rustls::SupportedProtocolVersion;
use sdforge::tonic::transport::{Certificate, Identity, ServerTlsConfig};
use std::fs::File;
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

/// `min_tls_version = tls13`：仅允许 TLS 1.3。
const PROTOCOL_VERSIONS_TLS13: &[&SupportedProtocolVersion] = &[&rustls::version::TLS13];

/// `min_tls_version = tls12`：TLS 1.3 优先，TLS 1.2 作为下限兼容。
const PROTOCOL_VERSIONS_TLS12_PLUS: &[&SupportedProtocolVersion] =
    &[&rustls::version::TLS13, &rustls::version::TLS12];

/// 把 `tls.min_tls_version` 映射为 rustls 允许协商的协议版本集合。
///
/// 非法取值在配置反序列化阶段被 serde 拒绝（闭合枚举）；穷尽 match 保证
/// 新增变体漏登记即编译失败，不会出现「配置写了但不生效」。
fn enabled_protocol_versions(
    min: crate::core::config::TlsVersion,
) -> &'static [&'static SupportedProtocolVersion] {
    match min {
        crate::core::config::TlsVersion::Tls12 => PROTOCOL_VERSIONS_TLS12_PLUS,
        crate::core::config::TlsVersion::Tls13 => PROTOCOL_VERSIONS_TLS13,
    }
}

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
            // 矛盾配置显性上报：enabled=false 时整体关闭 TLS，per-port 开关被忽略，
            // 端口按明文启动。
            if self.config.http_enabled || self.config.grpc_enabled {
                tracing::warn!(
                    event = "tls_config_conflict",
                    tls_enabled = false,
                    http_enabled = self.config.http_enabled,
                    grpc_enabled = self.config.grpc_enabled,
                    "tls.enabled = false while tls.http_enabled/tls.grpc_enabled = true: \
                     the per-port flags are ignored and HTTP/gRPC will serve PLAINTEXT; \
                     set tls.enabled = true with a valid cert_path/key_path to encrypt"
                );
            }
            return Ok(());
        }

        // 记录生效的最低 TLS 版本；非法值在配置反序列化阶段已被 serde 拒绝。
        match self.config.min_tls_version {
            crate::core::config::TlsVersion::Tls12 => {
                tracing::info!("{}", t!("log.server.config.tls.tls12_min_configured"));
            }
            crate::core::config::TlsVersion::Tls13 => {
                tracing::info!("{}", t!("log.server.config.tls.tls13_min_configured"));
            }
        }

        let (cert_der, private_key_der) = self.load_cert_and_key()?;

        // 获取 DER 字节用于 gRPC Identity
        let identity = Identity::from_pem(
            cert_der.as_ref(),
            Self::private_key_der_bytes(&private_key_der),
        );

        // 为 HTTP 配置 TLS（版本下限经 builder 强制）
        if self.config.http_enabled {
            self.setup_http_acceptor(cert_der.clone(), private_key_der.clone_key())?;
        }

        // 为 gRPC 配置 TLS
        if self.config.grpc_enabled {
            self.setup_grpc_config(identity)?;
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

    /// 加载并解析证书与私钥 PEM（首个匹配项；缺失或文件不存在显性报错）。
    fn load_cert_and_key(&self) -> TlsResult<(CertificateDer<'static>, PrivateKeyDer<'static>)> {
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

        // 首个证书项（非证书项跳过）；无证书项 → NoItemsFound → "Empty certificate chain"
        let cert_der = CertificateDer::pem_reader_iter(cert_file)
            .next()
            .ok_or_else(|| TlsError::CertificateLoadError("Empty certificate chain".to_string()))?
            .map_err(|e| TlsError::CertificateLoadError(e.to_string()))?;

        // 首个私钥项（Pkcs1/Pkcs8/Sec1 皆可）；无密钥项 → NoItemsFound → "Empty private key"
        let private_key_der = PrivateKeyDer::pem_reader_iter(key_file)
            .next()
            .ok_or_else(|| TlsError::PrivateKeyLoadError("Empty private key".to_string()))?
            .map_err(|e| TlsError::PrivateKeyLoadError(e.to_string()))?;

        Ok((cert_der, private_key_der))
    }

    /// 私钥的 DER 字节视图（gRPC Identity 装配用）。
    fn private_key_der_bytes<'a>(key: &'a PrivateKeyDer<'a>) -> &'a [u8] {
        match key {
            PrivateKeyDer::Pkcs1(k) => k.secret_pkcs1_der(),
            PrivateKeyDer::Pkcs8(k) => k.secret_pkcs8_der(),
            PrivateKeyDer::Sec1(k) => k.secret_sec1_der(),
            _ => unreachable!(),
        }
    }

    /// HTTP acceptor：builder_with_protocol_versions 强制版本下限，低于下限的
    /// ClientHello 被 rustls 拒绝；同时按配置设置 ALPN。
    fn setup_http_acceptor(
        &mut self,
        cert_der: CertificateDer<'static>,
        private_key: PrivateKeyDer<'static>,
    ) -> TlsResult<()> {
        let versions = enabled_protocol_versions(self.config.min_tls_version);
        let mut config_with_alpn = ServerConfig::builder_with_protocol_versions(versions)
            .with_no_client_auth()
            .with_single_cert(vec![cert_der], private_key)
            .map_err(|e| TlsError::InvalidConfig(e.to_string()))?;

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
        Ok(())
    }

    /// gRPC TLS 配置：Identity + 可选 mTLS CA。min_tls_version 只强制到 HTTP
    /// acceptor，gRPC 侧由 tonic 自行装配，恒可协商 TLS 1.2 + 1.3。
    fn setup_grpc_config(&mut self, identity: Identity) -> TlsResult<()> {
        let mut grpc_config = ServerTlsConfig::new().identity(identity);

        if let Some(ref ca_path) = self.config.ca_path {
            let ca_file =
                File::open(ca_path).map_err(|e| TlsError::CertificateLoadError(e.to_string()))?;

            // 首个 CA 证书项
            let ca_cert = CertificateDer::pem_reader_iter(ca_file)
                .next()
                .ok_or_else(|| TlsError::CertificateLoadError("Empty CA certificate".to_string()))?
                .map_err(|e| TlsError::CertificateLoadError(e.to_string()))?;

            let ca_cert_bytes = ca_cert.as_ref().to_vec();
            let ca_cert_tonic = Certificate::from_pem(ca_cert_bytes);
            grpc_config = grpc_config.client_ca_root(ca_cert_tonic);
        }

        self.grpc_tls_config = Some(Arc::new(grpc_config));

        if self.config.min_tls_version == crate::core::config::TlsVersion::Tls13 {
            tracing::warn!(
                event = "tls_min_version_scope",
                min_version = %self.config.min_tls_version,
                "min_tls_version is enforced on the HTTP acceptor only; the gRPC port \
                 uses tonic's default version range and still accepts TLS 1.2"
            );
        }
        Ok(())
    }
}

// ===== DualListener: HTTP 端口真实 TLS =====

/// 双模监听器：按配置选择明文 TCP 或 TLS（`TlsAcceptor` 包装）。
///
/// 实现 `axum::serve::Listener`（该 trait 未 sealed），使 `axum::serve` 无需
/// 额外依赖即在单一端口启用 HTTPS；未启用 TLS 时等价于裸 `TcpListener`。
///
/// TLS 握手在 per-connection 任务并发完成：只建连不发 ClientHello 的客户端
/// 无法冻结整个 HTTP 端口（未认证 DoS）。
pub enum DualListener {
    Plain(tokio::net::TcpListener),
    Tls(TlsEndpoint),
}

/// TLS 握手超时：慢发 / 挂起 ClientHello 的连接在此边界被丢弃。
const TLS_HANDSHAKE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// accept 错误退避（EMFILE 等持续错误时避免热自旋）。
const ACCEPT_ERROR_BACKOFF: std::time::Duration = std::time::Duration::from_millis(50);

/// 已完成握手连接的汇聚队列深度（对 axum serve 循环形成背压上限）。
const TLS_HANDSHAKE_QUEUE: usize = 128;

/// TLS 监听端：TCP accept 与 TLS 握手解耦，握手结果经有界 channel 汇聚。
pub struct TlsEndpoint {
    listener: tokio::net::TcpListener,
    acceptor: TlsAcceptor,
    /// 每个握手任务用的发送端克隆。
    tx: tokio::sync::mpsc::Sender<(DualStream, std::net::SocketAddr)>,
    /// 常驻发送端：防止 `rx` 因全部 sender drop 而返回 None，使 `select!` 空闲期正确挂起；只持有、从不发送。
    _keepalive: tokio::sync::mpsc::Sender<(DualStream, std::net::SocketAddr)>,
    rx: tokio::sync::mpsc::Receiver<(DualStream, std::net::SocketAddr)>,
}

/// DualListener 接受出的流：明文 TCP 或已握手的 TLS 服务端流。
pub enum DualStream {
    Plain(tokio::net::TcpStream),
    Tls(Box<tokio_rustls::server::TlsStream<tokio::net::TcpStream>>),
}

impl DualListener {
    /// 绑定地址；`tls` 为 `Some` 时该端口走 TLS 握手，否则明文。
    pub async fn bind(
        addr: std::net::SocketAddr,
        tls: Option<TlsAcceptor>,
    ) -> std::io::Result<Self> {
        let listener = tokio::net::TcpListener::bind(addr).await?;
        Ok(match tls {
            Some(acceptor) => {
                let (tx, rx) = tokio::sync::mpsc::channel(TLS_HANDSHAKE_QUEUE);
                DualListener::Tls(TlsEndpoint {
                    listener,
                    acceptor,
                    tx: tx.clone(),
                    _keepalive: tx,
                    rx,
                })
            }
            None => DualListener::Plain(listener),
        })
    }
}

impl axum::serve::Listener for DualListener {
    type Io = DualStream;
    type Addr = std::net::SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        match self {
            DualListener::Plain(listener) => {
                // 与 axum 内置实现一致：accept 错误记日志后按 50ms 退避重试
                // （EMFILE 等持续错误不热自旋）；用固有方法全限定调用避免解析到本 trait。
                loop {
                    match tokio::net::TcpListener::accept(listener).await {
                        Ok((stream, addr)) => return (DualStream::Plain(stream), addr),
                        Err(e) => {
                            tracing::warn!(error = %e, "tcp accept failed; retrying");
                            tokio::time::sleep(ACCEPT_ERROR_BACKOFF).await;
                        }
                    }
                }
            }
            DualListener::Tls(endpoint) => loop {
                tokio::select! {
                    // 已握手中的连接：握手成功即交付 axum
                    completed = endpoint.rx.recv() => {
                        if let Some(item) = completed {
                            return item;
                        }
                        // rx 不会返回 None（_keepalive 常驻）；到达此处仅为防御
                    }
                    // TCP accept 与握手并发推进：慢握手不阻塞后续连接受理
                    accepted = endpoint.listener.accept() => {
                        match accepted {
                            Ok((tcp, addr)) => {
                                let tx = endpoint.tx.clone();
                                let acceptor = endpoint.acceptor.clone();
                                tokio::spawn(async move {
                                    let handshake = tokio::time::timeout(
                                        TLS_HANDSHAKE_TIMEOUT,
                                        acceptor.accept(tcp),
                                    )
                                    .await;
                                    let stream = match handshake {
                                        Ok(Ok(stream)) => stream,
                                        // 握手失败/超时：记录并丢弃该连接
                                        Ok(Err(e)) => {
                                            tracing::warn!(error = %e, "TLS handshake failed; dropping connection");
                                            return;
                                        }
                                        Err(_) => {
                                            tracing::warn!("TLS handshake timed out; dropping connection");
                                            return;
                                        }
                                    };
                                    let _ =
                                        tx.send((DualStream::Tls(Box::new(stream)), addr)).await;
                                });
                            }
                            Err(e) => {
                                tracing::warn!(error = %e, "tcp accept failed; retrying");
                                tokio::time::sleep(ACCEPT_ERROR_BACKOFF).await;
                            }
                        }
                    }
                }
            },
        }
    }

    fn local_addr(&self) -> std::io::Result<std::net::SocketAddr> {
        match self {
            DualListener::Plain(listener) => listener.local_addr(),
            DualListener::Tls(endpoint) => endpoint.listener.local_addr(),
        }
    }
}

/// 由 axum 注入的连接对端地址：孤儿规则禁止为外来 `SocketAddr` 实现
/// `Connected`，故用 crate 新类型承载；读取端见 `middleware::utils::get_client_ip`。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PeerAddr(pub std::net::SocketAddr);

/// 支撑 `into_make_service_with_connect_info::<PeerAddr>()`：注入
/// `ConnectInfo<PeerAddr>`，使限流 / 认证失败计数 / 审计拿到真实 per-IP。
impl Connected<IncomingStream<'_, DualListener>> for PeerAddr {
    fn connect_info(target: IncomingStream<'_, DualListener>) -> Self {
        PeerAddr(*target.remote_addr())
    }
}

impl tokio::io::AsyncRead for DualStream {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match &mut *self {
            DualStream::Plain(s) => std::pin::Pin::new(s).poll_read(cx, buf),
            DualStream::Tls(s) => std::pin::Pin::new(&mut **s).poll_read(cx, buf),
        }
    }
}

impl tokio::io::AsyncWrite for DualStream {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        match &mut *self {
            DualStream::Plain(s) => std::pin::Pin::new(s).poll_write(cx, buf),
            DualStream::Tls(s) => std::pin::Pin::new(&mut **s).poll_write(cx, buf),
        }
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match &mut *self {
            DualStream::Plain(s) => std::pin::Pin::new(s).poll_flush(cx),
            DualStream::Tls(s) => std::pin::Pin::new(&mut **s).poll_flush(cx),
        }
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match &mut *self {
            DualStream::Plain(s) => std::pin::Pin::new(s).poll_shutdown(cx),
            DualStream::Tls(s) => std::pin::Pin::new(&mut **s).poll_shutdown(cx),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::{TlsConfig, TlsVersion};
    use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
    use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
    use rustls::{ClientConfig, DigitallySignedStruct, ProtocolVersion, SignatureScheme};
    use std::io::Write;
    use tempfile::NamedTempFile;
    use tokio_rustls::TlsConnector;

    /// 生成自签证书+私钥 PEM 临时文件，返回 (cert_file, key_file)。
    fn generate_test_cert_files() -> (NamedTempFile, NamedTempFile) {
        // 安装 ring crypto provider（幂等，可重复调用）。
        let _ = rustls::crypto::ring::default_provider().install_default();

        // rcgen 0.14：generate_simple_self_signed 返回 CertifiedKey。
        let certified = rcgen::generate_simple_self_signed(vec!["localhost".to_string()])
            .expect("generate self-signed cert");
        let cert_pem = certified.cert.pem();
        let key_pem = certified.signing_key.serialize_pem();

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
        assert_eq!(err1.clone(), err1);
    }

    // ===== TlsManager::new — default state =====

    #[test]
    fn test_tls_manager_new_disabled_config() {
        let config = TlsConfig::default();
        let manager = TlsManager::new(config);
        assert!(!manager.is_http_enabled());
        assert!(!manager.is_grpc_enabled());
        assert!(manager.http_acceptor().is_none());
        assert!(manager.grpc_tls_config().is_none());
    }

    #[test]
    fn test_tls_manager_new_preserves_disabled_flags() {
        // 未 initialize 时 acceptor 为 None，is_*_enabled 恒 false。
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
        // Empty cert file -> PemObject 迭代器无匹配项 -> "Empty certificate chain"
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

    // ===== min_tls_version 真实强制（以实际协商结果验证）=====
    // ServerConfig::versions 读不到，故用回环 TCP 真实握手证明：低于下限的
    // ClientHello 被拒，达到下限的协商成功。

    /// 握手两端的等待上限：断言前挂起的兜底（远超本地回环握手耗时）。
    const HANDSHAKE_TEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

    /// 客户端只通告 TLS 1.2（const 而非内联数组：实参位置需要 'static 借用）。
    const CLIENT_TLS12: &[&SupportedProtocolVersion] = &[&rustls::version::TLS12];

    /// 客户端只通告 TLS 1.3。
    const CLIENT_TLS13: &[&SupportedProtocolVersion] = &[&rustls::version::TLS13];

    /// 测试证书校验器：仅关心版本协商结果，跳过证书链与签名校验。
    #[derive(Debug)]
    struct AcceptAnyServerCert;

    impl ServerCertVerifier for AcceptAnyServerCert {
        fn verify_server_cert(
            &self,
            _end_entity: &CertificateDer<'_>,
            _intermediates: &[CertificateDer<'_>],
            _server_name: &ServerName<'_>,
            _ocsp_response: &[u8],
            _now: UnixTime,
        ) -> Result<ServerCertVerified, rustls::Error> {
            Ok(ServerCertVerified::assertion())
        }

        fn verify_tls12_signature(
            &self,
            _message: &[u8],
            _cert: &CertificateDer<'_>,
            _dss: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            Ok(HandshakeSignatureValid::assertion())
        }

        fn verify_tls13_signature(
            &self,
            _message: &[u8],
            _cert: &CertificateDer<'_>,
            _dss: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            Ok(HandshakeSignatureValid::assertion())
        }

        fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
            // 跟随 provider 能力（rcgen 默认 ECDSA P-256），避免握手在版本协商前失败。
            rustls::crypto::CryptoProvider::get_default()
                .map(|provider| {
                    provider
                        .signature_verification_algorithms
                        .mapping
                        .iter()
                        .map(|(scheme, _)| *scheme)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        }
    }

    /// 与只通告 `client_versions` 的客户端在回环地址握手，返回服务端结论：
    /// `Some(version)` = 协商成功，`None` = 被拒（含超时）。
    async fn negotiate_with_client_versions(
        acceptor: TlsAcceptor,
        client_versions: &'static [&'static SupportedProtocolVersion],
    ) -> Option<ProtocolVersion> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback listener");
        let addr = listener.local_addr().expect("loopback addr");

        let server = tokio::spawn(async move {
            let (tcp, _peer) = listener.accept().await.ok()?;
            let stream = tokio::time::timeout(HANDSHAKE_TEST_TIMEOUT, acceptor.accept(tcp))
                .await
                .ok()?
                .ok()?;
            stream.get_ref().1.protocol_version()
        });

        let client = tokio::spawn(async move {
            let config = ClientConfig::builder_with_protocol_versions(client_versions)
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(AcceptAnyServerCert))
                .with_no_client_auth();
            let connector = TlsConnector::from(Arc::new(config));
            let Ok(tcp) = tokio::net::TcpStream::connect(addr).await else {
                return;
            };
            let Ok(domain) = ServerName::try_from("localhost") else {
                return;
            };
            // 被服务端拒绝时这里返回 Err；结论统一由 server 侧判定。
            let _ = connector.connect(domain, tcp).await;
        });

        let verdict = match tokio::time::timeout(HANDSHAKE_TEST_TIMEOUT, server).await {
            Ok(Ok(version)) => version,
            // 服务端任务 panic 或超时：一律视为"未完成握手"
            _ => None,
        };
        client.abort();
        verdict
    }

    /// 走生产路径装配一个 HTTP acceptor（`tls.enabled = true` + `http_enabled`）。
    async fn http_acceptor_for(min_tls_version: TlsVersion) -> TlsAcceptor {
        let (cert_file, key_file) = generate_test_cert_files();
        let config = TlsConfig {
            enabled: true,
            cert_path: cert_file.path().to_str().unwrap().to_string(),
            key_path: key_file.path().to_str().unwrap().to_string(),
            http_enabled: true,
            grpc_enabled: false,
            min_tls_version,
            ..Default::default()
        };
        let mut manager = TlsManager::new(config);
        manager.initialize().await.expect("initialize tls manager");
        manager
            .http_acceptor()
            .cloned()
            .expect("http acceptor present after initialize")
    }

    /// tls13 下限：仅支持 1.2 的客户端被拒，TLS 1.3 客户端协商为 1.3。
    #[tokio::test]
    async fn test_http_min_tls13_acceptor_only_negotiates_tls13() {
        let acceptor = http_acceptor_for(TlsVersion::Tls13).await;

        assert_eq!(
            negotiate_with_client_versions(acceptor.clone(), CLIENT_TLS12).await,
            None,
            "min_tls_version = tls13 时不得与仅支持 TLS 1.2 的客户端完成握手"
        );
        assert_eq!(
            negotiate_with_client_versions(acceptor, CLIENT_TLS13).await,
            Some(ProtocolVersion::TLSv1_3),
            "TLS 1.3 客户端应握手成功并协商到 TLS 1.3"
        );
    }

    /// tls12 下限：TLS 1.2 客户端握手成功，TLS 1.3 客户端仍协商为 1.3。
    #[tokio::test]
    async fn test_http_min_tls12_acceptor_negotiates_both_versions() {
        let acceptor = http_acceptor_for(TlsVersion::Tls12).await;

        assert_eq!(
            negotiate_with_client_versions(acceptor.clone(), CLIENT_TLS12).await,
            Some(ProtocolVersion::TLSv1_2),
            "min_tls_version = tls12 时 TLS 1.2 客户端应握手成功"
        );
        assert_eq!(
            negotiate_with_client_versions(acceptor, CLIENT_TLS13).await,
            Some(ProtocolVersion::TLSv1_3),
            "TLS 1.3 客户端仍应优先协商到 TLS 1.3"
        );
    }

    /// enabled=false 且 per-port 开关为 true 属矛盾配置：不报错、不产出
    /// acceptor（明文启动），仅由 initialize 内 warn 上报。
    #[tokio::test]
    async fn test_disabled_with_per_port_flags_stays_plaintext() {
        let mut manager = TlsManager::new(TlsConfig {
            enabled: false,
            http_enabled: true,
            grpc_enabled: true,
            ..Default::default()
        });
        manager
            .initialize()
            .await
            .expect("contradictory config must not fail startup");
        assert!(
            manager.http_acceptor().is_none(),
            "tls.enabled = false 时不得产出 HTTP acceptor（明文）"
        );
        assert!(manager.grpc_tls_config().is_none());
        assert!(!manager.is_http_enabled());
        assert!(!manager.is_grpc_enabled());
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
        // ALPN 无法从 TlsAcceptor 读回；initialize() 成功即证明该分支被执行。
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
        // 自签证书在测试中同时充当 CA。
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
        // enabled=true 但两个 per-port 开关皆 false 时成功且不产出 acceptor。
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
        // Clone 用于 axum State。
        let _cloned = manager.clone();
    }
}
