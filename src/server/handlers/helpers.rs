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

//! Internal error-mapping helpers shared across handler sub-modules.
//!
//! Phase 8 T041 — the helpers in this file produce locale-translated
//! `ErrorResponse` payloads by reading the `Locale` negotiated by
//! `locale_middleware` (from the `Accept-Language` header). They are the
//! single entry point for `CoreError → HTTP response` conversion in
//! `router.rs`, ensuring consistent status codes and i18n coverage.
//!
//! # Style guide (LOW-004)
//!
//! This file intentionally uses two translation call styles:
//! - For errors derived from `CoreError`, use
//!   `e.to_localized_string(locale.as_str())` which delegates to
//!   `i18n_key()` + `i18n_args()` + `translate_with_locale_args_cow`.
//!   This is the path used by `core_error_to_response` for 4xx variants
//!   that carry a caller-supplied `String` payload.
//! - For handler-constructed errors (UUID parse failure, validation
//!   failure, workspace mismatch, workspace-name-not-found, etc.) where
//!   there is no `CoreError` instance to dispatch on, call
//!   `translate_with_locale_args(locale.as_str(), key, &args)` (or
//!   `translate_with_locale` for the no-args case) directly with the
//!   `api.error.*` key.
//! - For generic 5xx errors (`database_error`, `internal_error`, etc.),
//!   call `translate_with_locale(locale.as_str(), key)` — no args.
//!
//! The two styles exist because `CoreError::i18n_key()`/`i18n_args()`
//! only model the `error.*` namespace (Display strings); handler-constructed
//! errors live in the `api.error.*` namespace and have no `CoreError`
//! variant to dispatch on. Mixing them within a single helper would
//! require either a fake `CoreError` variant or a separate trait — both
//! add complexity without value.

use crate::core::i18n::{translate_with_locale, translate_with_locale_args};
use crate::core::CoreError;
use crate::server::middleware::locale::Locale;
use crate::server::models::ErrorResponse;
use axum::http::StatusCode;
use axum::Json;

/// Convert database errors to `CoreError::DatabaseError`.
pub(super) fn map_db_error<E: std::fmt::Display>(error: E) -> CoreError {
    CoreError::DatabaseError(error.to_string())
}

/// Convert UUID parse errors to `CoreError::InvalidInput`.
pub(super) fn map_uuid_error<E: std::fmt::Display>(error: E) -> CoreError {
    CoreError::InvalidInput(format!("Invalid UUID: {}", error))
}

/// HTTP status code for a `CoreError` variant.
///
/// Phase 8 T041 (LOW L-1 + L6 fix) — this is the single source of
/// truth for the `CoreError → axum::http::StatusCode` mapping. The
/// old `CoreError::to_http_response` / `http_status_code` /
/// `error_code` methods (which consulted the process-wide global
/// locale via `to_string()`) were removed as dead code; all
/// locale-aware translation now goes through `to_localized_string`
/// + the helpers below.
fn core_error_status_code(e: &CoreError) -> StatusCode {
    match e {
        CoreError::InvalidIdFormat(_)
        | CoreError::InvalidIdString(_)
        | CoreError::InvalidAlgorithmType(_)
        | CoreError::InvalidInput(_)
        | CoreError::ParseError(_) => StatusCode::BAD_REQUEST,
        CoreError::AuthenticationError(_)
        | CoreError::InvalidApiKeySignature
        | CoreError::ApiKeyDisabled
        | CoreError::ApiKeyExpired => StatusCode::UNAUTHORIZED,
        CoreError::WorkspaceDisabled(_) => StatusCode::FORBIDDEN,
        CoreError::NotFound(_) | CoreError::BizTagNotFound(_) => StatusCode::NOT_FOUND,
        CoreError::RateLimitExceeded => StatusCode::TOO_MANY_REQUESTS,
        CoreError::TimeoutError => StatusCode::SERVICE_UNAVAILABLE,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

/// Maximum message length returned to clients for 4xx-class errors.
///
/// Phase 8 T041 (CRITICAL C-1 / HIGH H-1 fix) — guards against
/// information disclosure and DoS when a 4xx `CoreError` embeds a
/// long caller-controlled string (e.g. `InvalidInput(user_input)`).
const MAX_CLIENT_MESSAGE_LEN: usize = 200;

/// Truncate a message to `MAX_CLIENT_MESSAGE_LEN` bytes, appending an
/// explicit truncation marker when cut.
///
/// SECURITY: this is the last-resort length filter applied to 4xx
/// responses whose underlying `CoreError` carries a caller-supplied
/// `String`. 5xx errors bypass this entirely — they return a fixed
/// generic message via [`core_error_to_response`].
fn sanitize_for_production(msg: &str) -> String {
    if msg.len() > MAX_CLIENT_MESSAGE_LEN {
        // `msg` originates from a UTF-8 `String` so slicing at a byte
        // boundary <= 200 is not always char-safe; round down to the
        // nearest char boundary to never panic on multi-byte sequences.
        let mut end = MAX_CLIENT_MESSAGE_LEN;
        while end > 0 && !msg.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}... (truncated)", &msg[..end])
    } else {
        msg.to_string()
    }
}

/// Convert `CoreError` to `(StatusCode, Json<ErrorResponse>)` with a
/// locale-translated message.
///
/// Phase 8 T041 (CRITICAL C-1 / HIGH H-1 fix) — the response message
/// is chosen per variant:
///
/// - **5xx-class internal errors** (`DatabaseError`, `CacheError`,
///   `InternalError`, `ConfigurationError`, `EtcdError`, `IoError`,
///   `ClockMovedBackward`, `SequenceOverflow`, `SegmentExhausted`,
///   `Unknown`): the full error (including the inner `String` which may
///   carry DB URLs, file paths, or stack traces) is recorded server-side
///   via `tracing::error!`. The client only sees a fixed generic message
///   looked up under `api.error.<variant>` — never the raw `String`.
/// - **4xx-class errors with caller-supplied `String`** (`InvalidInput`,
///   `NotFound`, `BizTagNotFound`, `AuthenticationError`,
///   `WorkspaceDisabled`, `InvalidIdFormat`, `InvalidIdString`,
///   `InvalidAlgorithmType`, `ParseError`): the localized message is
///   generated via `CoreError::to_localized_string` and then run through
///   [`sanitize_for_production`] to cap length at
///   `MAX_CLIENT_MESSAGE_LEN` bytes.
/// - **4xx-class errors without inner `String`**
///   (`RateLimitExceeded`, `ApiKeyDisabled`, `ApiKeyExpired`,
///   `InvalidApiKeySignature`, `TimeoutError`): the localized message is
///   returned verbatim — there is no caller-controlled content to filter.
pub(crate) fn core_error_to_response(
    e: &CoreError,
    locale: Locale,
) -> (StatusCode, Json<ErrorResponse>) {
    let status = core_error_status_code(e);

    // 5xx-class internal errors — log full detail server-side, return
    // generic locale-translated message to the client.
    let message = match e {
        CoreError::DatabaseError(_) => {
            tracing::error!(
                event = "core_error",
                variant = "database_error",
                error = ?e,
                "database error returned to client as generic message"
            );
            translate_with_locale(locale.as_str(), "api.error.database_error")
        }
        CoreError::CacheError(_) => {
            tracing::error!(
                event = "core_error",
                variant = "cache_error",
                error = ?e,
                "cache error returned to client as generic message"
            );
            translate_with_locale(locale.as_str(), "api.error.cache_error")
        }
        CoreError::InternalError(_) => {
            tracing::error!(
                event = "core_error",
                variant = "internal_error",
                error = ?e,
                "internal error returned to client as generic message"
            );
            translate_with_locale(locale.as_str(), "api.error.internal_error")
        }
        CoreError::ConfigurationError(_) => {
            tracing::error!(
                event = "core_error",
                variant = "configuration_error",
                error = ?e,
                "configuration error returned to client as generic message"
            );
            translate_with_locale(locale.as_str(), "api.error.configuration_error")
        }
        CoreError::EtcdError(_) => {
            tracing::error!(
                event = "core_error",
                variant = "etcd_error",
                error = ?e,
                "etcd error returned to client as generic message"
            );
            translate_with_locale(locale.as_str(), "api.error.etcd_error")
        }
        CoreError::IoError(_) => {
            tracing::error!(
                event = "core_error",
                variant = "io_error",
                error = ?e,
                "I/O error returned to client as generic message"
            );
            translate_with_locale(locale.as_str(), "api.error.io_error")
        }
        CoreError::ClockMovedBackward { .. }
        | CoreError::SequenceOverflow { .. }
        | CoreError::SegmentExhausted { .. } => {
            tracing::error!(
                event = "core_error",
                variant = "algorithm_error",
                error = ?e,
                "algorithm error returned to client as generic message"
            );
            translate_with_locale(locale.as_str(), "api.error.algorithm_error")
        }
        CoreError::Unknown => {
            tracing::error!(
                event = "core_error",
                variant = "unknown",
                error = ?e,
                "unknown error returned to client as generic message"
            );
            translate_with_locale(locale.as_str(), "api.error.internal_error")
        }
        // 4xx-class with caller-supplied String — localize, then cap
        // length via `sanitize_for_production`.
        CoreError::InvalidInput(_)
        | CoreError::NotFound(_)
        | CoreError::BizTagNotFound(_)
        | CoreError::AuthenticationError(_)
        | CoreError::WorkspaceDisabled(_)
        | CoreError::InvalidIdFormat(_)
        | CoreError::InvalidIdString(_)
        | CoreError::InvalidAlgorithmType(_)
        | CoreError::ParseError(_) => {
            let msg = e.to_localized_string(locale.as_str());
            sanitize_for_production(&msg)
        }
        // 4xx-class without inner String — no caller-controlled content,
        // no truncation needed.
        CoreError::RateLimitExceeded
        | CoreError::ApiKeyDisabled
        | CoreError::ApiKeyExpired
        | CoreError::InvalidApiKeySignature
        | CoreError::TimeoutError => e.to_localized_string(locale.as_str()),
    };

    let code = status.as_u16() as i32;
    (status, Json(ErrorResponse::new(code, message)))
}

/// Build a 400 response for an invalid UUID path parameter, with the
/// locale-translated message.
pub(crate) fn invalid_uuid_response(locale: Locale) -> (StatusCode, Json<ErrorResponse>) {
    let message = translate_with_locale(locale.as_str(), "api.error.invalid_uuid_format");
    (
        StatusCode::BAD_REQUEST,
        Json(ErrorResponse::new(400, message)),
    )
}

/// Build a 400 response for `validator::ValidationErrors`, with the
/// locale-translated message.
///
/// Phase 8 T041 (MEDIUM M-1 fix) — uses `errors.field_errors()` to
/// extract structured `(field, rule)` pairs instead of stringifying
/// the entire `ValidationErrors`. This avoids leaking internal
/// constraint values (e.g. `length [min = 1, max = 64]`,
/// `range [min = 100, max = 1000000]`) which `ValidationErrors::to_string()`
/// would otherwise expose via `ValidationError::params` formatting.
///
/// Each field error contributes one localized entry of the form
/// `"Validation error in field: <field> (<rule>)"`; multiple entries
/// are joined by `"; "` and capped at `MAX_CLIENT_MESSAGE_LEN` bytes
/// via `sanitize_for_production` to bound response size.
pub(crate) fn validation_error_response(
    errors: &validator::ValidationErrors,
    locale: Locale,
) -> (StatusCode, Json<ErrorResponse>) {
    let mut parts: Vec<String> = Vec::new();
    for (field, field_errs) in errors.field_errors() {
        for err in field_errs {
            // `err.code` is the rule name (e.g. "required", "length",
            // "range"). It carries no constraint values, only the rule
            // identifier — safe to surface to the client.
            let rule = err.code.as_ref();
            let msg = translate_with_locale_args(
                locale.as_str(),
                "api.error.validation_error_field",
                &[("field", field.to_string()), ("rule", rule.to_string())],
            );
            parts.push(msg);
        }
    }
    let message = if parts.is_empty() {
        // Fallback: no field-level errors extracted (e.g. only nested
        // struct errors). Use the generic key with an empty error
        // string so the client still sees a localized message.
        translate_with_locale_args(
            locale.as_str(),
            "api.error.validation_error",
            &[("error", String::new())],
        )
    } else {
        sanitize_for_production(&parts.join("; "))
    };

    (
        StatusCode::BAD_REQUEST,
        Json(ErrorResponse::new(400, message)),
    )
}

/// Build a 403 response for "Admin API key cannot perform this operation".
pub(crate) fn admin_cannot_perform_response(locale: Locale) -> (StatusCode, Json<ErrorResponse>) {
    let message = translate_with_locale(locale.as_str(), "api.error.admin_cannot_perform");
    (
        StatusCode::FORBIDDEN,
        Json(ErrorResponse::new(403, message)),
    )
}

/// Build a 403 response for "Access denied: workspace mismatch".
pub(crate) fn workspace_mismatch_response(locale: Locale) -> (StatusCode, Json<ErrorResponse>) {
    let message = translate_with_locale(locale.as_str(), "api.error.workspace_mismatch");
    (
        StatusCode::FORBIDDEN,
        Json(ErrorResponse::new(403, message)),
    )
}

/// Build a 404 response for "Workspace '<name>' not found".
///
/// Phase 8 T041 (LOW L-4 fix) — `name` originates from a URL path
/// parameter and is caller-controlled. JSON serialization already
/// escapes special characters (no XSS risk), but a pathologically
/// long `name` could inflate response size or be logged unescaped
/// downstream (log injection). We cap `name` at
/// `MAX_WORKSPACE_NAME_LEN` bytes (char-boundary-safe truncation)
/// before interpolation so the response message is bounded.
pub(crate) fn workspace_name_not_found_response(
    name: &str,
    locale: Locale,
) -> (StatusCode, Json<ErrorResponse>) {
    const MAX_WORKSPACE_NAME_LEN: usize = 64;
    let safe_name: String = if name.len() > MAX_WORKSPACE_NAME_LEN {
        // Round down to a char boundary to avoid splitting a
        // multi-byte UTF-8 codepoint.
        let mut end = MAX_WORKSPACE_NAME_LEN;
        while end > 0 && !name.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &name[..end])
    } else {
        name.to_string()
    };
    let message = translate_with_locale_args(
        locale.as_str(),
        "api.error.workspace_name_not_found",
        &[("name", safe_name)],
    );
    (
        StatusCode::NOT_FOUND,
        Json(ErrorResponse::new(404, message)),
    )
}

/// Build a 404 response for "Workspace not found" (no name).
pub(crate) fn workspace_not_found_response(locale: Locale) -> (StatusCode, Json<ErrorResponse>) {
    let message = translate_with_locale(locale.as_str(), "api.error.workspace_not_found");
    (
        StatusCode::NOT_FOUND,
        Json(ErrorResponse::new(404, message)),
    )
}

/// Build a 500 response for "Invalid workspace ID".
pub(crate) fn invalid_workspace_id_response(locale: Locale) -> (StatusCode, Json<ErrorResponse>) {
    let message = translate_with_locale(locale.as_str(), "api.error.invalid_workspace_id");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse::new(500, message)),
    )
}

/// Build a 400 response for "workspace_id is required for user keys".
pub(crate) fn workspace_id_required_response(locale: Locale) -> (StatusCode, Json<ErrorResponse>) {
    let message = translate_with_locale(locale.as_str(), "api.error.workspace_id_required");
    (
        StatusCode::BAD_REQUEST,
        Json(ErrorResponse::new(400, message)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::error::CoreError;
    use validator::Validate;

    /// Save and restore the global locale around tests.
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

    #[test]
    fn test_core_error_to_response_en() {
        let _g = LocaleGuard::new();
        rust_i18n::set_locale("en");

        let (status, json) =
            core_error_to_response(&CoreError::InvalidInput("negative".to_string()), Locale::En);
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json.code, 400);
        assert_eq!(json.message, "Invalid input: negative");
    }

    #[test]
    fn test_core_error_to_response_zh_cn() {
        let _g = LocaleGuard::new();
        rust_i18n::set_locale("en");

        let (status, json) = core_error_to_response(
            &CoreError::InvalidInput("negative".to_string()),
            Locale::ZhCn,
        );
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json.code, 400);
        assert_eq!(json.message, "无效输入：negative");

        // Global locale must remain "en"
        assert_eq!(&*rust_i18n::locale(), "en");
    }

    #[test]
    fn test_core_error_to_response_status_codes() {
        let _g = LocaleGuard::new();
        rust_i18n::set_locale("en");

        // 400
        let (s, _) = core_error_to_response(&CoreError::InvalidInput("x".to_string()), Locale::En);
        assert_eq!(s, StatusCode::BAD_REQUEST);
        let (s, _) =
            core_error_to_response(&CoreError::InvalidIdFormat("x".to_string()), Locale::En);
        assert_eq!(s, StatusCode::BAD_REQUEST);
        let (s, _) = core_error_to_response(
            &CoreError::InvalidAlgorithmType("x".to_string()),
            Locale::En,
        );
        assert_eq!(s, StatusCode::BAD_REQUEST);
        let (s, _) =
            core_error_to_response(&CoreError::InvalidIdString("x".to_string()), Locale::En);
        assert_eq!(s, StatusCode::BAD_REQUEST);
        let (s, _) = core_error_to_response(&CoreError::ParseError("x".to_string()), Locale::En);
        assert_eq!(s, StatusCode::BAD_REQUEST);

        // 401
        let (s, _) =
            core_error_to_response(&CoreError::AuthenticationError("x".to_string()), Locale::En);
        assert_eq!(s, StatusCode::UNAUTHORIZED);
        let (s, _) = core_error_to_response(&CoreError::ApiKeyDisabled, Locale::En);
        assert_eq!(s, StatusCode::UNAUTHORIZED);
        let (s, _) = core_error_to_response(&CoreError::ApiKeyExpired, Locale::En);
        assert_eq!(s, StatusCode::UNAUTHORIZED);
        let (s, _) = core_error_to_response(&CoreError::InvalidApiKeySignature, Locale::En);
        assert_eq!(s, StatusCode::UNAUTHORIZED);

        // 403
        let (s, _) =
            core_error_to_response(&CoreError::WorkspaceDisabled("x".to_string()), Locale::En);
        assert_eq!(s, StatusCode::FORBIDDEN);

        // 404
        let (s, _) = core_error_to_response(&CoreError::NotFound("x".to_string()), Locale::En);
        assert_eq!(s, StatusCode::NOT_FOUND);
        let (s, _) =
            core_error_to_response(&CoreError::BizTagNotFound("x".to_string()), Locale::En);
        assert_eq!(s, StatusCode::NOT_FOUND);

        // 429
        let (s, _) = core_error_to_response(&CoreError::RateLimitExceeded, Locale::En);
        assert_eq!(s, StatusCode::TOO_MANY_REQUESTS);

        // 503
        let (s, _) = core_error_to_response(&CoreError::TimeoutError, Locale::En);
        assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);

        // 500 — 5xx-class variants (full coverage)
        let (s, _) = core_error_to_response(&CoreError::DatabaseError("x".to_string()), Locale::En);
        assert_eq!(s, StatusCode::INTERNAL_SERVER_ERROR);
        let (s, _) = core_error_to_response(&CoreError::CacheError("x".to_string()), Locale::En);
        assert_eq!(s, StatusCode::INTERNAL_SERVER_ERROR);
        let (s, _) = core_error_to_response(&CoreError::InternalError("x".to_string()), Locale::En);
        assert_eq!(s, StatusCode::INTERNAL_SERVER_ERROR);
        let (s, _) =
            core_error_to_response(&CoreError::ConfigurationError("x".to_string()), Locale::En);
        assert_eq!(s, StatusCode::INTERNAL_SERVER_ERROR);
        let (s, _) = core_error_to_response(&CoreError::EtcdError("x".to_string()), Locale::En);
        assert_eq!(s, StatusCode::INTERNAL_SERVER_ERROR);
        let (s, _) = core_error_to_response(&CoreError::IoError("x".to_string()), Locale::En);
        assert_eq!(s, StatusCode::INTERNAL_SERVER_ERROR);
        let (s, _) = core_error_to_response(
            &CoreError::ClockMovedBackward { last_timestamp: 1 },
            Locale::En,
        );
        assert_eq!(s, StatusCode::INTERNAL_SERVER_ERROR);
        let (s, _) =
            core_error_to_response(&CoreError::SequenceOverflow { timestamp: 1 }, Locale::En);
        assert_eq!(s, StatusCode::INTERNAL_SERVER_ERROR);
        let (s, _) = core_error_to_response(&CoreError::SegmentExhausted { max_id: 1 }, Locale::En);
        assert_eq!(s, StatusCode::INTERNAL_SERVER_ERROR);
        let (s, _) = core_error_to_response(&CoreError::Unknown, Locale::En);
        assert_eq!(s, StatusCode::INTERNAL_SERVER_ERROR);
    }

    /// CRITICAL C-1 / HIGH H-1 — 5xx internal errors MUST NOT leak the
    /// raw `CoreError` inner `String` to the client. The full error is
    /// logged server-side via `tracing::error!`; the client only sees a
    /// generic locale-translated message.
    #[test]
    fn test_core_error_to_response_5xx_does_not_leak_internal_string() {
        let _g = LocaleGuard::new();
        rust_i18n::set_locale("en");

        // A sensitive DB URL embedded in DatabaseError — typical of what
        // an upstream diesel/sqlx error would stringify to.
        let sensitive = "postgres://user:pwd@internal-host:5432/nebulaid";
        let (status, json) =
            core_error_to_response(&CoreError::DatabaseError(sensitive.to_string()), Locale::En);
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(json.code, 500);
        assert_eq!(json.message, "Database operation failed");
        assert!(
            !json.message.contains(sensitive),
            "5xx response must not contain the raw DatabaseError string"
        );
        assert!(
            !json.message.contains("postgres://"),
            "5xx response must not contain the DB scheme"
        );
        assert!(
            !json.message.contains("pwd"),
            "5xx response must not contain the DB password"
        );

        // zh-CN locale returns the translated generic message.
        let (_, json_zh) = core_error_to_response(
            &CoreError::DatabaseError(sensitive.to_string()),
            Locale::ZhCn,
        );
        assert_eq!(json_zh.message, "数据库操作失败");
        assert!(!json_zh.message.contains(sensitive));

        // CacheError — sensitive Redis URL
        let redis_url = "redis://:secret@cache.internal:6379/0";
        let (_, json) =
            core_error_to_response(&CoreError::CacheError(redis_url.to_string()), Locale::En);
        assert_eq!(json.message, "Cache service unavailable");
        assert!(!json.message.contains(redis_url));
        assert!(!json.message.contains("secret"));

        // InternalError — sensitive file path
        let path = "/etc/nebulaid/secrets/admin.key";
        let (_, json) =
            core_error_to_response(&CoreError::InternalError(path.to_string()), Locale::En);
        assert_eq!(json.message, "Internal server error");
        assert!(!json.message.contains(path));

        // ConfigurationError — sensitive env var value
        let env_leak = "NEBULA_DATABASE_PASSWORD=hunter2";
        let (_, json) = core_error_to_response(
            &CoreError::ConfigurationError(env_leak.to_string()),
            Locale::En,
        );
        assert_eq!(json.message, "Configuration error");
        assert!(!json.message.contains("hunter2"));
        assert!(!json.message.contains("NEBULA_DATABASE_PASSWORD"));

        // EtcdError — sensitive endpoint
        let etcd_url = "http://etcd.internal:2379, token=admin-token";
        let (_, json) =
            core_error_to_response(&CoreError::EtcdError(etcd_url.to_string()), Locale::En);
        assert_eq!(json.message, "Etcd service unavailable");
        assert!(!json.message.contains(etcd_url));
        assert!(!json.message.contains("admin-token"));

        // IoError — sensitive fs path
        let fs_path = "/var/lib/nebulaid/private/keys/0xdeadbeef.pem";
        let (_, json) =
            core_error_to_response(&CoreError::IoError(fs_path.to_string()), Locale::En);
        assert_eq!(json.message, "I/O error");
        assert!(!json.message.contains(fs_path));

        // Algorithm errors — ClockMovedBackward / SequenceOverflow /
        // SegmentExhausted carry numeric context only, but verify the
        // generic message and that the numeric value is not surfaced.
        let (_, json) = core_error_to_response(
            &CoreError::ClockMovedBackward {
                last_timestamp: 1700000000000,
            },
            Locale::En,
        );
        assert_eq!(json.message, "ID generation algorithm error");
        assert!(!json.message.contains("1700000000000"));

        let (_, json) = core_error_to_response(
            &CoreError::SequenceOverflow {
                timestamp: 9999999999999,
            },
            Locale::En,
        );
        assert_eq!(json.message, "ID generation algorithm error");
        assert!(!json.message.contains("9999999999999"));

        let (_, json) = core_error_to_response(
            &CoreError::SegmentExhausted { max_id: 123456789 },
            Locale::En,
        );
        assert_eq!(json.message, "ID generation algorithm error");
        assert!(!json.message.contains("123456789"));

        // Unknown — no inner string, still returns generic message
        let (_, json) = core_error_to_response(&CoreError::Unknown, Locale::En);
        assert_eq!(json.message, "Internal server error");
    }

    /// HIGH H-1 — 4xx-class errors with caller-supplied `String` must
    /// be capped at `MAX_CLIENT_MESSAGE_LEN` (200) bytes via
    /// `sanitize_for_production` to prevent log-style overflow / DoS.
    #[test]
    fn test_core_error_to_response_4xx_sanitizes_long_message() {
        let _g = LocaleGuard::new();
        rust_i18n::set_locale("en");

        // 301-byte payload — exceeds the 200-byte cap.
        let big = "x".repeat(300);
        let (status, json) =
            core_error_to_response(&CoreError::InvalidInput(big.clone()), Locale::En);
        assert_eq!(status, StatusCode::BAD_REQUEST);
        // Prefix from the i18n template "Invalid input: " (14 bytes)
        // plus 200 bytes of 'x' plus "... (truncated)" (15 bytes) =
        // 229 bytes — well under hyper's default header limit.
        assert!(
            json.message.ends_with("... (truncated)"),
            "truncated message must end with sentinel, got: {}",
            json.message
        );
        assert!(
            json.message.len() < 301,
            "truncated message must be shorter than the original 300-byte payload, got len={}",
            json.message.len()
        );
        assert!(
            json.message.len() <= 200 + "... (truncated)".len(),
            "truncated message must respect the 200-byte cap plus sentinel, got len={}",
            json.message.len()
        );
        // The message still starts with the localized prefix.
        assert!(
            json.message.starts_with("Invalid input: x"),
            "truncated message must preserve the localized prefix"
        );

        // ParseError — also 4xx-class, sanitize path.
        let big_parse = "y".repeat(500);
        let (status, json) =
            core_error_to_response(&CoreError::ParseError(big_parse.clone()), Locale::En);
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(json.message.ends_with("... (truncated)"));
        assert!(json.message.len() < 500);

        // Sanity: short 4xx messages are returned verbatim (no truncation).
        let (status, json) =
            core_error_to_response(&CoreError::InvalidInput("short".to_string()), Locale::En);
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json.message, "Invalid input: short");
        assert!(!json.message.contains("truncated"));

        // zh-CN — same truncation behaviour with multi-byte chars.
        // The localized prefix is "无效输入：" (4 chars * 3 bytes = 12 bytes);
        // we craft a payload that crosses 200 bytes mid-char-boundary.
        let big_zh = "长".repeat(100); // 100 * 3 bytes = 300 bytes
        let (_, json_zh) = core_error_to_response(&CoreError::InvalidInput(big_zh), Locale::ZhCn);
        assert!(json_zh.message.ends_with("... (truncated)"));
        // Verify char-boundary safety: the message must be valid UTF-8
        // (it's a `String`, but assert to be explicit).
        assert!(std::str::from_utf8(json_zh.message.as_bytes()).is_ok());
    }

    /// 4xx-class errors WITHOUT an inner `String` (RateLimitExceeded,
    /// ApiKeyDisabled, ApiKeyExpired, InvalidApiKeySignature,
    /// TimeoutError) must return their localized message verbatim —
    /// no caller-controlled content, no truncation.
    #[test]
    fn test_core_error_to_response_4xx_no_inner_string_verbatim() {
        let _g = LocaleGuard::new();
        rust_i18n::set_locale("en");

        let (_, json) = core_error_to_response(&CoreError::RateLimitExceeded, Locale::En);
        assert_eq!(json.message, "Rate limit exceeded");

        let (_, json) = core_error_to_response(&CoreError::ApiKeyDisabled, Locale::En);
        assert_eq!(json.message, "API key disabled");

        let (_, json) = core_error_to_response(&CoreError::ApiKeyExpired, Locale::En);
        assert_eq!(json.message, "API key expired");

        let (_, json) = core_error_to_response(&CoreError::InvalidApiKeySignature, Locale::En);
        assert_eq!(json.message, "Invalid API key signature");

        // TimeoutError maps to 503 per `core_error_status_code`, but its
        // Display carries no inner String — verify verbatim message.
        let (status, json) = core_error_to_response(&CoreError::TimeoutError, Locale::En);
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(json.message, "Timeout error");
        assert!(!json.message.contains("truncated"));
    }

    /// `sanitize_for_production` directly — char-boundary safety and
    /// short-message passthrough.
    #[test]
    fn test_sanitize_for_production_direct() {
        // Short — verbatim
        assert_eq!(sanitize_for_production("hello"), "hello");
        assert_eq!(sanitize_for_production(""), "");

        // Exactly at the cap — verbatim (boundary is `>`, not `>=`)
        let exact = "a".repeat(MAX_CLIENT_MESSAGE_LEN);
        assert_eq!(
            sanitize_for_production(&exact).len(),
            MAX_CLIENT_MESSAGE_LEN
        );
        assert!(!sanitize_for_production(&exact).contains("truncated"));

        // Over the cap by 1 byte — truncated. Note: the sanitized form
        // is `prefix + "... (truncated)"`; for ASCII input `prefix` is
        // exactly `MAX_CLIENT_MESSAGE_LEN` bytes, so the sanitized total
        // is `MAX_CLIENT_MESSAGE_LEN + 15` bytes. When the input is
        // only `MAX_CLIENT_MESSAGE_LEN + 1` bytes long, the sanitized
        // form is actually LONGER than the input — that's expected
        // because the truncation sentinel itself carries information.
        // The invariant we enforce is that the *prefix* (content
        // portion) is at most `MAX_CLIENT_MESSAGE_LEN` bytes.
        let one_over = "a".repeat(MAX_CLIENT_MESSAGE_LEN + 1);
        let sanitized = sanitize_for_production(&one_over);
        assert!(sanitized.ends_with("... (truncated)"));
        let prefix_len = sanitized.len() - "... (truncated)".len();
        assert!(prefix_len <= MAX_CLIENT_MESSAGE_LEN);
        assert_eq!(prefix_len, MAX_CLIENT_MESSAGE_LEN); // ASCII round-down is exact

        // Substantially over the cap — sanitized must be shorter than
        // the original input.
        let way_over = "a".repeat(MAX_CLIENT_MESSAGE_LEN + 100); // 300 bytes
        let sanitized = sanitize_for_production(&way_over);
        assert!(sanitized.ends_with("... (truncated)"));
        assert!(
            sanitized.len() < way_over.len(),
            "sanitized={} way_over={}",
            sanitized.len(),
            way_over.len()
        );
        assert_eq!(
            sanitized.len(),
            MAX_CLIENT_MESSAGE_LEN + "... (truncated)".len()
        );

        // Multi-byte char boundary safety — slice at 200 must never
        // split a UTF-8 codepoint. Use a string of 4-byte chars (🤖)
        // so the function must round `end` down to a char boundary.
        // (200 % 4 == 0, so 200 is already a 🤖 boundary — the test
        // still validates UTF-8 validity and the multiple-of-4 rule.)
        let multi = "🤖".repeat(100); // 400 bytes total
        let sanitized = sanitize_for_production(&multi);
        assert!(sanitized.ends_with("... (truncated)"));
        // The truncated prefix must be valid UTF-8 (the function uses
        // `is_char_boundary` to round down).
        let prefix_end = sanitized.len() - "... (truncated)".len();
        assert!(std::str::from_utf8(&sanitized.as_bytes()[..prefix_end]).is_ok());
        // Prefix length must be a multiple of 4 (each 🤖 is 4 bytes)
        // and at most MAX_CLIENT_MESSAGE_LEN.
        assert_eq!(prefix_end % 4, 0);
        assert!(prefix_end <= MAX_CLIENT_MESSAGE_LEN);
    }

    #[test]
    fn test_invalid_uuid_response() {
        let _g = LocaleGuard::new();
        rust_i18n::set_locale("en");
        let (status, json) = invalid_uuid_response(Locale::En);
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json.message, "Invalid UUID format");

        let (status, json) = invalid_uuid_response(Locale::ZhCn);
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json.message, "无效的 UUID 格式");
    }

    #[test]
    fn test_admin_cannot_perform_response() {
        let _g = LocaleGuard::new();
        rust_i18n::set_locale("en");
        let (status, json) = admin_cannot_perform_response(Locale::En);
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(json.message, "Admin API key cannot perform this operation");

        let (status, json) = admin_cannot_perform_response(Locale::ZhCn);
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(json.message, "Admin API key 无法执行此操作");
    }

    #[test]
    fn test_workspace_name_not_found_response() {
        let _g = LocaleGuard::new();
        rust_i18n::set_locale("en");
        let (status, json) = workspace_name_not_found_response("my-ws", Locale::En);
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(json.message, "Workspace 'my-ws' not found");

        let (status, json) = workspace_name_not_found_response("my-ws", Locale::ZhCn);
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(json.message, "工作空间 'my-ws' 未找到");
    }

    /// LOW L-4 — long workspace `name` must be truncated before
    /// interpolation into the response message. Verifies char-boundary
    /// safety (multi-byte UTF-8) and that the response stays bounded.
    #[test]
    fn test_workspace_name_not_found_response_truncates_long_name() {
        let _g = LocaleGuard::new();
        rust_i18n::set_locale("en");

        // 200-byte ASCII name — well over the 64-byte cap.
        let long_name = "a".repeat(200);
        let (status, json) = workspace_name_not_found_response(&long_name, Locale::En);
        assert_eq!(status, StatusCode::NOT_FOUND);
        // Truncation marker must appear.
        assert!(
            json.message.contains("..."),
            "expected truncation marker '...' in message, got: {}",
            json.message
        );
        // Total message length must be bounded — prefix
        // "Workspace '" + 64 bytes + "..." + "' not found" = ~84 bytes.
        // We assert < 100 to leave room for locale-specific prefix/suffix.
        assert!(
            json.message.len() < 100,
            "truncated response message must be < 100 bytes, got len={} msg={:?}",
            json.message.len(),
            json.message
        );
        // The full 200-byte name must NOT appear verbatim.
        assert!(
            !json.message.contains(&long_name),
            "full long name must not appear in message"
        );

        // Multi-byte UTF-8 char-boundary safety: 100 x '长' (3 bytes
        // each = 300 bytes). The 64-byte cap is not a char boundary
        // (64 % 3 != 0), so the function must round down to 63 bytes
        // (21 chars). The message must be valid UTF-8 by construction
        // (it's a `String`), but assert to be explicit.
        let long_zh_name = "长".repeat(100);
        let (_, json_zh) = workspace_name_not_found_response(&long_zh_name, Locale::ZhCn);
        assert!(json_zh.message.contains("..."));
        assert!(std::str::from_utf8(json_zh.message.as_bytes()).is_ok());
        assert!(
            json_zh.message.len() < 100,
            "truncated zh message must be < 100 bytes, got len={}",
            json_zh.message.len()
        );

        // Boundary: exactly 64 bytes — no truncation.
        let exact = "b".repeat(64);
        let (status, json) = workspace_name_not_found_response(&exact, Locale::En);
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(
            !json.message.contains("..."),
            "exact-cap name must not be truncated, got: {}",
            json.message
        );
        assert!(json.message.contains(&exact));

        // Boundary: 65 bytes — truncated.
        let over_by_one = "c".repeat(65);
        let (_, json) = workspace_name_not_found_response(&over_by_one, Locale::En);
        assert!(
            json.message.contains("..."),
            "over-cap name must be truncated, got: {}",
            json.message
        );
        // Truncated prefix is 64 bytes of 'c' + "...".
        assert!(json.message.contains(&"c".repeat(64)));
        assert!(!json.message.contains(&"c".repeat(65)));
    }

    #[test]
    fn test_workspace_id_required_response() {
        let _g = LocaleGuard::new();
        rust_i18n::set_locale("en");
        let (status, json) = workspace_id_required_response(Locale::En);
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(json.message.contains("workspace_id"));

        let (status, json) = workspace_id_required_response(Locale::ZhCn);
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(json.message.contains("workspace_id"));
    }

    /// MEDIUM M-1 — `validation_error_response` must surface the field
    /// name and rule code (e.g. "length", "range") without leaking the
    /// constraint values (min/max) that `ValidationErrors::to_string()`
    /// would otherwise expose via `ValidationError::params`.
    #[test]
    fn test_validation_error_response_en() {
        let _g = LocaleGuard::new();
        rust_i18n::set_locale("en");

        // Construct a struct with `#[validate(length(min = 1, max = 64))]`
        // and trigger a validation failure by setting the field to "".
        #[derive(validator::Validate)]
        struct SampleReq {
            #[validate(length(min = 1, max = 64))]
            workspace_id: String,
        }

        let req = SampleReq {
            workspace_id: String::new(),
        };
        let errs = req.validate().unwrap_err();
        let (status, json) = validation_error_response(&errs, Locale::En);
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json.code, 400);
        // The field name and rule code must be present.
        assert!(
            json.message.contains("workspace_id"),
            "expected field name in message, got: {}",
            json.message
        );
        assert!(
            json.message.contains("length"),
            "expected rule code 'length' in message, got: {}",
            json.message
        );
        // Constraint values must NOT be leaked.
        assert!(
            !json.message.contains("min ="),
            "message must not leak min constraint, got: {}",
            json.message
        );
        assert!(
            !json.message.contains("max ="),
            "message must not leak max constraint, got: {}",
            json.message
        );
        assert!(
            !json.message.contains("64"),
            "message must not leak the numeric bound 64, got: {}",
            json.message
        );
    }

    /// zh-CN path — same assertions as the English variant.
    #[test]
    fn test_validation_error_response_zh_cn() {
        let _g = LocaleGuard::new();
        rust_i18n::set_locale("en");

        #[derive(validator::Validate)]
        struct SampleReq {
            #[validate(length(min = 1, max = 64))]
            workspace_id: String,
        }

        let req = SampleReq {
            workspace_id: String::new(),
        };
        let errs = req.validate().unwrap_err();
        let (status, json) = validation_error_response(&errs, Locale::ZhCn);
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json.code, 400);
        assert!(json.message.contains("workspace_id"));
        assert!(json.message.contains("length"));
        assert!(!json.message.contains("min ="));
        assert!(!json.message.contains("max ="));
        assert!(!json.message.contains("64"));
    }

    /// MEDIUM M-1 — explicit regression test: construct a struct with
    /// a `range(min = 100, max = 1000000)` constraint and trigger a
    /// failure by setting the field below the minimum. The response
    /// must NOT contain "100", "1000000", "min = 100", "max = 1000000".
    #[test]
    fn test_validation_error_response_does_not_leak_constraints() {
        let _g = LocaleGuard::new();
        rust_i18n::set_locale("en");

        #[derive(validator::Validate)]
        struct RangeReq {
            #[validate(range(min = 100, max = 1_000_000))]
            count: i64,
        }

        let req = RangeReq { count: 1 };
        let errs = req.validate().unwrap_err();
        let (status, json) = validation_error_response(&errs, Locale::En);
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json.code, 400);
        assert!(
            json.message.contains("count"),
            "expected field name 'count' in message, got: {}",
            json.message
        );
        assert!(
            json.message.contains("range"),
            "expected rule code 'range' in message, got: {}",
            json.message
        );
        // The numeric constraints must NOT be leaked.
        assert!(
            !json.message.contains("min ="),
            "message must not leak min constraint, got: {}",
            json.message
        );
        assert!(
            !json.message.contains("max ="),
            "message must not leak max constraint, got: {}",
            json.message
        );
        assert!(
            !json.message.contains("1000000"),
            "message must not leak upper bound, got: {}",
            json.message
        );
        assert!(
            !json.message.contains("100"),
            "message must not leak lower bound, got: {}",
            json.message
        );
    }

    /// MEDIUM M-1 — multiple field errors are joined by "; " and
    /// capped at `MAX_CLIENT_MESSAGE_LEN` bytes via
    /// `sanitize_for_production`.
    #[test]
    fn test_validation_error_response_multiple_fields_joined() {
        let _g = LocaleGuard::new();
        rust_i18n::set_locale("en");

        #[derive(validator::Validate)]
        struct MultiReq {
            #[validate(length(min = 1, max = 64))]
            workspace_id: String,
            #[validate(length(min = 1, max = 32))]
            name: String,
        }

        let req = MultiReq {
            workspace_id: String::new(),
            name: String::new(),
        };
        let errs = req.validate().unwrap_err();
        let (status, json) = validation_error_response(&errs, Locale::En);
        assert_eq!(status, StatusCode::BAD_REQUEST);
        // Both field names must appear, separated by "; ".
        assert!(json.message.contains("workspace_id"));
        assert!(json.message.contains("name"));
        assert!(
            json.message.contains("; "),
            "expected '; ' separator between field errors, got: {}",
            json.message
        );
    }
}
