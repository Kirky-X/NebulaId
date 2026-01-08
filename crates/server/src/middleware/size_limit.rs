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

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use tower_http::limit::RequestBodyLimitLayer;

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
        use crate::models::{ApiErrorCode, ApiErrorResponse};

        let error_response = ApiErrorResponse::new(
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

    #[test]
    fn test_request_body_too_large_response() {
        let response = RequestBodyTooLarge;

        // Convert to Response and check status code
        let axum_response = response.into_response();

        assert_eq!(axum_response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }
}
