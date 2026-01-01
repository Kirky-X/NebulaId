use nebula_core::algorithm::{AlgorithmRouter, IdGenerator};
use nebula_core::config::Config;
use nebula_core::coordinator::EtcdClusterHealthMonitor;
use nebula_core::types::Result;
use nebula_server::audit::AuditLogger;
use nebula_server::config_hot_reload::HotReloadConfig;
use nebula_server::config_management::ConfigManagementService;
use nebula_server::grpc::nebula_id::nebula_id_service_server::NebulaIdServiceServer;
use nebula_server::grpc::GrpcServer;
use nebula_server::handlers::ApiHandlers;
use nebula_server::middleware::ApiKeyAuth;
use nebula_server::rate_limit::RateLimiter;
use nebula_server::router::create_router;
use nebula_server::tls_server::TlsManager;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tonic::transport::Server;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::timeout::TimeoutLayer;
use tracing::warn;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

const DEFAULT_SERVER_PORT: u16 = 8080;
const DEFAULT_GRPC_PORT: u16 = 50051;

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub http_port: u16,
    pub grpc_port: u16,
    pub workers: usize,
    pub shutdown_timeout_secs: u64,
}

impl Default for ServerConfig {
    fn default() -> Self {
        let workers = std::thread::available_parallelism()
            .map(|p| p.get())
            .unwrap_or(1);
        Self {
            http_port: DEFAULT_SERVER_PORT,
            grpc_port: DEFAULT_GRPC_PORT,
            workers,
            shutdown_timeout_secs: 30,
        }
    }
}

async fn load_api_keys(_auth: &Arc<ApiKeyAuth>) {
    info!("Loading API keys from configuration...");
}

async fn create_id_generator(
    config: &Config,
    audit_logger: Arc<AuditLogger>,
    etcd_health_monitor: Option<Arc<EtcdClusterHealthMonitor>>,
) -> Result<Arc<dyn IdGenerator>> {
    info!("Initializing ID generators...");

    let audit_logger_for_core: nebula_core::algorithm::DynAuditLogger =
        audit_logger as Arc<dyn nebula_core::algorithm::AuditLogger>;
    let router = AlgorithmRouter::new(config.clone(), Some(audit_logger_for_core));

    let router = if let Some(monitor) = etcd_health_monitor {
        Arc::new(router.with_etcd_health_monitor(monitor))
    } else {
        Arc::new(router)
    };

    router.initialize().await?;

    info!("ID generators initialized successfully");
    Ok(router)
}

async fn start_http_server(
    _config: ServerConfig,
    handlers: Arc<ApiHandlers>,
    auth: Arc<ApiKeyAuth>,
    rate_limiter: Arc<RateLimiter>,
    audit_logger: Arc<AuditLogger>,
    _config_service: Arc<ConfigManagementService>,
    tls_manager: Option<Arc<TlsManager>>,
) -> Result<()> {
    let addr = SocketAddr::from(([0, 0, 0, 0], DEFAULT_SERVER_PORT));

    let router = create_router(handlers, auth, rate_limiter, audit_logger)
        .await
        .layer(TimeoutLayer::new(std::time::Duration::from_secs(30)))
        .layer(RequestBodyLimitLayer::new(1024 * 1024));

    // 检查是否启用 HTTPS (暂时回退到普通 HTTP，TLS 功能待完善)
    if let Some(ref tls) = tls_manager {
        if tls.is_http_enabled() {
            info!("HTTPS is enabled but using HTTP fallback for now");
        }
    }

    // 回退到普通 HTTP
    info!("Starting HTTP server on {}", addr);
    let listener = TcpListener::bind(addr).await?;

    axum::serve(listener, router)
        .with_graceful_shutdown(async {
            tokio::signal::ctrl_c().await.ok();
            info!("Shutting down HTTP server...");
        })
        .await?;

    Ok(())
}

async fn start_grpc_server(
    config: ServerConfig,
    handlers: Arc<ApiHandlers>,
    tls_manager: Option<Arc<TlsManager>>,
) -> Result<()> {
    let grpc_addr = SocketAddr::from(([0, 0, 0, 0], config.grpc_port));
    info!("Starting gRPC server on {}", grpc_addr);

    let grpc_server = GrpcServer::new(handlers);

    let shutdown = async {
        tokio::signal::ctrl_c().await.ok();
        info!("Shutting down gRPC server...");
    };

    let mut server_builder = Server::builder();

    if let Some(ref tls) = tls_manager {
        if tls.is_grpc_enabled() {
            info!("gRPC TLS is enabled, using secure connection");
            if let Some(grpc_tls_config) = tls.grpc_tls_config() {
                let config = grpc_tls_config.as_ref().clone();
                server_builder = server_builder.tls_config(config).map_err(|e| {
                    nebula_core::types::CoreError::InternalError(format!("TLS config error: {}", e))
                })?;
            }
        }
    }

    server_builder
        .add_service(NebulaIdServiceServer::new(grpc_server))
        .serve_with_shutdown(grpc_addr, shutdown)
        .await
        .map_err(|e| {
            nebula_core::types::CoreError::InternalError(format!("gRPC server error: {}", e))
        })?;

    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("Failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_target(false)
        .with_env_filter(EnvFilter::from_default_env())
        .json()
        .init();

    info!("Starting Nebula ID Generation Service");
    info!("Version: {}", env!("CARGO_PKG_VERSION"));

    let config = Config::load_from_file("config.toml")
        .or_else(|_| Config::load_from_env())
        .unwrap_or_else(|_| Config::default());
    info!("Configuration loaded successfully");

    let server_config = ServerConfig {
        http_port: config.app.http_port,
        grpc_port: config.app.grpc_port,
        workers: std::thread::available_parallelism()
            .map(|p| p.get())
            .unwrap_or(1),
        shutdown_timeout_secs: 30,
    };

    let auth = Arc::new(ApiKeyAuth::new());
    load_api_keys(&auth).await;

    let hot_config = Arc::new(HotReloadConfig::new(
        config.clone(),
        "config.toml".to_string(),
    ));
    let config_service = Arc::new(ConfigManagementService::new(hot_config));

    let audit_logger = Arc::new(AuditLogger::new(10000));

    info!("Initializing etcd cluster health monitor...");
    let etcd_cache_path = format!("./data/etcd_cache_{}.json", config.app.dc_id);
    let etcd_health_monitor = Arc::new(EtcdClusterHealthMonitor::new(
        config.etcd.clone(),
        etcd_cache_path,
    ));

    if let Err(e) = etcd_health_monitor.load_local_cache().await {
        warn!("Failed to load etcd local cache: {}", e);
    }

    // TODO: Start health check and cache persistence in background
    // tokio::spawn(etcd_health_monitor.start_health_check(std::time::Duration::from_secs(10)));
    // tokio::spawn(etcd_health_monitor.start_cache_persistence(std::time::Duration::from_secs(60)));
    info!("Etcd cluster health monitor initialized");

    let id_generator = create_id_generator(
        &config,
        audit_logger.clone(),
        Some(etcd_health_monitor.clone()),
    )
    .await?;
    let handlers = Arc::new(ApiHandlers::new(
        id_generator.clone(),
        config_service.clone(),
    ));

    let rate_limiter = Arc::new(RateLimiter::new(
        config.rate_limit.default_rps,
        config.rate_limit.burst_size,
    ));

    let mut tls_manager = TlsManager::new(config.tls.clone());
    if let Err(e) = tls_manager.initialize().await {
        error!("Failed to initialize TLS manager: {}", e);
        info!("TLS will be disabled");
    }
    let tls_manager = if tls_manager.is_http_enabled() || tls_manager.is_grpc_enabled() {
        Some(Arc::new(tls_manager))
    } else {
        None
    };

    info!("Starting degradation manager health check task...");
    let degradation_manager = id_generator.get_degradation_manager();
    degradation_manager.start_background_check();

    info!("Server initialized, starting HTTP and gRPC servers...");

    let http_server = tokio::spawn(start_http_server(
        server_config.clone(),
        handlers.clone(),
        auth.clone(),
        rate_limiter.clone(),
        audit_logger.clone(),
        config_service.clone(),
        tls_manager.clone(),
    ));
    let grpc_server = tokio::spawn(start_grpc_server(server_config, handlers, tls_manager));

    tokio::select! {
        http_result = http_server => {
            match http_result {
                Ok(Ok(())) => info!("HTTP server stopped"),
                Ok(Err(e)) => {
                    error!("HTTP server error: {}", e);
                    return Err(e);
                }
                Err(e) => {
                    error!("HTTP server task panicked: {}", e);
                    return Err(nebula_core::types::CoreError::InternalError(format!("HTTP server panic: {}", e)));
                }
            }
        }
        grpc_result = grpc_server => {
            match grpc_result {
                Ok(Ok(())) => info!("gRPC server stopped"),
                Ok(Err(e)) => {
                    error!("gRPC server error: {}", e);
                    return Err(e);
                }
                Err(e) => {
                    error!("gRPC server task panicked: {}", e);
                    return Err(nebula_core::types::CoreError::InternalError(format!("gRPC server panic: {}", e)));
                }
            }
        }
        _ = shutdown_signal() => {
            info!("Shutdown signal received");
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nebula_core::algorithm::AlgorithmRouter;
    use nebula_core::config::Config;
    use nebula_server::config_hot_reload::HotReloadConfig;
    use std::sync::Arc;

    #[tokio::test]
    async fn test_server_config_default() {
        let config = ServerConfig::default();
        assert_eq!(config.http_port, DEFAULT_SERVER_PORT);
        assert!(config.workers > 0);
        assert_eq!(config.shutdown_timeout_secs, 30);
    }

    #[tokio::test]
    async fn test_create_router_with_handlers() {
        let config = Config::default();
        let audit_logger: Arc<dyn nebula_core::algorithm::AuditLogger> =
            Arc::new(AuditLogger::new(10000));
        let router = AlgorithmRouter::new(config.clone(), Some(audit_logger));
        let hot_config = Arc::new(HotReloadConfig::new(
            config.clone(),
            "config.toml".to_string(),
        ));
        let config_service = Arc::new(ConfigManagementService::new(hot_config));
        let handlers = Arc::new(ApiHandlers::new(Arc::new(router), config_service));
        let auth = Arc::new(ApiKeyAuth::new());
        let rate_limiter = Arc::new(RateLimiter::new(10000, 100));
        let audit_logger = Arc::new(AuditLogger::new(10000));

        let _router = create_router(handlers, auth, rate_limiter, audit_logger).await;
    }

    #[tokio::test]
    async fn test_graceful_shutdown() {
        let config = Config::default();
        let audit_logger: Arc<dyn nebula_core::algorithm::AuditLogger> =
            Arc::new(AuditLogger::new(10000));
        let router = AlgorithmRouter::new(config.clone(), Some(audit_logger));
        let hot_config = Arc::new(HotReloadConfig::new(
            config.clone(),
            "config.toml".to_string(),
        ));
        let config_service = Arc::new(ConfigManagementService::new(hot_config));
        let handlers = Arc::new(ApiHandlers::new(Arc::new(router), config_service.clone()));
        let auth = Arc::new(ApiKeyAuth::new());
        let rate_limiter = Arc::new(RateLimiter::new(10000, 100));
        let audit_logger = Arc::new(AuditLogger::new(10000));

        let server_config = ServerConfig {
            http_port: 0,
            workers: 1,
            shutdown_timeout_secs: 1,
        };

        let server = tokio::spawn(async move {
            start_http_server(
                server_config,
                handlers,
                auth,
                rate_limiter,
                audit_logger,
                config_service,
                None,
            )
            .await
        });

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), shutdown_signal()).await;

        server.abort();
    }
}
