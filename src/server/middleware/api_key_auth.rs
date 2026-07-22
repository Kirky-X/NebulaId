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

use crate::core::database::ApiKeyRepository;
use base64::Engine;
use parking_lot::RwLock;
use sdforge::axum::body::Body;
use sdforge::axum::extract::State;
use sdforge::axum::http::{Request, StatusCode};
use sdforge::axum::middleware::Next;
use sdforge::axum::response::IntoResponse;
use sdforge::axum::response::Response;
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

// Re-export ApiKeyRole locally for use in this module
pub use crate::core::database::ApiKeyRole;

/// Phase 9 T043 (HIGH H6) — hard cap on the number of distinct IPs
/// tracked in `auth_failures`. When the map reaches this size, the
/// oldest entries are evicted to bound memory usage. Prevents an
/// attacker (especially one able to spoof IPs via the now-fixed
/// `X-Forwarded-For` issue, H3) from OOMing the process by sending
/// requests from many distinct source IPs.
const MAX_TRACKED_AUTH_FAILURE_IPS: usize = 10_000;

#[derive(Clone)]
pub struct ApiKeyAuth {
    pub(crate) repo: Arc<dyn ApiKeyRepository>,
    pub(crate) enabled: bool,
    trusted_proxies: Vec<IpAddr>,
    auth_failures: Arc<RwLock<HashMap<String, Vec<Instant>>>>,
}

impl ApiKeyAuth {
    pub fn new(repo: Arc<dyn ApiKeyRepository>, enabled: bool) -> Self {
        Self {
            repo,
            enabled,
            trusted_proxies: Vec::new(),
            auth_failures: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Phase 9 T043 (HIGH H3) — set the list of trusted proxy IPs.
    /// Requests whose direct peer IP appears in this list will have
    /// their `X-Forwarded-For` / `X-Real-IP` headers honored when
    /// determining the originating client IP for auth-failure
    /// tracking. Untrusted peers are identified by their direct
    /// connection IP, defeating spoofed-header attacks.
    pub fn with_trusted_proxies(mut self, proxies: Vec<IpAddr>) -> Self {
        self.trusted_proxies = proxies;
        self
    }

    fn check_auth_failure_rate(&self, client_ip: &str) -> bool {
        let now = Instant::now();
        let mut failures_map = self.auth_failures.write();
        let failures = failures_map.entry(client_ip.to_string()).or_default();

        // 移除 5 分钟前的记录
        failures.retain(|t| now.duration_since(*t) < Duration::from_secs(300));

        // Phase 9 T043 (HIGH H6) — evict empty entries so a long-lived
        // process does not accumulate one dead `Vec` per unique IP ever
        // seen. Without this, an attacker rotating IPs can OOM the
        // process even after the per-IP failure windows expire.
        if failures.is_empty() {
            failures_map.remove(client_ip);
            return true;
        }

        // 如果 5 分钟内失败超过 10 次，则阻止
        if failures.len() >= 10 {
            tracing::warn!(
                client_ip = %client_ip,
                failure_count = failures.len(),
                "{}",
                t!("log.server.middleware.api_key_auth.too_many_auth_failures")
            );
            return false;
        }

        // Phase 9 T043 (HIGH H6) — bound the map size. If we are at
        // capacity, drop the entry we just inserted (it has zero
        // failures) plus a sweep of any other empty entries. This
        // favors keeping actively-failing IPs over fresh ones.
        if failures_map.len() > MAX_TRACKED_AUTH_FAILURE_IPS {
            failures_map.retain(|_, v| !v.is_empty());
            if failures_map.len() > MAX_TRACKED_AUTH_FAILURE_IPS {
                // Still over capacity — clear the map entirely. This
                // is a last-resort safety valve; under normal load the
                // per-IP 5-minute window keeps the map small.
                failures_map.clear();
            }
        }

        true
    }

    fn record_auth_failure(&self, client_ip: &str) {
        let now = Instant::now();
        let mut failures_map = self.auth_failures.write();
        let failures = failures_map.entry(client_ip.to_string()).or_default();
        failures.push(now);
    }

    fn too_many_requests_response(&self) -> Response {
        let response = sdforge::axum::Json(serde_json::json!({
            "code": 429,
            "message": "Too many authentication attempts. Please try again later."
        }))
        .into_response();
        (StatusCode::TOO_MANY_REQUESTS, response).into_response()
    }

    fn get_client_ip(&self, req: &Request<Body>) -> Option<String> {
        // Phase 9 T043 (HIGH H3) — delegate to the shared, trusted-
        // proxy-aware implementation. Previously this method blindly
        // trusted `X-Forwarded-For`, allowing an attacker to forge
        // the header and bypass per-IP auth-failure rate limiting.
        crate::server::middleware::utils::get_client_ip(req, &self.trusted_proxies)
    }

    pub async fn validate_key(
        &self,
        key_id: &str,
        key_secret: &str,
    ) -> Option<(Option<uuid::Uuid>, ApiKeyRole)> {
        self.repo
            .validate_api_key(key_id, key_secret)
            .await
            .ok()
            .flatten()
    }

    pub async fn auth_middleware(&self, mut req: Request<Body>, next: Next) -> Response {
        let start_time = Instant::now();
        let path = req.uri().path().to_string();

        // 获取客户端 IP 和 User-Agent
        let client_ip = self
            .get_client_ip(&req)
            .unwrap_or_else(|| "unknown".to_string());
        let user_agent = req
            .headers()
            .get("user-agent")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("unknown")
            .to_string();

        tracing::debug!(event = "auth_middleware", path = %path, client_ip = %client_ip, "{}", t!("log.server.middleware.api_key_auth.auth_middleware_called"));

        // 如果认证禁用，记录警告日志并设置默认扩展值
        // SECURITY: Even when disabled, we must log the request for audit trail
        if !self.enabled {
            tracing::warn!(
                event = "auth_disabled_request",
                path = %path,
                client_ip = %client_ip,
                user_agent = %user_agent,
                "{}",
                t!("log.server.middleware.api_key_auth.auth_disabled_request")
            );

            // 设置默认的 workspace_id 和 role 扩展
            req.extensions_mut().insert(None::<uuid::Uuid>);
            // LOW-1 修复（CWE-1188）：禁用认证时不再赋予 User 角色
            // （User 是真实角色，有生成 ID 等业务权限）。改用 Anonymous，
            // 权限低于 User，只能访问公开端点（health/ready/metrics），
            // 其他端点由 `router.rs::verify_user_role` 拒绝。
            req.extensions_mut().insert(ApiKeyRole::Anonymous);

            // 记录审计日志（异步，不阻塞请求）
            tokio::spawn(async move {
                // 注意：这里无法访问审计日志器，需要通过 State 传递
                // 实际实现中应该在 router 层添加审计中间件
                tracing::info!(
                    event = "audit_auth_disabled",
                    path = %path,
                    client_ip = %client_ip,
                    "{}",
                    t!("log.server.middleware.api_key_auth.request_processed_without_auth")
                );
            });

            return next.run(req).await;
        }

        // 检查认证失败速率
        if !self.check_auth_failure_rate(&client_ip) {
            return self.too_many_requests_response();
        }

        let auth_header = req.headers().get("authorization").cloned();

        if let Some(header) = auth_header {
            if let Ok(value) = header.to_str() {
                // Support both "Basic base64(key_id:key_secret)" and "ApiKey key_id:key_secret"
                let (key_id, key_secret) = if let Some(credentials) = value.strip_prefix("Basic ") {
                    if let Ok(decoded) =
                        base64::engine::general_purpose::STANDARD.decode(credentials)
                    {
                        if let Ok(cred_str) = String::from_utf8(decoded) {
                            let parts: Vec<&str> = cred_str.splitn(2, ':').collect();
                            if parts.len() == 2 {
                                (parts[0].to_string(), parts[1].to_string())
                            } else {
                                tracing::warn!(
                                    event = "auth_failure",
                                    reason = "invalid_basic_format",
                                    client_ip = %client_ip,
                                    "{}",
                                    t!("log.server.middleware.api_key_auth.invalid_basic_format")
                                );
                                return self.unauthorized_response(&client_ip);
                            }
                        } else {
                            tracing::warn!(
                                event = "auth_failure",
                                reason = "invalid_encoding",
                                client_ip = %client_ip,
                                "{}",
                                t!("log.server.middleware.api_key_auth.invalid_base64_encoding")
                            );
                            return self.unauthorized_response(&client_ip);
                        }
                    } else {
                        tracing::warn!(
                            event = "auth_failure",
                            reason = "base64_decode_failed",
                            client_ip = %client_ip,
                            "{}",
                            t!("log.server.middleware.api_key_auth.base64_decode_failed")
                        );
                        return self.unauthorized_response(&client_ip);
                    }
                } else if let Some(api_key) = value.strip_prefix("ApiKey ") {
                    let parts: Vec<&str> = api_key.splitn(2, ':').collect();
                    if parts.len() == 2 {
                        (parts[0].to_string(), parts[1].to_string())
                    } else {
                        tracing::warn!(
                            event = "auth_failure",
                            reason = "invalid_apikey_format",
                            client_ip = %client_ip,
                            "{}",
                            t!("log.server.middleware.api_key_auth.invalid_apikey_format")
                        );
                        return self.unauthorized_response(&client_ip);
                    }
                } else {
                    tracing::warn!(
                        event = "auth_failure",
                        reason = "unsupported_format",
                        client_ip = %client_ip,
                        "{}",
                        t!("log.server.middleware.api_key_auth.unsupported_auth_format")
                    );
                    return self.unauthorized_response(&client_ip);
                };

                // Validate input lengths to prevent empty credentials
                if key_id.is_empty() || key_secret.is_empty() {
                    tracing::warn!(
                        event = "auth_failure",
                        reason = "empty_credentials",
                        client_ip = %client_ip,
                        "{}",
                        t!("log.server.middleware.api_key_auth.empty_credentials")
                    );
                    return self.unauthorized_response(&client_ip);
                }

                if let Some((workspace_id, role)) = self.validate_key(&key_id, &key_secret).await {
                    req.extensions_mut().insert(workspace_id);
                    req.extensions_mut().insert(role.clone());

                    // Log successful authentication
                    let duration = start_time.elapsed().as_millis() as u64;
                    let key_id_prefix = key_id.chars().take(8).collect::<String>();
                    tracing::info!(
                        event = "auth_success",
                        key_id_prefix = %key_id_prefix,
                        role = ?role,
                        client_ip = %client_ip,
                        duration_ms = duration,
                        "{}",
                        t!("log.server.middleware.api_key_auth.authentication_successful")
                    );

                    return next.run(req).await;
                } else {
                    // Log auth failure with key_id prefix (masked for security)
                    let key_id_prefix = key_id.chars().take(8).collect::<String>();
                    tracing::warn!(
                        event = "auth_failure",
                        reason = "invalid_credentials",
                        key_id_prefix = %key_id_prefix,
                        client_ip = %client_ip,
                        "{}",
                        t!("log.server.middleware.api_key_auth.invalid_credentials")
                    );
                }
            }
        } else {
            // Log missing auth header
            tracing::warn!(
                event = "auth_failure",
                reason = "missing_auth_header",
                client_ip = %client_ip,
                "{}",
                t!("log.server.middleware.api_key_auth.missing_auth_header")
            );
        }

        // Return 401 for both unknown routes and missing auth to avoid information disclosure
        // This prevents attackers from discovering which API endpoints exist
        self.unauthorized_response(&client_ip)
    }

    fn unauthorized_response(&self, client_ip: &str) -> Response {
        self.record_auth_failure(client_ip);
        let response = sdforge::axum::Json(serde_json::json!({
            "code": 401,
            "message": "Invalid or missing API key"
        }))
        .into_response();
        (StatusCode::UNAUTHORIZED, response).into_response()
    }
}

pub async fn admin_required_middleware(req: Request<Body>, next: Next) -> Response {
    if let Some(role) = req.extensions().get::<ApiKeyRole>() {
        tracing::debug!(event = "admin_check", role = ?role, "{}", t!("log.server.middleware.api_key_auth.checking_admin_role"));
        if *role == ApiKeyRole::Admin {
            return next.run(req).await;
        }
    } else {
        tracing::warn!(
            event = "admin_check",
            "{}",
            t!("log.server.middleware.api_key_auth.no_api_key_role_extension")
        );
    }

    let response = sdforge::axum::Json(serde_json::json!({
        "code": 403,
        "message": "Admin access required"
    }))
    .into_response();
    (StatusCode::FORBIDDEN, response).into_response()
}

pub async fn auth_middleware_fn(
    State(auth): State<Arc<ApiKeyAuth>>,
    req: Request<Body>,
    next: Next,
) -> Response {
    auth.auth_middleware(req, next).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::database::{
        ApiKeyInfo, ApiKeyRepository, ApiKeyResponse, ApiKeyRole, ApiKeyWithSecret,
        CreateApiKeyRequest,
    };
    use crate::core::types::Result;
    use async_trait::async_trait;
    use sdforge::axum::body::Body;
    use sdforge::axum::http::{Request, StatusCode};
    use sdforge::axum::middleware::from_fn_with_state;
    use sdforge::axum::routing::get;
    use sdforge::axum::Router;
    use sdforge::tower::ServiceExt;
    use sha2::Digest;
    use uuid::Uuid;

    #[derive(Clone)]
    struct MockApiKeyRepo {
        keys: std::collections::HashMap<String, (String, ApiKeyRole)>,
    }

    impl MockApiKeyRepo {
        fn hash_secret(secret: &str) -> String {
            let mut hasher = sha2::Sha256::default();
            hasher.update(secret);
            hex::encode(hasher.finalize())
        }
    }

    #[async_trait]
    impl ApiKeyRepository for MockApiKeyRepo {
        async fn create_api_key(&self, _request: &CreateApiKeyRequest) -> Result<ApiKeyWithSecret> {
            Ok(ApiKeyWithSecret {
                key: ApiKeyResponse {
                    id: Uuid::new_v4(),
                    key_id: "mock_key_id".to_string(),
                    key_prefix: "nino_".to_string(),
                    name: "Mock Key".to_string(),
                    description: None,
                    role: ApiKeyRole::User,
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
            key_id: &str,
            key_secret: &str,
        ) -> Result<Option<(Option<uuid::Uuid>, ApiKeyRole)>> {
            use subtle::ConstantTimeEq;
            if let Some((expected_secret, role)) = self.keys.get(key_id) {
                let incoming_hash = MockApiKeyRepo::hash_secret(key_secret);
                if expected_secret
                    .as_bytes()
                    .ct_eq(incoming_hash.as_bytes())
                    .into()
                {
                    // Admin keys have None workspace_id, user keys have Some(workspace_id)
                    let workspace_id = if *role == ApiKeyRole::Admin {
                        None
                    } else {
                        Some(uuid::Uuid::nil())
                    };
                    return Ok(Some((workspace_id, role.clone())));
                }
            }
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
            Err(crate::core::types::error::CoreError::InternalError(
                "rotate_api_key not implemented in mock".to_string(),
            ))
        }

        async fn get_keys_older_than(&self, _age_threshold_days: i64) -> Result<Vec<ApiKeyInfo>> {
            Ok(vec![])
        }
    }

    #[tokio::test]
    async fn test_api_key_auth_with_mock_repo() {
        let mut mock_keys = std::collections::HashMap::new();
        // Use hash_secret which only hashes the secret, matching the real validation logic
        mock_keys.insert(
            "test-key-id".to_string(),
            (MockApiKeyRepo::hash_secret("test-secret"), ApiKeyRole::User),
        );
        mock_keys.insert(
            "admin-key".to_string(),
            (
                MockApiKeyRepo::hash_secret("admin-secret"),
                ApiKeyRole::Admin,
            ),
        );

        let repo = MockApiKeyRepo { keys: mock_keys };
        let auth = ApiKeyAuth::new(Arc::new(repo), true);

        // Test valid user key
        let result = auth.validate_key("test-key-id", "test-secret").await;
        assert!(result.is_some());
        assert_eq!(result.unwrap().1, ApiKeyRole::User);

        // Test valid admin key
        let result = auth.validate_key("admin-key", "admin-secret").await;
        assert!(result.is_some());
        assert_eq!(result.unwrap().1, ApiKeyRole::Admin);

        // Test invalid secret
        let result = auth.validate_key("test-key-id", "wrong-secret").await;
        assert!(result.is_none());

        // Test non-existent key
        let result = auth.validate_key("non-existent", "secret").await;
        assert!(result.is_none());
    }

    // ========== Helper functions for middleware tests ==========

    fn make_mock_repo() -> MockApiKeyRepo {
        let mut mock_keys = std::collections::HashMap::new();
        mock_keys.insert(
            "user-key".to_string(),
            (MockApiKeyRepo::hash_secret("user-secret"), ApiKeyRole::User),
        );
        mock_keys.insert(
            "admin-key".to_string(),
            (
                MockApiKeyRepo::hash_secret("admin-secret"),
                ApiKeyRole::Admin,
            ),
        );
        MockApiKeyRepo { keys: mock_keys }
    }

    fn build_test_router(auth: Arc<ApiKeyAuth>) -> Router {
        Router::new()
            .route("/test", get(|| async { "ok" }))
            .layer(from_fn_with_state(auth, auth_middleware_fn))
    }

    fn basic_auth_header(key_id: &str, key_secret: &str) -> String {
        let credentials = format!("{}:{}", key_id, key_secret);
        let encoded = base64::engine::general_purpose::STANDARD.encode(credentials);
        format!("Basic {}", encoded)
    }

    fn api_key_header(key_id: &str, key_secret: &str) -> String {
        format!("ApiKey {}:{}", key_id, key_secret)
    }

    fn make_request(auth_header: Option<&str>) -> Request<Body> {
        let mut builder = Request::builder().uri("/test").method("GET");
        if let Some(value) = auth_header {
            builder = builder.header("authorization", value);
        }
        builder.body(Body::empty()).unwrap()
    }

    // ========== Constructor tests ==========

    #[test]
    fn test_api_key_auth_new_enabled() {
        let repo = Arc::new(make_mock_repo()) as Arc<dyn ApiKeyRepository>;
        let auth = ApiKeyAuth::new(repo, true);
        assert!(auth.enabled);
    }

    #[test]
    fn test_api_key_auth_new_disabled() {
        let repo = Arc::new(make_mock_repo()) as Arc<dyn ApiKeyRepository>;
        let auth = ApiKeyAuth::new(repo, false);
        assert!(!auth.enabled);
    }

    #[test]
    fn test_api_key_auth_with_trusted_proxies_does_not_panic() {
        let repo = Arc::new(make_mock_repo()) as Arc<dyn ApiKeyRepository>;
        let proxies = vec!["127.0.0.1".parse().unwrap()];
        let auth = ApiKeyAuth::new(repo, true).with_trusted_proxies(proxies);
        assert!(auth.enabled);
    }

    // ========== auth_middleware tests ==========

    #[tokio::test]
    async fn test_auth_middleware_disabled_calls_next() {
        let repo = Arc::new(make_mock_repo()) as Arc<dyn ApiKeyRepository>;
        let auth = Arc::new(ApiKeyAuth::new(repo, false));
        let router = build_test_router(auth);
        let resp = router.oneshot(make_request(None)).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_auth_middleware_enabled_no_auth_header_returns_401() {
        let repo = Arc::new(make_mock_repo()) as Arc<dyn ApiKeyRepository>;
        let auth = Arc::new(ApiKeyAuth::new(repo, true));
        let router = build_test_router(auth);
        let resp = router.oneshot(make_request(None)).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_auth_middleware_basic_valid_user_calls_next() {
        let repo = Arc::new(make_mock_repo()) as Arc<dyn ApiKeyRepository>;
        let auth = Arc::new(ApiKeyAuth::new(repo, true));
        let router = build_test_router(auth);
        let header = basic_auth_header("user-key", "user-secret");
        let resp = router.oneshot(make_request(Some(&header))).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_auth_middleware_basic_valid_admin_calls_next() {
        let repo = Arc::new(make_mock_repo()) as Arc<dyn ApiKeyRepository>;
        let auth = Arc::new(ApiKeyAuth::new(repo, true));
        let router = build_test_router(auth);
        let header = basic_auth_header("admin-key", "admin-secret");
        let resp = router.oneshot(make_request(Some(&header))).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_auth_middleware_basic_invalid_credentials_returns_401() {
        let repo = Arc::new(make_mock_repo()) as Arc<dyn ApiKeyRepository>;
        let auth = Arc::new(ApiKeyAuth::new(repo, true));
        let router = build_test_router(auth);
        let header = basic_auth_header("user-key", "wrong-secret");
        let resp = router.oneshot(make_request(Some(&header))).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_auth_middleware_basic_invalid_base64_returns_401() {
        let repo = Arc::new(make_mock_repo()) as Arc<dyn ApiKeyRepository>;
        let auth = Arc::new(ApiKeyAuth::new(repo, true));
        let router = build_test_router(auth);
        // "Basic !!!" is not valid base64.
        let resp = router
            .oneshot(make_request(Some("Basic !!!not-base64!!!")))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_auth_middleware_basic_no_colon_returns_401() {
        let repo = Arc::new(make_mock_repo()) as Arc<dyn ApiKeyRepository>;
        let auth = Arc::new(ApiKeyAuth::new(repo, true));
        let router = build_test_router(auth);
        // Encode a string without a colon.
        let encoded = base64::engine::general_purpose::STANDARD.encode("nocolonstring");
        let header = format!("Basic {}", encoded);
        let resp = router.oneshot(make_request(Some(&header))).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_auth_middleware_basic_empty_key_id_returns_401() {
        let repo = Arc::new(make_mock_repo()) as Arc<dyn ApiKeyRepository>;
        let auth = Arc::new(ApiKeyAuth::new(repo, true));
        let router = build_test_router(auth);
        let header = basic_auth_header("", "user-secret");
        let resp = router.oneshot(make_request(Some(&header))).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_auth_middleware_basic_empty_key_secret_returns_401() {
        let repo = Arc::new(make_mock_repo()) as Arc<dyn ApiKeyRepository>;
        let auth = Arc::new(ApiKeyAuth::new(repo, true));
        let router = build_test_router(auth);
        let header = basic_auth_header("user-key", "");
        let resp = router.oneshot(make_request(Some(&header))).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_auth_middleware_api_key_valid_calls_next() {
        let repo = Arc::new(make_mock_repo()) as Arc<dyn ApiKeyRepository>;
        let auth = Arc::new(ApiKeyAuth::new(repo, true));
        let router = build_test_router(auth);
        let header = api_key_header("user-key", "user-secret");
        let resp = router.oneshot(make_request(Some(&header))).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_auth_middleware_api_key_no_colon_returns_401() {
        let repo = Arc::new(make_mock_repo()) as Arc<dyn ApiKeyRepository>;
        let auth = Arc::new(ApiKeyAuth::new(repo, true));
        let router = build_test_router(auth);
        let header = "ApiKey nocolonstring".to_string();
        let resp = router.oneshot(make_request(Some(&header))).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_auth_middleware_unsupported_format_returns_401() {
        let repo = Arc::new(make_mock_repo()) as Arc<dyn ApiKeyRepository>;
        let auth = Arc::new(ApiKeyAuth::new(repo, true));
        let router = build_test_router(auth);
        let resp = router
            .oneshot(make_request(Some("Bearer some-token")))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_auth_middleware_too_many_failures_returns_429() {
        let repo = Arc::new(make_mock_repo()) as Arc<dyn ApiKeyRepository>;
        let auth = Arc::new(ApiKeyAuth::new(repo, true));
        let router = build_test_router(auth);
        // Send 10 invalid requests to trip the rate limiter (>= 10 failures
        // in 5 minutes triggers 429).
        let bad_header = basic_auth_header("user-key", "wrong");
        for _ in 0..10 {
            let _ = router
                .clone()
                .oneshot(make_request(Some(&bad_header)))
                .await
                .unwrap();
        }
        // 11th request should get 429.
        let resp = router
            .oneshot(make_request(Some(&bad_header)))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn test_auth_middleware_empty_authorization_value_returns_401() {
        let repo = Arc::new(make_mock_repo()) as Arc<dyn ApiKeyRepository>;
        let auth = Arc::new(ApiKeyAuth::new(repo, true));
        let router = build_test_router(auth);
        // Empty authorization header value: header is present but empty.
        let resp = router.oneshot(make_request(Some(""))).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    // ========== admin_required_middleware tests ==========

    fn build_admin_router() -> Router {
        // Build a router that applies both auth middleware and admin_required
        // middleware. We inject the role extension manually for the admin
        // tests since we want to isolate the admin check.
        Router::new().route("/test", get(|| async { "ok" })).layer(
            sdforge::axum::middleware::from_fn(admin_required_middleware),
        )
    }

    fn make_request_with_role(role: ApiKeyRole) -> Request<Body> {
        let mut builder = Request::builder().uri("/test").method("GET");
        builder = builder.extension(role);
        builder.body(Body::empty()).unwrap()
    }

    #[tokio::test]
    async fn test_admin_required_admin_role_calls_next() {
        let router = build_admin_router();
        let resp = router
            .oneshot(make_request_with_role(ApiKeyRole::Admin))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_admin_required_user_role_returns_403() {
        let router = build_admin_router();
        let resp = router
            .oneshot(make_request_with_role(ApiKeyRole::User))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn test_admin_required_no_role_extension_returns_403() {
        let router = build_admin_router();
        // No role extension injected.
        let resp = router.oneshot(make_request(None)).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn test_admin_required_anonymous_role_returns_403() {
        let router = build_admin_router();
        let resp = router
            .oneshot(make_request_with_role(ApiKeyRole::Anonymous))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    // ========== Role extension injection tests ==========

    fn build_role_check_router(auth: Arc<ApiKeyAuth>) -> Router {
        // A router that returns the injected role as text so tests can
        // observe which role was injected by auth_middleware.
        Router::new()
            .route(
                "/test",
                get(|request: Request<Body>| async move {
                    if let Some(role) = request.extensions().get::<ApiKeyRole>() {
                        format!("{:?}", role)
                    } else {
                        "no-role".to_string()
                    }
                }),
            )
            .layer(from_fn_with_state(auth, auth_middleware_fn))
    }

    async fn read_body_to_string(body: Body) -> String {
        // Use axum's built-in `to_bytes` (axum 0.8) instead of http_body_util,
        // which is not in the project's dev-dependencies.
        let bytes = sdforge::axum::body::to_bytes(body, usize::MAX)
            .await
            .expect("failed to read response body");
        String::from_utf8(bytes.to_vec()).expect("response body is not valid UTF-8")
    }

    #[tokio::test]
    async fn test_auth_disabled_injects_anonymous_role() {
        let repo = Arc::new(make_mock_repo()) as Arc<dyn ApiKeyRepository>;
        let auth = Arc::new(ApiKeyAuth::new(repo, false));
        let router = build_role_check_router(auth);
        let resp = router.oneshot(make_request(None)).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = read_body_to_string(resp.into_body()).await;
        assert_eq!(body, "Anonymous");
    }

    #[tokio::test]
    async fn test_auth_valid_basic_injects_user_role() {
        let repo = Arc::new(make_mock_repo()) as Arc<dyn ApiKeyRepository>;
        let auth = Arc::new(ApiKeyAuth::new(repo, true));
        let router = build_role_check_router(auth);
        let header = basic_auth_header("user-key", "user-secret");
        let resp = router.oneshot(make_request(Some(&header))).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = read_body_to_string(resp.into_body()).await;
        assert_eq!(body, "User");
    }

    #[tokio::test]
    async fn test_auth_valid_basic_admin_injects_admin_role() {
        let repo = Arc::new(make_mock_repo()) as Arc<dyn ApiKeyRepository>;
        let auth = Arc::new(ApiKeyAuth::new(repo, true));
        let router = build_role_check_router(auth);
        let header = basic_auth_header("admin-key", "admin-secret");
        let resp = router.oneshot(make_request(Some(&header))).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = read_body_to_string(resp.into_body()).await;
        assert_eq!(body, "Admin");
    }

    #[tokio::test]
    async fn test_auth_valid_api_key_injects_user_role() {
        let repo = Arc::new(make_mock_repo()) as Arc<dyn ApiKeyRepository>;
        let auth = Arc::new(ApiKeyAuth::new(repo, true));
        let router = build_role_check_router(auth);
        let header = api_key_header("user-key", "user-secret");
        let resp = router.oneshot(make_request(Some(&header))).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = read_body_to_string(resp.into_body()).await;
        assert_eq!(body, "User");
    }

    // ========== validate_key tests ==========

    #[tokio::test]
    async fn test_validate_key_empty_key_id_returns_none() {
        let repo = Arc::new(make_mock_repo()) as Arc<dyn ApiKeyRepository>;
        let auth = ApiKeyAuth::new(repo, true);
        let result = auth.validate_key("", "user-secret").await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_validate_key_empty_key_secret_returns_none() {
        let repo = Arc::new(make_mock_repo()) as Arc<dyn ApiKeyRepository>;
        let auth = ApiKeyAuth::new(repo, true);
        let result = auth.validate_key("user-key", "").await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_validate_key_admin_returns_none_workspace_id() {
        let repo = Arc::new(make_mock_repo()) as Arc<dyn ApiKeyRepository>;
        let auth = ApiKeyAuth::new(repo, true);
        let result = auth.validate_key("admin-key", "admin-secret").await;
        assert!(result.is_some());
        let (workspace_id, role) = result.unwrap();
        assert_eq!(role, ApiKeyRole::Admin);
        // Admin keys are global (workspace_id = None).
        assert!(workspace_id.is_none());
    }

    #[tokio::test]
    async fn test_validate_key_user_returns_some_workspace_id() {
        let repo = Arc::new(make_mock_repo()) as Arc<dyn ApiKeyRepository>;
        let auth = ApiKeyAuth::new(repo, true);
        let result = auth.validate_key("user-key", "user-secret").await;
        assert!(result.is_some());
        let (workspace_id, role) = result.unwrap();
        assert_eq!(role, ApiKeyRole::User);
        // User keys are bound to a workspace (Some(Uuid::nil()) per mock).
        assert!(workspace_id.is_some());
        assert_eq!(workspace_id.unwrap(), Uuid::nil());
    }
}
