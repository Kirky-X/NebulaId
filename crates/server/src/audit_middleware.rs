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

use crate::audit::{AuditEventType, AuditLogger, AuditResult};
use crate::middleware::ApiKeyAuth;
use crate::rate_limit::RateLimiter;
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

        let audit_event = crate::audit::AuditEvent::new(
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
                "Request audit recorded"
            );
        }

        response
    }
}

fn get_client_ip(req: &Request<Body>, trusted_proxies: &[IpAddr]) -> Option<String> {
    // Get direct connection IP
    let connection_ip = req
        .extensions()
        .get::<std::net::SocketAddr>()
        .map(|addr| addr.ip());

    // Only trust headers if the request comes from a trusted proxy
    if let Some(conn_ip) = connection_ip {
        if trusted_proxies.contains(&conn_ip) {
            // First try X-Forwarded-For
            if let Some(xff) = req.headers().get("x-forwarded-for") {
                if let Ok(xff_str) = xff.to_str() {
                    // Take the first IP (original client)
                    if let Some(client_ip) = xff_str.split(',').next() {
                        return Some(client_ip.trim().to_string());
                    }
                }
            }

            // Then try X-Real-IP
            if let Some(xri) = req.headers().get("x-real-ip") {
                if let Ok(xri_str) = xri.to_str() {
                    return Some(xri_str.trim().to_string());
                }
            }
        }
    }

    // Fallback to direct connection IP
    connection_ip.map(|ip| ip.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::middleware::ApiKeyAuth;
    use crate::rate_limit::RateLimiter;
    use axum::body::Body;
    use axum::http::Request;

    #[tokio::test]
    async fn test_audit_middleware_creation() {
        use async_trait::async_trait;
        use nebula_core::database::{
            ApiKeyInfo, ApiKeyRepository, ApiKeyResponse, ApiKeyRole as CoreApiKeyRole,
            ApiKeyWithSecret, CreateApiKeyRequest,
        };
        use nebula_core::types::Result;
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
            ) -> Result<Option<(Option<Uuid>, nebula_core::database::ApiKeyRole)>> {
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
        }

        let audit_logger = Arc::new(AuditLogger::new(100));
        let auth = Arc::new(ApiKeyAuth::new(Arc::new(MockApiKeyRepo)));
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
}
