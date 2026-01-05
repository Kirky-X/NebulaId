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

use crate::models::ErrorResponse;
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use nebula_core::types::error::CoreError;

/// Convert CoreError to HTTP response
pub fn handle_core_error(error: CoreError) -> Response {
    let (status_code, core_response) = error.to_http_response();

    let status = match status_code {
        400 => StatusCode::BAD_REQUEST,
        401 => StatusCode::UNAUTHORIZED,
        403 => StatusCode::FORBIDDEN,
        404 => StatusCode::NOT_FOUND,
        429 => StatusCode::TOO_MANY_REQUESTS,
        500 => StatusCode::INTERNAL_SERVER_ERROR,
        503 => StatusCode::SERVICE_UNAVAILABLE,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };

    let response = ErrorResponse {
        code: core_response.code,
        message: core_response.message,
        details: core_response.details.map(|d| d.to_string()),
    };

    (status, Json(response)).into_response()
}

/// Convert any error to HTTP response
pub fn handle_any_error<E: std::fmt::Display>(error: E) -> Response {
    tracing::error!("Unhandled error: {}", error);
    let response = ErrorResponse::new(500, format!("Internal server error: {}", error));
    (StatusCode::INTERNAL_SERVER_ERROR, Json(response)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_handle_core_error_invalid_input() {
        let error = CoreError::InvalidInput("Invalid input".to_string());
        let response = handle_core_error(error);

        // Should return 400 Bad Request
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn test_handle_core_error_authentication() {
        let error = CoreError::AuthenticationError("Invalid API key".to_string());
        let response = handle_core_error(error);

        // Should return 401 Unauthorized
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn test_handle_core_error_not_found() {
        let error = CoreError::NotFound("Resource not found".to_string());
        let response = handle_core_error(error);

        // Should return 404 Not Found
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn test_handle_core_error_rate_limit() {
        let error = CoreError::RateLimitExceeded;
        let response = handle_core_error(error);

        // Should return 429 Too Many Requests
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    #[test]
    fn test_handle_core_error_internal() {
        let error = CoreError::InternalError("Internal error".to_string());
        let response = handle_core_error(error);

        // Should return 500 Internal Server Error
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn test_handle_any_error() {
        let error = std::io::Error::other("IO error");
        let response = handle_any_error(error);

        // Should return 500 Internal Server Error
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }
}
