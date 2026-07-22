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

//! Rate limiting middleware powered by limiteron library.
//!
//! This middleware intercepts HTTP requests and applies rate limiting
//! based on client IP, workspace ID, or custom identifiers.

use crate::server::rate_limit::limiter::RateLimiter;
use sdforge::axum::middleware::Next;
use sdforge::axum::{
    body::Body,
    http::{Request, StatusCode},
    response::IntoResponse,
    response::Response,
};
use std::net::IpAddr;
use std::sync::Arc;

/// Rate limit middleware for Axum applications.
///
/// This middleware uses limiteron's ShardedSlidingWindowLimiter for
/// high-performance, accurate rate limiting.
#[derive(Clone)]
pub struct RateLimitMiddleware {
    rate_limiter: Arc<RateLimiter>,
    trusted_proxies: Vec<IpAddr>,
}

impl RateLimitMiddleware {
    /// Create a new rate limit middleware.
    ///
    /// # Arguments
    /// * `rate_limiter` - The rate limiter instance to use
    pub fn new(rate_limiter: Arc<RateLimiter>) -> Self {
        Self {
            rate_limiter,
            trusted_proxies: Vec::new(), // Default: no trusted proxies
        }
    }

    /// Create a middleware with trusted proxy IPs.
    ///
    /// Requests from these IPs will have their X-Forwarded-For header
    /// trusted to extract the real client IP.
    ///
    /// # Arguments
    /// * `proxies` - List of trusted proxy IP addresses
    pub fn with_trusted_proxies(mut self, proxies: Vec<IpAddr>) -> Self {
        self.trusted_proxies = proxies;
        self
    }

    /// Middleware handler that checks rate limits.
    ///
    /// Extracts client identifier from request extensions or IP,
    /// checks rate limit, and either allows or rejects the request.
    pub async fn rate_limit_middleware(&self, req: Request<Body>, next: Next) -> Response {
        // Extract identifiers from request
        let workspace_id = req.extensions().get::<String>().cloned();
        let client_ip = get_client_ip(&req, &self.trusted_proxies);

        // Determine rate limit key: workspace > IP > anonymous
        let key =
            workspace_id.unwrap_or_else(|| client_ip.unwrap_or_else(|| "anonymous".to_string()));

        // Check rate limit
        let result = self.rate_limiter.check_rate_limit(&key, None, None).await;

        if result.allowed {
            // Request allowed - proceed and add rate limit headers
            let mut response = next.run(req).await;

            // Add rate limit headers
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
            // Rate limited - return 429 response
            let mut response = sdforge::axum::Json(serde_json::json!({
                "code": 429,
                "message": "Rate limit exceeded",
                "retry_after": result.retry_after
            }))
            .into_response();
            *response.status_mut() = StatusCode::TOO_MANY_REQUESTS;

            // Add rate limit headers
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

/// Extract client IP from request.
///
/// Considers trusted proxies and X-Forwarded-For header.
///
/// Phase 9 T043 (LOW L3) — delegates to the single shared implementation
/// in `server::middleware::utils` so audit/rate_limit/api_key_auth
/// middleware all share one source of truth.
fn get_client_ip(req: &Request<Body>, trusted_proxies: &[IpAddr]) -> Option<String> {
    crate::server::middleware::utils::get_client_ip(req, trusted_proxies)
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use sdforge::axum::extract::State;
    use sdforge::axum::middleware::from_fn_with_state;
    use sdforge::axum::routing::get;
    use sdforge::axum::Router;
    use sdforge::tower::ServiceExt;
    use std::sync::Arc;

    #[tokio::test]
    async fn test_rate_limit_middleware_creation() {
        let rate_limiter = Arc::new(RateLimiter::new(10, 5));
        let middleware = RateLimitMiddleware::new(rate_limiter);
        let _middleware = middleware;
    }

    #[tokio::test]
    async fn test_rate_limit_middleware_with_proxies() {
        let rate_limiter = Arc::new(RateLimiter::new(10, 5));
        let proxies = vec![IpAddr::from([127, 0, 0, 1])];
        let middleware = RateLimitMiddleware::new(rate_limiter).with_trusted_proxies(proxies);
        let _middleware = middleware;
    }

    #[tokio::test]
    async fn test_get_client_ip_no_proxy() {
        use sdforge::axum::http::Request;

        let req = Request::builder()
            .uri("http://example.com")
            .body(Body::empty())
            .unwrap();

        let ip = get_client_ip(&req, &[]);
        assert!(ip.is_none());
    }

    #[tokio::test]
    async fn test_get_client_ip_with_trusted_proxy() {
        use sdforge::axum::http::Request;

        let mut req = Request::builder()
            .uri("http://example.com")
            .header("X-Forwarded-For", "203.0.113.195")
            .body(Body::empty())
            .unwrap();

        // Insert socket address extension
        req.extensions_mut()
            .insert(std::net::SocketAddr::from(([127, 0, 0, 1], 8080)));

        let trusted_proxies = vec![IpAddr::from([127, 0, 0, 1])];
        let ip = get_client_ip(&req, &trusted_proxies);
        assert_eq!(ip, Some("203.0.113.195".to_string()));
    }

    #[tokio::test]
    async fn test_get_client_ip_untrusted_proxy() {
        use sdforge::axum::http::Request;

        let mut req = Request::builder()
            .uri("http://example.com")
            .header("X-Forwarded-For", "203.0.113.195")
            .body(Body::empty())
            .unwrap();

        // Insert socket address extension
        req.extensions_mut()
            .insert(std::net::SocketAddr::from(([192, 168, 1, 1], 8080)));

        // 192.168.1.1 is not in trusted proxies
        let trusted_proxies = vec![IpAddr::from([127, 0, 0, 1])];
        let ip = get_client_ip(&req, &trusted_proxies);
        // Should return the direct connection IP, not X-Forwarded-For
        assert_eq!(ip, Some("192.168.1.1".to_string()));
    }

    #[tokio::test]
    async fn test_get_client_ip_with_x_real_ip_header() {
        use sdforge::axum::http::Request;

        let mut req = Request::builder()
            .uri("http://example.com")
            .header("X-Real-IP", "10.0.0.99")
            .body(Body::empty())
            .unwrap();

        // Insert socket address extension with trusted proxy IP
        req.extensions_mut()
            .insert(std::net::SocketAddr::from(([127, 0, 0, 1], 8080)));

        let trusted_proxies = vec![IpAddr::from([127, 0, 0, 1])];
        let ip = get_client_ip(&req, &trusted_proxies);
        // Trusted proxy + no XFF but X-Real-IP → returns X-Real-IP value
        assert_eq!(ip, Some("10.0.0.99".to_string()));
    }

    #[tokio::test]
    async fn test_get_client_ip_trusted_proxy_without_forwarded_headers() {
        use sdforge::axum::http::Request;

        let mut req = Request::builder()
            .uri("http://example.com")
            .body(Body::empty())
            .unwrap();

        // Insert socket address extension with trusted proxy IP, but no XFF/XRI headers
        req.extensions_mut()
            .insert(std::net::SocketAddr::from(([127, 0, 0, 1], 8080)));

        let trusted_proxies = vec![IpAddr::from([127, 0, 0, 1])];
        let ip = get_client_ip(&req, &trusted_proxies);
        // Trusted proxy but no XFF/XRI → falls back to connection IP
        assert_eq!(ip, Some("127.0.0.1".to_string()));
    }

    // ========================================================================
    // rate_limit_middleware 端到端测试：通过 Router + oneshot 覆盖完整路径
    // ========================================================================

    /// Wrap `&self` instance method into a `from_fn_with_state`-compatible handler.
    /// Uses `State<S>` extractor per axum 0.8 contract — not a closure, to keep
    /// clippy happy.
    async fn rate_limit_handler(
        State(state): State<RateLimitMiddleware>,
        req: Request<Body>,
        next: Next,
    ) -> Response {
        state.rate_limit_middleware(req, next).await
    }

    fn build_app(middleware: RateLimitMiddleware) -> Router {
        Router::new()
            .route("/", get(|| async { "ok" }))
            .layer(from_fn_with_state(middleware, rate_limit_handler))
    }

    fn make_request(uri: &str) -> Request<Body> {
        Request::builder().uri(uri).body(Body::empty()).unwrap()
    }

    fn make_request_with_extension(uri: &str, workspace_id: &str) -> Request<Body> {
        let mut req = Request::builder().uri(uri).body(Body::empty()).unwrap();
        req.extensions_mut().insert(workspace_id.to_string());
        req
    }

    #[tokio::test]
    async fn test_rate_limit_middleware_allows_request_under_limit() {
        // capacity=5 → first request should be allowed
        let rate_limiter = Arc::new(RateLimiter::new(10, 5));
        let middleware = RateLimitMiddleware::new(rate_limiter);
        let app = build_app(middleware);

        let response = app.oneshot(make_request("/")).await.unwrap();

        // Should pass through (200 OK)
        assert_eq!(response.status(), StatusCode::OK);

        // Should carry X-RateLimit-Limit and X-RateLimit-Remaining headers
        let limit_header = response
            .headers()
            .get("X-RateLimit-Limit")
            .expect("X-RateLimit-Limit header should be present");
        assert_eq!(limit_header.to_str().unwrap(), "5");

        let remaining_header = response
            .headers()
            .get("X-RateLimit-Remaining")
            .expect("X-RateLimit-Remaining header should be present");
        assert_eq!(remaining_header.to_str().unwrap(), "4");
    }

    #[tokio::test]
    async fn test_rate_limit_middleware_rejects_when_limit_exceeded() {
        // capacity=1 → second request should be rate-limited.
        // Use the same rate_limiter Arc so both apps share one bucket.
        let rate_limiter = Arc::new(RateLimiter::new(1, 1));

        // First request: allowed (shared limiter consumes 1 token)
        let app1 = build_app(RateLimitMiddleware::new(rate_limiter.clone()));
        let first = app1.oneshot(make_request("/")).await.unwrap();
        assert_eq!(first.status(), StatusCode::OK);

        // Second request (same rate_limiter, same key "anonymous") → rate-limited
        let app2 = build_app(RateLimitMiddleware::new(rate_limiter));
        let response = app2.oneshot(make_request("/")).await.unwrap();

        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);

        // 429 response should also carry X-RateLimit-Limit header
        let limit_header = response
            .headers()
            .get("X-RateLimit-Limit")
            .expect("X-RateLimit-Limit header should be present on 429 too");
        assert_eq!(limit_header.to_str().unwrap(), "1");

        // Should carry Retry-After header (limiter.rs sets retry_after=Some(1) on reject)
        let retry_after = response
            .headers()
            .get("Retry-After")
            .expect("Retry-After header should be present on 429");
        assert_eq!(retry_after.to_str().unwrap(), "1");
    }

    #[tokio::test]
    async fn test_rate_limit_middleware_429_response_body_contains_message() {
        // Verify the 429 response body contains the expected JSON payload
        let rate_limiter = Arc::new(RateLimiter::new(1, 1));
        // Pre-consume 1 token so the middleware request is rejected
        rate_limiter.check_rate_limit("anonymous", None, None).await;

        let middleware = RateLimitMiddleware::new(rate_limiter);
        let app = build_app(middleware);

        let response = app.oneshot(make_request("/")).await.unwrap();

        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);

        // Read body and verify JSON content
        let body_bytes = sdforge::axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body_json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(body_json["code"], 429);
        assert_eq!(body_json["message"], "Rate limit exceeded");
        // retry_after is Some(1) on rejection per limiteron TokenBucket
        assert_eq!(body_json["retry_after"], 1);
    }

    #[tokio::test]
    async fn test_rate_limit_middleware_uses_workspace_id_as_key() {
        // Inject workspace_id via extensions → should be used as the rate-limit key
        let rate_limiter = Arc::new(RateLimiter::new(1, 1));
        let middleware = RateLimitMiddleware::new(rate_limiter.clone());

        // First request from "tenant-A" → allowed
        let app = build_app(middleware.clone());
        let response = app
            .oneshot(make_request_with_extension("/", "tenant-A"))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        // Second request from "tenant-A" (same rate_limiter) → rate-limited
        let app2 = build_app(middleware.clone());
        let response = app2
            .oneshot(make_request_with_extension("/", "tenant-A"))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);

        // First request from "tenant-B" (different key, independent bucket) → allowed
        let app3 = build_app(middleware);
        let response = app3
            .oneshot(make_request_with_extension("/", "tenant-B"))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        // Verify limiter has 2 buckets: tenant-A and tenant-B
        assert_eq!(rate_limiter.bucket_count(), 2);
        assert!(rate_limiter.get_usage("tenant-A").is_some());
        assert!(rate_limiter.get_usage("tenant-B").is_some());
    }

    #[tokio::test]
    async fn test_rate_limit_middleware_falls_back_to_anonymous_key() {
        // No extension, no SocketAddr → key should be "anonymous"
        let rate_limiter = Arc::new(RateLimiter::new(1, 1));
        let middleware = RateLimitMiddleware::new(rate_limiter.clone());
        let app = build_app(middleware);

        let response = app.oneshot(make_request("/")).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        // Verify the bucket was keyed as "anonymous"
        assert_eq!(rate_limiter.bucket_count(), 1);
        assert!(rate_limiter.get_usage("anonymous").is_some());
    }

    #[tokio::test]
    async fn test_rate_limit_middleware_falls_back_to_client_ip_key() {
        // No workspace_id extension, but SocketAddr present (non-trusted IP) →
        // key should be the connection IP string, not "anonymous"
        let rate_limiter = Arc::new(RateLimiter::new(1, 1));
        // Pre-consume the "192.168.1.1" bucket so middleware request is rejected
        rate_limiter
            .check_rate_limit("192.168.1.1", None, None)
            .await;

        let middleware = RateLimitMiddleware::new(rate_limiter.clone());
        let app = build_app(middleware);

        let mut req = make_request("/");
        req.extensions_mut()
            .insert(std::net::SocketAddr::from(([192, 168, 1, 1], 8080)));

        let response = app.oneshot(req).await.unwrap();
        // Bucket "192.168.1.1" was already exhausted → 429
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);

        // Verify the bucket was keyed by IP, not "anonymous"
        assert!(rate_limiter.get_usage("192.168.1.1").is_some());
        assert!(rate_limiter.get_usage("anonymous").is_none());
    }

    #[tokio::test]
    async fn test_rate_limit_middleware_concurrent_keys_are_isolated() {
        // tenant-A exhausts its bucket, tenant-B and "anonymous" stay independent
        let rate_limiter = Arc::new(RateLimiter::new(1, 1));
        let middleware = RateLimitMiddleware::new(rate_limiter.clone());

        // tenant-A: 2 requests (1 allowed, 1 rejected)
        let app1 = build_app(middleware.clone());
        let r1 = app1
            .oneshot(make_request_with_extension("/", "tenant-A"))
            .await
            .unwrap();
        assert_eq!(r1.status(), StatusCode::OK);

        let app2 = build_app(middleware.clone());
        let r2 = app2
            .oneshot(make_request_with_extension("/", "tenant-A"))
            .await
            .unwrap();
        assert_eq!(r2.status(), StatusCode::TOO_MANY_REQUESTS);

        // tenant-B: still allowed (independent bucket)
        let app3 = build_app(middleware.clone());
        let r3 = app3
            .oneshot(make_request_with_extension("/", "tenant-B"))
            .await
            .unwrap();
        assert_eq!(r3.status(), StatusCode::OK);

        // anonymous: still allowed (independent bucket)
        let app4 = build_app(middleware);
        let r4 = app4.oneshot(make_request("/")).await.unwrap();
        assert_eq!(r4.status(), StatusCode::OK);

        // 3 independent buckets
        assert_eq!(rate_limiter.bucket_count(), 3);
    }

    #[tokio::test]
    async fn test_rate_limit_middleware_with_trusted_proxies_uses_xff() {
        // When connection IP is a trusted proxy, X-Forwarded-For should be used as key
        let rate_limiter = Arc::new(RateLimiter::new(1, 1));
        // Pre-consume the "203.0.113.195" bucket so middleware request is rejected
        rate_limiter
            .check_rate_limit("203.0.113.195", None, None)
            .await;

        let middleware = RateLimitMiddleware::new(rate_limiter.clone())
            .with_trusted_proxies(vec![IpAddr::from([127, 0, 0, 1])]);
        let app = build_app(middleware);

        let mut req = Request::builder()
            .uri("/")
            .header("X-Forwarded-For", "203.0.113.195")
            .body(Body::empty())
            .unwrap();
        req.extensions_mut()
            .insert(std::net::SocketAddr::from(([127, 0, 0, 1], 8080)));

        let response = app.oneshot(req).await.unwrap();
        // Bucket "203.0.113.195" was exhausted → 429
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);

        // Verify keying by XFF value, not by connection IP "127.0.0.1"
        assert!(rate_limiter.get_usage("203.0.113.195").is_some());
        assert!(rate_limiter.get_usage("127.0.0.1").is_none());
    }

    #[tokio::test]
    async fn test_rate_limit_middleware_with_trusted_proxies_no_xff_uses_connection_ip() {
        // Trusted proxy present but no XFF/XRI → key falls back to connection IP
        let rate_limiter = Arc::new(RateLimiter::new(1, 1));
        let middleware = RateLimitMiddleware::new(rate_limiter.clone())
            .with_trusted_proxies(vec![IpAddr::from([127, 0, 0, 1])]);
        let app = build_app(middleware);

        let mut req = make_request("/");
        req.extensions_mut()
            .insert(std::net::SocketAddr::from(([127, 0, 0, 1], 8080)));

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        // Key should be "127.0.0.1" (connection IP), not "anonymous"
        assert!(rate_limiter.get_usage("127.0.0.1").is_some());
        assert!(rate_limiter.get_usage("anonymous").is_none());
    }
}
