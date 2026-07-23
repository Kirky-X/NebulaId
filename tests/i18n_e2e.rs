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

//! Phase 8 T042 — End-to-end i18n integration test.
//!
//! Verifies the full chain: `Accept-Language` header → `locale_middleware`
//! negotiates `Locale` → handler reads `Extension<Locale>` → produces a
//! locale-translated error response via the helpers in
//! `src/server/handlers/helpers.rs`.
//!
//! This test does NOT require PostgreSQL / etcd — it builds a minimal
//! axum router with only `locale_middleware` + a test handler that
//! deliberately returns a `CoreError` to exercise the translation path.

#![cfg(test)]

use axum::{
    body::Body,
    http::{Request, StatusCode},
    middleware::from_fn,
    routing::get,
    Router,
};
use nebulaid::core::types::{CoreError, ErrorResponse};
use nebulaid::server::handlers::helpers::core_error_to_response;
use nebulaid::server::middleware::locale::{locale_middleware, Locale};
use tower::ServiceExt;

/// Build a minimal router that exercises the i18n error-response path.
///
/// The handler reads `Extension<Locale>` (injected by `locale_middleware`)
/// and converts a fixed `CoreError::InvalidInput("negative")` into a JSON
/// `ErrorResponse` via `core_error_to_response`. This is the same path
/// production handlers in `router.rs` use.
fn i18n_router() -> Router {
    Router::new()
        .route(
            "/error",
            get(
                |axum::Extension(locale): axum::Extension<Locale>| async move {
                    let err = CoreError::InvalidInput("negative".to_string());
                    let (status, body) = core_error_to_response(&err, locale);
                    (status, body)
                },
            ),
        )
        .route(
            "/uuid",
            get(
                |axum::Extension(locale): axum::Extension<Locale>| async move {
                    let (status, body) =
                        nebulaid::server::handlers::helpers::invalid_uuid_response(locale);
                    (status, body)
                },
            ),
        )
        .layer(from_fn(locale_middleware))
}

async fn decode_json(resp: axum::response::Response) -> ErrorResponse {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body should decode");
    serde_json::from_slice(&bytes).expect("body should be valid ErrorResponse JSON")
}

#[tokio::test]
async fn t042_zh_cn_accept_language_returns_chinese_error_message() {
    let app = i18n_router();

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/error")
                .header("Accept-Language", "zh-CN,zh;q=0.9,en;q=0.8")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = decode_json(resp).await;
    assert_eq!(body.code, 400);
    // CoreError::InvalidInput("negative") under zh-CN renders as "无效输入：negative"
    assert_eq!(body.message, "无效输入：negative");
}

#[tokio::test]
async fn t042_en_accept_language_returns_english_error_message() {
    let app = i18n_router();

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/error")
                .header("Accept-Language", "en-US,en;q=0.9")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = decode_json(resp).await;
    assert_eq!(body.code, 400);
    assert_eq!(body.message, "Invalid input: negative");
}

#[tokio::test]
async fn t042_missing_accept_language_defaults_to_english() {
    let app = i18n_router();

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/error")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = decode_json(resp).await;
    assert_eq!(body.message, "Invalid input: negative");
}

#[tokio::test]
async fn t042_unsupported_locale_falls_back_to_english() {
    let app = i18n_router();

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/error")
                .header("Accept-Language", "fr-FR,fr;q=0.9,ja;q=0.8")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = decode_json(resp).await;
    // fr and ja are not supported — fallback to en
    assert_eq!(body.message, "Invalid input: negative");
}

#[tokio::test]
async fn t042_zh_cn_invalid_uuid_response_returns_chinese() {
    let app = i18n_router();

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/uuid")
                .header("Accept-Language", "zh-CN")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = decode_json(resp).await;
    assert_eq!(body.code, 400);
    assert_eq!(body.message, "无效的 UUID 格式");
}

#[tokio::test]
async fn t042_en_invalid_uuid_response_returns_english() {
    let app = i18n_router();

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/uuid")
                .header("Accept-Language", "en")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = decode_json(resp).await;
    assert_eq!(body.message, "Invalid UUID format");
}

/// Verifies that 5xx-class CoreError variants do NOT leak internal strings.
/// This is the CWE-209 fix from the T041 security review.
#[tokio::test]
async fn t042_5xx_database_error_does_not_leak_internal_string() {
    let app = Router::new()
        .route(
            "/db",
            get(
                |axum::Extension(locale): axum::Extension<Locale>| async move {
                    let err =
                        CoreError::DatabaseError("postgres://user:secret@host:5432/db".to_string());
                    let (status, body) = core_error_to_response(&err, locale);
                    (status, body)
                },
            ),
        )
        .layer(from_fn(locale_middleware));

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/db")
                .header("Accept-Language", "zh-CN")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = decode_json(resp).await;
    assert_eq!(body.code, 500);
    // Must be the generic translated message, NOT the connection string
    assert!(!body.message.contains("postgres://"));
    assert!(!body.message.contains("secret"));
    assert!(!body.message.contains("host:5432"));
    // zh-CN generic database error message
    assert_eq!(body.message, "数据库操作失败");
}
