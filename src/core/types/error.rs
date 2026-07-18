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

// Phase 8 T038 ICU i18n — Display strings extracted to `locales/{en,zh-CN}.yml`
// under `error.<variant_snake>` keys. thiserror's `#[error("{}", t!(...))]`
// attribute generates `impl Display` that calls `t!()` for translation lookup
// at runtime. Default locale is "en" (set in main.rs via `init_i18n("en")`).

use rust_i18n::t;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, Serialize, Deserialize, Clone)]
pub enum CoreError {
    #[error("{}", t!("error.invalid_id_format", value = _0))]
    InvalidIdFormat(String),

    #[error("{}", t!("error.invalid_id_string", value = _0))]
    InvalidIdString(String),

    #[error("{}", t!("error.invalid_algorithm_type", value = _0))]
    InvalidAlgorithmType(String),

    #[error(
        "{}",
        t!("error.clock_moved_backward", last_timestamp = last_timestamp)
    )]
    ClockMovedBackward { last_timestamp: u64 },

    #[error("{}", t!("error.sequence_overflow", timestamp = timestamp))]
    SequenceOverflow { timestamp: u64 },

    #[error("{}", t!("error.segment_exhausted", max_id = max_id))]
    SegmentExhausted { max_id: u64 },

    #[error("{}", t!("error.database_error", value = _0))]
    DatabaseError(String),

    #[error("{}", t!("error.cache_error", value = _0))]
    CacheError(String),

    #[error("{}", t!("error.configuration_error", value = _0))]
    ConfigurationError(String),

    #[error("{}", t!("error.authentication_error", value = _0))]
    AuthenticationError(String),

    #[error("{}", t!("error.rate_limit_exceeded"))]
    RateLimitExceeded,

    #[error("{}", t!("error.not_found", value = _0))]
    NotFound(String),

    #[error("{}", t!("error.workspace_disabled", value = _0))]
    WorkspaceDisabled(String),

    #[error("{}", t!("error.biz_tag_not_found", value = _0))]
    BizTagNotFound(String),

    #[error("{}", t!("error.api_key_disabled"))]
    ApiKeyDisabled,

    #[error("{}", t!("error.api_key_expired"))]
    ApiKeyExpired,

    #[error("{}", t!("error.invalid_api_key_signature"))]
    InvalidApiKeySignature,

    #[error("{}", t!("error.etcd_error", value = _0))]
    EtcdError(String),

    #[error("{}", t!("error.parse_error", value = _0))]
    ParseError(String),

    #[error("{}", t!("error.io_error", value = _0))]
    IoError(String),

    #[error("{}", t!("error.timeout_error"))]
    TimeoutError,

    #[error("{}", t!("error.internal_error", value = _0))]
    InternalError(String),

    #[error("{}", t!("error.invalid_input", value = _0))]
    InvalidInput(String),

    #[error("{}", t!("error.unknown"))]
    #[from(ignore)]
    Unknown,
}

impl From<std::num::ParseIntError> for CoreError {
    fn from(e: std::num::ParseIntError) -> Self {
        CoreError::ParseError(e.to_string())
    }
}

impl From<std::io::Error> for CoreError {
    fn from(e: std::io::Error) -> Self {
        CoreError::IoError(e.to_string())
    }
}

impl From<uuid::Error> for CoreError {
    fn from(e: uuid::Error) -> Self {
        CoreError::ParseError(e.to_string())
    }
}

impl From<oxcache::OxCacheError> for CoreError {
    fn from(e: oxcache::OxCacheError) -> Self {
        CoreError::CacheError(e.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub code: i32,
    pub message: String,
    pub details: Option<serde_json::Value>,
}

impl ErrorResponse {
    pub fn new(code: i32, message: String) -> Self {
        Self {
            code,
            message,
            details: None,
        }
    }

    pub fn with_details(code: i32, message: String, details: serde_json::Value) -> Self {
        Self {
            code,
            message,
            details: Some(details),
        }
    }
}

pub type Result<T> = std::result::Result<T, CoreError>;

pub const ERROR_CODE_INVALID_REQUEST: i32 = 400;
pub const ERROR_CODE_UNAUTHORIZED: i32 = 401;
pub const ERROR_CODE_FORBIDDEN: i32 = 403;
pub const ERROR_CODE_NOT_FOUND: i32 = 404;
pub const ERROR_CODE_RATE_LIMIT: i32 = 429;
pub const ERROR_CODE_INTERNAL_ERROR: i32 = 500;
pub const ERROR_CODE_SERVICE_UNAVAILABLE: i32 = 503;

impl CoreError {
    /// Convert CoreError to HTTP status code and error response
    pub fn to_http_response(&self) -> (i32, ErrorResponse) {
        let (status_code, error_code) = match self {
            CoreError::InvalidIdFormat(_)
            | CoreError::InvalidIdString(_)
            | CoreError::InvalidAlgorithmType(_)
            | CoreError::InvalidInput(_)
            | CoreError::ParseError(_) => (ERROR_CODE_INVALID_REQUEST, ERROR_CODE_INVALID_REQUEST),

            CoreError::AuthenticationError(_)
            | CoreError::InvalidApiKeySignature
            | CoreError::ApiKeyDisabled
            | CoreError::ApiKeyExpired => (ERROR_CODE_UNAUTHORIZED, ERROR_CODE_UNAUTHORIZED),

            CoreError::WorkspaceDisabled(_) => (ERROR_CODE_FORBIDDEN, ERROR_CODE_FORBIDDEN),

            CoreError::NotFound(_) | CoreError::BizTagNotFound(_) => {
                (ERROR_CODE_NOT_FOUND, ERROR_CODE_NOT_FOUND)
            }

            CoreError::RateLimitExceeded => (ERROR_CODE_RATE_LIMIT, ERROR_CODE_RATE_LIMIT),

            CoreError::TimeoutError => (
                ERROR_CODE_SERVICE_UNAVAILABLE,
                ERROR_CODE_SERVICE_UNAVAILABLE,
            ),

            CoreError::ClockMovedBackward { .. }
            | CoreError::SequenceOverflow { .. }
            | CoreError::SegmentExhausted { .. }
            | CoreError::DatabaseError(_)
            | CoreError::CacheError(_)
            | CoreError::ConfigurationError(_)
            | CoreError::EtcdError(_)
            | CoreError::IoError(_)
            | CoreError::InternalError(_)
            | CoreError::Unknown => (ERROR_CODE_INTERNAL_ERROR, ERROR_CODE_INTERNAL_ERROR),
        };

        (
            status_code,
            ErrorResponse::new(error_code, self.to_string()),
        )
    }

    /// Get HTTP status code for this error
    pub fn http_status_code(&self) -> i32 {
        self.to_http_response().0
    }

    /// Get error code for this error
    pub fn error_code(&self) -> i32 {
        self.to_http_response().1.code
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify CoreError Display impl delegates to `t!()` lookups.
    /// Covers all 24 variants under "en" (default) locale, plus a
    /// representative subset under "zh-CN" locale.
    ///
    /// Both locales are exercised in a single test function to avoid
    /// parallel `set_locale` races with other tests that may rely on
    /// the default locale.
    #[test]
    fn test_core_error_display_i18n() {
        // --- English locale (default) ---
        rust_i18n::set_locale("en");

        // Positional-arg variants
        assert_eq!(
            CoreError::InvalidIdFormat("test".to_string()).to_string(),
            "Invalid ID format: test"
        );
        assert_eq!(
            CoreError::InvalidIdString("bad".to_string()).to_string(),
            "Invalid ID string: bad"
        );
        assert_eq!(
            CoreError::InvalidAlgorithmType("foo".to_string()).to_string(),
            "Invalid algorithm type: foo"
        );
        assert_eq!(
            CoreError::DatabaseError("conn lost".to_string()).to_string(),
            "Database error: conn lost"
        );
        assert_eq!(
            CoreError::CacheError("miss".to_string()).to_string(),
            "Cache error: miss"
        );
        assert_eq!(
            CoreError::ConfigurationError("bad".to_string()).to_string(),
            "Configuration error: bad"
        );
        assert_eq!(
            CoreError::AuthenticationError("bad token".to_string()).to_string(),
            "Authentication error: bad token"
        );
        assert_eq!(
            CoreError::NotFound("widget".to_string()).to_string(),
            "Resource not found: widget"
        );
        assert_eq!(
            CoreError::WorkspaceDisabled("ws-1".to_string()).to_string(),
            "Workspace disabled: ws-1"
        );
        assert_eq!(
            CoreError::BizTagNotFound("tag-1".to_string()).to_string(),
            "Biz tag not found: tag-1"
        );
        assert_eq!(
            CoreError::EtcdError("no quorum".to_string()).to_string(),
            "Etcd error: no quorum"
        );
        assert_eq!(
            CoreError::ParseError("syntax".to_string()).to_string(),
            "Parse error: syntax"
        );
        assert_eq!(
            CoreError::IoError("eof".to_string()).to_string(),
            "I/O error: eof"
        );
        assert_eq!(
            CoreError::InternalError("boom".to_string()).to_string(),
            "Internal error: boom"
        );
        assert_eq!(
            CoreError::InvalidInput("negative".to_string()).to_string(),
            "Invalid input: negative"
        );

        // Named-arg variants
        assert_eq!(
            CoreError::ClockMovedBackward {
                last_timestamp: 123
            }
            .to_string(),
            "Clock moved backward, last timestamp: 123"
        );
        assert_eq!(
            CoreError::SequenceOverflow { timestamp: 999 }.to_string(),
            "Sequence overflow, timestamp: 999"
        );
        assert_eq!(
            CoreError::SegmentExhausted { max_id: 42 }.to_string(),
            "Segment exhausted, max_id: 42"
        );

        // No-arg variants
        assert_eq!(
            CoreError::RateLimitExceeded.to_string(),
            "Rate limit exceeded"
        );
        assert_eq!(CoreError::ApiKeyDisabled.to_string(), "API key disabled");
        assert_eq!(CoreError::ApiKeyExpired.to_string(), "API key expired");
        assert_eq!(
            CoreError::InvalidApiKeySignature.to_string(),
            "Invalid API key signature"
        );
        assert_eq!(CoreError::TimeoutError.to_string(), "Timeout error");
        assert_eq!(CoreError::Unknown.to_string(), "Unknown error");

        // --- Chinese (zh-CN) locale — representative subset ---
        rust_i18n::set_locale("zh-CN");
        assert_eq!(
            CoreError::InvalidIdFormat("test".to_string()).to_string(),
            "无效的 ID 格式：test"
        );
        assert_eq!(
            CoreError::ClockMovedBackward {
                last_timestamp: 123
            }
            .to_string(),
            "时钟回拨，最后时间戳：123"
        );
        assert_eq!(
            CoreError::SegmentExhausted { max_id: 42 }.to_string(),
            "号段耗尽，max_id：42"
        );
        assert_eq!(CoreError::RateLimitExceeded.to_string(), "速率限制超出");
        assert_eq!(
            CoreError::InvalidApiKeySignature.to_string(),
            "无效的 API 密钥签名"
        );
        assert_eq!(CoreError::Unknown.to_string(), "未知错误");

        // Restore default locale for subsequent parallel tests.
        rust_i18n::set_locale("en");
    }

    /// Verify `to_http_response` populates the message field with the
    /// translated Display string (not a hardcoded English literal).
    #[test]
    fn test_to_http_response_uses_translated_message() {
        // Force default locale in case the i18n test above races ahead.
        rust_i18n::set_locale("en");
        let (status, resp) = CoreError::InvalidIdFormat("xyz".to_string()).to_http_response();
        assert_eq!(status, 400);
        assert_eq!(resp.code, 400);
        assert_eq!(resp.message, "Invalid ID format: xyz");
    }
}
