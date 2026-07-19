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

use crate::server::api_version::{api_version_middleware, API_V1};
use crate::server::audit::{AuditLogger, AuditMiddleware};
use crate::server::config::{cors, management::ConfigManagementService};
use crate::server::handlers::helpers::{
    admin_cannot_perform_response, auth_required_response, core_error_to_response,
    invalid_uuid_response, invalid_workspace_id_response, validation_error_response,
    workspace_id_required_response, workspace_mismatch_response, workspace_name_not_found_response,
    workspace_not_found_response,
};
use crate::server::handlers::ApiHandlers;
use crate::server::middleware::locale::Locale;
use crate::server::middleware::{locale_middleware, ApiKeyAuth};
use crate::server::models::{
    ApiInfoResponse, ApiKeyListResponse, ApiKeyWithSecretResponse, BatchGenerateRequest,
    BatchGenerateResponse, BizTagListResponse, BizTagResponse, CreateApiKeyRequest,
    CreateBizTagRequest, CreateGroupRequest, CreateWorkspaceRequest, ErrorResponse,
    GenerateRequest, GenerateResponse, GroupListParams, GroupListResponse, GroupResponse,
    HealthResponse, MetricsResponse, PaginationParams, ParseRequest, ParseResponse, ReadyResponse,
    RevokeApiKeyResponse, SecureConfigResponse, SetAlgorithmRequest, SetAlgorithmResponse,
    UpdateBizTagRequest, UpdateConfigResponse, UpdateLoggingRequest, UpdateRateLimitRequest,
    WorkspaceListResponse, WorkspaceResponse,
};
use crate::server::rate_limit::{limiter::RateLimiter, middleware::RateLimitMiddleware};
use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderValue, StatusCode},
    routing::{delete, get, post},
    Extension, Json, Router,
};
use std::sync::Arc;
use tower_http::set_header::SetResponseHeaderLayer;
use validator::Validate;

#[derive(Clone)]
pub struct AppState {
    pub handlers: Arc<ApiHandlers>,
    pub auth: Arc<ApiKeyAuth>,
    pub config_service: Arc<dyn ConfigManagementService>,
}

pub async fn create_router(
    handlers: Arc<ApiHandlers>,
    auth: Arc<ApiKeyAuth>,
    rate_limiter: Arc<RateLimiter>,
    audit_logger: Arc<AuditLogger>,
) -> Router {
    // Configure CORS with environment-aware settings
    // In production, specify your actual frontend origins via ALLOWED_ORIGINS env var
    // Format: comma-separated list of allowed origins
    // Example: ALLOWED_ORIGINS="https://example.com,https://app.example.com"
    let cors = cors::create_env_aware_cors_layer();

    // Phase 9 T043 (HIGH H3) — read trusted proxies from the same env
    // var that `main.rs` uses for `ApiKeyAuth`. Keeping a single source
    // of truth (env var) avoids plumbing a new parameter through
    // `create_router` and its 6 call sites. Default: empty list (no
    // headers trusted).
    let trusted_proxies: Vec<std::net::IpAddr> = std::env::var("NEBULA_TRUSTED_PROXIES")
        .ok()
        .map(|s| s.split(',').filter_map(|p| p.trim().parse().ok()).collect())
        .unwrap_or_default();

    let rate_limit_middleware = RateLimitMiddleware::new(rate_limiter.clone())
        .with_trusted_proxies(trusted_proxies.clone());
    let audit_middleware = AuditMiddleware::new(audit_logger.clone(), auth.clone(), rate_limiter)
        .with_trusted_proxies(trusted_proxies);

    let config_service = handlers.get_config_service();

    let app_state = AppState {
        handlers: handlers.clone(),
        auth: auth.clone(),
        config_service: config_service.clone(),
    };

    // ========== V1 API Routes ==========
    // Admin-only endpoints (require admin API key)
    let v1_admin_routes = Router::new()
        // API Key management endpoints (admin only)
        .route(
            "/api-keys",
            post(handle_create_api_key).get(handle_list_api_keys),
        )
        .route("/api-keys/{id}", delete(handle_revoke_api_key))
        // Workspace creation (admin only)
        .route("/workspaces", post(handle_create_workspace))
        // Workspace user key regeneration (admin only)
        .route(
            "/workspaces/{name}/regenerate-user-key",
            post(handle_regenerate_user_key),
        )
        // Apply admin requirement middleware first, then auth middleware
        // This ensures auth runs first to set the ApiKeyRole extension
        .layer(axum::middleware::from_fn(
            crate::server::middleware::admin_required_middleware,
        ))
        .layer(axum::middleware::from_fn_with_state(
            auth.clone(),
            crate::server::middleware::auth_middleware_fn,
        ));

    // Authenticated endpoints (require API key)
    let v1_authenticated_routes = Router::new()
        .route("/config", get(handle_get_config))
        .route("/config/rate-limit", post(handle_update_rate_limit))
        .route("/config/logging", post(handle_update_logging))
        .route("/config/reload", post(handle_reload_config))
        .route("/config/algorithm", post(handle_set_algorithm))
        // ID generation (user only)
        .route("/generate", post(handle_generate))
        .route("/generate/batch", post(handle_batch_generate))
        .route("/parse", post(handle_parse))
        // Workspace query endpoints
        .route("/workspaces", get(handle_list_workspaces))
        .route("/workspaces/{name}", get(handle_get_workspace))
        // Group CRUD endpoints
        .route("/groups", post(handle_create_group).get(handle_list_groups))
        // BizTag CRUD endpoints
        .route(
            "/biz-tags",
            post(handle_create_biz_tag).get(handle_list_biz_tags),
        )
        .route(
            "/biz-tags/{id}",
            get(handle_get_biz_tag)
                .put(handle_update_biz_tag)
                .delete(handle_delete_biz_tag),
        )
        // Apply auth middleware
        //
        // SEC-CRITICAL-001 修复（CWE-1188）—— axum 0.8.x layer 语义：
        // 后 `.layer()` 的中间件先执行（outer），先 `.layer()` 的后执行
        // （inner）。因此此处必须先 `anonymous_block_middleware`（inner，
        // 后执行），再 `auth_middleware_fn`（outer，先执行注入
        // `ApiKeyRole` 扩展）。这样 `anonymous_block_middleware` 执行时
        // `ApiKeyRole` 扩展已由 `auth_middleware_fn` 注入，可正确拒绝
        // Anonymous 角色。前次修复顺序相反，导致中间件完全无效。
        .layer(axum::middleware::from_fn(anonymous_block_middleware))
        .layer(axum::middleware::from_fn_with_state(
            auth.clone(),
            crate::server::middleware::auth_middleware_fn,
        ));

    // Public endpoints (no authentication)
    let v1_public_routes = Router::new()
        .route("/", get(handle_api_info))
        .merge(v1_authenticated_routes)
        .merge(v1_admin_routes);

    // ========== API Versioning ==========
    // Nest V1 routes under /api/v1/
    let api_v1_routes = Router::new()
        .nest(&format!("/api/{}", API_V1), v1_public_routes)
        // Apply API version middleware to all API routes
        .layer(axum::middleware::from_fn(api_version_middleware))
        // Phase 8 T041 (M4 perf fix) — locale negotiation only needs
        // to run on `/api/v1/*` routes (handlers under this prefix
        // read `Extension<Locale>` for error translation). `/health`,
        // `/ready`, `/metrics`, and `/api-docs/openapi.json` do not
        // consume the locale and were needlessly paying the
        // `Accept-Language` parse cost on every request.
        .layer(axum::middleware::from_fn(locale_middleware));

    // ========== Root Routes ==========
    // Public router (no authentication) - includes health check and metrics
    Router::new()
        .route("/health", get(handle_health))
        .route("/ready", get(handle_ready))
        .route("/metrics", get(handle_metrics))
        .route(
            "/api-docs/openapi.json",
            get(crate::server::openapi::openapi_json_handler),
        )
        .merge(api_v1_routes)
        .with_state(app_state)
        // Security headers
        .layer(SetResponseHeaderLayer::overriding(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::X_FRAME_OPTIONS,
            HeaderValue::from_static("DENY"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static("default-src 'self'"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::STRICT_TRANSPORT_SECURITY,
            HeaderValue::from_static("max-age=31536000; includeSubDomains"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::X_XSS_PROTECTION,
            HeaderValue::from_static("1; mode=block"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::REFERRER_POLICY,
            HeaderValue::from_static("strict-origin-when-cross-origin"),
        ))
        .layer(cors)
        .layer(axum::Extension(rate_limit_middleware))
        .layer(axum::Extension(audit_middleware))
        .layer(axum::Extension(audit_logger))
}

// ========== Helper Functions ==========

/// SEC-CRITICAL-001 修复（CWE-1188）：v1_authenticated_routes 的全局
/// Anonymous 拒绝中间件。
///
/// **背景**：LOW-1 修复在禁用认证时把请求注入 `ApiKeyRole::Anonymous`，
/// 但 `verify_user_role` 只在 4 个 handler（generate/batch_generate/
/// create_biz_tag/create_group）中调用，其余 13 个 authenticated
/// endpoint（含 `POST /config/algorithm`、`DELETE /biz-tags/{id}` 等
/// 破坏性操作）对 Anonymous 完全开放，违反「Anonymous 无业务权限」契约。
///
/// **修复**：在 `v1_authenticated_routes` 的 `auth_middleware_fn` 之后
/// 叠加本中间件，统一拒绝 Anonymous 角色。这样所有 v1 authenticated
/// endpoint 都受保护，无需逐个 handler 调用 `verify_user_role`。
///
/// **layer 顺序**：本中间件必须作为 inner layer（先 `.layer()`），
/// `auth_middleware_fn` 作为 outer layer（后 `.layer()`），保证
/// `auth_middleware_fn` 先执行并注入 `ApiKeyRole` 扩展。详见
/// `v1_authenticated_routes` 处的注释。
///
/// **NEW-LOW-002 修复（规则 12 失败显性化）**：若 `ApiKeyRole` 扩展
/// 缺失（说明 `auth_middleware_fn` 未执行或未注入），fail-closed 返回
/// 401 + warn 日志，避免静默放行。
///
/// `verify_user_role` 保留用于 handler 内的 Admin 拒绝场景。
pub async fn anonymous_block_middleware(
    req: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    match req
        .extensions()
        .get::<crate::server::middleware::ApiKeyRole>()
    {
        Some(role) if *role == crate::server::middleware::ApiKeyRole::Anonymous => {
            // Locale 由外层 locale_middleware 注入；若未注入则用默认值。
            let locale = req
                .extensions()
                .get::<Locale>()
                .cloned()
                .unwrap_or_default();
            auth_required_response(locale).into_response()
        }
        Some(_) => next.run(req).await,
        None => {
            // NEW-LOW-002：ApiKeyRole 扩展缺失，fail-closed。
            tracing::warn!(
                path = %req.uri().path(),
                method = %req.method(),
                "ApiKeyRole extension missing in anonymous_block_middleware; \
                 auth_middleware_fn may not have run; denying request"
            );
            let locale = req
                .extensions()
                .get::<Locale>()
                .cloned()
                .unwrap_or_default();
            auth_required_response(locale).into_response()
        }
    }
}

/// Verify that the request is from a User API key (not Admin, not Anonymous).
///
/// Phase 8 T041 — returns a locale-translated error response when the
/// caller is an Admin key.
///
/// LOW-1 修复（CWE-1188）：当认证被禁用时，请求会携带 `ApiKeyRole::Anonymous`
/// 扩展。此角色无业务权限，必须被拒绝（避免禁用认证时所有端点都开放）。
fn verify_user_role(
    role: crate::server::middleware::ApiKeyRole,
    locale: Locale,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    if role == crate::server::middleware::ApiKeyRole::Admin {
        return Err(admin_cannot_perform_response(locale));
    }
    if role == crate::server::middleware::ApiKeyRole::Anonymous {
        return Err(auth_required_response(locale));
    }
    Ok(())
}

/// Validate request and return locale-translated error if invalid.
fn validate_request<T: Validate>(
    req: &T,
    locale: Locale,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    if let Err(validation_errors) = req.validate() {
        return Err(validation_error_response(&validation_errors, locale));
    }
    Ok(())
}

/// Verify workspace_id match for User API Key (by workspace name lookup).
///
/// Phase 8 T041 — all error responses are locale-translated.
async fn verify_user_workspace(
    workspace_name: &str,
    key_workspace_id: &Option<uuid::Uuid>,
    handlers: &Arc<ApiHandlers>,
    locale: Locale,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    let workspace_result = handlers
        .get_workspace(workspace_name)
        .await
        .map_err(|e| core_error_to_response(&e, locale))?;

    let workspace = workspace_result
        .ok_or_else(|| workspace_name_not_found_response(workspace_name, locale))?;

    let workspace_uuid =
        uuid::Uuid::parse_str(&workspace.id).map_err(|_| invalid_workspace_id_response(locale))?;

    verify_workspace_id_match(workspace_uuid, key_workspace_id, locale)
}

/// 提取通用的 workspace_id 比较逻辑
///
/// Phase 8 T041 — returns locale-translated error on mismatch.
fn verify_workspace_id_match(
    workspace_uuid: uuid::Uuid,
    key_workspace_id: &Option<uuid::Uuid>,
    locale: Locale,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    // 当认证禁用时，key_workspace_id 为 None，允许访问任何 workspace
    if key_workspace_id.is_none() {
        return Ok(());
    }
    if Some(workspace_uuid) != *key_workspace_id {
        return Err(workspace_mismatch_response(locale));
    }
    Ok(())
}

/// Verify workspace_id match for User API Key (direct Uuid comparison)
fn verify_workspace_id(
    req_workspace_id: uuid::Uuid,
    key_workspace_id: &Option<uuid::Uuid>,
    locale: Locale,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    verify_workspace_id_match(req_workspace_id, key_workspace_id, locale)
}

async fn handle_generate(
    State(state): State<AppState>,
    extensions: axum::Extension<Option<uuid::Uuid>>,
    extensions_role: axum::Extension<crate::server::middleware::ApiKeyRole>,
    Extension(locale): Extension<Locale>,
    Json(req): Json<GenerateRequest>,
) -> Result<Json<GenerateResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Only User API Key can generate IDs
    verify_user_role(extensions_role.0, locale)?;

    // Validate request parameters
    validate_request(&req, locale)?;

    // Verify workspace_id match for User API Key
    verify_user_workspace(&req.workspace, &extensions.0, &state.handlers, locale).await?;

    state
        .handlers
        .generate(req)
        .await
        .map(Json)
        .map_err(|e: crate::core::types::CoreError| core_error_to_response(&e, locale))
}

async fn handle_batch_generate(
    State(state): State<AppState>,
    extensions: axum::Extension<Option<uuid::Uuid>>,
    extensions_role: axum::Extension<crate::server::middleware::ApiKeyRole>,
    Extension(locale): Extension<Locale>,
    Json(req): Json<BatchGenerateRequest>,
) -> Result<Json<BatchGenerateResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Only User API Key can generate IDs
    verify_user_role(extensions_role.0, locale)?;

    // Phase 8 T041 (M5 perf fix) — server-side log uses structured
    // fields only; no `t!()` translation. The downstream
    // `core_error_to_response` already localizes the client-facing
    // message, so a second `t!()` lookup here would be redundant
    // work (and would mutate global locale state under the
    // `tracing::warn!` path).
    tracing::debug!(
        event = "batch_generate_request",
        workspace = %req.workspace,
        group = %req.group,
        size = ?req.size,
    );

    // Validate request parameters
    validate_request(&req, locale)?;

    // Verify workspace_id match for User API Key
    verify_user_workspace(&req.workspace, &extensions.0, &state.handlers, locale).await?;

    // Phase 8 T041 (HIGH H-2 fix) — route through `core_error_to_response`
    // so 5xx internal errors are sanitized to generic messages (CRITICAL
    // C-1) and 4xx caller-supplied strings are length-capped. The helper
    // records 5xx via `tracing::error!`; we additionally `warn!` 4xx
    // client errors for observability without re-implementing the
    // status-code mapping in this handler.
    match state.handlers.batch_generate(req).await {
        Ok(resp) => Ok(Json(resp)),
        Err(e) => {
            let resp = core_error_to_response(&e, locale);
            if resp.0.is_client_error() {
                tracing::warn!(
                    event = "batch_generation_failed",
                    status = %resp.0,
                    error = ?e,
                    "batch_generate returned 4xx"
                );
            }
            Err(resp)
        }
    }
}

async fn handle_health(State(state): State<AppState>) -> Json<HealthResponse> {
    Json(state.handlers.health().await)
}

async fn handle_ready(State(state): State<AppState>) -> Json<ReadyResponse> {
    Json(state.handlers.ready().await)
}

async fn handle_metrics(State(state): State<AppState>) -> Json<MetricsResponse> {
    Json(state.handlers.metrics().await)
}

async fn handle_parse(
    State(state): State<AppState>,
    Extension(locale): Extension<Locale>,
    Json(req): Json<ParseRequest>,
) -> Result<Json<ParseResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Validate request parameters
    if let Err(validation_errors) = req.validate() {
        return Err(validation_error_response(&validation_errors, locale));
    }

    // Phase 8 T041 (HIGH H-2 fix) — route through `core_error_to_response`
    // so 5xx internal errors are sanitized to generic messages (CRITICAL
    // C-1) and 4xx caller-supplied strings are length-capped. The helper
    // records 5xx via `tracing::error!`; we additionally `warn!` 4xx
    // client errors for observability without re-implementing the
    // status-code mapping in this handler.
    match state.handlers.parse(req).await {
        Ok(resp) => Ok(Json(resp)),
        Err(e) => {
            let resp = core_error_to_response(&e, locale);
            if resp.0.is_client_error() {
                tracing::warn!(
                    event = "parse_failed",
                    status = %resp.0,
                    error = ?e,
                    "parse returned 4xx"
                );
            }
            Err(resp)
        }
    }
}

async fn handle_get_config(State(state): State<AppState>) -> Json<SecureConfigResponse> {
    Json(state.config_service.get_secure_config())
}

async fn handle_update_rate_limit(
    State(state): State<AppState>,
    Extension(locale): Extension<Locale>,
    Json(req): Json<UpdateRateLimitRequest>,
) -> Result<Json<UpdateConfigResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Phase 8 T041 (MEDIUM M-2 fix) — invoke `validate_request` so
    // `#[validate(range(min = 1, max = 1000000))]` etc. on
    // `UpdateRateLimitRequest` are actually enforced at the handler
    // boundary, rather than silently accepted by the config service.
    validate_request(&req, locale)?;
    Ok(Json(state.config_service.update_rate_limit(req).await))
}

async fn handle_update_logging(
    State(state): State<AppState>,
    Extension(locale): Extension<Locale>,
    Json(req): Json<UpdateLoggingRequest>,
) -> Result<Json<UpdateConfigResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Phase 8 T041 (MEDIUM M-2 fix) — enforce `#[validate(length(min = 1, max = 20))]`
    // on `UpdateLoggingRequest::level` before forwarding to the service.
    validate_request(&req, locale)?;
    Ok(Json(state.config_service.update_logging(req).await))
}

#[axum::debug_handler]
async fn handle_reload_config(State(state): State<AppState>) -> Json<UpdateConfigResponse> {
    Json(state.config_service.reload_config().await)
}

async fn handle_set_algorithm(
    State(state): State<AppState>,
    Extension(locale): Extension<Locale>,
    Json(req): Json<SetAlgorithmRequest>,
) -> Result<Json<SetAlgorithmResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Phase 8 T041 (MEDIUM M-2 fix) — enforce `#[validate(length(min = 1, max = 64))]`
    // on `biz_tag` and `#[validate(length(min = 1, max = 20))]` on `algorithm`.
    validate_request(&req, locale)?;
    Ok(Json(state.config_service.set_algorithm(req).await))
}

async fn handle_api_info() -> Json<ApiInfoResponse> {
    Json(ApiInfoResponse {
        name: "Nebula ID Service".to_string(),
        version: "1.0.0".to_string(),
        description: "Distributed ID Generation Service".to_string(),
        endpoints: vec![
            "GET /health - Health check".to_string(),
            "GET /ready - Readiness probe".to_string(),
            "GET /metrics - Prometheus metrics".to_string(),
            "GET /api/v1 - API information".to_string(),
            "POST /api/v1/generate - Generate ID".to_string(),
            "POST /api/v1/generate/batch - Batch generate IDs".to_string(),
            "POST /api/v1/parse - Parse ID".to_string(),
            "GET /api/v1/config - Get configuration".to_string(),
            "POST /api/v1/config/rate-limit - Update rate limit".to_string(),
            "POST /api/v1/config/logging - Update logging".to_string(),
            "POST /api/v1/config/reload - Reload configuration".to_string(),
            "POST /api/v1/config/algorithm - Set algorithm".to_string(),
            "POST /api/v1/biz-tags - Create biz tag".to_string(),
            "GET /api/v1/biz-tags - List biz tags".to_string(),
            "GET /api/v1/biz-tags/:id - Get biz tag".to_string(),
            "PUT /api/v1/biz-tags/:id - Update biz tag".to_string(),
            "DELETE /api/v1/biz-tags/:id - Delete biz tag".to_string(),
        ],
    })
}

// ========== BizTag Handlers ==========

async fn handle_create_biz_tag(
    State(state): State<AppState>,
    extensions: axum::Extension<Option<uuid::Uuid>>,
    extensions_role: axum::Extension<crate::server::middleware::ApiKeyRole>,
    Extension(locale): Extension<Locale>,
    Json(req): Json<CreateBizTagRequest>,
) -> Result<Json<BizTagResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Validate request parameters
    validate_request(&req, locale)?;

    // Only User API Key can create biz_tags
    verify_user_role(extensions_role.0, locale)?;

    // Verify workspace_id match for User API Key
    verify_workspace_id(req.workspace_id, &extensions.0, locale)?;

    state
        .handlers
        .create_biz_tag(req)
        .await
        .map(Json)
        .map_err(|e| core_error_to_response(&e, locale))
}

async fn handle_get_biz_tag(
    State(state): State<AppState>,
    Extension(locale): Extension<Locale>,
    Path(id): Path<String>,
) -> Result<Json<BizTagResponse>, (StatusCode, Json<ErrorResponse>)> {
    let uuid = uuid::Uuid::parse_str(&id).map_err(|_| invalid_uuid_response(locale))?;

    state
        .handlers
        .get_biz_tag(uuid)
        .await
        .map(Json)
        .map_err(|e| core_error_to_response(&e, locale))
}

async fn handle_update_biz_tag(
    State(state): State<AppState>,
    Extension(locale): Extension<Locale>,
    Path(id): Path<String>,
    Json(req): Json<UpdateBizTagRequest>,
) -> Result<Json<BizTagResponse>, (StatusCode, Json<ErrorResponse>)> {
    let uuid = uuid::Uuid::parse_str(&id).map_err(|_| invalid_uuid_response(locale))?;

    if let Err(validation_errors) = req.validate() {
        return Err(validation_error_response(&validation_errors, locale));
    }

    state
        .handlers
        .update_biz_tag(uuid, req)
        .await
        .map(Json)
        .map_err(|e| core_error_to_response(&e, locale))
}

async fn handle_delete_biz_tag(
    State(state): State<AppState>,
    Extension(locale): Extension<Locale>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let uuid = uuid::Uuid::parse_str(&id).map_err(|_| invalid_uuid_response(locale))?;

    state
        .handlers
        .delete_biz_tag(uuid)
        .await
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(|e| core_error_to_response(&e, locale))
}

async fn handle_list_biz_tags(
    State(state): State<AppState>,
    Extension(locale): Extension<Locale>,
    Query(params): Query<PaginationParams>,
) -> Result<Json<BizTagListResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Phase 8 T041 (MEDIUM M-3/M-4/MED-003 fix) — surface errors
    // via `core_error_to_response` instead of silently returning an
    // empty list. The `Extension<Locale>` parameter ensures the
    // locale-translated message is used.
    validate_request(&params, locale)?;

    let page = params.page.max(1);
    let page_size = params.page_size.clamp(1, 100);
    let offset = (page - 1) * page_size;

    let response = state
        .handlers
        .list_biz_tags_with_pagination(None, None, page_size as usize, offset as usize)
        .await
        .map_err(|e| core_error_to_response(&e, locale))?;

    Ok(Json(BizTagListResponse {
        biz_tags: response.biz_tags,
        total: response.total,
        page,
        page_size,
    }))
}

// ========== Workspace Handlers ==========

async fn handle_create_workspace(
    State(state): State<AppState>,
    Extension(locale): Extension<Locale>,
    Json(req): Json<CreateWorkspaceRequest>,
) -> Result<Json<WorkspaceResponse>, (StatusCode, Json<ErrorResponse>)> {
    if let Err(validation_errors) = req.validate() {
        return Err(validation_error_response(&validation_errors, locale));
    }

    state
        .handlers
        .create_workspace(req)
        .await
        .map(Json)
        .map_err(|e| core_error_to_response(&e, locale))
}

async fn handle_list_workspaces(
    State(state): State<AppState>,
    Extension(locale): Extension<Locale>,
) -> Result<Json<WorkspaceListResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Phase 8 T041 (MEDIUM M-3/M-4/MED-003 fix) — surface errors via
    // `core_error_to_response` instead of silently returning an empty
    // list, so 5xx internal errors are logged server-side and the
    // client sees a generic locale-translated message.
    let response = state
        .handlers
        .list_workspaces()
        .await
        .map_err(|e| core_error_to_response(&e, locale))?;
    Ok(Json(response))
}

async fn handle_get_workspace(
    State(state): State<AppState>,
    Extension(locale): Extension<Locale>,
    Path(name): Path<String>,
) -> Result<Json<WorkspaceResponse>, (StatusCode, Json<ErrorResponse>)> {
    match state.handlers.get_workspace(&name).await {
        Ok(Some(ws)) => Ok(Json(ws)),
        Ok(None) => Err(workspace_not_found_response(locale)),
        Err(e) => Err(core_error_to_response(&e, locale)),
    }
}

// ========== Group Handlers ==========

async fn handle_create_group(
    State(state): State<AppState>,
    extensions: axum::Extension<Option<uuid::Uuid>>,
    extensions_role: axum::Extension<crate::server::middleware::ApiKeyRole>,
    Extension(locale): Extension<Locale>,
    Json(req): Json<CreateGroupRequest>,
) -> Result<Json<GroupResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Validate request parameters
    validate_request(&req, locale)?;

    // Only User API Key can create groups
    verify_user_role(extensions_role.0, locale)?;

    // Verify workspace_id match for User API Key
    verify_user_workspace(&req.workspace, &extensions.0, &state.handlers, locale).await?;

    state
        .handlers
        .create_group(req)
        .await
        .map(Json)
        .map_err(|e| core_error_to_response(&e, locale))
}

async fn handle_regenerate_user_key(
    State(state): State<AppState>,
    Extension(locale): Extension<Locale>,
    Path(name): Path<String>,
) -> Result<Json<ApiKeyWithSecretResponse>, (StatusCode, Json<ErrorResponse>)> {
    state
        .handlers
        .regenerate_user_api_key(&name)
        .await
        .map(Json)
        .map_err(|e: crate::core::CoreError| core_error_to_response(&e, locale))
}

async fn handle_list_groups(
    State(state): State<AppState>,
    Extension(locale): Extension<Locale>,
    Query(params): Query<GroupListParams>,
) -> Result<Json<GroupListResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Phase 8 T041 (MEDIUM M-3/M-4/MED-003 fix) — surface errors via
    // `core_error_to_response` and enforce `#[validate(range(min = 1, max = 100))]`
    // on `page_size`. The `Extension<Locale>` parameter ensures the
    // locale-translated message is used.
    validate_request(&params, locale)?;

    let workspace = params.workspace.clone();

    let response = state
        .handlers
        .list_groups(&workspace)
        .await
        .map_err(|e| core_error_to_response(&e, locale))?;
    Ok(Json(response))
}

// ========== API Key Handlers ==========

async fn handle_create_api_key(
    State(state): State<AppState>,
    Extension(locale): Extension<Locale>,
    Json(req): Json<CreateApiKeyRequest>,
) -> Result<Json<ApiKeyWithSecretResponse>, (StatusCode, Json<ErrorResponse>)> {
    validate_request(&req, locale)?;

    // Phase 8 T041 (MEDIUM M-5 fix) — distinguish `workspace_id`
    // *missing* (return `workspace_id_required_response`) from
    // `workspace_id` *malformed* (return `invalid_uuid_response`).
    // Previously a malformed UUID silently returned "workspace_id is
    // required", misleading the client about which field was at
    // fault. The closure below encodes the explicit two-error path.
    let parse_user_workspace_id =
        |raw: &Option<String>| -> Result<uuid::Uuid, (StatusCode, Json<ErrorResponse>)> {
            match raw.as_deref() {
                Some(s) => uuid::Uuid::parse_str(s).map_err(|_| invalid_uuid_response(locale)),
                None => Err(workspace_id_required_response(locale)),
            }
        };

    // Determine workspace_id based on role
    let workspace_id = if let Some(ref role_str) = req.role {
        if role_str == "admin" {
            // Admin keys are global, not bound to any workspace
            None
        } else {
            // User keys must be bound to a workspace — distinguish
            // missing vs malformed (M-5 fix).
            Some(parse_user_workspace_id(&req.workspace_id)?)
        }
    } else {
        // Default to user role if not specified
        Some(parse_user_workspace_id(&req.workspace_id)?)
    };

    state
        .handlers
        .create_api_key(workspace_id, req)
        .await
        .map(Json)
        .map_err(|e: crate::core::types::CoreError| core_error_to_response(&e, locale))
}

async fn handle_list_api_keys(
    State(state): State<AppState>,
    Extension(locale): Extension<Locale>,
    Query(params): Query<PaginationParams>,
) -> Result<Json<ApiKeyListResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Phase 8 T041 (MEDIUM M-3/M-4/MED-003 fix) — surface errors via
    // `core_error_to_response` and enforce `#[validate(range(min = 1, max = 100))]`
    // on `page_size`. The `Extension<Locale>` parameter ensures the
    // locale-translated message is used.
    validate_request(&params, locale)?;

    let page = params.page.max(1);
    let page_size = params.page_size.clamp(1, 100);
    let offset = (page - 1) * page_size;

    // Get workspace_id from query parameters. LOW L-3 fix — when
    // `workspace_id` is provided but is not a valid UUID, log a
    // server-side warning and fall back to `Uuid::nil()` (admin key
    // semantics: list across all workspaces). Returning an error here
    // would break admin-key listing flows that rely on the nil UUID.
    let workspace_id = match params.workspace_id.as_deref() {
        Some(s) => match uuid::Uuid::parse_str(s) {
            Ok(id) => id,
            Err(_) => {
                tracing::warn!(
                    event = "list_api_keys_invalid_workspace_id",
                    raw = %s,
                    "invalid workspace_id query parameter; falling back to nil UUID"
                );
                uuid::Uuid::nil()
            }
        },
        None => uuid::Uuid::nil(),
    };

    let response = state
        .handlers
        .list_api_keys(workspace_id, Some(page_size as u32), Some(offset as u32))
        .await
        .map_err(|e| core_error_to_response(&e, locale))?;
    Ok(Json(response))
}

async fn handle_revoke_api_key(
    State(state): State<AppState>,
    Extension(locale): Extension<Locale>,
    Path(id): Path<String>,
) -> Result<Json<RevokeApiKeyResponse>, (StatusCode, Json<ErrorResponse>)> {
    let uuid = uuid::Uuid::parse_str(&id).map_err(|_| invalid_uuid_response(locale))?;

    state
        .handlers
        .revoke_api_key(uuid)
        .await
        .map(Json)
        .map_err(|e: crate::core::types::CoreError| core_error_to_response(&e, locale))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::algorithm::AlgorithmRouter;
    use crate::core::config::Config;
    use crate::server::config::management::ConfigManager;
    use crate::server::config::HotReloadConfig;
    use std::sync::Arc;

    fn create_test_api_handlers() -> Arc<ApiHandlers> {
        let config = Config::default();
        let hot_config = Arc::new(HotReloadConfig::new(
            config.clone(),
            "config/config.toml".to_string(),
        ));
        let algorithm_router = Arc::new(AlgorithmRouter::new(config.clone(), None));
        let config_service = Arc::new(ConfigManager::new(hot_config, algorithm_router.clone()));
        let handlers = ApiHandlers::new(algorithm_router, config_service);
        Arc::new(handlers)
    }

    fn create_test_auth() -> Arc<ApiKeyAuth> {
        use crate::core::database::{
            ApiKeyInfo, ApiKeyRepository, ApiKeyResponse, ApiKeyRole as CoreApiKeyRole,
            ApiKeyWithSecret, CreateApiKeyRequest,
        };
        use crate::core::types::Result;
        use async_trait::async_trait;
        use uuid::Uuid;

        #[derive(Clone)]
        struct MockApiKeyRepo;

        #[async_trait]
        impl ApiKeyRepository for MockApiKeyRepo {
            async fn create_api_key(
                &self,
                _request: &CreateApiKeyRequest,
            ) -> Result<ApiKeyWithSecret> {
                Ok(ApiKeyWithSecret {
                    key: ApiKeyResponse {
                        id: Uuid::new_v4(),
                        key_id: "mock_key_id".to_string(),
                        key_prefix: "nino_".to_string(),
                        name: "Mock Key".to_string(),
                        description: None,
                        role: CoreApiKeyRole::User,
                        rate_limit: 10000,
                        enabled: true,
                        expires_at: None,
                        created_at: chrono::Utc::now().naive_utc(),
                    },
                    key_secret: "mock_secret".to_string(),
                })
            }

            async fn get_api_key_by_id(&self, _key_id: &str) -> Result<Option<ApiKeyInfo>> {
                Ok(None)
            }

            async fn validate_api_key(
                &self,
                _key_id: &str,
                _key_secret: &str,
            ) -> Result<Option<(Option<Uuid>, crate::core::database::ApiKeyRole)>> {
                Ok(None)
            }

            async fn list_api_keys(
                &self,
                _workspace_id: Uuid,
                _limit: Option<u32>,
                _offset: Option<u32>,
            ) -> Result<Vec<ApiKeyInfo>> {
                Ok(vec![])
            }

            async fn delete_api_key(&self, _id: Uuid) -> Result<()> {
                Ok(())
            }

            async fn revoke_api_key(&self, _id: Uuid) -> Result<()> {
                Ok(())
            }

            async fn update_last_used(&self, _key: Uuid) -> Result<()> {
                Ok(())
            }

            async fn get_admin_api_key(&self, _workspace_id: Uuid) -> Result<Option<ApiKeyInfo>> {
                Ok(None)
            }

            async fn count_api_keys(&self, _workspace_id: Uuid) -> Result<u64> {
                Ok(0)
            }

            async fn rotate_api_key(
                &self,
                _key_id: &str,
                _grace_period_seconds: u64,
            ) -> Result<ApiKeyWithSecret> {
                Err(crate::core::types::error::CoreError::InternalError(
                    "rotate_api_key not implemented in mock".to_string(),
                ))
            }

            async fn get_keys_older_than(
                &self,
                _age_threshold_days: i64,
            ) -> Result<Vec<ApiKeyInfo>> {
                Ok(vec![])
            }
        }

        Arc::new(ApiKeyAuth::new(Arc::new(MockApiKeyRepo), true))
    }

    fn create_test_rate_limiter() -> Arc<RateLimiter> {
        Arc::new(RateLimiter::new(10000, 100))
    }

    fn create_test_audit_logger() -> Arc<AuditLogger> {
        Arc::new(AuditLogger::new(10000))
    }

    #[tokio::test]
    async fn test_router_creation() {
        let handlers = create_test_api_handlers();
        let auth = create_test_auth();
        let rate_limiter = create_test_rate_limiter();
        let audit_logger = create_test_audit_logger();

        let router = create_router(handlers, auth, rate_limiter, audit_logger).await;
        let _router = router;
    }

    #[tokio::test]
    async fn test_router_endpoints() {
        let handlers = create_test_api_handlers();
        let auth = create_test_auth();
        let rate_limiter = create_test_rate_limiter();
        let audit_logger = create_test_audit_logger();

        let router = create_router(handlers, auth, rate_limiter, audit_logger).await;
        let _router = router;
    }
}
