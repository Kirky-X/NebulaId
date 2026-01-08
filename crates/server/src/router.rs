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

use crate::api_version::{api_version_middleware, API_V1};
use crate::audit::AuditLogger;
use crate::audit_middleware::AuditMiddleware;
use crate::config_management::ConfigManagementService;
use crate::cors_config;
use crate::handlers::ApiHandlers;
use crate::middleware::ApiKeyAuth;
use crate::models::{
    ApiInfoResponse, ApiKeyListResponse, ApiKeyWithSecretResponse, BatchGenerateRequest,
    BatchGenerateResponse, BizTagListResponse, BizTagResponse, CreateApiKeyRequest,
    CreateBizTagRequest, CreateGroupRequest, CreateWorkspaceRequest, ErrorResponse,
    GenerateRequest, GenerateResponse, GroupListParams, GroupListResponse, GroupResponse,
    HealthResponse, MetricsResponse, PaginationParams, ParseRequest, ParseResponse, ReadyResponse,
    RevokeApiKeyResponse, SecureConfigResponse, SetAlgorithmRequest, SetAlgorithmResponse,
    UpdateBizTagRequest, UpdateConfigResponse, UpdateLoggingRequest, UpdateRateLimitRequest,
    WorkspaceListResponse, WorkspaceResponse,
};
use crate::rate_limit::RateLimiter;
use crate::rate_limit_middleware::RateLimitMiddleware;
use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderValue, StatusCode},
    routing::{delete, get, post},
    Json, Router,
};
use std::sync::Arc;
use tower_http::set_header::SetResponseHeaderLayer;
use validator::Validate;

use std::ops::Deref;

#[derive(Clone)]
pub struct AppState {
    pub handlers: Arc<ApiHandlers>,
    pub auth: Arc<ApiKeyAuth>,
    pub config_service: Arc<ConfigManagementService>,
}

impl Deref for AppState {
    type Target = ConfigManagementService;

    fn deref(&self) -> &Self::Target {
        &self.config_service
    }
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
    let cors = cors_config::create_env_aware_cors_layer();

    let rate_limit_middleware = RateLimitMiddleware::new(rate_limiter.clone());
    let audit_middleware = AuditMiddleware::new(audit_logger.clone(), auth.clone(), rate_limiter);

    let config_service = handlers.get_config_service();

    let app_state = AppState {
        handlers: handlers.clone(),
        auth: auth.clone(),
        config_service: config_service.clone(),
    };

    // ========== V1 API Routes ==========
    // Admin-only endpoints (require admin API key)
    let v1_admin_routes = Router::new()
        .route("/metrics", get(handle_metrics))
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
            crate::middleware::admin_required_middleware,
        ))
        .layer(axum::middleware::from_fn_with_state(
            auth.clone(),
            crate::middleware::auth_middleware_fn,
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
        .layer(axum::middleware::from_fn_with_state(
            auth.clone(),
            crate::middleware::auth_middleware_fn,
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
        .layer(axum::middleware::from_fn(api_version_middleware));

    // ========== Root Routes ==========
    // Public router (no authentication) - only health check
    // Note: Swagger UI temporarily disabled
    Router::new()
        .route("/health", get(handle_health))
        .route("/ready", get(handle_ready))
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

/// Verify that the request is from a User API key (not Admin)
fn verify_user_role(
    role: crate::middleware::ApiKeyRole,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    if role == crate::middleware::ApiKeyRole::Admin {
        return Err((
            StatusCode::FORBIDDEN,
            Json(ErrorResponse::new(
                403,
                "Admin API key cannot perform this operation".to_string(),
            )),
        ));
    }
    Ok(())
}

/// Validate request and return error if invalid
fn validate_request<T: Validate>(req: &T) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    if let Err(validation_errors) = req.validate() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(
                400,
                format!("Validation error: {}", validation_errors),
            )),
        ));
    }
    Ok(())
}

/// Verify workspace_id match for User API Key (by workspace name lookup)
async fn verify_user_workspace(
    workspace_name: &str,
    key_workspace_id: &Option<uuid::Uuid>,
    handlers: &Arc<ApiHandlers>,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    let workspace_result = handlers.get_workspace(workspace_name).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(500, e.to_string())),
        )
    })?;

    let workspace = workspace_result.ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new(
                404,
                format!("Workspace '{}' not found", workspace_name),
            )),
        )
    })?;

    let workspace_uuid = uuid::Uuid::parse_str(&workspace.id).map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(500, "Invalid workspace ID".to_string())),
        )
    })?;

    verify_workspace_id_match(workspace_uuid, key_workspace_id)
}

/// 提取通用的 workspace_id 比较逻辑
fn verify_workspace_id_match(
    workspace_uuid: uuid::Uuid,
    key_workspace_id: &Option<uuid::Uuid>,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    if Some(workspace_uuid) != *key_workspace_id {
        return Err((
            StatusCode::FORBIDDEN,
            Json(ErrorResponse::new(
                403,
                "Access denied: workspace mismatch".to_string(),
            )),
        ));
    }
    Ok(())
}

/// Verify workspace_id match for User API Key (direct Uuid comparison)
fn verify_workspace_id(
    req_workspace_id: uuid::Uuid,
    key_workspace_id: &Option<uuid::Uuid>,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    verify_workspace_id_match(req_workspace_id, key_workspace_id)
}

async fn handle_generate(
    State(state): State<AppState>,
    extensions: axum::Extension<Option<uuid::Uuid>>,
    extensions_role: axum::Extension<crate::middleware::ApiKeyRole>,
    Json(req): Json<GenerateRequest>,
) -> Result<Json<GenerateResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Only User API Key can generate IDs
    verify_user_role(extensions_role.0)?;

    // Validate request parameters
    validate_request(&req)?;

    // Verify workspace_id match for User API Key
    verify_user_workspace(&req.workspace, &extensions.0, &state.handlers).await?;

    state
        .handlers
        .generate(req)
        .await
        .map(Json)
        .map_err(|e: nebula_core::types::CoreError| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(500, e.to_string())),
            )
        })
}

async fn handle_batch_generate(
    State(state): State<AppState>,
    extensions: axum::Extension<Option<uuid::Uuid>>,
    extensions_role: axum::Extension<crate::middleware::ApiKeyRole>,
    Json(req): Json<BatchGenerateRequest>,
) -> Result<Json<BatchGenerateResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Only User API Key can generate IDs
    verify_user_role(extensions_role.0)?;

    tracing::debug!(
        "HTTP batch_generate request: workspace={}, group={}, size={:?}",
        req.workspace,
        req.group,
        req.size
    );

    // Validate request parameters
    validate_request(&req)?;

    // Verify workspace_id match for User API Key
    verify_user_workspace(&req.workspace, &extensions.0, &state.handlers).await?;

    state.handlers.batch_generate(req).await.map(Json).map_err(
        |e: nebula_core::types::CoreError| {
            let status_code = match &e {
                nebula_core::types::CoreError::InvalidInput(msg) => {
                    tracing::warn!("HTTP batch generation failed: {}", msg);
                    StatusCode::BAD_REQUEST
                }
                _ => {
                    tracing::error!("HTTP batch generation error: {}", e);
                    StatusCode::INTERNAL_SERVER_ERROR
                }
            };
            (
                status_code,
                Json(ErrorResponse::new(
                    status_code.as_u16() as i32,
                    e.to_string(),
                )),
            )
        },
    )
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
    Json(req): Json<ParseRequest>,
) -> Result<Json<ParseResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Validate request parameters
    if let Err(validation_errors) = req.validate() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(
                400,
                format!("Validation error: {}", validation_errors),
            )),
        ));
    }

    state
        .handlers
        .parse(req)
        .await
        .map(Json)
        .map_err(|e: nebula_core::types::CoreError| {
            (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse::new(400, e.to_string())),
            )
        })
}

async fn handle_get_config(State(state): State<AppState>) -> Json<SecureConfigResponse> {
    Json(state.config_service.get_secure_config())
}

async fn handle_update_rate_limit(
    State(state): State<AppState>,
    Json(req): Json<UpdateRateLimitRequest>,
) -> Json<UpdateConfigResponse> {
    Json(state.config_service.update_rate_limit(req).await)
}

async fn handle_update_logging(
    State(state): State<AppState>,
    Json(req): Json<UpdateLoggingRequest>,
) -> Json<UpdateConfigResponse> {
    Json(state.config_service.update_logging(req).await)
}

#[axum::debug_handler]
async fn handle_reload_config(State(state): State<AppState>) -> Json<UpdateConfigResponse> {
    Json(state.config_service.reload_config().await)
}

async fn handle_set_algorithm(
    State(state): State<AppState>,
    Json(req): Json<SetAlgorithmRequest>,
) -> Json<SetAlgorithmResponse> {
    Json(state.config_service.set_algorithm(req).await)
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
    extensions_role: axum::Extension<crate::middleware::ApiKeyRole>,
    Json(req): Json<CreateBizTagRequest>,
) -> Result<Json<BizTagResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Validate request parameters
    validate_request(&req)?;

    // Only User API Key can create biz_tags
    verify_user_role(extensions_role.0)?;

    // Verify workspace_id match for User API Key
    verify_workspace_id(req.workspace_id, &extensions.0)?;

    state
        .handlers
        .create_biz_tag(req)
        .await
        .map(Json)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(500, e.to_string())),
            )
        })
}

async fn handle_get_biz_tag(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<BizTagResponse>, (StatusCode, Json<ErrorResponse>)> {
    let uuid = uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(400, "Invalid UUID format".to_string())),
        )
    })?;

    state
        .handlers
        .get_biz_tag(uuid)
        .await
        .map(Json)
        .map_err(|e| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse::new(404, e.to_string())),
            )
        })
}

async fn handle_update_biz_tag(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateBizTagRequest>,
) -> Result<Json<BizTagResponse>, (StatusCode, Json<ErrorResponse>)> {
    let uuid = uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(400, "Invalid UUID format".to_string())),
        )
    })?;

    if let Err(validation_errors) = req.validate() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(
                400,
                format!("Validation error: {}", validation_errors),
            )),
        ));
    }

    state
        .handlers
        .update_biz_tag(uuid, req)
        .await
        .map(Json)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(500, e.to_string())),
            )
        })
}

async fn handle_delete_biz_tag(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let uuid = uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(400, "Invalid UUID format".to_string())),
        )
    })?;

    state
        .handlers
        .delete_biz_tag(uuid)
        .await
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(500, e.to_string())),
            )
        })
}

async fn handle_list_biz_tags(
    State(state): State<AppState>,
    Query(params): Query<PaginationParams>,
) -> Json<BizTagListResponse> {
    let page = params.page.max(1);
    let page_size = params.page_size.clamp(1, 100);
    let offset = (page - 1) * page_size;

    match state
        .handlers
        .list_biz_tags_with_pagination(None, None, page_size as usize, offset as usize)
        .await
    {
        Ok(response) => Json(BizTagListResponse {
            biz_tags: response.biz_tags,
            total: response.total,
            page,
            page_size,
        }),
        Err(_) => Json(BizTagListResponse {
            biz_tags: vec![],
            total: 0,
            page,
            page_size,
        }),
    }
}

// ========== Workspace Handlers ==========

async fn handle_create_workspace(
    State(state): State<AppState>,
    Json(req): Json<CreateWorkspaceRequest>,
) -> Result<Json<WorkspaceResponse>, (StatusCode, Json<ErrorResponse>)> {
    if let Err(validation_errors) = req.validate() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(
                400,
                format!("Validation error: {}", validation_errors),
            )),
        ));
    }

    state
        .handlers
        .create_workspace(req)
        .await
        .map(Json)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(500, e.to_string())),
            )
        })
}

async fn handle_list_workspaces(State(state): State<AppState>) -> Json<WorkspaceListResponse> {
    match state.handlers.list_workspaces().await {
        Ok(response) => Json(response),
        Err(_) => Json(WorkspaceListResponse {
            workspaces: vec![],
            total: 0,
        }),
    }
}

async fn handle_get_workspace(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<WorkspaceResponse>, (StatusCode, Json<ErrorResponse>)> {
    match state.handlers.get_workspace(&name).await {
        Ok(Some(ws)) => Ok(Json(ws)),
        Ok(None) => Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new(404, "Workspace not found".to_string())),
        )),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(500, e.to_string())),
        )),
    }
}

// ========== Group Handlers ==========

async fn handle_create_group(
    State(state): State<AppState>,
    extensions: axum::Extension<Option<uuid::Uuid>>,
    extensions_role: axum::Extension<crate::middleware::ApiKeyRole>,
    Json(req): Json<CreateGroupRequest>,
) -> Result<Json<GroupResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Validate request parameters
    validate_request(&req)?;

    // Only User API Key can create groups
    verify_user_role(extensions_role.0)?;

    // Verify workspace_id match for User API Key
    verify_user_workspace(&req.workspace, &extensions.0, &state.handlers).await?;

    state
        .handlers
        .create_group(req)
        .await
        .map(Json)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(500, e.to_string())),
            )
        })
}

async fn handle_regenerate_user_key(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<ApiKeyWithSecretResponse>, (StatusCode, Json<ErrorResponse>)> {
    state
        .handlers
        .regenerate_user_api_key(&name)
        .await
        .map(Json)
        .map_err(|e: nebula_core::CoreError| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(500, e.to_string())),
            )
        })
}

async fn handle_list_groups(
    State(state): State<AppState>,
    Query(params): Query<GroupListParams>,
) -> Json<GroupListResponse> {
    let workspace = params.workspace.clone();

    match state.handlers.list_groups(&workspace).await {
        Ok(response) => Json(response),
        Err(_) => Json(GroupListResponse {
            groups: vec![],
            total: 0,
        }),
    }
}

// ========== API Key Handlers ==========

async fn handle_create_api_key(
    State(state): State<AppState>,
    Json(req): Json<CreateApiKeyRequest>,
) -> Result<Json<ApiKeyWithSecretResponse>, (StatusCode, Json<ErrorResponse>)> {
    if let Err(validation_errors) = req.validate() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(
                400,
                format!("Validation error: {}", validation_errors),
            )),
        ));
    }

    // Determine workspace_id based on role
    let workspace_id = if let Some(ref role_str) = req.role {
        if role_str == "admin" {
            // Admin keys are global, not bound to any workspace
            None
        } else {
            // User keys must be bound to a workspace
            Some(
                req.workspace_id
                    .as_ref()
                    .and_then(|s| uuid::Uuid::parse_str(s).ok())
                    .ok_or((
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse::new(
                            400,
                            "workspace_id is required for user keys".to_string(),
                        )),
                    ))?,
            )
        }
    } else {
        // Default to user role if not specified
        Some(
            req.workspace_id
                .as_ref()
                .and_then(|s| uuid::Uuid::parse_str(s).ok())
                .ok_or((
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse::new(
                        400,
                        "workspace_id is required for user keys".to_string(),
                    )),
                ))?,
        )
    };

    state
        .handlers
        .create_api_key(workspace_id, req)
        .await
        .map(Json)
        .map_err(|e: nebula_core::types::CoreError| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(500, e.to_string())),
            )
        })
}

async fn handle_list_api_keys(
    State(state): State<AppState>,
    Query(params): Query<PaginationParams>,
) -> Json<ApiKeyListResponse> {
    let page = params.page.max(1);
    let page_size = params.page_size.clamp(1, 100);
    let offset = (page - 1) * page_size;

    // Get workspace_id from query parameters
    let workspace_id = params
        .workspace_id
        .as_ref()
        .and_then(|s| uuid::Uuid::parse_str(s).ok())
        .unwrap_or_else(uuid::Uuid::nil);

    match state
        .handlers
        .list_api_keys(workspace_id, Some(page_size as u32), Some(offset as u32))
        .await
    {
        Ok(response) => Json(response),
        Err(_) => Json(ApiKeyListResponse {
            api_keys: vec![],
            total: 0,
        }),
    }
}

async fn handle_revoke_api_key(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<RevokeApiKeyResponse>, (StatusCode, Json<ErrorResponse>)> {
    let uuid = uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(400, "Invalid UUID format".to_string())),
        )
    })?;

    state
        .handlers
        .revoke_api_key(uuid)
        .await
        .map(Json)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(500, e.to_string())),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config_hot_reload::HotReloadConfig;
    use nebula_core::algorithm::AlgorithmRouter;
    use nebula_core::config::Config;
    use std::sync::Arc;

    fn create_test_api_handlers() -> Arc<ApiHandlers> {
        let config = Config::default();
        let hot_config = Arc::new(HotReloadConfig::new(
            config.clone(),
            "config/config.toml".to_string(),
        ));
        let algorithm_router = Arc::new(AlgorithmRouter::new(config.clone(), None));
        let config_service = Arc::new(ConfigManagementService::new(
            hot_config,
            algorithm_router.clone(),
        ));
        let handlers = ApiHandlers::new(algorithm_router, config_service);
        Arc::new(handlers)
    }

    fn create_test_auth() -> Arc<ApiKeyAuth> {
        use async_trait::async_trait;
        use nebula_core::database::{
            ApiKeyInfo, ApiKeyRepository, ApiKeyResponse, ApiKeyRole as CoreApiKeyRole,
            ApiKeyWithSecret, CreateApiKeyRequest,
        };
        use nebula_core::types::Result;
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
            ) -> Result<Option<(Option<Uuid>, nebula_core::database::ApiKeyRole)>> {
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
                Err(nebula_core::types::error::CoreError::InternalError(
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

        Arc::new(ApiKeyAuth::new(Arc::new(MockApiKeyRepo)))
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
