// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! 限流中间件：按 workspace ID、客户端 IP 或自定义标识对请求限流。

use crate::server::rate_limit::limiter::RateLimiter;
use axum::middleware::Next;
use axum::{
    body::Body,
    http::{header::HeaderValue, Request, StatusCode},
    response::IntoResponse,
    response::Response,
};
use std::net::IpAddr;
use std::sync::Arc;

/// 限流响应头名（规范小写形式）。
/// 与 `cors::EXPOSED_HEADERS` 同常量，拼写不一致浏览器将读不到该头。
pub const HEADER_RATE_LIMIT: &str = "x-ratelimit-limit";
pub const HEADER_RATE_LIMIT_REMAINING: &str = "x-ratelimit-remaining";

/// Axum 限流中间件，基于 limiteron 分片滑窗。
#[derive(Clone)]
pub struct RateLimitMiddleware {
    rate_limiter: Arc<RateLimiter>,
    trusted_proxies: Vec<IpAddr>,
}

impl RateLimitMiddleware {
    /// 新建限流中间件。
    pub fn new(rate_limiter: Arc<RateLimiter>) -> Self {
        Self {
            rate_limiter,
            trusted_proxies: Vec::new(), // Default: no trusted proxies
        }
    }

    /// 设置可信代理 IP：仅来自这些 IP 的请求才信任 X-Forwarded-For。
    pub fn with_trusted_proxies(mut self, proxies: Vec<IpAddr>) -> Self {
        self.trusted_proxies = proxies;
        self
    }

    /// 限流处理：从 extensions 或 IP 提取标识，放行或拒绝请求。
    pub async fn rate_limit_middleware(&self, req: Request<Body>, next: Next) -> Response {
        let workspace_id = req.extensions().get::<String>().cloned();
        let client_ip = get_client_ip(&req, &self.trusted_proxies);

        // 限流 key 优先级：workspace > IP > anonymous
        let key =
            workspace_id.unwrap_or_else(|| client_ip.unwrap_or_else(|| "anonymous".to_string()));

        let result = self.rate_limiter.check_rate_limit(&key, None, None).await;

        if result.allowed {
            // 放行：透传响应并附加限流头
            let mut response = next.run(req).await;

            if let Ok(limit_header) = result.limit.to_string().parse() {
                response
                    .headers_mut()
                    .insert(HEADER_RATE_LIMIT, limit_header);
            }
            if let Ok(remaining_header) = result.remaining.to_string().parse() {
                response
                    .headers_mut()
                    .insert(HEADER_RATE_LIMIT_REMAINING, remaining_header);
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
                    .insert(HEADER_RATE_LIMIT, limit_header);
            }
            // 拒绝响应显式声明剩余配额为 0，客户端无需从 Retry-After 推断。
            response
                .headers_mut()
                .insert(HEADER_RATE_LIMIT_REMAINING, HeaderValue::from_static("0"));

            let retry_after = result.retry_after.unwrap_or(1);
            if let Ok(retry_header) = retry_after.to_string().parse() {
                response.headers_mut().insert("Retry-After", retry_header);
            }
            response
        }
    }
}

/// 提取客户端 IP（可信代理时采用 X-Forwarded-For），委托共享实现。
fn get_client_ip(req: &Request<Body>, trusted_proxies: &[IpAddr]) -> Option<String> {
    crate::server::middleware::utils::get_client_ip(req, trusted_proxies)
}

/// `from_fn_with_state` 兼容包装：axum 0.8 要求自由函数，实例方法无法直接挂载。
pub async fn rate_limit_middleware_fn(
    axum::extract::State(middleware): axum::extract::State<RateLimitMiddleware>,
    req: Request<Body>,
    next: Next,
) -> Response {
    middleware.rate_limit_middleware(req, next).await
}

// Unit Tests

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::State;
    use axum::middleware::from_fn_with_state;
    use axum::routing::get;
    use axum::Router;
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

        // 192.168.1.1 不在可信代理列表
        let trusted_proxies = vec![IpAddr::from([127, 0, 0, 1])];
        let ip = get_client_ip(&req, &trusted_proxies);
        // 应返回直连 IP，而非 X-Forwarded-For
        assert_eq!(ip, Some("192.168.1.1".to_string()));
    }

    #[tokio::test]
    async fn test_get_client_ip_with_x_real_ip_header() {
        use axum::http::Request;

        let mut req = Request::builder()
            .uri("http://example.com")
            .header("X-Real-IP", "10.0.0.99")
            .body(Body::empty())
            .unwrap();

        req.extensions_mut()
            .insert(std::net::SocketAddr::from(([127, 0, 0, 1], 8080)));

        let trusted_proxies = vec![IpAddr::from([127, 0, 0, 1])];
        let ip = get_client_ip(&req, &trusted_proxies);
        // Trusted proxy + X-Real-IP（无 XFF）→ 返回 X-Real-IP 值
        assert_eq!(ip, Some("10.0.0.99".to_string()));
    }

    #[tokio::test]
    async fn test_get_client_ip_trusted_proxy_without_forwarded_headers() {
        use axum::http::Request;

        let mut req = Request::builder()
            .uri("http://example.com")
            .body(Body::empty())
            .unwrap();

        req.extensions_mut()
            .insert(std::net::SocketAddr::from(([127, 0, 0, 1], 8080)));

        let trusted_proxies = vec![IpAddr::from([127, 0, 0, 1])];
        let ip = get_client_ip(&req, &trusted_proxies);
        // 可信代理但无 XFF/XRI → 回退直连 IP
        assert_eq!(ip, Some("127.0.0.1".to_string()));
    }

    // rate_limit_middleware 端到端测试（Router + oneshot）

    /// 把 `&self` 实例方法包装为 `from_fn_with_state` 兼容 handler（axum 0.8 要求自由函数）。
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
        // capacity=5，首个请求应放行
        let rate_limiter = Arc::new(RateLimiter::new(10, 5));
        let middleware = RateLimitMiddleware::new(rate_limiter);
        let app = build_app(middleware);

        let response = app.oneshot(make_request("/")).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);

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
        // capacity=1：第二个请求应被限流；共享同一 rate_limiter Arc 以复用桶
        let rate_limiter = Arc::new(RateLimiter::new(1, 1));

        // 首个请求消费 1 个令牌
        let app1 = build_app(RateLimitMiddleware::new(rate_limiter.clone()));
        let first = app1.oneshot(make_request("/")).await.unwrap();
        assert_eq!(first.status(), StatusCode::OK);

        // 第二个请求（同一 rate_limiter、同一 key "anonymous"）被限流
        let app2 = build_app(RateLimitMiddleware::new(rate_limiter));
        let response = app2.oneshot(make_request("/")).await.unwrap();

        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);

        let limit_header = response
            .headers()
            .get("X-RateLimit-Limit")
            .expect("X-RateLimit-Limit header should be present on 429 too");
        assert_eq!(limit_header.to_str().unwrap(), "1");

        // 拒绝响应必须显式声明剩余配额为 0
        let remaining = response
            .headers()
            .get("X-RateLimit-Remaining")
            .expect("X-RateLimit-Remaining header should be present on 429");
        assert_eq!(remaining.to_str().unwrap(), "0");

        // Retry-After 取快照 reset_secs；rate=1/capacity=1 桶补满需 1 秒
        let retry_after = response
            .headers()
            .get("Retry-After")
            .expect("Retry-After header should be present on 429");
        assert_eq!(retry_after.to_str().unwrap(), "1");
    }

    #[tokio::test]
    async fn test_rate_limit_middleware_429_response_body_contains_message() {
        let rate_limiter = Arc::new(RateLimiter::new(1, 1));
        // 预先消费 1 个令牌，使中间件请求被拒
        rate_limiter.check_rate_limit("anonymous", None, None).await;

        let middleware = RateLimitMiddleware::new(rate_limiter);
        let app = build_app(middleware);

        let response = app.oneshot(make_request("/")).await.unwrap();

        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);

        let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body_json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(body_json["code"], 429);
        assert_eq!(body_json["message"], "Rate limit exceeded");
        // retry_after 取快照 reset_secs（rate=1/capacity=1 → 补满 1 秒）
        assert_eq!(body_json["retry_after"], 1);
    }

    #[tokio::test]
    async fn test_rate_limit_middleware_uses_workspace_id_as_key() {
        // extensions 注入 workspace_id → 作为限流 key
        let rate_limiter = Arc::new(RateLimiter::new(1, 1));
        let middleware = RateLimitMiddleware::new(rate_limiter.clone());

        // tenant-A 首个请求放行
        let app = build_app(middleware.clone());
        let response = app
            .oneshot(make_request_with_extension("/", "tenant-A"))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        // tenant-A 第二个请求被限流
        let app2 = build_app(middleware.clone());
        let response = app2
            .oneshot(make_request_with_extension("/", "tenant-A"))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);

        // tenant-B 是不同 key、独立桶 → 放行
        let app3 = build_app(middleware);
        let response = app3
            .oneshot(make_request_with_extension("/", "tenant-B"))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        assert_eq!(rate_limiter.bucket_count(), 2);
        assert!(rate_limiter.get_usage("tenant-A").is_some());
        assert!(rate_limiter.get_usage("tenant-B").is_some());
    }

    #[tokio::test]
    async fn test_rate_limit_middleware_falls_back_to_anonymous_key() {
        // 无 extension、无 SocketAddr → key 为 "anonymous"
        let rate_limiter = Arc::new(RateLimiter::new(1, 1));
        let middleware = RateLimitMiddleware::new(rate_limiter.clone());
        let app = build_app(middleware);

        let response = app.oneshot(make_request("/")).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        assert_eq!(rate_limiter.bucket_count(), 1);
        assert!(rate_limiter.get_usage("anonymous").is_some());
    }

    #[tokio::test]
    async fn test_rate_limit_middleware_falls_back_to_client_ip_key() {
        // 无 workspace_id 扩展但有 SocketAddr（非可信 IP）→ key 为直连 IP 字符串
        let rate_limiter = Arc::new(RateLimiter::new(1, 1));
        // 预先耗尽 "192.168.1.1" 桶，使中间件请求被拒
        rate_limiter
            .check_rate_limit("192.168.1.1", None, None)
            .await;

        let middleware = RateLimitMiddleware::new(rate_limiter.clone());
        let app = build_app(middleware);

        let mut req = make_request("/");
        req.extensions_mut()
            .insert(std::net::SocketAddr::from(([192, 168, 1, 1], 8080)));

        let response = app.oneshot(req).await.unwrap();
        // "192.168.1.1" 桶已耗尽 → 429
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);

        // key 为 IP 而非 anonymous
        assert!(rate_limiter.get_usage("192.168.1.1").is_some());
        assert!(rate_limiter.get_usage("anonymous").is_none());
    }

    #[tokio::test]
    async fn test_rate_limit_middleware_concurrent_keys_are_isolated() {
        // tenant-A 耗尽自身桶，tenant-B 与 anonymous 保持独立
        let rate_limiter = Arc::new(RateLimiter::new(1, 1));
        let middleware = RateLimitMiddleware::new(rate_limiter.clone());

        // tenant-A：2 个请求（1 放行、1 拒绝）
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

        // tenant-B 独立桶仍放行
        let app3 = build_app(middleware.clone());
        let r3 = app3
            .oneshot(make_request_with_extension("/", "tenant-B"))
            .await
            .unwrap();
        assert_eq!(r3.status(), StatusCode::OK);

        // anonymous 独立桶仍放行
        let app4 = build_app(middleware);
        let r4 = app4.oneshot(make_request("/")).await.unwrap();
        assert_eq!(r4.status(), StatusCode::OK);

        assert_eq!(rate_limiter.bucket_count(), 3);
    }

    #[tokio::test]
    async fn test_rate_limit_middleware_with_trusted_proxies_uses_xff() {
        // 直连 IP 为可信代理时，X-Forwarded-For 值应作为 key
        let rate_limiter = Arc::new(RateLimiter::new(1, 1));
        // 预先耗尽 "203.0.113.195" 桶，使中间件请求被拒
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
        // "203.0.113.195" 桶已耗尽 → 429
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);

        // key 取 XFF 值，而非直连 IP "127.0.0.1"
        assert!(rate_limiter.get_usage("203.0.113.195").is_some());
        assert!(rate_limiter.get_usage("127.0.0.1").is_none());
    }

    #[tokio::test]
    async fn test_rate_limit_middleware_with_trusted_proxies_no_xff_uses_connection_ip() {
        // 可信代理但无 XFF/XRI → key 回退直连 IP
        let rate_limiter = Arc::new(RateLimiter::new(1, 1));
        let middleware = RateLimitMiddleware::new(rate_limiter.clone())
            .with_trusted_proxies(vec![IpAddr::from([127, 0, 0, 1])]);
        let app = build_app(middleware);

        let mut req = make_request("/");
        req.extensions_mut()
            .insert(std::net::SocketAddr::from(([127, 0, 0, 1], 8080)));

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        // key 应为直连 IP "127.0.0.1"，而非 "anonymous"
        assert!(rate_limiter.get_usage("127.0.0.1").is_some());
        assert!(rate_limiter.get_usage("anonymous").is_none());
    }
}
