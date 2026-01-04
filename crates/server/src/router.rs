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

use crate::audit::AuditLogger;
use crate::audit_middleware::AuditMiddleware;
use crate::config_management::ConfigManagementService;
use crate::handlers::ApiHandlers;
use crate::middleware::ApiKeyAuth;
use crate::models::{
    ApiInfoResponse, BatchGenerateRequest, BatchGenerateResponse, BizTagListResponse,
    BizTagResponse, CreateBizTagRequest, ErrorResponse, GenerateRequest, GenerateResponse,
    HealthResponse, MetricsResponse, PaginationParams, ParseRequest, ParseResponse,
    SecureConfigResponse, SetAlgorithmRequest, SetAlgorithmResponse, UpdateBizTagRequest,
    UpdateConfigResponse, UpdateLoggingRequest, UpdateRateLimitRequest,
};
use crate::rate_limit::RateLimiter;
use crate::rate_limit_middleware::RateLimitMiddleware;
use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderValue, Method, StatusCode},
    routing::{get, post},
    Json, Router,
};
use std::sync::Arc;
use tower_http::{cors::CorsLayer, set_header::SetResponseHeaderLayer};
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
    // Configure CORS with strict settings
    // In production, specify your actual frontend origins via ALLOWED_ORIGINS env var
    // Format: comma-separated list of allowed origins
    // Example: ALLOWED_ORIGINS="https://example.com,https://app.example.com"
    let allowed_origins: Vec<HeaderValue> = std::env::var("ALLOWED_ORIGINS")
        .ok()
        .map(|origins| {
            origins
                .split(',')
                .filter_map(|origin| origin.trim().parse().ok())
                .collect()
        })
        .unwrap_or_else(|| {
            // Development: allow localhost for testing
            vec!["http://localhost:3000".parse().unwrap()]
        });

    // Use empty list to deny all origins in production without ALLOWED_ORIGINS
    let cors = CorsLayer::new()
        .allow_origin(tower_http::cors::AllowOrigin::list(allowed_origins))
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers(tower_http::cors::Any)
        .allow_credentials(false);

    let rate_limit_middleware = RateLimitMiddleware::new(rate_limiter.clone());
    let audit_middleware = AuditMiddleware::new(audit_logger.clone(), auth.clone(), rate_limiter);

    let config_service = handlers.get_config_service();

    let app_state = AppState {
        handlers: handlers.clone(),
        auth: auth.clone(),
        config_service: config_service.clone(),
    };

    // Public router (no authentication) - only health check
    let public_routes = Router::new()
        .route("/api/v1", get(handle_api_info))
        .route("/api/v1/generate", post(handle_generate))
        .route("/api/v1/generate/batch", post(handle_batch_generate))
        .route("/api/v1/parse", post(handle_parse))
        .route("/health", get(handle_health));

    // Admin-only router (requires admin API key)
    let admin_only_routes = Router::new()
        .route("/metrics", get(handle_metrics))
        // Apply admin requirement middleware
        .layer(axum::middleware::from_fn(
            crate::middleware::admin_required_middleware,
        ));

    // Authenticated router (requires API key)
    let authenticated_routes = Router::new()
        .route("/api/v1/config", get(handle_get_config))
        .route("/api/v1/config/rate-limit", post(handle_update_rate_limit))
        .route("/api/v1/config/logging", post(handle_update_logging))
        .route("/api/v1/config/reload", post(handle_reload_config))
        .route("/api/v1/config/algorithm", post(handle_set_algorithm))
        // BizTag CRUD endpoints
        .route(
            "/api/v1/biz-tags",
            post(handle_create_biz_tag).get(handle_list_biz_tags),
        )
        .route(
            "/api/v1/biz-tags/{id}",
            get(handle_get_biz_tag)
                .put(handle_update_biz_tag)
                .delete(handle_delete_biz_tag),
        )
        // Apply auth middleware
        .layer(axum::middleware::from_fn_with_state(
            auth.clone(),
            crate::middleware::auth_middleware_fn,
        ));

    // Merge routes with shared state
    public_routes
        .merge(authenticated_routes)
        .merge(admin_only_routes)
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

async fn handle_generate(
    State(state): State<AppState>,
    Json(req): Json<GenerateRequest>,
) -> Result<Json<GenerateResponse>, (StatusCode, Json<ErrorResponse>)> {
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

    state.handlers.generate(req).await.map(Json).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(500, e.to_string())),
        )
    })
}

async fn handle_batch_generate(
    State(state): State<AppState>,
    Json(req): Json<BatchGenerateRequest>,
) -> Result<Json<BatchGenerateResponse>, (StatusCode, Json<ErrorResponse>)> {
    tracing::info!(
        "Received HTTP batch_generate request with size: {:?}",
        req.size
    );

    // Validate the request using validator
    use validator::Validate;
    if let Err(errors) = req.validate() {
        tracing::warn!("HTTP batch size validation failed: {}", errors);
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(
                400,
                format!("Validation error: {}", errors),
            )),
        ));
    }

    tracing::info!("HTTP batch size validation passed: {:?}", req.size);

    state
        .handlers
        .batch_generate(req)
        .await
        .map(Json)
        .map_err(|e| {
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
        })
}

async fn handle_health(State(state): State<AppState>) -> Json<HealthResponse> {
    Json(state.handlers.health().await)
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

    state.handlers.parse(req).await.map(Json).map_err(|e| {
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
    Json(req): Json<CreateBizTagRequest>,
) -> Result<Json<BizTagResponse>, (StatusCode, Json<ErrorResponse>)> {
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
        Arc::new(ApiKeyAuth::new())
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
