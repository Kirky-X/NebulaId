use crate::audit::AuditLogger;
use crate::audit_middleware::AuditMiddleware;
use crate::handlers::ApiHandlers;
use crate::middleware::ApiKeyAuth;
use crate::models::{
    BatchGenerateRequest, BatchGenerateResponse, ErrorResponse, GenerateRequest, GenerateResponse,
    HealthResponse, MetricsResponse, ParseRequest, ParseResponse,
};
use crate::rate_limit::RateLimiter;
use crate::rate_limit_middleware::RateLimitMiddleware;
use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};

pub async fn create_router(
    handlers: Arc<ApiHandlers>,
    auth: Arc<ApiKeyAuth>,
    rate_limiter: Arc<RateLimiter>,
    audit_logger: Arc<AuditLogger>,
) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let rate_limit_middleware = RateLimitMiddleware::new(rate_limiter.clone());
    let audit_middleware = AuditMiddleware::new(audit_logger.clone(), auth.clone(), rate_limiter);

    let router = Router::new()
        .route("/api/v1/generate", post(handle_generate))
        .route("/api/v1/generate/batch", post(handle_batch_generate))
        .route("/api/v1/parse", post(handle_parse))
        .route("/metrics", get(handle_metrics))
        .route("/health", get(handle_health))
        .with_state(handlers)
        .with_state(auth.clone())
        .layer(cors)
        .layer(axum::Extension(rate_limit_middleware))
        .layer(axum::Extension(audit_middleware))
        .layer(axum::Extension(audit_logger));

    router
}

async fn handle_generate(
    State(handlers): State<Arc<ApiHandlers>>,
    Json(req): Json<GenerateRequest>,
) -> Result<Json<GenerateResponse>, (StatusCode, Json<ErrorResponse>)> {
    handlers.generate(req).await.map(Json).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(500, e.to_string())),
        )
    })
}

async fn handle_batch_generate(
    State(handlers): State<Arc<ApiHandlers>>,
    Json(req): Json<BatchGenerateRequest>,
) -> Result<Json<BatchGenerateResponse>, (StatusCode, Json<ErrorResponse>)> {
    handlers.batch_generate(req).await.map(Json).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(500, e.to_string())),
        )
    })
}

async fn handle_health(State(handlers): State<Arc<ApiHandlers>>) -> Json<HealthResponse> {
    Json(handlers.health().await)
}

async fn handle_metrics(State(handlers): State<Arc<ApiHandlers>>) -> Json<MetricsResponse> {
    Json(handlers.metrics().await)
}

async fn handle_parse(
    State(handlers): State<Arc<ApiHandlers>>,
    Json(req): Json<ParseRequest>,
) -> Result<Json<ParseResponse>, (StatusCode, Json<ErrorResponse>)> {
    handlers.parse(req).await.map(Json).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(400, e.to_string())),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nebula_core::algorithm::AlgorithmRouter;
    use nebula_core::config::Config;
    use std::sync::Arc;

    fn create_test_api_handlers() -> Arc<ApiHandlers> {
        let config = Config::default();
        let algorithm_router = Arc::new(AlgorithmRouter::new(config));
        let handlers = ApiHandlers::new(algorithm_router);
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
