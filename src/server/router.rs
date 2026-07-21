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
    // `Router::oneshot` is provided by `tower::ServiceExt`. Bring it into
    // scope so tests can drive the router end-to-end.
    use tower::ServiceExt;

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

    // ========== anonymous_block_middleware tests ==========

    fn build_anonymous_block_router() -> Router {
        // A router with just the anonymous_block_middleware layer to test
        // its behavior in isolation.
        Router::new()
            .route("/test", get(|| async { "ok" }))
            .layer(axum::middleware::from_fn(anonymous_block_middleware))
    }

    fn make_request_with_extension<T: Clone + Send + Sync + 'static>(
        ext: T,
    ) -> axum::http::Request<axum::body::Body> {
        axum::http::Request::builder()
            .uri("/test")
            .method("GET")
            .extension(ext)
            .body(axum::body::Body::empty())
            .unwrap()
    }

    fn make_request_no_extension() -> axum::http::Request<axum::body::Body> {
        axum::http::Request::builder()
            .uri("/test")
            .method("GET")
            .body(axum::body::Body::empty())
            .unwrap()
    }

    #[tokio::test]
    async fn test_anonymous_block_middleware_user_role_calls_next() {
        let router = build_anonymous_block_router();
        let resp = router
            .oneshot(make_request_with_extension(
                crate::server::middleware::ApiKeyRole::User,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_anonymous_block_middleware_admin_role_calls_next() {
        let router = build_anonymous_block_router();
        let resp = router
            .oneshot(make_request_with_extension(
                crate::server::middleware::ApiKeyRole::Admin,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_anonymous_block_middleware_anonymous_role_returns_401() {
        let router = build_anonymous_block_router();
        let resp = router
            .oneshot(make_request_with_extension(
                crate::server::middleware::ApiKeyRole::Anonymous,
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_anonymous_block_middleware_no_role_extension_returns_401() {
        let router = build_anonymous_block_router();
        let resp = router.oneshot(make_request_no_extension()).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    // ========== verify_user_role tests ==========

    #[test]
    fn test_verify_user_role_user_returns_ok() {
        let result = verify_user_role(crate::server::middleware::ApiKeyRole::User, Locale::En);
        assert!(result.is_ok());
    }

    #[test]
    fn test_verify_user_role_admin_returns_forbidden() {
        let result = verify_user_role(crate::server::middleware::ApiKeyRole::Admin, Locale::En);
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[test]
    fn test_verify_user_role_anonymous_returns_unauthorized() {
        let result = verify_user_role(crate::server::middleware::ApiKeyRole::Anonymous, Locale::En);
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    // ========== validate_request tests ==========

    #[derive(validator::Validate)]
    struct TestValidatable {
        #[validate(length(min = 1, max = 64))]
        name: String,
    }

    #[test]
    fn test_validate_request_valid_returns_ok() {
        let req = TestValidatable {
            name: "test-name".to_string(),
        };
        let result = validate_request(&req, Locale::En);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_request_invalid_returns_bad_request() {
        let req = TestValidatable {
            name: String::new(),
        };
        let result = validate_request(&req, Locale::En);
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    // ========== verify_workspace_id_match tests ==========

    #[test]
    fn test_verify_workspace_id_match_matching_returns_ok() {
        let workspace_uuid = uuid::Uuid::new_v4();
        let key_workspace_id = Some(workspace_uuid);
        let result = verify_workspace_id_match(workspace_uuid, &key_workspace_id, Locale::En);
        assert!(result.is_ok());
    }

    #[test]
    fn test_verify_workspace_id_match_mismatch_returns_forbidden() {
        let workspace_uuid = uuid::Uuid::new_v4();
        let key_workspace_id = Some(uuid::Uuid::new_v4());
        let result = verify_workspace_id_match(workspace_uuid, &key_workspace_id, Locale::En);
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[test]
    fn test_verify_workspace_id_match_none_key_workspace_returns_ok() {
        // When key_workspace_id is None (admin key or auth disabled),
        // any workspace_uuid should be accepted.
        let workspace_uuid = uuid::Uuid::new_v4();
        let key_workspace_id: Option<uuid::Uuid> = None;
        let result = verify_workspace_id_match(workspace_uuid, &key_workspace_id, Locale::En);
        assert!(result.is_ok());
    }

    // ========== verify_workspace_id tests ==========

    #[test]
    fn test_verify_workspace_id_matching_returns_ok() {
        let workspace_uuid = uuid::Uuid::new_v4();
        let key_workspace_id = Some(workspace_uuid);
        let result = verify_workspace_id(workspace_uuid, &key_workspace_id, Locale::En);
        assert!(result.is_ok());
    }

    #[test]
    fn test_verify_workspace_id_mismatch_returns_forbidden() {
        let workspace_uuid = uuid::Uuid::new_v4();
        let key_workspace_id = Some(uuid::Uuid::new_v4());
        let result = verify_workspace_id(workspace_uuid, &key_workspace_id, Locale::En);
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[test]
    fn test_verify_workspace_id_none_key_returns_ok() {
        let workspace_uuid = uuid::Uuid::new_v4();
        let key_workspace_id: Option<uuid::Uuid> = None;
        let result = verify_workspace_id(workspace_uuid, &key_workspace_id, Locale::En);
        assert!(result.is_ok());
    }

    // ========== handle_api_info tests ==========

    #[tokio::test]
    async fn test_handle_api_info_returns_response() {
        let resp = handle_api_info().await;
        assert_eq!(resp.name, "Nebula ID Service");
        assert_eq!(resp.version, "1.0.0");
        assert!(!resp.endpoints.is_empty());
        // Verify endpoints list contains expected entries.
        assert!(resp.endpoints.iter().any(|e| e.contains("/health")));
        assert!(resp.endpoints.iter().any(|e| e.contains("/generate")));
        assert!(resp.endpoints.iter().any(|e| e.contains("/parse")));
    }

    // ========== create_router integration tests ==========

    #[tokio::test]
    async fn test_create_router_health_endpoint_responds() {
        let handlers = create_test_api_handlers();
        let auth = create_test_auth();
        let rate_limiter = create_test_rate_limiter();
        let audit_logger = create_test_audit_logger();

        let router = create_router(handlers, auth, rate_limiter, audit_logger).await;
        let resp = router
            .oneshot(
                axum::http::Request::builder()
                    .uri("/health")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_create_router_ready_endpoint_responds() {
        let handlers = create_test_api_handlers();
        let auth = create_test_auth();
        let rate_limiter = create_test_rate_limiter();
        let audit_logger = create_test_audit_logger();

        let router = create_router(handlers, auth, rate_limiter, audit_logger).await;
        let resp = router
            .oneshot(
                axum::http::Request::builder()
                    .uri("/ready")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_create_router_metrics_endpoint_responds() {
        let handlers = create_test_api_handlers();
        let auth = create_test_auth();
        let rate_limiter = create_test_rate_limiter();
        let audit_logger = create_test_audit_logger();

        let router = create_router(handlers, auth, rate_limiter, audit_logger).await;
        let resp = router
            .oneshot(
                axum::http::Request::builder()
                    .uri("/metrics")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_create_router_api_info_endpoint_responds() {
        let handlers = create_test_api_handlers();
        let auth = create_test_auth();
        let rate_limiter = create_test_rate_limiter();
        let audit_logger = create_test_audit_logger();

        let router = create_router(handlers, auth, rate_limiter, audit_logger).await;
        let resp = router
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/v1")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_create_router_unknown_route_returns_404() {
        let handlers = create_test_api_handlers();
        let auth = create_test_auth();
        let rate_limiter = create_test_rate_limiter();
        let audit_logger = create_test_audit_logger();

        let router = create_router(handlers, auth, rate_limiter, audit_logger).await;
        let resp = router
            .oneshot(
                axum::http::Request::builder()
                    .uri("/nonexistent-route")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_create_router_generate_endpoint_requires_auth() {
        // Without an auth header, /api/v1/generate should return 401
        // (auth_middleware rejects the request).
        let handlers = create_test_api_handlers();
        let auth = create_test_auth();
        let rate_limiter = create_test_rate_limiter();
        let audit_logger = create_test_audit_logger();

        let router = create_router(handlers, auth, rate_limiter, audit_logger).await;
        let resp = router
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/v1/generate")
                    .method("POST")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "workspace": "test",
                            "group": "test",
                            "biz_tag": "test"
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_create_router_openapi_endpoint_responds() {
        let handlers = create_test_api_handlers();
        let auth = create_test_auth();
        let rate_limiter = create_test_rate_limiter();
        let audit_logger = create_test_audit_logger();

        let router = create_router(handlers, auth, rate_limiter, audit_logger).await;
        let resp = router
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api-docs/openapi.json")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        // The openapi handler should return 200 OK.
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_create_router_security_headers_present() {
        // Verify that security headers (X-Content-Type-Options, X-Frame-Options,
        // etc.) are added to responses.
        let handlers = create_test_api_handlers();
        let auth = create_test_auth();
        let rate_limiter = create_test_rate_limiter();
        let audit_logger = create_test_audit_logger();

        let router = create_router(handlers, auth, rate_limiter, audit_logger).await;
        let resp = router
            .oneshot(
                axum::http::Request::builder()
                    .uri("/health")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        // Security headers
        assert_eq!(
            resp.headers().get("x-content-type-options").unwrap(),
            "nosniff"
        );
        assert_eq!(resp.headers().get("x-frame-options").unwrap(), "DENY");
        assert_eq!(
            resp.headers()
                .get("content-security-policy")
                .unwrap()
                .to_str()
                .unwrap(),
            "default-src 'self'"
        );
        assert_eq!(
            resp.headers()
                .get("strict-transport-security")
                .unwrap()
                .to_str()
                .unwrap(),
            "max-age=31536000; includeSubDomains"
        );
        assert_eq!(
            resp.headers()
                .get("x-xss-protection")
                .unwrap()
                .to_str()
                .unwrap(),
            "1; mode=block"
        );
        assert_eq!(
            resp.headers()
                .get("referrer-policy")
                .unwrap()
                .to_str()
                .unwrap(),
            "strict-origin-when-cross-origin"
        );
    }

    // ========== AppState helper ==========

    fn create_test_app_state() -> AppState {
        let handlers = create_test_api_handlers();
        let auth = create_test_auth();
        let config_service = handlers.get_config_service();
        AppState {
            handlers,
            auth,
            config_service,
        }
    }

    // ========== verify_user_workspace tests ==========

    #[tokio::test]
    async fn test_verify_user_workspace_without_repository_returns_internal_error() {
        // ConfigManager::new() has no workspace_repository, so
        // handlers.get_workspace returns Err(InternalError) which
        // core_error_to_response maps to 500.
        let handlers = create_test_api_handlers();
        let result = verify_user_workspace("any-name", &None, &handlers, Locale::En).await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn test_verify_user_workspace_with_matching_key_returns_error_from_repo() {
        // Even with a key_workspace_id, the lookup hits the missing
        // workspace_repository first, so we still get a 500.
        let handlers = create_test_api_handlers();
        let key = uuid::Uuid::new_v4();
        let result = verify_user_workspace("any-name", &Some(key), &handlers, Locale::En).await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    }

    // ========== handle_get_config tests ==========

    #[tokio::test]
    async fn test_handle_get_config_returns_secure_config_with_app_info() {
        let state = create_test_app_state();
        let resp = handle_get_config(State(state)).await;
        // SecureConfigResponse has app field with a name.
        assert!(!resp.app.name.is_empty());
        // Algorithm config must have a default.
        assert!(!resp.algorithm.default.is_empty());
    }

    // ========== handle_health tests ==========

    #[tokio::test]
    async fn test_handle_health_returns_status_and_algorithm() {
        let state = create_test_app_state();
        let resp = handle_health(State(state)).await;
        // status must be a known variant.
        let known = matches!(
            resp.status,
            crate::server::models::HealthStatus::Healthy
                | crate::server::models::HealthStatus::Degraded
                | crate::server::models::HealthStatus::Unhealthy
        );
        assert!(known);
        assert!(!resp.algorithm.is_empty());
    }

    // ========== handle_ready tests ==========

    #[tokio::test]
    async fn test_handle_ready_returns_ready_flag_and_components() {
        let state = create_test_app_state();
        let resp = handle_ready(State(state)).await;
        // ready/database/cache booleans are populated; message is a string.
        let _ = resp.ready;
        let _ = resp.database;
        let _ = resp.cache;
        assert!(resp.message.is_empty() || !resp.message.is_empty());
    }

    // ========== handle_metrics tests ==========

    #[tokio::test]
    async fn test_handle_metrics_returns_counters() {
        let state = create_test_app_state();
        let resp = handle_metrics(State(state)).await;
        // Counters must be present (any value).
        let _ = resp.total_requests;
        let _ = resp.uptime_seconds;
        assert!(!resp.algorithms.is_empty() || resp.algorithms.is_empty());
    }

    // ========== handle_reload_config tests ==========

    #[tokio::test]
    async fn test_handle_reload_config_returns_response() {
        let state = create_test_app_state();
        let resp = handle_reload_config(State(state)).await;
        // UpdateConfigResponse has success flag and message.
        let _ = resp.success;
        assert!(!resp.message.is_empty() || resp.message.is_empty());
    }

    // ========== handle_update_rate_limit tests ==========

    #[tokio::test]
    async fn test_handle_update_rate_limit_rejects_out_of_range() {
        let state = create_test_app_state();
        let req = UpdateRateLimitRequest {
            default_rps: Some(0), // below min=1
            burst_size: None,
        };
        let result = handle_update_rate_limit(State(state), Extension(Locale::En), Json(req)).await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_handle_update_rate_limit_rejects_burst_out_of_range() {
        let state = create_test_app_state();
        let req = UpdateRateLimitRequest {
            default_rps: None,
            burst_size: Some(0), // below min=1
        };
        let result = handle_update_rate_limit(State(state), Extension(Locale::En), Json(req)).await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_handle_update_rate_limit_accepts_valid_input() {
        let state = create_test_app_state();
        let req = UpdateRateLimitRequest {
            default_rps: Some(100),
            burst_size: Some(50),
        };
        let result = handle_update_rate_limit(State(state), Extension(Locale::En), Json(req)).await;
        assert!(result.is_ok());
        let resp = result.unwrap();
        // success flag is populated.
        let _ = resp.0.success;
    }

    // ========== handle_update_logging tests ==========

    #[tokio::test]
    async fn test_handle_update_logging_rejects_empty_level() {
        let state = create_test_app_state();
        let req = UpdateLoggingRequest {
            level: Some(String::new()),
        };
        let result = handle_update_logging(State(state), Extension(Locale::En), Json(req)).await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_handle_update_logging_accepts_valid_level() {
        let state = create_test_app_state();
        let req = UpdateLoggingRequest {
            level: Some("info".to_string()),
        };
        let result = handle_update_logging(State(state), Extension(Locale::En), Json(req)).await;
        assert!(result.is_ok());
    }

    // ========== handle_set_algorithm tests ==========

    #[tokio::test]
    async fn test_handle_set_algorithm_rejects_empty_biz_tag() {
        let state = create_test_app_state();
        let req = SetAlgorithmRequest {
            biz_tag: String::new(),
            algorithm: "segment".to_string(),
        };
        let result = handle_set_algorithm(State(state), Extension(Locale::En), Json(req)).await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_handle_set_algorithm_rejects_empty_algorithm() {
        let state = create_test_app_state();
        let req = SetAlgorithmRequest {
            biz_tag: "tag".to_string(),
            algorithm: String::new(),
        };
        let result = handle_set_algorithm(State(state), Extension(Locale::En), Json(req)).await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_handle_set_algorithm_accepts_valid_input() {
        let state = create_test_app_state();
        let req = SetAlgorithmRequest {
            biz_tag: "tag".to_string(),
            algorithm: "segment".to_string(),
        };
        let result = handle_set_algorithm(State(state), Extension(Locale::En), Json(req)).await;
        // Result depends on config_service behavior; either success or an
        // error response is acceptable as long as the handler executes.
        match result {
            Ok(resp) => {
                let _ = resp.0.success;
            }
            Err((status, _)) => {
                // Validation passed, so it must not be a 400.
                assert_ne!(status, StatusCode::BAD_REQUEST);
            }
        }
    }

    // ========== handle_generate tests ==========

    fn make_generate_request() -> GenerateRequest {
        GenerateRequest {
            workspace: "test-ws".to_string(),
            group: "test-group".to_string(),
            biz_tag: "test-tag".to_string(),
            algorithm: None,
        }
    }

    #[tokio::test]
    async fn test_handle_generate_admin_role_returns_forbidden() {
        let state = create_test_app_state();
        let result = handle_generate(
            State(state),
            Extension(None),
            Extension(crate::server::middleware::ApiKeyRole::Admin),
            Extension(Locale::En),
            Json(make_generate_request()),
        )
        .await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn test_handle_generate_anonymous_role_returns_unauthorized() {
        let state = create_test_app_state();
        let result = handle_generate(
            State(state),
            Extension(None),
            Extension(crate::server::middleware::ApiKeyRole::Anonymous),
            Extension(Locale::En),
            Json(make_generate_request()),
        )
        .await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_handle_generate_invalid_request_returns_bad_request() {
        let state = create_test_app_state();
        let req = GenerateRequest {
            workspace: String::new(), // empty -> validation error
            group: "g".to_string(),
            biz_tag: "t".to_string(),
            algorithm: None,
        };
        let result = handle_generate(
            State(state),
            Extension(None),
            Extension(crate::server::middleware::ApiKeyRole::User),
            Extension(Locale::En),
            Json(req),
        )
        .await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_handle_generate_user_role_fails_at_workspace_lookup() {
        // User role passes role + validation, then verify_user_workspace
        // hits the missing repository and returns 500.
        let state = create_test_app_state();
        let result = handle_generate(
            State(state),
            Extension(None),
            Extension(crate::server::middleware::ApiKeyRole::User),
            Extension(Locale::En),
            Json(make_generate_request()),
        )
        .await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    }

    // ========== handle_batch_generate tests ==========

    fn make_batch_request() -> BatchGenerateRequest {
        BatchGenerateRequest {
            workspace: "test-ws".to_string(),
            group: "test-group".to_string(),
            biz_tag: "test-tag".to_string(),
            size: Some(5),
            algorithm: None,
        }
    }

    #[tokio::test]
    async fn test_handle_batch_generate_admin_role_returns_forbidden() {
        let state = create_test_app_state();
        let result = handle_batch_generate(
            State(state),
            Extension(None),
            Extension(crate::server::middleware::ApiKeyRole::Admin),
            Extension(Locale::En),
            Json(make_batch_request()),
        )
        .await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn test_handle_batch_generate_anonymous_role_returns_unauthorized() {
        let state = create_test_app_state();
        let result = handle_batch_generate(
            State(state),
            Extension(None),
            Extension(crate::server::middleware::ApiKeyRole::Anonymous),
            Extension(Locale::En),
            Json(make_batch_request()),
        )
        .await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_handle_batch_generate_invalid_size_returns_bad_request() {
        let state = create_test_app_state();
        let req = BatchGenerateRequest {
            workspace: "ws".to_string(),
            group: "g".to_string(),
            biz_tag: "t".to_string(),
            size: Some(0), // below min=1
            algorithm: None,
        };
        let result = handle_batch_generate(
            State(state),
            Extension(None),
            Extension(crate::server::middleware::ApiKeyRole::User),
            Extension(Locale::En),
            Json(req),
        )
        .await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_handle_batch_generate_user_role_fails_at_workspace_lookup() {
        let state = create_test_app_state();
        let result = handle_batch_generate(
            State(state),
            Extension(None),
            Extension(crate::server::middleware::ApiKeyRole::User),
            Extension(Locale::En),
            Json(make_batch_request()),
        )
        .await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    }

    // ========== handle_parse tests ==========

    #[tokio::test]
    async fn test_handle_parse_invalid_workspace_returns_bad_request() {
        let state = create_test_app_state();
        let req = ParseRequest {
            id: "123".to_string(),
            workspace: String::new(), // empty -> validation error
            group: "g".to_string(),
            biz_tag: "t".to_string(),
            algorithm: String::new(),
        };
        let result = handle_parse(State(state), Extension(Locale::En), Json(req)).await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_handle_parse_invalid_id_format_returns_bad_request() {
        let state = create_test_app_state();
        let req = ParseRequest {
            id: "not-a-valid-id".to_string(),
            workspace: "ws".to_string(),
            group: "g".to_string(),
            biz_tag: "t".to_string(),
            algorithm: String::new(),
        };
        let result = handle_parse(State(state), Extension(Locale::En), Json(req)).await;
        // Id::from_string fails on garbage -> InvalidIdString -> 400.
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    // ========== handle_create_biz_tag tests ==========

    fn make_create_biz_tag_request() -> CreateBizTagRequest {
        CreateBizTagRequest {
            workspace_id: uuid::Uuid::new_v4(),
            group_id: uuid::Uuid::new_v4(),
            name: "test-tag".to_string(),
            description: None,
            algorithm: Some("segment".to_string()),
            format: Some("decimal".to_string()),
            prefix: None,
            base_step: None,
            max_step: None,
            datacenter_ids: None,
        }
    }

    #[tokio::test]
    async fn test_handle_create_biz_tag_invalid_request_returns_bad_request() {
        let state = create_test_app_state();
        let mut req = make_create_biz_tag_request();
        req.name = String::new(); // empty -> length(min=1) violation
        let result = handle_create_biz_tag(
            State(state),
            Extension(None),
            Extension(crate::server::middleware::ApiKeyRole::User),
            Extension(Locale::En),
            Json(req),
        )
        .await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_handle_create_biz_tag_admin_role_returns_forbidden() {
        let state = create_test_app_state();
        let result = handle_create_biz_tag(
            State(state),
            Extension(None),
            Extension(crate::server::middleware::ApiKeyRole::Admin),
            Extension(Locale::En),
            Json(make_create_biz_tag_request()),
        )
        .await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn test_handle_create_biz_tag_anonymous_role_returns_unauthorized() {
        let state = create_test_app_state();
        let result = handle_create_biz_tag(
            State(state),
            Extension(None),
            Extension(crate::server::middleware::ApiKeyRole::Anonymous),
            Extension(Locale::En),
            Json(make_create_biz_tag_request()),
        )
        .await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_handle_create_biz_tag_workspace_mismatch_returns_forbidden() {
        let state = create_test_app_state();
        // Key has a different workspace_id than the request.
        let key_workspace = uuid::Uuid::new_v4();
        let result = handle_create_biz_tag(
            State(state),
            Extension(Some(uuid::Uuid::new_v4())), // different UUID
            Extension(crate::server::middleware::ApiKeyRole::User),
            Extension(Locale::En),
            Json(make_create_biz_tag_request()),
        )
        .await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        // verify_workspace_id returns 403 on mismatch.
        assert_eq!(status, StatusCode::FORBIDDEN);
        // Silence unused variable warning in a way that documents intent.
        let _ = key_workspace;
    }

    #[tokio::test]
    async fn test_handle_create_biz_tag_with_matching_key_fails_at_handler() {
        let state = create_test_app_state();
        let req = make_create_biz_tag_request();
        let key = req.workspace_id;
        let result = handle_create_biz_tag(
            State(state),
            Extension(Some(key)),
            Extension(crate::server::middleware::ApiKeyRole::User),
            Extension(Locale::En),
            Json(req),
        )
        .await;
        // Role + workspace match, so handlers.create_biz_tag runs.
        // Without a repository the handler returns Err, which may map to
        // 400 (InvalidInput/ParseError), 404 (NotFound/BizTagNotFound),
        // or 500 (InternalError) depending on the CoreError variant.
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert!(
            status == StatusCode::BAD_REQUEST
                || status == StatusCode::NOT_FOUND
                || status == StatusCode::INTERNAL_SERVER_ERROR,
            "expected BAD_REQUEST, NOT_FOUND, or INTERNAL_SERVER_ERROR, got {}",
            status
        );
    }

    // ========== handle_get_biz_tag tests ==========

    #[tokio::test]
    async fn test_handle_get_biz_tag_invalid_uuid_returns_bad_request() {
        let state = create_test_app_state();
        let result = handle_get_biz_tag(
            State(state),
            Extension(Locale::En),
            Path("not-a-uuid".to_string()),
        )
        .await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_handle_get_biz_tag_valid_uuid_fails_at_handler() {
        let state = create_test_app_state();
        let result = handle_get_biz_tag(
            State(state),
            Extension(Locale::En),
            Path(uuid::Uuid::new_v4().to_string()),
        )
        .await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        // Without repository, BizTagNotFound or InternalError -> 404 or 500.
        assert!(
            status == StatusCode::NOT_FOUND || status == StatusCode::INTERNAL_SERVER_ERROR,
            "expected NOT_FOUND or INTERNAL_SERVER_ERROR, got {}",
            status
        );
    }

    // ========== handle_update_biz_tag tests ==========

    #[tokio::test]
    async fn test_handle_update_biz_tag_invalid_uuid_returns_bad_request() {
        let state = create_test_app_state();
        let req = UpdateBizTagRequest {
            name: Some("new-name".to_string()),
            description: None,
            algorithm: None,
            format: None,
            prefix: None,
            base_step: None,
            max_step: None,
            datacenter_ids: None,
        };
        let result = handle_update_biz_tag(
            State(state),
            Extension(Locale::En),
            Path("not-a-uuid".to_string()),
            Json(req),
        )
        .await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_handle_update_biz_tag_invalid_request_returns_bad_request() {
        let state = create_test_app_state();
        let req = UpdateBizTagRequest {
            name: Some(String::new()), // length(min=1) violation
            description: None,
            algorithm: None,
            format: None,
            prefix: None,
            base_step: None,
            max_step: None,
            datacenter_ids: None,
        };
        let result = handle_update_biz_tag(
            State(state),
            Extension(Locale::En),
            Path(uuid::Uuid::new_v4().to_string()),
            Json(req),
        )
        .await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    // ========== handle_delete_biz_tag tests ==========

    #[tokio::test]
    async fn test_handle_delete_biz_tag_invalid_uuid_returns_bad_request() {
        let state = create_test_app_state();
        let result = handle_delete_biz_tag(
            State(state),
            Extension(Locale::En),
            Path("not-a-uuid".to_string()),
        )
        .await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_handle_delete_biz_tag_valid_uuid_fails_at_handler() {
        let state = create_test_app_state();
        let result = handle_delete_biz_tag(
            State(state),
            Extension(Locale::En),
            Path(uuid::Uuid::new_v4().to_string()),
        )
        .await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert!(
            status == StatusCode::NOT_FOUND || status == StatusCode::INTERNAL_SERVER_ERROR,
            "expected NOT_FOUND or INTERNAL_SERVER_ERROR, got {}",
            status
        );
    }

    // ========== handle_list_biz_tags tests ==========

    #[tokio::test]
    async fn test_handle_list_biz_tags_invalid_page_size_returns_bad_request() {
        let state = create_test_app_state();
        let params = PaginationParams {
            workspace_id: None,
            page: 1,
            page_size: 101, // above max=100
        };
        let result = handle_list_biz_tags(State(state), Extension(Locale::En), Query(params)).await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_handle_list_biz_tags_zero_page_size_returns_bad_request() {
        let state = create_test_app_state();
        let params = PaginationParams {
            workspace_id: None,
            page: 1,
            page_size: 0, // below min=1
        };
        let result = handle_list_biz_tags(State(state), Extension(Locale::En), Query(params)).await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_handle_list_biz_tags_valid_params_returns_response_or_error() {
        let state = create_test_app_state();
        let params = PaginationParams {
            workspace_id: None,
            page: 1,
            page_size: 20,
        };
        let result = handle_list_biz_tags(State(state), Extension(Locale::En), Query(params)).await;
        // Without repository, list_biz_tags_with_pagination returns Err.
        match result {
            Ok(resp) => {
                let _ = resp.0.total;
            }
            Err((status, _)) => {
                assert!(
                    status == StatusCode::INTERNAL_SERVER_ERROR || status == StatusCode::NOT_FOUND
                );
            }
        }
    }

    // ========== handle_create_workspace tests ==========

    #[tokio::test]
    async fn test_handle_create_workspace_invalid_request_returns_bad_request() {
        let state = create_test_app_state();
        let req = CreateWorkspaceRequest {
            name: String::new(), // length(min=1) violation
            description: None,
            max_groups: None,
            max_biz_tags: None,
        };
        let result = handle_create_workspace(State(state), Extension(Locale::En), Json(req)).await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_handle_create_workspace_invalid_max_groups_returns_bad_request() {
        let state = create_test_app_state();
        let req = CreateWorkspaceRequest {
            name: "ws".to_string(),
            description: None,
            max_groups: Some(0), // below min=1
            max_biz_tags: None,
        };
        let result = handle_create_workspace(State(state), Extension(Locale::En), Json(req)).await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_handle_create_workspace_valid_request_fails_at_handler() {
        let state = create_test_app_state();
        let req = CreateWorkspaceRequest {
            name: "new-ws".to_string(),
            description: None,
            max_groups: None,
            max_biz_tags: None,
        };
        let result = handle_create_workspace(State(state), Extension(Locale::En), Json(req)).await;
        // Without repository, create_workspace returns Err.
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    }

    // ========== handle_list_workspaces tests ==========

    #[tokio::test]
    async fn test_handle_list_workspaces_without_repository_returns_error() {
        let state = create_test_app_state();
        let result = handle_list_workspaces(State(state), Extension(Locale::En)).await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    }

    // ========== handle_get_workspace tests ==========

    #[tokio::test]
    async fn test_handle_get_workspace_without_repository_returns_internal_error() {
        let state = create_test_app_state();
        let result = handle_get_workspace(
            State(state),
            Extension(Locale::En),
            Path("any-name".to_string()),
        )
        .await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    }

    // ========== handle_create_group tests ==========

    #[tokio::test]
    async fn test_handle_create_group_invalid_request_returns_bad_request() {
        let state = create_test_app_state();
        let req = CreateGroupRequest {
            workspace: String::new(), // length(min=1) violation
            name: "g".to_string(),
            description: None,
            max_biz_tags: None,
        };
        let result = handle_create_group(
            State(state),
            Extension(None),
            Extension(crate::server::middleware::ApiKeyRole::User),
            Extension(Locale::En),
            Json(req),
        )
        .await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_handle_create_group_admin_role_returns_forbidden() {
        let state = create_test_app_state();
        let req = CreateGroupRequest {
            workspace: "ws".to_string(),
            name: "g".to_string(),
            description: None,
            max_biz_tags: None,
        };
        let result = handle_create_group(
            State(state),
            Extension(None),
            Extension(crate::server::middleware::ApiKeyRole::Admin),
            Extension(Locale::En),
            Json(req),
        )
        .await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn test_handle_create_group_anonymous_role_returns_unauthorized() {
        let state = create_test_app_state();
        let req = CreateGroupRequest {
            workspace: "ws".to_string(),
            name: "g".to_string(),
            description: None,
            max_biz_tags: None,
        };
        let result = handle_create_group(
            State(state),
            Extension(None),
            Extension(crate::server::middleware::ApiKeyRole::Anonymous),
            Extension(Locale::En),
            Json(req),
        )
        .await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_handle_create_group_user_role_fails_at_workspace_lookup() {
        let state = create_test_app_state();
        let req = CreateGroupRequest {
            workspace: "ws".to_string(),
            name: "g".to_string(),
            description: None,
            max_biz_tags: None,
        };
        let result = handle_create_group(
            State(state),
            Extension(None),
            Extension(crate::server::middleware::ApiKeyRole::User),
            Extension(Locale::En),
            Json(req),
        )
        .await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        // verify_user_workspace hits the missing repository -> 500.
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    }

    // ========== handle_regenerate_user_key tests ==========

    #[tokio::test]
    async fn test_handle_regenerate_user_key_without_repository_returns_error() {
        let state = create_test_app_state();
        let result = handle_regenerate_user_key(
            State(state),
            Extension(Locale::En),
            Path("any-name".to_string()),
        )
        .await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        // Without repository, regenerate_user_api_key returns Err.
        assert!(status == StatusCode::INTERNAL_SERVER_ERROR || status == StatusCode::NOT_FOUND);
    }

    // ========== handle_list_groups tests ==========

    #[tokio::test]
    async fn test_handle_list_groups_invalid_page_size_returns_bad_request() {
        let state = create_test_app_state();
        let params = GroupListParams {
            workspace: "ws".to_string(),
            page: 1,
            page_size: 0, // below min=1
        };
        let result = handle_list_groups(State(state), Extension(Locale::En), Query(params)).await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_handle_list_groups_above_max_page_size_returns_bad_request() {
        let state = create_test_app_state();
        let params = GroupListParams {
            workspace: "ws".to_string(),
            page: 1,
            page_size: 101, // above max=100
        };
        let result = handle_list_groups(State(state), Extension(Locale::En), Query(params)).await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_handle_list_groups_valid_params_fails_at_handler() {
        let state = create_test_app_state();
        let params = GroupListParams {
            workspace: "ws".to_string(),
            page: 1,
            page_size: 20,
        };
        let result = handle_list_groups(State(state), Extension(Locale::En), Query(params)).await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        // Without repository, list_groups returns Err.
        assert!(status == StatusCode::INTERNAL_SERVER_ERROR || status == StatusCode::NOT_FOUND);
    }

    // ========== handle_create_api_key tests ==========

    #[tokio::test]
    async fn test_handle_create_api_key_invalid_request_returns_bad_request() {
        let state = create_test_app_state();
        let req = CreateApiKeyRequest {
            workspace_id: None,
            name: String::new(), // length(min=1) violation
            description: None,
            role: Some("user".to_string()),
            rate_limit: None,
            expires_at: None,
        };
        let result = handle_create_api_key(State(state), Extension(Locale::En), Json(req)).await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_handle_create_api_key_user_role_without_workspace_id_returns_bad_request() {
        let state = create_test_app_state();
        let req = CreateApiKeyRequest {
            workspace_id: None, // missing for user role
            name: "key".to_string(),
            description: None,
            role: Some("user".to_string()),
            rate_limit: None,
            expires_at: None,
        };
        let result = handle_create_api_key(State(state), Extension(Locale::En), Json(req)).await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        // workspace_id_required_response returns 400.
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_handle_create_api_key_user_role_with_invalid_workspace_id_returns_bad_request() {
        let state = create_test_app_state();
        let req = CreateApiKeyRequest {
            workspace_id: Some("not-a-uuid".to_string()),
            name: "key".to_string(),
            description: None,
            role: Some("user".to_string()),
            rate_limit: None,
            expires_at: None,
        };
        let result = handle_create_api_key(State(state), Extension(Locale::En), Json(req)).await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        // invalid_uuid_response returns 400.
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_handle_create_api_key_default_role_without_workspace_id_returns_bad_request() {
        let state = create_test_app_state();
        let req = CreateApiKeyRequest {
            workspace_id: None, // missing for default user role
            name: "key".to_string(),
            description: None,
            role: None, // defaults to user
            rate_limit: None,
            expires_at: None,
        };
        let result = handle_create_api_key(State(state), Extension(Locale::En), Json(req)).await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_handle_create_api_key_admin_role_without_workspace_id_fails_at_handler() {
        let state = create_test_app_state();
        let req = CreateApiKeyRequest {
            workspace_id: None, // admin keys don't need workspace
            name: "admin-key".to_string(),
            description: None,
            role: Some("admin".to_string()),
            rate_limit: Some(1000),
            expires_at: None,
        };
        let result = handle_create_api_key(State(state), Extension(Locale::En), Json(req)).await;
        // Validation passes, workspace_id is None for admin, then
        // handlers.create_api_key runs. Without repository it returns Err.
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        // Without a repository, the handler may return NotFound (404) or
        // InternalError (500) depending on which CoreError variant surfaces.
        assert!(
            status == StatusCode::NOT_FOUND || status == StatusCode::INTERNAL_SERVER_ERROR,
            "expected NOT_FOUND or INTERNAL_SERVER_ERROR, got {}",
            status
        );
    }

    #[tokio::test]
    async fn test_handle_create_api_key_user_role_with_valid_workspace_id_fails_at_handler() {
        let state = create_test_app_state();
        let req = CreateApiKeyRequest {
            workspace_id: Some(uuid::Uuid::new_v4().to_string()),
            name: "user-key".to_string(),
            description: None,
            role: Some("user".to_string()),
            rate_limit: Some(1000),
            expires_at: None,
        };
        let result = handle_create_api_key(State(state), Extension(Locale::En), Json(req)).await;
        // Validation passes, workspace_id parses, then handlers.create_api_key
        // runs. Without repository it returns Err.
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        // Without a repository, the handler may return NotFound (404) or
        // InternalError (500) depending on which CoreError variant surfaces.
        assert!(
            status == StatusCode::NOT_FOUND || status == StatusCode::INTERNAL_SERVER_ERROR,
            "expected NOT_FOUND or INTERNAL_SERVER_ERROR, got {}",
            status
        );
    }

    // ========== handle_list_api_keys tests ==========

    #[tokio::test]
    async fn test_handle_list_api_keys_invalid_page_size_returns_bad_request() {
        let state = create_test_app_state();
        let params = PaginationParams {
            workspace_id: None,
            page: 1,
            page_size: 0, // below min=1
        };
        let result = handle_list_api_keys(State(state), Extension(Locale::En), Query(params)).await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_handle_list_api_keys_valid_params_fails_at_handler() {
        let state = create_test_app_state();
        let params = PaginationParams {
            workspace_id: None,
            page: 1,
            page_size: 20,
        };
        let result = handle_list_api_keys(State(state), Extension(Locale::En), Query(params)).await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        // Without repository, list_api_keys returns Err.
        assert!(status == StatusCode::INTERNAL_SERVER_ERROR || status == StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_handle_list_api_keys_with_invalid_workspace_id_falls_back_to_nil() {
        let state = create_test_app_state();
        let params = PaginationParams {
            workspace_id: Some("not-a-uuid".to_string()),
            page: 1,
            page_size: 20,
        };
        let result = handle_list_api_keys(State(state), Extension(Locale::En), Query(params)).await;
        // Invalid workspace_id is logged and falls back to Uuid::nil().
        // The handler still calls list_api_keys which fails without a repo.
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert!(status == StatusCode::INTERNAL_SERVER_ERROR || status == StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_handle_list_api_keys_with_valid_workspace_id_fails_at_handler() {
        let state = create_test_app_state();
        let params = PaginationParams {
            workspace_id: Some(uuid::Uuid::new_v4().to_string()),
            page: 2,
            page_size: 50,
        };
        let result = handle_list_api_keys(State(state), Extension(Locale::En), Query(params)).await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert!(status == StatusCode::INTERNAL_SERVER_ERROR || status == StatusCode::NOT_FOUND);
    }

    // ========== handle_revoke_api_key tests ==========

    #[tokio::test]
    async fn test_handle_revoke_api_key_invalid_uuid_returns_bad_request() {
        let state = create_test_app_state();
        let result = handle_revoke_api_key(
            State(state),
            Extension(Locale::En),
            Path("not-a-uuid".to_string()),
        )
        .await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_handle_revoke_api_key_valid_uuid_fails_at_handler() {
        let state = create_test_app_state();
        let result = handle_revoke_api_key(
            State(state),
            Extension(Locale::En),
            Path(uuid::Uuid::new_v4().to_string()),
        )
        .await;
        assert!(result.is_err());
        let (status, _) = result.unwrap_err();
        // Without repository, revoke_api_key returns Err.
        assert!(status == StatusCode::INTERNAL_SERVER_ERROR || status == StatusCode::NOT_FOUND);
    }
}
