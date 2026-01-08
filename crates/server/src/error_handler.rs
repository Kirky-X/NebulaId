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

use crate::models::{ApiErrorCode, ApiErrorResponse, ErrorResponse};
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
    let request_id = uuid::Uuid::new_v4().to_string();
    tracing::error!(
        event = "unhandled_error",
        request_id = %request_id,
        error = %error
    );
    let response = ErrorResponse::new(500, "Internal server error".to_string());
    (StatusCode::INTERNAL_SERVER_ERROR, Json(response)).into_response()
}

/// 将 CoreError 转换为增强的 API 错误响应（带结构化错误码）
pub fn core_error_to_api_response(error: &CoreError) -> (StatusCode, Json<ApiErrorResponse>) {
    use nebula_core::types::error::CoreError;

    let (code, message, status) = match error {
        CoreError::InvalidInput(msg) => (
            ApiErrorCode::InvalidInput,
            format!("Invalid input: {}", msg),
            StatusCode::BAD_REQUEST,
        ),
        CoreError::InvalidIdFormat(msg)
        | CoreError::InvalidIdString(msg)
        | CoreError::InvalidAlgorithmType(msg) => (
            ApiErrorCode::InvalidInput,
            msg.clone(),
            StatusCode::BAD_REQUEST,
        ),
        CoreError::NotFound(msg) => (
            ApiErrorCode::WorkspaceNotFound, // 默认资源错误
            msg.clone(),
            StatusCode::NOT_FOUND,
        ),
        CoreError::BizTagNotFound(msg) => (
            ApiErrorCode::BizTagNotFound,
            msg.clone(),
            StatusCode::NOT_FOUND,
        ),
        CoreError::AuthenticationError(msg) => (
            ApiErrorCode::InvalidApiKey,
            msg.clone(),
            StatusCode::UNAUTHORIZED,
        ),
        CoreError::InvalidApiKeySignature => (
            ApiErrorCode::InvalidApiKey,
            "Invalid API key signature".to_string(),
            StatusCode::UNAUTHORIZED,
        ),
        CoreError::ApiKeyDisabled => (
            ApiErrorCode::ApiKeyDisabled,
            "API key has been disabled".to_string(),
            StatusCode::UNAUTHORIZED,
        ),
        CoreError::ApiKeyExpired => (
            ApiErrorCode::ApiKeyExpired,
            "API key has expired".to_string(),
            StatusCode::UNAUTHORIZED,
        ),
        CoreError::WorkspaceDisabled(msg) => {
            (ApiErrorCode::Forbidden, msg.clone(), StatusCode::FORBIDDEN)
        }
        CoreError::RateLimitExceeded => (
            ApiErrorCode::RateLimitExceeded,
            "Rate limit exceeded".to_string(),
            StatusCode::TOO_MANY_REQUESTS,
        ),
        CoreError::DatabaseError(_msg) => (
            ApiErrorCode::DatabaseError,
            "Database operation failed".to_string(),
            StatusCode::INTERNAL_SERVER_ERROR,
        ),
        CoreError::CacheError(_msg) => (
            ApiErrorCode::CacheError,
            "Cache service unavailable".to_string(),
            StatusCode::INTERNAL_SERVER_ERROR,
        ),
        CoreError::EtcdError(msg) | CoreError::ParseError(msg) | CoreError::IoError(msg) => (
            ApiErrorCode::InternalError,
            format!("Operation failed: {}", msg),
            StatusCode::INTERNAL_SERVER_ERROR,
        ),
        CoreError::TimeoutError => (
            ApiErrorCode::ServiceUnavailable,
            "Request timeout".to_string(),
            StatusCode::SERVICE_UNAVAILABLE,
        ),
        CoreError::ClockMovedBackward { .. }
        | CoreError::SequenceOverflow { .. }
        | CoreError::SegmentExhausted { .. } => (
            ApiErrorCode::InternalError,
            "ID generation algorithm error".to_string(),
            StatusCode::INTERNAL_SERVER_ERROR,
        ),
        CoreError::InternalError(_msg) => (
            ApiErrorCode::InternalError,
            "Internal server error".to_string(),
            StatusCode::INTERNAL_SERVER_ERROR,
        ),
        CoreError::ConfigurationError(msg) => (
            ApiErrorCode::InternalError,
            format!("Configuration error: {}", msg),
            StatusCode::INTERNAL_SERVER_ERROR,
        ),
        CoreError::Unknown => (
            ApiErrorCode::InternalError,
            "Unknown error occurred".to_string(),
            StatusCode::INTERNAL_SERVER_ERROR,
        ),
    };

    let details = if cfg!(debug_assertions) {
        Some(error.to_string())
    } else {
        None
    };

    (
        status,
        Json(
            ApiErrorResponse::new(code, message)
                .with_details(details.unwrap_or_else(|| "See request_id for support".to_string())),
        ),
    )
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

    #[test]
    fn test_core_error_to_api_response_invalid_input() {
        let error = CoreError::InvalidInput("test error".to_string());
        let (status, response) = core_error_to_api_response(&error);

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(response.code, "3001"); // InvalidInput code
        assert!(response.message.contains("Invalid input"));
    }

    #[test]
    fn test_core_error_to_api_response_not_found() {
        let error = CoreError::NotFound("resource not found".to_string());
        let (status, response) = core_error_to_api_response(&error);

        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(response.code, "2001"); // WorkspaceNotFound code
    }

    #[test]
    fn test_core_error_to_api_response_biz_tag_not_found() {
        let error = CoreError::BizTagNotFound("biz_tag not found".to_string());
        let (status, response) = core_error_to_api_response(&error);

        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(response.code, "2003"); // BizTagNotFound code
    }

    #[test]
    fn test_core_error_to_api_response_authentication() {
        let error = CoreError::AuthenticationError("Invalid API key".to_string());
        let (status, response) = core_error_to_api_response(&error);

        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(response.code, "1003"); // InvalidApiKey code
    }

    #[test]
    fn test_core_error_to_api_response_api_key_disabled() {
        let error = CoreError::ApiKeyDisabled;
        let (status, response) = core_error_to_api_response(&error);

        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(response.code, "1005"); // ApiKeyDisabled code
    }

    #[test]
    fn test_core_error_to_api_response_api_key_expired() {
        let error = CoreError::ApiKeyExpired;
        let (status, response) = core_error_to_api_response(&error);

        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(response.code, "1004"); // ApiKeyExpired code
    }

    #[test]
    fn test_core_error_to_api_response_workspace_disabled() {
        let error = CoreError::WorkspaceDisabled("workspace disabled".to_string());
        let (status, response) = core_error_to_api_response(&error);

        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(response.code, "1002"); // Forbidden code
    }

    #[test]
    fn test_core_error_to_api_response_rate_limit() {
        let error = CoreError::RateLimitExceeded;
        let (status, response) = core_error_to_api_response(&error);

        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(response.code, "4001"); // RateLimitExceeded code
    }

    #[test]
    fn test_core_error_to_api_response_database() {
        let error = CoreError::DatabaseError("DB error".to_string());
        let (status, response) = core_error_to_api_response(&error);

        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(response.code, "5002"); // DatabaseError code
    }

    #[test]
    fn test_core_error_to_api_response_cache() {
        let error = CoreError::CacheError("cache error".to_string());
        let (status, response) = core_error_to_api_response(&error);

        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(response.code, "5003"); // CacheError code
    }

    #[test]
    fn test_core_error_to_api_response_timeout() {
        let error = CoreError::TimeoutError;
        let (status, response) = core_error_to_api_response(&error);

        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(response.code, "5004"); // ServiceUnavailable code
    }

    #[test]
    fn test_core_error_to_api_response_internal() {
        let error = CoreError::InternalError("internal error".to_string());
        let (status, response) = core_error_to_api_response(&error);

        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(response.code, "5001"); // InternalError code
    }

    #[test]
    fn test_core_error_to_api_response_clock_backward() {
        let error = CoreError::ClockMovedBackward {
            last_timestamp: 1234567890,
        };
        let (status, response) = core_error_to_api_response(&error);

        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(response.code, "5001"); // InternalError code
        assert!(response.message.contains("algorithm error"));
    }

    #[test]
    fn test_api_response_structure() {
        let response =
            ApiErrorResponse::new(ApiErrorCode::InvalidInput, "test message".to_string());

        assert_eq!(response.code, "3001");
        assert_eq!(response.message, "test message");
        assert!(response.details.is_none());
        assert!(!response.request_id.is_empty());
        assert!(response.timestamp > 0);
    }

    #[test]
    fn test_api_response_with_details() {
        let response =
            ApiErrorResponse::new(ApiErrorCode::InternalError, "error message".to_string())
                .with_details("additional details".to_string());

        assert_eq!(response.details, Some("additional details".to_string()));
    }

    #[test]
    fn test_error_response_conversion() {
        let old_response = ErrorResponse::new(404, "Not found".to_string());

        let new_response = ApiErrorResponse::from(old_response);

        assert_eq!(new_response.code, "2001"); // WorkspaceNotFound
        assert_eq!(new_response.message, "Not found");
    }
}
