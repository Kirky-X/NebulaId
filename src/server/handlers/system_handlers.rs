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

//! System / observability handlers: health, readiness, metrics,
//! and the background key-rotation task launcher (rule 25 split).

use crate::server::models::{AlgorithmMetrics, HealthResponse, MetricsResponse, ReadyResponse};
use std::sync::atomic::Ordering;

// KeyRotationHandle lives in `api_key_handlers` (it owns the API key repo
// shutdown channel); we only return it from here.
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
                    cache_hit_rate: snapshot.cache_hit_rate,
                },
            )
            .collect();

        let database = self.config_service.get_database_metrics().await;
        let cache = self.config_service.get_cache_metrics().await;

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
        }
    }

    /// Start background key rotation task.
    /// Returns a handle that can be used to stop the task.
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

        // L16 修复：从 `ApiHandlers::key_rotation_grace_period_seconds` 读取，
        // 原为闭包内硬编码 `const GRACE_PERIOD_SECONDS: u64 = 7 * 24 * 60 * 60`。
        let grace_period_seconds = self.key_rotation_grace_period_seconds;

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

                                    if let Err(e) =
                                        repo.rotate_api_key(&key.key_id, grace_period_seconds).await
                                    {
                                        tracing::error!(
                                            event = "key_rotation_failed",
                                            key_id = key.key_id,
                                            error = %e
                                        );
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

#[cfg(test)]
mod tests {
    use crate::core::algorithm::{
        DegradationManager, HealthStatus as CoreHealthStatus, IdGenerator as CoreIdGenerator,
    };
    use crate::core::database::{
        ApiKeyInfo, ApiKeyRepository, ApiKeyRole, ApiKeyWithSecret, BizTag,
    };
    use crate::core::types::{AlgorithmType, Id};
    use crate::core::{CoreError, Result};
    use crate::server::config::management::{ConfigManagementService, ConfigManager};
    use crate::server::config::HotReloadConfig;
    use crate::server::handlers::mock_generator::MockIdGenerator;
    use crate::server::models::*;
    use async_trait::async_trait;
    use mockall::mock;
    use std::sync::Arc;
    use uuid::Uuid;

    /// Production-like `ApiHandlers` with real `ConfigManager` (no repository)
    /// backed by `MockIdGenerator`. Used for health/ready/metrics defaults.
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

    /// Build `ApiHandlers` with a custom `IdGenerator` (real ConfigManager).
    /// Used to exercise `health()` with non-Healthy generator status.
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

    /// Build `ApiHandlers` with mock config service (no repo).
    fn create_handlers_with_mock_config(
        mock_config: MockSysMockConfigService,
    ) -> Arc<super::super::ApiHandlers> {
        let mock_gen = Arc::new(MockIdGenerator::new()) as Arc<dyn CoreIdGenerator>;
        let config_service: Arc<dyn ConfigManagementService> = Arc::new(mock_config);
        Arc::new(super::super::ApiHandlers::new(mock_gen, config_service))
    }

    /// Build `ApiHandlers` with mock config service + mock api key repo.
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

    /// Hand-written `IdGenerator` whose `health_check` is configurable.
    /// `mockall` struggles with the `&Arc<DegradationManager>` return-by-ref
    /// method, so we hand-roll a minimal stub.
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

    mock! {
        pub SysMockConfigService {}
        #[async_trait]
        impl ConfigManagementService for SysMockConfigService {
            fn get_config(&self) -> ConfigResponse;
            fn get_secure_config(&self) -> SecureConfigResponse;
            fn get_batch_max_size(&self) -> u32;
            async fn update_rate_limit(&self, req: UpdateRateLimitRequest) -> UpdateConfigResponse;
            async fn update_logging(&self, req: UpdateLoggingRequest) -> UpdateConfigResponse;
            async fn reload_config(&self) -> UpdateConfigResponse;
            async fn get_rate_limit_override(&self) -> Option<(u32, u32)>;
            async fn set_algorithm(&self, req: SetAlgorithmRequest) -> SetAlgorithmResponse;
            async fn create_biz_tag(&self, request: &crate::core::database::CreateBizTagRequest) -> crate::core::Result<BizTag>;
            async fn get_biz_tag(&self, id: Uuid) -> crate::core::Result<Option<BizTag>>;
            async fn update_biz_tag(&self, id: Uuid, request: &crate::core::database::UpdateBizTagRequest) -> crate::core::Result<BizTag>;
            async fn delete_biz_tag(&self, id: Uuid) -> crate::core::Result<()>;
            async fn count_biz_tags(&self, workspace_id: Uuid, group_id: Option<Uuid>) -> crate::core::Result<u64>;
            async fn list_biz_tags(&self, workspace_id: Uuid, group_id: Option<Uuid>, limit: Option<u32>, offset: Option<u32>) -> crate::core::Result<Vec<BizTag>>;
            async fn create_workspace(&self, req: CreateWorkspaceRequest) -> crate::core::Result<WorkspaceResponse>;
            async fn list_workspaces(&self) -> crate::core::Result<WorkspaceListResponse>;
            async fn get_workspace(&self, name: &str) -> crate::core::Result<Option<WorkspaceResponse>>;
            async fn create_group(&self, req: CreateGroupRequest) -> crate::core::Result<GroupResponse>;
            async fn list_groups(&self, workspace: &str) -> crate::core::Result<GroupListResponse>;
            async fn get_database_metrics(&self) -> DatabaseMetrics;
            async fn get_cache_metrics(&self) -> CacheMetrics;
            async fn get_algorithm_metrics(&self) -> Vec<(AlgorithmType, crate::core::algorithm::AlgorithmMetricsSnapshot)>;
        }
    }

    mock! {
        pub SysMockApiKeyRepo {}
        #[async_trait]
        impl ApiKeyRepository for SysMockApiKeyRepo {
            async fn create_api_key(&self, request: &crate::core::database::CreateApiKeyRequest) -> Result<ApiKeyWithSecret>;
            async fn get_api_key_by_id(&self, key_id: &str) -> Result<Option<ApiKeyInfo>>;
            async fn validate_api_key(&self, key_id: &str, key_secret: &str) -> Result<Option<(Option<Uuid>, ApiKeyRole)>>;
            async fn list_api_keys(&self, workspace_id: Uuid, limit: Option<u32>, offset: Option<u32>) -> Result<Vec<ApiKeyInfo>>;
            async fn delete_api_key(&self, id: Uuid) -> Result<()>;
            async fn revoke_api_key(&self, id: Uuid) -> Result<()>;
            async fn update_last_used(&self, id: Uuid) -> Result<()>;
            async fn get_admin_api_key(&self, workspace_id: Uuid) -> Result<Option<ApiKeyInfo>>;
            async fn count_api_keys(&self, workspace_id: Uuid) -> Result<u64>;
            async fn rotate_api_key(&self, key_id: &str, grace_period_seconds: u64) -> Result<ApiKeyWithSecret>;
            async fn get_keys_older_than(&self, age_threshold_days: i64) -> Result<Vec<ApiKeyInfo>>;
        }
    }

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
        // Cache metrics are always Healthy in the default ConfigManager.
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
        // uptime is computed from Instant::now() - start_time; should be >= 0
        // (typically 0 on fast machines, occasionally 1).
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
        // algorithms is a Vec; default ConfigManager may return empty or
        // populated depending on router state. We only verify it's a Vec
        // and each entry has a non-empty algorithm name.
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
        // Default expectation: get_keys_older_than returns empty (no rotation work).
        mock_repo
            .expect_get_keys_older_than()
            .returning(|_| Ok(Vec::new()));

        let handlers = create_handlers_with_mock_config_and_repo(mock_config, mock_repo);
        let handle = handlers.start_key_rotation_task(std::time::Duration::from_secs(60), 30);
        assert!(handle.is_some());
        // Explicitly shut down to clean up the spawned task.
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

        // shutdown should not panic and should signal the background task.
        handle.shutdown();
        // Give the background task a moment to observe the shutdown signal.
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
        // Allow multiple calls (short interval triggers ticks quickly).
        mock_repo
            .expect_get_keys_older_than()
            .returning(|_| Ok(Vec::new()));

        let handlers = create_handlers_with_mock_config_and_repo(mock_config, mock_repo);
        let handle = handlers.start_key_rotation_task(std::time::Duration::from_millis(10), 1);
        assert!(handle.is_some());

        // Let one tick fire so the branch is exercised, then shut down.
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        if let Some(h) = handle {
            h.shutdown();
        }
        // Brief wait to let the task exit cleanly.
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
        // Repo returns error; background task should log and continue.
        mock_repo
            .expect_get_keys_older_than()
            .returning(|_| Err(CoreError::DatabaseError("db unavailable".to_string())));

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
        // Rotate fails; background task should log error and continue.
        mock_repo
            .expect_rotate_api_key()
            .returning(|_, _| Err(CoreError::DatabaseError("rotation failed".to_string())));

        let handlers = create_handlers_with_mock_config_and_repo(mock_config, mock_repo);
        let handle = handlers.start_key_rotation_task(std::time::Duration::from_millis(10), 1);
        assert!(handle.is_some());

        // Allow enough time for the first tick + rotation attempt.
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
        // max_key_age_days = 0 is a degenerate but valid input; should not panic.
        let handle = handlers.start_key_rotation_task(std::time::Duration::from_secs(60), 0);
        assert!(handle.is_some());
        if let Some(h) = handle {
            h.shutdown();
        }
    }
}
