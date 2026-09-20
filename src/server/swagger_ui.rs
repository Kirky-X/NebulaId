// Copyright (c) 2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Swagger UI 静态资源路由（吸收自 sdforge `docs` feature 的 UI 面）。
//!
//! 已发布的 sdforge 0.5.0-rc.4 只提供 [`sdforge::docs::swagger_ui_router`]——
//! 它会在 `/api-docs/openapi.json` 注册 sdforge 自有 inventory 聚合 spec，
//! 与本服务自有的同名 spec 端点（`server::openapi::openapi_json_handler`）
//! 冲突，`Router::merge` 直接 panic。故将「仅挂 UI、spec 地址由调用方指定」
//! 的 `swagger_ui_router_with_spec` 变体本地化（该函数在 sdforge rc.5 WIP 中，
//! 待其发布后可评估切回 `sdforge::docs` 并移除直接依赖 `utoipa-swagger-ui`）。

use std::sync::Arc;

use axum::extract::Path;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Extension;
use utoipa_swagger_ui::{serve, Config};

/// 构建仅含 Swagger UI 路由的 axum Router，spec 地址由调用方指定。
///
/// 不注册 `/api-docs/openapi.json`——宿主应用自行提供 spec 端点（可以是
/// 动态生成的、带认证的或来自独立文档服务的），Swagger UI 从 `openapi_url`
/// 加载。
///
/// # 参数
///
/// - `openapi_url`: Swagger UI 页面加载的 OpenAPI JSON 端点（绝对路径）。
///
/// # 示例
///
/// ```ignore
/// use nebulaid::server::swagger_ui::swagger_ui_router_with_spec;
/// // 宿主已在 /api-docs/openapi.json 提供自己的 spec
/// let app = axum::Router::new()
///     .merge(swagger_ui_router_with_spec("/api-docs/openapi.json"));
/// ```
pub fn swagger_ui_router_with_spec(openapi_url: &str) -> axum::Router {
    let config: Arc<Config<'static>> = Arc::new(Config::new([openapi_url.to_string()]));

    axum::Router::new()
        .route("/swagger-ui/", get(serve_swagger_ui))
        .route("/swagger-ui/{*rest}", get(serve_swagger_ui))
        .layer(Extension(config))
}

/// 服务 Swagger UI 静态资源（index.html / swagger-ui.css / ...）。
///
/// `/swagger-ui/` → tail = ""（渲染 index.html）
/// `/swagger-ui/swagger-ui.css` → tail = "swagger-ui.css"
async fn serve_swagger_ui(
    path: Option<Path<String>>,
    Extension(state): Extension<Arc<Config<'static>>>,
) -> impl IntoResponse {
    let tail = path.as_ref().map(|p| p.as_str()).unwrap_or("");

    // 路径遍历防护：拒绝包含 `..` 的请求，防止读取 Swagger UI 静态包外的文件。
    if tail.contains("..") {
        return StatusCode::BAD_REQUEST.into_response();
    }

    match serve(tail, state) {
        Ok(Some(file)) => (
            StatusCode::OK,
            [("Content-Type", file.content_type)],
            file.bytes.into_owned(),
        )
            .into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(error) => {
            // 不向客户端泄露内部错误详情，仅记录日志。
            tracing::error!("Swagger UI serve failed: {}", error);
            (StatusCode::INTERNAL_SERVER_ERROR, "internal server error").into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use sdforge::tower::ServiceExt;

    fn app() -> axum::Router {
        swagger_ui_router_with_spec("/api-docs/openapi.json")
    }

    #[tokio::test]
    async fn serves_index_and_static_assets() {
        let app = app();

        // 首页：/swagger-ui/ → tail = "" → index.html
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/swagger-ui/")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert!(
            res.headers()["content-type"]
                .to_str()
                .unwrap()
                .starts_with("text/html"),
            "index 应以 text/html 返回"
        );

        // 静态资源：/swagger-ui/swagger-ui.css
        let res = app
            .oneshot(
                Request::builder()
                    .uri("/swagger-ui/swagger-ui.css")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert!(
            res.headers()["content-type"]
                .to_str()
                .unwrap()
                .starts_with("text/css"),
            "css 应以 text/css 返回"
        );
    }

    #[tokio::test]
    async fn missing_asset_returns_404() {
        let res = app()
            .oneshot(
                Request::builder()
                    .uri("/swagger-ui/no-such-file.css")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn path_traversal_returns_400() {
        let res = app()
            .oneshot(
                Request::builder()
                    // %2e%2e 经 Path 提取器解码为 ".."，必须命中遍历防护
                    .uri("/swagger-ui/%2e%2e/secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    }
}
