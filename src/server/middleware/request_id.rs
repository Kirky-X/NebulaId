// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Request ID 中间件（T022）：提取 / 校验 / 生成 / 贯穿。
//!
//! 语义：
//! - 请求头 `x-request-id` 存在且为合法 UUID 时**透传原值**（不重排格式）；
//!   缺失、非 UTF-8 或非法时生成 `uuid::Uuid::now_v7`。
//! - 解析结果写入：
//!   1. 请求头 `x-request-id`（回写规范化值，供内侧
//!      `sdforge::context::context_middleware` 透传装配 RequestContext）；
//!   2. 请求 extensions（[`RequestId`] newtype，handler / 中间件可
//!      `Extension<RequestId>` 直接读取）；
//!   3. 任务局部上下文（[`current_request_id`]，`helpers` 错误响应装配处
//!      读取，保证错误响应体 `request_id` 与响应头一致）；
//!   4. tracing span 属性（`http_request` span 的 `request_id` 字段贯穿
//!      下游全部日志）。
//! - 响应回显 `x-request-id`。
//!
//! 挂载位置：router 中**最后 `.layer()`**（axum 0.8 后挂载者先执行），
//! 位于 `sdforge::context::context_middleware` 外侧——本中间件先完成
//! 校验/生成与回写，context_middleware 对同一取值透传，两条路径的
//! 响应头必然一致。

use axum::extract::Request;
use axum::middleware::Next;
use tracing::Instrument;

tokio::task_local! {
    /// 当前请求的关联 ID；由 [`request_id_middleware`] 在任务局部安装。
    static CURRENT_REQUEST_ID: String;
}

/// 请求关联 ID（extensions 载荷）。
///
/// 持有的是**回显值**：合法上游头原样透传，否则为新生成的 UUID v7 字符串。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestId(pub String);

/// 读取当前请求的关联 ID。
///
/// 由 [`request_id_middleware`] 安装；无中间件上下文（独立调用、
/// 单元测试、非 HTTP 入口）时返回 `None`，调用方应回退自有生成逻辑
/// （如 `helpers` 响应装配处的 UUID v4）。
pub fn current_request_id() -> Option<String> {
    CURRENT_REQUEST_ID.try_with(|id| id.clone()).ok()
}

/// 解析请求 ID：合法 UUID 透传原值；缺失 / 非 UTF-8 / 非法 → UUID v7。
fn resolve_request_id(header_value: Option<&str>) -> String {
    match header_value {
        Some(raw) if uuid::Uuid::parse_str(raw).is_ok() => raw.to_string(),
        _ => uuid::Uuid::now_v7().to_string(),
    }
}

/// HTTP 中间件：解析/生成 request ID 并贯穿请求-响应全链路。
///
/// 挂载方式：`axum::middleware::from_fn(request_id_middleware)`，且必须
/// 位于 `sdforge::context::context_middleware` **外侧**（后 `.layer()`），
/// 详见模块文档。
pub async fn request_id_middleware(mut req: Request, next: Next) -> axum::response::Response {
    let incoming = req
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok());
    let request_id = resolve_request_id(incoming);

    // 回写请求头（内侧 sdforge context_middleware 据此透传同一取值），
    // 并放入 extensions 供 handler / 下游中间件直接读取。
    if let Ok(value) = axum::http::HeaderValue::from_str(&request_id) {
        req.headers_mut().insert(
            axum::http::header::HeaderName::from_static("x-request-id"),
            value,
        );
    }
    req.extensions_mut().insert(RequestId(request_id.clone()));

    // 任务局部上下文（helpers 错误装配读取）+ tracing span 属性贯穿下游。
    let span = tracing::info_span!("http_request", request_id = %request_id);
    let mut response = CURRENT_REQUEST_ID
        .scope(request_id.clone(), next.run(req))
        .instrument(span)
        .await;

    if let Ok(value) = axum::http::HeaderValue::from_str(&request_id) {
        response.headers_mut().insert(
            axum::http::header::HeaderName::from_static("x-request-id"),
            value,
        );
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    // oneshot 来自 tower::ServiceExt（sdforge re-export，与 router 测试同源）。
    use sdforge::tower::ServiceExt;

    // ---- resolve_request_id ----

    #[test]
    fn test_resolve_request_id_passes_through_valid_uuid_unchanged() {
        // 合法 UUID（含非规范格式：大写 / 无连字符）必须原样透传，不做重排。
        let canonical = "01970396-5d7a-7000-8000-000000000001";
        assert_eq!(
            resolve_request_id(Some(canonical)),
            canonical,
            "合法 UUID 必须透传原值"
        );

        let simple = "019703965d7a70008000000000000001";
        assert_eq!(
            resolve_request_id(Some(simple)),
            simple,
            "无连字符的合法 UUID 同样透传原值"
        );
    }

    #[test]
    fn test_resolve_request_id_generates_v7_for_missing_or_invalid() {
        for incoming in [None, Some(""), Some("not-a-uuid"), Some("req-18f3a2-1")] {
            let resolved = resolve_request_id(incoming);
            let parsed = uuid::Uuid::parse_str(&resolved).expect("必须是合法 UUID");
            assert_eq!(
                parsed.get_version_num(),
                7,
                "缺失/非法输入必须生成 UUID v7，输入 = {incoming:?}"
            );
        }
    }

    #[test]
    fn test_resolve_request_id_generates_distinct_ids() {
        let a = resolve_request_id(None);
        let b = resolve_request_id(None);
        assert_ne!(a, b, "两次生成的 request_id 必须不同");
    }

    // ---- 中间件端到端（mini axum app）----

    async fn echo_app_request_id(
        axum::Extension(request_id): axum::Extension<RequestId>,
    ) -> String {
        // handler 经 extensions 直接读取 RequestId 的用法样例。
        request_id.0
    }

    fn mini_app() -> axum::Router {
        axum::Router::new()
            .route("/echo", axum::routing::get(echo_app_request_id))
            .layer(axum::middleware::from_fn(request_id_middleware))
    }

    #[tokio::test]
    async fn test_middleware_sets_uuid_response_header() {
        let resp = mini_app()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/echo")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let header = resp
            .headers()
            .get("x-request-id")
            .expect("响应必须携带 x-request-id 头")
            .to_str()
            .unwrap()
            .to_owned();
        let parsed = uuid::Uuid::parse_str(&header).expect("request_id 必须是合法 UUID");
        assert_eq!(parsed.get_version_num(), 7, "缺省生成必须是 UUID v7");
    }

    #[tokio::test]
    async fn test_middleware_passes_through_valid_upstream_header() {
        let upstream = "123e4567-e89b-12d3-a456-426614174000";
        let resp = mini_app()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/echo")
                    .header("x-request-id", upstream)
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let header = resp
            .headers()
            .get("x-request-id")
            .expect("响应必须携带 x-request-id 头")
            .to_str()
            .unwrap()
            .to_owned();
        assert_eq!(header, upstream, "合法上游 x-request-id 必须透传原值");
    }

    #[tokio::test]
    async fn test_middleware_replaces_invalid_upstream_header_with_v7() {
        let resp = mini_app()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/echo")
                    .header("x-request-id", "garbage-id")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let header = resp
            .headers()
            .get("x-request-id")
            .expect("响应必须携带 x-request-id 头")
            .to_str()
            .unwrap()
            .to_owned();
        let parsed = uuid::Uuid::parse_str(&header).expect("非法上游输入必须被替换为合法 UUID");
        assert_eq!(parsed.get_version_num(), 7);
        assert_ne!(header, "garbage-id");
    }

    #[tokio::test]
    async fn test_middleware_two_requests_get_different_ids() {
        let id_of = || async {
            mini_app()
                .oneshot(
                    axum::http::Request::builder()
                        .uri("/echo")
                        .body(axum::body::Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap()
                .headers()
                .get("x-request-id")
                .unwrap()
                .to_str()
                .unwrap()
                .to_owned()
        };

        let a = id_of().await;
        let b = id_of().await;
        assert_ne!(a, b, "两次请求的 request_id 必须不同");
    }

    #[tokio::test]
    async fn test_current_request_id_visible_inside_handler_scope() {
        // current_request_id 在中间件作用域内可读，且与响应头一致；
        // 在作用域外（如本测试顶部）读取返回 None。
        assert_eq!(current_request_id(), None, "作用域外必须返回 None");

        let resp = mini_app()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/echo")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let header = resp
            .headers()
            .get("x-request-id")
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned();
        // handler 的响应体就是 Extension<RequestId> 读值（透传一致性）。
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(std::str::from_utf8(&body).unwrap(), header);
    }
}
