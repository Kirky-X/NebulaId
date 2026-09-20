// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

use nebulaid::core::algorithm::AlgorithmRouter;
use nebulaid::core::config::{resolve_startup_config, Config, Environment, StartupConfig};
// EtcdClientWrapper 仅在 assemble_coordination 内部经局部 use 引入。
#[cfg(feature = "etcd")]
use nebulaid::core::coordinator::{EtcdClusterHealthMonitor, WorkerIdAllocator};
use nebulaid::core::database::{self, ApiKeyRepository};
use nebulaid::core::types::Result;
use nebulaid::server::audit::AuditLogger;
use nebulaid::server::config::hot_reload::HotReloadConfig;
use nebulaid::server::config::management::{ConfigManagementService, ConfigManager};
use nebulaid::server::config::tls::{DualListener, PeerAddr, TlsManager};
use nebulaid::server::grpc::GrpcServer;
use nebulaid::server::handlers::ApiHandlers;
use nebulaid::server::middleware::size_limit::create_size_limit_middleware;
use nebulaid::server::middleware::ApiKeyAuth;
use nebulaid::server::proto::nebula::id::v1::nebula_id_service_server::NebulaIdServiceServer;
use nebulaid::server::rate_limit::limiter::RateLimiter;
use nebulaid::server::router::create_router_with_rate_limit;
use nebulaid::server::sdforge_adapter::{init_sdforge, merge_sdforge_routes};
use sdforge::tonic::transport::Server;
use std::env;
use std::net::SocketAddr;
use std::sync::Arc;
use tracing::warn;
use tracing::{error, info};

// unify-rust-i18n — bin 侧宏引入:t! 经 nebulaid 的 #[macro_export] 挂在
// lib crate 根,`#[macro_use] extern crate` 把它拉入 bin crate 的文本作用域
// (替换原 rust-i18n 的 `#[macro_use] extern crate rust_i18n;` + `i18n!`)。
// FTL 资源由 lib 侧 core::i18n 编译期内嵌,bin 无需再独立加载。
#[macro_use]
extern crate nebulaid;

const DEFAULT_CONFIG_PATH: &str = "config/config.toml";

/// 服务器配置
///
/// 端口默认值唯一归属 `core::config::NebulaIdConfig::default()`（8080/9091），
/// 此处经 `NebulaIdConfig::default()` 派生，禁止再引入本地端口常量。
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// HTTP 服务端口
    pub http_port: u16,
    /// gRPC 服务端口
    pub grpc_port: u16,
    /// 工作线程数
    pub workers: usize,
    /// 关闭超时时间（秒）
    pub shutdown_timeout_secs: u64,
}

impl Default for ServerConfig {
    fn default() -> Self {
        let app = nebulaid::core::config::NebulaIdConfig::default();
        let workers = std::thread::available_parallelism()
            .map(|p| p.get())
            .unwrap_or(1);
        Self {
            http_port: app.http_port,
            grpc_port: app.grpc_port,
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
    use nebulaid::core::database::{ApiKeyRole, CreateApiKeyRequest};
    use uuid::Uuid;

    info!("{}", t!("log.main.loading_api_keys"));

    if let Some(ref repo) = repository {
        // First, create API key from environment variable if configured
        // Format: NEBULA_ADMIN_API_KEY_SECRET=your-secret-key
        let admin_key_secret_from_env = std::env::var("NEBULA_ADMIN_API_KEY_SECRET").ok();
        let admin_key_id_from_env = std::env::var("NEBULA_ADMIN_API_KEY_ID").ok();

        // Second, create API keys from config if configured
        // Config format: api_keys = [{ key_secret = "xxx", workspace = "global", role = "admin", rate_limit = 100000, name = "Admin" }]
        let configured_keys = if !config.auth.api_keys.is_empty() {
            Some(config.auth.api_keys.clone())
        } else {
            None
        };

        // If configured via env or config, create the key (key_id generated internally)
        if let Some(ref secret) = admin_key_secret_from_env {
            info!("{}", t!("log.main.creating_admin_api_key_from_env"));

            let request = CreateApiKeyRequest {
                workspace_id: None,
                name: "admin".to_string(),
                description: Some("Admin API key from environment".to_string()),
                role: ApiKeyRole::Admin,
                rate_limit: Some(100000),
                expires_at: None,
                key_secret: Some(secret.to_string()),
                key_id: admin_key_id_from_env,
            };

            match repo.create_api_key(&request).await {
                Ok(key) => {
                    info!(
                        "{}",
                        t!("log.main.admin_api_key_created", key_id = key.key.key_id)
                    );
                }
                Err(e) => {
                    if !e.to_string().contains("duplicate key") {
                        error!("{}", t!("log.main.admin_api_key_create_failed", error = e));
                    }
                }
            }
        } else if let Some(ref keys) = configured_keys {
            if let Some(first_key) = keys.first() {
                info!("{}", t!("log.main.creating_api_key_from_config"));

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
                    key_secret: Some(first_key.key_secret.clone()),
                    key_id: Some(first_key.key_id.clone()),
                };

                match repo.create_api_key(&request).await {
                    Ok(key) => {
                        info!(
                            "{}",
                            t!(
                                "log.main.api_key_created_from_config",
                                key_id = key.key.key_id,
                                role = first_key.role
                            )
                        );
                    }
                    Err(e) => {
                        if !e.to_string().contains("duplicate key") {
                            warn!(
                                "{}",
                                t!("log.main.api_key_create_from_config_failed", error = e)
                            );
                        }
                    }
                }
            }
        } else {
            // No configuration, check if admin key already exists
            match repo.get_admin_api_key(Uuid::nil()).await {
                Ok(Some(admin_key)) => {
                    info!(
                        key_id = ?admin_key.key_id,
                        workspace_id = ?admin_key.workspace_id,
                        "{}",
                        t!("log.main.existing_admin_api_key_found")
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
                        key_id: None,
                    };
                    match repo.create_api_key(&admin_request).await {
                        Ok(key) => {
                            info!(
                                "{}",
                                t!(
                                    "log.main.admin_api_key_created_no_workspace",
                                    key_id = key.key.key_id
                                )
                            );

                            let is_production = nebulaid::core::config::is_production();

                            if !is_production {
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
                                tracing::warn!("{}", t!("log.main.admin_api_key_secret_printed"));
                            } else {
                                tracing::warn!(
                                    "{}",
                                    t!("log.main.admin_api_key_generated_no_print")
                                );
                            }
                        }
                        Err(e) => {
                            error!("{}", t!("log.main.admin_api_key_create_failed", error = e));
                        }
                    }
                }
                Err(e) => {
                    error!("{}", t!("log.main.admin_api_key_check_failed", error = e));
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
                key_id: None,
            };
            match repo.create_api_key(&test_request).await {
                Ok(key) => {
                    info!(
                        "{}",
                        t!(
                            "log.main.test_admin_api_key_created",
                            key_id = key.key.key_id
                        )
                    );
                }
                Err(e) => {
                    if !e.to_string().contains("duplicate key") {
                        warn!("{}", t!("log.main.test_api_key_create_failed", error = e));
                    }
                }
            }
        }
    } else {
        warn!("{}", t!("log.main.no_database_connection"));
    }
}

#[cfg(feature = "etcd")]
async fn create_id_generator(
    config: &Config,
    audit_logger: Arc<AuditLogger>,
    etcd_health_monitor: Option<Arc<EtcdClusterHealthMonitor>>,
) -> Result<Arc<AlgorithmRouter>> {
    info!("{}", t!("log.main.id_generators_initializing"));

    let audit_logger_for_core: nebulaid::core::algorithm::DynAuditLogger =
        audit_logger as Arc<dyn nebulaid::core::algorithm::AuditLogger>;

    // Create CPU monitor
    let cpu_monitor = Arc::new(nebulaid::core::algorithm::CpuMonitor::new());
    // 启动 CPU 周期采样（仅 Linux：读取 /proc/stat）。任务 detached，
    // 只要进程存活即持续更新共享的 current_usage，供 Segment 动态步长使用。
    #[cfg(target_os = "linux")]
    let _cpu_sampler = cpu_monitor.start_monitoring();
    let router = AlgorithmRouter::new(config.clone(), Some(audit_logger_for_core));

    let router = router.with_cpu_monitor(cpu_monitor);
    let router = if let Some(monitor) = etcd_health_monitor {
        Arc::new(router.with_etcd_health_monitor(monitor))
    } else {
        Arc::new(router)
    };

    router.initialize().await?;

    info!("{}", t!("log.main.id_generators_initialized"));
    Ok(router)
}

#[cfg(not(feature = "etcd"))]
async fn create_id_generator(
    config: &Config,
    audit_logger: Arc<AuditLogger>,
    _etcd_health_monitor: Option<Arc<()>>,
) -> Result<Arc<AlgorithmRouter>> {
    info!(
        "{}",
        t!("log.main.id_generators_initializing_etcd_disabled")
    );

    let audit_logger_for_core: nebulaid::core::algorithm::DynAuditLogger =
        audit_logger as Arc<dyn nebulaid::core::algorithm::AuditLogger>;

    // Create CPU monitor
    let cpu_monitor = Arc::new(nebulaid::core::algorithm::CpuMonitor::new());
    // 启动 CPU 周期采样（仅 Linux：读取 /proc/stat）。任务 detached，
    // 只要进程存活即持续更新共享的 current_usage，供 Segment 动态步长使用。
    #[cfg(target_os = "linux")]
    let _cpu_sampler = cpu_monitor.start_monitoring();
    let router = AlgorithmRouter::new(config.clone(), Some(audit_logger_for_core));
    let router = router.with_cpu_monitor(cpu_monitor);
    let router = Arc::new(router);

    router.initialize().await?;

    info!("{}", t!("log.main.id_generators_initialized"));
    Ok(router)
}

async fn start_http_server(
    bind_addr: SocketAddr,
    handlers: Arc<ApiHandlers>,
    auth: Arc<ApiKeyAuth>,
    rate_limiter: Arc<RateLimiter>,
    audit_logger: Arc<AuditLogger>,
    config_service: Arc<dyn ConfigManagementService>,
    tls_manager: Option<Arc<TlsManager>>,
) -> Result<()> {
    // 限流开关来自真实配置（此前该参数被忽略，
    // `[rate_limit].enabled=false` 也照样挂限流层）。
    let rate_limit_enabled = config_service.get_config().rate_limit.enabled;
    let router = create_router_with_rate_limit(
        handlers,
        auth,
        rate_limiter,
        audit_logger,
        rate_limit_enabled,
    )
    .await
    .layer(create_size_limit_middleware())
    .merge(merge_sdforge_routes(axum::Router::new()));

    // 按配置选择明文或 TLS 监听（此前 http_acceptor 只构造
    // 不消费，HTTPS 宣称启用却恒为明文）。
    let tls_acceptor = tls_manager.as_ref().and_then(|tls| {
        if tls.is_http_enabled() {
            // 文案纠偏 —— 后 DualListener 用 TlsAcceptor 在
            // 该端口做真实 TLS 终结，不存在旧文案宣称的 "HTTP fallback"；
            // 保留旧文案会让运维误判加密端口仍是明文。
            info!("HTTPS enabled: HTTP port terminates TLS (rustls acceptor)");
            tls.http_acceptor().cloned()
        } else {
            None
        }
    });

    // 绑定地址由调用方从 config.app.http_addr() 解析传入
    info!("{}", t!("log.main.starting_http_server", addr = bind_addr));
    let listener = DualListener::bind(bind_addr, tls_acceptor).await?;

    // 注入 ConnectInfo<PeerAddr>，使限流键、认证失败计数、
    // 审计 client_ip 恢复 per-IP 语义（缺失时 get_client_ip 恒 None，全站
    // 共享单一 "anonymous" 桶——单攻击者可把 /health、/metrics 一并打成 429）。
    axum::serve(
        listener,
        router.into_make_service_with_connect_info::<PeerAddr>(),
    )
    .with_graceful_shutdown(async {
        tokio::signal::ctrl_c().await.ok();
        info!("{}", t!("log.main.shutting_down_http_server"));
    })
    .await?;

    Ok(())
}

async fn start_grpc_server(
    config: ServerConfig,
    handlers: Arc<ApiHandlers>,
    auth: Arc<ApiKeyAuth>,
    tls_manager: Option<Arc<TlsManager>>,
) -> Result<()> {
    let grpc_addr = SocketAddr::from(([0, 0, 0, 0], config.grpc_port));
    info!("{}", t!("log.main.starting_grpc_server", addr = grpc_addr));
    info!(
        "{}",
        t!("log.main.configured_grpc_port", port = config.grpc_port)
    );

    // gRPC 与 HTTP 使用同一认证器（共享 API key 仓储与失败限流）
    let grpc_server = GrpcServer::with_auth(handlers, auth);

    let shutdown = async {
        tokio::signal::ctrl_c().await.ok();
        info!("{}", t!("log.main.shutting_down_grpc_server"));
    };

    let mut server_builder = Server::builder();

    if let Some(ref tls) = tls_manager {
        if tls.is_grpc_enabled() {
            info!("{}", t!("log.main.grpc_tls_enabled"));
            if let Some(grpc_tls_config) = tls.grpc_tls_config() {
                let config = grpc_tls_config.as_ref().clone();
                server_builder = server_builder.tls_config(config).map_err(|e| {
                    nebulaid::core::types::CoreError::InternalError(t!(
                        "error.main.tls_config_error",
                        reason = e
                    ))
                })?;
            }
        }
    }

    server_builder
        .add_service(NebulaIdServiceServer::new(grpc_server))
        .serve_with_shutdown(grpc_addr, shutdown)
        .await
        .map_err(|e| {
            nebulaid::core::types::CoreError::InternalError(t!(
                "error.main.grpc_server_error",
                reason = e
            ))
        })?;

    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect(&t!("error.main.ctrl_c_handler_install_failed"));
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect(&t!("error.main.terminate_handler_install_failed"))
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

/// 抽取 etcd / non-etcd 分支共用的 ApiHandlers 构造逻辑。
///
/// 原代码在两个 cfg 分支重复调用 `ApiHandlers::with_api_key_repository`
/// + `.with_key_rotation_grace_period`，未来新增 builder 方法需同步改两处
/// （霰弹手术气味）。本 helper 集中构造逻辑。
#[allow(clippy::too_many_arguments, clippy::doc_lazy_continuation)]
fn build_api_handlers(
    id_generator: Arc<nebulaid::core::algorithm::AlgorithmRouter>,
    cs: Arc<dyn ConfigManagementService>,
    repo: Arc<dyn ApiKeyRepository>,
    grace_period_seconds: u64,
) -> ApiHandlers {
    ApiHandlers::with_api_key_repository(id_generator, cs, repo)
        .with_key_rotation_grace_period(grace_period_seconds)
}

/// 生产环境强制 TLS 的判定核心（纯判定 + 一处逃生门 warn，便于单测）。
///
/// 规则（fail-fast）：
/// - 非 production，或 `tls.enabled == true` → 放行；
/// - production 且 TLS 关闭时：
///   - `allow_insecure_tls == Some("1")`（即 `NEBULA_ALLOW_INSECURE_TLS=1`）→
///     放行并 warn（仅限内网评估的显式豁免）；
///   - 其余情形 → `Err`，调用方打印本地化 error 并以非零码退出。
///
/// production 判定含 反向默认：`NEBULA_ENV` 缺失/未知值按生产执行，
/// 因此缺省部署同样被本校验覆盖。
fn validate_tls_required_in_production(
    environment: Environment,
    tls_enabled: bool,
    allow_insecure_tls: Option<&str>,
) -> Result<()> {
    if !environment.is_production() || tls_enabled {
        return Ok(());
    }
    if allow_insecure_tls == Some("1") {
        warn!("{}", t!("log.main.tls_insecure_escape_hatch_active"));
        return Ok(());
    }
    Err(nebulaid::core::types::CoreError::ConfigurationError(
        "TLS is required in production but tls.enabled = false".to_string(),
    ))
}

/// etcd 协调组件装配产物（仅 etcd feature）。
///
/// `lock` 供仓储号段分配跨进程互斥；`client` 为共享长连接 etcd 客户端，
/// 供 worker 分配与 健康巡检注入复用；`None` = 未配置 etcd 的单机模式。
#[cfg(feature = "etcd")]
struct CoordinationComponents {
    lock: std::sync::Arc<dyn nebulaid::core::coordinator::DistributedLock + Send + Sync>,
    /// 共享长连接 etcd client（worker 分配 / 健康巡检注入复用）；
    /// `None` = 未配置 etcd 的单机模式。
    client: Option<std::sync::Arc<dyn nebulaid::core::coordinator::EtcdClientOps>>,
}

/// worker 租约守护（仅 etcd feature）：持有分配器与 keepalive 停机通道。
///
/// 续期连续失败的致命错误经独立的 oneshot 通道（`allocate_worker_id` 的
/// `failure_tx` 入参）上报，main 的 select 接入后触发优雅停机（fail-stop）；
/// `stop_tx` 用于正常停机时让 keepalive 任务退出。
#[cfg(feature = "etcd")]
struct WorkerLeaseGuard {
    allocator: std::sync::Arc<nebulaid::core::coordinator::EtcdWorkerAllocator>,
    worker_id: u16,
    stop_tx: tokio::sync::watch::Sender<bool>,
}

/// worker_id 运行时分配（Snowflake 构造前调用）。
///
/// 经 `EtcdWorkerAllocator::allocate()` 从 etcd 抢占 worker key（key value =
/// 本实例 instance_id，lease 30s 绑定存活），成功后 spawn lease keepalive
/// 任务（interval = lease_ttl/3）。任一步失败返回 `Err`，调用方拒绝启动 ——
/// 多实例部署回退静态默认 0 必然产生重复 worker_id（数据正确性事故）。
#[cfg(feature = "etcd")]
async fn allocate_worker_id(
    client: std::sync::Arc<dyn nebulaid::core::coordinator::EtcdClientOps>,
    config: &Config,
    failure_tx: tokio::sync::oneshot::Sender<String>,
) -> Result<WorkerLeaseGuard> {
    use nebulaid::core::coordinator::EtcdWorkerAllocator;

    let allocator = EtcdWorkerAllocator::new(client.clone(), config.app.dc_id, config.etcd.clone())
        .await
        .map_err(|e| {
            nebulaid::core::types::CoreError::ConfigurationError(t!(
                "error.main.etcd_allocator_init_failed",
                reason = e
            ))
        })?;
    let worker_id = allocator.allocate().await.map_err(|e| {
        nebulaid::core::types::CoreError::ConfigurationError(t!(
            "error.main.etcd_worker_id_allocation_failed",
            reason = e
        ))
    })?;
    if worker_id > u8::MAX as u16 {
        return Err(nebulaid::core::types::CoreError::ConfigurationError(t!(
            "error.main.etcd_worker_id_range_exceeded",
            worker_id = worker_id
        )));
    }

    let lease_id = allocator.current_lease_id();
    let allocator = std::sync::Arc::new(allocator);
    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    tokio::spawn(EtcdWorkerAllocator::run_lease_keepalive_loop(
        client,
        lease_id,
        EtcdWorkerAllocator::keepalive_interval(),
        EtcdWorkerAllocator::KEEPALIVE_MAX_CONSECUTIVE_FAILURES,
        stop_rx,
        failure_tx,
    ));
    info!(
        "{}",
        t!(
            "log.main.worker_id_allocated_from_etcd",
            worker_id = worker_id,
            lease_id = lease_id
        )
    );
    Ok(WorkerLeaseGuard {
        allocator,
        worker_id,
        stop_tx,
    })
}

/// etcd 协调组件装配（fail-closed）。
///
/// 规则：
/// - 未配置 etcd endpoints → 单机部署合法，返回 `LocalDistributedLock`
///   （`client` 为 `None`），不拒绝启动；
/// - 配置了 endpoints → 建立长连接 client 并 ping 探活（`etcd_client::Client`
///   的 connect 是 lazy 的，构造成功不代表可达），再构造 `EtcdDistributedLock`；
///   任一步失败返回 `Err`，调用方打印本地化 error 并以非零码退出 —— 多实例
///   部署静默回退进程内锁必然产生重复 ID（数据正确性事故），禁止降级。
#[cfg(feature = "etcd")]
async fn assemble_coordination(config: &Config) -> Result<CoordinationComponents> {
    use nebulaid::core::coordinator::{
        EtcdClientWrapper, EtcdDistributedLock, LocalDistributedLock, SEGMENT_LOCK_PATH_PREFIX,
    };

    if config.etcd.endpoints.is_empty() {
        warn!("Etcd endpoints not configured, using LocalDistributedLock (single-process only)");
        return Ok(CoordinationComponents {
            lock: std::sync::Arc::new(LocalDistributedLock::new()),
            client: None,
        });
    }

    // fail-closed：配置显式含 etcd endpoints，装配任一步失败 → Err（拒绝启动）。
    let wrapper = EtcdClientWrapper::new(config.etcd.endpoints.clone())
        .await
        .map_err(|e| {
            nebulaid::core::types::CoreError::ConfigurationError(t!(
                "error.main.etcd_client_connect_failed",
                endpoints = format!("{:?}", config.etcd.endpoints),
                reason = e
            ))
        })?
        // operation_timeout_secs 自配置接线（默认 3s）。
        .with_operation_timeout(std::time::Duration::from_secs(
            config.etcd.operation_timeout_secs,
        ));
    let client: std::sync::Arc<dyn nebulaid::core::coordinator::EtcdClientOps> =
        std::sync::Arc::new(wrapper);

    // lazy connect 探活：ping 不通 = etcd 实际不可用，与连接失败同口径 fail-closed。
    let probe_timeout = std::time::Duration::from_millis(config.etcd.connect_timeout_ms.max(1));
    match tokio::time::timeout(probe_timeout, client.ping()).await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            return Err(nebulaid::core::types::CoreError::ConfigurationError(t!(
                "error.main.etcd_ping_failed",
                endpoints = format!("{:?}", config.etcd.endpoints),
                reason = e
            )));
        }
        Err(_) => {
            return Err(nebulaid::core::types::CoreError::ConfigurationError(t!(
                "error.main.etcd_ping_timed_out",
                timeout_ms = config.etcd.connect_timeout_ms,
                endpoints = format!("{:?}", config.etcd.endpoints)
            )));
        }
    }

    let etcd_lock = EtcdDistributedLock::new(client.clone(), SEGMENT_LOCK_PATH_PREFIX.to_string())
        .await
        .map_err(|e| {
            nebulaid::core::types::CoreError::ConfigurationError(t!(
                "error.main.etcd_lock_create_failed",
                reason = e
            ))
        })?;

    Ok(CoordinationComponents {
        lock: std::sync::Arc::new(etcd_lock),
        client: Some(client),
    })
}

/// 无 etcd 路径默认 worker 标识风险判定（纯判定，便于单测）。
///
/// 未配置 etcd（无运行时 worker 分配）且 worker_id / dc_id 任一仍为默认值 0
/// 时返回 true：此时 Snowflake 直接以配置值作为机器标识，多实例部署若不显式
/// 配置 `WORKER_ID` / `DC_ID`（或写进配置文件），两实例拿到同一 (dc_id,
/// worker_id) 组合必然产生重复 ID。仅告警不拦截 —— 单实例部署取默认值合法。
fn should_warn_default_worker_identity(
    etcd_endpoints_configured: bool,
    worker_id: u8,
    dc_id: u8,
) -> bool {
    !etcd_endpoints_configured && (worker_id == 0 || dc_id == 0)
}

/// (unify-rust-i18n)—— 进程默认 locale 解析(生产入口)。
///
/// 检测链(unify-rust-i18n 统一基线 §2,顺序不可调换):
/// `NEBULA_LOCALE`(项目覆盖变量)
/// → `config.app.locale`(用户显式配置,**优先于系统语言**)
/// → `LC_ALL` → `LC_MESSAGES` → `LANG`(POSIX 环境链)
/// → `sys-locale` 系统探测
/// → `en`(终极回退,链尾必达)。
///
/// 用户配置优先于系统语言的语义:只有 NEBULA_LOCALE 与 config.app.locale
/// 都未表达偏好(缺省/空串)时,系统语言才参与;config.app.locale 的
/// serde 默认值为空串(auto),显式写入 "en"/"zh-CN" 即可钉死进程语言。
fn resolve_locale(nebula_locale_env: Option<&str>, config_locale: &str) -> (String, bool) {
    let lc_all = env::var("LC_ALL").ok();
    let lc_messages = env::var("LC_MESSAGES").ok();
    let lang_env = env::var("LANG").ok();
    let sys = sys_locale::get_locale();
    resolve_locale_from(
        nebula_locale_env,
        config_locale,
        lc_all.as_deref(),
        lc_messages.as_deref(),
        lang_env.as_deref(),
        sys.as_deref(),
    )
}

/// 检测链纯函数(env/sys 全部注入,便于单测)。
///
/// - 显式入口(NEBULA_LOCALE / config.app.locale)仅接受规范值 "en"/"zh-CN"
///   (大小写敏感,保留 语义:不做隐式归一化);空串视同未设置。
/// - 环境链/sys-locale 走 POSIX 归一化([`normalize_posix_locale`]):
///   zh* → "zh-CN",en* → "en",C/POSIX/其余语言跳过继续走链。
/// - 显式入口出现非空非法值时置 `invalid = true`(调用方输出本地化 warn)
///   但**继续走链**(参考基线:未知取值回退链继续,不短路),链尾仍落 en。
fn resolve_locale_from(
    nebula_locale_env: Option<&str>,
    config_locale: &str,
    lc_all: Option<&str>,
    lc_messages: Option<&str>,
    lang_env: Option<&str>,
    sys_locale: Option<&str>,
) -> (String, bool) {
    const SUPPORTED_LOCALES: [&str; 2] = ["en", "zh-CN"];
    let mut invalid = false;

    // 1. 项目覆盖变量 NEBULA_LOCALE(空串视同未设置:存在但无值不视为显式选择)
    if let Some(v) = nebula_locale_env.map(str::trim).filter(|v| !v.is_empty()) {
        if SUPPORTED_LOCALES.contains(&v) {
            return (v.to_string(), false);
        }
        invalid = true;
    }

    // 2. 用户配置 config.app.locale(显式配置优先于系统语言)
    let config = config_locale.trim();
    if !config.is_empty() {
        if SUPPORTED_LOCALES.contains(&config) {
            return (config.to_string(), invalid);
        }
        invalid = true;
    }

    // 3. POSIX 环境链 + 4. sys-locale(逐级归一化,首个命中即胜出)
    for raw in [lc_all, lc_messages, lang_env].into_iter().flatten() {
        if let Some(locale) = normalize_posix_locale(raw) {
            return (locale.to_string(), invalid);
        }
    }
    if let Some(raw) = sys_locale {
        if let Some(locale) = normalize_posix_locale(raw) {
            return (locale.to_string(), invalid);
        }
    }

    // 5. 终极回退:en
    ("en".to_string(), invalid)
}

/// POSIX 风格 locale 名归一化(unify-rust-i18n 统一基线 §2 normalize)。
///
/// 去 `@modifier` 与 `.codeset`(`zh_CN.UTF-8` → `zh_CN`),按主语言归一:
/// zh* → "zh-CN",en* → "en";`C`/`POSIX`/其余语言 → `None`(回退链继续)。
fn normalize_posix_locale(raw: &str) -> Option<&'static str> {
    let s = raw.split('@').next()?.trim();
    let s = s.split('.').next()?.trim();
    if matches!(s, "" | "C" | "POSIX") {
        return None;
    }
    let primary = s.split(['-', '_']).next()?;
    if primary.eq_ignore_ascii_case("zh") {
        Some("zh-CN")
    } else if primary.eq_ignore_ascii_case("en") {
        Some("en")
    } else {
        None
    }
}

/// 可观测性初始化:inklog 日志 + 早期 `en` i18n。
///
/// 日志初始化由 inklog 接管(替换原手写的 tracing_subscriber::fmt() 链)。
/// 本地 ../inklog 已切换至 EnvFilter,自动从 RUST_LOG 读取按模块过滤规则
/// (如 `RUST_LOG=nebulaid=debug,hyper=warn`),无需手动读取环境变量。
/// format 是渲染模板(inklog 0.3 语义,非旧命名格式);JSON 行输出由
/// console_json 开启。Arc 持有实例:热重载回调需借同一实例热调全局
/// 级别(set_level 经内部 reload 句柄作用于 live subscriber)。
///
/// i18n 先以内置默认 en 初始化,覆盖配置加载前的极早期日志;配置与
/// NEBULA_LOCALE 检测链解析完成后按生效 locale 重新初始化(
/// 见 [`load_config`])。
async fn init_observability() -> Result<Arc<inklog::LoggerManager>> {
    let logger = Arc::new(
        inklog::LoggerManager::builder()
            .level("info")
            .format("{timestamp} [{level}] {target} - {message}")
            .console_json(true)
            // 必须显式给 file sink 一个路径：inklog 的默认语义是 file_sink 未
            // 配置（None）等价启用，主 async 通道指向 file 通道；但无路径时
            // file worker 不排水，通道积压到 channel_capacity（默认 10000）后
            // 每条日志经 send_timeout(100ms) 阻塞后降级——HTTP 每请求 ≥2 条
            // 日志，审计锁又横跨日志调用，全服务随之串行化（历史事故：环满
            // 后吞吐塌到 ~6 rps、每请求 ~11s）。显式路径让 worker 持续排水。
            .file("logs/nebula.log")
            .build()
            .await?,
    );
    nebulaid::core::i18n::init_i18n("en");
    Ok(logger)
}

/// 命令行配置路径解析(`--config <path>` 显式指定,否则内置默认路径)。
fn parse_config_path(args: &[String]) -> (String, bool) {
    let explicit_path = args.len() > 2 && args[1] == "--config";
    if explicit_path {
        (args[2].clone(), true)
    } else {
        (DEFAULT_CONFIG_PATH.to_string(), false)
    }
}

/// 配置加载合并与 fail-fast 校验。
///
/// 顺序与原 main 内联实现逐字节一致:resolve_startup_config(仅「未显式
/// 指定 --config 且该路径确实不存在」允许回落内置默认值,且必须显式 warn)
/// → 环境变量覆盖合并 → 进程默认 locale 检测链(NEBULA_LOCALE >
/// config.app.locale > LC_ALL/LC_MESSAGES/LANG > sys-locale > "en",非法值
/// 告警并继续走链)→ 无 etcd 默认
/// worker 标识告警 → 生产环境强制 TLS(校验失败打印本地化 error 并
/// 以非零码退出)。
///
/// 注:api_key_salt 的生产校验仍留在 [`init_repository`](与「DB 连接成功」
/// 耦合),保持原有失败顺序不变。
fn load_config(config_path: &str, explicit_path: bool) -> Result<Config> {
    info!("{}", t!("log.main.loading_config", path = config_path));

    // Load config from file first, then merge with environment variables
    //
    // 加载失败不再降级为 `Config::default()`。原实现把错误 `error!` 一行
    // 后继续启动,等价于用一套没人审阅过的默认配置对外提供服务;坏配置现在直接让
    // 进程以非零码退出,消息里带上路径与原因。只有"未显式指定 --config 且该路径确实
    // 不存在"才允许回落内置默认值,且必须显式 warn。
    let (mut config, source) = resolve_startup_config(config_path, explicit_path).map_err(|e| {
        nebulaid::core::types::CoreError::ConfigurationError(t!(
            "error.main.config_load_failed",
            path = config_path,
            reason = e
        ))
    })?;
    if matches!(source, StartupConfig::DefaultsBecauseMissing) {
        warn!(
            "{}",
            t!(
                "log.main.config_defaults_because_missing",
                path = config_path
            )
        );
    }

    // Apply environment variable overrides
    config.merge(Config::load_from_env().map_err(|e| {
        nebulaid::core::types::CoreError::ConfigurationError(t!(
            "error.main.config_env_load_failed",
            reason = e
        ))
    })?);
    info!("{}", t!("log.main.config_loaded"));

    // —— 进程默认 locale 检测链:NEBULA_LOCALE >
    // config.app.locale > LC_ALL/LC_MESSAGES/LANG > sys-locale > "en"
    //(用户显式配置优先于系统语言;config.app.locale serde 默认空串 = auto,
    // 显式 "en"/"zh-CN" 钉死进程语言)。非法值告警并继续走链。此后所有
    // t!() 输出按生效 locale 渲染。
    let nebula_locale_env = env::var("NEBULA_LOCALE").ok();
    let (locale, invalid_locale) = resolve_locale(nebula_locale_env.as_deref(), &config.app.locale);
    if invalid_locale {
        let invalid_value = nebula_locale_env.unwrap_or_else(|| config.app.locale.clone());
        warn!(
            "{}",
            t!(
                "log.main.invalid_locale_falling_back",
                locale = invalid_value.as_str()
            )
        );
    }
    info!("{}", t!("log.main.locale_initialized", locale = &locale));
    nebulaid::core::i18n::init_i18n(&locale);

    // 无 etcd 时默认 worker 标识多实例风险告警:etcd 未配置意味着
    // 没有 worker_id 运行时分配兜底,worker_id/dc_id 任一为默认 0 时显性
    // 提醒(多实例必须显式配置,否则 Snowflake 会重复)。
    if should_warn_default_worker_identity(
        !config.etcd.endpoints.is_empty(),
        config.app.worker_id,
        config.app.dc_id,
    ) {
        warn!(
            "{}",
            t!(
                "log.main.default_worker_id_warning",
                worker_id = config.app.worker_id,
                dc_id = config.app.dc_id
            )
        );
    }

    // 生产环境强制 TLS(fail-fast)。环境判定经 Environment::from_env()
    //(含 反向默认:NEBULA_ENV 缺失/未知值按生产执行,缺失时此处顺带
    // 触发唯一一次 warn)。校验失败打印本地化 error 并以非零码退出;
    // NEBULA_ALLOW_INSECURE_TLS=1 显式放行(函数内 warn)。
    if validate_tls_required_in_production(
        Environment::from_env(),
        config.tls.enabled,
        env::var("NEBULA_ALLOW_INSECURE_TLS").ok().as_deref(),
    )
    .is_err()
    {
        error!("{}", t!("error.main.tls_required_in_production"));
        error!("{}", t!("log.main.shutting_down"));
        std::process::exit(1);
    }

    Ok(config)
}

/// 仓储装配产物:SeaOrmRepository(可选)+ 协调组件(仅 etcd)。
struct RepositoryStack {
    repository: Option<Arc<database::SeaOrmRepository>>,
    /// 协调装配产物;`client` 供健康巡检与 worker 分配复用。
    #[cfg(feature = "etcd")]
    coordination: CoordinationComponents,
}

/// DB 连接/迁移/分布式锁协调/仓储构造。
///
/// 顺序与原 main 内联实现一致:连接(失败 exit 1)→ 迁移(失败 exit 1)→
/// 分布式锁装配(fail-closed,etcd 配置了 endpoints 却不可达时
/// exit 1;未配置 endpoints 才允许单机本地锁)→ 仓储构造(生产环境
/// api_key_salt 弱默认校验 panic)。
async fn init_repository(config: &Config) -> Result<RepositoryStack> {
    // Initialize database connection first (needed for API key auth)
    info!("{}", t!("log.main.connecting_to_database"));
    let db_connection = match database::create_connection(&config.database).await {
        Ok(conn) => {
            info!("{}", t!("log.main.database_connected"));

            // Run auto migrations to create tables
            if let Err(e) = database::run_migrations(&conn).await {
                error!("{}", t!("log.main.migrations_failed", error = e));
                error!("{}", t!("log.main.shutting_down"));
                std::process::exit(1);
            }

            Some(conn)
        }
        Err(e) => {
            error!("{}", t!("log.main.database_connect_failed", error = e));
            error!("{}", t!("log.main.check_database_url"));
            error!("{}", t!("log.main.shutting_down"));
            std::process::exit(1);
        }
    };

    // 分布式锁装配(fail-closed),提前到仓储构造之前
    // 配置显式含 etcd endpoints 时,etcd 不可用直接拒绝启动(多实例场景
    // 静默回退进程内锁必然重复 ID);未配置 endpoints 才允许单机本地锁。
    #[cfg(not(feature = "etcd"))]
    let lock: std::sync::Arc<dyn nebulaid::core::coordinator::DistributedLock + Send + Sync> =
        std::sync::Arc::new(nebulaid::core::coordinator::LocalDistributedLock::new());
    #[cfg(feature = "etcd")]
    let coordination = match assemble_coordination(config).await {
        Ok(components) => components,
        Err(e) => {
            error!(
                "{}",
                t!("error.main.etcd_required_but_unavailable", error = e)
            );
            error!("{}", t!("log.main.shutting_down"));
            std::process::exit(1);
        }
    };
    #[cfg(feature = "etcd")]
    let lock: std::sync::Arc<dyn nebulaid::core::coordinator::DistributedLock + Send + Sync> =
        coordination.lock.clone();

    let repository: Option<Arc<database::SeaOrmRepository>> = if let Some(conn) = db_connection {
        // 生产环境强制校验 api_key_salt 非空且非弱默认值。
        // 规则 12(失败必须显性化):校验失败时 panic,禁止弱 pepper 静默放行。
        if nebulaid::core::config::is_production() {
            let salt = &config.auth.api_key_salt;
            if salt.is_empty()
                || salt.len() < 16
                || salt == "test"
                || salt == "test-secret-value-12345"
            {
                panic!(
                    "{}",
                    t!(
                        "error.main.invalid_api_key_salt_production",
                        env = "NEBULA_API_KEY_SALT"
                    )
                );
            }
        }

        let repo = Arc::new(
            database::SeaOrmRepository::new(conn, config.auth.api_key_salt.clone())
                .with_distributed_lock(lock)
                // statement_timeout_secs 自配置接线（默认 5s）。
                .with_statement_timeout(std::time::Duration::from_secs(
                    config.database.statement_timeout_secs,
                )),
        );
        info!("{}", t!("log.main.database_repository_initialized"));
        Some(repo)
    } else {
        None
    };

    Ok(RepositoryStack {
        repository,
        #[cfg(feature = "etcd")]
        coordination,
    })
}

/// 认证/审计栈装配产物。
struct AuthStack {
    auth: Arc<ApiKeyAuth>,
    audit_logger: Arc<AuditLogger>,
    hot_config: Arc<HotReloadConfig>,
    /// garrison 认证决策缓存(与 ApiKeyAuth 共享同一实例)。
    #[cfg(feature = "garrison-auth")]
    auth_cache: Option<Arc<nebulaid::server::auth::AuthCache>>,
}

/// 认证栈装配:决策缓存、ApiKeyAuth、API key 初始化、审计 logger、
/// 热重载配置与文件监视。
///
/// - 认证决策缓存 —— ApiKeyAuth 与 ApiHandlers 共享同一实例:前者读缓存
///   加速校验,后者在吊销/轮换/重置时失效条目。`cache_ttl_seconds = 0`
///   表示禁用,此时不再装配实例(装配了也不会写入,却仍要在每次校验后
///   多打一次 get_api_key_by_id 判因,纯空转开销)。
/// - —— 审计内存容量改用独立的 audit.memory_capacity;生产环境按
///   audit.file_logging_enabled(默认 true)落盘 audit.file_logging_path
///   (SOC2/GDPR 审计留痕);开发环境维持内存环形:最小意外原则。
async fn init_auth_stack(
    config: &Config,
    repository: &Option<Arc<database::SeaOrmRepository>>,
    logger: Arc<inklog::LoggerManager>,
) -> AuthStack {
    info!(
        "{}",
        t!("log.main.auth_enabled", enabled = config.auth.enabled)
    );

    #[cfg(feature = "garrison-auth")]
    let auth_cache: Option<Arc<nebulaid::server::auth::AuthCache>> =
        match (repository.as_ref(), config.auth.cache_ttl_seconds) {
            (Some(_), ttl @ 1..) => {
                info!(
                    ttl_seconds = ttl,
                    "auth decision cache enabled (in-process GarrisonDao impl)"
                );
                Some(Arc::new(nebulaid::server::auth::AuthCache::new(ttl).await))
            }
            (Some(_), 0) => {
                info!("auth decision cache disabled (auth.cache_ttl_seconds = 0)");
                None
            }
            (None, _) => None,
        };

    // Create API key auth with repository for database-backed storage
    // Phase 9 — configure trusted proxies so the auth
    // middleware only honors `X-Forwarded-For` / `X-Real-IP` when the
    // direct peer IP is in `NEBULA_TRUSTED_PROXIES`. Default: empty
    // (no headers trusted, all clients identified by TCP peer IP).
    let trusted_proxies: Vec<std::net::IpAddr> = std::env::var("NEBULA_TRUSTED_PROXIES")
        .ok()
        .map(|s| s.split(',').filter_map(|p| p.trim().parse().ok()).collect())
        .unwrap_or_default();
    let auth: Arc<ApiKeyAuth> = if let Some(ref repo) = repository {
        #[cfg_attr(not(feature = "garrison-auth"), allow(unused_mut))]
        let mut auth_builder = ApiKeyAuth::new(repo.clone(), config.auth.enabled)
            .with_trusted_proxies(trusted_proxies.clone());
        #[cfg(feature = "garrison-auth")]
        if let Some(cache) = auth_cache.clone() {
            auth_builder = auth_builder.with_cache(cache);
        }
        Arc::new(auth_builder)
    } else {
        error!("{}", t!("log.main.fatal_api_key_auth_requires_database"));
        std::process::exit(1);
    };
    let _ = trusted_proxies; // also consumed by router.rs via env var
    load_api_keys(&auth, repository, config).await;

    // Initialize audit logger and config (used by both etcd and non-etcd modes)
    let audit_logger = Arc::new(
        if nebulaid::core::config::is_production() && config.audit.file_logging_enabled {
            AuditLogger::with_file_logging(
                config.audit.memory_capacity,
                config.audit.file_logging_path.clone(),
            )
            .await
        } else {
            AuditLogger::new(config.audit.memory_capacity)
        },
    );
    let hot_config = Arc::new(HotReloadConfig::new(
        config.clone(),
        "config/config.toml".to_string(),
    ));

    // inklog set_level 热调:热重载/管理端改 logging.level 时同步热调全局
    // subscriber(此前仅更新 hot config 快照,级别变更不生效,需重启进程)。
    {
        let logger = Arc::clone(&logger);
        hot_config.add_reload_callback(move |cfg| {
            let level = cfg.logging.level.to_string();
            if let Err(e) = logger.set_level(None, &level) {
                tracing::error!(error = ?e, level = %level, "inklog set_level hot-apply failed");
            }
        });
    }

    // hot_reload 文件监视启动条件与 feature / repo 解耦 ——
    // 仅取决于 `hot_reload.auto_watch_enabled`,etcd 与非 etcd 分支共用。
    // (原实现位于 etcd 分支"无 repo"路径,非 etcd 构建从不启动监视。)
    if config.hot_reload.auto_watch_enabled {
        let watcher = hot_config.clone();
        tokio::spawn(async move {
            watcher.watch(2000).await;
        });
        info!("{}", t!("log.main.hot_reload_watcher_started"));
    }

    AuthStack {
        auth,
        audit_logger,
        hot_config,
        #[cfg(feature = "garrison-auth")]
        auth_cache,
    }
}

/// 服务器运行栈:[`run_servers`] 的全部输入。
struct ServerStack {
    config: Config,
    server_config: ServerConfig,
    http_bind_addr: SocketAddr,
    repository: Option<Arc<database::SeaOrmRepository>>,
    auth: Arc<ApiKeyAuth>,
    audit_logger: Arc<AuditLogger>,
    hot_config: Arc<HotReloadConfig>,
    /// garrison 认证决策缓存(与 ApiKeyAuth 共享同一实例)。
    #[cfg(feature = "garrison-auth")]
    auth_cache: Option<Arc<nebulaid::server::auth::AuthCache>>,
    /// 协调组件(仅 etcd;`client` 供健康巡检与 worker 分配复用)。
    #[cfg(feature = "etcd")]
    coordination: CoordinationComponents,
}

/// etcd/non-etcd 共用的 handlers + 配置服务构造。
///
/// 原实现两个 cfg 块各自内联 ConfigManager 与 `build_api_handlers` 调用
/// (霰弹手术气味:新增 builder 方法需同步改两处)。本 helper 集中构造逻辑,
/// 与既有 [`build_api_handlers`] 同一意图的延伸。
fn build_handlers_and_config_service(
    config: &Config,
    id_generator: &Arc<nebulaid::core::algorithm::AlgorithmRouter>,
    repository: &Option<Arc<database::SeaOrmRepository>>,
    rate_limiter: &Arc<RateLimiter>,
    hot_config: Arc<HotReloadConfig>,
) -> (ApiHandlers, Arc<dyn ConfigManagementService>) {
    if let Some(ref repo) = repository {
        let cs = Arc::new(
            ConfigManager::with_repository(
                hot_config,
                id_generator.clone(),
                repo.clone(),
                repo.clone(),
                repo.clone(),
            )
            .with_rate_limiter(rate_limiter.clone()),
        );
        let h = build_api_handlers(
            id_generator.clone(),
            cs.clone(),
            repo.clone(),
            config.auth.key_rotation_grace_period_seconds,
        );
        (h, cs)
    } else {
        let cs = Arc::new(
            ConfigManager::new(hot_config, id_generator.clone())
                .with_rate_limiter(rate_limiter.clone()),
        );
        // hot_reload 监视已在公共路径启动,此处不重复。
        (ApiHandlers::new(id_generator.clone(), cs.clone()), cs)
    }
}

/// TLS 管理器装配。
///
/// TLS 配置错误 fail-fast —— enabled=true 且证书缺失/解析失败时拒绝启动
/// (不再静默降级明文)。enabled=false 时 initialize() 直接返回 Ok,明文
/// 部署不受影响。未启用任何 TLS 端口时返回 None。
async fn init_tls_manager(config: &Config) -> Result<Option<Arc<TlsManager>>> {
    let mut tls_manager = TlsManager::new(config.tls.clone());
    tls_manager.initialize().await.map_err(|e| {
        error!("{}", t!("log.main.tls_init_failed", error = e));
        nebulaid::core::types::CoreError::InternalError(t!(
            "error.main.tls_configuration_error",
            reason = e
        ))
    })?;
    Ok(
        if tls_manager.is_http_enabled() || tls_manager.is_grpc_enabled() {
            Some(Arc::new(tls_manager))
        } else {
            None
        },
    )
}

/// etcd 运行时组件(仅 etcd feature): 健康巡检 + worker 租约。
#[cfg(feature = "etcd")]
struct EtcdRuntimeComponents {
    health_monitor: Arc<EtcdClusterHealthMonitor>,
    worker_lease: Option<WorkerLeaseGuard>,
    /// worker 租约续期失败上报通道;仅在实际分配到 worker_id 时为 Some
    /// (未分配时 select 臂的 future 恒 pending,与原 `worker_lease.is_some()`
    /// 守卫等价)。
    lease_failure_rx: Option<tokio::sync::oneshot::Receiver<String>>,
}

/// etcd 运行时装配(仅 etcd feature)。
///
/// 健康巡检接线(注入协调装配的共享长连接 client + 缓存周期落盘)与
/// worker_id 运行时分配(分配失败 fail-closed:打印本地化 error 并以
/// 非零码退出 —— 多实例回退静态默认 0 必然产生重复 worker_id)。
#[cfg(feature = "etcd")]
async fn init_etcd_runtime(
    config: &mut Config,
    coordination: &CoordinationComponents,
) -> EtcdRuntimeComponents {
    info!("{}", t!("log.main.initializing_etcd_health_monitor"));
    // 缓存文件名追加 pid 段:同机多副本共用 dc_id 时
    // `./data/etcd_cache_{dc_id}.json` 会互相踩踏覆盖。
    let etcd_cache_path = format!(
        "./data/etcd_cache_{}_{}.json",
        config.app.dc_id,
        std::process::id()
    );

    // 健康巡检统一走注入的长连接 client:复用 协调装配
    // 的 EtcdClientWrapper(fail-closed 已 ping 探活)。原实现此处再建
    // 一个 client 且 fallback 路径每次检查新建 client —— etcd connect
    // 是 lazy 的,构造成功不代表可达,Failed 判定几乎永不触发。
    let etcd_health_monitor = match &coordination.client {
        Some(client) => {
            info!("{}", t!("log.main.etcd_client_wrapper_initialized"));
            Arc::new(EtcdClusterHealthMonitor::new_with_client(
                config.etcd.clone(),
                etcd_cache_path,
                client.clone(),
            ))
        }
        None => Arc::new(EtcdClusterHealthMonitor::new(
            config.etcd.clone(),
            etcd_cache_path,
        )),
    };

    if let Err(e) = etcd_health_monitor.load_local_cache().await {
        warn!("{}", t!("log.main.etcd_local_cache_load_failed", error = e));
    }

    // 巡检与缓存持久化接线:健康状态周期刷新(真实 ping 判定
    // Degraded/Failed 并驱动降级),本地缓存周期落盘(etcd 故障时
    // 供重启后的实例读取)。未配置 etcd(单机)时同样不启动巡检意义
    // 不大,但保持一致行为无副作用(check 走 no_endpoints early-return)。
    etcd_health_monitor
        .start_health_check(std::time::Duration::from_secs(30))
        .await;
    etcd_health_monitor
        .start_cache_persistence(std::time::Duration::from_secs(300))
        .await;

    info!("{}", t!("log.main.etcd_health_monitor_initialized"));

    // worker_id 运行时分配:etcd 已配置(coordination.client 为
    // Some)时于 Snowflake 构造前分配并覆盖静态配置值;分配失败 fail-closed
    //(多实例回退静态默认 0 必然重复 ID)。
    let (lease_failure_tx, lease_failure_rx) = tokio::sync::oneshot::channel::<String>();
    let worker_lease: Option<WorkerLeaseGuard> = match &coordination.client {
        Some(client) => match allocate_worker_id(client.clone(), config, lease_failure_tx).await {
            Ok(guard) => Some(guard),
            Err(e) => {
                error!(
                    "{}",
                    t!("error.main.worker_id_allocation_failed", error = e)
                );
                error!("{}", t!("log.main.shutting_down"));
                std::process::exit(1);
            }
        },
        None => None,
    };
    if let Some(guard) = &worker_lease {
        // 分配器 MAX_WORKER_ID=255,覆盖值必在 config.worker_id 的 u8 值域内
        config.app.worker_id = guard.worker_id as u8;
    }

    // 仅在实际分配到 worker_id 时启用失败上报臂(未分配时恒 pending)
    let lease_failure_rx = worker_lease.is_some().then_some(lease_failure_rx);
    EtcdRuntimeComponents {
        health_monitor: etcd_health_monitor,
        worker_lease,
        lease_failure_rx,
    }
}

/// lease 续期失败等待:etcd 构建等待上报通道(未分配 worker 时
/// 恒 pending);非 etcd 构建恒 pending(select 臂永不触发,等价于原实现
/// 「非 etcd 无此臂」)。
async fn wait_for_lease_failure(
    #[cfg_attr(not(feature = "etcd"), allow(unused))] rx: Option<
        tokio::sync::oneshot::Receiver<String>,
    >,
) -> String {
    #[cfg(feature = "etcd")]
    if let Some(rx) = rx {
        return rx
            .await
            .unwrap_or_else(|_| "lease keepalive task dropped".to_string());
    }
    std::future::pending::<String>().await
}

/// 服务器运行编排:ID 生成器/限流/handlers/TLS/降级巡检装配、
/// HTTP/gRPC spawn 与优雅停机 select。
///
/// etcd 与非 etcd 构建共用同一实现(此前为两段近乎复制的 cfg 块:限流器、
/// handlers/TLS/降级巡检/服务器 spawn/select/收尾清理全部双写,新增装配
/// 步骤需霰弹式同步修改两处)。etcd 专属阶段(健康巡检 + worker 租约)经
/// cfg 隔离在 [`init_etcd_runtime`]。
async fn run_servers(stack: ServerStack) -> Result<()> {
    #[cfg(feature = "etcd")]
    let coordination = stack.coordination;
    #[cfg_attr(not(feature = "etcd"), allow(unused_mut))]
    let mut config = stack.config;
    let server_config = stack.server_config;
    let http_bind_addr = stack.http_bind_addr;
    let repository = stack.repository;
    let auth = stack.auth;
    let audit_logger = stack.audit_logger;
    let hot_config = stack.hot_config;
    #[cfg(feature = "garrison-auth")]
    let auth_cache = stack.auth_cache;

    // 健康巡检 + worker 租约(仅 etcd feature;非 etcd 构建
    // 无此阶段)。
    #[cfg(feature = "etcd")]
    let etcd_runtime = init_etcd_runtime(&mut config, &coordination).await;

    #[cfg(feature = "etcd")]
    let id_generator = create_id_generator(
        &config,
        audit_logger.clone(),
        Some(etcd_runtime.health_monitor.clone()),
    )
    .await?;
    #[cfg(not(feature = "etcd"))]
    let id_generator = {
        info!("{}", t!("log.main.etcd_disabled"));

        // 删除原此处创建后即丢弃的 AlgorithmRouter 死代码;
        // id_generator 由 create_id_generator 构建并作为实际生成器使用。
        create_id_generator(&config, audit_logger.clone(), None).await?
    };

    // 限流器先于 ConfigManager 创建,经 with_rate_limiter
    // 共享给运行时配置服务,使 POST /config/rate-limit 热更新作用于流量。
    let rate_limiter = Arc::new(RateLimiter::new(
        config.rate_limit.default_rps,
        config.rate_limit.burst_size,
    ));
    // 启动限流桶清理后台任务(5 分钟空闲桶回收,60s 周期),
    // 停机时经 abort 退出,防止任务泄漏。
    let rate_limit_cleanup = rate_limiter.start_cleanup(
        std::time::Duration::from_secs(300),
        std::time::Duration::from_secs(60),
    );

    let (handlers, config_service) = build_handlers_and_config_service(
        &config,
        &id_generator,
        &repository,
        &rate_limiter,
        hot_config,
    );
    // garrison-auth 决策缓存注入(原两个 cfg 分支重复的尾部装配)。
    #[cfg(feature = "garrison-auth")]
    let handlers = if let Some(cache) = auth_cache {
        handlers.with_auth_cache(cache)
    } else {
        handlers
    };
    let handlers = Arc::new(handlers);

    let tls_manager = init_tls_manager(&config).await?;

    info!("{}", t!("log.main.starting_degradation_check"));
    let degradation_manager = id_generator.get_degradation_manager();
    degradation_manager.start_background_check();

    info!("{}", t!("log.main.server_initialized_starting"));

    let http_server = tokio::spawn(start_http_server(
        http_bind_addr,
        handlers.clone(),
        auth.clone(),
        rate_limiter.clone(),
        audit_logger.clone(),
        config_service.clone(),
        tls_manager.clone(),
    ));
    let grpc_server = tokio::spawn(start_grpc_server(
        server_config,
        handlers,
        auth.clone(),
        tls_manager,
    ));

    // lease 续期失败臂的 future 输入:非 etcd 构建恒 None。
    #[cfg(feature = "etcd")]
    let lease_failure_rx = etcd_runtime.lease_failure_rx;
    #[cfg(not(feature = "etcd"))]
    let lease_failure_rx: Option<tokio::sync::oneshot::Receiver<String>> = None;

    // http / grpc / 停机信号 / lease 续期失败任一路径先就绪时,统一在
    // select 之后回收后台任务(降级巡检 + 限流桶清理)再返回。原实现只在
    // shutdown_signal 分支 abort:服务器先退出(正常停止或错误退出)时清理
    // 任务泄漏,tokio 运行时 drop 还会一直等这个永不自退的循环任务。
    let server_result: Result<()> = tokio::select! {
        http_result = http_server => match http_result {
            Ok(Ok(())) => {
                info!("{}", t!("log.main.http_server_stopped"));
                Ok(())
            }
            Ok(Err(e)) => {
                error!("{}", t!("log.main.http_server_error", error = e));
                Err(e)
            }
            Err(e) => {
                error!("{}", t!("log.main.http_server_panic", error = e));
                Err(nebulaid::core::types::CoreError::InternalError(t!(
                    "error.main.http_server_panic",
                    reason = e
                )))
            }
        },
        grpc_result = grpc_server => match grpc_result {
            Ok(Ok(())) => {
                info!("{}", t!("log.main.grpc_server_stopped"));
                Ok(())
            }
            Ok(Err(e)) => {
                error!("{}", t!("log.main.grpc_server_error", error = e));
                Err(e)
            }
            Err(e) => {
                error!("{}", t!("log.main.grpc_server_panic", error = e));
                Err(nebulaid::core::types::CoreError::InternalError(t!(
                    "error.main.grpc_server_panic",
                    reason = e
                )))
            }
        },
        _ = shutdown_signal() => {
            info!("{}", t!("log.main.shutdown_signal_received"));
            Ok(())
        }
        // lease 续期连续失败 fail-stop:etcd 不可达使 lease 失效后,
        // 该 worker_id 可能已被其他实例接管,继续发号会重复 ID → 触发优雅停机。
        // 未分配 worker lease(含非 etcd 构建)时该 future 恒 pending。
        lease_reason = wait_for_lease_failure(lease_failure_rx) => {
            error!(
                "{}",
                t!("error.main.worker_lease_renewal_failed", reason = lease_reason)
            );
            error!("{}", t!("log.main.shutting_down"));
            Err(nebulaid::core::types::CoreError::InternalError(t!(
                "error.main.worker_lease_renewal_failed",
                reason = lease_reason
            )))
        }
    };

    degradation_manager.stop_background_check().await;
    rate_limit_cleanup.abort();

    // 停机收尾:停 keepalive 任务,并经归属校验释放 worker_id
    //(best-effort:失败仅告警,lease TTL 到期后 etcd 会自动回收 key)。
    #[cfg(feature = "etcd")]
    if let Some(guard) = &etcd_runtime.worker_lease {
        let _ = guard.stop_tx.send(true);
        if let Err(e) = guard.allocator.release(guard.worker_id).await {
            warn!(
                "{}",
                t!(
                    "log.main.worker_id_release_on_shutdown_failed",
                    worker_id = guard.worker_id,
                    error = e
                )
            );
        } else {
            info!(
                "{}",
                t!(
                    "log.main.worker_id_released_on_shutdown",
                    worker_id = guard.worker_id
                )
            );
        }
    }

    server_result
}

/// main 主体只保留顺序编排:可观测性 → 配置 → 仓储 → 认证/审计
/// 栈 → 服务器运行。各阶段细节见对应装配函数。
#[tokio::main]
async fn main() -> Result<()> {
    let logger = init_observability().await?;

    info!("{}", t!("log.main.starting_service"));
    info!(
        "{}",
        t!("log.main.version", version = env!("CARGO_PKG_VERSION"))
    );

    // Initialize sdforge plugins so inventory-registered routes are linked
    // into the final binary (prevents linker stripping). Must be called
    // before merge_sdforge_routes builds the axum Router.
    let plugin_counts = init_sdforge();
    info!(
        routes = plugin_counts.routes,
        "{}",
        t!("log.main.sdforge_plugins_initialized")
    );

    // Parse command line arguments
    let args: Vec<String> = env::args().collect();
    let (config_path, explicit_path) = parse_config_path(&args);

    let config = load_config(&config_path, explicit_path)?;

    let server_config = ServerConfig {
        http_port: config.app.http_port,
        grpc_port: config.app.grpc_port,
        workers: std::thread::available_parallelism()
            .map(|p| p.get())
            .unwrap_or(1),
        shutdown_timeout_secs: config.app.shutdown_timeout_seconds,
    };

    // HTTP 绑定地址唯一来源于 config.app(host + http_port),修复原先
    // 忽略配置、硬编码 [0,0,0,0]:8080 导致 http_port 配置失效的缺陷。
    let http_bind_addr: SocketAddr = config.app.http_addr().map_err(|e| {
        nebulaid::core::types::CoreError::InternalError(t!(
            "error.main.http_bind_address_invalid",
            host = config.app.host,
            port = config.app.http_port,
            reason = e
        ))
    })?;

    info!(
        "{}",
        t!(
            "log.main.starting_server_on_ports",
            http_port = server_config.http_port,
            grpc_port = server_config.grpc_port
        )
    );

    let stacks = init_repository(&config).await?;
    let auth_stack = init_auth_stack(&config, &stacks.repository, logger).await;

    run_servers(ServerStack {
        config,
        server_config,
        http_bind_addr,
        repository: stacks.repository,
        auth: auth_stack.auth,
        audit_logger: auth_stack.audit_logger,
        hot_config: auth_stack.hot_config,
        #[cfg(feature = "garrison-auth")]
        auth_cache: auth_stack.auth_cache,
        #[cfg(feature = "etcd")]
        coordination: stacks.coordination,
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use nebulaid::core::algorithm::AlgorithmRouter;
    use nebulaid::core::config::Config;
    use nebulaid::server::config::hot_reload::HotReloadConfig;
    use std::sync::Arc;

    /// Setup test environment - must be called at the start of each test
    fn setup_test_env() {
        std::env::set_var("NEBULA_DATABASE_PASSWORD", "test_password");
    }

    #[tokio::test]
    async fn test_server_config_default() {
        let config = ServerConfig::default();
        // 端口默认值唯一归属 NebulaIdConfig::default（删除本地端口常量后）
        let app = nebulaid::core::config::NebulaIdConfig::default();
        assert_eq!(config.http_port, app.http_port);
        assert_eq!(config.grpc_port, app.grpc_port);
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
            ) -> nebulaid::core::types::Result<database::ApiKeyWithSecret> {
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
                    grace_expires_at: None,
                })
            }

            async fn get_api_key_by_id(
                &self,
                _key_id: &str,
            ) -> nebulaid::core::types::Result<Option<database::ApiKeyInfo>> {
                Ok(None)
            }

            async fn validate_api_key(
                &self,
                _key_id: &str,
                _key_secret: &str,
            ) -> nebulaid::core::types::Result<Option<database::AuthenticatedKey>> {
                Ok(None)
            }

            async fn list_api_keys(
                &self,
                _workspace_id: uuid::Uuid,
                _limit: Option<u32>,
                _offset: Option<u32>,
            ) -> nebulaid::core::types::Result<Vec<database::ApiKeyInfo>> {
                Ok(vec![])
            }

            async fn delete_api_key(&self, _id: uuid::Uuid) -> nebulaid::core::types::Result<()> {
                Ok(())
            }

            async fn revoke_api_key(&self, _id: uuid::Uuid) -> nebulaid::core::types::Result<()> {
                Ok(())
            }

            async fn update_last_used(
                &self,
                _key: uuid::Uuid,
            ) -> nebulaid::core::types::Result<()> {
                Ok(())
            }

            async fn get_admin_api_key(
                &self,
                _workspace_id: uuid::Uuid,
            ) -> nebulaid::core::types::Result<Option<database::ApiKeyInfo>> {
                Ok(None)
            }

            async fn count_api_keys(
                &self,
                _workspace_id: uuid::Uuid,
            ) -> nebulaid::core::types::Result<u64> {
                Ok(0)
            }

            /// admin 守卫用：本 mock 只做 bin 装配，与 get_api_key_by_id 一样返回 None。
            async fn find_api_key_by_row_id(
                &self,
                _id: uuid::Uuid,
            ) -> nebulaid::core::types::Result<Option<database::ApiKeyInfo>> {
                Ok(None)
            }

            /// admin 守卫用：本 mock 只做 bin 装配，与 count_api_keys 一样返回 0。
            async fn count_admin_keys(&self) -> nebulaid::core::types::Result<u64> {
                Ok(0)
            }

            async fn rotate_api_key(
                &self,
                _key_id: &str,
                _grace_period_seconds: u64,
            ) -> nebulaid::core::types::Result<database::ApiKeyWithSecret> {
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
                    grace_expires_at: None,
                })
            }

            async fn get_keys_older_than(
                &self,
                _age_threshold_days: i64,
            ) -> nebulaid::core::types::Result<Vec<database::ApiKeyInfo>> {
                Ok(vec![])
            }
        }

        let config = Config::default();
        let audit_logger: Arc<dyn nebulaid::core::algorithm::AuditLogger> =
            Arc::new(AuditLogger::new(10000));
        let router = AlgorithmRouter::new(config.clone(), Some(audit_logger));
        let router = Arc::new(router);
        let hot_config = Arc::new(HotReloadConfig::new(
            config.clone(),
            "config/config.toml".to_string(),
        ));
        let config_service = Arc::new(ConfigManager::new(hot_config, router.clone()));
        let handlers = Arc::new(ApiHandlers::with_api_key_repository(
            router,
            config_service,
            Arc::new(MockApiKeyRepo),
        ));
        let auth = Arc::new(ApiKeyAuth::new(Arc::new(MockApiKeyRepo), true));
        let rate_limiter = Arc::new(RateLimiter::new(10000, 100));
        let audit_logger = Arc::new(AuditLogger::new(10000));

        let _router =
            nebulaid::server::router::create_router(handlers, auth, rate_limiter, audit_logger)
                .await;
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
            ) -> nebulaid::core::types::Result<database::ApiKeyWithSecret> {
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
                    grace_expires_at: None,
                })
            }

            async fn get_api_key_by_id(
                &self,
                _key_id: &str,
            ) -> nebulaid::core::types::Result<Option<database::ApiKeyInfo>> {
                Ok(None)
            }

            async fn validate_api_key(
                &self,
                _key_id: &str,
                _key_secret: &str,
            ) -> nebulaid::core::types::Result<Option<database::AuthenticatedKey>> {
                Ok(None)
            }

            async fn list_api_keys(
                &self,
                _workspace_id: uuid::Uuid,
                _limit: Option<u32>,
                _offset: Option<u32>,
            ) -> nebulaid::core::types::Result<Vec<database::ApiKeyInfo>> {
                Ok(vec![])
            }

            async fn delete_api_key(&self, _id: uuid::Uuid) -> nebulaid::core::types::Result<()> {
                Ok(())
            }

            async fn revoke_api_key(&self, _id: uuid::Uuid) -> nebulaid::core::types::Result<()> {
                Ok(())
            }

            async fn update_last_used(
                &self,
                _key: uuid::Uuid,
            ) -> nebulaid::core::types::Result<()> {
                Ok(())
            }

            async fn get_admin_api_key(
                &self,
                _workspace_id: uuid::Uuid,
            ) -> nebulaid::core::types::Result<Option<database::ApiKeyInfo>> {
                Ok(None)
            }

            async fn count_api_keys(
                &self,
                _workspace_id: uuid::Uuid,
            ) -> nebulaid::core::types::Result<u64> {
                Ok(0)
            }

            /// admin 守卫用：本 mock 只做 bin 装配，与 get_api_key_by_id 一样返回 None。
            async fn find_api_key_by_row_id(
                &self,
                _id: uuid::Uuid,
            ) -> nebulaid::core::types::Result<Option<database::ApiKeyInfo>> {
                Ok(None)
            }

            /// admin 守卫用：本 mock 只做 bin 装配，与 count_api_keys 一样返回 0。
            async fn count_admin_keys(&self) -> nebulaid::core::types::Result<u64> {
                Ok(0)
            }

            async fn rotate_api_key(
                &self,
                _key_id: &str,
                _grace_period_seconds: u64,
            ) -> nebulaid::core::types::Result<database::ApiKeyWithSecret> {
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
                    grace_expires_at: None,
                })
            }

            async fn get_keys_older_than(
                &self,
                _age_threshold_days: i64,
            ) -> nebulaid::core::types::Result<Vec<database::ApiKeyInfo>> {
                Ok(vec![])
            }
        }

        let config = Config::default();
        let audit_logger: Arc<dyn nebulaid::core::algorithm::AuditLogger> =
            Arc::new(AuditLogger::new(10000));
        let router = AlgorithmRouter::new(config.clone(), Some(audit_logger));
        let router = Arc::new(router);
        let hot_config = Arc::new(HotReloadConfig::new(
            config.clone(),
            "config/config.toml".to_string(),
        ));
        let config_service = Arc::new(ConfigManager::new(hot_config, router.clone()));
        let handlers = Arc::new(ApiHandlers::with_api_key_repository(
            router,
            config_service.clone(),
            Arc::new(MockApiKeyRepo),
        ));
        let auth = Arc::new(ApiKeyAuth::new(Arc::new(MockApiKeyRepo), true));
        let rate_limiter = Arc::new(RateLimiter::new(10000, 100));
        let audit_logger = Arc::new(AuditLogger::new(10000));

        let server = tokio::spawn(async move {
            start_http_server(
                SocketAddr::from(([127, 0, 0, 1], 0)),
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

    // ==================== 生产环境 TLS 强制校验 ====================

    #[test]
    fn test_validate_tls_production_tls_disabled_no_escape_hatch_errors() {
        // production + tls.enabled=false + 无逃生门 → 拒绝启动（Err → exit 非零）
        let result = validate_tls_required_in_production(Environment::Production, false, None);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_tls_production_tls_disabled_escape_hatch_allows() {
        // NEBULA_ALLOW_INSECURE_TLS=1 → 显式放行（调用方负责 warn）
        let result = validate_tls_required_in_production(Environment::Production, false, Some("1"));
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_tls_production_tls_enabled_ok() {
        // TLS 已启用 → 直接通过，与逃生门无关
        let result = validate_tls_required_in_production(Environment::Production, true, None);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_tls_development_unaffected() {
        // 开发模式不强制 TLS
        let result = validate_tls_required_in_production(Environment::Development, false, None);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_tls_escape_hatch_value_other_than_1_rejected() {
        // 逃生门仅认字面量 "1"，其余取值视为未启用
        let result =
            validate_tls_required_in_production(Environment::Production, false, Some("true"));
        assert!(result.is_err());
        let result = validate_tls_required_in_production(Environment::Production, false, Some(""));
        assert!(result.is_err());
    }

    // ==================== 分布式锁装配 fail-closed ====================

    /// 配置显式含 etcd endpoints + 不可达端点 → 装配必须失败（fail-closed）。
    ///
    /// `etcd_client::Client::connect` 是 lazy 的，`EtcdClientWrapper::new` 对不可达
    /// endpoint 也返回 Ok；`assemble_coordination` 因此在构造后 ping 探活，把
    /// "配置要求 etcd 但实际不可用" 确定性地转为 Err（main 据此打印本地化错误
    /// 并以非零码退出），不再静默回退 LocalDistributedLock。
    #[cfg(feature = "etcd")]
    #[tokio::test]
    async fn test_assemble_coordination_fail_closed_on_unreachable_etcd() {
        let mut config = Config::default();
        config.etcd.endpoints = vec!["http://127.0.0.1:1".to_string()];
        config.etcd.connect_timeout_ms = 300;

        let result = assemble_coordination(&config).await;
        let err_msg = match result {
            Err(e) => e.to_string(),
            Ok(_) => panic!("配置显式含 etcd endpoints 但端点不可达时必须拒绝装配（fail-closed）"),
        };
        assert!(
            err_msg.contains("127.0.0.1:1") || err_msg.contains("ping"),
            "错误信息应指向不可达端点或 ping 探活，实际: {err_msg}"
        );
    }

    /// 未配置 endpoints → 单机本地锁放行（合法），共享 client 为 None。
    #[cfg(feature = "etcd")]
    #[tokio::test]
    async fn test_assemble_coordination_local_lock_when_no_endpoints() {
        let mut config = Config::default();
        config.etcd.endpoints = vec![];

        let components = assemble_coordination(&config)
            .await
            .expect("未配置 etcd 时装配必须放行（单机合法）");
        assert!(
            components.client.is_none(),
            "未配置 etcd 时不应产出共享 client"
        );
        assert!(
            components.lock.is_healthy(),
            "LocalDistributedLock 应恒健康"
        );

        // 本地锁应可正常 acquire/release（号段分配互斥在单机内仍生效）
        let guard = components
            .lock
            .acquire("t016-local-key", 5)
            .await
            .expect("本地锁 acquire 必须成功");
        guard.release().await.expect("本地锁 release 必须成功");
    }

    /// verify 钉（bin 侧可达性）—— `DbSegmentLoader` 经 mod.rs re-export
    /// 后可从 bin crate 构造（`SegmentAlgorithm::new(dc).with_segment_loader(...)`
    /// 装配契约的 bin 侧入口）。注意：服务端 Segment 实例由
    /// `AlgorithmRouter::initialize` 经 `SegmentFactory` 内部构建，bin 侧
    /// 注入缝（router/traits 增设）不在本 lane 文件所有权内。
    #[test]
    fn test_db_segment_loader_assembly_api_reachable_from_bin() {
        use dbnexus::sea_orm::{DatabaseBackend, MockDatabase};

        let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();
        let repo = Arc::new(database::SeaOrmRepository::new(db, "test_salt".to_string()));
        // 构造成功即证明 re-export 与泛型约束（SeaOrmRepository: SegmentRepository）
        // 在 bin 侧可用；字段与步长断言归 lib 侧单测（segment.rs）所有。
        let _loader = nebulaid::core::algorithm::DbSegmentLoader::new(repo);
    }

    // ==================== 无 etcd 默认 worker 标识告警 ====================

    /// 未配置 etcd + worker_id/dc_id 默认 0 → 必须告警。
    #[test]
    fn test_warn_default_worker_identity_triggers_on_defaults() {
        assert!(should_warn_default_worker_identity(false, 0, 0));
        assert!(should_warn_default_worker_identity(false, 0, 3));
        assert!(should_warn_default_worker_identity(false, 5, 0));
    }

    /// 已配置 etcd（有运行时分配兜底）→ 不告警，即使值为默认 0。
    #[test]
    fn test_warn_default_worker_identity_skipped_when_etcd_configured() {
        assert!(!should_warn_default_worker_identity(true, 0, 0));
        assert!(!should_warn_default_worker_identity(true, 0, 3));
    }

    /// 未配置 etcd 但标识均已显式配置（非 0）→ 不告警。
    #[test]
    fn test_warn_default_worker_identity_skipped_when_explicitly_configured() {
        assert!(!should_warn_default_worker_identity(false, 1, 1));
        assert!(!should_warn_default_worker_identity(false, 255, 31));
    }

    // ==================== 默认 locale 配置化 ====================

    /// 环境变量 NEBULA_LOCALE 优先于配置值。
    #[test]
    fn test_resolve_locale_env_overrides_config() {
        let (locale, invalid) = resolve_locale(Some("zh-CN"), "en");
        assert_eq!(locale, "zh-CN");
        assert!(!invalid);
    }

    /// 环境变量缺失时使用配置值。
    #[test]
    fn test_resolve_locale_falls_back_to_config() {
        let (locale, invalid) = resolve_locale(None, "zh-CN");
        assert_eq!(locale, "zh-CN");
        assert!(!invalid);

        let (locale, invalid) = resolve_locale(None, "en");
        assert_eq!(locale, "en");
        assert!(!invalid);
    }

    /// 环境变量为空串视同未设置（沿环境变量覆盖惯例）。
    #[test]
    fn test_resolve_locale_empty_env_treated_as_unset() {
        let (locale, invalid) = resolve_locale(Some(""), "zh-CN");
        assert_eq!(locale, "zh-CN");
        assert!(!invalid);
    }

    /// 非法值（env 与 config 两侧）回退 en 并标记 invalid。
    #[test]
    fn test_resolve_locale_invalid_values_fall_back_to_en() {
        // env 非法
        let (locale, invalid) = resolve_locale(Some("fr"), "en");
        assert_eq!(locale, "en", "非法 env 值必须回退 en");
        assert!(invalid, "非法值必须被标记，供调用方告警");

        // config 非法
        let (locale, invalid) = resolve_locale(None, "es-ES");
        assert_eq!(locale, "en", "非法 config 值必须回退 en");
        assert!(invalid);

        // 大小写敏感：非规范取值按非法处理（不做隐式归一化）
        let (locale, invalid) = resolve_locale(Some("ZH-cn"), "en");
        assert_eq!(locale, "en");
        assert!(invalid);
    }

    // ==================== (unify-rust-i18n): 检测链扩展 ====================

    /// POSIX 名归一化:`zh_CN.UTF-8` → zh-CN、`zh_TW` → zh-CN、
    /// `en_US` → en、C/POSIX/未知语言 → None(回退链继续)。
    #[test]
    fn test_normalize_posix_locale() {
        assert_eq!(normalize_posix_locale("zh_CN.UTF-8"), Some("zh-CN"));
        assert_eq!(normalize_posix_locale("zh_TW"), Some("zh-CN"));
        assert_eq!(normalize_posix_locale("zh-Hans-CN"), Some("zh-CN"));
        assert_eq!(normalize_posix_locale("en_US.UTF-8"), Some("en"));
        assert_eq!(normalize_posix_locale("en"), Some("en"));
        assert_eq!(normalize_posix_locale("C"), None);
        assert_eq!(normalize_posix_locale("POSIX"), None);
        assert_eq!(normalize_posix_locale("C.UTF-8"), None);
        assert_eq!(normalize_posix_locale("fr_FR.UTF-8"), None);
        assert_eq!(normalize_posix_locale("ja_JP.eucJP"), None);
        assert_eq!(normalize_posix_locale(""), None);
    }

    /// NEBULA_LOCALE 优先于整条检测链(env 变量 + sys-locale 同时存在)。
    #[test]
    fn test_detect_chain_nebula_locale_beats_env_and_sys() {
        let (locale, invalid) = resolve_locale_from(
            Some("zh-CN"),
            "",
            Some("en_US.UTF-8"),
            Some("en_US.UTF-8"),
            Some("en_US.UTF-8"),
            Some("en-US"),
        );
        assert_eq!(locale, "zh-CN");
        assert!(!invalid);
    }

    /// 用户显式配置优先于系统语言:config "zh-CN" 胜过 LANG/en 变体,
    /// config "en" 能钉死英文(屏蔽 zh 系统)。
    #[test]
    fn test_detect_chain_config_beats_env_chain() {
        let (locale, invalid) = resolve_locale_from(
            None,
            "zh-CN",
            Some("en_US.UTF-8"),
            None,
            Some("en_US.UTF-8"),
            None,
        );
        assert_eq!(locale, "zh-CN");
        assert!(!invalid);

        let (locale, invalid) = resolve_locale_from(
            None,
            "en",
            Some("zh_CN.UTF-8"),
            Some("zh_CN.UTF-8"),
            Some("zh_CN.UTF-8"),
            Some("zh-CN"),
        );
        assert_eq!(locale, "en", "显式配置 en 必须屏蔽 zh 系统");
        assert!(!invalid);
    }

    /// 无显式偏好(NEBULA_LOCALE 未设、config 空 = auto)时,
    /// POSIX 环境链按 LC_ALL → LC_MESSAGES → LANG 顺序生效。
    #[test]
    fn test_detect_chain_posix_env_order() {
        // LC_ALL 最优先
        let (locale, _) = resolve_locale_from(
            None,
            "",
            Some("zh_CN.UTF-8"),
            Some("en_US.UTF-8"),
            Some("en_US.UTF-8"),
            None,
        );
        assert_eq!(locale, "zh-CN");
        // LC_ALL 缺失 → LC_MESSAGES
        let (locale, _) = resolve_locale_from(
            None,
            "",
            None,
            Some("zh_CN.UTF-8"),
            Some("en_US.UTF-8"),
            None,
        );
        assert_eq!(locale, "zh-CN");
        // LC_MESSAGES 也缺失 → LANG
        let (locale, _) = resolve_locale_from(None, "", None, None, Some("zh_CN.UTF-8"), None);
        assert_eq!(locale, "zh-CN");
        // LANG 是 en 变体也命中
        let (locale, _) = resolve_locale_from(None, "", None, None, Some("en_GB.UTF-8"), None);
        assert_eq!(locale, "en");
    }

    /// 环境链全空时由 sys-locale 兜底;C/POSIX/未知语言跳过继续走链。
    #[test]
    fn test_detect_chain_sys_locale_and_unsupported_skip() {
        let (locale, _) = resolve_locale_from(None, "", None, None, None, Some("zh-CN"));
        assert_eq!(locale, "zh-CN");
        let (locale, _) = resolve_locale_from(None, "", None, None, None, Some("en-US"));
        assert_eq!(locale, "en");

        // LANG=C 与 LANG=fr 不命中 → 继续走链落到 sys-locale
        let (locale, _) = resolve_locale_from(None, "", None, None, Some("C"), Some("zh_CN.UTF-8"));
        assert_eq!(locale, "zh-CN");
        // 全部不支持 → 链尾 en(无非法显式值 → 无告警)
        let (locale, invalid) =
            resolve_locale_from(None, "", None, None, Some("fr_FR.UTF-8"), Some("fr-FR"));
        assert_eq!(locale, "en");
        assert!(!invalid);
    }

    /// 空串视同未设置(NEBULA_LOCALE 与 config 两侧),auto 走系统链。
    #[test]
    fn test_detect_chain_empty_values_treated_as_unset() {
        let (locale, invalid) =
            resolve_locale_from(Some(""), "", Some("zh_CN.UTF-8"), None, None, None);
        assert_eq!(locale, "zh-CN");
        assert!(!invalid);
    }

    /// 显式入口非法值不短路:置 invalid 并继续走链(系统语言兜底),
    /// 链尾仍落 en;告警标记保留给调用方。
    #[test]
    fn test_detect_chain_invalid_explicit_value_continues_chain() {
        // NEBULA_LOCALE 非法 → 环境链 zh 兜底,invalid 仍为 true(告警保留)
        let (locale, invalid) =
            resolve_locale_from(Some("fr"), "", None, None, Some("zh_CN.UTF-8"), None);
        assert_eq!(locale, "zh-CN");
        assert!(invalid, "显式非法值必须保留告警标记");

        // config 非法 → 走链,全链无命中 → en + invalid
        let (locale, invalid) = resolve_locale_from(None, "es-ES", None, None, None, None);
        assert_eq!(locale, "en");
        assert!(invalid);
    }
}
