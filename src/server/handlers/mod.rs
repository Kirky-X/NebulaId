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

//! API handlers for Nebula ID.
//!
//! `ApiHandlers` struct + constructors live here; per-domain method impls
//! are split into sub-modules (`id_handlers`, `system_handlers`,
//! `biz_tag_handlers`, `workspace_handlers`, `api_key_handlers`)
//! (rule 25: mod.rs 只放 trait + pub struct + re-export).

use crate::core::database::ApiKeyRepository;
use crate::server::config::management::ConfigManagementService;
use std::sync::Arc;

pub mod api_key_handlers;
pub mod biz_tag_handlers;
pub mod helpers;
pub mod id_handlers;
// pre-existing test helper module (MockIdGenerator); not part of T027-T033 split,
// retained from pre-refactor codebase (T047 convergence annotation).
pub mod mock_generator;
pub mod system_handlers;
pub mod workspace_handlers;

pub use api_key_handlers::KeyRotationHandle;

/// Top-level API handler aggregating ID generator, metrics, config service
/// and optional API key repository.
pub struct ApiHandlers {
    pub(super) id_generator: Arc<dyn crate::core::algorithm::IdGenerator>,
    pub(super) metrics: ApiMetrics,
    pub(super) start_time: std::time::Instant,
    pub(super) config_service: Arc<dyn ConfigManagementService>,
    pub(super) api_key_repo: Option<Arc<dyn ApiKeyRepository>>,
}

#[derive(Default)]
pub struct ApiMetrics {
    pub total_requests: std::sync::atomic::AtomicU64,
    pub successful_generations: std::sync::atomic::AtomicU64,
    pub failed_generations: std::sync::atomic::AtomicU64,
    pub total_ids_generated: std::sync::atomic::AtomicU64,
    pub avg_latency_ms: std::sync::atomic::AtomicU64,
}

impl ApiHandlers {
    pub fn new(
        id_generator: Arc<dyn crate::core::algorithm::IdGenerator>,
        config_service: Arc<dyn ConfigManagementService>,
    ) -> Self {
        Self {
            id_generator,
            metrics: ApiMetrics::default(),
            start_time: std::time::Instant::now(),
            config_service,
            api_key_repo: None,
        }
    }

    pub fn with_api_key_repository(
        id_generator: Arc<dyn crate::core::algorithm::IdGenerator>,
        config_service: Arc<dyn ConfigManagementService>,
        api_key_repo: Arc<dyn ApiKeyRepository>,
    ) -> Self {
        Self {
            id_generator,
            metrics: ApiMetrics::default(),
            start_time: std::time::Instant::now(),
            config_service,
            api_key_repo: Some(api_key_repo),
        }
    }

    pub fn get_config_service(&self) -> Arc<dyn ConfigManagementService> {
        self.config_service.clone()
    }

    /// Shut down a previously started key rotation background task.
    ///
    /// Delegate entry point on `ApiHandlers` (aligns with spec T033 wording);
    /// the actual shutdown signalling stays on `KeyRotationHandle::shutdown`
    /// (T045 convergence: closes the partial gap where spec listed
    /// `shutdown` under `ApiHandlers` but impl placed it on the handle).
    pub fn shutdown(&self, handle: KeyRotationHandle) {
        handle.shutdown();
    }
}

#[cfg(test)]
mod mock_tests {
    use super::*;
    use crate::core::algorithm::AlgorithmMetricsSnapshot;
    use crate::core::database::{
        ApiKey, ApiKeyInfo, ApiKeyRepository, ApiKeyResponse as CoreApiKeyResponse, ApiKeyRole,
        ApiKeyWithSecret, BizTag, CreateApiKeyRequest as CoreCreateApiKeyRequest,
    };
    use crate::core::types::AlgorithmType;
    use crate::core::{CoreError, Result};
    use crate::server::config::management::ConfigManagementService;
    use crate::server::handlers::mock_generator::MockIdGenerator;
    use crate::server::models::*;
    use mockall::mock;
    use uuid::Uuid;

    mock! {
        pub ConfigManagementService {}

        #[async_trait::async_trait]
        impl ConfigManagementService for ConfigManagementService {
            fn get_config(&self) -> ConfigResponse;
            fn get_secure_config(&self) -> SecureConfigResponse;
            fn get_batch_max_size(&self) -> u32;
            async fn update_rate_limit(&self, req: UpdateRateLimitRequest) -> UpdateConfigResponse;
            async fn update_logging(&self, req: UpdateLoggingRequest) -> UpdateConfigResponse;
            async fn reload_config(&self) -> UpdateConfigResponse;
            async fn get_rate_limit_override(&self) -> Option<(u32, u32)>;
            async fn set_algorithm(&self, req: SetAlgorithmRequest) -> SetAlgorithmResponse;
            async fn create_biz_tag(&self, request: &crate::core::database::CreateBizTagRequest) -> crate::core::Result<crate::core::database::BizTag>;
            async fn get_biz_tag(&self, id: Uuid) -> crate::core::Result<Option<crate::core::database::BizTag>>;
            async fn update_biz_tag(&self, id: Uuid, request: &crate::core::database::UpdateBizTagRequest) -> crate::core::Result<crate::core::database::BizTag>;
            async fn delete_biz_tag(&self, id: Uuid) -> crate::core::Result<()>;
            async fn count_biz_tags(&self, workspace_id: Uuid, group_id: Option<Uuid>) -> crate::core::Result<u64>;
            async fn list_biz_tags(&self, workspace_id: Uuid, group_id: Option<Uuid>, limit: Option<u32>, offset: Option<u32>) -> crate::core::Result<Vec<crate::core::database::BizTag>>;
            async fn create_workspace(&self, req: CreateWorkspaceRequest) -> crate::core::Result<WorkspaceResponse>;
            async fn list_workspaces(&self) -> crate::core::Result<WorkspaceListResponse>;
            async fn get_workspace(&self, name: &str) -> crate::core::Result<Option<WorkspaceResponse>>;
            async fn create_group(&self, req: CreateGroupRequest) -> crate::core::Result<GroupResponse>;
            async fn list_groups(&self, workspace: &str) -> crate::core::Result<GroupListResponse>;
            async fn get_database_metrics(&self) -> DatabaseMetrics;
            async fn get_cache_metrics(&self) -> CacheMetrics;
            async fn get_algorithm_metrics(&self) -> Vec<(crate::core::types::AlgorithmType, crate::core::algorithm::AlgorithmMetricsSnapshot)>;
        }
    }

    mock! {
        pub ApiKeyRepository {}

        #[async_trait::async_trait]
        impl ApiKeyRepository for ApiKeyRepository {
            async fn create_api_key(&self, request: &CoreCreateApiKeyRequest) -> Result<ApiKeyWithSecret>;
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

    // ========== Helpers ==========

    fn create_mock_handlers(mock_config: MockConfigManagementService) -> Arc<ApiHandlers> {
        let mock_gen = Arc::new(MockIdGenerator::new());
        let config_service: Arc<dyn ConfigManagementService> = Arc::new(mock_config);
        Arc::new(ApiHandlers::new(mock_gen, config_service))
    }

    fn create_mock_handlers_with_repo(
        mock_config: MockConfigManagementService,
        mock_repo: MockApiKeyRepository,
    ) -> Arc<ApiHandlers> {
        let mock_gen = Arc::new(MockIdGenerator::new());
        let config_service: Arc<dyn ConfigManagementService> = Arc::new(mock_config);
        let repo: Arc<dyn ApiKeyRepository> = Arc::new(mock_repo);
        Arc::new(ApiHandlers::with_api_key_repository(
            mock_gen,
            config_service,
            repo,
        ))
    }

    fn test_biz_tag() -> BizTag {
        BizTag {
            id: Uuid::new_v4(),
            workspace_id: Uuid::new_v4(),
            group_id: Uuid::new_v4(),
            name: "test-biz-tag".to_string(),
            description: Some("Test biz tag".to_string()),
            algorithm: AlgorithmType::Segment,
            format: crate::core::types::IdFormat::Numeric,
            prefix: "test_".to_string(),
            base_step: 100,
            max_step: 1000,
            datacenter_ids: vec![0],
            created_at: chrono::Utc::now().naive_utc(),
            updated_at: chrono::Utc::now().naive_utc(),
        }
    }

    fn test_workspace_response() -> WorkspaceResponse {
        WorkspaceResponse {
            id: Uuid::new_v4().to_string(),
            name: "test-workspace".to_string(),
            description: Some("Test workspace".to_string()),
            status: "active".to_string(),
            max_groups: 10,
            max_biz_tags: 100,
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            user_api_key: None,
        }
    }

    fn test_group_response() -> GroupResponse {
        GroupResponse {
            id: Uuid::new_v4().to_string(),
            workspace_id: Uuid::new_v4().to_string(),
            workspace_name: "test-workspace".to_string(),
            name: "test-group".to_string(),
            description: Some("Test group".to_string()),
            max_biz_tags: 50,
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    fn test_api_key_with_secret() -> ApiKeyWithSecret {
        ApiKeyWithSecret {
            key: CoreApiKeyResponse {
                id: Uuid::new_v4(),
                key_id: "nino_test-key-id".to_string(),
                key_prefix: "nino_".to_string(),
                name: "test-key".to_string(),
                description: Some("Test API key".to_string()),
                role: ApiKeyRole::User,
                rate_limit: 10000,
                enabled: true,
                expires_at: None,
                created_at: chrono::Utc::now().naive_utc(),
            },
            key_secret: "test-secret-value-12345".to_string(),
        }
    }

    fn test_api_key(role: ApiKeyRole) -> ApiKey {
        ApiKey {
            id: Uuid::new_v4(),
            key_id: "nino_test-key-id".to_string(),
            key_prefix: "nino_".to_string(),
            role,
            workspace_id: Some(Uuid::new_v4()),
            name: "test-key".to_string(),
            description: Some("Test API key".to_string()),
            rate_limit: 10000,
            enabled: true,
            expires_at: None,
            last_used_at: None,
            created_at: chrono::Utc::now().naive_utc(),
        }
    }

    fn test_database_metrics(status: HealthStatus) -> DatabaseMetrics {
        DatabaseMetrics {
            status,
            connection_pool: ConnectionPoolMetrics {
                active_connections: 1,
                idle_connections: 4,
                max_connections: 10,
            },
            last_error: None,
        }
    }

    fn test_cache_metrics(status: HealthStatus) -> CacheMetrics {
        CacheMetrics {
            status,
            hit_rate: 0.85,
            memory_usage_mb: Some(128),
            key_count: Some(1000),
        }
    }

    // ========== ID Generation Tests (15) ==========

    #[tokio::test]
    async fn mock_test_generate_happy_path() {
        let handlers = create_mock_handlers(MockConfigManagementService::new());
        let req = GenerateRequest {
            workspace: "test-ws".to_string(),
            group: "test-group".to_string(),
            biz_tag: "test-biz".to_string(),
            algorithm: None,
        };
        let result = handlers.generate(req).await;
        assert!(result.is_ok());
        let response = result.unwrap();
        assert!(!response.id.is_empty());
        assert_eq!(response.algorithm, "segment");
    }

    #[tokio::test]
    async fn mock_test_generate_with_algorithm_segment() {
        let handlers = create_mock_handlers(MockConfigManagementService::new());
        let req = GenerateRequest {
            workspace: "test-ws".to_string(),
            group: "test-group".to_string(),
            biz_tag: "test-biz".to_string(),
            algorithm: Some("segment".to_string()),
        };
        let result = handlers.generate(req).await;
        assert!(result.is_ok());
        let response = result.unwrap();
        assert!(!response.id.is_empty());
    }

    #[tokio::test]
    async fn mock_test_generate_with_algorithm_snowflake() {
        let handlers = create_mock_handlers(MockConfigManagementService::new());
        let req = GenerateRequest {
            workspace: "test-ws".to_string(),
            group: "test-group".to_string(),
            biz_tag: "test-biz".to_string(),
            algorithm: Some("snowflake".to_string()),
        };
        let result = handlers.generate(req).await;
        assert!(result.is_ok());
        let response = result.unwrap();
        assert!(!response.id.is_empty());
    }

    #[tokio::test]
    async fn mock_test_generate_invalid_algorithm() {
        let handlers = create_mock_handlers(MockConfigManagementService::new());
        let req = GenerateRequest {
            workspace: "test-ws".to_string(),
            group: "test-group".to_string(),
            biz_tag: "test-biz".to_string(),
            algorithm: Some("invalid-algo".to_string()),
        };
        let result = handlers.generate(req).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            CoreError::InvalidAlgorithmType(s) => assert_eq!(s, "invalid-algo"),
            e => panic!("Expected InvalidAlgorithmType, got {:?}", e),
        }
    }

    #[tokio::test]
    async fn mock_test_generate_empty_workspace() {
        let handlers = create_mock_handlers(MockConfigManagementService::new());
        let req = GenerateRequest {
            workspace: String::new(),
            group: "test-group".to_string(),
            biz_tag: "test-biz".to_string(),
            algorithm: None,
        };
        let result = handlers.generate(req).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            CoreError::InvalidInput(msg) => assert!(msg.contains("workspace cannot be empty")),
            e => panic!("Expected InvalidInput, got {:?}", e),
        }
    }

    #[tokio::test]
    async fn mock_test_batch_generate_happy_path() {
        let mut mock_config = MockConfigManagementService::new();
        mock_config.expect_get_batch_max_size().return_once(|| 100);
        let handlers = create_mock_handlers(mock_config);
        let req = BatchGenerateRequest {
            workspace: "test-ws".to_string(),
            group: "test-group".to_string(),
            biz_tag: "test-biz".to_string(),
            size: Some(5),
            algorithm: None,
        };
        let result = handlers.batch_generate(req).await;
        assert!(result.is_ok());
        let response = result.unwrap();
        assert_eq!(response.ids.len(), 5);
        assert_eq!(response.size, 5);
    }

    #[tokio::test]
    async fn mock_test_batch_generate_default_size() {
        let mut mock_config = MockConfigManagementService::new();
        mock_config.expect_get_batch_max_size().return_once(|| 100);
        let handlers = create_mock_handlers(mock_config);
        let req = BatchGenerateRequest {
            workspace: "test-ws".to_string(),
            group: "test-group".to_string(),
            biz_tag: "test-biz".to_string(),
            size: None,
            algorithm: None,
        };
        let result = handlers.batch_generate(req).await;
        assert!(result.is_ok());
        let response = result.unwrap();
        assert_eq!(response.ids.len(), 10);
        assert_eq!(response.size, 10);
    }

    #[tokio::test]
    async fn mock_test_batch_generate_zero_size() {
        let mut mock_config = MockConfigManagementService::new();
        mock_config.expect_get_batch_max_size().return_once(|| 100);
        let handlers = create_mock_handlers(mock_config);
        let req = BatchGenerateRequest {
            workspace: "test-ws".to_string(),
            group: "test-group".to_string(),
            biz_tag: "test-biz".to_string(),
            size: Some(0),
            algorithm: None,
        };
        let result = handlers.batch_generate(req).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            CoreError::InvalidInput(msg) => assert!(msg.contains("Batch size cannot be zero")),
            e => panic!("Expected InvalidInput, got {:?}", e),
        }
    }

    #[tokio::test]
    async fn mock_test_batch_generate_exceeds_max() {
        let mut mock_config = MockConfigManagementService::new();
        mock_config.expect_get_batch_max_size().return_once(|| 50);
        let handlers = create_mock_handlers(mock_config);
        let req = BatchGenerateRequest {
            workspace: "test-ws".to_string(),
            group: "test-group".to_string(),
            biz_tag: "test-biz".to_string(),
            size: Some(100),
            algorithm: None,
        };
        let result = handlers.batch_generate(req).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            CoreError::InvalidInput(msg) => assert!(msg.contains("exceeds maximum")),
            e => panic!("Expected InvalidInput, got {:?}", e),
        }
    }

    #[tokio::test]
    async fn mock_test_batch_generate_with_algorithm() {
        let mut mock_config = MockConfigManagementService::new();
        mock_config.expect_get_batch_max_size().return_once(|| 100);
        let handlers = create_mock_handlers(mock_config);
        let req = BatchGenerateRequest {
            workspace: "test-ws".to_string(),
            group: "test-group".to_string(),
            biz_tag: "test-biz".to_string(),
            size: Some(3),
            algorithm: Some("snowflake".to_string()),
        };
        let result = handlers.batch_generate(req).await;
        assert!(result.is_ok());
        let response = result.unwrap();
        assert_eq!(response.ids.len(), 3);
    }

    #[tokio::test]
    async fn mock_test_parse_numeric_id() {
        let handlers = create_mock_handlers(MockConfigManagementService::new());
        let req = ParseRequest {
            id: "12345".to_string(),
            workspace: "test-ws".to_string(),
            group: "test-group".to_string(),
            biz_tag: "test-biz".to_string(),
            algorithm: "segment".to_string(),
        };
        let result = handlers.parse(req).await;
        assert!(result.is_ok());
        let response = result.unwrap();
        assert_eq!(response.original_id, "12345");
        assert_eq!(response.numeric_value, "12345");
        assert_eq!(response.algorithm, "segment");
    }

    #[tokio::test]
    async fn mock_test_parse_uuid_id() {
        let handlers = create_mock_handlers(MockConfigManagementService::new());
        let uuid_str = "01890679-2c4c-7e8c-9f1a-3d5e7f9a1b2c";
        let req = ParseRequest {
            id: uuid_str.to_string(),
            workspace: "test-ws".to_string(),
            group: "test-group".to_string(),
            biz_tag: "test-biz".to_string(),
            algorithm: "uuid_v7".to_string(),
        };
        let result = handlers.parse(req).await;
        assert!(result.is_ok());
        let response = result.unwrap();
        assert_eq!(response.original_id, uuid_str);
        assert_eq!(response.algorithm, "uuid_v7");
    }

    #[tokio::test]
    async fn mock_test_parse_invalid_id() {
        let handlers = create_mock_handlers(MockConfigManagementService::new());
        let req = ParseRequest {
            id: "not-a-valid-id".to_string(),
            workspace: "test-ws".to_string(),
            group: "test-group".to_string(),
            biz_tag: "test-biz".to_string(),
            algorithm: "segment".to_string(),
        };
        let result = handlers.parse(req).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            CoreError::InvalidIdString(msg) => assert!(msg.contains("Failed to parse ID")),
            e => panic!("Expected InvalidIdString, got {:?}", e),
        }
    }

    #[tokio::test]
    async fn mock_test_parse_with_snowflake_algorithm() {
        let handlers = create_mock_handlers(MockConfigManagementService::new());
        let req = ParseRequest {
            id: "99999999999999999999".to_string(),
            workspace: "test-ws".to_string(),
            group: "test-group".to_string(),
            biz_tag: "test-biz".to_string(),
            algorithm: "snowflake".to_string(),
        };
        let result = handlers.parse(req).await;
        assert!(result.is_ok());
        let response = result.unwrap();
        assert_eq!(response.algorithm, "snowflake");
        assert_eq!(response.metadata.algorithm, "snowflake");
    }

    #[tokio::test]
    async fn mock_test_parse_empty_algorithm_uses_generator() {
        let handlers = create_mock_handlers(MockConfigManagementService::new());
        let req = ParseRequest {
            id: "42".to_string(),
            workspace: "test-ws".to_string(),
            group: "test-group".to_string(),
            biz_tag: "test-biz".to_string(),
            algorithm: String::new(),
        };
        let result = handlers.parse(req).await;
        assert!(result.is_ok());
        let response = result.unwrap();
        // MockIdGenerator.get_algorithm_name returns "segment"
        assert_eq!(response.algorithm, "segment");
    }

    // ========== System Tests (9) ==========

    #[tokio::test]
    async fn mock_test_health_healthy() {
        let handlers = create_mock_handlers(MockConfigManagementService::new());
        let response = handlers.health().await;
        assert_eq!(response.status, HealthStatus::Healthy);
    }

    #[tokio::test]
    async fn mock_test_health_returns_algorithm() {
        let handlers = create_mock_handlers(MockConfigManagementService::new());
        let response = handlers.health().await;
        assert_eq!(response.algorithm, "segment");
    }

    #[tokio::test]
    async fn mock_test_health_status_correct() {
        let handlers = create_mock_handlers(MockConfigManagementService::new());
        let response = handlers.health().await;
        assert_eq!(response.status, HealthStatus::Healthy);
        assert!(!response.algorithm.is_empty());
    }

    #[tokio::test]
    async fn mock_test_ready_all_healthy() {
        let mut mock_config = MockConfigManagementService::new();
        mock_config
            .expect_get_database_metrics()
            .return_once(|| test_database_metrics(HealthStatus::Healthy));
        mock_config
            .expect_get_cache_metrics()
            .return_once(|| test_cache_metrics(HealthStatus::Healthy));
        let handlers = create_mock_handlers(mock_config);
        let response = handlers.ready().await;
        assert!(response.ready);
        assert!(response.database);
        assert!(response.cache);
        assert!(response.message.contains("Ready"));
    }

    #[tokio::test]
    async fn mock_test_ready_db_unhealthy() {
        let mut mock_config = MockConfigManagementService::new();
        mock_config
            .expect_get_database_metrics()
            .return_once(|| test_database_metrics(HealthStatus::Unhealthy));
        mock_config
            .expect_get_cache_metrics()
            .return_once(|| test_cache_metrics(HealthStatus::Healthy));
        let handlers = create_mock_handlers(mock_config);
        let response = handlers.ready().await;
        assert!(!response.ready);
        assert!(!response.database);
        assert!(response.cache);
    }

    #[tokio::test]
    async fn mock_test_ready_cache_unhealthy() {
        let mut mock_config = MockConfigManagementService::new();
        mock_config
            .expect_get_database_metrics()
            .return_once(|| test_database_metrics(HealthStatus::Healthy));
        mock_config
            .expect_get_cache_metrics()
            .return_once(|| test_cache_metrics(HealthStatus::Unhealthy));
        let handlers = create_mock_handlers(mock_config);
        let response = handlers.ready().await;
        assert!(!response.ready);
        assert!(response.database);
        assert!(!response.cache);
    }

    #[tokio::test]
    async fn mock_test_metrics_empty() {
        let mut mock_config = MockConfigManagementService::new();
        mock_config
            .expect_get_algorithm_metrics()
            .return_once(Vec::new);
        mock_config
            .expect_get_database_metrics()
            .return_once(|| test_database_metrics(HealthStatus::Healthy));
        mock_config
            .expect_get_cache_metrics()
            .return_once(|| test_cache_metrics(HealthStatus::Healthy));
        let handlers = create_mock_handlers(mock_config);
        let response = handlers.metrics().await;
        assert!(response.algorithms.is_empty());
        assert_eq!(response.total_requests, 0);
    }

    #[tokio::test]
    async fn mock_test_metrics_with_algorithm() {
        let mut mock_config = MockConfigManagementService::new();
        mock_config.expect_get_algorithm_metrics().return_once(|| {
            vec![(
                AlgorithmType::Segment,
                AlgorithmMetricsSnapshot {
                    total_generated: 100,
                    total_failed: 5,
                    cache_hit_rate: 0.9,
                    ..Default::default()
                },
            )]
        });
        mock_config
            .expect_get_database_metrics()
            .return_once(|| test_database_metrics(HealthStatus::Healthy));
        mock_config
            .expect_get_cache_metrics()
            .return_once(|| test_cache_metrics(HealthStatus::Healthy));
        let handlers = create_mock_handlers(mock_config);
        let response = handlers.metrics().await;
        assert_eq!(response.algorithms.len(), 1);
        assert_eq!(response.algorithms[0].algorithm, "segment");
        assert_eq!(response.algorithms[0].total_generated, 100);
        assert_eq!(response.algorithms[0].total_failed, 5);
    }

    #[tokio::test]
    async fn mock_test_metrics_database_status() {
        let mut mock_config = MockConfigManagementService::new();
        mock_config
            .expect_get_algorithm_metrics()
            .return_once(Vec::new);
        mock_config
            .expect_get_database_metrics()
            .return_once(|| test_database_metrics(HealthStatus::Unhealthy));
        mock_config
            .expect_get_cache_metrics()
            .return_once(|| test_cache_metrics(HealthStatus::Healthy));
        let handlers = create_mock_handlers(mock_config);
        let response = handlers.metrics().await;
        assert_eq!(response.database.status, HealthStatus::Unhealthy);
        assert_eq!(response.cache.status, HealthStatus::Healthy);
    }

    // ========== BizTag CRUD Tests (12) ==========

    #[tokio::test]
    async fn mock_test_create_biz_tag_happy_path() {
        let biz_tag = test_biz_tag();
        let expected_id = biz_tag.id.to_string();
        let mut mock_config = MockConfigManagementService::new();
        mock_config
            .expect_create_biz_tag()
            .return_once(move |_| Ok(biz_tag));
        let handlers = create_mock_handlers(mock_config);
        let req = CreateBizTagRequest {
            workspace_id: Uuid::new_v4(),
            group_id: Uuid::new_v4(),
            name: "test-biz-tag".to_string(),
            description: Some("Test biz tag".to_string()),
            algorithm: Some("segment".to_string()),
            format: Some("numeric".to_string()),
            prefix: Some("test_".to_string()),
            base_step: Some(100),
            max_step: Some(1000),
            datacenter_ids: Some(vec![0]),
        };
        let result = handlers.create_biz_tag(req).await;
        assert!(result.is_ok());
        let response = result.unwrap();
        assert_eq!(response.id, expected_id);
        assert_eq!(response.name, "test-biz-tag");
        assert_eq!(response.algorithm, "segment");
    }

    #[tokio::test]
    async fn mock_test_create_biz_tag_invalid_algorithm() {
        let handlers = create_mock_handlers(MockConfigManagementService::new());
        let req = CreateBizTagRequest {
            workspace_id: Uuid::new_v4(),
            group_id: Uuid::new_v4(),
            name: "test-biz-tag".to_string(),
            description: None,
            algorithm: Some("invalid-algo".to_string()),
            format: Some("numeric".to_string()),
            prefix: None,
            base_step: None,
            max_step: None,
            datacenter_ids: None,
        };
        let result = handlers.create_biz_tag(req).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            CoreError::InvalidAlgorithmType(s) => assert_eq!(s, "invalid-algo"),
            e => panic!("Expected InvalidAlgorithmType, got {:?}", e),
        }
    }

    #[tokio::test]
    async fn mock_test_create_biz_tag_service_error() {
        let mut mock_config = MockConfigManagementService::new();
        mock_config.expect_create_biz_tag().return_once(|_| {
            Err(CoreError::InternalError(
                "Database repository not configured".to_string(),
            ))
        });
        let handlers = create_mock_handlers(mock_config);
        let req = CreateBizTagRequest {
            workspace_id: Uuid::new_v4(),
            group_id: Uuid::new_v4(),
            name: "test-biz-tag".to_string(),
            description: None,
            algorithm: Some("segment".to_string()),
            format: Some("numeric".to_string()),
            prefix: None,
            base_step: None,
            max_step: None,
            datacenter_ids: None,
        };
        let result = handlers.create_biz_tag(req).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            CoreError::InternalError(msg) => {
                assert!(msg.contains("Database repository not configured"))
            }
            e => panic!("Expected InternalError, got {:?}", e),
        }
    }

    #[tokio::test]
    async fn mock_test_update_biz_tag_happy_path() {
        let biz_tag = test_biz_tag();
        let expected_name = biz_tag.name.clone();
        let mut mock_config = MockConfigManagementService::new();
        mock_config
            .expect_update_biz_tag()
            .return_once(move |_, _| Ok(biz_tag));
        let handlers = create_mock_handlers(mock_config);
        let req = UpdateBizTagRequest {
            name: Some("updated-name".to_string()),
            description: None,
            algorithm: None,
            format: None,
            prefix: None,
            base_step: None,
            max_step: None,
            datacenter_ids: None,
        };
        let result = handlers.update_biz_tag(Uuid::new_v4(), req).await;
        assert!(result.is_ok());
        let response = result.unwrap();
        assert_eq!(response.name, expected_name);
    }

    #[tokio::test]
    async fn mock_test_update_biz_tag_invalid_algorithm() {
        let handlers = create_mock_handlers(MockConfigManagementService::new());
        let req = UpdateBizTagRequest {
            name: None,
            description: None,
            algorithm: Some("invalid-algo".to_string()),
            format: None,
            prefix: None,
            base_step: None,
            max_step: None,
            datacenter_ids: None,
        };
        let result = handlers.update_biz_tag(Uuid::new_v4(), req).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            CoreError::InvalidAlgorithmType(s) => assert_eq!(s, "invalid-algo"),
            e => panic!("Expected InvalidAlgorithmType, got {:?}", e),
        }
    }

    #[tokio::test]
    async fn mock_test_update_biz_tag_service_error() {
        let mut mock_config = MockConfigManagementService::new();
        mock_config
            .expect_update_biz_tag()
            .return_once(|_, _| Err(CoreError::NotFound("BizTag not found".to_string())));
        let handlers = create_mock_handlers(mock_config);
        let req = UpdateBizTagRequest {
            name: Some("updated".to_string()),
            description: None,
            algorithm: None,
            format: None,
            prefix: None,
            base_step: None,
            max_step: None,
            datacenter_ids: None,
        };
        let result = handlers.update_biz_tag(Uuid::new_v4(), req).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            CoreError::NotFound(msg) => assert!(msg.contains("BizTag not found")),
            e => panic!("Expected NotFound, got {:?}", e),
        }
    }

    #[tokio::test]
    async fn mock_test_list_biz_tags_happy_path() {
        let biz_tag = test_biz_tag();
        let expected_name = biz_tag.name.clone();
        let mut mock_config = MockConfigManagementService::new();
        mock_config
            .expect_list_biz_tags()
            .return_once(move |_, _, _, _| Ok(vec![biz_tag]));
        let handlers = create_mock_handlers(mock_config);
        let result = handlers.list_biz_tags(Some(Uuid::new_v4()), None).await;
        assert!(result.is_ok());
        let response = result.unwrap();
        assert_eq!(response.total, 1);
        assert_eq!(response.biz_tags.len(), 1);
        assert_eq!(response.biz_tags[0].name, expected_name);
    }

    #[tokio::test]
    async fn mock_test_list_biz_tags_empty() {
        let mut mock_config = MockConfigManagementService::new();
        mock_config
            .expect_list_biz_tags()
            .return_once(|_, _, _, _| Ok(vec![]));
        let handlers = create_mock_handlers(mock_config);
        let result = handlers.list_biz_tags(None, None).await;
        assert!(result.is_ok());
        let response = result.unwrap();
        assert_eq!(response.total, 0);
        assert!(response.biz_tags.is_empty());
    }

    #[tokio::test]
    async fn mock_test_list_biz_tags_service_error() {
        let mut mock_config = MockConfigManagementService::new();
        mock_config
            .expect_list_biz_tags()
            .return_once(|_, _, _, _| Err(CoreError::InternalError("DB error".to_string())));
        let handlers = create_mock_handlers(mock_config);
        let result = handlers.list_biz_tags(None, None).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            CoreError::InternalError(msg) => assert!(msg.contains("DB error")),
            e => panic!("Expected InternalError, got {:?}", e),
        }
    }

    #[tokio::test]
    async fn mock_test_delete_biz_tag_happy_path() {
        let mut mock_config = MockConfigManagementService::new();
        mock_config.expect_delete_biz_tag().return_once(|_| Ok(()));
        let handlers = create_mock_handlers(mock_config);
        let result = handlers.delete_biz_tag(Uuid::new_v4()).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn mock_test_delete_biz_tag_service_error() {
        let mut mock_config = MockConfigManagementService::new();
        mock_config
            .expect_delete_biz_tag()
            .return_once(|_| Err(CoreError::NotFound("BizTag not found".to_string())));
        let handlers = create_mock_handlers(mock_config);
        let result = handlers.delete_biz_tag(Uuid::new_v4()).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            CoreError::NotFound(msg) => assert!(msg.contains("BizTag not found")),
            e => panic!("Expected NotFound, got {:?}", e),
        }
    }

    #[tokio::test]
    async fn mock_test_delete_biz_tag_propagates_error() {
        let target_id = Uuid::new_v4();
        let mut mock_config = MockConfigManagementService::new();
        mock_config
            .expect_delete_biz_tag()
            .withf(move |id| *id == target_id)
            .return_once(|_| Ok(()));
        let handlers = create_mock_handlers(mock_config);
        let result = handlers.delete_biz_tag(target_id).await;
        assert!(result.is_ok());
    }

    // ========== Workspace/Group Tests (12) ==========

    #[tokio::test]
    async fn mock_test_create_workspace_no_api_key_repo() {
        let mut mock_config = MockConfigManagementService::new();
        mock_config
            .expect_create_workspace()
            .return_once(|_| Ok(test_workspace_response()));
        let handlers = create_mock_handlers(mock_config);
        let req = CreateWorkspaceRequest {
            name: "test-ws".to_string(),
            description: Some("Test".to_string()),
            max_groups: Some(10),
            max_biz_tags: Some(100),
        };
        let result = handlers.create_workspace(req).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            CoreError::NotFound(msg) => {
                assert!(msg.contains("API key repository not configured"))
            }
            e => panic!("Expected NotFound, got {:?}", e),
        }
    }

    #[tokio::test]
    async fn mock_test_create_workspace_service_error() {
        let mut mock_config = MockConfigManagementService::new();
        mock_config
            .expect_create_workspace()
            .return_once(|_| Err(CoreError::InternalError("DB error".to_string())));
        let handlers = create_mock_handlers(mock_config);
        let req = CreateWorkspaceRequest {
            name: "test-ws".to_string(),
            description: None,
            max_groups: None,
            max_biz_tags: None,
        };
        let result = handlers.create_workspace(req).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            CoreError::DatabaseError(msg) => assert!(msg.contains("DB error")),
            e => panic!("Expected DatabaseError, got {:?}", e),
        }
    }

    #[tokio::test]
    async fn mock_test_create_workspace_happy_path() {
        let ws_response = test_workspace_response();
        let expected_name = ws_response.name.clone();
        let mut mock_config = MockConfigManagementService::new();
        mock_config
            .expect_create_workspace()
            .return_once(move |_| Ok(ws_response));
        let mut mock_repo = MockApiKeyRepository::new();
        mock_repo
            .expect_create_api_key()
            .return_once(|_| Ok(test_api_key_with_secret()));
        let handlers = create_mock_handlers_with_repo(mock_config, mock_repo);
        let req = CreateWorkspaceRequest {
            name: "test-ws".to_string(),
            description: Some("Test".to_string()),
            max_groups: Some(10),
            max_biz_tags: Some(100),
        };
        let result = handlers.create_workspace(req).await;
        assert!(result.is_ok());
        let response = result.unwrap();
        assert_eq!(response.name, expected_name);
        assert!(response.user_api_key.is_some());
    }

    #[tokio::test]
    async fn mock_test_list_workspaces_happy_path() {
        let ws = test_workspace_response();
        let expected_name = ws.name.clone();
        let mut mock_config = MockConfigManagementService::new();
        mock_config.expect_list_workspaces().return_once(move || {
            Ok(WorkspaceListResponse {
                workspaces: vec![ws],
                total: 1,
            })
        });
        let handlers = create_mock_handlers(mock_config);
        let result = handlers.list_workspaces().await;
        assert!(result.is_ok());
        let response = result.unwrap();
        assert_eq!(response.total, 1);
        assert_eq!(response.workspaces.len(), 1);
        assert_eq!(response.workspaces[0].name, expected_name);
    }

    #[tokio::test]
    async fn mock_test_list_workspaces_empty() {
        let mut mock_config = MockConfigManagementService::new();
        mock_config.expect_list_workspaces().return_once(|| {
            Ok(WorkspaceListResponse {
                workspaces: vec![],
                total: 0,
            })
        });
        let handlers = create_mock_handlers(mock_config);
        let result = handlers.list_workspaces().await;
        assert!(result.is_ok());
        let response = result.unwrap();
        assert_eq!(response.total, 0);
        assert!(response.workspaces.is_empty());
    }

    #[tokio::test]
    async fn mock_test_list_workspaces_service_error() {
        let mut mock_config = MockConfigManagementService::new();
        mock_config
            .expect_list_workspaces()
            .return_once(|| Err(CoreError::InternalError("DB error".to_string())));
        let handlers = create_mock_handlers(mock_config);
        let result = handlers.list_workspaces().await;
        assert!(result.is_err());
        match result.unwrap_err() {
            CoreError::DatabaseError(msg) => assert!(msg.contains("DB error")),
            e => panic!("Expected DatabaseError, got {:?}", e),
        }
    }

    #[tokio::test]
    async fn mock_test_get_workspace_happy_path() {
        let ws = test_workspace_response();
        let expected_name = ws.name.clone();
        let mut mock_config = MockConfigManagementService::new();
        mock_config
            .expect_get_workspace()
            .return_once(move |_| Ok(Some(ws)));
        let handlers = create_mock_handlers(mock_config);
        let result = handlers.get_workspace("test-workspace").await;
        assert!(result.is_ok());
        let response = result.unwrap();
        assert!(response.is_some());
        assert_eq!(response.unwrap().name, expected_name);
    }

    #[tokio::test]
    async fn mock_test_get_workspace_not_found() {
        let mut mock_config = MockConfigManagementService::new();
        mock_config.expect_get_workspace().return_once(|_| Ok(None));
        let handlers = create_mock_handlers(mock_config);
        let result = handlers.get_workspace("nonexistent").await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    #[tokio::test]
    async fn mock_test_get_workspace_service_error() {
        let mut mock_config = MockConfigManagementService::new();
        mock_config
            .expect_get_workspace()
            .return_once(|_| Err(CoreError::InternalError("DB error".to_string())));
        let handlers = create_mock_handlers(mock_config);
        let result = handlers.get_workspace("test-workspace").await;
        assert!(result.is_err());
        match result.unwrap_err() {
            CoreError::DatabaseError(msg) => assert!(msg.contains("DB error")),
            e => panic!("Expected DatabaseError, got {:?}", e),
        }
    }

    #[tokio::test]
    async fn mock_test_create_group_happy_path() {
        let group = test_group_response();
        let expected_name = group.name.clone();
        let mut mock_config = MockConfigManagementService::new();
        mock_config
            .expect_create_group()
            .return_once(move |_| Ok(group));
        let handlers = create_mock_handlers(mock_config);
        let req = CreateGroupRequest {
            workspace: "test-workspace".to_string(),
            name: "test-group".to_string(),
            description: Some("Test group".to_string()),
            max_biz_tags: Some(50),
        };
        let result = handlers.create_group(req).await;
        assert!(result.is_ok());
        let response = result.unwrap();
        assert_eq!(response.name, expected_name);
    }

    #[tokio::test]
    async fn mock_test_create_group_service_error() {
        let mut mock_config = MockConfigManagementService::new();
        mock_config
            .expect_create_group()
            .return_once(|_| Err(CoreError::InternalError("Workspace not found".to_string())));
        let handlers = create_mock_handlers(mock_config);
        let req = CreateGroupRequest {
            workspace: "nonexistent".to_string(),
            name: "test-group".to_string(),
            description: None,
            max_biz_tags: None,
        };
        let result = handlers.create_group(req).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            CoreError::DatabaseError(msg) => assert!(msg.contains("Workspace not found")),
            e => panic!("Expected DatabaseError, got {:?}", e),
        }
    }

    #[tokio::test]
    async fn mock_test_create_group_passes_request_correctly() {
        let mut mock_config = MockConfigManagementService::new();
        mock_config
            .expect_create_group()
            .withf(|req| req.workspace == "my-ws" && req.name == "my-group")
            .return_once(|_| Ok(test_group_response()));
        let handlers = create_mock_handlers(mock_config);
        let req = CreateGroupRequest {
            workspace: "my-ws".to_string(),
            name: "my-group".to_string(),
            description: None,
            max_biz_tags: Some(20),
        };
        let result = handlers.create_group(req).await;
        assert!(result.is_ok());
    }

    // ========== API Key Tests (12) ==========

    #[tokio::test]
    async fn mock_test_create_api_key_no_repo() {
        let handlers = create_mock_handlers(MockConfigManagementService::new());
        let req = CreateApiKeyRequest {
            workspace_id: None,
            name: "test-key".to_string(),
            description: None,
            role: Some("admin".to_string()),
            rate_limit: None,
            expires_at: None,
        };
        let result = handlers.create_api_key(None, req).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            CoreError::NotFound(msg) => {
                assert!(msg.contains("API key repository not configured"))
            }
            e => panic!("Expected NotFound, got {:?}", e),
        }
    }

    #[tokio::test]
    async fn mock_test_create_api_key_invalid_role() {
        let mock_repo = MockApiKeyRepository::new();
        let handlers =
            create_mock_handlers_with_repo(MockConfigManagementService::new(), mock_repo);
        let req = CreateApiKeyRequest {
            workspace_id: None,
            name: "test-key".to_string(),
            description: None,
            role: Some("superadmin".to_string()),
            rate_limit: None,
            expires_at: None,
        };
        let result = handlers.create_api_key(None, req).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            CoreError::AuthenticationError(msg) => assert!(msg.contains("Invalid role")),
            e => panic!("Expected AuthenticationError, got {:?}", e),
        }
    }

    #[tokio::test]
    async fn mock_test_create_api_key_user_happy_path() {
        let ws_id = Uuid::new_v4();
        let mut mock_repo = MockApiKeyRepository::new();
        mock_repo
            .expect_list_api_keys()
            .return_once(|_, _, _| Ok(vec![]));
        mock_repo
            .expect_create_api_key()
            .return_once(|_| Ok(test_api_key_with_secret()));
        let handlers =
            create_mock_handlers_with_repo(MockConfigManagementService::new(), mock_repo);
        let req = CreateApiKeyRequest {
            workspace_id: Some(ws_id.to_string()),
            name: "test-user-key".to_string(),
            description: None,
            role: Some("user".to_string()),
            rate_limit: None,
            expires_at: None,
        };
        let result = handlers.create_api_key(Some(ws_id), req).await;
        assert!(result.is_ok());
        let response = result.unwrap();
        assert!(!response.key_secret.is_empty());
        assert_eq!(response.key.role, "user");
    }

    #[tokio::test]
    async fn mock_test_list_api_keys_no_repo() {
        let handlers = create_mock_handlers(MockConfigManagementService::new());
        let result = handlers.list_api_keys(Uuid::new_v4(), None, None).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            CoreError::NotFound(msg) => {
                assert!(msg.contains("API key repository not configured"))
            }
            e => panic!("Expected NotFound, got {:?}", e),
        }
    }

    #[tokio::test]
    async fn mock_test_list_api_keys_happy_path() {
        let mut mock_repo = MockApiKeyRepository::new();
        mock_repo
            .expect_list_api_keys()
            .return_once(|_, _, _| Ok(vec![test_api_key(ApiKeyRole::User)]));
        mock_repo.expect_count_api_keys().return_once(|_| Ok(1));
        let handlers =
            create_mock_handlers_with_repo(MockConfigManagementService::new(), mock_repo);
        let result = handlers
            .list_api_keys(Uuid::new_v4(), Some(10), Some(0))
            .await;
        assert!(result.is_ok());
        let response = result.unwrap();
        assert_eq!(response.total, 1);
        assert_eq!(response.api_keys.len(), 1);
    }

    #[tokio::test]
    async fn mock_test_list_api_keys_service_error() {
        let mut mock_repo = MockApiKeyRepository::new();
        mock_repo
            .expect_list_api_keys()
            .return_once(|_, _, _| Err(CoreError::DatabaseError("DB error".to_string())));
        let handlers =
            create_mock_handlers_with_repo(MockConfigManagementService::new(), mock_repo);
        let result = handlers.list_api_keys(Uuid::new_v4(), None, None).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            CoreError::DatabaseError(msg) => assert!(msg.contains("DB error")),
            e => panic!("Expected DatabaseError, got {:?}", e),
        }
    }

    #[tokio::test]
    async fn mock_test_revoke_api_key_no_repo() {
        let handlers = create_mock_handlers(MockConfigManagementService::new());
        let result = handlers.revoke_api_key(Uuid::new_v4()).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            CoreError::NotFound(msg) => {
                assert!(msg.contains("API key repository not configured"))
            }
            e => panic!("Expected NotFound, got {:?}", e),
        }
    }

    #[tokio::test]
    async fn mock_test_revoke_api_key_user_happy_path() {
        let target_id = Uuid::new_v4();
        let mut mock_repo = MockApiKeyRepository::new();
        mock_repo
            .expect_get_api_key_by_id()
            .return_once(|_| Ok(Some(test_api_key(ApiKeyRole::User))));
        mock_repo.expect_delete_api_key().return_once(|_| Ok(()));
        let handlers =
            create_mock_handlers_with_repo(MockConfigManagementService::new(), mock_repo);
        let result = handlers.revoke_api_key(target_id).await;
        assert!(result.is_ok());
        let response = result.unwrap();
        assert!(response.success);
        assert!(response.message.contains("revoked successfully"));
    }

    #[tokio::test]
    async fn mock_test_revoke_api_key_last_admin() {
        let admin_key = test_api_key(ApiKeyRole::Admin);
        let mut mock_repo = MockApiKeyRepository::new();
        mock_repo
            .expect_get_api_key_by_id()
            .return_once(|_| Ok(Some(admin_key)));
        mock_repo
            .expect_list_api_keys()
            .return_once(|_, _, _| Ok(vec![test_api_key(ApiKeyRole::Admin)]));
        let handlers =
            create_mock_handlers_with_repo(MockConfigManagementService::new(), mock_repo);
        let result = handlers.revoke_api_key(Uuid::new_v4()).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            CoreError::AuthenticationError(msg) => {
                assert!(msg.contains("Cannot revoke the last admin key"))
            }
            e => panic!("Expected AuthenticationError, got {:?}", e),
        }
    }

    #[tokio::test]
    async fn mock_test_rotate_api_key_empty_key_id() {
        let handlers = create_mock_handlers(MockConfigManagementService::new());
        let result = handlers.rotate_api_key("").await;
        assert!(result.is_err());
        match result.unwrap_err() {
            CoreError::InvalidInput(msg) => assert!(msg.contains("key_id cannot be empty")),
            e => panic!("Expected InvalidInput, got {:?}", e),
        }
    }

    #[tokio::test]
    async fn mock_test_rotate_api_key_no_repo() {
        let handlers = create_mock_handlers(MockConfigManagementService::new());
        let result = handlers.rotate_api_key("nino_some-key-id").await;
        assert!(result.is_err());
        match result.unwrap_err() {
            CoreError::NotFound(msg) => {
                assert!(msg.contains("API key repository not configured"))
            }
            e => panic!("Expected NotFound, got {:?}", e),
        }
    }

    #[tokio::test]
    async fn mock_test_rotate_api_key_happy_path() {
        let mut mock_repo = MockApiKeyRepository::new();
        mock_repo
            .expect_rotate_api_key()
            .return_once(|_, _| Ok(test_api_key_with_secret()));
        let handlers =
            create_mock_handlers_with_repo(MockConfigManagementService::new(), mock_repo);
        let result = handlers.rotate_api_key("nino_test-key-id").await;
        assert!(result.is_ok());
        let response = result.unwrap();
        assert!(!response.key_secret.is_empty());
    }

    // ========== Key Rotation + Shutdown Tests (2) ==========

    #[tokio::test]
    async fn mock_test_start_key_rotation_task_no_repo() {
        let handlers = create_mock_handlers(MockConfigManagementService::new());
        let handle = handlers.start_key_rotation_task(std::time::Duration::from_secs(60), 30);
        assert!(handle.is_none());
    }

    #[tokio::test]
    async fn mock_test_shutdown_handle() {
        let (shutdown_tx, _shutdown_rx) = tokio::sync::watch::channel(false);
        let handle = KeyRotationHandle { shutdown_tx };
        let handlers = create_mock_handlers(MockConfigManagementService::new());
        handlers.shutdown(handle);
        // If we reach here without panic, the test passes
    }
}
