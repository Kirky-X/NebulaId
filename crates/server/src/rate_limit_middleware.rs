use crate::rate_limit::RateLimiter;
use axum::middleware::Next;
use axum::{
    body::Body,
    http::{Request, StatusCode},
    response::IntoResponse,
    response::Response,
};
use std::net::IpAddr;
use std::sync::Arc;

#[derive(Clone)]
pub struct RateLimitMiddleware {
    rate_limiter: Arc<RateLimiter>,
    trusted_proxies: Vec<IpAddr>,
}

impl RateLimitMiddleware {
    pub fn new(rate_limiter: Arc<RateLimiter>) -> Self {
        Self {
            rate_limiter,
            trusted_proxies: Vec::new(), // Default: no trusted proxies
        }
    }

    pub fn with_trusted_proxies(mut self, proxies: Vec<IpAddr>) -> Self {
        self.trusted_proxies = proxies;
        self
    }

    pub async fn rate_limit_middleware(&self, req: Request<Body>, next: Next) -> Response {
        let workspace_id = req.extensions().get::<String>().cloned();
        let client_ip = get_client_ip(&req, &self.trusted_proxies);

        let key =
            workspace_id.unwrap_or_else(|| client_ip.unwrap_or_else(|| "anonymous".to_string()));

        let result = self.rate_limiter.check_rate_limit(&key, None, None).await;

        if result.allowed {
            let mut response = next.run(req).await;
            if let Ok(limit_header) = result.limit.to_string().parse() {
                response
                    .headers_mut()
                    .insert("X-RateLimit-Limit", limit_header);
            }
            if let Ok(remaining_header) = result.remaining.to_string().parse() {
                response
                    .headers_mut()
                    .insert("X-RateLimit-Remaining", remaining_header);
            }
            response
        } else {
            let mut response = axum::Json(serde_json::json!({
                "code": 429,
                "message": "Rate limit exceeded",
                "retry_after": result.retry_after
            }))
            .into_response();
            *response.status_mut() = StatusCode::TOO_MANY_REQUESTS;
            if let Ok(limit_header) = result.limit.to_string().parse() {
                response
                    .headers_mut()
                    .insert("X-RateLimit-Limit", limit_header);
            }
            let retry_after = result.retry_after.unwrap_or(1);
            if let Ok(retry_header) = retry_after.to_string().parse() {
                response.headers_mut().insert("Retry-After", retry_header);
            }
            response
        }
    }
}

fn get_client_ip(req: &Request<Body>, trusted_proxies: &[IpAddr]) -> Option<String> {
    // Get direct connection IP
    let connection_ip = req
        .extensions()
        .get::<std::net::SocketAddr>()
        .map(|addr| addr.ip());

    // Only trust X-Forwarded-For if the request comes from a trusted proxy
    if let Some(conn_ip) = connection_ip {
        if trusted_proxies.contains(&conn_ip) {
            // Trust X-Forwarded-For from trusted proxy
            if let Some(xff) = req.headers().get("X-Forwarded-For") {
                if let Ok(xff_str) = xff.to_str() {
                    // Take the first IP (original client)
                    if let Some(client_ip) = xff_str.split(',').next() {
                        return Some(client_ip.trim().to_string());
                    }
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
    use std::sync::Arc;

    #[tokio::test]
    async fn test_rate_limit_middleware_creation() {
        let rate_limiter = Arc::new(RateLimiter::new(10, 5));
        let middleware = RateLimitMiddleware::new(rate_limiter);
        let _middleware = middleware;
    }
}
