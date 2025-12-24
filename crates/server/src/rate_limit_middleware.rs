use crate::rate_limit::RateLimiter;
use axum::middleware::Next;
use axum::{
    body::Body,
    http::{Request, StatusCode},
    response::IntoResponse,
    response::Response,
};
use std::sync::Arc;

#[derive(Clone)]
pub struct RateLimitMiddleware {
    rate_limiter: Arc<RateLimiter>,
}

impl RateLimitMiddleware {
    pub fn new(rate_limiter: Arc<RateLimiter>) -> Self {
        Self { rate_limiter }
    }

    pub async fn rate_limit_middleware(&self, req: Request<Body>, next: Next) -> Response {
        let workspace_id = req.extensions().get::<String>().cloned();
        let client_ip = get_client_ip(&req);

        let key =
            workspace_id.unwrap_or_else(|| client_ip.unwrap_or_else(|| "anonymous".to_string()));

        let result = self.rate_limiter.check_rate_limit(&key, None, None).await;

        if result.allowed {
            let mut response = next.run(req).await;
            response.headers_mut().insert(
                "X-RateLimit-Limit",
                result.limit.to_string().parse().unwrap(),
            );
            response.headers_mut().insert(
                "X-RateLimit-Remaining",
                result.remaining.to_string().parse().unwrap(),
            );
            response
        } else {
            let mut response = axum::Json(serde_json::json!({
                "code": 429,
                "message": "Rate limit exceeded",
                "retry_after": result.retry_after
            }))
            .into_response();
            *response.status_mut() = StatusCode::TOO_MANY_REQUESTS;
            response.headers_mut().insert(
                "X-RateLimit-Limit",
                result.limit.to_string().parse().unwrap(),
            );
            response.headers_mut().insert(
                "Retry-After",
                result.retry_after.unwrap_or(1).to_string().parse().unwrap(),
            );
            response
        }
    }
}

fn get_client_ip(req: &Request<Body>) -> Option<String> {
    req.headers()
        .get("X-Forwarded-For")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.split(',').next().unwrap_or(s).trim().to_string())
        .or_else(|| {
            req.headers()
                .get("X-Real-IP")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string())
        })
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
