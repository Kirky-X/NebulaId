use crate::audit::{AuditEventType, AuditLogger, AuditResult};
use crate::middleware::ApiKeyAuth;
use crate::rate_limit::RateLimiter;
use axum::body::Body;
use axum::http::Request;
use axum::response::Response;
use std::sync::Arc;
use tracing::debug;

#[derive(Clone)]
#[allow(dead_code)]
pub struct AuditMiddleware {
    audit_logger: Arc<AuditLogger>,
    auth: Arc<ApiKeyAuth>,
    rate_limiter: Arc<RateLimiter>,
}

impl AuditMiddleware {
    pub fn new(audit_logger: Arc<AuditLogger>, auth: Arc<ApiKeyAuth>, rate_limiter: Arc<RateLimiter>) -> Self {
        Self {
            audit_logger,
            auth,
            rate_limiter,
        }
    }

    pub async fn audit_middleware(&self, req: Request<Body>, next: axum::middleware::Next) -> Response {
        let start = std::time::Instant::now();
        let path = req.uri().path().to_string();
        let method = req.method().to_string();
        let client_ip = get_client_ip(&req);
        let user_agent = req
            .headers()
            .get("user-agent")
            .and_then(|h| h.to_str().ok())
            .map(|s| s.to_string());

        let workspace_id = req.extensions().get::<String>().cloned();

        let response = next.run(req).await;

        let duration_ms = start.elapsed().as_millis() as u64;
        let status_code = response.status().as_u16();

        let result = if status_code >= 200 && status_code < 300 {
            AuditResult::Success
        } else if status_code >= 400 && status_code < 500 {
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

fn get_client_ip(req: &Request<Body>) -> Option<String> {
    req.headers()
        .get("x-forwarded-for")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.split(',').next())
        .map(|s| s.trim().to_string())
        .or_else(|| {
            req.headers()
                .get("x-real-ip")
                .and_then(|h| h.to_str().ok())
                .map(|s| s.to_string())
        })
        .or_else(|| {
            req.extensions()
                .get::<std::net::SocketAddr>()
                .map(|s| s.to_string())
        })
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
        let audit_logger = Arc::new(AuditLogger::new(100));
        let auth = Arc::new(ApiKeyAuth::new());
        let rate_limiter = Arc::new(RateLimiter::new(10000, 100));

        let middleware = AuditMiddleware::new(audit_logger, auth, rate_limiter);
        assert!(middleware.audit_logger.get_total_logged() == 0);
    }

    #[tokio::test]
    async fn test_get_client_ip_from_header() {
        let req = Request::builder()
            .header("x-forwarded-for", "192.168.1.1, 10.0.0.1")
            .body(Body::empty())
            .unwrap();

        let ip = get_client_ip(&req);
        assert_eq!(ip, Some("192.168.1.1".to_string()));
    }

    #[tokio::test]
    async fn test_get_client_ip_from_real_ip() {
        let req = Request::builder()
            .header("x-real-ip", "192.168.1.100")
            .body(Body::empty())
            .unwrap();

        let ip = get_client_ip(&req);
        assert_eq!(ip, Some("192.168.1.100".to_string()));
    }

    #[tokio::test]
    async fn test_get_client_ip_fallback() {
        let req = Request::builder().body(Body::empty()).unwrap();

        let ip = get_client_ip(&req);
        assert!(ip.is_none());
    }
}
