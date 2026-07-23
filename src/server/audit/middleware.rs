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

use crate::server::audit::{AuditEventType, AuditLogger, AuditResult};
use crate::server::middleware::ApiKeyAuth;
use crate::server::rate_limit::RateLimiter;
use axum::body::Body;
use axum::http::Request;
use axum::response::Response;
use std::net::IpAddr;
use std::sync::Arc;
use tracing::debug;

#[derive(Clone)]
#[allow(dead_code)]
pub struct AuditMiddleware {
    audit_logger: Arc<AuditLogger>,
    auth: Arc<ApiKeyAuth>,
    rate_limiter: Arc<RateLimiter>,
    trusted_proxies: Vec<IpAddr>,
}

impl AuditMiddleware {
    pub fn new(
        audit_logger: Arc<AuditLogger>,
        auth: Arc<ApiKeyAuth>,
        rate_limiter: Arc<RateLimiter>,
    ) -> Self {
        Self {
            audit_logger,
            auth,
            rate_limiter,
            trusted_proxies: Vec::new(), // Default: no trusted proxies
        }
    }

    pub fn with_trusted_proxies(mut self, proxies: Vec<IpAddr>) -> Self {
        self.trusted_proxies = proxies;
        self
    }

    pub async fn audit_middleware(
        &self,
        req: Request<Body>,
        next: axum::middleware::Next,
    ) -> Response {
        let start = std::time::Instant::now();
        let path = req.uri().path().to_string();
        let method = req.method().to_string();
        let client_ip = get_client_ip(&req, &self.trusted_proxies);
        let user_agent = req
            .headers()
            .get("user-agent")
            .and_then(|h| h.to_str().ok())
            .map(|s| s.to_string());

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

        let action = format!("{} {}", method, path);

        let audit_event = crate::server::audit::AuditEvent::new(
            AuditEventType::IdGeneration,
            workspace_id.clone(),
            action,
            path.clone(),
            result,
        )
        .with_client_ip(client_ip.unwrap_or_default())
        .with_user_agent(user_agent.unwrap_or_default())
        .with_duration(duration_ms);

        self.audit_logger.log(audit_event).await;

        if let Some(ws_id) = workspace_id {
            debug!(
                workspace_id = ws_id,
                path = path,
                method = method,
                status = status_code,
                duration_ms = duration_ms,
                "{}",
                t!("log.server.audit.middleware.request_recorded")
            );
        }

        response
    }
}

fn get_client_ip(req: &Request<Body>, trusted_proxies: &[IpAddr]) -> Option<String> {
    // Phase 9 T043 (LOW L3) — delegate to the single shared implementation
    // in `server::middleware::utils`. Previously this was a duplicate copy
    // of the same logic; now the audit/rate_limit/api_key_auth middleware
    // all share one source of truth.
    crate::server::middleware::utils::get_client_ip(req, trusted_proxies)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::middleware::ApiKeyAuth;
    use crate::server::rate_limit::RateLimiter;
    use axum::body::Body;
    use axum::http::Request;

    #[tokio::test]
    async fn test_audit_middleware_creation() {
        use crate::core::database::{
            ApiKeyInfo, ApiKeyRepository, ApiKeyResponse, ApiKeyRole as CoreApiKeyRole,
            ApiKeyWithSecret, CreateApiKeyRequest,
        };
        use crate::core::types::Result;
        use async_trait::async_trait;
        use uuid::Uuid;

        #[derive(Clone)]
        struct MockApiKeyRepo;

        #[async_trait]
        impl ApiKeyRepository for MockApiKeyRepo {
            async fn create_api_key(
                &self,
                _request: &CreateApiKeyRequest,
            ) -> Result<ApiKeyWithSecret> {
                Ok(ApiKeyWithSecret {
                    key: ApiKeyResponse {
                        id: Uuid::new_v4(),
                        key_id: "mock_key_id".to_string(),
                        key_prefix: "nino_".to_string(),
                        name: "Mock Key".to_string(),
                        description: None,
                        role: CoreApiKeyRole::User,
                        rate_limit: 10000,
                        enabled: true,
                        expires_at: None,
                        created_at: chrono::Utc::now().naive_utc(),
                    },
                    key_secret: "mock_secret".to_string(),
                })
            }

            async fn get_api_key_by_id(&self, _key_id: &str) -> Result<Option<ApiKeyInfo>> {
                Ok(None)
            }

            async fn validate_api_key(
                &self,
                _key_id: &str,
                _key_secret: &str,
            ) -> Result<Option<(Option<Uuid>, crate::core::database::ApiKeyRole)>> {
                Ok(None)
            }

            async fn list_api_keys(
                &self,
                _workspace_id: Uuid,
                _limit: Option<u32>,
                _offset: Option<u32>,
            ) -> Result<Vec<ApiKeyInfo>> {
                Ok(vec![])
            }

            async fn delete_api_key(&self, _id: Uuid) -> Result<()> {
                Ok(())
            }

            async fn revoke_api_key(&self, _id: Uuid) -> Result<()> {
                Ok(())
            }

            async fn update_last_used(&self, _key: Uuid) -> Result<()> {
                Ok(())
            }

            async fn get_admin_api_key(&self, _workspace_id: Uuid) -> Result<Option<ApiKeyInfo>> {
                Ok(None)
            }

            async fn count_api_keys(&self, _workspace_id: Uuid) -> Result<u64> {
                Ok(0)
            }

            async fn rotate_api_key(
                &self,
                _key_id: &str,
                _grace_period_seconds: u64,
            ) -> Result<ApiKeyWithSecret> {
                Err(crate::core::CoreError::InternalError(
                    "rotate_api_key not implemented in mock".to_string(),
                ))
            }

            async fn get_keys_older_than(
                &self,
                _age_threshold_days: i64,
            ) -> Result<Vec<ApiKeyInfo>> {
                Ok(vec![])
            }
        }

        let audit_logger = Arc::new(AuditLogger::new(100));
        let auth = Arc::new(ApiKeyAuth::new(Arc::new(MockApiKeyRepo), true));
        let rate_limiter = Arc::new(RateLimiter::new(10000, 100));

        let middleware = AuditMiddleware::new(audit_logger, auth, rate_limiter);
        assert!(middleware.audit_logger.get_total_logged() == 0);
    }

    #[tokio::test]
    async fn test_get_client_ip_from_header() {
        // Create a request with a connection IP
        let req = Request::builder()
            .extension(std::net::SocketAddr::from(([10, 0, 0, 1], 8080)))
            .header("x-forwarded-for", "192.168.1.1, 10.0.0.1")
            .body(Body::empty())
            .unwrap();

        // Add the connection IP to trusted proxies
        let trusted_proxies: Vec<std::net::IpAddr> = vec!["10.0.0.1".parse().unwrap()];
        let ip = get_client_ip(&req, &trusted_proxies);
        assert_eq!(ip, Some("192.168.1.1".to_string()));
    }

    #[tokio::test]
    async fn test_get_client_ip_from_real_ip() {
        // Create a request with a connection IP
        let req = Request::builder()
            .extension(std::net::SocketAddr::from(([10, 0, 0, 1], 8080)))
            .header("x-real-ip", "192.168.1.100")
            .body(Body::empty())
            .unwrap();

        // Add the connection IP to trusted proxies
        let trusted_proxies: Vec<std::net::IpAddr> = vec!["10.0.0.1".parse().unwrap()];
        let ip = get_client_ip(&req, &trusted_proxies);
        assert_eq!(ip, Some("192.168.1.100".to_string()));
    }

    #[tokio::test]
    async fn test_get_client_ip_fallback() {
        let req = Request::builder().body(Body::empty()).unwrap();

        let trusted_proxies: Vec<std::net::IpAddr> = Vec::new();
        let ip = get_client_ip(&req, &trusted_proxies);
        assert!(ip.is_none());
    }

    // ========== Helpers for audit_middleware tests ==========
    // 复用 mock repo 模式（与 test_audit_middleware_creation 中的实现一致），
    // 提取到模块级以便多个测试共享。

    use async_trait::async_trait;
    use axum::extract::State;
    use axum::http::StatusCode;
    use axum::middleware::from_fn_with_state;
    use axum::routing::get;
    use axum::Router;
    use tower::ServiceExt;
    use uuid::Uuid;

    #[derive(Clone)]
    struct MockRepo;

    #[async_trait]
    impl crate::core::database::ApiKeyRepository for MockRepo {
        async fn create_api_key(
            &self,
            _request: &crate::core::database::CreateApiKeyRequest,
        ) -> crate::core::types::Result<crate::core::database::ApiKeyWithSecret> {
            Ok(crate::core::database::ApiKeyWithSecret {
                key: crate::core::database::ApiKeyResponse {
                    id: Uuid::new_v4(),
                    key_id: "mock_key_id".to_string(),
                    key_prefix: "nino_".to_string(),
                    name: "Mock Key".to_string(),
                    description: None,
                    role: crate::core::database::ApiKeyRole::User,
                    rate_limit: 10000,
                    enabled: true,
                    expires_at: None,
                    created_at: chrono::Utc::now().naive_utc(),
                },
                key_secret: "mock_secret".to_string(),
            })
        }

        async fn get_api_key_by_id(
            &self,
            _key_id: &str,
        ) -> crate::core::types::Result<Option<crate::core::database::ApiKeyInfo>> {
            Ok(None)
        }

        async fn validate_api_key(
            &self,
            _key_id: &str,
            _key_secret: &str,
        ) -> crate::core::types::Result<Option<(Option<Uuid>, crate::core::database::ApiKeyRole)>>
        {
            Ok(None)
        }

        async fn list_api_keys(
            &self,
            _workspace_id: Uuid,
            _limit: Option<u32>,
            _offset: Option<u32>,
        ) -> crate::core::types::Result<Vec<crate::core::database::ApiKeyInfo>> {
            Ok(vec![])
        }

        async fn delete_api_key(&self, _id: Uuid) -> crate::core::types::Result<()> {
            Ok(())
        }

        async fn revoke_api_key(&self, _id: Uuid) -> crate::core::types::Result<()> {
            Ok(())
        }

        async fn update_last_used(&self, _key: Uuid) -> crate::core::types::Result<()> {
            Ok(())
        }

        async fn get_admin_api_key(
            &self,
            _workspace_id: Uuid,
        ) -> crate::core::types::Result<Option<crate::core::database::ApiKeyInfo>> {
            Ok(None)
        }

        async fn count_api_keys(&self, _workspace_id: Uuid) -> crate::core::types::Result<u64> {
            Ok(0)
        }

        async fn rotate_api_key(
            &self,
            _key_id: &str,
            _grace_period_seconds: u64,
        ) -> crate::core::types::Result<crate::core::database::ApiKeyWithSecret> {
            Err(crate::core::CoreError::InternalError(
                "rotate_api_key not implemented in mock".to_string(),
            ))
        }

        async fn get_keys_older_than(
            &self,
            _age_threshold_days: i64,
        ) -> crate::core::types::Result<Vec<crate::core::database::ApiKeyInfo>> {
            Ok(vec![])
        }
    }

    fn make_audit_middleware() -> (Arc<AuditMiddleware>, Arc<AuditLogger>) {
        let audit_logger = Arc::new(AuditLogger::new(100));
        let auth = Arc::new(ApiKeyAuth::new(Arc::new(MockRepo), true));
        let rate_limiter = Arc::new(RateLimiter::new(10000, 100));
        let mid = Arc::new(AuditMiddleware::new(
            audit_logger.clone(),
            auth,
            rate_limiter,
        ));
        (mid, audit_logger)
    }

    fn make_audit_middleware_with_proxies(
        proxies: Vec<IpAddr>,
    ) -> (Arc<AuditMiddleware>, Arc<AuditLogger>) {
        let audit_logger = Arc::new(AuditLogger::new(100));
        let auth = Arc::new(ApiKeyAuth::new(Arc::new(MockRepo), true));
        let rate_limiter = Arc::new(RateLimiter::new(10000, 100));
        let mid = Arc::new(
            AuditMiddleware::new(audit_logger.clone(), auth, rate_limiter)
                .with_trusted_proxies(proxies),
        );
        (mid, audit_logger)
    }

    /// 包装 AuditMiddleware::audit_middleware 方法为 axum middleware 函数。
    /// from_fn_with_state 要求自由函数，方法必须通过 wrapper 调用。
    async fn audit_middleware_fn(
        State(mid): State<Arc<AuditMiddleware>>,
        req: Request<Body>,
        next: axum::middleware::Next,
    ) -> Response {
        mid.audit_middleware(req, next).await
    }

    fn build_ok_router(mid: Arc<AuditMiddleware>) -> Router {
        Router::new()
            .route("/test", get(|| async { "ok" }))
            .layer(from_fn_with_state(mid, audit_middleware_fn))
    }

    fn build_status_router(mid: Arc<AuditMiddleware>, status: StatusCode) -> Router {
        Router::new()
            .route("/test", get(move || async move { status }))
            .layer(from_fn_with_state(mid, audit_middleware_fn))
    }

    fn make_test_request(method: &str, uri: &str) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .body(Body::empty())
            .unwrap()
    }

    // ========== audit_middleware result branch tests ==========

    #[tokio::test]
    async fn test_audit_middleware_2xx_response_logs_success_result() {
        let (mid, logger) = make_audit_middleware();
        let router = build_ok_router(mid);
        let resp = router
            .oneshot(make_test_request("GET", "/test"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let events = logger.get_recent_events(10).await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].result, AuditResult::Success);
    }

    #[tokio::test]
    async fn test_audit_middleware_4xx_response_logs_failure_result() {
        let (mid, logger) = make_audit_middleware();
        let router = build_status_router(mid, StatusCode::BAD_REQUEST);
        let resp = router
            .oneshot(make_test_request("GET", "/test"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let events = logger.get_recent_events(10).await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].result, AuditResult::Failure);
    }

    #[tokio::test]
    async fn test_audit_middleware_5xx_response_logs_partial_result() {
        let (mid, logger) = make_audit_middleware();
        let router = build_status_router(mid, StatusCode::INTERNAL_SERVER_ERROR);
        let resp = router
            .oneshot(make_test_request("GET", "/test"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let events = logger.get_recent_events(10).await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].result, AuditResult::Partial);
    }

    #[tokio::test]
    async fn test_audit_middleware_3xx_response_logs_partial_result() {
        // 3xx 不在 200..300 也不在 400..500，应归为 Partial
        let (mid, logger) = make_audit_middleware();
        let router = build_status_router(mid, StatusCode::MOVED_PERMANENTLY);
        let resp = router
            .oneshot(make_test_request("GET", "/test"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::MOVED_PERMANENTLY);
        let events = logger.get_recent_events(10).await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].result, AuditResult::Partial);
    }

    // ========== audit_middleware request enrichment tests ==========

    #[tokio::test]
    async fn test_audit_middleware_with_user_agent_records_it() {
        let (mid, logger) = make_audit_middleware();
        let router = build_ok_router(mid);
        let req = Request::builder()
            .method("GET")
            .uri("/test")
            .header("user-agent", "Mozilla/5.0 (test)")
            .body(Body::empty())
            .unwrap();
        let _resp = router.oneshot(req).await.unwrap();
        let events = logger.get_recent_events(10).await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].user_agent.as_deref(), Some("Mozilla/5.0 (test)"));
    }

    #[tokio::test]
    async fn test_audit_middleware_without_user_agent_records_empty_string() {
        // unwrap_or_default() 在 None 时返回空字符串
        let (mid, logger) = make_audit_middleware();
        let router = build_ok_router(mid);
        let _resp = router
            .oneshot(make_test_request("GET", "/test"))
            .await
            .unwrap();
        let events = logger.get_recent_events(10).await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].user_agent.as_deref(), Some(""));
    }

    #[tokio::test]
    async fn test_audit_middleware_with_workspace_id_extension_records_it() {
        let (mid, logger) = make_audit_middleware();
        let router = build_ok_router(mid);
        let req = Request::builder()
            .method("GET")
            .uri("/test")
            .extension("workspace-123".to_string())
            .body(Body::empty())
            .unwrap();
        let _resp = router.oneshot(req).await.unwrap();
        let events = logger.get_recent_events(10).await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].workspace_id.as_deref(), Some("workspace-123"));
    }

    #[tokio::test]
    async fn test_audit_middleware_without_workspace_id_records_none() {
        let (mid, logger) = make_audit_middleware();
        let router = build_ok_router(mid);
        let _resp = router
            .oneshot(make_test_request("GET", "/test"))
            .await
            .unwrap();
        let events = logger.get_recent_events(10).await;
        assert_eq!(events.len(), 1);
        assert!(events[0].workspace_id.is_none());
    }

    #[tokio::test]
    async fn test_audit_middleware_with_client_ip_via_xff_records_it() {
        // trusted proxy 场景：X-Forwarded-For 被采用
        let proxies: Vec<IpAddr> = vec!["10.0.0.1".parse().unwrap()];
        let (mid, logger) = make_audit_middleware_with_proxies(proxies);
        let router = build_ok_router(mid);
        let req = Request::builder()
            .method("GET")
            .uri("/test")
            .extension(std::net::SocketAddr::from(([10, 0, 0, 1], 8080)))
            .header("x-forwarded-for", "192.168.1.1")
            .body(Body::empty())
            .unwrap();
        let _resp = router.oneshot(req).await.unwrap();
        let events = logger.get_recent_events(10).await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].client_ip.as_deref(), Some("192.168.1.1"));
    }

    #[tokio::test]
    async fn test_audit_middleware_without_client_ip_records_empty_string() {
        // 无 SocketAddr extension 且无 trusted proxies → client_ip = None
        // unwrap_or_default() → 空字符串
        let (mid, logger) = make_audit_middleware();
        let router = build_ok_router(mid);
        let _resp = router
            .oneshot(make_test_request("GET", "/test"))
            .await
            .unwrap();
        let events = logger.get_recent_events(10).await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].client_ip.as_deref(), Some(""));
    }

    #[tokio::test]
    async fn test_audit_middleware_untrusted_proxy_ignores_xff_uses_connection_ip() {
        // 无 trusted proxies 时，XFF 被忽略，回退到 connection IP
        let (mid, logger) = make_audit_middleware();
        let router = build_ok_router(mid);
        let req = Request::builder()
            .method("GET")
            .uri("/test")
            .extension(std::net::SocketAddr::from(([10, 0, 0, 1], 8080)))
            .header("x-forwarded-for", "192.168.1.1")
            .body(Body::empty())
            .unwrap();
        let _resp = router.oneshot(req).await.unwrap();
        let events = logger.get_recent_events(10).await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].client_ip.as_deref(), Some("10.0.0.1"));
    }

    #[tokio::test]
    async fn test_audit_middleware_with_trusted_proxies_builder_honors_xff() {
        // 验证 with_trusted_proxies builder 方法确实保存了代理列表
        let proxies: Vec<IpAddr> = vec!["127.0.0.1".parse().unwrap()];
        let (mid, logger) = make_audit_middleware_with_proxies(proxies);
        let router = build_ok_router(mid);
        let req = Request::builder()
            .method("GET")
            .uri("/test")
            .extension(std::net::SocketAddr::from(([127, 0, 0, 1], 8080)))
            .header("x-forwarded-for", "203.0.113.5")
            .body(Body::empty())
            .unwrap();
        let _resp = router.oneshot(req).await.unwrap();
        let events = logger.get_recent_events(10).await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].client_ip.as_deref(), Some("203.0.113.5"));
    }

    #[tokio::test]
    async fn test_audit_middleware_records_method_and_path_in_action() {
        let (mid, logger) = make_audit_middleware();
        let router = build_ok_router(mid);
        let _resp = router
            .oneshot(make_test_request("POST", "/api/v1/generate"))
            .await
            .unwrap();
        let events = logger.get_recent_events(10).await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].action, "POST /api/v1/generate");
        assert_eq!(events[0].resource, "/api/v1/generate");
    }

    #[tokio::test]
    async fn test_audit_middleware_event_type_is_id_generation() {
        let (mid, logger) = make_audit_middleware();
        let router = build_ok_router(mid);
        let _resp = router
            .oneshot(make_test_request("GET", "/test"))
            .await
            .unwrap();
        let events = logger.get_recent_events(10).await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, AuditEventType::IdGeneration);
    }

    #[tokio::test]
    async fn test_audit_middleware_returns_response_status_unchanged() {
        // 中间件不应改变 next.run(req) 返回的响应状态码
        let (mid, _logger) = make_audit_middleware();
        let router = build_status_router(mid, StatusCode::NOT_FOUND);
        let resp = router
            .oneshot(make_test_request("GET", "/test"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_audit_middleware_increments_total_logged() {
        let (mid, logger) = make_audit_middleware();
        let router = build_ok_router(mid.clone());
        let _resp = router
            .oneshot(make_test_request("GET", "/test"))
            .await
            .unwrap();
        assert_eq!(logger.get_total_logged(), 1);
        // 通过 middleware 字段访问也应一致
        assert_eq!(mid.audit_logger.get_total_logged(), 1);
    }

    #[tokio::test]
    async fn test_audit_middleware_records_duration_ms_field() {
        // duration_ms 由 Instant::elapsed 计算，可能为 0（快机器），
        // 仅验证字段被设置（u64 类型，默认 0 也会被 with_duration 覆盖）
        let (mid, logger) = make_audit_middleware();
        let router = build_ok_router(mid);
        let _resp = router
            .oneshot(make_test_request("GET", "/test"))
            .await
            .unwrap();
        let events = logger.get_recent_events(10).await;
        assert_eq!(events.len(), 1);
        // duration_ms 是 u64，验证它被设置了（非默认值 0 也会被 with_duration 覆盖）
        // 由于无法保证 > 0，仅验证字段存在
        let _duration = events[0].duration_ms;
    }

    #[tokio::test]
    async fn test_audit_middleware_multiple_requests_log_multiple_events() {
        let (mid, logger) = make_audit_middleware();
        // 每次调用 build_ok_router 消费 mid，所以需要 Clone
        let mid_clone = mid.clone();
        let router = build_ok_router(mid_clone);
        let _resp1 = router
            .oneshot(make_test_request("GET", "/test"))
            .await
            .unwrap();
        let mid_clone2 = mid.clone();
        let router2 = build_ok_router(mid_clone2);
        let _resp2 = router2
            .oneshot(make_test_request("POST", "/test"))
            .await
            .unwrap();
        let events = logger.get_recent_events(10).await;
        assert_eq!(events.len(), 2);
        assert_eq!(logger.get_total_logged(), 2);
    }
}
