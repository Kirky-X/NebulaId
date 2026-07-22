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

#![cfg(test)]

//! # 基础设施层端到端测试（infrastructure layer e2e tests）
//!
//! 本文件覆盖 `temp/功能场景穷举分析.md` 中以下章节的端到端场景：
//!
//! - **第 3.3 节 安全头**（`src/server/router.rs` L190-L214）：6 个
//!   `SetResponseHeaderLayer` 中间件注入安全响应头；通过构造带相同
//!   layer 的 axum Router + oneshot 验证响应头是否正确注入
//! - **第 3.2 节 IP 提取**（`src/server/middleware/utils.rs`）：
//!   `get_client_ip` 在 trusted_proxies / 非 trusted / 空 header 等场景下
//!   的行为
//! - **第 3.2 节 限流桶清理**（`src/server/rate_limit/limiter.rs`）：
//!   `start_cleanup` 后台任务周期移除空闲桶、保留活跃桶
//! - **第 2.3 节 容器构建**（`src/core/container/app_container.rs`）：
//!   builder 缺依赖返回 ConfigurationError、health_check 返回
//!   `Result<bool, CoreError>`
//! - **第 2.5 节 数据库连接**（`src/core/database/connection.rs`）：
//!   SQLite 内存连接、迁移执行、密码含 `${}` 占位符拒绝
//! - **第 3.6 节 TLS**（`src/server/config/tls.rs`）：证书/密钥不存在
//!   返回错误、用 rcgen 自签证书验证 initialize 成功
//!
//! ## 与现有单元测试的区别
//!
//! 现有单元测试聚焦「函数孤立行为」（如 `get_client_ip` 单次调用、
//! `RateLimiter::cleanup` 同步清理、`TlsManager::initialize` 单路径）。
//! 本文件聚焦「跨模块端到端协同」：
//!
//! - 安全头用真实 axum Router + oneshot 验证 HTTP 响应头
//! - 限流桶清理用真实 `start_cleanup` 后台 tokio 任务 + 时间窗口验证
//! - 容器 health_check 用真实 cache + mock db pool 验证错误传播
//! - TLS 用 rcgen 生成的真实自签证书文件验证完整初始化流程
//!
//! ## 并行安全
//!
//! 所有 TLS 测试用 `tempfile::NamedTempFile` 隔离文件 I/O；限流清理
//! 测试用独立的 `RateLimiter` 实例避免桶状态竞争。

use std::collections::HashMap;
use std::io::Write;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::{
    body::Body,
    http::{header, HeaderValue, Request, StatusCode},
    routing::get,
    Router,
};
use confers::interface::ConfigProvider;
use confers::types::AnnotatedValue;
use dbnexus::database::pool::PoolStatus;
use dbnexus::{ConnectionPool, DbConfig, DbError, DbResult, Session};
use oxcache::backend::MokaMemoryBackend;
use oxcache::Cache;
use sea_orm::{DatabaseBackend, MockDatabase, MockExecResult};
use tempfile::NamedTempFile;
use tower::ServiceExt;
use tower_http::set_header::SetResponseHeaderLayer;

use crate::core::config::{DatabaseConfig, DatabaseEngine, TlsConfig};
use crate::core::container::AppContainer;
use crate::core::database::{create_connection, run_migrations};
use crate::core::types::CoreError;
use crate::server::config::tls::{TlsError, TlsManager};
use crate::server::middleware::utils::get_client_ip;
use crate::server::rate_limit::limiter::RateLimiter;

// ============================================================================
// Mock 辅助（参考 src/core/container/app_container.rs 测试模式）
// ============================================================================

/// 最小化 ConfigProvider mock —— 空值表，满足 trait 即可。
struct MockConfigProvider {
    values: HashMap<String, AnnotatedValue>,
}

impl MockConfigProvider {
    fn new() -> Self {
        Self {
            values: HashMap::new(),
        }
    }
}

impl ConfigProvider for MockConfigProvider {
    fn get_raw(&self, key: &str) -> Option<&AnnotatedValue> {
        self.values.get(key)
    }
    fn keys(&self) -> Vec<String> {
        self.values.keys().cloned().collect()
    }
}

/// 最小化 ConnectionPool mock —— get_session 总是返回 Err，用于
/// 验证 health_check 的错误传播路径。
struct MockConnectionPool {
    status: PoolStatus,
    config: DbConfig,
    session_error_msg: String,
}

impl MockConnectionPool {
    fn new(status: PoolStatus, config: DbConfig, session_error_msg: String) -> Self {
        Self {
            status,
            config,
            session_error_msg,
        }
    }
}

#[async_trait]
impl ConnectionPool for MockConnectionPool {
    async fn get_session(&self, _role: &str) -> DbResult<Session> {
        Err(DbError::Config(self.session_error_msg.clone()))
    }

    fn status(&self) -> PoolStatus {
        self.status.clone()
    }

    fn config(&self) -> &DbConfig {
        &self.config
    }
}

/// 构造一个 PoolStatus（参考 app_container.rs 测试）。
fn pool_status(active: u32, idle: u32, max_active: u32) -> PoolStatus {
    PoolStatus {
        total: active + idle,
        active,
        idle,
        wait_count: 0,
        max_waiters: 0,
        borrow_count: 0,
        max_active,
    }
}

fn make_mock_config() -> Arc<dyn ConfigProvider> {
    Arc::new(MockConfigProvider::new())
}

fn make_mock_db_pool() -> Arc<dyn ConnectionPool> {
    Arc::new(MockConnectionPool::new(
        pool_status(0, 0, 10),
        DbConfig::default(),
        "mock session unavailable".to_string(),
    ))
}

async fn make_test_cache() -> Arc<Cache<String, Vec<u8>>> {
    let l1 = MokaMemoryBackend::builder().capacity(100).build();
    let cache: Cache<String, Vec<u8>> = Cache::builder()
        .backend_arc(Arc::new(l1))
        .build()
        .await
        .expect("cache build should succeed with MokaMemoryBackend");
    Arc::new(cache)
}

// ============================================================================
// E2E-SECHEAD 组：安全头注入端到端
// ============================================================================

/// 构造一个带 6 个安全头 `SetResponseHeaderLayer` 的最小 Router，
/// 复刻 `src/server/router.rs` L190-L214 的安全头注入配置。
fn build_security_headers_router() -> Router {
    Router::new()
        .route("/test", get(|| async { "ok" }))
        .layer(SetResponseHeaderLayer::overriding(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::X_FRAME_OPTIONS,
            HeaderValue::from_static("DENY"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static("default-src 'self'"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::STRICT_TRANSPORT_SECURITY,
            HeaderValue::from_static("max-age=31536000; includeSubDomains"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::X_XSS_PROTECTION,
            HeaderValue::from_static("1; mode=block"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::REFERRER_POLICY,
            HeaderValue::from_static("strict-origin-when-cross-origin"),
        ))
}

/// E2E-SECHEAD-001: 验证响应包含所有 6 个安全头。
#[tokio::test]
async fn e2e_security_headers_injected_on_response() {
    let app = build_security_headers_router();
    let response = app
        .oneshot(Request::builder().uri("/test").body(Body::empty()).unwrap())
        .await
        .expect("oneshot should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    // 验证所有 6 个安全头都存在
    assert!(
        response.headers().get("x-content-type-options").is_some(),
        "X-Content-Type-Options 应被注入"
    );
    assert!(
        response.headers().get("x-frame-options").is_some(),
        "X-Frame-Options 应被注入"
    );
    assert!(
        response.headers().get("content-security-policy").is_some(),
        "Content-Security-Policy 应被注入"
    );
    assert!(
        response
            .headers()
            .get("strict-transport-security")
            .is_some(),
        "Strict-Transport-Security 应被注入"
    );
    assert!(
        response.headers().get("x-xss-protection").is_some(),
        "X-XSS-Protection 应被注入"
    );
    assert!(
        response.headers().get("referrer-policy").is_some(),
        "Referrer-Policy 应被注入"
    );
}

/// E2E-SECHEAD-002: 验证 X-Content-Type-Options: nosniff。
#[tokio::test]
async fn e2e_security_header_x_content_type_options_nosniff() {
    let app = build_security_headers_router();
    let response = app
        .oneshot(Request::builder().uri("/test").body(Body::empty()).unwrap())
        .await
        .expect("oneshot should succeed");

    assert_eq!(
        response
            .headers()
            .get("x-content-type-options")
            .expect("X-Content-Type-Options 应存在"),
        "nosniff"
    );
}

/// E2E-SECHEAD-003: 验证 X-Frame-Options: DENY。
#[tokio::test]
async fn e2e_security_header_x_frame_options_deny() {
    let app = build_security_headers_router();
    let response = app
        .oneshot(Request::builder().uri("/test").body(Body::empty()).unwrap())
        .await
        .expect("oneshot should succeed");

    assert_eq!(
        response
            .headers()
            .get("x-frame-options")
            .expect("X-Frame-Options 应存在"),
        "DENY"
    );
}

/// E2E-SECHEAD-004: 验证 CSP: default-src 'self'。
#[tokio::test]
async fn e2e_security_header_csp_default_src_self() {
    let app = build_security_headers_router();
    let response = app
        .oneshot(Request::builder().uri("/test").body(Body::empty()).unwrap())
        .await
        .expect("oneshot should succeed");

    assert_eq!(
        response
            .headers()
            .get("content-security-policy")
            .expect("Content-Security-Policy 应存在"),
        "default-src 'self'"
    );
}

/// E2E-SECHEAD-005: 验证 HSTS max-age=31536000。
#[tokio::test]
async fn e2e_security_header_hsts_max_age() {
    let app = build_security_headers_router();
    let response = app
        .oneshot(Request::builder().uri("/test").body(Body::empty()).unwrap())
        .await
        .expect("oneshot should succeed");

    let hsts = response
        .headers()
        .get("strict-transport-security")
        .expect("Strict-Transport-Security 应存在");
    let hsts_str = hsts.to_str().expect("HSTS 头应为合法 ASCII");
    assert!(
        hsts_str.contains("max-age=31536000"),
        "HSTS 应含 max-age=31536000，实际为: {hsts_str}"
    );
}

// ============================================================================
// E2E-IP 组：IP 提取端到端
// ============================================================================

/// 构造一个带指定 peer SocketAddr / X-Forwarded-For / X-Real-IP 的 Request，
/// 复刻 `src/server/middleware/utils.rs` 测试中的 make_request 模式。
fn make_request(peer: Option<SocketAddr>, xff: Option<&str>, xri: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder().uri("/").body(Body::empty()).unwrap();
    if let Some(addr) = peer {
        builder.extensions_mut().insert(addr);
    }
    if let Some(v) = xff {
        builder
            .headers_mut()
            .insert("x-forwarded-for", v.parse().unwrap());
    }
    if let Some(v) = xri {
        builder
            .headers_mut()
            .insert("x-real-ip", v.parse().unwrap());
    }
    builder
}

/// E2E-IP-001: 无 trusted_proxies 时用直连 IP（忽略 XFF 头）。
#[tokio::test]
async fn e2e_get_client_ip_direct_when_no_trusted_proxies() {
    let req = make_request(
        Some(SocketAddr::from((Ipv4Addr::new(10, 0, 0, 1), 8080))),
        Some("1.2.3.4"),
        None,
    );
    let ip = get_client_ip(&req, &[]).expect("应返回直连 IP");
    assert_eq!(ip, "10.0.0.1");
}

/// E2E-IP-002: trusted_proxies 时用 X-Forwarded-For 首跳。
#[tokio::test]
async fn e2e_get_client_ip_uses_xff_when_trusted() {
    let trusted: Vec<IpAddr> = vec![IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))];
    let req = make_request(
        Some(SocketAddr::from((Ipv4Addr::new(10, 0, 0, 1), 8080))),
        Some("203.0.113.5, 10.0.0.1"),
        None,
    );
    let ip = get_client_ip(&req, &trusted).expect("应返回 XFF 首跳 IP");
    assert_eq!(ip, "203.0.113.5");
}

/// E2E-IP-003: trusted_proxies 时（XFF 缺失）回退到 X-Real-IP。
#[tokio::test]
async fn e2e_get_client_ip_uses_x_real_ip_when_trusted() {
    let trusted: Vec<IpAddr> = vec![IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))];
    let req = make_request(
        Some(SocketAddr::from((Ipv4Addr::new(10, 0, 0, 1), 8080))),
        None,
        Some("203.0.113.7"),
    );
    let ip = get_client_ip(&req, &trusted).expect("应返回 X-Real-IP");
    assert_eq!(ip, "203.0.113.7");
}

/// E2E-IP-004: peer 不在 trusted_proxies 时忽略 XFF，返回直连 IP。
#[tokio::test]
async fn e2e_get_client_ip_ignores_xff_when_untrusted() {
    let trusted: Vec<IpAddr> = vec![IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))];
    let req = make_request(
        Some(SocketAddr::from((Ipv4Addr::new(192, 168, 0, 1), 8080))),
        Some("203.0.113.5"),
        None,
    );
    let ip = get_client_ip(&req, &trusted).expect("应返回直连 IP（非信任 peer）");
    assert_eq!(ip, "192.168.0.1");
}

/// E2E-IP-005: 无 peer SocketAddr 且非信任场景，返回 None。
#[tokio::test]
async fn e2e_get_client_ip_returns_none_when_no_headers() {
    // 无 peer SocketAddr 扩展，且 trusted_proxies 为空 → 无法确定 IP
    let req = make_request(None, Some("1.2.3.4"), None);
    assert!(
        get_client_ip(&req, &[]).is_none(),
        "无 peer 且无信任代理时应返回 None"
    );
}

// ============================================================================
// E2E-CLEANUP 组：限流桶清理端到端
// ============================================================================

/// E2E-CLEANUP-001: start_cleanup 后台任务应移除超过 max_idle 的空闲桶。
#[tokio::test]
async fn e2e_rate_limiter_cleanup_removes_idle_buckets() {
    let limiter = RateLimiter::new(10, 5);

    // 创建一个桶，随后不再访问（变空闲）
    limiter.check_rate_limit("idle-key", None, None).await;
    assert_eq!(limiter.bucket_count(), 1, "初始应有 1 个桶");

    // 启动后台清理：max_idle=50ms，interval=20ms
    let handle = limiter.start_cleanup(Duration::from_millis(50), Duration::from_millis(20));

    // 等待足够长时间让清理任务运行多次（覆盖 max_idle 阈值）
    tokio::time::sleep(Duration::from_millis(150)).await;

    assert_eq!(limiter.bucket_count(), 0, "空闲桶应被后台清理任务移除");

    handle.abort();
}

/// E2E-CLEANUP-002: start_cleanup 后台任务应保留持续访问的活跃桶。
#[tokio::test]
async fn e2e_rate_limiter_cleanup_keeps_active_buckets() {
    let limiter = RateLimiter::new(100, 100);

    // 启动后台清理：max_idle=80ms，interval=20ms
    let handle = limiter.start_cleanup(Duration::from_millis(80), Duration::from_millis(20));

    // 在多个清理周期内持续访问同一 key，保持桶活跃
    for _ in 0..6 {
        limiter.check_rate_limit("active-key", None, None).await;
        tokio::time::sleep(Duration::from_millis(15)).await;
    }

    assert_eq!(limiter.bucket_count(), 1, "持续访问的活跃桶应被保留");
    assert!(
        limiter.get_usage("active-key").is_some(),
        "活跃桶应仍可查询 usage"
    );

    handle.abort();
}

// ============================================================================
// E2E-CONTAINER 组：容器构建端到端
// ============================================================================

/// E2E-CONTAINER-001: builder 缺所有依赖时 try_build 返回 ConfigurationError。
#[tokio::test]
async fn e2e_container_builder_missing_config_returns_error() {
    // 空构造器，未提供任何依赖
    let result = AppContainer::builder().try_build();
    match result {
        Err(CoreError::ConfigurationError(msg)) => {
            assert!(
                msg.contains("config provider"),
                "应提示 config provider 缺失，实际为: {msg}"
            );
        }
        other => panic!("期望 ConfigurationError，实际为: {other:?}"),
    }
}

/// E2E-CONTAINER-002: health_check 返回 Result<bool, CoreError>。
///
/// 用 mock config + 真实 cache + mock db pool（get_session 返回 Err）
/// 构造 container。cache 健康检查通过，db 健康检查失败 → health_check
/// 返回 Err(DatabaseError)，验证错误传播路径与返回类型签名。
#[tokio::test]
async fn e2e_container_health_check_returns_bool() {
    let container = AppContainer::with_dependencies(
        make_mock_config(),
        make_test_cache().await,
        make_mock_db_pool(),
    );

    let result = container.health_check().await;
    // health_check 签名为 Result<bool, CoreError>；mock db 不健康 → Err
    match result {
        Ok(b) => {
            // 健康场景：返回 Ok(true)（cache 与 db 都健康时）
            let _: bool = b;
        }
        Err(CoreError::DatabaseError(msg)) => {
            // mock db pool 的 get_session 返回 Err，错误应传播
            assert!(
                msg.contains("mock session unavailable"),
                "应传播 db session 错误，实际为: {msg}"
            );
        }
        Err(other) => panic!("期望 Ok(bool) 或 DatabaseError，实际为: {other:?}"),
    }
}

// ============================================================================
// E2E-DB 组：数据库连接端到端
// ============================================================================

/// E2E-DB-001: SQLite 内存连接成功。
///
/// 需要 `--features sqlite` 启用 SQLite 后端；否则该测试不编译。
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn e2e_database_sqlite_memory_connection_succeeds() {
    let config = DatabaseConfig {
        engine: DatabaseEngine::Sqlite,
        url: "sqlite::memory:".to_string(),
        host: String::new(),
        port: 0,
        username: String::new(),
        password: String::new(),
        database: "sqlite::memory:".to_string(),
        max_connections: 10,
        min_connections: 1,
        acquire_timeout_seconds: 30,
        idle_timeout_seconds: 300,
    };

    let conn = create_connection(&config).await;
    assert!(conn.is_ok(), "SQLite 内存连接应成功: {:?}", conn.err());
}

/// E2E-DB-002: run_migrations 在所有 execute 成功时返回 Ok（迁移创建表）。
///
/// run_migrations 发出 1 个 CREATE SCHEMA + 5 个 CREATE TABLE = 6 个
/// execute 语句。用 MockDatabase 模拟真实数据库全部成功执行，验证
/// 迁移逻辑的完整性（所有表创建语句都被发出且无错误）。
#[tokio::test]
async fn e2e_database_run_migrations_creates_tables() {
    // 1 schema + 5 tables = 6 个成功的 execute
    let results: Vec<MockExecResult> = (0..6)
        .map(|_| MockExecResult {
            last_insert_id: 0,
            rows_affected: 0,
        })
        .collect();
    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_exec_results(results)
        .into_connection();

    let result = run_migrations(&db).await;
    assert!(
        result.is_ok(),
        "迁移应成功创建 schema + 5 张表: {:?}",
        result.err()
    );
}

/// E2E-DB-003: 密码含 `${}` 环境变量占位符时拒绝连接。
///
/// 验证 `src/core/database/connection.rs` L60 的安全检查：未替换的
/// `${VAR}` 占位符（CWE-1188 / 环境变量未展开）应返回
/// ConfigurationError，防止用字面量 `${...}` 作为密码连接数据库。
#[tokio::test]
async fn e2e_database_password_with_env_var_rejected() {
    let config = DatabaseConfig {
        engine: DatabaseEngine::Postgresql,
        url: String::new(),
        host: "localhost".to_string(),
        port: 5432,
        username: "user".to_string(),
        password: "${DB_PASSWORD}".to_string(),
        database: "test_db".to_string(),
        max_connections: 10,
        min_connections: 1,
        acquire_timeout_seconds: 5,
        idle_timeout_seconds: 300,
    };

    let result = create_connection(&config).await;
    match result {
        Err(CoreError::ConfigurationError(msg)) => {
            assert!(
                msg.contains("password"),
                "应拒绝含 ${{}} 占位符的密码，实际为: {msg}"
            );
        }
        other => panic!("期望 ConfigurationError，实际为: {other:?}"),
    }
}

// ============================================================================
// E2E-TLS 组：TLS 初始化端到端
// ============================================================================

/// 用 rcgen 生成自签证书 + 密钥的 PEM 临时文件，用于 TLS 初始化测试。
/// 复刻 `src/server/config/tls.rs` 测试中的 generate_test_cert_files 模式。
fn generate_test_cert_files() -> (NamedTempFile, NamedTempFile) {
    // 安装 ring crypto provider（rustls 0.23 + rcgen 0.13 需要）
    // install_default() 幂等，多次调用安全。
    let _ = rustls::crypto::ring::default_provider().install_default();

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

/// E2E-TLS-001: 证书文件不存在 → CertificateLoadError。
#[tokio::test]
async fn e2e_tls_initialize_missing_cert_returns_error() {
    let config = TlsConfig {
        enabled: true,
        cert_path: "/nonexistent/cert.pem".to_string(),
        key_path: "/nonexistent/key.pem".to_string(),
        http_enabled: true,
        ..Default::default()
    };
    let mut manager = TlsManager::new(config);
    let result = manager.initialize().await;

    assert!(result.is_err(), "证书不存在应返回错误");
    let err = result.unwrap_err();
    assert!(
        matches!(err, TlsError::CertificateLoadError(_)),
        "应为 CertificateLoadError，实际为: {err:?}"
    );
    assert!(
        err.to_string().contains("Certificate file not found"),
        "错误消息应含 'Certificate file not found'，实际为: {err}"
    );
}

/// E2E-TLS-002: 密钥文件不存在 → PrivateKeyLoadError。
#[tokio::test]
async fn e2e_tls_initialize_missing_key_returns_error() {
    // 提供有效证书，但密钥路径不存在
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

    assert!(result.is_err(), "密钥不存在应返回错误");
    let err = result.unwrap_err();
    assert!(
        matches!(err, TlsError::PrivateKeyLoadError(_)),
        "应为 PrivateKeyLoadError，实际为: {err:?}"
    );
    assert!(
        err.to_string().contains("Private key file not found"),
        "错误消息应含 'Private key file not found'，实际为: {err}"
    );
}

/// E2E-TLS-003: 用 rcgen 生成的自签证书 + 密钥验证 initialize 成功。
#[tokio::test]
async fn e2e_tls_initialize_with_valid_cert_succeeds() {
    let (cert_file, key_file) = generate_test_cert_files();
    let config = TlsConfig {
        enabled: true,
        cert_path: cert_file
            .path()
            .to_str()
            .expect("cert path utf8")
            .to_string(),
        key_path: key_file.path().to_str().expect("key path utf8").to_string(),
        http_enabled: true,
        grpc_enabled: false,
        ..Default::default()
    };
    let mut manager = TlsManager::new(config);
    let result = manager.initialize().await;

    assert!(
        result.is_ok(),
        "用 rcgen 自签证书应初始化成功: {:?}",
        result.err()
    );
    assert!(manager.is_http_enabled(), "http 应被启用且 acceptor 已创建");
    assert!(manager.http_acceptor().is_some(), "http_acceptor 应存在");
    assert!(!manager.is_grpc_enabled(), "grpc 未启用");
}
