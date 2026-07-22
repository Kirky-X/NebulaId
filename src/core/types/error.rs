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

impl From<inklog::InklogError> for CoreError {
    fn from(e: inklog::InklogError) -> Self {
        CoreError::ConfigurationError(e.to_string())
    }
}

impl From<trait_kit::TraitKitError> for CoreError {
    fn from(e: trait_kit::TraitKitError) -> Self {
        CoreError::ConfigurationError(e.to_string())
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
    /// Return the i18n key for this error variant.
    ///
    /// Phase 8 T041 (MED-002 + LOW-001 fix) — single source of truth
    /// for the i18n key. The `Display` impl (thiserror `#[error(...)]`)
    /// still embeds the key as a string literal because thiserror
    /// requires `&str` literals in the attribute; new variants must
    /// update both `i18n_key()` and the `#[error(...)]` attribute, but
    /// the args are now produced by a single `i18n_args_ref()` impl
    /// so the value-side can only drift in one place.
    pub fn i18n_key(&self) -> &'static str {
        match self {
            CoreError::InvalidIdFormat(_) => "error.invalid_id_format",
            CoreError::InvalidIdString(_) => "error.invalid_id_string",
            CoreError::InvalidAlgorithmType(_) => "error.invalid_algorithm_type",
            CoreError::ClockMovedBackward { .. } => "error.clock_moved_backward",
            CoreError::SequenceOverflow { .. } => "error.sequence_overflow",
            CoreError::SegmentExhausted { .. } => "error.segment_exhausted",
            CoreError::DatabaseError(_) => "error.database_error",
            CoreError::CacheError(_) => "error.cache_error",
            CoreError::ConfigurationError(_) => "error.configuration_error",
            CoreError::AuthenticationError(_) => "error.authentication_error",
            CoreError::RateLimitExceeded => "error.rate_limit_exceeded",
            CoreError::NotFound(_) => "error.not_found",
            CoreError::WorkspaceDisabled(_) => "error.workspace_disabled",
            CoreError::BizTagNotFound(_) => "error.biz_tag_not_found",
            CoreError::ApiKeyDisabled => "error.api_key_disabled",
            CoreError::ApiKeyExpired => "error.api_key_expired",
            CoreError::InvalidApiKeySignature => "error.invalid_api_key_signature",
            CoreError::EtcdError(_) => "error.etcd_error",
            CoreError::ParseError(_) => "error.parse_error",
            CoreError::IoError(_) => "error.io_error",
            CoreError::TimeoutError => "error.timeout_error",
            CoreError::InternalError(_) => "error.internal_error",
            CoreError::InvalidInput(_) => "error.invalid_input",
            CoreError::Unknown => "error.unknown",
        }
    }

    /// Return the i18n args for this error variant, borrowing from
    /// `self` instead of cloning the inner `String`.
    ///
    /// Phase 8 T041 (M3 perf + LOW-001 fix) — returns
    /// `SmallVec<[(&'static str, Cow<'_, str>); 4]>` so the typical
    /// 1-arg case lives entirely on the stack. Variants whose payload
    /// is a `String` return `Cow::Borrowed` (zero-clone); variants
    /// whose payload is a numeric type return `Cow::Owned` (one
    /// `String` allocation for the numeric text, same as before).
    pub fn i18n_args(&self) -> smallvec::SmallVec<[(&'static str, std::borrow::Cow<'_, str>); 4]> {
        use smallvec::smallvec;
        use std::borrow::Cow;
        match self {
            CoreError::InvalidIdFormat(s) => smallvec![("value", Cow::Borrowed(s.as_str()))],
            CoreError::InvalidIdString(s) => smallvec![("value", Cow::Borrowed(s.as_str()))],
            CoreError::InvalidAlgorithmType(s) => {
                smallvec![("value", Cow::Borrowed(s.as_str()))]
            }
            CoreError::ClockMovedBackward { last_timestamp } => {
                smallvec![("last_timestamp", Cow::Owned(last_timestamp.to_string()))]
            }
            CoreError::SequenceOverflow { timestamp } => {
                smallvec![("timestamp", Cow::Owned(timestamp.to_string()))]
            }
            CoreError::SegmentExhausted { max_id } => {
                smallvec![("max_id", Cow::Owned(max_id.to_string()))]
            }
            CoreError::DatabaseError(s) => smallvec![("value", Cow::Borrowed(s.as_str()))],
            CoreError::CacheError(s) => smallvec![("value", Cow::Borrowed(s.as_str()))],
            CoreError::ConfigurationError(s) => {
                smallvec![("value", Cow::Borrowed(s.as_str()))]
            }
            CoreError::AuthenticationError(s) => {
                smallvec![("value", Cow::Borrowed(s.as_str()))]
            }
            CoreError::RateLimitExceeded => smallvec![],
            CoreError::NotFound(s) => smallvec![("value", Cow::Borrowed(s.as_str()))],
            CoreError::WorkspaceDisabled(s) => smallvec![("value", Cow::Borrowed(s.as_str()))],
            CoreError::BizTagNotFound(s) => smallvec![("value", Cow::Borrowed(s.as_str()))],
            CoreError::ApiKeyDisabled => smallvec![],
            CoreError::ApiKeyExpired => smallvec![],
            CoreError::InvalidApiKeySignature => smallvec![],
            CoreError::EtcdError(s) => smallvec![("value", Cow::Borrowed(s.as_str()))],
            CoreError::ParseError(s) => smallvec![("value", Cow::Borrowed(s.as_str()))],
            CoreError::IoError(s) => smallvec![("value", Cow::Borrowed(s.as_str()))],
            CoreError::TimeoutError => smallvec![],
            CoreError::InternalError(s) => smallvec![("value", Cow::Borrowed(s.as_str()))],
            CoreError::InvalidInput(s) => smallvec![("value", Cow::Borrowed(s.as_str()))],
            CoreError::Unknown => smallvec![],
        }
    }

    /// Return the localized Display string for this error under the given locale.
    ///
    /// Unlike `to_string()` (which uses the global locale set via `set_locale`),
    /// this method performs per-call translation and is safe for concurrent
    /// use across requests with different `Accept-Language` headers.
    ///
    /// Phase 8 T041 — used by HTTP handlers to translate error responses
    /// based on the negotiated `Locale` from `locale_middleware`.
    ///
    /// Phase 8 T041 (MED-002 + M3 perf fix) — delegates to
    /// `i18n_key()` + `i18n_args()` so the key/args mapping has a
    /// single source of truth, and routes through
    /// `translate_with_locale_args_cow` (SmallVec + `Cow<str>`) to
    /// avoid the per-call `Vec` + `String::clone` allocations the
    /// previous `Vec<(&str, String)>` impl performed unconditionally.
    pub fn to_localized_string(&self, locale: &str) -> String {
        let key = self.i18n_key();
        let args = self.i18n_args();
        crate::core::i18n::translate_with_locale_args_cow(locale, key, &args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// 串行化所有调用 `rust_i18n::set_locale` 的测试，避免并行 set_locale
    /// 竞态导致 `to_string()`（依赖全局 locale）读到被其他测试改写的值。
    static LOCALE_LOCK: Mutex<()> = Mutex::new(());

    /// Verify CoreError Display impl delegates to `t!()` lookups.
    /// Covers all 24 variants under "en" (default) locale, plus a
    /// representative subset under "zh-CN" locale.
    ///
    /// Both locales are exercised in a single test function to avoid
    /// parallel `set_locale` races with other tests that may rely on
    /// the default locale.
    #[test]
    fn test_core_error_display_i18n() {
        let _locale_lock = LOCALE_LOCK.lock().unwrap();
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

    /// Verify `to_localized_string` returns per-locale translations
    /// without mutating global locale state (Phase 8 T041).
    #[test]
    fn test_to_localized_string_per_locale() {
        let _locale_lock = LOCALE_LOCK.lock().unwrap();
        // Pin global locale to en to detect any accidental state coupling.
        rust_i18n::set_locale("en");

        // English
        assert_eq!(
            CoreError::InvalidInput("negative".to_string()).to_localized_string("en"),
            "Invalid input: negative"
        );
        // Chinese (Simplified)
        assert_eq!(
            CoreError::InvalidInput("negative".to_string()).to_localized_string("zh-CN"),
            "无效输入：negative"
        );
        // Global locale must remain "en" after the zh-CN call
        assert_eq!(&*rust_i18n::locale(), "en");
    }

    /// Verify `to_localized_string` covers all variants (no panic, no
    /// empty string). Exercises named-arg and no-arg variants too.
    #[test]
    fn test_to_localized_string_all_variants_en() {
        let _locale_lock = LOCALE_LOCK.lock().unwrap();
        rust_i18n::set_locale("en");

        assert_eq!(
            CoreError::InvalidIdFormat("v".to_string()).to_localized_string("en"),
            "Invalid ID format: v"
        );
        assert_eq!(
            CoreError::InvalidIdString("v".to_string()).to_localized_string("en"),
            "Invalid ID string: v"
        );
        assert_eq!(
            CoreError::InvalidAlgorithmType("v".to_string()).to_localized_string("en"),
            "Invalid algorithm type: v"
        );
        assert_eq!(
            CoreError::ClockMovedBackward { last_timestamp: 7 }.to_localized_string("en"),
            "Clock moved backward, last timestamp: 7"
        );
        assert_eq!(
            CoreError::SequenceOverflow { timestamp: 7 }.to_localized_string("en"),
            "Sequence overflow, timestamp: 7"
        );
        assert_eq!(
            CoreError::SegmentExhausted { max_id: 7 }.to_localized_string("en"),
            "Segment exhausted, max_id: 7"
        );
        assert_eq!(
            CoreError::DatabaseError("v".to_string()).to_localized_string("en"),
            "Database error: v"
        );
        assert_eq!(
            CoreError::CacheError("v".to_string()).to_localized_string("en"),
            "Cache error: v"
        );
        assert_eq!(
            CoreError::ConfigurationError("v".to_string()).to_localized_string("en"),
            "Configuration error: v"
        );
        assert_eq!(
            CoreError::AuthenticationError("v".to_string()).to_localized_string("en"),
            "Authentication error: v"
        );
        assert_eq!(
            CoreError::RateLimitExceeded.to_localized_string("en"),
            "Rate limit exceeded"
        );
        assert_eq!(
            CoreError::NotFound("v".to_string()).to_localized_string("en"),
            "Resource not found: v"
        );
        assert_eq!(
            CoreError::WorkspaceDisabled("v".to_string()).to_localized_string("en"),
            "Workspace disabled: v"
        );
        assert_eq!(
            CoreError::BizTagNotFound("v".to_string()).to_localized_string("en"),
            "Biz tag not found: v"
        );
        assert_eq!(
            CoreError::ApiKeyDisabled.to_localized_string("en"),
            "API key disabled"
        );
        assert_eq!(
            CoreError::ApiKeyExpired.to_localized_string("en"),
            "API key expired"
        );
        assert_eq!(
            CoreError::InvalidApiKeySignature.to_localized_string("en"),
            "Invalid API key signature"
        );
        assert_eq!(
            CoreError::EtcdError("v".to_string()).to_localized_string("en"),
            "Etcd error: v"
        );
        assert_eq!(
            CoreError::ParseError("v".to_string()).to_localized_string("en"),
            "Parse error: v"
        );
        assert_eq!(
            CoreError::IoError("v".to_string()).to_localized_string("en"),
            "I/O error: v"
        );
        assert_eq!(
            CoreError::TimeoutError.to_localized_string("en"),
            "Timeout error"
        );
        assert_eq!(
            CoreError::InternalError("v".to_string()).to_localized_string("en"),
            "Internal error: v"
        );
        assert_eq!(
            CoreError::Unknown.to_localized_string("en"),
            "Unknown error"
        );
    }

    /// LOW L-7 — `to_localized_string` for an unsupported locale must
    /// not panic and must fall back to the crate's fallback locale
    /// (`en`, configured via `i18n!("locales", fallback = "en")` in
    /// `lib.rs`). Verifies the fallback is consistent across multiple
    /// unsupported locales (fr, ja, de, xx-XX).
    #[test]
    fn test_to_localized_string_unsupported_locale_falls_back_to_default() {
        let _locale_lock = LOCALE_LOCK.lock().unwrap();
        let _g = LocaleGuard::new();
        rust_i18n::set_locale("en");

        let err = CoreError::InvalidIdFormat("test".to_string());
        let en_msg = err.to_localized_string("en");

        // French is not shipped (only en + zh-CN exist). rust-i18n's
        // fallback chain resolves unknown locales to the fallback
        // locale ("en").
        let fr_msg = err.to_localized_string("fr");
        assert!(
            !fr_msg.is_empty(),
            "unsupported locale must not produce an empty message"
        );
        assert_eq!(
            fr_msg, en_msg,
            "unsupported locale (fr) must fall back to en, got fr={:?} en={:?}",
            fr_msg, en_msg
        );

        // Japanese — also unsupported.
        let ja_msg = err.to_localized_string("ja");
        assert_eq!(
            ja_msg, en_msg,
            "unsupported locale (ja) must fall back to en, got ja={:?} en={:?}",
            ja_msg, en_msg
        );

        // German — also unsupported.
        let de_msg = err.to_localized_string("de-DE");
        assert_eq!(de_msg, en_msg, "de-DE must fall back to en");

        // Completely bogus locale string — still must not panic and
        // must fall back to en.
        let bogus_msg = err.to_localized_string("xx-XX");
        assert_eq!(bogus_msg, en_msg, "xx-XX must fall back to en");

        // Empty locale — must not panic; rust-i18n treats empty as
        // unknown and falls back to en.
        let empty_msg = err.to_localized_string("");
        assert_eq!(empty_msg, en_msg, "empty locale must fall back to en");
    }

    /// LOW L-7 — fallback consistency for a variant with no args
    /// (`RateLimitExceeded`) and a variant with named args
    /// (`ClockMovedBackward`). Ensures the fallback path handles
    /// both arg shapes.
    #[test]
    fn test_to_localized_string_ja_falls_back_to_en() {
        let _locale_lock = LOCALE_LOCK.lock().unwrap();
        let _g = LocaleGuard::new();
        rust_i18n::set_locale("en");

        let err = CoreError::DatabaseError("db err".to_string());
        let ja_msg = err.to_localized_string("ja");
        let en_msg = err.to_localized_string("en");
        assert_eq!(ja_msg, en_msg);

        let err = CoreError::ClockMovedBackward { last_timestamp: 99 };
        let ja_msg = err.to_localized_string("ja");
        let en_msg = err.to_localized_string("en");
        assert_eq!(ja_msg, en_msg);

        let err = CoreError::RateLimitExceeded;
        let ja_msg = err.to_localized_string("ja");
        let en_msg = err.to_localized_string("en");
        assert_eq!(ja_msg, en_msg);
    }

    /// Test-only helper to save/restore the global locale around tests
    /// that call `rust_i18n::set_locale`.
    struct LocaleGuard {
        saved: String,
    }

    impl LocaleGuard {
        fn new() -> Self {
            Self {
                saved: rust_i18n::locale().to_string(),
            }
        }
    }

    impl Drop for LocaleGuard {
        fn drop(&mut self) {
            rust_i18n::set_locale(&self.saved);
        }
    }
}
