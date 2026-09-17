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
//! Phase 8 — the helpers in this file produce locale-translated
//! `ErrorResponse` payloads by reading the `Locale` negotiated by
//! `locale_middleware` (from the `Accept-Language` header). They are the
//! single entry point for `CoreError → HTTP response` conversion in
//! `router.rs`, ensuring consistent status codes and i18n coverage.
//!
//! # Style guide
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

use crate::core::database::ApiKeyRole;
use crate::core::i18n::{translate_with_locale, translate_with_locale_args};
use crate::core::CoreError;
use crate::server::middleware::locale::Locale;
use crate::server::models::{ApiErrorCode, ErrorResponse};
use axum::http::StatusCode;
use axum::Json;

/// Convert database errors to `CoreError::DatabaseError`.
pub(super) fn map_db_error<E: std::fmt::Display>(error: E) -> CoreError {
    CoreError::DatabaseError(error.to_string())
}

/// Convert UUID parse errors to `CoreError::InvalidInput`.
pub(super) fn map_uuid_error<E: std::fmt::Display>(error: E) -> CoreError {
    CoreError::InvalidInput(
        t!("api.error.handlers.helpers.invalid_uuid", error = error).to_string(),
    )
}

/// `CoreError` → `(HTTP 状态码, 业务错误码)` 单表分类（T032）。
///
/// 这是唯一的事实来源：HTTP 错误响应的状态码与 `business_code`
/// （[`ErrorResponse`]）都从本表取值，保证两者不会各自漂移；
/// gRPC 侧的 [`core_error_grpc_code`] 与本表逐行同语义。
///
/// Phase 8 (LOW fix) 历史 — 本表承接自原 `core_error_status_code`
/// （旧 `CoreError::to_http_response` / `http_status_code` /
/// `error_code` 依赖进程级全局 locale，已作为死代码移除）；locale
/// 化文案统一经 `to_localized_string` + 下方 helpers 生成。
/// T032 起原函数并入本表（一张映射表两用），不再单设状态码查询口。
///
/// 业务码沿用 `ApiErrorCode` 注册表：
/// - 404 泛型 `NotFound` 默认映射 `WorkspaceNotFound`（2001，沿用被移除的
///   `From<ErrorResponse>` 的「默认资源错误」惯例）；特定资源由
///   `BizTagNotFound`（2003）区分。
/// - `GroupNotFound`（2002）/`ResourceAlreadyExists`（2004）当前无对应
///   `CoreError` 变体、不会出现（预留码，见 openapi 错误码表标注）。
fn core_error_classification(e: &CoreError) -> (StatusCode, ApiErrorCode) {
    match e {
        CoreError::InvalidIdFormat(_)
        | CoreError::InvalidIdString(_)
        | CoreError::InvalidAlgorithmType(_)
        | CoreError::InvalidInput(_)
        | CoreError::ParseError(_) => (StatusCode::BAD_REQUEST, ApiErrorCode::InvalidInput),
        CoreError::AuthenticationError(_) => (StatusCode::UNAUTHORIZED, ApiErrorCode::Unauthorized),
        CoreError::InvalidApiKeySignature => {
            (StatusCode::UNAUTHORIZED, ApiErrorCode::InvalidApiKey)
        }
        CoreError::ApiKeyDisabled => (StatusCode::UNAUTHORIZED, ApiErrorCode::ApiKeyDisabled),
        CoreError::ApiKeyExpired => (StatusCode::UNAUTHORIZED, ApiErrorCode::ApiKeyExpired),
        CoreError::WorkspaceDisabled(_) => (StatusCode::FORBIDDEN, ApiErrorCode::Forbidden),
        CoreError::NotFound(_) => (StatusCode::NOT_FOUND, ApiErrorCode::WorkspaceNotFound),
        CoreError::BizTagNotFound(_) => (StatusCode::NOT_FOUND, ApiErrorCode::BizTagNotFound),
        CoreError::RateLimitExceeded => (
            StatusCode::TOO_MANY_REQUESTS,
            ApiErrorCode::RateLimitExceeded,
        ),
        CoreError::TimeoutError => (
            StatusCode::SERVICE_UNAVAILABLE,
            ApiErrorCode::ServiceUnavailable,
        ),
        CoreError::DatabaseError(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            ApiErrorCode::DatabaseError,
        ),
        CoreError::CacheError(_) => (StatusCode::INTERNAL_SERVER_ERROR, ApiErrorCode::CacheError),
        // 其余 5xx 类（InternalError/ConfigurationError/EtcdError/IoError/
        // 算法类/Unknown）统一收敛 InternalError（5001）。
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            ApiErrorCode::InternalError,
        ),
    }
}

/// HTTP status code for a `CoreError` variant.
///
/// Phase 8 (LOW fix) — this is the single source of
/// truth for the `CoreError → axum::http::StatusCode` mapping. The
/// old `CoreError::to_http_response` / `http_status_code` /
/// `error_code` methods (which consulted the process-wide global
/// locale via `to_string()`) were removed as dead code; all
/// locale-aware translation now goes through `to_localized_string`
/// + the helpers below.
///
/// T032 — 状态码判定委托给 [`core_error_classification`] 单表
/// （同一张表同时给出 `business_code`），本函数只保留旧行接口。
#[cfg(test)]
fn core_error_status_code(e: &CoreError) -> StatusCode {
    core_error_classification(e).0
}

/// Maximum message length returned to clients for 4xx-class errors.
///
/// Phase 8 (CRITICAL C-1 / HIGH fix) — guards against
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
/// Phase 8 (CRITICAL C-1 / HIGH fix) — the response message
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
///   `sanitize_for_production` to cap length at
///   `MAX_CLIENT_MESSAGE_LEN` bytes.
/// - **4xx-class errors without inner `String`**
///   (`RateLimitExceeded`, `ApiKeyDisabled`, `ApiKeyExpired`,
///   `InvalidApiKeySignature`, `TimeoutError`): the localized message is
///   returned verbatim — there is no caller-controlled content to filter.
pub fn core_error_to_response(e: &CoreError, locale: Locale) -> (StatusCode, Json<ErrorResponse>) {
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

    // T032 — 状态码与业务码同表判定，request_id/timestamp 在装配处生成。
    // T022 — request_id 优先取请求上下文（request_id 中间件安装的任务局部
    // 上下文），保证错误响应体与 x-request-id 响应头一致；无中间件上下文
    // （单元测试 / 独立调用）时保留装配处新生成的 UUID v4。
    let (status, business_code) = core_error_classification(e);
    let code = status.as_u16() as i32;
    let mut error = ErrorResponse::new(code, business_code, message);
    if let Some(request_id) = crate::server::middleware::request_id::current_request_id() {
        error.request_id = request_id;
    }
    (status, Json(error))
}

// ========== 共享授权（T010）==========

/// 共享的 workspace 资源级授权决策（HTTP 与 gRPC 同源）。
///
/// 语义 = `router.rs` 既有 `verify_user_role` / `verify_user_workspace`
/// 泛化：User 仅可访问自身 workspace，Admin 跨租户放行，Anonymous 一律拒绝。
/// 「按 workspace 名反查 UUID」「NotFound 处理」等传输相关步骤由调用方完成，
/// 本函数只做**确定性角色-租户判定**，保证两条传输线的判定不会漂移。
///
/// # 错误变体契约（与现实现映射表一致）
///
/// 返回的 `CoreError` 是「拒绝类别」的载体，其选择与
/// [`core_error_status_code`] 既有 variant→状态码表保持一致：
///
/// - User 跨 workspace → `CoreError::WorkspaceDisabled`：映射表中唯一落到
///   `FORBIDDEN`（403）的变体，即 HTTP 侧 `workspace_mismatch_response` 的
///   同类。HTTP 调用点将其重新映射回 locale 化的 workspace mismatch 响应，
///   gRPC 调用点映射为 `Status::permission_denied`；内层 String 不面向客户端
///   透出，仅作服务端日志/判因用途。
/// - Anonymous → `CoreError::AuthenticationError`：映射表中 401 载体，与
///   HTTP 侧 `auth_required_response` 同状态码语义。
///
/// # Errors
///
/// 见上方错误变体契约；放行时返回 `Ok(())`。
pub(crate) async fn authorize_workspace_access(
    role: &ApiKeyRole,
    key_workspace_id: uuid::Uuid,
    target_workspace_id: uuid::Uuid,
) -> Result<(), CoreError> {
    match role {
        // Admin：跨租户放行（管理面语义）。
        ApiKeyRole::Admin => Ok(()),
        // User：仅自身 workspace。
        ApiKeyRole::User if key_workspace_id == target_workspace_id => Ok(()),
        ApiKeyRole::User => Err(CoreError::WorkspaceDisabled(format!(
            "workspace {} is not owned by the caller",
            target_workspace_id
        ))),
        // Anonymous（认证禁用时注入）：无业务权限，fail-closed。
        ApiKeyRole::Anonymous => Err(CoreError::AuthenticationError(
            "authentication required".to_string(),
        )),
    }
}

// ========== gRPC 错误消毒（T013）==========

/// gRPC 侧「变体 → Code」映射，与 [`core_error_status_code`] 同源：
/// 4xx 变体逐一对应同语义的 gRPC Code（400→InvalidArgument、401→
/// Unauthenticated、403→PermissionDenied、404→NotFound、429→
/// ResourceExhausted、503→Unavailable），5xx 一律收敛到 `Code::Internal`。
///
/// 映射表不含 `Code::AlreadyExists`：`CoreError` 当前没有冲突类变体，
/// 不预造空映射。
fn core_error_grpc_code(e: &CoreError) -> sdforge::tonic::Code {
    match e {
        CoreError::InvalidIdFormat(_)
        | CoreError::InvalidIdString(_)
        | CoreError::InvalidAlgorithmType(_)
        | CoreError::InvalidInput(_)
        | CoreError::ParseError(_) => sdforge::tonic::Code::InvalidArgument,
        CoreError::AuthenticationError(_)
        | CoreError::InvalidApiKeySignature
        | CoreError::ApiKeyDisabled
        | CoreError::ApiKeyExpired => sdforge::tonic::Code::Unauthenticated,
        CoreError::WorkspaceDisabled(_) => sdforge::tonic::Code::PermissionDenied,
        CoreError::NotFound(_) | CoreError::BizTagNotFound(_) => sdforge::tonic::Code::NotFound,
        CoreError::RateLimitExceeded => sdforge::tonic::Code::ResourceExhausted,
        CoreError::TimeoutError => sdforge::tonic::Code::Unavailable,
        _ => sdforge::tonic::Code::Internal,
    }
}

/// Convert `CoreError` to a sanitized gRPC `Status`（与 HTTP 侧
/// [`core_error_status_code`] / [`core_error_to_response`] 同源）。
///
/// 消毒策略与 HTTP 侧逐条对应：
///
/// - **5xx 类**（`DatabaseError`、`CacheError`、`InternalError` 等）：全量
///   错误细节（内层 `String` 可能携带 DB URL、文件路径）只进服务端
///   `tracing::error!` 日志；客户端只拿固定文案 `"internal error"` 的
///   `Code::Internal` —— 此前 gRPC 的 `Status::internal(format!("{}", e))`
///   会把内部细节明文回传，与 HTTP 侧的消毒承诺不一致。
/// - **4xx 类**：本地化 Display 消息，经 [`sanitize_for_production`] 截断
///   （与 HTTP 4xx 同一上限），消息本身面向调用方（与 HTTP 同策略）。
pub(crate) fn core_error_to_grpc_status(e: &CoreError) -> sdforge::tonic::Status {
    let code = core_error_grpc_code(e);
    if code == sdforge::tonic::Code::Internal {
        tracing::error!(
            event = "core_error",
            error = ?e,
            "internal error returned to grpc client as generic message"
        );
        return sdforge::tonic::Status::internal("internal error");
    }
    sdforge::tonic::Status::new(code, sanitize_for_production(&e.to_string()))
}

/// Build a 400 response for an invalid UUID path parameter, with the
/// locale-translated message.
pub fn invalid_uuid_response(locale: Locale) -> (StatusCode, Json<ErrorResponse>) {
    let message = translate_with_locale(locale.as_str(), "api.error.invalid_uuid_format");
    (
        StatusCode::BAD_REQUEST,
        Json(ErrorResponse::new(400, ApiErrorCode::InvalidUuid, message)),
    )
}

/// Build a 400 response for `validator::ValidationErrors`, with the
/// locale-translated message.
///
/// Phase 8 (MEDIUM fix) — uses `errors.field_errors()` to
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
        Json(ErrorResponse::new(
            400,
            ApiErrorCode::ValidationError,
            message,
        )),
    )
}

/// Build a 403 response for "Admin API key cannot perform this operation".
pub(crate) fn admin_cannot_perform_response(locale: Locale) -> (StatusCode, Json<ErrorResponse>) {
    let message = translate_with_locale(locale.as_str(), "api.error.admin_cannot_perform");
    (
        StatusCode::FORBIDDEN,
        Json(ErrorResponse::new(403, ApiErrorCode::Forbidden, message)),
    )
}

/// （CWE-1188）：Anonymous 角色（认证禁用时）访问受保护端点的响应。
/// 返回 401 Unauthorized，明确要求启用认证并提供有效 API key。
pub(crate) fn auth_required_response(locale: Locale) -> (StatusCode, Json<ErrorResponse>) {
    let message = translate_with_locale(locale.as_str(), "api.error.auth_required");
    (
        StatusCode::UNAUTHORIZED,
        Json(ErrorResponse::new(401, ApiErrorCode::Unauthorized, message)),
    )
}

/// Build a 403 response for "Access denied: workspace mismatch".
pub(crate) fn workspace_mismatch_response(locale: Locale) -> (StatusCode, Json<ErrorResponse>) {
    let message = translate_with_locale(locale.as_str(), "api.error.workspace_mismatch");
    (
        StatusCode::FORBIDDEN,
        Json(ErrorResponse::new(403, ApiErrorCode::Forbidden, message)),
    )
}

/// Build a 404 response for "Workspace '<name>' not found".
///
/// Phase 8 (LOW fix) — `name` originates from a URL path
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
        Json(ErrorResponse::new(
            404,
            ApiErrorCode::WorkspaceNotFound,
            message,
        )),
    )
}

/// Build a 404 response for "Workspace not found" (no name).
pub(crate) fn workspace_not_found_response(locale: Locale) -> (StatusCode, Json<ErrorResponse>) {
    let message = translate_with_locale(locale.as_str(), "api.error.workspace_not_found");
    (
        StatusCode::NOT_FOUND,
        Json(ErrorResponse::new(
            404,
            ApiErrorCode::WorkspaceNotFound,
            message,
        )),
    )
}

/// Build a 500 response for "Invalid workspace ID".
pub(crate) fn invalid_workspace_id_response(locale: Locale) -> (StatusCode, Json<ErrorResponse>) {
    let message = translate_with_locale(locale.as_str(), "api.error.invalid_workspace_id");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse::new(
            500,
            ApiErrorCode::InternalError,
            message,
        )),
    )
}

/// Build a 400 response for "workspace_id is required for user keys".
pub(crate) fn workspace_id_required_response(locale: Locale) -> (StatusCode, Json<ErrorResponse>) {
    let message = translate_with_locale(locale.as_str(), "api.error.workspace_id_required");
    (
        StatusCode::BAD_REQUEST,
        Json(ErrorResponse::new(
            400,
            ApiErrorCode::MissingRequiredField,
            message,
        )),
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

    // ========== T032 —— 统一错误信封 business_code ==========

    /// 单表分类全变体矩阵：`core_error_classification` 给出的
    /// (状态码, business_code) 必须与 `core_error_status_code` 既有
    /// 映射逐行一致（一张映射表两用的钉桩）。
    #[test]
    fn test_core_error_classification_full_matrix() {
        use crate::server::models::ApiErrorCode;

        let cases: &[(CoreError, StatusCode, ApiErrorCode)] = &[
            (
                CoreError::InvalidInput("x".into()),
                StatusCode::BAD_REQUEST,
                ApiErrorCode::InvalidInput,
            ),
            (
                CoreError::InvalidIdFormat("x".into()),
                StatusCode::BAD_REQUEST,
                ApiErrorCode::InvalidInput,
            ),
            (
                CoreError::InvalidIdString("x".into()),
                StatusCode::BAD_REQUEST,
                ApiErrorCode::InvalidInput,
            ),
            (
                CoreError::InvalidAlgorithmType("x".into()),
                StatusCode::BAD_REQUEST,
                ApiErrorCode::InvalidInput,
            ),
            (
                CoreError::ParseError("x".into()),
                StatusCode::BAD_REQUEST,
                ApiErrorCode::InvalidInput,
            ),
            (
                CoreError::AuthenticationError("x".into()),
                StatusCode::UNAUTHORIZED,
                ApiErrorCode::Unauthorized,
            ),
            (
                CoreError::InvalidApiKeySignature,
                StatusCode::UNAUTHORIZED,
                ApiErrorCode::InvalidApiKey,
            ),
            (
                CoreError::ApiKeyDisabled,
                StatusCode::UNAUTHORIZED,
                ApiErrorCode::ApiKeyDisabled,
            ),
            (
                CoreError::ApiKeyExpired,
                StatusCode::UNAUTHORIZED,
                ApiErrorCode::ApiKeyExpired,
            ),
            (
                CoreError::WorkspaceDisabled("x".into()),
                StatusCode::FORBIDDEN,
                ApiErrorCode::Forbidden,
            ),
            (
                CoreError::NotFound("x".into()),
                StatusCode::NOT_FOUND,
                ApiErrorCode::WorkspaceNotFound,
            ),
            (
                CoreError::BizTagNotFound("x".into()),
                StatusCode::NOT_FOUND,
                ApiErrorCode::BizTagNotFound,
            ),
            (
                CoreError::RateLimitExceeded,
                StatusCode::TOO_MANY_REQUESTS,
                ApiErrorCode::RateLimitExceeded,
            ),
            (
                CoreError::TimeoutError,
                StatusCode::SERVICE_UNAVAILABLE,
                ApiErrorCode::ServiceUnavailable,
            ),
            (
                CoreError::DatabaseError("x".into()),
                StatusCode::INTERNAL_SERVER_ERROR,
                ApiErrorCode::DatabaseError,
            ),
            (
                CoreError::CacheError("x".into()),
                StatusCode::INTERNAL_SERVER_ERROR,
                ApiErrorCode::CacheError,
            ),
            (
                CoreError::InternalError("x".into()),
                StatusCode::INTERNAL_SERVER_ERROR,
                ApiErrorCode::InternalError,
            ),
            (
                CoreError::ConfigurationError("x".into()),
                StatusCode::INTERNAL_SERVER_ERROR,
                ApiErrorCode::InternalError,
            ),
            (
                CoreError::EtcdError("x".into()),
                StatusCode::INTERNAL_SERVER_ERROR,
                ApiErrorCode::InternalError,
            ),
            (
                CoreError::IoError("x".into()),
                StatusCode::INTERNAL_SERVER_ERROR,
                ApiErrorCode::InternalError,
            ),
            (
                CoreError::ClockMovedBackward { last_timestamp: 1 },
                StatusCode::INTERNAL_SERVER_ERROR,
                ApiErrorCode::InternalError,
            ),
            (
                CoreError::SequenceOverflow { timestamp: 1 },
                StatusCode::INTERNAL_SERVER_ERROR,
                ApiErrorCode::InternalError,
            ),
            (
                CoreError::SegmentExhausted { max_id: 1 },
                StatusCode::INTERNAL_SERVER_ERROR,
                ApiErrorCode::InternalError,
            ),
            (
                CoreError::Unknown,
                StatusCode::INTERNAL_SERVER_ERROR,
                ApiErrorCode::InternalError,
            ),
        ];

        for (e, expected_status, expected_business) in cases {
            let (status, business) = core_error_classification(e);
            assert_eq!(status, *expected_status, "variant: {e:?}");
            assert_eq!(business, *expected_business, "variant: {e:?}");
            // 同源性：core_error_status_code 与单表 .0 一致。
            assert_eq!(core_error_status_code(e), *expected_status);
        }
    }

    /// 任务规定的三类代表性错误信封断言：
    /// 限流 → 4001；校验失败 → 3002；资源不存在 → 2001/2003。
    #[test]
    fn test_error_envelope_business_codes_for_required_classes() {
        let _g = LocaleGuard::new();
        rust_i18n::set_locale("en");

        // 限流：RateLimitExceeded → "4001"。
        let (_, json) = core_error_to_response(&CoreError::RateLimitExceeded, Locale::En);
        assert_eq!(json.business_code, "4001");
        assert_eq!(json.code, 429);

        // 校验失败：validation_error_response → "3002"。
        #[derive(validator::Validate)]
        struct SampleReq {
            #[validate(length(min = 1, max = 64))]
            name: String,
        }
        let errs = SampleReq {
            name: String::new(),
        }
        .validate()
        .unwrap_err();
        let (_, json) = validation_error_response(&errs, Locale::En);
        assert_eq!(json.business_code, "3002");
        assert_eq!(json.code, 400);

        // 资源不存在：泛型 NotFound → "2001"；BizTagNotFound → "2003"；
        // workspace 名未找到（handler 构造）→ "2001"。
        let (_, json) = core_error_to_response(&CoreError::NotFound("ghost".into()), Locale::En);
        assert_eq!(json.business_code, "2001");
        assert_eq!(json.code, 404);
        let (_, json) =
            core_error_to_response(&CoreError::BizTagNotFound("tag".into()), Locale::En);
        assert_eq!(json.business_code, "2003");
        let (_, json) = workspace_name_not_found_response("ghost-ws", Locale::En);
        assert_eq!(json.business_code, "2001");
        assert_eq!(json.code, 404);
    }

    /// handler 构造类错误的业务码：UUID 非法 → 3004、缺必填 → 3003、
    /// 跨 workspace/Admin 拒绝 → 1002、未认证 → 1001。
    #[test]
    fn test_error_envelope_business_codes_for_handler_constructed() {
        let _g = LocaleGuard::new();
        rust_i18n::set_locale("en");

        let (_, json) = invalid_uuid_response(Locale::En);
        assert_eq!(json.business_code, "3004");

        let (_, json) = workspace_id_required_response(Locale::En);
        assert_eq!(json.business_code, "3003");

        let (_, json) = admin_cannot_perform_response(Locale::En);
        assert_eq!(json.business_code, "1002");

        let (_, json) = workspace_mismatch_response(Locale::En);
        assert_eq!(json.business_code, "1002");

        let (_, json) = auth_required_response(Locale::En);
        assert_eq!(json.business_code, "1001");

        let (_, json) = invalid_workspace_id_response(Locale::En);
        assert_eq!(json.business_code, "5001");
    }

    /// 信封装配：`core_error_to_response` 生成的响应必须携带
    /// request_id（UUID v4）与毫秒级 timestamp。
    #[test]
    fn test_core_error_to_response_envelope_tracks_request() {
        let _g = LocaleGuard::new();
        rust_i18n::set_locale("en");

        let (_, json) =
            core_error_to_response(&CoreError::InvalidInput("x".to_string()), Locale::En);
        let parsed =
            uuid::Uuid::parse_str(&json.request_id).expect("request_id must be a valid UUID");
        assert_eq!(parsed.get_version_num(), 4);
        assert!(json.timestamp > 0, "timestamp must be millis since epoch");
    }

    /// CRITICAL C-1 / HIGH — 5xx internal errors MUST NOT leak the
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

    /// HIGH — 4xx-class errors with caller-supplied `String` must
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

    // ========== authorize_workspace_access（T010 共享授权）==========

    /// User 访问自身 workspace → 放行。
    #[tokio::test]
    async fn test_authorize_workspace_access_user_own_workspace_ok() {
        let ws = uuid::Uuid::new_v4();
        let result = authorize_workspace_access(&ApiKeyRole::User, ws, ws).await;
        assert!(result.is_ok(), "User must access its own workspace");
    }

    /// User 跨 workspace → 拒绝，错误变体为 `WorkspaceDisabled`
    /// （core_error_status_code 映射表中唯一的 403/FORBIDDEN 载体，
    /// 与 HTTP 侧 workspace_mismatch 响应同类）。
    #[tokio::test]
    async fn test_authorize_workspace_access_user_cross_workspace_denied() {
        let key_ws = uuid::Uuid::new_v4();
        let other_ws = uuid::Uuid::new_v4();
        assert_ne!(key_ws, other_ws);
        let err = authorize_workspace_access(&ApiKeyRole::User, key_ws, other_ws)
            .await
            .expect_err("cross-workspace access must be denied");
        assert!(
            matches!(err, CoreError::WorkspaceDisabled(_)),
            "expected WorkspaceDisabled (403 carrier), got {:?}",
            err
        );
        // 同源映射：该变体必须落到 403。
        let (status, _) = core_error_to_response(&err, Locale::En);
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    /// Admin 跨租户 → 放行。
    #[tokio::test]
    async fn test_authorize_workspace_access_admin_cross_tenant_ok() {
        let result = authorize_workspace_access(
            &ApiKeyRole::Admin,
            uuid::Uuid::new_v4(),
            uuid::Uuid::new_v4(),
        )
        .await;
        assert!(result.is_ok(), "Admin must have cross-tenant access");
    }

    /// Anonymous（认证禁用时注入的角色）→ 一律拒绝（fail-closed），
    /// 变体为 AuthenticationError（映射表 401 载体）。
    #[tokio::test]
    async fn test_authorize_workspace_access_anonymous_denied() {
        let ws = uuid::Uuid::new_v4();
        let err = authorize_workspace_access(&ApiKeyRole::Anonymous, ws, ws)
            .await
            .expect_err("Anonymous must never pass resource authorization");
        assert!(matches!(err, CoreError::AuthenticationError(_)));
        let (status, _) = core_error_to_response(&err, Locale::En);
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    // ========== core_error_to_grpc_status（T013 gRPC 错误消毒）==========

    /// 5xx 类错误必须消毒：Code 固定 Internal，message 固定 "internal
    /// error"，不得携带 Display 明文（含变体前缀 "Database error"）或内层
    /// 敏感串 —— 与 HTTP 侧 5xx 泛化承诺对齐。
    #[test]
    fn test_core_error_to_grpc_status_sanitizes_5xx() {
        let sensitive = "postgres://idgen:pwd@internal-host:5432/nebulaid";
        let e = CoreError::DatabaseError(format!("Database error: {sensitive}"));
        let status = core_error_to_grpc_status(&e);
        assert_eq!(status.code(), sdforge::tonic::Code::Internal);
        assert_eq!(status.message(), "internal error");
        assert!(
            !status.message().contains("Database error"),
            "message must not carry the Display text, got: {}",
            status.message()
        );
        assert!(
            !status.message().contains(sensitive),
            "message must not leak the inner string"
        );
        assert!(!status.message().contains("pwd"));

        // 其余 5xx 变体同样收敛（抽验 CacheError / SegmentExhausted）。
        let status = core_error_to_grpc_status(&CoreError::CacheError("redis://:s@h:1".into()));
        assert_eq!(status.code(), sdforge::tonic::Code::Internal);
        assert_eq!(status.message(), "internal error");
        let status = core_error_to_grpc_status(&CoreError::SegmentExhausted { max_id: 42 });
        assert_eq!(status.code(), sdforge::tonic::Code::Internal);
        assert_eq!(status.message(), "internal error");
    }

    /// NotFound → Code::NotFound 且消息保留（4xx 面向调用方）；
    /// 4xx 变体逐一映射到同语义 Code（抽验 400/401/403/404/429/503）。
    #[test]
    fn test_core_error_to_grpc_status_maps_4xx_codes() {
        let status = core_error_to_grpc_status(&CoreError::NotFound("ghost-ws".to_string()));
        assert_eq!(status.code(), sdforge::tonic::Code::NotFound);
        assert!(status.message().contains("ghost-ws"));

        let status = core_error_to_grpc_status(&CoreError::BizTagNotFound("tag".into()));
        assert_eq!(status.code(), sdforge::tonic::Code::NotFound);

        let status = core_error_to_grpc_status(&CoreError::InvalidInput("bad".into()));
        assert_eq!(status.code(), sdforge::tonic::Code::InvalidArgument);

        let status = core_error_to_grpc_status(&CoreError::AuthenticationError("x".into()));
        assert_eq!(status.code(), sdforge::tonic::Code::Unauthenticated);

        let status = core_error_to_grpc_status(&CoreError::WorkspaceDisabled("x".into()));
        assert_eq!(status.code(), sdforge::tonic::Code::PermissionDenied);

        let status = core_error_to_grpc_status(&CoreError::RateLimitExceeded);
        assert_eq!(status.code(), sdforge::tonic::Code::ResourceExhausted);

        let status = core_error_to_grpc_status(&CoreError::TimeoutError);
        assert_eq!(status.code(), sdforge::tonic::Code::Unavailable);
    }

    /// 4xx 消息走与 HTTP 相同的 200 字节截断上限。
    #[test]
    fn test_core_error_to_grpc_status_4xx_truncates_long_message() {
        let big = "x".repeat(300);
        let status = core_error_to_grpc_status(&CoreError::InvalidInput(big));
        assert_eq!(status.code(), sdforge::tonic::Code::InvalidArgument);
        assert!(
            status.message().ends_with("... (truncated)"),
            "4xx message must respect the shared truncation cap, got: {}",
            status.message()
        );
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

    /// LOW — long workspace `name` must be truncated before
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

    /// MEDIUM — `validation_error_response` must surface the field
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

    /// MEDIUM — explicit regression test: construct a struct with
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

    /// MEDIUM — multiple field errors are joined by "; " and
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

    #[test]
    fn test_workspace_not_found_and_invalid_id_responses() {
        let _g = LocaleGuard::new();
        rust_i18n::set_locale("en");

        let (status, json) = workspace_not_found_response(Locale::En);
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(!json.message.is_empty());

        let (status, json) = invalid_workspace_id_response(Locale::En);
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(!json.message.is_empty());
    }
}
