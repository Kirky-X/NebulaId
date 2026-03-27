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
use axum::middleware::Next;
use axum::{
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
            let mut response = axum::Json(serde_json::json!({
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

// ============================================================================
// Unit Tests
// ============================================================================

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

    #[tokio::test]
    async fn test_rate_limit_middleware_with_proxies() {
        let rate_limiter = Arc::new(RateLimiter::new(10, 5));
        let proxies = vec![IpAddr::from([127, 0, 0, 1])];
        let middleware = RateLimitMiddleware::new(rate_limiter).with_trusted_proxies(proxies);
        let _middleware = middleware;
    }

    #[tokio::test]
    async fn test_get_client_ip_no_proxy() {
        use axum::http::Request;

        let req = Request::builder()
            .uri("http://example.com")
            .body(Body::empty())
            .unwrap();

        let ip = get_client_ip(&req, &[]);
        assert!(ip.is_none());
    }

    #[tokio::test]
    async fn test_get_client_ip_with_trusted_proxy() {
        use axum::http::Request;

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
        use axum::http::Request;

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
}
