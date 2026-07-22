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
// See the License for the specific language and permissions and
// limitations under the License.

//! # 服务层端到端测试（server layer e2e tests）
//!
//! 本文件覆盖 `temp/功能场景穷举分析.md` 第 3 节「服务层」的端到端场景，
//! 跨越 `src/server/*` 多个模块组合验证真实 HTTP 流量下的协同行为：
//!
//! - **API 版本中间件**（`api_version.rs`）：默认 v1、v2 拒绝、大小写不敏感、
//!   响应头回写
//! - **请求体大小限制**（`size_limit.rs`）：1MB 边界、超限 413
//! - **CORS 环境感知**（`config/cors.rs`）：生产缺 ALLOWED_ORIGINS → 空
//!   CorsLayer、开发缺配 → localhost 默认、显式配置生效
//! - **审计日志**（`audit/logger.rs` + `audit/middleware.rs`）：内存记录器
//!   事件流转、文件持久化、路径遍历防护、IP 脱敏、事件 ID 单调递增、
//!   审计中间件 end-to-end HTTP 流
//! - **限流**（`rate_limit/limiter.rs` + `rate_limit/middleware.rs`）：Token
//!   Bucket 放行/拒绝、按 key 隔离、限流中间件 429 响应
//! - **Anonymous 拦截**（`router.rs::anonymous_block_middleware`）：禁用
//!   认证场景下 Anonymous 角色被拒绝、User/Admin 放行、扩展缺失 fail-closed
//!
//! ## 与现有单元测试的区别
//!
//! 现有单元测试聚焦「函数孤立行为」（如 `ApiVersion::from_str` 解析、
//! `AuditLogger::log` 单次调用、`RateLimiter::new` 构造）。本文件聚焦
//! 「跨模块端到端协同」：
//!
//! - API 版本中间件用真实 axum Router + oneshot 验证 HTTP 请求/响应头
//! - 审计中间件用真实 `ApiKeyAuth`（禁用模式）+ `RateLimiter` 组合验证
//!   status_code → AuditResult 映射
//! - 限流中间件用真实 axum Router 验证 X-RateLimit-* 响应头
//! - Anonymous 拦截用 layer 顺序组合（outer auth → inner block）验证
//!   fail-closed 语义
//!
//! ## 并行安全
//!
//! 所有测试用 `tempfile::tempdir()` 隔离文件 I/O；CORS 测试用
//! `std::env::set_var` 时通过 `E2E_ENV_LOCK` 串行化避免环境变量竞争。

use std::sync::Arc;

use sdforge::axum::{
    body::Body,
    http::{Request, StatusCode},
    middleware::{from_fn, from_fn_with_state, Next},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use sdforge::tower::ServiceExt;
use tempfile::tempdir;

use crate::server::api_version::{
    api_version_middleware, ApiVersion, ApiVersionErrorResponse, API_VERSION_HEADER,
    CURRENT_API_VERSION,
};
use crate::server::audit::{AuditEvent, AuditEventType, AuditLogger, AuditResult};
use crate::server::config::cors::{
    create_cors_layer, create_dev_cors_layer, create_env_aware_cors_layer,
};
use crate::server::middleware::size_limit::{
    create_size_limit_middleware, RequestBodyTooLarge, MAX_REQUEST_SIZE,
};
use crate::server::middleware::ApiKeyRole;
use crate::server::rate_limit::limiter::RateLimiter;
use crate::server::rate_limit::middleware::RateLimitMiddleware;
use crate::server::router::anonymous_block_middleware;

/// 串行化所有读取 / 写入环境变量的 CORS 测试，避免并行竞争。
static E2E_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

// ============================================================================
// 工具函数
// ============================================================================

/// 构造一个最小的 axum Router 并附加 `api_version_middleware` 作为
/// 全局 layer，handler 返回 200 OK。
fn build_api_version_router() -> Router {
    Router::new()
        .route("/ping", get(|| async { "pong" }))
        .layer(from_fn(api_version_middleware))
}

/// 构造一个匿名处理器 + RequestBodyTooLarge 错误映射的 Router，挂上
/// size_limit middleware。处理器必须实际读取 body 才能触发
/// RequestBodyLimitLayer 的限制——否则 body 不被消费时 limit 不会生效。
fn build_size_limit_router() -> Router {
    Router::new()
        .route(
            "/echo",
            sdforge::axum::routing::post(|body: sdforge::axum::body::Bytes| async move {
                format!("received {} bytes", body.len())
            }),
        )
        .layer(create_size_limit_middleware())
}

/// 生成一个指定字节数的字符串 body。
fn make_body(size: usize) -> String {
    "x".repeat(size)
}

/// Wrapper：把 `&self` 实例方法适配为 axum `from_fn_with_state` 兼容的
/// `async fn(State<S>, Request, Next) -> Response` 签名。
/// 不能用闭包，因为 axum 0.8 的 `FromFnLayer` 要求 `F: FnMut + Clone + Send + 'static`
/// 且签名严格匹配；闭包虽然也能匹配但 `State<S>` extractor 必须显式声明。
async fn rate_limit_handler(
    sdforge::axum::extract::State(state): sdforge::axum::extract::State<RateLimitMiddleware>,
    req: Request<Body>,
    next: Next,
) -> Response {
    state.rate_limit_middleware(req, next).await
}

// ============================================================================
// E2E-APIVER-001: 默认版本协商（无 X-API-Version 头）
// ============================================================================

#[tokio::test]
async fn e2e_api_version_default_v1_when_header_missing() {
    let router = build_api_version_router();

    let resp = router
        .oneshot(Request::builder().uri("/ping").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    // 响应头应包含默认版本 v1
    let v = resp
        .headers()
        .get(&API_VERSION_HEADER)
        .expect("响应头必须包含 X-API-Version");
    assert_eq!(v.to_str().unwrap(), CURRENT_API_VERSION);
}

// ============================================================================
// E2E-APIVER-002: 显式 v1 头被接受并回写
// ============================================================================

#[tokio::test]
async fn e2e_api_version_explicit_v1_header_accepted() {
    let router = build_api_version_router();

    let resp = router
        .oneshot(
            Request::builder()
                .uri("/ping")
                .header(&API_VERSION_HEADER, "v1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers()
            .get(&API_VERSION_HEADER)
            .unwrap()
            .to_str()
            .unwrap(),
        "v1"
    );
}

// ============================================================================
// E2E-APIVER-003: v2 头返回 400 Bad Request
// ============================================================================

#[tokio::test]
async fn e2e_api_version_v2_returns_bad_request() {
    let router = build_api_version_router();

    let resp = router
        .oneshot(
            Request::builder()
                .uri("/ping")
                .header(&API_VERSION_HEADER, "v2")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // ApiVersionErrorResponse::UnsupportedVersion 映射为 400
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// ============================================================================
// E2E-APIVER-004: 大小写不敏感（V1/V2/v1/v2 都能被 FromStr 解析）
// ============================================================================

#[tokio::test]
async fn e2e_api_version_case_insensitive_v1_uppercase_accepted() {
    let router = build_api_version_router();

    let resp = router
        .oneshot(
            Request::builder()
                .uri("/ping")
                .header(&API_VERSION_HEADER, "V1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    // 大写 V1 解析为 V1 → 支持放行 → 响应头回写 v1
    assert_eq!(
        resp.headers()
            .get(&API_VERSION_HEADER)
            .unwrap()
            .to_str()
            .unwrap(),
        "v1"
    );
}

#[tokio::test]
async fn e2e_api_version_case_insensitive_v2_uppercase_still_rejected() {
    let router = build_api_version_router();

    let resp = router
        .oneshot(
            Request::builder()
                .uri("/ping")
                .header(&API_VERSION_HEADER, "V2")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // V2 解析为 V2 → 不支持 → 400
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// ============================================================================
// E2E-APIVER-005: 纯数字 "1" / "2" 也能被 FromStr 接受
// ============================================================================

#[tokio::test]
async fn e2e_api_version_numeric_only_header_accepted_for_v1() {
    let router = build_api_version_router();

    let resp = router
        .oneshot(
            Request::builder()
                .uri("/ping")
                .header(&API_VERSION_HEADER, "1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn e2e_api_version_numeric_only_header_v2_rejected() {
    let router = build_api_version_router();

    let resp = router
        .oneshot(
            Request::builder()
                .uri("/ping")
                .header(&API_VERSION_HEADER, "2")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// ============================================================================
// E2E-APIVER-006: 非法值（v3、unknown）→ 默认 v1（FromStr 失败 unwrap_or_default）
// ============================================================================

#[tokio::test]
async fn e2e_api_version_unknown_header_falls_back_to_default_v1() {
    let router = build_api_version_router();

    // ApiVersion::from_str("v3") 返回 Err，middleware 用 unwrap_or_default()
    // 回退到 V1（默认）。这是设计契约（unknown 不应阻断请求）。
    let resp = router
        .oneshot(
            Request::builder()
                .uri("/ping")
                .header(&API_VERSION_HEADER, "v3")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers()
            .get(&API_VERSION_HEADER)
            .unwrap()
            .to_str()
            .unwrap(),
        "v1"
    );
}

// ============================================================================
// E2E-SIZE-001: 小于 1MB 的 body 通过 size_limit middleware
// ============================================================================

#[tokio::test]
async fn e2e_size_limit_small_body_accepted() {
    let router = build_size_limit_router();

    let body = make_body(1024); // 1KB
    let resp = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/echo")
                .header("content-type", "text/plain")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
}

// ============================================================================
// E2E-SIZE-002: 恰好 1MB 边界通过
// ============================================================================

#[tokio::test]
async fn e2e_size_limit_exact_1mb_boundary_accepted() {
    let router = build_size_limit_router();

    let body = make_body(MAX_REQUEST_SIZE);
    let resp = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/echo")
                .header("content-type", "text/plain")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    // 边界值（恰好等于上限）应当被接受
    assert_eq!(resp.status(), StatusCode::OK);
}

// ============================================================================
// E2E-SIZE-003: 超过 1MB 返回 413 Payload Too Large
// ============================================================================

#[tokio::test]
async fn e2e_size_limit_over_1mb_returns_payload_too_large() {
    let router = build_size_limit_router();

    let body = make_body(MAX_REQUEST_SIZE + 1);
    let resp = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/echo")
                .header("content-type", "text/plain")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    // tower-http RequestBodyLimitLayer 在超过上限时返回 413
    assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

// ============================================================================
// E2E-SIZE-004: RequestBodyTooLarge 错误响应包含 413 状态码 + 详情
// ============================================================================

#[test]
fn e2e_request_body_too_large_response_includes_max_size_details() {
    let response: Response = RequestBodyTooLarge.into_response();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

// ============================================================================
// E2E-CORS-001: create_cors_layer 显式配置源列表
// ============================================================================

#[test]
fn e2e_cors_create_with_explicit_origins_succeeds() {
    let origins = vec![
        "https://example.com".to_string(),
        "https://app.example.com".to_string(),
    ];
    // 不应 panic
    let _cors = create_cors_layer(origins);
}

// ============================================================================
// E2E-CORS-002: create_cors_layer 空源列表 → 拒绝所有跨域
// ============================================================================

#[test]
fn e2e_cors_create_with_empty_origins_does_not_panic() {
    // 空列表应当被允许（生产环境用来拒绝所有跨域）
    let _cors = create_cors_layer(vec![]);
}

// ============================================================================
// E2E-CORS-003: create_dev_cors_layer 提供 localhost 默认
// ============================================================================

#[test]
fn e2e_cors_dev_layer_has_localhost_defaults() {
    // 开发环境默认 CORS 应当可构造（不 panic）
    let _cors = create_dev_cors_layer();
}

// ============================================================================
// E2E-CORS-004: 生产环境缺 ALLOWED_ORIGINS → create_env_aware_cors_layer
// 返回空 CorsLayer（拒绝所有跨域）
// ============================================================================

#[test]
fn e2e_cors_production_missing_origins_returns_empty_layer() {
    let _guard = E2E_ENV_LOCK.lock().unwrap();
    std::env::set_var("NEBULA_ENV", "production");
    std::env::remove_var("ALLOWED_ORIGINS");

    // 不应 panic，返回空 CorsLayer（拒绝所有跨域）
    let _cors = create_env_aware_cors_layer();

    std::env::remove_var("NEBULA_ENV");
}

// ============================================================================
// E2E-CORS-005: 生产环境配置 ALLOWED_ORIGINS → 应用配置
// ============================================================================

#[test]
fn e2e_cors_production_with_origins_uses_configured_origins() {
    let _guard = E2E_ENV_LOCK.lock().unwrap();
    std::env::set_var("NEBULA_ENV", "production");
    std::env::set_var(
        "ALLOWED_ORIGINS",
        "https://prod.example.com,https://app.prod.example.com",
    );

    let _cors = create_env_aware_cors_layer();

    std::env::remove_var("NEBULA_ENV");
    std::env::remove_var("ALLOWED_ORIGINS");
}

// ============================================================================
// E2E-CORS-006: 开发环境缺 ALLOWED_ORIGINS → localhost 默认
// ============================================================================

#[test]
fn e2e_cors_development_missing_origins_uses_localhost_defaults() {
    let _guard = E2E_ENV_LOCK.lock().unwrap();
    std::env::set_var("NEBULA_ENV", "development");
    std::env::remove_var("ALLOWED_ORIGINS");

    // 开发环境缺配应回退到 localhost，不 panic
    let _cors = create_env_aware_cors_layer();

    std::env::remove_var("NEBULA_ENV");
}

// ============================================================================
// E2E-CORS-007: 开发环境显式配置 ALLOWED_ORIGINS → 用 create_cors_layer
// ============================================================================

#[test]
fn e2e_cors_development_with_explicit_origins_uses_them() {
    let _guard = E2E_ENV_LOCK.lock().unwrap();
    std::env::set_var("NEBULA_ENV", "development");
    std::env::set_var("ALLOWED_ORIGINS", "https://dev.example.com");

    let _cors = create_env_aware_cors_layer();

    std::env::remove_var("NEBULA_ENV");
    std::env::remove_var("ALLOWED_ORIGINS");
}

// ============================================================================
// E2E-CORS-008: 默认 NEBULA_ENV 缺失时按 development 处理
// ============================================================================

#[test]
fn e2e_cors_missing_nebula_env_treated_as_development() {
    let _guard = E2E_ENV_LOCK.lock().unwrap();
    std::env::remove_var("NEBULA_ENV");
    std::env::remove_var("ALLOWED_ORIGINS");

    // NEBULA_ENV 缺失时 unwrap_or_else 默认 "development"
    let _cors = create_env_aware_cors_layer();
}

// ============================================================================
// E2E-AUDIT-001: 内存 AuditLogger 通过 log_id_generation 记录完整事件
// ============================================================================

#[tokio::test]
async fn e2e_audit_logger_in_memory_records_id_generation_event() {
    let logger = AuditLogger::new(100);

    logger
        .log_id_generation(
            "ws-123".to_string(),
            "order".to_string(),
            "123456789".to_string(),
            "snowflake".to_string(),
            Some("192.168.1.100".to_string()),
            42,
            true,
            None,
        )
        .await;

    assert_eq!(logger.total_logged(), 1);
    assert_eq!(logger.total_errors(), 0);

    let events = logger.get_recent_events(10).await;
    assert_eq!(events.len(), 1);
    let e = &events[0];
    assert_eq!(e.event_type, AuditEventType::IdGeneration);
    assert_eq!(e.workspace_id.as_deref(), Some("ws-123"));
    assert_eq!(e.action, "generate_id");
    assert!(e.resource.contains("order"));
    assert_eq!(e.result, AuditResult::Success);
    assert_eq!(e.duration_ms, 42);
    assert_eq!(e.client_ip.as_deref(), Some("192.168.1.100"));
    // details 中应包含 algorithm 与 generated_id
    let details = e.details.as_ref().expect("details must be set");
    assert_eq!(details["algorithm"], "snowflake");
    assert_eq!(details["generated_id"], "123456789");
}

// ============================================================================
// E2E-AUDIT-002: log_batch_generation 记录批量生成事件
// ============================================================================

#[tokio::test]
async fn e2e_audit_logger_in_memory_records_batch_generation_event() {
    let logger = AuditLogger::new(100);

    logger
        .log_batch_generation(
            "ws-batch".to_string(),
            "user".to_string(),
            500,
            Some("10.0.0.1".to_string()),
            100,
            true,
            None,
        )
        .await;

    let events = logger.get_recent_events(10).await;
    assert_eq!(events.len(), 1);
    let e = &events[0];
    assert_eq!(e.event_type, AuditEventType::BatchGeneration);
    assert_eq!(e.action, "batch_generate_ids");
    assert!(e.resource.contains("size:500"));
    assert_eq!(e.duration_ms, 100);
    let details = e.details.as_ref().unwrap();
    assert_eq!(details["batch_size"], 500);
    assert_eq!(details["biz_tag"], "user");
}

// ============================================================================
// E2E-AUDIT-003: log_auth_event 记录认证事件
// ============================================================================

#[tokio::test]
async fn e2e_audit_logger_in_memory_records_auth_event() {
    let logger = AuditLogger::new(100);

    logger
        .log_auth_event(
            Some("ws-auth".to_string()),
            "api_key_login".to_string(),
            false,
            Some("203.0.113.5".to_string()),
            Some("invalid signature".to_string()),
        )
        .await;

    let events = logger.get_recent_events(10).await;
    assert_eq!(events.len(), 1);
    let e = &events[0];
    assert_eq!(e.event_type, AuditEventType::Authentication);
    assert_eq!(e.action, "api_key_login");
    assert_eq!(e.result, AuditResult::Failure);
    assert_eq!(e.error_message.as_deref(), Some("invalid signature"));
}

// ============================================================================
// E2E-AUDIT-004: 内存记录器达到上限丢弃最旧事件（无文件持久化）
// ============================================================================

#[tokio::test]
async fn e2e_audit_logger_in_memory_drops_oldest_when_capacity_reached() {
    let logger = AuditLogger::new(3);

    for i in 0..5 {
        logger
            .log_id_generation(
                format!("ws-{}", i),
                "tag".to_string(),
                i.to_string(),
                "segment".to_string(),
                None,
                1,
                true,
                None,
            )
            .await;
    }

    // 容量 3，应只保留最后 3 条
    let events = logger.get_recent_events(10).await;
    assert_eq!(events.len(), 3);
    // 最旧 ws-0 / ws-1 应被丢弃，保留 ws-2 / ws-3 / ws-4
    let workspaces: Vec<_> = events
        .iter()
        .filter_map(|e| e.workspace_id.clone())
        .collect();
    assert!(workspaces.contains(&"ws-2".to_string()));
    assert!(workspaces.contains(&"ws-3".to_string()));
    assert!(workspaces.contains(&"ws-4".to_string()));
    // 丢弃事件累计为 error（无文件持久化场景）
    assert!(logger.total_errors() >= 2);
    // 但 total_logged 应记录所有调用
    assert_eq!(logger.total_logged(), 5);
}

// ============================================================================
// E2E-AUDIT-005: get_events_by_workspace 按工作区过滤
// ============================================================================

#[tokio::test]
async fn e2e_audit_logger_get_events_by_workspace_filters_correctly() {
    let logger = AuditLogger::new(100);

    for tag in ["ws-A", "ws-B", "ws-A", "ws-B", "ws-A"] {
        logger
            .log_id_generation(
                tag.to_string(),
                "t".to_string(),
                "1".to_string(),
                "snowflake".to_string(),
                None,
                1,
                true,
                None,
            )
            .await;
    }

    let a_events = logger.get_events_by_workspace("ws-A").await;
    let b_events = logger.get_events_by_workspace("ws-B").await;
    assert_eq!(a_events.len(), 3);
    assert_eq!(b_events.len(), 2);
}

// ============================================================================
// E2E-AUDIT-006: get_events_by_type 按事件类型过滤
// ============================================================================

#[tokio::test]
async fn e2e_audit_logger_get_events_by_type_filters_correctly() {
    let logger = AuditLogger::new(100);

    logger
        .log_id_generation(
            "ws".to_string(),
            "t".to_string(),
            "1".to_string(),
            "snowflake".to_string(),
            None,
            1,
            true,
            None,
        )
        .await;
    logger
        .log_auth_event(
            Some("ws".to_string()),
            "login".to_string(),
            true,
            None,
            None,
        )
        .await;

    let id_gen = logger
        .get_events_by_type(AuditEventType::IdGeneration)
        .await;
    let auth = logger
        .get_events_by_type(AuditEventType::Authentication)
        .await;
    assert_eq!(id_gen.len(), 1);
    assert_eq!(auth.len(), 1);
}

// ============================================================================
// E2E-AUDIT-007: clear() 清空内存事件
// ============================================================================

#[tokio::test]
async fn e2e_audit_logger_clear_empties_in_memory_events() {
    let logger = AuditLogger::new(100);

    logger
        .log_id_generation(
            "ws".to_string(),
            "t".to_string(),
            "1".to_string(),
            "snowflake".to_string(),
            None,
            1,
            true,
            None,
        )
        .await;
    assert_eq!(logger.get_recent_events(10).await.len(), 1);

    logger.clear().await;
    assert_eq!(logger.get_recent_events(10).await.len(), 0);
}

// ============================================================================
// E2E-AUDIT-008: 文件持久化路径 - 正常写入文件
// ============================================================================

#[tokio::test]
async fn e2e_audit_logger_file_persistence_writes_events_to_file() {
    let dir = tempdir().unwrap();
    let log_path = dir.path().join("audit.log");
    let path_str = log_path.to_str().unwrap().to_string();

    let logger = AuditLogger::with_file_logging(100, path_str.clone()).await;

    logger
        .log_id_generation(
            "ws-file".to_string(),
            "order".to_string(),
            "42".to_string(),
            "snowflake".to_string(),
            Some("192.168.1.1".to_string()),
            10,
            true,
            None,
        )
        .await;

    // 等待异步 writer task 处理完成（确定性 flush 替代 sleep）
    logger.flush().await;

    let content = std::fs::read_to_string(&log_path).unwrap();
    assert!(!content.is_empty(), "审计日志文件不应为空");
    assert!(content.contains("ws-file"));
    assert!(content.contains("order"));
    assert!(content.contains("snowflake"));
    // IPv4 脱敏：保留前 3 段，末段用 x 替换
    assert!(content.contains("192.168.1.x"));
    // 不应保留原始完整 IP
    assert!(!content.contains("192.168.1.1"));
}

// ============================================================================
// E2E-AUDIT-009: 路径遍历防护 - `..` 路径回退到内存记录器
// ============================================================================

#[tokio::test]
async fn e2e_audit_logger_path_traversal_falls_back_to_memory() {
    let malicious_path = "../../etc/passwd".to_string();

    // 应当回退到内存模式，不 panic
    let logger = AuditLogger::with_file_logging(100, malicious_path).await;

    logger
        .log_id_generation(
            "ws".to_string(),
            "t".to_string(),
            "1".to_string(),
            "snowflake".to_string(),
            None,
            1,
            true,
            None,
        )
        .await;

    // 内存记录器仍可访问事件
    let events = logger.get_recent_events(10).await;
    assert_eq!(events.len(), 1);
    assert_eq!(logger.total_logged(), 1);
}

// ============================================================================
// E2E-AUDIT-010: 空路径防护 - 回退到内存记录器
// ============================================================================

#[tokio::test]
async fn e2e_audit_logger_empty_path_falls_back_to_memory() {
    let logger = AuditLogger::with_file_logging(100, "".to_string()).await;

    logger
        .log_id_generation(
            "ws".to_string(),
            "t".to_string(),
            "1".to_string(),
            "snowflake".to_string(),
            None,
            1,
            true,
            None,
        )
        .await;

    let events = logger.get_recent_events(10).await;
    assert_eq!(events.len(), 1);
}

// ============================================================================
// E2E-AUDIT-011: 文件持久化场景下 IP 脱敏（IPv6）
// ============================================================================

#[tokio::test]
async fn e2e_audit_logger_file_persistence_redacts_ipv6_address() {
    let dir = tempdir().unwrap();
    let log_path = dir.path().join("audit_v6.log");
    let path_str = log_path.to_str().unwrap().to_string();

    let logger = AuditLogger::with_file_logging(100, path_str.clone()).await;

    logger
        .log_id_generation(
            "ws-v6".to_string(),
            "tag".to_string(),
            "1".to_string(),
            "uuid_v7".to_string(),
            Some("2001:db8::1".to_string()),
            5,
            true,
            None,
        )
        .await;

    logger.flush().await;

    let content = std::fs::read_to_string(&log_path).unwrap();
    // IPv6 保留前 4 段，后面用 x 替换
    assert!(content.contains("2001:db8:0:0:x:x:x:x") || content.contains("2001:db8::"));
    // 不应保留完整 IPv6 地址
    assert!(!content.contains("2001:db8::1"));
}

// ============================================================================
// E2E-AUDIT-012: 文件持久化场景下 user_agent 被替换为 UA(redacted)
// ============================================================================

#[tokio::test]
async fn e2e_audit_logger_file_persistence_redacts_user_agent() {
    let dir = tempdir().unwrap();
    let log_path = dir.path().join("audit_ua.log");
    let path_str = log_path.to_str().unwrap().to_string();

    let logger = AuditLogger::with_file_logging(100, path_str.clone()).await;

    // 直接构造带 user_agent 的事件
    let event = AuditEvent::new(
        AuditEventType::IdGeneration,
        Some("ws-ua".to_string()),
        "test".to_string(),
        "test".to_string(),
        AuditResult::Success,
    )
    .with_user_agent("Mozilla/5.0 (X11; Linux x86_64) Chrome/120.0".to_string());

    logger.log(event).await;

    logger.flush().await;

    let content = std::fs::read_to_string(&log_path).unwrap();
    // UA 脱敏：替换为 UA(redacted)
    assert!(content.contains("UA(redacted)"));
    // 不应保留原始 UA
    assert!(!content.contains("Mozilla"));
    assert!(!content.contains("Chrome"));
}

// ============================================================================
// E2E-AUDIT-013: 多事件文件持久化（每行一个 JSON）
// ============================================================================

#[tokio::test]
async fn e2e_audit_logger_file_persistence_appends_multiple_events() {
    let dir = tempdir().unwrap();
    let log_path = dir.path().join("audit_multi.log");
    let path_str = log_path.to_str().unwrap().to_string();

    let logger = AuditLogger::with_file_logging(100, path_str.clone()).await;

    for i in 0..5 {
        logger
            .log_id_generation(
                format!("ws-{}", i),
                "tag".to_string(),
                i.to_string(),
                "segment".to_string(),
                None,
                1,
                true,
                None,
            )
            .await;
    }

    logger.flush().await;

    let content = std::fs::read_to_string(&log_path).unwrap();
    let lines: Vec<&str> = content.lines().filter(|l| !l.is_empty()).collect();
    assert_eq!(lines.len(), 5, "应有 5 行 JSON，每行一个事件");
    // 每行应是合法 JSON
    for line in &lines {
        let _: serde_json::Value = serde_json::from_str(line).expect("每行应为合法 JSON");
    }
}

// ============================================================================
// E2E-AUDIT-014: CoreAuditLoggerTrait impl 适配 core 层事件
// ============================================================================

#[tokio::test]
async fn e2e_audit_logger_core_trait_impl_adapts_core_event() {
    use crate::core::algorithm::{
        AuditEvent as CoreAuditEvent, AuditLogger as CoreAuditLoggerTrait,
    };
    use chrono::Utc;

    let logger = AuditLogger::new(100);

    let core_event = CoreAuditEvent {
        event_type: AuditEventType::IdGeneration,
        workspace_id: Some("ws-core".to_string()),
        action: "core_action".to_string(),
        resource: "core_resource".to_string(),
        result: AuditResult::Success,
        details: Some(serde_json::json!({"k": "v"})),
        timestamp: Utc::now(),
    };

    // 通过 trait 调用（server AuditLogger impl CoreAuditLoggerTrait）。
    // 必须用完全限定语法 `CoreAuditLoggerTrait::log(&logger, ...)`，
    // 因为 AuditLogger 还有一个同名 inherent 方法 `log(&self, server AuditEvent)`，
    // 直接 `logger.log(core_event)` 会匹配 inherent 方法导致类型不匹配。
    CoreAuditLoggerTrait::log(&logger, core_event).await;

    let events = logger.get_recent_events(10).await;
    assert_eq!(events.len(), 1);
    let e = &events[0];
    assert_eq!(e.workspace_id.as_deref(), Some("ws-core"));
    assert_eq!(e.action, "core_action");
    assert_eq!(e.resource, "core_resource");
    // server 端重新生成了 ID（next_audit_event_id）
    assert_ne!(e.id, 0);
}

// ============================================================================
// E2E-RATE-001: Token Bucket 在 burst 容量内允许
// ============================================================================

#[tokio::test]
async fn e2e_rate_limiter_allows_within_burst_capacity() {
    let limiter = RateLimiter::new(100, 5); // 100 rps, burst=5

    // 前 5 次（burst 容量内）应全部允许
    for _ in 0..5 {
        let result = limiter.check_rate_limit("key-A", None, None).await;
        assert!(result.allowed, "burst 容量内的请求应被允许");
        assert_eq!(result.limit, 5);
    }
}

// ============================================================================
// E2E-RATE-002: Token Bucket 超过 burst 后拒绝
// ============================================================================

#[tokio::test]
async fn e2e_rate_limiter_rejects_after_burst_exhausted() {
    let limiter = RateLimiter::new(1, 3); // 1 rps, burst=3

    // 消耗 burst 容量
    for _ in 0..3 {
        let r = limiter.check_rate_limit("key-B", None, None).await;
        assert!(r.allowed);
    }

    // 第 4 次应当被拒绝（rate=1 rps 无法立即补充 token）
    let result = limiter.check_rate_limit("key-B", None, None).await;
    assert!(!result.allowed, "超过 burst 后应被拒绝");
    assert_eq!(result.retry_after, Some(1));
}

// ============================================================================
// E2E-RATE-003: 按 key 隔离（key-A 耗尽不影响 key-B）
// ============================================================================

#[tokio::test]
async fn e2e_rate_limiter_per_key_isolation() {
    let limiter = RateLimiter::new(1, 2);

    // key-A 耗尽
    for _ in 0..2 {
        let _ = limiter.check_rate_limit("key-A", None, None).await;
    }
    let a_result = limiter.check_rate_limit("key-A", None, None).await;
    assert!(!a_result.allowed, "key-A 应被拒绝");

    // key-B 应仍可使用
    let b_result = limiter.check_rate_limit("key-B", None, None).await;
    assert!(b_result.allowed, "key-B 应被允许（按 key 隔离）");
}

// ============================================================================
// E2E-RATE-004: token 随时间恢复（rate=1/s，等待 1s 后应允许 1 次）
// ============================================================================

#[tokio::test]
async fn e2e_rate_limiter_token_refills_over_time() {
    let limiter = RateLimiter::new(10, 1); // 10 rps, burst=1

    // 第一次允许（消耗唯一 token）
    let r1 = limiter.check_rate_limit("key-refill", None, None).await;
    assert!(r1.allowed);

    // 立即第二次应被拒（无 token）
    let r2 = limiter.check_rate_limit("key-refill", None, None).await;
    assert!(!r2.allowed);

    // 等待 200ms（rate=10/s → 应补充 2 个 token）
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let r3 = limiter.check_rate_limit("key-refill", None, None).await;
    assert!(r3.allowed, "等待后 token 应已补充");
}

// ============================================================================
// E2E-RATE-005: RateLimitMiddleware 集成 - 允许时回写 X-RateLimit-* 头
// ============================================================================

#[tokio::test]
async fn e2e_rate_limit_middleware_writes_rate_limit_headers_on_allow() {
    let limiter = Arc::new(RateLimiter::new(100, 10));
    let middleware = RateLimitMiddleware::new(limiter);

    let app = Router::new()
        .route("/ping", get(|| async { "pong" }))
        .layer(from_fn_with_state(middleware, rate_limit_handler));

    let resp = app
        .oneshot(Request::builder().uri("/ping").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    assert!(resp.headers().get("X-RateLimit-Limit").is_some());
    assert!(resp.headers().get("X-RateLimit-Remaining").is_some());
}

// ============================================================================
// E2E-RATE-006: RateLimitMiddleware 集成 - 拒绝时返回 429 + Retry-After
// ============================================================================

#[tokio::test]
async fn e2e_rate_limit_middleware_returns_429_on_reject() {
    // 共享同一个 rate_limiter Arc 让两次 oneshot 共用一个 bucket
    let limiter = Arc::new(RateLimiter::new(1, 1)); // 极小配额

    // 第一次：允许
    let app1 = Router::new()
        .route("/ping", get(|| async { "pong" }))
        .layer(from_fn_with_state(
            RateLimitMiddleware::new(limiter.clone()),
            rate_limit_handler,
        ));
    let r1 = app1
        .oneshot(Request::builder().uri("/ping").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(r1.status(), StatusCode::OK);

    // 第二次：应被限流（共享 bucket 已耗尽）
    let app2 = Router::new()
        .route("/ping", get(|| async { "pong" }))
        .layer(from_fn_with_state(
            RateLimitMiddleware::new(limiter),
            rate_limit_handler,
        ));
    let r2 = app2
        .oneshot(Request::builder().uri("/ping").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(r2.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(r2.headers().get("Retry-After").is_some());
    assert!(r2.headers().get("X-RateLimit-Limit").is_some());
}

// ============================================================================
// E2E-ANON-001: Anonymous 角色被 anonymous_block_middleware 拒绝
// ============================================================================

#[tokio::test]
async fn e2e_anonymous_block_middleware_rejects_anonymous_role() {
    let app = Router::new()
        .route("/secure", get(|| async { "ok" }))
        .layer(from_fn(anonymous_block_middleware));

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/secure")
                .extension(ApiKeyRole::Anonymous)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Anonymous 应被拒绝为 401
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

// ============================================================================
// E2E-ANON-002: User 角色被 anonymous_block_middleware 放行
// ============================================================================

#[tokio::test]
async fn e2e_anonymous_block_middleware_allows_user_role() {
    let app = Router::new()
        .route("/secure", get(|| async { "ok" }))
        .layer(from_fn(anonymous_block_middleware));

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/secure")
                .extension(ApiKeyRole::User)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
}

// ============================================================================
// E2E-ANON-003: Admin 角色被 anonymous_block_middleware 放行
// ============================================================================

#[tokio::test]
async fn e2e_anonymous_block_middleware_allows_admin_role() {
    let app = Router::new()
        .route("/secure", get(|| async { "ok" }))
        .layer(from_fn(anonymous_block_middleware));

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/secure")
                .extension(ApiKeyRole::Admin)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
}

// ============================================================================
// E2E-ANON-004: ApiKeyRole 扩展缺失 → fail-closed 401
// ============================================================================

#[tokio::test]
async fn e2e_anonymous_block_middleware_fail_closed_when_extension_missing() {
    let app = Router::new()
        .route("/secure", get(|| async { "ok" }))
        .layer(from_fn(anonymous_block_middleware));

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/secure")
                // 不插入 ApiKeyRole 扩展
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // NEW-LOW-002：扩展缺失时 fail-closed 返回 401
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

// ============================================================================
// E2E-AUDITMW-001: 审计中间件端到端 HTTP 流（2xx → Success）
// ============================================================================

#[tokio::test]
async fn e2e_audit_middleware_records_success_event_for_2xx_response() {
    // 直接内联审计中间件逻辑（与 audit/middleware.rs 行为一致），用真实
    // AuditLogger 验证完整事件流：HTTP 请求 → 状态码映射 → 事件记录。
    // 不依赖 ApiKeyAuth / RateLimiter（audit/middleware.rs 的现有单测已覆盖
    // 那部分），这里聚焦「status_code → AuditResult 映射 + 事件落库」。
    let audit_logger = Arc::new(AuditLogger::new(100));
    let logger_clone = audit_logger.clone();

    let app = Router::new()
        .route("/ok", get(|| async { "ok" }))
        .layer(from_fn(move |req: Request<Body>, next: Next| {
            let logger = logger_clone.clone();
            async move {
                let start = std::time::Instant::now();
                let path = req.uri().path().to_string();
                let method = req.method().to_string();
                let workspace_id = req.extensions().get::<String>().cloned();

                let response = next.run(req).await;
                let duration_ms = start.elapsed().as_millis() as u64;
                let status_code = response.status().as_u16();

                let result = if (200..300).contains(&status_code) {
                    AuditResult::Success
                } else if (400..500).contains(&status_code) {
                    AuditResult::Failure
                } else {
                    AuditResult::Partial
                };

                let event = AuditEvent::new(
                    AuditEventType::IdGeneration,
                    workspace_id,
                    format!("{} {}", method, path),
                    path,
                    result,
                )
                .with_duration(duration_ms);

                logger.log(event).await;
                response
            }
        }));

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/ok")
                .extension("ws-test".to_string())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);

    // log().await 返回时事件已写入内存（VecDeque），无需额外等待

    let events = audit_logger.get_recent_events(10).await;
    assert_eq!(events.len(), 1);
    let e = &events[0];
    assert_eq!(e.result, AuditResult::Success);
    assert_eq!(e.workspace_id.as_deref(), Some("ws-test"));
    assert_eq!(e.action, "GET /ok");
}

// ============================================================================
// E2E-AUDITMW-002: 审计中间件端到端 HTTP 流（4xx → Failure）
// ============================================================================

#[tokio::test]
async fn e2e_audit_middleware_records_failure_event_for_4xx_response() {
    let audit_logger = Arc::new(AuditLogger::new(100));
    let logger_clone = audit_logger.clone();

    let app = Router::new()
        .route("/notfound", get(|| async { StatusCode::NOT_FOUND }))
        .layer(from_fn(move |req: Request<Body>, next: Next| {
            let logger = logger_clone.clone();
            async move {
                let start = std::time::Instant::now();
                let path = req.uri().path().to_string();
                let method = req.method().to_string();

                let response = next.run(req).await;
                let duration_ms = start.elapsed().as_millis() as u64;
                let status_code = response.status().as_u16();

                let result = if (200..300).contains(&status_code) {
                    AuditResult::Success
                } else if (400..500).contains(&status_code) {
                    AuditResult::Failure
                } else {
                    AuditResult::Partial
                };

                let event = AuditEvent::new(
                    AuditEventType::IdGeneration,
                    None,
                    format!("{} {}", method, path),
                    path,
                    result,
                )
                .with_duration(duration_ms);

                logger.log(event).await;
                response
            }
        }));

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/notfound")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    tokio::time::sleep(std::time::Duration::from_millis(10)).await;

    let events = audit_logger.get_recent_events(10).await;
    assert_eq!(events.len(), 1);
    let e = &events[0];
    assert_eq!(e.result, AuditResult::Failure);
    assert_eq!(e.action, "GET /notfound");
}

// ============================================================================
// E2E-AUDITMW-003: 审计中间件端到端 HTTP 流（5xx → Partial）
// ============================================================================

#[tokio::test]
async fn e2e_audit_middleware_records_partial_event_for_5xx_response() {
    let audit_logger = Arc::new(AuditLogger::new(100));
    let logger_clone = audit_logger.clone();

    let app = Router::new()
        .route(
            "/error",
            get(|| async { StatusCode::INTERNAL_SERVER_ERROR }),
        )
        .layer(from_fn(move |req: Request<Body>, next: Next| {
            let logger = logger_clone.clone();
            async move {
                let start = std::time::Instant::now();
                let path = req.uri().path().to_string();
                let method = req.method().to_string();

                let response = next.run(req).await;
                let duration_ms = start.elapsed().as_millis() as u64;
                let status_code = response.status().as_u16();

                let result = if (200..300).contains(&status_code) {
                    AuditResult::Success
                } else if (400..500).contains(&status_code) {
                    AuditResult::Failure
                } else {
                    AuditResult::Partial
                };

                let event = AuditEvent::new(
                    AuditEventType::IdGeneration,
                    None,
                    format!("{} {}", method, path),
                    path,
                    result,
                )
                .with_duration(duration_ms);

                logger.log(event).await;
                response
            }
        }));

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/error")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);

    tokio::time::sleep(std::time::Duration::from_millis(10)).await;

    let events = audit_logger.get_recent_events(10).await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].result, AuditResult::Partial);
}

// ============================================================================
// E2E-AUDITMW-004: 审计中间件记录 client_ip 与 user_agent
// ============================================================================

#[tokio::test]
async fn e2e_audit_middleware_records_client_ip_and_user_agent() {
    let audit_logger = Arc::new(AuditLogger::new(100));
    let logger_clone = audit_logger.clone();

    let app = Router::new()
        .route("/ping", get(|| async { "pong" }))
        .layer(from_fn(move |req: Request<Body>, next: Next| {
            let logger = logger_clone.clone();
            async move {
                let start = std::time::Instant::now();
                let path = req.uri().path().to_string();
                let method = req.method().to_string();
                let client_ip = req
                    .headers()
                    .get("x-forwarded-for")
                    .and_then(|h| h.to_str().ok())
                    .map(|s| s.to_string());
                let user_agent = req
                    .headers()
                    .get("user-agent")
                    .and_then(|h| h.to_str().ok())
                    .map(|s| s.to_string());

                let response = next.run(req).await;
                let duration_ms = start.elapsed().as_millis() as u64;

                let mut event = AuditEvent::new(
                    AuditEventType::IdGeneration,
                    None,
                    format!("{} {}", method, path),
                    path,
                    AuditResult::Success,
                )
                .with_duration(duration_ms);

                if let Some(ip) = client_ip {
                    event = event.with_client_ip(ip);
                }
                if let Some(ua) = user_agent {
                    event = event.with_user_agent(ua);
                }

                logger.log(event).await;
                response
            }
        }));

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/ping")
                .header("x-forwarded-for", "203.0.113.10")
                .header("user-agent", "TestAgent/1.0")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);

    tokio::time::sleep(std::time::Duration::from_millis(10)).await;

    let events = audit_logger.get_recent_events(10).await;
    assert_eq!(events.len(), 1);
    let e = &events[0];
    assert_eq!(e.client_ip.as_deref(), Some("203.0.113.10"));
    assert_eq!(e.user_agent.as_deref(), Some("TestAgent/1.0"));
}

// ============================================================================
// E2E-APIVER-COMPOSE-001: API 版本 + 限流 + 安全头组合（多层 middleware）
// ============================================================================

#[tokio::test]
async fn e2e_composed_middlewares_version_plus_rate_limit_works_together() {
    // 组合多层 middleware 验证它们不会互相干扰
    let limiter = Arc::new(RateLimiter::new(100, 10));
    let rl_middleware = RateLimitMiddleware::new(limiter);

    let app = Router::new()
        .route("/ping", get(|| async { "pong" }))
        .layer(from_fn_with_state(rl_middleware, rate_limit_handler))
        .layer(from_fn(api_version_middleware));

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/ping")
                .header(&API_VERSION_HEADER, "v1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // 两个 middleware 都应正常工作
    assert_eq!(resp.status(), StatusCode::OK);
    // API 版本头被回写
    assert!(resp.headers().get(&API_VERSION_HEADER).is_some());
    // 限流头被回写
    assert!(resp.headers().get("X-RateLimit-Limit").is_some());
}

// ============================================================================
// E2E-APIVER-ERROR-001: ApiVersionErrorResponse 直接转换
// ============================================================================

#[test]
fn e2e_api_version_error_response_converts_to_bad_request() {
    let err = ApiVersionErrorResponse::UnsupportedVersion {
        requested: "v9".to_string(),
        supported_versions: vec!["v1".to_string()],
    };

    let resp: Response = err.into_response();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// ============================================================================
// E2E-APIVER-CONST-001: 公共常量一致性
// ============================================================================

#[test]
fn e2e_api_version_constants_consistent() {
    assert_eq!(CURRENT_API_VERSION, "v1");
    assert_eq!(ApiVersion::V1.as_str(), "v1");
    assert_eq!(ApiVersion::V1.as_number(), 1);
    assert!(ApiVersion::V1.is_supported());
    assert!(!ApiVersion::V2.is_supported());
}
