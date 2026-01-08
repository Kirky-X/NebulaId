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

use nebula_core::algorithm::AlgorithmRouter;
use nebula_core::config::Config;
#[cfg(feature = "etcd")]
use nebula_core::coordinator::EtcdClusterHealthMonitor;
use nebula_core::database::{self, ApiKeyRepository};
use nebula_core::types::Result;
use nebula_server::audit::AuditLogger;
use nebula_server::config_hot_reload::HotReloadConfig;
use nebula_server::config_management::ConfigManagementService;
use nebula_server::grpc::GrpcServer;
use nebula_server::handlers::ApiHandlers;
use nebula_server::middleware::size_limit::create_size_limit_middleware;
use nebula_server::middleware::ApiKeyAuth;
use nebula_server::proto::nebula::id::v1::nebula_id_service_server::NebulaIdServiceServer;
use nebula_server::rate_limit::RateLimiter;
use nebula_server::router::create_router;
use nebula_server::tls_server::TlsManager;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tonic::transport::Server;
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

async fn load_api_keys(
    _auth: &Arc<ApiKeyAuth>,
    repository: &Option<Arc<database::SeaOrmRepository>>,
    config: &Config,
) {
    use nebula_core::database::{ApiKeyRole, CreateApiKeyRequest};
    use uuid::Uuid;

    info!("Loading API keys...");

    if let Some(ref repo) = repository {
        // First, create API key from environment variable if configured
        // Format: NEBULA_ADMIN_API_KEY_SECRET=your-secret-key
        let admin_key_secret_from_env = std::env::var("NEBULA_ADMIN_API_KEY_SECRET").ok();

        // Second, create API keys from config if configured
        // Config format: api_keys = [{ key_secret = "xxx", workspace = "global", role = "admin", rate_limit = 100000, name = "Admin" }]
        let configured_keys = if !config.auth.api_keys.is_empty() {
            Some(config.auth.api_keys.clone())
        } else {
            None
        };

        // If configured via env or config, create the key (key_id generated internally)
        if let Some(ref secret) = admin_key_secret_from_env {
            info!("Creating admin API key from environment variable");

            let request = CreateApiKeyRequest {
                workspace_id: None,
                name: "admin".to_string(),
                description: Some("Admin API key from environment".to_string()),
                role: ApiKeyRole::Admin,
                rate_limit: Some(100000),
                expires_at: None,
                key_secret: Some(secret.to_string()),
            };

            match repo.create_api_key(&request).await {
                Ok(key) => {
                    info!("Admin API key created: {}", key.key.key_id);
                }
                Err(e) => {
                    if !e.to_string().contains("duplicate key") {
                        error!("Failed to create admin API key: {}", e);
                    }
                }
            }
        } else if let Some(ref keys) = configured_keys {
            if let Some(first_key) = keys.first() {
                info!("Creating API key from configuration");

                let role = match first_key.role.to_lowercase().as_str() {
                    "admin" => ApiKeyRole::Admin,
                    _ => ApiKeyRole::User,
                };

                let request = CreateApiKeyRequest {
                    workspace_id: if first_key.workspace == "global" {
                        None
                    } else {
                        Some(Uuid::parse_str(&first_key.workspace).unwrap_or(Uuid::nil()))
                    },
                    name: first_key.name.clone(),
                    description: Some(format!("Configured via config, role: {}", first_key.role)),
                    role,
                    rate_limit: Some(first_key.rate_limit as i32),
                    expires_at: None,
                    key_secret: None,
                };

                match repo.create_api_key(&request).await {
                    Ok(key) => {
                        info!(
                            "API key created from config: {} (role: {})",
                            key.key.key_id, first_key.role
                        );
                    }
                    Err(e) => {
                        if !e.to_string().contains("duplicate key") {
                            warn!("Failed to create API key from config: {}", e);
                        }
                    }
                }
            }
        } else {
            // No configuration, check if admin key already exists
            match repo.get_admin_api_key(Uuid::nil()).await {
                Ok(Some(admin_key)) => {
                    info!(
                        "Found existing admin API key: {:?} (workspace: {:?})",
                        admin_key.key_id, admin_key.workspace_id
                    );
                }
                Ok(None) => {
                    // Generate new admin API key
                    let admin_request = CreateApiKeyRequest {
                        workspace_id: None,
                        name: "admin".to_string(),
                        description: Some("Default Admin API Key".to_string()),
                        role: ApiKeyRole::Admin,
                        rate_limit: Some(100000),
                        expires_at: None,
                        key_secret: None,
                    };
                    match repo.create_api_key(&admin_request).await {
                        Ok(key) => {
                            info!(
                                "Admin API key created: {} (workspace: None)",
                                key.key.key_id
                            );
                            // WARN: Print secret to console only once - user must save it
                            println!("\n╔════════════════════════════════════════════════════════════════════╗");
                            println!("║           ⚠️  ADMIN API KEY GENERATED - SAVE NOW!              ║");
                            println!("╠════════════════════════════════════════════════════════════════════╣");
                            println!(
                                "║  Key ID: {}                                                ║",
                                key.key.key_id
                            );
                            println!(
                                "║  Secret: {}                                    ║",
                                key.key_secret
                            );
                            println!("║                                                                    ║");
                            println!("║  ⚠️  THIS IS THE ONLY TIME THE SECRET WILL BE SHOWN!           ║");
                            println!("║  Save it securely - you will need it for API authentication.    ║");
                            println!("╚════════════════════════════════════════════════════════════════════╝\n");
                            tracing::warn!("Admin API key secret printed to console - ensure it is saved securely");
                        }
                        Err(e) => {
                            error!("Failed to create admin API key: {}", e);
                        }
                    }
                }
                Err(e) => {
                    error!("Failed to check admin API key: {}", e);
                }
            }
        }

        // Add test API key in development mode only if no configuration provided
        #[cfg(debug_assertions)]
        if admin_key_secret_from_env.is_none() && configured_keys.is_none() {
            let test_request = CreateApiKeyRequest {
                workspace_id: None,
                name: "Test Admin API Key".to_string(),
                description: Some("Default test admin API key".to_string()),
                role: ApiKeyRole::Admin,
                rate_limit: Some(10000),
                expires_at: None,
                key_secret: None,
            };
            match repo.create_api_key(&test_request).await {
                Ok(key) => {
                    info!("Created test admin API key: {}", key.key.key_id);
                }
                Err(e) => {
                    if !e.to_string().contains("duplicate key") {
                        warn!("Failed to create test API key: {}", e);
                    }
                }
            }
        }
    } else {
        warn!("No database connection, API keys cannot be stored persistently");
    }
}

#[cfg(feature = "etcd")]
async fn create_id_generator(
    config: &Config,
    audit_logger: Arc<AuditLogger>,
    etcd_health_monitor: Option<Arc<EtcdClusterHealthMonitor>>,
) -> Result<Arc<AlgorithmRouter>> {
    info!("Initializing ID generators...");

    let audit_logger_for_core: nebula_core::algorithm::DynAuditLogger =
        audit_logger as Arc<dyn nebula_core::algorithm::AuditLogger>;

    // Create CPU monitor
    let cpu_monitor = Arc::new(nebula_core::algorithm::CpuMonitor::new());
    let router = AlgorithmRouter::new(config.clone(), Some(audit_logger_for_core));

    let router = router.with_cpu_monitor(cpu_monitor);
    let router = if let Some(monitor) = etcd_health_monitor {
        Arc::new(router.with_etcd_health_monitor(monitor))
    } else {
        Arc::new(router)
    };

    router.initialize().await?;

    info!("ID generators initialized successfully");
    Ok(router)
}

#[cfg(not(feature = "etcd"))]
async fn create_id_generator(
    config: &Config,
    audit_logger: Arc<AuditLogger>,
    _etcd_health_monitor: Option<Arc<()>>,
) -> Result<Arc<AlgorithmRouter>> {
    info!("Initializing ID generators (etcd disabled)...");

    let audit_logger_for_core: nebula_core::algorithm::DynAuditLogger =
        audit_logger as Arc<dyn nebula_core::algorithm::AuditLogger>;

    // Create CPU monitor
    let cpu_monitor = Arc::new(nebula_core::algorithm::CpuMonitor::new());
    let router = AlgorithmRouter::new(config.clone(), Some(audit_logger_for_core));
    let router = router.with_cpu_monitor(cpu_monitor);
    let router = Arc::new(router);

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
        .layer(create_size_limit_middleware());

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
    info!("Configured gRPC port: {}", config.grpc_port);

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

    // Load config from file first, then merge with environment variables
    let mut config =
        Config::load_from_file("config/config.toml").unwrap_or_else(|_| Config::default());

    // Apply environment variable overrides
    config.merge(Config::load_from_env().unwrap_or_default());
    info!("Configuration loaded successfully");

    let server_config = ServerConfig {
        http_port: config.app.http_port,
        grpc_port: config.app.grpc_port,
        workers: std::thread::available_parallelism()
            .map(|p| p.get())
            .unwrap_or(1),
        shutdown_timeout_secs: 30,
    };

    info!(
        "Server configuration: HTTP port={}, gRPC port={}",
        server_config.http_port, server_config.grpc_port
    );

    // Initialize database connection first (needed for API key auth)
    info!("Connecting to database...");
    let db_connection = match database::create_connection(&config.database).await {
        Ok(conn) => {
            info!("Database connected successfully");

            // Run auto migrations to create tables
            if let Err(e) = database::run_migrations(&conn).await {
                error!("Failed to run database migrations: {}. Application cannot start without required tables.", e);
                error!("Shutting down...");
                std::process::exit(1);
            }

            Some(conn)
        }
        Err(e) => {
            error!("Failed to connect to database: {}.", e);
            error!(
                "Please check your DATABASE_URL environment variable or database configuration."
            );
            error!("Shutting down...");
            std::process::exit(1);
        }
    };

    let repository: Option<Arc<database::SeaOrmRepository>> = db_connection.map(|conn| {
        let repo = Arc::new(database::SeaOrmRepository::new(conn));
        info!("Database repository initialized");
        repo
    });

    // Create API key auth with repository for database-backed storage
    let auth: Arc<ApiKeyAuth> = if let Some(ref repo) = repository {
        Arc::new(ApiKeyAuth::new(repo.clone()))
    } else {
        error!(
            "FATAL: API key authentication requires database connection.
            Nebula ID requires a database for API key storage and validation.
            Please ensure your configuration has valid database settings:
            - database.url or database.engine/host/port/database/username/password
            - database.max_connections should be > 0
            Shutting down..."
        );
        std::process::exit(1);
    };
    load_api_keys(&auth, &repository, &config).await;

    // Initialize audit logger and config (used by both etcd and non-etcd modes)
    let audit_logger = Arc::new(AuditLogger::new(config.rate_limit.default_rps as usize));
    let hot_config = Arc::new(HotReloadConfig::new(
        config.clone(),
        "config/config.toml".to_string(),
    ));

    #[cfg(feature = "etcd")]
    {
        info!("Initializing etcd cluster health monitor...");
        let etcd_cache_path = format!("./data/etcd_cache_{}.json", config.app.dc_id);
        let etcd_health_monitor = Arc::new(EtcdClusterHealthMonitor::new(
            config.etcd.clone(),
            etcd_cache_path,
        ));

        if let Err(e) = etcd_health_monitor.load_local_cache().await {
            warn!("Failed to load etcd local cache: {}", e);
        }

        info!("Etcd cluster health monitor initialized");

        let id_generator = create_id_generator(
            &config,
            audit_logger.clone(),
            Some(etcd_health_monitor.clone()),
        )
        .await?;

        let (handlers, config_service) = if let Some(ref repo) = repository {
            let cs = Arc::new(ConfigManagementService::with_repository(
                hot_config,
                id_generator.clone(),
                repo.clone(),
                repo.clone(),
                repo.clone(),
            ));
            let h = Arc::new(ApiHandlers::with_api_key_repository(
                id_generator.clone(),
                cs.clone(),
                repo.clone(),
            ));
            (h, cs)
        } else {
            let cs = Arc::new(ConfigManagementService::new(
                hot_config,
                id_generator.clone(),
            ));
            let h = Arc::new(ApiHandlers::new(id_generator.clone(), cs.clone()));
            (h, cs)
        };

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

    #[cfg(not(feature = "etcd"))]
    {
        info!("etcd feature disabled, initializing without etcd cluster health monitor");

        let audit_logger_for_core: nebula_core::algorithm::DynAuditLogger =
            audit_logger.clone() as Arc<dyn nebula_core::algorithm::AuditLogger>;
        let router = AlgorithmRouter::new(config.clone(), Some(audit_logger_for_core));
        let _router = Arc::new(router);

        let id_generator = create_id_generator(&config, audit_logger.clone(), None).await?;

        let (handlers, config_service) = if let Some(ref repo) = repository {
            let cs = Arc::new(ConfigManagementService::with_repository(
                hot_config,
                id_generator.clone(),
                repo.clone(),
                repo.clone(),
                repo.clone(),
            ));
            let h = Arc::new(ApiHandlers::with_api_key_repository(
                id_generator.clone(),
                cs.clone(),
                repo.clone(),
            ));
            (h, cs)
        } else {
            let cs = Arc::new(ConfigManagementService::new(
                hot_config,
                id_generator.clone(),
            ));
            let h = Arc::new(ApiHandlers::new(id_generator.clone(), cs.clone()));
            (h, cs)
        };

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
}

#[cfg(test)]
mod tests {
    use super::*;
    use nebula_core::algorithm::AlgorithmRouter;
    use nebula_core::config::Config;
    use nebula_server::config_hot_reload::HotReloadConfig;
    use std::sync::Arc;

    /// Setup test environment - must be called at the start of each test
    fn setup_test_env() {
        std::env::set_var("NEBULA_DATABASE_PASSWORD", "test_password");
    }

    #[tokio::test]
    async fn test_server_config_default() {
        let config = ServerConfig::default();
        assert_eq!(config.http_port, DEFAULT_SERVER_PORT);
        assert!(config.workers > 0);
        assert_eq!(config.shutdown_timeout_secs, 30);
    }

    #[tokio::test]
    async fn test_create_router_with_handlers() {
        setup_test_env();
        use async_trait::async_trait;

        #[derive(Clone)]
        struct MockApiKeyRepo;

        #[async_trait]
        impl database::ApiKeyRepository for MockApiKeyRepo {
            async fn create_api_key(
                &self,
                _request: &database::CreateApiKeyRequest,
            ) -> nebula_core::types::Result<database::ApiKeyWithSecret> {
                Ok(database::ApiKeyWithSecret {
                    key: database::ApiKeyResponse {
                        id: uuid::Uuid::new_v4(),
                        key_id: "mock_key_id".to_string(),
                        key_prefix: "nino_".to_string(),
                        name: "Mock Key".to_string(),
                        description: None,
                        role: database::ApiKeyRole::User,
                        rate_limit: 10000,
                        enabled: true,
                        expires_at: None,
                        created_at: chrono::Utc::now().naive_utc(),
                    },
                    key_secret: "mock_secret".to_string(),
                })
            }

            async fn get_api_key_by_id(
                &self,
                _key_id: &str,
            ) -> nebula_core::types::Result<Option<database::ApiKeyInfo>> {
                Ok(None)
            }

            async fn validate_api_key(
                &self,
                _key_id: &str,
                _key_secret: &str,
            ) -> nebula_core::types::Result<Option<(Option<uuid::Uuid>, database::ApiKeyRole)>>
            {
                Ok(None)
            }

            async fn list_api_keys(
                &self,
                _workspace_id: uuid::Uuid,
                _limit: Option<u32>,
                _offset: Option<u32>,
            ) -> nebula_core::types::Result<Vec<database::ApiKeyInfo>> {
                Ok(vec![])
            }

            async fn delete_api_key(&self, _id: uuid::Uuid) -> nebula_core::types::Result<()> {
                Ok(())
            }

            async fn revoke_api_key(&self, _id: uuid::Uuid) -> nebula_core::types::Result<()> {
                Ok(())
            }

            async fn update_last_used(&self, _key: uuid::Uuid) -> nebula_core::types::Result<()> {
                Ok(())
            }

            async fn get_admin_api_key(
                &self,
                _workspace_id: uuid::Uuid,
            ) -> nebula_core::types::Result<Option<database::ApiKeyInfo>> {
                Ok(None)
            }

            async fn count_api_keys(
                &self,
                _workspace_id: uuid::Uuid,
            ) -> nebula_core::types::Result<u64> {
                Ok(0)
            }

            async fn rotate_api_key(
                &self,
                _key_id: &str,
                _grace_period_seconds: u64,
            ) -> nebula_core::types::Result<database::ApiKeyWithSecret> {
                Ok(database::ApiKeyWithSecret {
                    key: database::ApiKeyResponse {
                        id: uuid::Uuid::new_v4(),
                        key_id: "mock_rotated_key_id".to_string(),
                        key_prefix: "nino_".to_string(),
                        name: "Mock Rotated Key".to_string(),
                        description: None,
                        role: database::ApiKeyRole::User,
                        rate_limit: 10000,
                        enabled: true,
                        expires_at: None,
                        created_at: chrono::Utc::now().naive_utc(),
                    },
                    key_secret: "mock_rotated_secret".to_string(),
                })
            }

            async fn get_keys_older_than(
                &self,
                _age_threshold_days: i64,
            ) -> nebula_core::types::Result<Vec<database::ApiKeyInfo>> {
                Ok(vec![])
            }
        }

        let config = Config::default();
        let audit_logger: Arc<dyn nebula_core::algorithm::AuditLogger> =
            Arc::new(AuditLogger::new(10000));
        let router = AlgorithmRouter::new(config.clone(), Some(audit_logger));
        let router = Arc::new(router);
        let hot_config = Arc::new(HotReloadConfig::new(
            config.clone(),
            "config/config.toml".to_string(),
        ));
        let config_service = Arc::new(ConfigManagementService::new(hot_config, router.clone()));
        let handlers = Arc::new(ApiHandlers::with_api_key_repository(
            router,
            config_service,
            Arc::new(MockApiKeyRepo),
        ));
        let auth = Arc::new(ApiKeyAuth::new(Arc::new(MockApiKeyRepo)));
        let rate_limiter = Arc::new(RateLimiter::new(10000, 100));
        let audit_logger = Arc::new(AuditLogger::new(10000));

        let _router = create_router(handlers, auth, rate_limiter, audit_logger).await;
    }

    #[tokio::test]
    async fn test_graceful_shutdown() {
        setup_test_env();
        use async_trait::async_trait;

        #[derive(Clone)]
        struct MockApiKeyRepo;

        #[async_trait]
        impl database::ApiKeyRepository for MockApiKeyRepo {
            async fn create_api_key(
                &self,
                _request: &database::CreateApiKeyRequest,
            ) -> nebula_core::types::Result<database::ApiKeyWithSecret> {
                Ok(database::ApiKeyWithSecret {
                    key: database::ApiKeyResponse {
                        id: uuid::Uuid::new_v4(),
                        key_id: "mock_key_id".to_string(),
                        key_prefix: "nino_".to_string(),
                        name: "Mock Key".to_string(),
                        description: None,
                        role: database::ApiKeyRole::User,
                        rate_limit: 10000,
                        enabled: true,
                        expires_at: None,
                        created_at: chrono::Utc::now().naive_utc(),
                    },
                    key_secret: "mock_secret".to_string(),
                })
            }

            async fn get_api_key_by_id(
                &self,
                _key_id: &str,
            ) -> nebula_core::types::Result<Option<database::ApiKeyInfo>> {
                Ok(None)
            }

            async fn validate_api_key(
                &self,
                _key_id: &str,
                _key_secret: &str,
            ) -> nebula_core::types::Result<Option<(Option<uuid::Uuid>, database::ApiKeyRole)>>
            {
                Ok(None)
            }

            async fn list_api_keys(
                &self,
                _workspace_id: uuid::Uuid,
                _limit: Option<u32>,
                _offset: Option<u32>,
            ) -> nebula_core::types::Result<Vec<database::ApiKeyInfo>> {
                Ok(vec![])
            }

            async fn delete_api_key(&self, _id: uuid::Uuid) -> nebula_core::types::Result<()> {
                Ok(())
            }

            async fn revoke_api_key(&self, _id: uuid::Uuid) -> nebula_core::types::Result<()> {
                Ok(())
            }

            async fn update_last_used(&self, _key: uuid::Uuid) -> nebula_core::types::Result<()> {
                Ok(())
            }

            async fn get_admin_api_key(
                &self,
                _workspace_id: uuid::Uuid,
            ) -> nebula_core::types::Result<Option<database::ApiKeyInfo>> {
                Ok(None)
            }

            async fn count_api_keys(
                &self,
                _workspace_id: uuid::Uuid,
            ) -> nebula_core::types::Result<u64> {
                Ok(0)
            }

            async fn rotate_api_key(
                &self,
                _key_id: &str,
                _grace_period_seconds: u64,
            ) -> nebula_core::types::Result<database::ApiKeyWithSecret> {
                Ok(database::ApiKeyWithSecret {
                    key: database::ApiKeyResponse {
                        id: uuid::Uuid::new_v4(),
                        key_id: "mock_rotated_key_id".to_string(),
                        key_prefix: "nino_".to_string(),
                        name: "Mock Rotated Key".to_string(),
                        description: None,
                        role: database::ApiKeyRole::User,
                        rate_limit: 10000,
                        enabled: true,
                        expires_at: None,
                        created_at: chrono::Utc::now().naive_utc(),
                    },
                    key_secret: "mock_rotated_secret".to_string(),
                })
            }

            async fn get_keys_older_than(
                &self,
                _age_threshold_days: i64,
            ) -> nebula_core::types::Result<Vec<database::ApiKeyInfo>> {
                Ok(vec![])
            }
        }

        let config = Config::default();
        let audit_logger: Arc<dyn nebula_core::algorithm::AuditLogger> =
            Arc::new(AuditLogger::new(10000));
        let router = AlgorithmRouter::new(config.clone(), Some(audit_logger));
        let router = Arc::new(router);
        let hot_config = Arc::new(HotReloadConfig::new(
            config.clone(),
            "config/config.toml".to_string(),
        ));
        let config_service = Arc::new(ConfigManagementService::new(hot_config, router.clone()));
        let handlers = Arc::new(ApiHandlers::with_api_key_repository(
            router,
            config_service.clone(),
            Arc::new(MockApiKeyRepo),
        ));
        let auth = Arc::new(ApiKeyAuth::new(Arc::new(MockApiKeyRepo)));
        let rate_limiter = Arc::new(RateLimiter::new(10000, 100));
        let audit_logger = Arc::new(AuditLogger::new(10000));

        let server_config = ServerConfig {
            http_port: 0,
            grpc_port: 0,
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
