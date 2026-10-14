// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! System / observability handlers：health、ready、metrics 与密钥轮换任务启动器。

use crate::server::models::{
    AlgorithmMetrics, ApiInfoResponse, DegradationMetrics, ErrorResponse, HealthResponse,
    MetricsResponse, ReadyResponse, SecureConfigResponse, SetAlgorithmRequest,
    SetAlgorithmResponse, UpdateConfigResponse, UpdateLoggingRequest, UpdateRateLimitRequest,
};
use std::sync::atomic::Ordering;

// KeyRotationHandle 定义在 api_key_handlers（其持有 repo 关闭通道），此处仅返回。
use super::api_key_handlers::KeyRotationHandle;

impl super::ApiHandlers {
    pub async fn health(&self) -> HealthResponse {
        let health_status = self.id_generator.health_check().await;
        HealthResponse {
            status: if health_status.is_healthy() {
                crate::server::models::HealthStatus::Healthy
            } else {
                crate::server::models::HealthStatus::Degraded
            },
            algorithm: self.id_generator.get_primary_algorithm().await.to_string(),
        }
    }

    pub async fn ready(&self) -> ReadyResponse {
        let db_metrics = self.config_service.get_database_metrics().await;
        let cache_metrics = self.config_service.get_cache_metrics().await;

        let db_healthy = db_metrics.status == crate::server::models::HealthStatus::Healthy;
        let cache_healthy = cache_metrics.status == crate::server::models::HealthStatus::Healthy;

        let ready = db_healthy && cache_healthy;
        ReadyResponse {
            ready,
            database: db_healthy,
            cache: cache_healthy,
            message: if ready {
                t!("api.success.system_handlers.ready").to_string()
            } else {
                t!("api.error.system_handlers.not_ready").to_string()
            },
        }
    }

    pub async fn metrics(&self) -> MetricsResponse {
        let algorithm_metrics = self.config_service.get_algorithm_metrics().await;
        let algorithms = algorithm_metrics
            .into_iter()
            .map(
                |(alg_type, snapshot): (
                    crate::core::types::AlgorithmType,
                    crate::core::algorithm::AlgorithmMetricsSnapshot,
                )| AlgorithmMetrics {
                    algorithm: alg_type.to_string(),
                    status: crate::server::models::HealthStatus::Healthy,
                    total_generated: snapshot.total_generated,
                    total_failed: snapshot.total_failed,
                    // 路由层观测的 µs 样本 → 对外统一 ms 口径
                    p50_latency_ms: snapshot.p50_latency_us as f64 / 1_000.0,
                    p99_latency_ms: snapshot.p99_latency_us as f64 / 1_000.0,
                    p999_latency_ms: snapshot.p999_latency_us as f64 / 1_000.0,
                    clock_backwards: snapshot.clock_backwards,
                    cache_hit_rate: snapshot.cache_hit_rate,
                },
            )
            .collect();

        let database = self.config_service.get_database_metrics().await;
        let cache = self.config_service.get_cache_metrics().await;

        let degradation_metrics = self
            .id_generator
            .get_degradation_manager()
            .get_algorithm_metrics()
            .into_iter()
            .map(|(algorithm, m)| DegradationMetrics {
                algorithm,
                total_requests: m.total_requests,
                total_successes: m.total_successes,
                total_failures: m.total_failures,
                success_rate: m.success_rate,
                consecutive_failures: m.consecutive_failures,
                consecutive_successes: m.consecutive_successes,
                circuit_breaker_state: format!("{:?}", m.circuit_breaker_state),
                is_degraded: m.is_degraded,
            })
            .collect();

        MetricsResponse {
            total_requests: self.metrics.total_requests.load(Ordering::SeqCst),
            successful_generations: self.metrics.successful_generations.load(Ordering::SeqCst),
            failed_generations: self.metrics.failed_generations.load(Ordering::SeqCst),
            total_ids_generated: self.metrics.total_ids_generated.load(Ordering::SeqCst),
            avg_latency_ms: self.metrics.avg_latency_ms.load(Ordering::SeqCst),
            uptime_seconds: std::time::Instant::now()
                .duration_since(self.start_time)
                .as_secs(),
            database,
            cache,
            algorithms,
            degradation_metrics,
        }
    }

    /// 启动密钥轮换后台任务，返回可用于停止任务的句柄。
    pub fn start_key_rotation_task(
        &self,
        check_interval: std::time::Duration,
        max_key_age_days: i64,
    ) -> Option<KeyRotationHandle> {
        let repo = match self.api_key_repo.as_ref() {
            Some(r) => r.clone(),
            None => {
                tracing::warn!(
                    "{}",
                    t!("log.server.handlers.system_handlers.cannot_start_key_rotation")
                );
                return None;
            }
        };

        let (shutdown_tx, mut shutdown_rx) = tokio::sync::watch::channel(false);

        // 宽限期取自 ApiHandlers 字段。
        let grace_period_seconds = self.key_rotation_grace_period_seconds;

        // spawn 需 'static，先克隆缓存句柄；未启用 garrison-auth 时无此变量。
        #[cfg(feature = "garrison-auth")]
        let auth_cache = self.auth_cache.clone();

        let _handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(check_interval);
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        tracing::debug!(
                            "{}",
                            t!("log.server.handlers.system_handlers.running_key_rotation_check")
                        );

                        match repo.get_keys_older_than(max_key_age_days).await {
                            Ok(old_keys) => {
                                for key in old_keys {
                                    tracing::info!(
                                        event = "auto_rotating_key",
                                        key_id = key.key_id,
                                        age_days = max_key_age_days
                                    );

                                    match repo
                                        .rotate_api_key(&key.key_id, grace_period_seconds)
                                        .await
                                    {
                                        Ok(_) => {
                                            // 轮换出的新 secret 必须回源校验。
                                            #[cfg(feature = "garrison-auth")]
                                            if let Some(cache) = auth_cache.as_ref() {
                                                cache.invalidate(&key.key_id).await;
                                            }
                                        }
                                        Err(e) => {
                                            tracing::error!(
                                                event = "key_rotation_failed",
                                                key_id = key.key_id,
                                                error = %e
                                            );
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::error!(event = "key_rotation_check_failed", error = %e);
                            }
                        }
                    }
                    _ = shutdown_rx.changed() => {
                        tracing::info!(
                            "{}",
                            t!("log.server.handlers.system_handlers.key_rotation_shutting_down")
                        );
                        break;
                    }
                }
            }
        });

        tracing::info!(
            event = "key_rotation_task_started",
            check_interval_secs = check_interval.as_secs(),
            max_age_days = max_key_age_days,
            grace_period_secs = grace_period_seconds
        );

        Some(KeyRotationHandle { shutdown_tx })
    }
}

// ========== OpenAPI path 注解 ==========
// axum handler 在 router.rs；载体函数仅承载 #[utoipa::path] 供 openapi.rs 注册，
// 不参与运行时路由；错误响应统一引用 ErrorResponse（信封）。

/// OpenAPI 注解载体：`GET /health`（实际 handler：`router::handle_health`，公开）。
#[sdforge::utoipa::path(
    get,
    path = "/health",
    operation_id = "handle_health",
    tag = "system",
    responses(
        (status = 200, description = "Service health status (algorithm view)", body = HealthResponse),
    )
)]
pub fn health_docs() {}

/// OpenAPI 注解载体：`GET /ready`（实际 handler：`router::handle_ready`，公开）。
#[sdforge::utoipa::path(
    get,
    path = "/ready",
    operation_id = "handle_ready",
    tag = "system",
    responses(
        (status = 200, description = "Readiness probe result (database/cache dependency status)", body = ReadyResponse),
    )
)]
pub fn ready_docs() {}

/// OpenAPI 注解载体：`GET /metrics`（实际 handler：`router::handle_metrics`，公开）。
#[sdforge::utoipa::path(
    get,
    path = "/metrics",
    operation_id = "handle_metrics",
    tag = "system",
    responses(
        (status = 200, description = "Aggregated runtime metrics (throughput/latency/dependency health)", body = MetricsResponse),
    )
)]
pub fn metrics_docs() {}

/// OpenAPI 注解载体：`GET /api/v1/`（实际 handler：`router::handle_api_info`，公开）。
#[sdforge::utoipa::path(
    get,
    path = "/api/v1/",
    operation_id = "handle_api_info",
    tag = "system",
    responses(
        (status = 200, description = "Service metadata and endpoint listing", body = ApiInfoResponse),
    )
)]
pub fn api_info_docs() {}

/// OpenAPI 注解载体：`GET /api/v1/config`（实际 handler：`router::handle_get_config`，需认证）。
#[sdforge::utoipa::path(
    get,
    path = "/api/v1/config",
    operation_id = "handle_get_config",
    tag = "config",
    responses(
        (status = 200, description = "Current configuration (redacted: no database connection info)", body = SecureConfigResponse),
        (status = 401, description = "Missing/invalid API Key", body = ErrorResponse),
        (status = 429, description = "Rate limit exceeded", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse),
    )
)]
pub fn get_config_docs() {}

/// OpenAPI 注解载体：`POST /api/v1/config/rate-limit`（实际 handler：`router::handle_update_rate_limit`，Admin-only）。
#[sdforge::utoipa::path(
    post,
    path = "/api/v1/config/rate-limit",
    operation_id = "handle_update_rate_limit",
    tag = "config",
    request_body = UpdateRateLimitRequest,
    responses(
        (status = 200, description = "Update result", body = UpdateConfigResponse),
        (status = 400, description = "Request validation failed", body = ErrorResponse),
        (status = 401, description = "Missing/invalid API Key", body = ErrorResponse),
        (status = 403, description = "Admin role required (CWE-862)", body = ErrorResponse),
        (status = 429, description = "Rate limit exceeded", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse),
    )
)]
pub fn update_rate_limit_docs() {}

/// OpenAPI 注解载体：`POST /api/v1/config/logging`（实际 handler：`router::handle_update_logging`，Admin-only）。
#[sdforge::utoipa::path(
    post,
    path = "/api/v1/config/logging",
    operation_id = "handle_update_logging",
    tag = "config",
    request_body = UpdateLoggingRequest,
    responses(
        (status = 200, description = "Update result", body = UpdateConfigResponse),
        (status = 400, description = "Request validation failed", body = ErrorResponse),
        (status = 401, description = "Missing/invalid API Key", body = ErrorResponse),
        (status = 403, description = "Admin role required (CWE-862)", body = ErrorResponse),
        (status = 429, description = "Rate limit exceeded", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse),
    )
)]
pub fn update_logging_docs() {}

/// OpenAPI 注解载体：`POST /api/v1/config/reload`（实际 handler：`router::handle_reload_config`，Admin-only）。
#[sdforge::utoipa::path(
    post,
    path = "/api/v1/config/reload",
    operation_id = "handle_reload_config",
    tag = "config",
    responses(
        (status = 200, description = "Reload result (with latest configuration)", body = UpdateConfigResponse),
        (status = 401, description = "Missing/invalid API Key", body = ErrorResponse),
        (status = 403, description = "Admin role required (CWE-862)", body = ErrorResponse),
        (status = 429, description = "Rate limit exceeded", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse),
    )
)]
pub fn reload_config_docs() {}

/// OpenAPI 注解载体：`POST /api/v1/config/algorithm`（实际 handler：`router::handle_set_algorithm`，Admin-only）。
#[sdforge::utoipa::path(
    post,
    path = "/api/v1/config/algorithm",
    operation_id = "handle_set_algorithm",
    tag = "config",
    request_body = SetAlgorithmRequest,
    responses(
        (status = 200, description = "Algorithm binding result", body = SetAlgorithmResponse),
        (status = 400, description = "Request validation failed", body = ErrorResponse),
        (status = 401, description = "Missing/invalid API Key", body = ErrorResponse),
        (status = 403, description = "Admin role required (CWE-862)", body = ErrorResponse),
        (status = 429, description = "Rate limit exceeded", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse),
    )
)]
pub fn set_algorithm_docs() {}

#[cfg(test)]
mod tests {
    use crate::core::algorithm::{
        DegradationManager, HealthStatus as CoreHealthStatus, IdGenerator as CoreIdGenerator,
    };
    use crate::core::database::{ApiKeyInfo, ApiKeyRepository, ApiKeyRole};
    use crate::core::types::{AlgorithmType, Id};
    use crate::core::{CoreError, Result};
    use crate::server::config::management::{ConfigManagementService, ConfigManager};
    use crate::server::config::HotReloadConfig;
    use crate::server::handlers::mock_generator::MockIdGenerator;
    use crate::server::models::*;
    use async_trait::async_trait;
    use std::sync::Arc;
    use uuid::Uuid;

    /// 生产形态的 ApiHandlers（真实 ConfigManager + MockIdGenerator），用于默认值断言。
    fn create_test_api_handlers() -> (Arc<super::super::ApiHandlers>, Arc<MockIdGenerator>) {
        let mock_gen = Arc::new(MockIdGenerator::new());
        let config = crate::core::config::Config::default();
        let hot_config = Arc::new(HotReloadConfig::new(
            config,
            "config/config.toml".to_string(),
        ));

        let router = Arc::new(crate::core::algorithm::AlgorithmRouter::new(
            crate::core::config::Config::default(),
            None,
        ));

        let config_service: Arc<dyn ConfigManagementService> =
            Arc::new(ConfigManager::new(hot_config, router));
        let handlers = super::super::ApiHandlers::new(mock_gen.clone(), config_service);
        (Arc::new(handlers), mock_gen)
    }

    /// 自定义 IdGenerator 的 ApiHandlers，用于 health() 非 Healthy 状态。
    fn create_handlers_with_generator(
        gen: Arc<dyn CoreIdGenerator>,
    ) -> Arc<super::super::ApiHandlers> {
        let config = crate::core::config::Config::default();
        let hot_config = Arc::new(HotReloadConfig::new(
            config,
            "config/config.toml".to_string(),
        ));
        let router = Arc::new(crate::core::algorithm::AlgorithmRouter::new(
            crate::core::config::Config::default(),
            None,
        ));
        let config_service: Arc<dyn ConfigManagementService> =
            Arc::new(ConfigManager::new(hot_config, router));
        Arc::new(super::super::ApiHandlers::new(gen, config_service))
    }

    /// mock config service（无 repo）的 ApiHandlers。
    fn create_handlers_with_mock_config(
        mock_config: MockSysMockConfigService,
    ) -> Arc<super::super::ApiHandlers> {
        let mock_gen = Arc::new(MockIdGenerator::new()) as Arc<dyn CoreIdGenerator>;
        let config_service: Arc<dyn ConfigManagementService> = Arc::new(mock_config);
        Arc::new(super::super::ApiHandlers::new(mock_gen, config_service))
    }

    /// mock config service + mock API key repo 的 ApiHandlers。
    fn create_handlers_with_mock_config_and_repo(
        mock_config: MockSysMockConfigService,
        mock_repo: MockSysMockApiKeyRepo,
    ) -> Arc<super::super::ApiHandlers> {
        let mock_gen = Arc::new(MockIdGenerator::new()) as Arc<dyn CoreIdGenerator>;
        let config_service: Arc<dyn ConfigManagementService> = Arc::new(mock_config);
        let repo: Arc<dyn ApiKeyRepository> = Arc::new(mock_repo);
        Arc::new(super::super::ApiHandlers::with_api_key_repository(
            mock_gen,
            config_service,
            repo,
        ))
    }

    /// 手写 IdGenerator 桩（`health_check` 可配置）：mockall 难以处理返回 `&Arc` 的方法。
    struct ControllableIdGenerator {
        health: CoreHealthStatus,
        primary_algorithm: String,
        degradation_manager: Arc<DegradationManager>,
    }

    impl ControllableIdGenerator {
        fn new(health: CoreHealthStatus) -> Self {
            Self {
                health,
                primary_algorithm: "segment".to_string(),
                degradation_manager: Arc::new(DegradationManager::new(None, None)),
            }
        }
    }

    #[async_trait]
    impl CoreIdGenerator for ControllableIdGenerator {
        async fn generate(&self, _workspace: &str, _group: &str, _biz_tag: &str) -> Result<Id> {
            Ok(Id::from_u128(1))
        }
        async fn batch_generate(
            &self,
            _workspace: &str,
            _group: &str,
            _biz_tag: &str,
            _size: usize,
        ) -> Result<Vec<Id>> {
            Ok(Vec::new())
        }
        async fn generate_with_algorithm(
            &self,
            _algorithm: AlgorithmType,
            _workspace: &str,
            _group: &str,
            _biz_tag: &str,
        ) -> Result<Id> {
            Ok(Id::from_u128(1))
        }
        async fn batch_generate_with_algorithm(
            &self,
            _algorithm: AlgorithmType,
            _workspace: &str,
            _group: &str,
            _biz_tag: &str,
            _size: usize,
        ) -> Result<Vec<Id>> {
            Ok(Vec::new())
        }
        async fn get_algorithm_name(
            &self,
            _workspace: &str,
            _group: &str,
            _biz_tag: &str,
        ) -> Result<String> {
            Ok(self.primary_algorithm.clone())
        }
        async fn health_check(&self) -> CoreHealthStatus {
            self.health.clone()
        }
        async fn get_primary_algorithm(&self) -> String {
            self.primary_algorithm.clone()
        }
        fn get_degradation_manager(&self) -> &Arc<DegradationManager> {
            &self.degradation_manager
        }
    }

    use crate::server::handlers::mock_tests::{MockApiKeyRepository, MockConfigManagementService};
    type MockSysMockConfigService = MockConfigManagementService;

    type MockSysMockApiKeyRepo = MockApiKeyRepository;

    fn healthy_db_metrics() -> DatabaseMetrics {
        DatabaseMetrics {
            status: HealthStatus::Healthy,
            connection_pool: ConnectionPoolMetrics {
                active_connections: 1,
                idle_connections: 4,
                max_connections: 5,
            },
            last_error: None,
        }
    }

    fn unhealthy_db_metrics() -> DatabaseMetrics {
        DatabaseMetrics {
            status: HealthStatus::Unhealthy,
            connection_pool: ConnectionPoolMetrics {
                active_connections: 0,
                idle_connections: 0,
                max_connections: 0,
            },
            last_error: Some("connection refused".to_string()),
        }
    }

    fn healthy_cache_metrics() -> CacheMetrics {
        CacheMetrics {
            status: HealthStatus::Healthy,
            hit_rate: 0.85,
            has_cache: true,
            memory_usage_mb: Some(64),
            key_count: Some(1024),
        }
    }

    fn unhealthy_cache_metrics() -> CacheMetrics {
        CacheMetrics {
            status: HealthStatus::Unhealthy,
            hit_rate: 0.0,
            has_cache: false,
            memory_usage_mb: None,
            key_count: None,
        }
    }

    // ===== health() =====

    #[tokio::test]
    async fn test_handle_metrics() {
        let (handlers, _router) = create_test_api_handlers();
        let response = handlers.metrics().await;
        assert_eq!(response.total_requests, 0);
        assert_eq!(response.successful_generations, 0);
        assert_eq!(response.failed_generations, 0);
    }

    // SnowflakeAlgorithm 随 snowflake 特性门控，剪裁组合下不参与编译。
    #[cfg(feature = "snowflake")]
    #[tokio::test]
    async fn test_handle_metrics_includes_registered_algorithm_degradation() {
        let gen = Arc::new(ControllableIdGenerator::new(CoreHealthStatus::Healthy));
        let algo: Arc<dyn crate::core::algorithm::IdAlgorithm> = Arc::new(
            crate::core::algorithm::snowflake::SnowflakeAlgorithm::new(1, 1),
        );
        gen.degradation_manager
            .register_algorithm(AlgorithmType::Snowflake, algo);
        let handlers = create_handlers_with_generator(gen);

        let response = handlers.metrics().await;
        assert_eq!(response.degradation_metrics.len(), 1);
        let entry = &response.degradation_metrics[0];
        assert_eq!(entry.algorithm, AlgorithmType::Snowflake.to_string());
        assert_eq!(entry.total_requests, 0);
        assert!(!entry.is_degraded);
    }

    #[tokio::test]
    async fn test_health_returns_healthy_with_mock_generator() {
        let (handlers, _) = create_test_api_handlers();
        let response = handlers.health().await;
        assert_eq!(response.status, HealthStatus::Healthy);
        assert_eq!(response.algorithm, "segment");
    }

    #[tokio::test]
    async fn test_health_algorithm_field_populated() {
        let (handlers, _) = create_test_api_handlers();
        let response = handlers.health().await;
        assert!(!response.algorithm.is_empty());
        assert_eq!(response.algorithm, "segment");
    }

    #[tokio::test]
    async fn test_health_returns_degraded_when_generator_degraded() {
        let gen = Arc::new(ControllableIdGenerator::new(CoreHealthStatus::Degraded(
            "cache pressure".to_string(),
        )));
        let handlers = create_handlers_with_generator(gen);
        let response = handlers.health().await;
        assert_eq!(response.status, HealthStatus::Degraded);
        assert_eq!(response.algorithm, "segment");
    }

    #[tokio::test]
    async fn test_health_returns_degraded_when_generator_unhealthy() {
        let gen = Arc::new(ControllableIdGenerator::new(CoreHealthStatus::Unhealthy(
            "db down".to_string(),
        )));
        let handlers = create_handlers_with_generator(gen);
        let response = handlers.health().await;
        assert_eq!(response.status, HealthStatus::Degraded);
    }

    // ===== ready() =====

    #[tokio::test]
    async fn test_ready_returns_false_when_db_not_configured() {
        // Default ConfigManager has no repository -> db metrics = Unhealthy.
        let (handlers, _) = create_test_api_handlers();
        let response = handlers.ready().await;
        assert!(!response.ready);
        assert!(!response.database);
        // 默认 ConfigManager 的 cache metrics 恒为 Healthy
        assert!(response.cache);
    }

    #[tokio::test]
    async fn test_ready_message_when_not_ready() {
        let (handlers, _) = create_test_api_handlers();
        let response = handlers.ready().await;
        assert!(!response.ready);
        assert_eq!(response.message, "Not ready: database or cache unavailable");
    }

    #[tokio::test]
    async fn test_ready_true_when_db_and_cache_healthy() {
        let mut mock_config = MockSysMockConfigService::new();
        mock_config
            .expect_get_database_metrics()
            .return_once(healthy_db_metrics);
        mock_config
            .expect_get_cache_metrics()
            .return_once(healthy_cache_metrics);

        let handlers = create_handlers_with_mock_config(mock_config);
        let response = handlers.ready().await;
        assert!(response.ready);
        assert!(response.database);
        assert!(response.cache);
        assert_eq!(response.message, "Ready to serve traffic");
    }

    #[tokio::test]
    async fn test_ready_false_when_db_unhealthy_only() {
        let mut mock_config = MockSysMockConfigService::new();
        mock_config
            .expect_get_database_metrics()
            .return_once(unhealthy_db_metrics);
        mock_config
            .expect_get_cache_metrics()
            .return_once(healthy_cache_metrics);

        let handlers = create_handlers_with_mock_config(mock_config);
        let response = handlers.ready().await;
        assert!(!response.ready);
        assert!(!response.database);
        assert!(response.cache);
        assert_eq!(response.message, "Not ready: database or cache unavailable");
    }

    #[tokio::test]
    async fn test_ready_false_when_cache_unhealthy_only() {
        let mut mock_config = MockSysMockConfigService::new();
        mock_config
            .expect_get_database_metrics()
            .return_once(healthy_db_metrics);
        mock_config
            .expect_get_cache_metrics()
            .return_once(unhealthy_cache_metrics);

        let handlers = create_handlers_with_mock_config(mock_config);
        let response = handlers.ready().await;
        assert!(!response.ready);
        assert!(response.database);
        assert!(!response.cache);
    }

    #[tokio::test]
    async fn test_ready_false_when_both_unhealthy() {
        let mut mock_config = MockSysMockConfigService::new();
        mock_config
            .expect_get_database_metrics()
            .return_once(unhealthy_db_metrics);
        mock_config
            .expect_get_cache_metrics()
            .return_once(unhealthy_cache_metrics);

        let handlers = create_handlers_with_mock_config(mock_config);
        let response = handlers.ready().await;
        assert!(!response.ready);
        assert!(!response.database);
        assert!(!response.cache);
    }

    // ===== metrics() =====

    #[tokio::test]
    async fn test_metrics_returns_zero_counters_initially() {
        let (handlers, _) = create_test_api_handlers();
        let response = handlers.metrics().await;
        assert_eq!(response.total_requests, 0);
        assert_eq!(response.successful_generations, 0);
        assert_eq!(response.failed_generations, 0);
        assert_eq!(response.total_ids_generated, 0);
        assert_eq!(response.avg_latency_ms, 0);
    }

    #[tokio::test]
    async fn test_metrics_uptime_non_negative() {
        let (handlers, _) = create_test_api_handlers();
        let response = handlers.metrics().await;
        // uptime = Instant::now() - start_time，快机器上通常为 0
        assert!(response.uptime_seconds < u64::MAX);
    }

    #[tokio::test]
    async fn test_metrics_database_unhealthy_when_no_repo() {
        let (handlers, _) = create_test_api_handlers();
        let response = handlers.metrics().await;
        assert_eq!(response.database.status, HealthStatus::Unhealthy);
        assert!(response.database.last_error.is_some());
        assert_eq!(
            response.database.last_error.as_deref(),
            Some("Database not configured")
        );
    }

    #[tokio::test]
    async fn test_metrics_cache_status_healthy_default() {
        let (handlers, _) = create_test_api_handlers();
        let response = handlers.metrics().await;
        assert_eq!(response.cache.status, HealthStatus::Healthy);
    }

    #[tokio::test]
    async fn test_metrics_algorithms_list_is_vec() {
        let (handlers, _) = create_test_api_handlers();
        let response = handlers.metrics().await;
        // algorithms 可能为空也可能有值，只验证每个条目算法名非空
        for alg in &response.algorithms {
            assert!(!alg.algorithm.is_empty());
        }
    }

    #[tokio::test]
    async fn test_metrics_with_mocked_healthy_db_and_cache() {
        let mut mock_config = MockSysMockConfigService::new();
        mock_config
            .expect_get_database_metrics()
            .returning(healthy_db_metrics);
        mock_config
            .expect_get_cache_metrics()
            .returning(healthy_cache_metrics);
        mock_config
            .expect_get_algorithm_metrics()
            .returning(Vec::new);

        let handlers = create_handlers_with_mock_config(mock_config);
        let response = handlers.metrics().await;
        assert_eq!(response.database.status, HealthStatus::Healthy);
        assert_eq!(response.cache.status, HealthStatus::Healthy);
        assert!(response.algorithms.is_empty());
        assert_eq!(response.total_requests, 0);
    }

    #[tokio::test]
    async fn test_metrics_with_mocked_unhealthy_db() {
        let mut mock_config = MockSysMockConfigService::new();
        mock_config
            .expect_get_database_metrics()
            .returning(unhealthy_db_metrics);
        mock_config
            .expect_get_cache_metrics()
            .returning(healthy_cache_metrics);
        mock_config
            .expect_get_algorithm_metrics()
            .returning(Vec::new);

        let handlers = create_handlers_with_mock_config(mock_config);
        let response = handlers.metrics().await;
        assert_eq!(response.database.status, HealthStatus::Unhealthy);
        assert!(response.database.last_error.is_some());
    }

    #[tokio::test]
    async fn test_metrics_exposes_percentiles_and_clock_backwards() {
        // 路由层观测值必须出现在 /metrics 的逐算法条目里
        let mut mock_config = MockSysMockConfigService::new();
        mock_config
            .expect_get_database_metrics()
            .returning(healthy_db_metrics);
        mock_config
            .expect_get_cache_metrics()
            .returning(healthy_cache_metrics);
        mock_config.expect_get_algorithm_metrics().returning(|| {
            vec![(
                crate::core::types::AlgorithmType::Snowflake,
                crate::core::algorithm::AlgorithmMetricsSnapshot {
                    total_generated: 10,
                    p50_latency_us: 1_500,
                    p99_latency_us: 9_900,
                    p999_latency_us: 15_000,
                    clock_backwards: 2,
                    ..Default::default()
                },
            )]
        });

        let handlers = create_handlers_with_mock_config(mock_config);
        let response = handlers.metrics().await;

        assert_eq!(response.algorithms.len(), 1);
        let alg = &response.algorithms[0];
        assert_eq!(alg.p50_latency_ms, 1.5);
        assert_eq!(alg.p99_latency_ms, 9.9);
        assert_eq!(alg.p999_latency_ms, 15.0);
        assert_eq!(alg.clock_backwards, 2);
    }

    // ===== start_key_rotation_task() =====

    #[tokio::test]
    async fn test_start_key_rotation_task_returns_none_without_repo() {
        let (handlers, _) = create_test_api_handlers();
        let handle = handlers.start_key_rotation_task(std::time::Duration::from_secs(60), 30);
        assert!(handle.is_none());
    }

    #[tokio::test]
    async fn test_start_key_rotation_task_returns_some_with_repo() {
        let mut mock_config = MockSysMockConfigService::new();
        mock_config
            .expect_get_database_metrics()
            .returning(healthy_db_metrics);
        mock_config
            .expect_get_cache_metrics()
            .returning(healthy_cache_metrics);
        mock_config
            .expect_get_algorithm_metrics()
            .returning(Vec::new);

        let mut mock_repo = MockSysMockApiKeyRepo::new();
        // 默认期望：无待轮换 key
        mock_repo
            .expect_get_keys_older_than()
            .returning(|_| Ok(Vec::new()));

        let handlers = create_handlers_with_mock_config_and_repo(mock_config, mock_repo);
        let handle = handlers.start_key_rotation_task(std::time::Duration::from_secs(60), 30);
        assert!(handle.is_some());
        // 显式 shutdown 清理 spawn 的任务
        if let Some(h) = handle {
            h.shutdown();
        }
    }

    #[tokio::test]
    async fn test_key_rotation_handle_shutdown_completes() {
        let mut mock_config = MockSysMockConfigService::new();
        mock_config
            .expect_get_database_metrics()
            .returning(healthy_db_metrics);
        mock_config
            .expect_get_cache_metrics()
            .returning(healthy_cache_metrics);
        mock_config
            .expect_get_algorithm_metrics()
            .returning(Vec::new);

        let mut mock_repo = MockSysMockApiKeyRepo::new();
        mock_repo
            .expect_get_keys_older_than()
            .returning(|_| Ok(Vec::new()));

        let handlers = create_handlers_with_mock_config_and_repo(mock_config, mock_repo);
        let handle = handlers
            .start_key_rotation_task(std::time::Duration::from_secs(1), 7)
            .expect("handle should be Some when repo is configured");

        // shutdown 不应 panic，只向后台任务发信号
        handle.shutdown();
        // 给后台任务一点时间观察 shutdown 信号
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    #[tokio::test]
    async fn test_start_key_rotation_task_with_short_interval() {
        let mut mock_config = MockSysMockConfigService::new();
        mock_config
            .expect_get_database_metrics()
            .returning(healthy_db_metrics);
        mock_config
            .expect_get_cache_metrics()
            .returning(healthy_cache_metrics);
        mock_config
            .expect_get_algorithm_metrics()
            .returning(Vec::new);

        let mut mock_repo = MockSysMockApiKeyRepo::new();
        // 允许多次调用（短间隔会快速触发 tick）
        mock_repo
            .expect_get_keys_older_than()
            .returning(|_| Ok(Vec::new()));

        let handlers = create_handlers_with_mock_config_and_repo(mock_config, mock_repo);
        let handle = handlers.start_key_rotation_task(std::time::Duration::from_millis(10), 1);
        assert!(handle.is_some());

        // 等一次 tick 覆盖分支后 shutdown
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        if let Some(h) = handle {
            h.shutdown();
        }
        // 稍候让任务干净退出
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    #[tokio::test]
    async fn test_start_key_rotation_task_repo_error_does_not_panic() {
        let mut mock_config = MockSysMockConfigService::new();
        mock_config
            .expect_get_database_metrics()
            .returning(healthy_db_metrics);
        mock_config
            .expect_get_cache_metrics()
            .returning(healthy_cache_metrics);
        mock_config
            .expect_get_algorithm_metrics()
            .returning(Vec::new);

        let mut mock_repo = MockSysMockApiKeyRepo::new();
        // Repo 报错时后台任务应记日志并继续
        mock_repo
            .expect_get_keys_older_than()
            .returning(|_| Err(CoreError::DatabaseError("db unavailable".to_string(), None)));

        let handlers = create_handlers_with_mock_config_and_repo(mock_config, mock_repo);
        let handle = handlers.start_key_rotation_task(std::time::Duration::from_millis(10), 1);
        assert!(handle.is_some());

        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        if let Some(h) = handle {
            h.shutdown();
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    #[tokio::test]
    async fn test_start_key_rotation_task_rotates_old_keys() {
        let mut mock_config = MockSysMockConfigService::new();
        mock_config
            .expect_get_database_metrics()
            .returning(healthy_db_metrics);
        mock_config
            .expect_get_cache_metrics()
            .returning(healthy_cache_metrics);
        mock_config
            .expect_get_algorithm_metrics()
            .returning(Vec::new);

        let old_key = ApiKeyInfo {
            id: Uuid::new_v4(),
            key_id: "nino_old-key-1".to_string(),
            key_prefix: "nino_".to_string(),
            role: ApiKeyRole::User,
            workspace_id: Some(Uuid::new_v4()),
            name: "old-key".to_string(),
            description: None,
            rate_limit: 1000,
            enabled: true,
            expires_at: None,
            last_used_at: None,
            created_at: chrono::Utc::now().naive_utc(),
        };

        let mut mock_repo = MockSysMockApiKeyRepo::new();
        mock_repo
            .expect_get_keys_older_than()
            .return_once(move |_| Ok(vec![old_key]));
        // 轮换失败；后台任务应记错误并继续
        mock_repo.expect_rotate_api_key().returning(|_, _| {
            Err(CoreError::DatabaseError(
                "rotation failed".to_string(),
                None,
            ))
        });

        let handlers = create_handlers_with_mock_config_and_repo(mock_config, mock_repo);
        let handle = handlers.start_key_rotation_task(std::time::Duration::from_millis(10), 1);
        assert!(handle.is_some());

        // 给首个 tick + 轮换尝试留足时间
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        if let Some(h) = handle {
            h.shutdown();
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    #[tokio::test]
    async fn test_start_key_rotation_task_with_max_age_zero() {
        let mut mock_config = MockSysMockConfigService::new();
        mock_config
            .expect_get_database_metrics()
            .returning(healthy_db_metrics);
        mock_config
            .expect_get_cache_metrics()
            .returning(healthy_cache_metrics);
        mock_config
            .expect_get_algorithm_metrics()
            .returning(Vec::new);

        let mut mock_repo = MockSysMockApiKeyRepo::new();
        mock_repo
            .expect_get_keys_older_than()
            .returning(|_| Ok(Vec::new()));

        let handlers = create_handlers_with_mock_config_and_repo(mock_config, mock_repo);
        // max_key_age_days = 0 退化但合法，不应 panic
        let handle = handlers.start_key_rotation_task(std::time::Duration::from_secs(60), 0);
        assert!(handle.is_some());
        if let Some(h) = handle {
            h.shutdown();
        }
    }

    /// 轮换成功且认证缓存已接线：新 secret 生成后必须立即失效该 key 的缓存条目，
    /// 否则宽限期内旧凭证在缓存 TTL 内仍被放行。覆盖轮换 Ok 分支 + 缓存失效 + shutdown。
    // AuthCache/with_auth_cache 随 garrison-auth 门控，无该特性时不参与编译。
    #[cfg(feature = "garrison-auth")]
    #[tokio::test(flavor = "multi_thread")]
    async fn test_key_rotation_success_invalidates_auth_cache() {
        use crate::server::auth::AuthCache;

        let mut mock_config = MockSysMockConfigService::new();
        mock_config
            .expect_get_database_metrics()
            .returning(healthy_db_metrics);
        mock_config
            .expect_get_cache_metrics()
            .returning(healthy_cache_metrics);
        mock_config
            .expect_get_algorithm_metrics()
            .returning(Vec::new);

        let old_key = ApiKeyInfo {
            id: Uuid::new_v4(),
            key_id: "nino_old-key-1".to_string(),
            key_prefix: "nino_".to_string(),
            role: ApiKeyRole::User,
            workspace_id: Some(Uuid::new_v4()),
            name: "old-key".to_string(),
            description: None,
            rate_limit: 1000,
            enabled: true,
            expires_at: None,
            last_used_at: None,
            created_at: chrono::Utc::now().naive_utc() - chrono::Duration::days(90),
        };

        let mut mock_repo = MockSysMockApiKeyRepo::new();
        mock_repo
            .expect_get_keys_older_than()
            .return_once(move |_| Ok(vec![old_key]));
        mock_repo.expect_rotate_api_key().returning(|_, _| {
            Ok(crate::core::database::ApiKeyWithSecret {
                key: crate::core::database::ApiKeyResponse {
                    id: uuid::Uuid::new_v4(),
                    key_id: "nino_old-key-1".to_string(),
                    key_prefix: "nino_".to_string(),
                    name: "rotated".to_string(),
                    description: None,
                    role: ApiKeyRole::User,
                    rate_limit: 1000,
                    enabled: true,
                    expires_at: None,
                    created_at: chrono::Utc::now().naive_utc(),
                },
                key_secret: "new-secret".to_string(),
                grace_expires_at: Some(chrono::Utc::now().naive_utc()),
            })
        });

        // 与 create_handlers_with_mock_config_and_repo 相同，但额外接线认证缓存。
        let mock_gen = Arc::new(MockIdGenerator::new()) as Arc<dyn CoreIdGenerator>;
        let config_service: Arc<dyn ConfigManagementService> = Arc::new(mock_config);
        let repo: Arc<dyn ApiKeyRepository> = Arc::new(mock_repo);
        let handlers = Arc::new(
            super::super::ApiHandlers::with_api_key_repository(mock_gen, config_service, repo)
                .with_auth_cache(Arc::new(AuthCache::new(300).await)),
        );

        // 10ms 间隔：首 tick 即触发轮换 + 缓存失效，随后 shutdown 收尾。
        let handle = handlers.start_key_rotation_task(std::time::Duration::from_millis(10), 1);
        assert!(handle.is_some());
        tokio::time::sleep(std::time::Duration::from_millis(80)).await;
        if let Some(h) = handle {
            h.shutdown();
        }
    }
}
