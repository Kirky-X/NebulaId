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
use axum::extract::connect_info::Connected;
use axum::serve::IncomingStream;
use rustls::pki_types::PrivateKeyDer;
use rustls::SupportedProtocolVersion;
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

/// `min_tls_version = tls13`：仅允许 TLS 1.3。
const PROTOCOL_VERSIONS_TLS13: &[&SupportedProtocolVersion] = &[&rustls::version::TLS13];

/// `min_tls_version = tls12`：TLS 1.3 优先，TLS 1.2 作为下限兼容。
const PROTOCOL_VERSIONS_TLS12_PLUS: &[&SupportedProtocolVersion] =
    &[&rustls::version::TLS13, &rustls::version::TLS12];

/// 把 `tls.min_tls_version` 映射为 rustls 服务端允许协商的协议版本集合
/// （T027⑤ 真实强制：此前 `min_tls_version` 只写日志，不进 `ServerConfig`，
/// 实际恒为 rustls 默认的 TLS 1.2 + 1.3）。
///
/// 「无法识别的取值显性失败、绝不静默降级」由以下两道防线保证：
/// 1. 运行期：[`crate::core::config::TlsVersion`] 是闭合枚举，
///    `min_tls_version = "tls11"` 之类的字面量在配置反序列化阶段就被 serde 拒绝
///    （`#[serde(default)]` 只兜底「字段缺失」，不会吞掉非法值），
///    配置根本到不了这里；
/// 2. 编译期：此处是穷尽 match，后续给 `TlsVersion` 加变体而忘记登记会直接
///    编译失败，不会出现「配置写了但不生效」。
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
            // T027④：矛盾配置显性上报。`enabled = false` 会整体关闭 TLS，
            // 此时 `http_enabled` / `grpc_enabled` 被忽略，端口按明文启动。
            // 旧实现静默返回，运维读到 `tls.http_enabled = true` 会误以为链路已加密。
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
            // T027⑤：min_tls_version 进 ServerConfig —— 用
            // builder_with_protocol_versions 声明服务端可协商的版本集合，
            // 低于该集合的 ClientHello 会被 rustls 直接拒绝（不再依赖
            // rustls 默认的 TLS 1.2 + 1.3，也不再"只记日志不生效"）。
            let versions = enabled_protocol_versions(self.config.min_tls_version);
            let mut config_with_alpn = ServerConfig::builder_with_protocol_versions(versions)
                .with_no_client_auth()
                .with_single_cert(vec![cert_der.clone()], private_key_der.clone_key())
                .map_err(|e| TlsError::InvalidConfig(e.to_string()))?;

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

            // T027⑤ 边界如实上报：min_tls_version 只强制到上面的 HTTP acceptor，
            // gRPC 侧 TLS 由 tonic 自行装配 ServerConfig（不接受本 crate 注入
            // 版本集合），其可协商范围恒为 tonic/rustls 默认的 TLS 1.2 + 1.3。
            // 配置 1.3 时两者不一致，必须显式说明，否则 tls_initialized 日志
            // 会让人误以为两个端口都只允许 TLS 1.3。
            if self.config.min_tls_version == crate::core::config::TlsVersion::Tls13 {
                tracing::warn!(
                    event = "tls_min_version_scope",
                    min_version = %self.config.min_tls_version,
                    "min_tls_version is enforced on the HTTP acceptor only; the gRPC port \
                     uses tonic's default version range and still accepts TLS 1.2"
                );
            }
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

// ============================================================================
// DualListener（wiring T005）: HTTP 端口真实 TLS
// ============================================================================

/// 双模监听器：按配置选择明文 TCP 或 TLS（经 `TlsAcceptor` 包装）。
///
/// 实现 `axum::serve::Listener`（axum 0.8 该 trait 未 sealed，可外部实现），
/// 使 `axum::serve` 无需 `axum-server` 等额外依赖即可在单一端口上启用 HTTPS。
/// 未启用 TLS 时行为与裸 `TcpListener` 完全一致（明文回退）。
///
/// TLS 模式下握手在 per-connection 任务中并发完成（converge T019）：
/// 旧实现把 `acceptor.accept(tcp).await` 内联在 accept 循环里，客户端
/// 只建连不发 ClientHello 即可永久冻结整个 HTTP 端口（未认证 DoS）。
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
    /// 常驻发送端：保证 `rx` 不因"所有 sender 已 drop"而立刻返回 None，
    /// 使 `select!` 在空闲期正确挂起而非忙轮询。本字段只持有、从不发送。
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
                // 与 axum 内置 TcpListener 实现一致：accept 错误记日志后重试
                // （T019 补充 50ms 退避：EMFILE 等持续性错误不再热自旋）。
                // 注意用固有方法全限定调用，避免解析到本 trait 的 accept。
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

/// 由 axum 注入的连接对端地址（converge T018）。
///
/// 孤儿规则禁止为外来的 `SocketAddr` 实现 axum 的 `Connected`，
/// 故用本 crate 新类型承载；读取端见 `middleware::utils::get_client_ip`。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PeerAddr(pub std::net::SocketAddr);

/// 支撑 `into_make_service_with_connect_info::<PeerAddr>()`：axum 经本 trait
/// 从已接受的流提取对端地址并注入 `ConnectInfo<PeerAddr>`，使限流 / 认证失败
/// 计数 / 审计拿到真实 per-IP 语义。
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

    /// Generate a self-signed cert + key pair as PEM files for testing.
    /// Returns (cert_file, key_file) NamedTempFile handles.
    fn generate_test_cert_files() -> (NamedTempFile, NamedTempFile) {
        // Install ring crypto provider (required by rustls 0.23 + rcgen 0.13).
        // `install_default()` is idempotent; safe to call multiple times.
        let _ = rustls::crypto::ring::default_provider().install_default();

        // rcgen 0.14 API: `generate_simple_self_signed` returns a
        // `CertifiedKey { cert, signing_key }`. `cert.pem()` and
        // `signing_key.serialize_pem()` both return `String`.
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

    // ===== T027⑤: min_tls_version 真实强制（公开可观测面 = 实际协商结果）=====
    //
    // `rustls::ServerConfig` 的 `versions` 字段是 `pub(super)`（本 crate 读不到），
    // 因此不引入任何测试专用 pub API，改用真实回环 TCP + TLS 握手的协商结果证明：
    // 低于下限的 ClientHello 被服务端拒绝（握手不完成），达到下限的协商成功。

    /// 握手两端的等待上限：断言前挂起的兜底（远超本地回环握手耗时）。
    const HANDSHAKE_TEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

    /// 客户端只通告 TLS 1.2（const 而非内联数组：实参位置需要 'static 借用）。
    const CLIENT_TLS12: &[&SupportedProtocolVersion] = &[&rustls::version::TLS12];

    /// 客户端只通告 TLS 1.3。
    const CLIENT_TLS13: &[&SupportedProtocolVersion] = &[&rustls::version::TLS13];

    /// 测试专用证书校验器：本模块证书由 rcgen 现造（自签、非 CA、不在信任库），
    /// 而这里只关心**版本协商结果**，故跳过证书链与签名校验。
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
            // 跟随已安装 provider 的能力（rcgen 默认签发 ECDSA P-256 证书），
            // 避免因通告算法缺失导致握手在版本协商之前就失败。
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

    /// 用 `acceptor`（生产路径 `TlsManager::initialize()` 的产物）与一个
    /// 只通告 `client_versions` 的 rustls 客户端在 127.0.0.1 上握手。
    ///
    /// 返回服务端视角结论：`Some(version)` 表示握手完成并协商到该版本，
    /// `None` 表示服务端拒绝（含超时）。
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

    /// min_tls_version = tls13：只允许 TLS 1.3 —— 仅支持 1.2 的客户端被拒，
    /// 支持 1.3 的客户端握手成功且协商结果为 1.3。
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

    /// min_tls_version = tls12：下限放宽真实生效 —— TLS 1.2 客户端握手成功，
    /// 同时 TLS 1.3 客户端仍优先协商到 1.3（证明未写死为单一版本）。
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

    /// `enabled = false` 且任一 per-port 开关为 true 属矛盾配置：
    /// 不报错但必须保持"不产出 acceptor"（明文启动），由 initialize 内的
    /// warn 显式上报（T027④）。
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
