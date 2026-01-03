use axum::{body::Body as AxumBody, Router};
use nebula_core::{
    algorithm::AlgorithmRouter,
    config::Config,
};
use nebula_server::{
    create_router,
    handlers::ApiHandlers,
    AuditLogger,
    config_management::ConfigManagementService,
    config_hot_reload::HotReloadConfig,
    middleware::ApiKeyAuth,
    rate_limit::RateLimiter,
};
use std::sync::Arc;
use tokio::sync::OnceCell;
use tower::ServiceExt;
use vercel_runtime::{run, Body, Error, Request, Response};

static APP: OnceCell<Router> = OnceCell::const_new();

async fn get_app() -> Result<&'static Router, Error> {
    APP.get_or_try_init(|| async {
        // 1. Load Configuration
        // On Vercel, config is loaded from environment variables.
        let config = Config::load_from_env().unwrap_or_else(|e| {
            eprintln!("Failed to load config from env: {}, using default", e);
            Config::default()
        });

        // 2. Initialize Dependencies
        
        // Audit Logger
        let audit_logger = Arc::new(AuditLogger::new(1000));
        
        // Rate Limiter (using default values or from config if available)
        let rate_limiter = Arc::new(RateLimiter::new(
            config.rate_limit.default_rps as u32, 
            config.rate_limit.burst_size as u32
        ));

        // Auth
        let auth = Arc::new(ApiKeyAuth::new());
        // In a real scenario, we would load keys here.
        // For Vercel, maybe load from a secret env var?
        // auth.load_key(...)

        // 3. Initialize Core Algorithm Router
        // We cast the audit logger to the trait object required by core
        let audit_logger_for_core = audit_logger.clone() as Arc<dyn nebula_core::algorithm::AuditLogger>;
        
        // Create AlgorithmRouter
        let router_algo = AlgorithmRouter::new(config.clone(), Some(audit_logger_for_core));
        let router_algo = Arc::new(router_algo);
        
        // Initialize the router (connects to DB/Redis if configured)
        // This might fail if DB is not reachable.
        router_algo.initialize().await.map_err(|e| Error::from(format!("Failed to initialize algorithm router: {}", e)))?;

        // 4. Initialize Config Service
        // We pass a dummy path for file config since we rely on env vars in Vercel
        let hot_config = Arc::new(HotReloadConfig::new(
            config.clone(),
            "/tmp/config.toml".to_string(),
        ));
        
        let config_service = Arc::new(ConfigManagementService::new(
            hot_config,
            router_algo.clone(),
        ));

        // 5. Create Handlers
        let handlers = Arc::new(ApiHandlers::new(router_algo, config_service));

        // 6. Create Axum Router
        let app = create_router(handlers, auth, rate_limiter, audit_logger).await;

        Ok(app)
    }).await
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    run(handler).await
}

pub async fn handler(req: Request) -> Result<Response<Body>, Error> {
    let app = get_app().await?;

    // Convert Vercel Request to Axum Request
    let (parts, body) = req.into_parts();
    let body_bytes = match body {
        Body::Text(s) => s.into_bytes(),
        Body::Binary(b) => b,
        Body::Empty => vec![],
    };
    
    let axum_req = http::Request::from_parts(parts, AxumBody::from(body_bytes));

    // Call the Axum App
    let resp = app.clone().oneshot(axum_req).await.map_err(|e| Error::from(format!("Request failed: {}", e)))?;

    // Convert Axum Response to Vercel Response
    let (parts, body) = resp.into_parts();
    
    // We need to read the full body
    let bytes = axum::body::to_bytes(body, usize::MAX).await.map_err(|e| Error::from(format!("Failed to read response body: {}", e)))?;
    
    Ok(http::Response::from_parts(parts, Body::Binary(bytes.to_vec())))
}
