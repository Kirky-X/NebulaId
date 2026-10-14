// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! CoreError → HTTP/gRPC 响应的错误映射助手（单一口径）。
//! 文案经 `locale_middleware` 协商的 `Locale` 翻译；5xx 只回固定文案，
//! 4xx 携带的调用方字符串经 `sanitize_for_production` 截断至 200 字节。

use crate::core::database::ApiKeyRole;
use crate::core::i18n::{translate_with_locale, translate_with_locale_args};
use crate::core::CoreError;
use crate::server::middleware::locale::Locale;
use crate::server::models::{ApiErrorCode, ErrorResponse};
use axum::http::StatusCode;
use axum::Json;

/// 数据库错误 → `CoreError::DatabaseError`，底层错误经 `ErrorSource` 保链
/// （错误链仅进服务端日志，见 `core_error_to_response`）。
pub(super) fn map_db_error<E>(error: E) -> CoreError
where
    E: std::error::Error + Send + Sync + 'static,
{
    CoreError::DatabaseError(
        error.to_string(),
        Some(crate::core::types::ErrorSource::new(error)),
    )
}

/// UUID 解析错误 → `CoreError::InvalidInput`。
pub(super) fn map_uuid_error<E: std::fmt::Display>(error: E) -> CoreError {
    CoreError::InvalidInput(
        t!("api.error.handlers.helpers.invalid_uuid", error = error).to_string(),
    )
}

/// `CoreError` → `(HTTP 状态码, 业务错误码)` 单表分类，唯一事实来源：
/// 状态码与 `business_code` 都取自此表；gRPC 侧 `core_error_grpc_code` 与之同语义。
///
/// 404 泛型 `NotFound` 默认映射 `WorkspaceNotFound`（2001），特定资源用 `BizTagNotFound`（2003）；
/// `GroupNotFound`/`ResourceAlreadyExists` 为预留码，当前无对应 CoreError 变体。
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
        CoreError::DatabaseError(_, _) => (
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

/// `CoreError` 的 HTTP 状态码；判定委托 `core_error_classification` 单表，
/// 本函数只保留旧行接口（测试用）。
#[cfg(test)]
fn core_error_status_code(e: &CoreError) -> StatusCode {
    core_error_classification(e).0
}

/// 4xx 错误返回客户端的消息长度上限（200 字节），防长调用方字符串撑大响应。
const MAX_CLIENT_MESSAGE_LEN: usize = 200;

/// 按 `MAX_CLIENT_MESSAGE_LEN` 字节截断，超长时追加显式标记（char 边界安全）。
/// 仅作用于携带调用方字符串的 4xx；5xx 走固定文案不经此处。
fn sanitize_for_production(msg: &str) -> String {
    if msg.len() > MAX_CLIENT_MESSAGE_LEN {
        // 按 char 边界向下取整，避免切断多字节 UTF-8。
        let mut end = MAX_CLIENT_MESSAGE_LEN;
        while end > 0 && !msg.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}... (truncated)", &msg[..end])
    } else {
        msg.to_string()
    }
}

/// `CoreError` → `(StatusCode, Json<ErrorResponse>)`，message 按 locale 翻译。
///
/// 5xx 类：完整错误只记 `tracing::error!`，客户端仅见 `api.error.<variant>` 固定文案。
/// 4xx 携带调用方字符串：`to_localized_string` 后经 `sanitize_for_production` 截断；
/// 4xx 无内层字符串（RateLimitExceeded/ApiKeyDisabled 等）原样返回本地化文案。
pub fn core_error_to_response(e: &CoreError, locale: Locale) -> (StatusCode, Json<ErrorResponse>) {
    // 5xx 类：仅服务端记录完整错误，客户端拿固定本地化文案。
    let message = match e {
        CoreError::DatabaseError(_, _) => {
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
        CoreError::EtcdError(_, _) => {
            tracing::error!(
                event = "core_error",
                variant = "etcd_error",
                error = ?e,
                "etcd error returned to client as generic message"
            );
            translate_with_locale(locale.as_str(), "api.error.etcd_error")
        }
        CoreError::IoError(_, _) => {
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
        // 4xx 携带调用方字符串：本地化后截断。
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
        // 4xx 无内层字符串：无调用方内容，无需截断。
        CoreError::RateLimitExceeded
        | CoreError::ApiKeyDisabled
        | CoreError::ApiKeyExpired
        | CoreError::InvalidApiKeySignature
        | CoreError::TimeoutError => e.to_localized_string(locale.as_str()),
    };

    // 状态码与业务码同表判定；request_id 优先取请求上下文（与 x-request-id
    // 响应头一致），无中间件上下文（单元测试 / 独立调用）时保留装配处生成的 UUID v4。
    let (status, business_code) = core_error_classification(e);
    let code = status.as_u16() as i32;
    let mut error = ErrorResponse::new(code, business_code, message);
    if let Some(request_id) = crate::server::middleware::request_id::current_request_id() {
        error.request_id = request_id;
    }
    (status, Json(error))
}

// ========== 共享授权 ==========

/// 共享的 workspace 资源级授权决策（HTTP 与 gRPC 同源）：User 仅可访问自身
/// workspace，Admin 跨租户放行，Anonymous 一律拒绝；调用方负责按名反查 UUID 等传输步骤。
///
/// 拒绝类别载体：User 跨 workspace → `WorkspaceDisabled`（唯一 403/FORBIDDEN
/// 载体，HTTP 侧重映射为 workspace mismatch，gRPC 映射 `permission_denied`）；
/// Anonymous → `AuthenticationError`（401 载体）；放行返回 `Ok(())`。
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
        ApiKeyRole::User => Err(CoreError::WorkspaceDisabled(t!(
            "error.detail.workspace_not_owned",
            workspace_id = target_workspace_id
        ))),
        // Anonymous（认证禁用时注入）：无业务权限，fail-closed。
        ApiKeyRole::Anonymous => Err(CoreError::AuthenticationError(
            "authentication required".to_string(),
        )),
    }
}

// ========== gRPC 错误消毒 ==========

/// gRPC 侧「变体 → Code」映射，与 `core_error_status_code` 同源：4xx 逐一对应
/// 同语义 Code，5xx 收敛 `Code::Internal`；不预造无变体的 `AlreadyExists`。
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

/// `CoreError` → 消毒后的 gRPC `Status`（与 HTTP 侧同源）。
///
/// 5xx 类：细节只进 `tracing::error!`，客户端只拿固定 `"internal error"`。
/// 4xx 类：本地化 Display 消息经 `sanitize_for_production` 截断（与 HTTP 同上限）。
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

/// 非法 UUID 路径参数 → 400（本地化文案）。
pub fn invalid_uuid_response(locale: Locale) -> (StatusCode, Json<ErrorResponse>) {
    let message = translate_with_locale(locale.as_str(), "api.error.invalid_uuid_format");
    (
        StatusCode::BAD_REQUEST,
        Json(ErrorResponse::new(400, ApiErrorCode::InvalidUuid, message)),
    )
}

/// `ValidationErrors` → 400（本地化文案）：用 `field_errors()` 提取 (field, rule)，
/// 不 stringify 整个 `ValidationErrors`，避免泄露 length/range 的 min/max 参数。
/// 多条以 `"; "` 连接，经 `sanitize_for_production` 截断以限制响应大小。
pub(crate) fn validation_error_response(
    errors: &sdforge::validator::ValidationErrors,
    locale: Locale,
) -> (StatusCode, Json<ErrorResponse>) {
    let mut parts: Vec<String> = Vec::new();
    for (field, field_errs) in errors.field_errors() {
        for err in field_errs {
            // `err.code` 只是规则名（无约束值），可安全下发客户端。
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
        // 无字段级错误（如仅嵌套结构错误）时回退通用 key，客户端仍得本地化文案。
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

/// Admin key 不可执行该操作 → 403。
pub(crate) fn admin_cannot_perform_response(locale: Locale) -> (StatusCode, Json<ErrorResponse>) {
    let message = translate_with_locale(locale.as_str(), "api.error.admin_cannot_perform");
    (
        StatusCode::FORBIDDEN,
        Json(ErrorResponse::new(403, ApiErrorCode::Forbidden, message)),
    )
}

/// Anonymous 角色（认证禁用时注入）访问受保护端点 → 401，要求有效 API key（CWE-1188）。
pub(crate) fn auth_required_response(locale: Locale) -> (StatusCode, Json<ErrorResponse>) {
    let message = translate_with_locale(locale.as_str(), "api.error.auth_required");
    (
        StatusCode::UNAUTHORIZED,
        Json(ErrorResponse::new(401, ApiErrorCode::Unauthorized, message)),
    )
}

/// workspace 不匹配 → 403。
pub(crate) fn workspace_mismatch_response(locale: Locale) -> (StatusCode, Json<ErrorResponse>) {
    let message = translate_with_locale(locale.as_str(), "api.error.workspace_mismatch");
    (
        StatusCode::FORBIDDEN,
        Json(ErrorResponse::new(403, ApiErrorCode::Forbidden, message)),
    )
}

/// workspace 名未找到 → 404；`name` 来自 URL 路径且受调用方控制，
/// 超 64 字节先按 char 边界截断再插值，限制响应体大小与日志注入面。
pub(crate) fn workspace_name_not_found_response(
    name: &str,
    locale: Locale,
) -> (StatusCode, Json<ErrorResponse>) {
    const MAX_WORKSPACE_NAME_LEN: usize = 64;
    let safe_name: String = if name.len() > MAX_WORKSPACE_NAME_LEN {
        // 按 char 边界向下取整，避免切断多字节 UTF-8。
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

/// workspace 未找到（不带名字）→ 404。
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

/// Invalid workspace ID → 500。
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

/// 用户 key 缺 workspace_id → 400。
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
    use sdforge::validator::Validate;

    /// 保存并在测试期间恢复全局 locale（与 i18n.rs / error.rs 共用 test_support 锁串行化）。
    struct LocaleGuard {
        saved: String,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl LocaleGuard {
        fn new() -> Self {
            Self {
                saved: crate::core::i18n::current_locale(),
                _lock: crate::core::i18n::test_support::lock(),
            }
        }
    }

    impl Drop for LocaleGuard {
        fn drop(&mut self) {
            crate::core::i18n::init_i18n(&self.saved);
        }
    }

    #[test]
    fn test_core_error_to_response_en() {
        let _g = LocaleGuard::new();
        crate::core::i18n::init_i18n("en");

        let (status, json) =
            core_error_to_response(&CoreError::InvalidInput("negative".to_string()), Locale::En);
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json.code, 400);
        assert_eq!(json.message, "Invalid input: negative");
    }

    #[test]
    fn test_core_error_to_response_zh_cn() {
        let _g = LocaleGuard::new();
        crate::core::i18n::init_i18n("en");

        let (status, json) = core_error_to_response(
            &CoreError::InvalidInput("negative".to_string()),
            Locale::ZhCn,
        );
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json.code, 400);
        assert_eq!(json.message, "无效输入：negative");

        // 全局 locale 必须保持 "en"
        assert_eq!(crate::core::i18n::current_locale(), "en");
    }

    #[test]
    fn test_core_error_to_response_status_codes() {
        let _g = LocaleGuard::new();
        crate::core::i18n::init_i18n("en");

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

        // 500：5xx 变体全量覆盖
        let (s, _) =
            core_error_to_response(&CoreError::DatabaseError("x".to_string(), None), Locale::En);
        assert_eq!(s, StatusCode::INTERNAL_SERVER_ERROR);
        let (s, _) = core_error_to_response(&CoreError::CacheError("x".to_string()), Locale::En);
        assert_eq!(s, StatusCode::INTERNAL_SERVER_ERROR);
        let (s, _) = core_error_to_response(&CoreError::InternalError("x".to_string()), Locale::En);
        assert_eq!(s, StatusCode::INTERNAL_SERVER_ERROR);
        let (s, _) =
            core_error_to_response(&CoreError::ConfigurationError("x".to_string()), Locale::En);
        assert_eq!(s, StatusCode::INTERNAL_SERVER_ERROR);
        let (s, _) =
            core_error_to_response(&CoreError::EtcdError("x".to_string(), None), Locale::En);
        assert_eq!(s, StatusCode::INTERNAL_SERVER_ERROR);
        let (s, _) = core_error_to_response(&CoreError::IoError("x".to_string(), None), Locale::En);
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

    // ========== —— 统一错误信封 business_code ==========

    /// 单表分类全变体矩阵：`core_error_classification` 的 (状态码, business_code)
    /// 必须与既有映射逐行一致（一张映射表两用的钉桩）。
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
                CoreError::DatabaseError("x".into(), None),
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
                CoreError::EtcdError("x".into(), None),
                StatusCode::INTERNAL_SERVER_ERROR,
                ApiErrorCode::InternalError,
            ),
            (
                CoreError::IoError("x".into(), None),
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
            // 同源性：core_error_status_code 与单表 .0 一致
            assert_eq!(core_error_status_code(e), *expected_status);
        }
    }

    /// 任务规定的三类代表性错误信封断言：限流 → 4001；校验失败 → 3002；资源不存在 → 2001/2003。
    #[test]
    fn test_error_envelope_business_codes_for_required_classes() {
        let _g = LocaleGuard::new();
        crate::core::i18n::init_i18n("en");

        // 限流：RateLimitExceeded → "4001"
        let (_, json) = core_error_to_response(&CoreError::RateLimitExceeded, Locale::En);
        assert_eq!(json.business_code, "4001");
        assert_eq!(json.code, 429);

        // 校验失败：validation_error_response → "3002"
        #[derive(sdforge::validator::Validate)]
        #[validate(crate = "::sdforge::validator")]
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
        // workspace 名未找到（handler 构造）→ "2001"
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

    /// handler 构造类错误的业务码：UUID 非法 3004、缺必填 3003、跨 workspace/Admin 1002、未认证 1001。
    #[test]
    fn test_error_envelope_business_codes_for_handler_constructed() {
        let _g = LocaleGuard::new();
        crate::core::i18n::init_i18n("en");

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

    /// 信封装配：响应必须携带 UUID v4 request_id 与毫秒级 timestamp。
    #[test]
    fn test_core_error_to_response_envelope_tracks_request() {
        let _g = LocaleGuard::new();
        crate::core::i18n::init_i18n("en");

        let (_, json) =
            core_error_to_response(&CoreError::InvalidInput("x".to_string()), Locale::En);
        let parsed =
            uuid::Uuid::parse_str(&json.request_id).expect("request_id must be a valid UUID");
        assert_eq!(parsed.get_version_num(), 4);
        assert!(json.timestamp > 0, "timestamp must be millis since epoch");
    }

    /// 5xx 内部错误不得把内层 `String` 泄露给客户端：完整错误仅进 `tracing::error!`，
    /// 客户端只拿通用本地化文案。
    #[test]
    fn test_core_error_to_response_5xx_does_not_leak_internal_string() {
        let _g = LocaleGuard::new();
        crate::core::i18n::init_i18n("en");

        // 典型泄露样本：上游 DB 错误内嵌的连接串
        let sensitive = "postgres://user:pwd@internal-host:5432/nebulaid";
        let (status, json) = core_error_to_response(
            &CoreError::DatabaseError(sensitive.to_string(), None),
            Locale::En,
        );
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

        // zh-CN locale 返回翻译后的通用文案
        let (_, json_zh) = core_error_to_response(
            &CoreError::DatabaseError(sensitive.to_string(), None),
            Locale::ZhCn,
        );
        assert_eq!(json_zh.message, "数据库操作失败");
        assert!(!json_zh.message.contains(sensitive));

        // CacheError：敏感 Redis URL
        let redis_url = "redis://:secret@cache.internal:6379/0";
        let (_, json) =
            core_error_to_response(&CoreError::CacheError(redis_url.to_string()), Locale::En);
        assert_eq!(json.message, "Cache service unavailable");
        assert!(!json.message.contains(redis_url));
        assert!(!json.message.contains("secret"));

        // InternalError：敏感文件路径
        let path = "/etc/nebulaid/secrets/admin.key";
        let (_, json) =
            core_error_to_response(&CoreError::InternalError(path.to_string()), Locale::En);
        assert_eq!(json.message, "Internal server error");
        assert!(!json.message.contains(path));

        // ConfigurationError：敏感环境变量值
        let env_leak = "NEBULA_DATABASE_PASSWORD=hunter2";
        let (_, json) = core_error_to_response(
            &CoreError::ConfigurationError(env_leak.to_string()),
            Locale::En,
        );
        assert_eq!(json.message, "Configuration error");
        assert!(!json.message.contains("hunter2"));
        assert!(!json.message.contains("NEBULA_DATABASE_PASSWORD"));

        // EtcdError：敏感端点
        let etcd_url = "http://etcd.internal:2379, token=admin-token";
        let (_, json) = core_error_to_response(
            &CoreError::EtcdError(etcd_url.to_string(), None),
            Locale::En,
        );
        assert_eq!(json.message, "Etcd service unavailable");
        assert!(!json.message.contains(etcd_url));
        assert!(!json.message.contains("admin-token"));

        // IoError：敏感文件系统路径
        let fs_path = "/var/lib/nebulaid/private/keys/0xdeadbeef.pem";
        let (_, json) =
            core_error_to_response(&CoreError::IoError(fs_path.to_string(), None), Locale::En);
        assert_eq!(json.message, "I/O error");
        assert!(!json.message.contains(fs_path));

        // 算法类错误只带数值上下文，同样验证不下发。
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

        // Unknown：无内层字符串，仍返回通用文案
        let (_, json) = core_error_to_response(&CoreError::Unknown, Locale::En);
        assert_eq!(json.message, "Internal server error");
    }

    /// 4xx 携带调用方 `String` 必须经 `sanitize_for_production` 截断到 200 字节，防响应撑大。
    #[test]
    fn test_core_error_to_response_4xx_sanitizes_long_message() {
        let _g = LocaleGuard::new();
        crate::core::i18n::init_i18n("en");

        // 300 字节载荷，超过 200 字节上限
        let big = "x".repeat(300);
        let (status, json) =
            core_error_to_response(&CoreError::InvalidInput(big.clone()), Locale::En);
        assert_eq!(status, StatusCode::BAD_REQUEST);
        // 前缀 "Invalid input: "（14 字节）+ 200 字节 + 标记（15 字节）≈ 229 字节
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
        // 消息仍以本地化前缀开头
        assert!(
            json.message.starts_with("Invalid input: x"),
            "truncated message must preserve the localized prefix"
        );

        // ParseError 同走 4xx 截断路径
        let big_parse = "y".repeat(500);
        let (status, json) =
            core_error_to_response(&CoreError::ParseError(big_parse.clone()), Locale::En);
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(json.message.ends_with("... (truncated)"));
        assert!(json.message.len() < 500);

        // 短 4xx 消息原样返回，不截断
        let (status, json) =
            core_error_to_response(&CoreError::InvalidInput("short".to_string()), Locale::En);
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json.message, "Invalid input: short");
        assert!(!json.message.contains("truncated"));

        // zh-CN：多字节字符同样截断且不切断 char 边界（"无效输入：" 前缀 12 字节）
        let big_zh = "长".repeat(100); // 100 * 3 bytes = 300 bytes
        let (_, json_zh) = core_error_to_response(&CoreError::InvalidInput(big_zh), Locale::ZhCn);
        assert!(json_zh.message.ends_with("... (truncated)"));
        // 消息必须是合法 UTF-8
        assert!(std::str::from_utf8(json_zh.message.as_bytes()).is_ok());
    }

    /// 无内层 `String` 的 4xx（RateLimitExceeded/ApiKeyDisabled/ApiKeyExpired/
    /// InvalidApiKeySignature/TimeoutError）原样返回本地化文案，不截断。
    #[test]
    fn test_core_error_to_response_4xx_no_inner_string_verbatim() {
        let _g = LocaleGuard::new();
        crate::core::i18n::init_i18n("en");

        let (_, json) = core_error_to_response(&CoreError::RateLimitExceeded, Locale::En);
        assert_eq!(json.message, "Rate limit exceeded");

        let (_, json) = core_error_to_response(&CoreError::ApiKeyDisabled, Locale::En);
        assert_eq!(json.message, "API key disabled");

        let (_, json) = core_error_to_response(&CoreError::ApiKeyExpired, Locale::En);
        assert_eq!(json.message, "API key expired");

        let (_, json) = core_error_to_response(&CoreError::InvalidApiKeySignature, Locale::En);
        assert_eq!(json.message, "Invalid API key signature");

        // TimeoutError 映射 503，Display 无内层字符串，验证原样消息
        let (status, json) = core_error_to_response(&CoreError::TimeoutError, Locale::En);
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(json.message, "Timeout error");
        assert!(!json.message.contains("truncated"));
    }

    /// `sanitize_for_production` 直测：char 边界安全与短消息原样返回。
    #[test]
    fn test_sanitize_for_production_direct() {
        // 短消息：原样
        assert_eq!(sanitize_for_production("hello"), "hello");
        assert_eq!(sanitize_for_production(""), "");

        // 恰好等于上限：不截断（边界是 `>` 而非 `>=`）
        let exact = "a".repeat(MAX_CLIENT_MESSAGE_LEN);
        assert_eq!(
            sanitize_for_production(&exact).len(),
            MAX_CLIENT_MESSAGE_LEN
        );
        assert!(!sanitize_for_production(&exact).contains("truncated"));

        // 超上限 1 字节即截断：净化结果是 prefix + "... (truncated)"，ASCII 下
        // prefix 恰为上限；输入仅超 1 字节时净化结果反而更长，属预期（标记本身携带信息）。
        // 不变量是 prefix（内容部分）不超过上限。
        let one_over = "a".repeat(MAX_CLIENT_MESSAGE_LEN + 1);
        let sanitized = sanitize_for_production(&one_over);
        assert!(sanitized.ends_with("... (truncated)"));
        let prefix_len = sanitized.len() - "... (truncated)".len();
        assert!(prefix_len <= MAX_CLIENT_MESSAGE_LEN);
        assert_eq!(prefix_len, MAX_CLIENT_MESSAGE_LEN); // ASCII round-down is exact

        // 明显超上限：净化后必须短于原输入。
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

        // 多字节边界安全：200 字节处不得切断 UTF-8（🤖 为 4 字节，200 % 4 == 0）。
        let multi = "🤖".repeat(100); // 400 bytes total
        let sanitized = sanitize_for_production(&multi);
        assert!(sanitized.ends_with("... (truncated)"));
        // 截断前缀必须是合法 UTF-8（`is_char_boundary` 向下取整）。
        let prefix_end = sanitized.len() - "... (truncated)".len();
        assert!(std::str::from_utf8(&sanitized.as_bytes()[..prefix_end]).is_ok());
        // 前缀长度须为 4 的倍数且不超过上限。
        assert_eq!(prefix_end % 4, 0);
        assert!(prefix_end <= MAX_CLIENT_MESSAGE_LEN);
    }

    // ========== authorize_workspace_access（共享授权）==========

    /// User 访问自身 workspace → 放行。
    #[tokio::test]
    async fn test_authorize_workspace_access_user_own_workspace_ok() {
        let ws = uuid::Uuid::new_v4();
        let result = authorize_workspace_access(&ApiKeyRole::User, ws, ws).await;
        assert!(result.is_ok(), "User must access its own workspace");
    }

    /// User 跨 workspace → 拒绝，变体 `WorkspaceDisabled`（映射表唯一 403 载体，
    /// 与 HTTP 侧 workspace_mismatch 同类）。
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

    /// Anonymous（认证禁用时注入）→ 一律拒绝（fail-closed），变体 `AuthenticationError`（401 载体）。
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

    // ========== core_error_to_grpc_status（gRPC 错误消毒）==========

    /// 5xx 必须消毒：Code 固定 Internal、message 固定 "internal error"，
    /// 不得携带 Display 明文或内层敏感串（与 HTTP 侧 5xx 泛化承诺对齐）。
    #[test]
    fn test_core_error_to_grpc_status_sanitizes_5xx() {
        let sensitive = "postgres://idgen:pwd@internal-host:5432/nebulaid";
        let e = CoreError::DatabaseError(format!("Database error: {sensitive}"), None);
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

    /// NotFound → Code::NotFound 且消息保留；4xx 逐一映射同语义 Code（抽验 400/401/403/404/429/503）。
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
        crate::core::i18n::init_i18n("en");
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
        crate::core::i18n::init_i18n("en");
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
        crate::core::i18n::init_i18n("en");
        let (status, json) = workspace_name_not_found_response("my-ws", Locale::En);
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(json.message, "Workspace 'my-ws' not found");

        let (status, json) = workspace_name_not_found_response("my-ws", Locale::ZhCn);
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(json.message, "工作空间 'my-ws' 未找到");
    }

    /// 超长 workspace `name` 必须先截断再插值；验证 char 边界安全与响应有界。
    #[test]
    fn test_workspace_name_not_found_response_truncates_long_name() {
        let _g = LocaleGuard::new();
        crate::core::i18n::init_i18n("en");

        // 200 字节 ASCII name，远超 64 字节上限
        let long_name = "a".repeat(200);
        let (status, json) = workspace_name_not_found_response(&long_name, Locale::En);
        assert_eq!(status, StatusCode::NOT_FOUND);
        // 必须出现截断标记
        assert!(
            json.message.contains("..."),
            "expected truncation marker '...' in message, got: {}",
            json.message
        );
        // 消息总长有界：前缀 + 64 字节 + "..." ≈ 84 字节，断言 < 100 留 locale 余量。
        assert!(
            json.message.len() < 100,
            "truncated response message must be < 100 bytes, got len={} msg={:?}",
            json.message.len(),
            json.message
        );
        // 完整的 200 字节 name 不得原样出现
        assert!(
            !json.message.contains(&long_name),
            "full long name must not appear in message"
        );

        // 多字节边界：100 × '长'（300 字节），64 不是 char 边界（64 % 3 != 0），
        // 须向下取整到 63 字节（21 字符）；消息必须合法 UTF-8。
        let long_zh_name = "长".repeat(100);
        let (_, json_zh) = workspace_name_not_found_response(&long_zh_name, Locale::ZhCn);
        assert!(json_zh.message.contains("..."));
        assert!(std::str::from_utf8(json_zh.message.as_bytes()).is_ok());
        assert!(
            json_zh.message.len() < 100,
            "truncated zh message must be < 100 bytes, got len={}",
            json_zh.message.len()
        );

        // 边界：恰好 64 字节，不截断
        let exact = "b".repeat(64);
        let (status, json) = workspace_name_not_found_response(&exact, Locale::En);
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(
            !json.message.contains("..."),
            "exact-cap name must not be truncated, got: {}",
            json.message
        );
        assert!(json.message.contains(&exact));

        // 边界：65 字节，截断
        let over_by_one = "c".repeat(65);
        let (_, json) = workspace_name_not_found_response(&over_by_one, Locale::En);
        assert!(
            json.message.contains("..."),
            "over-cap name must be truncated, got: {}",
            json.message
        );
        // 截断前缀是 64 字节 'c' + "..."
        assert!(json.message.contains(&"c".repeat(64)));
        assert!(!json.message.contains(&"c".repeat(65)));
    }

    #[test]
    fn test_workspace_id_required_response() {
        let _g = LocaleGuard::new();
        crate::core::i18n::init_i18n("en");
        let (status, json) = workspace_id_required_response(Locale::En);
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(json.message.contains("workspace_id"));

        let (status, json) = workspace_id_required_response(Locale::ZhCn);
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(json.message.contains("workspace_id"));
    }

    /// `validation_error_response` 必须给出字段名与规则码（length/range），
    /// 但不得泄露 min/max 约束值。
    #[test]
    fn test_validation_error_response_en() {
        let _g = LocaleGuard::new();
        crate::core::i18n::init_i18n("en");

        // 构造 length(min = 1, max = 64)，以空串触发校验失败。
        #[derive(sdforge::validator::Validate)]
        #[validate(crate = "::sdforge::validator")]
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
        // 字段名与规则码必须出现
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
        // 约束值不得泄露
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

    /// zh-CN 路径，断言与英文版相同。
    #[test]
    fn test_validation_error_response_zh_cn() {
        let _g = LocaleGuard::new();
        crate::core::i18n::init_i18n("en");

        #[derive(sdforge::validator::Validate)]
        #[validate(crate = "::sdforge::validator")]
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

    /// 回归：range(min = 100, max = 1000000) 校验失败的响应不得含
    /// "100"、"1000000"、"min = 100"、"max = 1000000"。
    #[test]
    fn test_validation_error_response_does_not_leak_constraints() {
        let _g = LocaleGuard::new();
        crate::core::i18n::init_i18n("en");

        #[derive(sdforge::validator::Validate)]
        #[validate(crate = "::sdforge::validator")]
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
        // 数值约束不得泄露
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

    /// 多条字段错误以 "; " 连接，并经 `sanitize_for_production` 截断。
    #[test]
    fn test_validation_error_response_multiple_fields_joined() {
        let _g = LocaleGuard::new();
        crate::core::i18n::init_i18n("en");

        #[derive(sdforge::validator::Validate)]
        #[validate(crate = "::sdforge::validator")]
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
        // 两个字段名都要出现，以 "; " 分隔
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
        crate::core::i18n::init_i18n("en");

        let (status, json) = workspace_not_found_response(Locale::En);
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(!json.message.is_empty());

        let (status, json) = invalid_workspace_id_response(Locale::En);
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(!json.message.is_empty());
    }
}
