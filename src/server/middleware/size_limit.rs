// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use sdforge::tower_http::limit::RequestBodyLimitLayer;

/// 请求体大小限制（1MB）
pub const MAX_REQUEST_SIZE: usize = 1_048_576;

/// 创建请求体大小限制中间件
pub fn create_size_limit_middleware() -> RequestBodyLimitLayer {
    RequestBodyLimitLayer::new(MAX_REQUEST_SIZE)
}

/// 请求体过大错误响应
#[derive(Debug)]
pub struct RequestBodyTooLarge;

impl IntoResponse for RequestBodyTooLarge {
    fn into_response(self) -> Response {
        // T032 —— 统一错误信封（ErrorResponse，含 business_code）；
        // 原 ApiErrorResponse 双格式已移除。
        use crate::server::models::{ApiErrorCode, ErrorResponse};

        let error_response = ErrorResponse::new(
            StatusCode::PAYLOAD_TOO_LARGE.as_u16() as i32,
            ApiErrorCode::InvalidInput,
            "Request body too large".to_string(),
        )
        .with_details(format!(
            "Maximum request size is {} bytes ({:.2} MB)",
            MAX_REQUEST_SIZE,
            MAX_REQUEST_SIZE as f64 / (1024.0 * 1024.0)
        ));

        (StatusCode::PAYLOAD_TOO_LARGE, axum::Json(error_response)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_max_request_size() {
        assert_eq!(MAX_REQUEST_SIZE, 1_048_576);
    }

    #[test]
    fn test_create_size_limit_middleware() {
        let middleware = create_size_limit_middleware();
        // This test verifies that the middleware can be created without panicking
        // Actual size limit testing would require setting up a full Axum server
        let _ = middleware;
    }

    #[tokio::test]
    async fn test_request_body_too_large_response() {
        let response = RequestBodyTooLarge;

        // Convert to Response and check status code
        let axum_response = response.into_response();

        assert_eq!(axum_response.status(), StatusCode::PAYLOAD_TOO_LARGE);

        // T032 — 统一错误信封：413 + business_code=3001（InvalidInput），
        // 且携带装配处生成的 request_id/timestamp。
        let bytes = axum::body::to_bytes(axum_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["code"], 413);
        assert_eq!(json["business_code"], "3001");
        assert_eq!(json["message"], "Request body too large");
        assert!(
            !json["request_id"].as_str().unwrap_or_default().is_empty(),
            "request_id must be populated in the unified envelope"
        );
        assert!(json["timestamp"].as_i64().unwrap_or(0) > 0);
    }
}
