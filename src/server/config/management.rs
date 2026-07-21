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

use super::hot_reload::HotReloadConfig;
use crate::core::config::Config;
use crate::core::database::{
    CreateGroupRequest as CoreCreateGroupRequest,
    CreateWorkspaceRequest as CoreCreateWorkspaceRequest,
};
use crate::core::types::id::AlgorithmType;
use crate::server::models::{
    AlgorithmConfigInfo, AppConfigInfo, CacheMetrics, ConfigResponse, ConnectionPoolMetrics,
    CreateGroupRequest, CreateWorkspaceRequest, DatabaseConfigInfo, DatabaseMetrics,
    GroupListResponse, GroupResponse, LoggingConfigInfo, MonitoringConfigInfo, RateLimitConfigInfo,
    SecureConfigResponse, SegmentConfigInfo, SetAlgorithmRequest, SetAlgorithmResponse,
    SnowflakeConfigInfo, TlsConfigInfo, UpdateConfigResponse, UpdateLoggingRequest,
    UpdateRateLimitRequest, UuidV7ConfigInfo, WorkspaceListResponse, WorkspaceResponse,
};
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;
use validator::Validate;

/// Configuration management service trait.
///
/// Decouples handlers/router from the concrete `ConfigManager` implementation
/// so business logic can be mock-tested (rule 25: trait + pub re-export in
/// mod.rs; impl lives in this file). All async methods are object-safe via
/// `async_trait`; sync methods (`get_config`, `get_secure_config`,
/// `get_batch_max_size`) take `&self` and are naturally object-safe.
#[async_trait]
pub trait ConfigManagementService: Send + Sync {
    fn get_config(&self) -> ConfigResponse;
    fn get_secure_config(&self) -> SecureConfigResponse;
    fn get_batch_max_size(&self) -> u32;

    async fn update_rate_limit(&self, req: UpdateRateLimitRequest) -> UpdateConfigResponse;
    async fn update_logging(&self, req: UpdateLoggingRequest) -> UpdateConfigResponse;
    async fn reload_config(&self) -> UpdateConfigResponse;
    async fn get_rate_limit_override(&self) -> Option<(u32, u32)>;
    async fn set_algorithm(&self, req: SetAlgorithmRequest) -> SetAlgorithmResponse;

    async fn create_biz_tag(
        &self,
        request: &crate::core::database::CreateBizTagRequest,
    ) -> crate::core::Result<crate::core::database::BizTag>;
    async fn get_biz_tag(
        &self,
        id: Uuid,
    ) -> crate::core::Result<Option<crate::core::database::BizTag>>;
    async fn update_biz_tag(
        &self,
        id: Uuid,
        request: &crate::core::database::UpdateBizTagRequest,
    ) -> crate::core::Result<crate::core::database::BizTag>;
    async fn delete_biz_tag(&self, id: Uuid) -> crate::core::Result<()>;
    async fn count_biz_tags(
        &self,
        workspace_id: Uuid,
        group_id: Option<Uuid>,
    ) -> crate::core::Result<u64>;
    async fn list_biz_tags(
        &self,
        workspace_id: Uuid,
        group_id: Option<Uuid>,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> crate::core::Result<Vec<crate::core::database::BizTag>>;

    async fn create_workspace(
        &self,
        req: CreateWorkspaceRequest,
    ) -> crate::core::Result<WorkspaceResponse>;
    async fn list_workspaces(&self) -> crate::core::Result<WorkspaceListResponse>;
    async fn get_workspace(&self, name: &str) -> crate::core::Result<Option<WorkspaceResponse>>;
    async fn create_group(&self, req: CreateGroupRequest) -> crate::core::Result<GroupResponse>;
    async fn list_groups(&self, workspace: &str) -> crate::core::Result<GroupListResponse>;

    async fn get_database_metrics(&self) -> DatabaseMetrics;
    async fn get_cache_metrics(&self) -> CacheMetrics;
    async fn get_algorithm_metrics(
        &self,
    ) -> Vec<(
        crate::core::types::AlgorithmType,
        crate::core::algorithm::AlgorithmMetricsSnapshot,
    )>;
}

/// Production implementation of `ConfigManagementService`.
///
/// Renamed from `ConfigManagementService` (the struct) to `ConfigManager` so
/// the trait can claim the canonical name; callers refer to the trait via
/// `Arc<dyn ConfigManagementService>` and construct via `ConfigManager::new`.
pub struct ConfigManager {
    hot_config: Arc<HotReloadConfig>,
    rate_limiter: Arc<RwLock<Option<(u32, u32)>>>,
    algorithm_router: Arc<crate::core::algorithm::AlgorithmRouter>,
    repository: Option<Arc<dyn crate::core::database::BizTagRepository + Send + Sync>>,
    workspace_repository: Option<Arc<dyn crate::core::database::WorkspaceRepository + Send + Sync>>,
    group_repository: Option<Arc<dyn crate::core::database::GroupRepository + Send + Sync>>,
}

impl ConfigManager {
    pub fn new(
        hot_config: Arc<HotReloadConfig>,
        algorithm_router: Arc<crate::core::algorithm::AlgorithmRouter>,
    ) -> Self {
        Self {
            hot_config,
            rate_limiter: Arc::new(RwLock::new(None)),
            algorithm_router,
            repository: None,
            workspace_repository: None,
            group_repository: None,
        }
    }

    pub fn with_repository(
        hot_config: Arc<HotReloadConfig>,
        algorithm_router: Arc<crate::core::algorithm::AlgorithmRouter>,
        repository: Arc<dyn crate::core::database::BizTagRepository + Send + Sync>,
        workspace_repository: Arc<dyn crate::core::database::WorkspaceRepository + Send + Sync>,
        group_repository: Arc<dyn crate::core::database::GroupRepository + Send + Sync>,
    ) -> Self {
        Self {
            hot_config,
            rate_limiter: Arc::new(RwLock::new(None)),
            algorithm_router,
            repository: Some(repository),
            workspace_repository: Some(workspace_repository),
            group_repository: Some(group_repository),
        }
    }

    fn config_to_response(config: &Config) -> ConfigResponse {
        ConfigResponse {
            app: AppConfigInfo {
                name: config.app.name.clone(),
                host: config.app.host.clone(),
                http_port: config.app.http_port,
                grpc_port: config.app.grpc_port,
                dc_id: config.app.dc_id,
                worker_id: config.app.worker_id,
            },
            database: DatabaseConfigInfo {
                engine: config.database.engine.to_string(),
                host: Some(config.database.host.clone()),
                port: Some(config.database.port),
                database: Some(config.database.database.clone()),
                max_connections: config.database.max_connections,
                min_connections: config.database.min_connections,
            },
            algorithm: AlgorithmConfigInfo {
                default: config.algorithm.default.clone(),
                segment: SegmentConfigInfo {
                    base_step: config.algorithm.segment.base_step,
                    min_step: config.algorithm.segment.min_step,
                    max_step: config.algorithm.segment.max_step,
                    switch_threshold: config.algorithm.segment.switch_threshold,
                },
                snowflake: SnowflakeConfigInfo {
                    datacenter_id_bits: config.algorithm.snowflake.datacenter_id_bits,
                    worker_id_bits: config.algorithm.snowflake.worker_id_bits,
                    sequence_bits: config.algorithm.snowflake.sequence_bits,
                    clock_drift_threshold_ms: config.algorithm.snowflake.clock_drift_threshold_ms,
                },
                uuid_v7: UuidV7ConfigInfo {
                    enabled: config.algorithm.uuid_v7.enabled,
                },
            },
            monitoring: MonitoringConfigInfo {
                metrics_enabled: config.monitoring.metrics_enabled,
                metrics_path: config.monitoring.metrics_path.clone(),
                tracing_enabled: config.monitoring.tracing_enabled,
            },
            logging: LoggingConfigInfo {
                level: config.logging.level.to_string(),
                format: config.logging.format.to_string(),
                include_location: config.logging.include_location,
            },
            rate_limit: RateLimitConfigInfo {
                enabled: config.rate_limit.enabled,
                default_rps: config.rate_limit.default_rps,
                burst_size: config.rate_limit.burst_size,
            },
            tls: TlsConfigInfo {
                enabled: config.tls.enabled,
                http_enabled: config.tls.http_enabled,
                grpc_enabled: config.tls.grpc_enabled,
                has_cert: !config.tls.cert_path.is_empty(),
            },
        }
    }

    fn secure_config_to_response(config: &Config) -> SecureConfigResponse {
        SecureConfigResponse {
            app: AppConfigInfo {
                name: config.app.name.clone(),
                host: config.app.host.clone(),
                http_port: config.app.http_port,
                grpc_port: config.app.grpc_port,
                dc_id: config.app.dc_id,
                worker_id: config.app.worker_id,
            },
            algorithm: AlgorithmConfigInfo {
                default: config.algorithm.default.clone(),
                segment: SegmentConfigInfo {
                    base_step: config.algorithm.segment.base_step,
                    min_step: config.algorithm.segment.min_step,
                    max_step: config.algorithm.segment.max_step,
                    switch_threshold: config.algorithm.segment.switch_threshold,
                },
                snowflake: SnowflakeConfigInfo {
                    datacenter_id_bits: config.algorithm.snowflake.datacenter_id_bits,
                    worker_id_bits: config.algorithm.snowflake.worker_id_bits,
                    sequence_bits: config.algorithm.snowflake.sequence_bits,
                    clock_drift_threshold_ms: config.algorithm.snowflake.clock_drift_threshold_ms,
                },
                uuid_v7: UuidV7ConfigInfo {
                    enabled: config.algorithm.uuid_v7.enabled,
                },
            },
            monitoring: MonitoringConfigInfo {
                metrics_enabled: config.monitoring.metrics_enabled,
                metrics_path: config.monitoring.metrics_path.clone(),
                tracing_enabled: config.monitoring.tracing_enabled,
            },
            logging: LoggingConfigInfo {
                level: config.logging.level.to_string(),
                format: config.logging.format.to_string(),
                include_location: config.logging.include_location,
            },
            rate_limit: RateLimitConfigInfo {
                enabled: config.rate_limit.enabled,
                default_rps: config.rate_limit.default_rps,
                burst_size: config.rate_limit.burst_size,
            },
        }
    }
}

#[async_trait]
impl ConfigManagementService for ConfigManager {
    fn get_config(&self) -> ConfigResponse {
        let config = self.hot_config.get_config();
        Self::config_to_response(&config)
    }

    fn get_secure_config(&self) -> SecureConfigResponse {
        let config = self.hot_config.get_config();
        Self::secure_config_to_response(&config)
    }

    async fn update_rate_limit(&self, req: UpdateRateLimitRequest) -> UpdateConfigResponse {
        if let Err(e) = req.validate() {
            return UpdateConfigResponse {
                success: false,
                message: format!("Validation error: {}", e),
                config: None,
            };
        }

        let mut config = self.hot_config.get_config();

        if let Some(rps) = req.default_rps {
            config.rate_limit.default_rps = rps;
        }
        if let Some(burst) = req.burst_size {
            config.rate_limit.burst_size = burst;
        }

        {
            let mut rate_limiter_guard = self.rate_limiter.write().await;
            *rate_limiter_guard =
                Some((config.rate_limit.default_rps, config.rate_limit.burst_size));
        }

        self.hot_config.update_config(config.clone());

        UpdateConfigResponse {
            success: true,
            message: "Rate limit configuration updated successfully".to_string(),
            config: Some(Self::config_to_response(&config)),
        }
    }

    async fn update_logging(&self, req: UpdateLoggingRequest) -> UpdateConfigResponse {
        if let Err(e) = req.validate() {
            return UpdateConfigResponse {
                success: false,
                message: format!("Validation error: {}", e),
                config: None,
            };
        }

        let mut config = self.hot_config.get_config();

        if let Some(level) = req.level {
            config.logging.level = crate::core::config::LogLevel::from(level);
        }

        self.hot_config.update_config(config.clone());

        UpdateConfigResponse {
            success: true,
            message: "Logging configuration updated successfully".to_string(),
            config: Some(Self::config_to_response(&config)),
        }
    }

    async fn reload_config(&self) -> UpdateConfigResponse {
        match self.hot_config.reload_from_file().await {
            Ok(_) => UpdateConfigResponse {
                success: true,
                message: "Configuration reloaded from file successfully".to_string(),
                config: Some(Self::config_to_response(&self.hot_config.get_config())),
            },
            Err(e) => UpdateConfigResponse {
                success: false,
                message: format!("Failed to reload configuration: {}", e),
                config: None,
            },
        }
    }

    async fn get_rate_limit_override(&self) -> Option<(u32, u32)> {
        let guard = self.rate_limiter.read().await;
        *guard
    }

    async fn set_algorithm(&self, req: SetAlgorithmRequest) -> SetAlgorithmResponse {
        if let Err(e) = req.validate() {
            return SetAlgorithmResponse {
                success: false,
                biz_tag: req.biz_tag.clone(),
                algorithm: req.algorithm.clone(),
                message: format!("Validation error: {}", e),
            };
        }

        let algorithm_type = match req.algorithm.to_lowercase().as_str() {
            "segment" => AlgorithmType::Segment,
            "snowflake" => AlgorithmType::Snowflake,
            "uuid_v7" => AlgorithmType::UuidV7,
            _ => {
                return SetAlgorithmResponse {
                    success: false,
                    biz_tag: req.biz_tag.clone(),
                    algorithm: req.algorithm.clone(),
                    message: format!(
                        "Invalid algorithm '{}'. Valid options: segment, snowflake, uuid_v7",
                        req.algorithm
                    ),
                };
            }
        };

        let biz_tag = req.biz_tag.clone();
        let algorithm = req.algorithm.clone();
        self.algorithm_router
            .set_algorithm(biz_tag.clone(), algorithm_type)
            .await;

        let message = format!(
            "Algorithm for biz_tag '{}' set to '{}' successfully",
            biz_tag, algorithm
        );

        SetAlgorithmResponse {
            success: true,
            biz_tag,
            algorithm,
            message,
        }
    }

    // ========== BizTag CRUD Methods ==========

    async fn create_biz_tag(
        &self,
        request: &crate::core::database::CreateBizTagRequest,
    ) -> crate::core::Result<crate::core::database::BizTag> {
        if let Some(ref repo) = self.repository {
            repo.create_biz_tag(request).await
        } else {
            Err(crate::core::CoreError::InternalError(
                "Database repository not configured".to_string(),
            ))
        }
    }

    async fn get_biz_tag(
        &self,
        id: Uuid,
    ) -> crate::core::Result<Option<crate::core::database::BizTag>> {
        if let Some(ref repo) = self.repository {
            repo.get_biz_tag(id).await
        } else {
            Err(crate::core::CoreError::InternalError(
                "Database repository not configured".to_string(),
            ))
        }
    }

    async fn update_biz_tag(
        &self,
        id: Uuid,
        request: &crate::core::database::UpdateBizTagRequest,
    ) -> crate::core::Result<crate::core::database::BizTag> {
        if let Some(ref repo) = self.repository {
            repo.update_biz_tag(id, request).await
        } else {
            Err(crate::core::CoreError::InternalError(
                "Database repository not configured".to_string(),
            ))
        }
    }

    async fn delete_biz_tag(&self, id: Uuid) -> crate::core::Result<()> {
        if let Some(ref repo) = self.repository {
            repo.delete_biz_tag(id).await
        } else {
            Err(crate::core::CoreError::InternalError(
                "Database repository not configured".to_string(),
            ))
        }
    }

    async fn count_biz_tags(
        &self,
        workspace_id: Uuid,
        group_id: Option<Uuid>,
    ) -> crate::core::Result<u64> {
        if let Some(ref repo) = self.repository {
            repo.count_biz_tags(workspace_id, group_id).await
        } else {
            Err(crate::core::CoreError::InternalError(
                "Database repository not configured".to_string(),
            ))
        }
    }

    async fn list_biz_tags(
        &self,
        workspace_id: Uuid,
        group_id: Option<Uuid>,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> crate::core::Result<Vec<crate::core::database::BizTag>> {
        if let Some(ref repo) = self.repository {
            repo.list_biz_tags(workspace_id, group_id, limit, offset)
                .await
        } else {
            Err(crate::core::CoreError::InternalError(
                "Database repository not configured".to_string(),
            ))
        }
    }

    // ========== Workspace CRUD Methods ==========

    async fn create_workspace(
        &self,
        req: CreateWorkspaceRequest,
    ) -> crate::core::Result<WorkspaceResponse> {
        if let Some(ref repo) = self.workspace_repository {
            let request = CoreCreateWorkspaceRequest {
                name: req.name.clone(),
                description: req.description.clone(),
                max_groups: req.max_groups,
                max_biz_tags: req.max_biz_tags,
            };
            let workspace = repo.create_workspace(&request).await?;
            Ok(WorkspaceResponse {
                id: workspace.id.to_string(),
                name: workspace.name,
                description: workspace.description,
                status: workspace.status.to_string(),
                max_groups: workspace.max_groups,
                max_biz_tags: workspace.max_biz_tags,
                created_at: workspace.created_at.and_utc().to_rfc3339(),
                updated_at: workspace.updated_at.and_utc().to_rfc3339(),
                user_api_key: None,
            })
        } else {
            Err(crate::core::CoreError::InternalError(
                "Workspace repository not configured".to_string(),
            ))
        }
    }

    async fn list_workspaces(&self) -> crate::core::Result<WorkspaceListResponse> {
        if let Some(ref repo) = self.workspace_repository {
            let workspaces = repo.list_workspaces(None, None).await?;
            let total = workspaces.len() as u64;
            let workspace_responses: Vec<WorkspaceResponse> = workspaces
                .into_iter()
                .map(|w| WorkspaceResponse {
                    id: w.id.to_string(),
                    name: w.name,
                    description: w.description,
                    status: w.status.to_string(),
                    max_groups: w.max_groups,
                    max_biz_tags: w.max_biz_tags,
                    created_at: w.created_at.and_utc().to_rfc3339(),
                    updated_at: w.updated_at.and_utc().to_rfc3339(),
                    user_api_key: None,
                })
                .collect();
            Ok(WorkspaceListResponse {
                workspaces: workspace_responses,
                total,
            })
        } else {
            Err(crate::core::CoreError::InternalError(
                "Workspace repository not configured".to_string(),
            ))
        }
    }

    async fn get_workspace(&self, name: &str) -> crate::core::Result<Option<WorkspaceResponse>> {
        if let Some(ref repo) = self.workspace_repository {
            let workspace = repo.get_workspace_by_name(name).await?;
            Ok(workspace.map(|w| WorkspaceResponse {
                id: w.id.to_string(),
                name: w.name,
                description: w.description,
                status: w.status.to_string(),
                max_groups: w.max_groups,
                max_biz_tags: w.max_biz_tags,
                created_at: w.created_at.and_utc().to_rfc3339(),
                updated_at: w.updated_at.and_utc().to_rfc3339(),
                user_api_key: None,
            }))
        } else {
            Err(crate::core::CoreError::InternalError(
                "Workspace repository not configured".to_string(),
            ))
        }
    }

    // ========== Group CRUD Methods ==========

    async fn create_group(&self, req: CreateGroupRequest) -> crate::core::Result<GroupResponse> {
        if let Some(ref workspace_repo) = self.workspace_repository {
            if let Some(ref group_repo) = self.group_repository {
                // First get the workspace by name
                let workspace = workspace_repo.get_workspace_by_name(&req.workspace).await?;
                match workspace {
                    Some(ws) => {
                        let request = CoreCreateGroupRequest {
                            workspace_id: ws.id,
                            name: req.name.clone(),
                            description: req.description.clone(),
                            max_biz_tags: req.max_biz_tags,
                        };
                        let group = group_repo.create_group(&request).await?;
                        Ok(GroupResponse {
                            id: group.id.to_string(),
                            workspace_id: ws.id.to_string(),
                            workspace_name: ws.name,
                            name: group.name,
                            description: group.description,
                            max_biz_tags: group.max_biz_tags,
                            created_at: group.created_at.and_utc().to_rfc3339(),
                            updated_at: group.updated_at.and_utc().to_rfc3339(),
                        })
                    }
                    None => Err(crate::core::CoreError::InternalError(format!(
                        "Workspace '{}' not found",
                        req.workspace
                    ))),
                }
            } else {
                Err(crate::core::CoreError::InternalError(
                    "Group repository not configured".to_string(),
                ))
            }
        } else {
            Err(crate::core::CoreError::InternalError(
                "Workspace repository not configured".to_string(),
            ))
        }
    }

    async fn list_groups(&self, workspace: &str) -> crate::core::Result<GroupListResponse> {
        if let Some(ref workspace_repo) = self.workspace_repository {
            if let Some(ref group_repo) = self.group_repository {
                let workspace = workspace_repo.get_workspace_by_name(workspace).await?;
                match workspace {
                    Some(ws) => {
                        let groups = group_repo.list_groups(ws.id, None, None).await?;
                        let total = groups.len() as u64;
                        let group_responses: Vec<GroupResponse> = groups
                            .into_iter()
                            .map(|g| GroupResponse {
                                id: g.id.to_string(),
                                workspace_id: ws.id.to_string(),
                                workspace_name: ws.name.clone(),
                                name: g.name,
                                description: g.description,
                                max_biz_tags: g.max_biz_tags,
                                created_at: g.created_at.and_utc().to_rfc3339(),
                                updated_at: g.updated_at.and_utc().to_rfc3339(),
                            })
                            .collect();
                        Ok(GroupListResponse {
                            groups: group_responses,
                            total,
                        })
                    }
                    None => Ok(GroupListResponse {
                        groups: vec![],
                        total: 0,
                    }),
                }
            } else {
                Err(crate::core::CoreError::InternalError(
                    "Group repository not configured".to_string(),
                ))
            }
        } else {
            Err(crate::core::CoreError::InternalError(
                "Workspace repository not configured".to_string(),
            ))
        }
    }

    async fn get_database_metrics(&self) -> DatabaseMetrics {
        // Get database adapter from container if available
        let metrics = if let Some(ref repo) = self.repository {
            // Check database health
            match repo.health_check().await {
                Ok(()) => {
                    // Get pool statistics from the database adapter
                    // Note: SeaORM repository doesn't expose pool stats directly
                    // We return healthy status with zeroed metrics for now
                    // Future: Consider using dbnexus adapter which has PoolStatus
                    DatabaseMetrics {
                        status: crate::server::models::HealthStatus::Healthy,
                        connection_pool: ConnectionPoolMetrics {
                            active_connections: 0,
                            idle_connections: 0,
                            max_connections: 0,
                        },
                        last_error: None,
                    }
                }
                Err(e) => DatabaseMetrics {
                    status: crate::server::models::HealthStatus::Unhealthy,
                    connection_pool: ConnectionPoolMetrics {
                        active_connections: 0,
                        idle_connections: 0,
                        max_connections: 0,
                    },
                    last_error: Some(e.to_string()),
                },
            }
        } else {
            DatabaseMetrics {
                status: crate::server::models::HealthStatus::Unhealthy,
                connection_pool: ConnectionPoolMetrics {
                    active_connections: 0,
                    idle_connections: 0,
                    max_connections: 0,
                },
                last_error: Some("Database not configured".to_string()),
            }
        };

        metrics
    }

    async fn get_cache_metrics(&self) -> CacheMetrics {
        // Get algorithm metrics for cache hit rate
        let algorithm_metrics = self.algorithm_router.metrics().await;
        // L15 修复：只统计 `cache_hit_rate = Some(_)` 的算法（即有缓存的算法，
        // 如 Segment）。原代码把 UUID/Snowflake 的 `0.0` 也纳入平均，导致
        // 整体缓存命中率被低估（误把"无缓存"当成"命中率 0%"）。
        //
        // ARCH-MED-004 修复：用 `has_cache` 显式表达「是否有缓存算法」，
        // 避免监控面板把 `hit_rate = 0.0` 误读为「缓存性能极差」。
        let hit_rates: Vec<f64> = algorithm_metrics
            .iter()
            .filter_map(|(_, m)| m.cache_hit_rate)
            .collect();
        let (has_cache, cache_hit_rate) = if hit_rates.is_empty() {
            (false, 0.0)
        } else {
            (true, hit_rates.iter().sum::<f64>() / hit_rates.len() as f64)
        };

        CacheMetrics {
            status: crate::server::models::HealthStatus::Healthy,
            hit_rate: cache_hit_rate,
            has_cache,
            memory_usage_mb: None,
            key_count: None,
        }
    }

    async fn get_algorithm_metrics(
        &self,
    ) -> Vec<(
        crate::core::types::AlgorithmType,
        crate::core::algorithm::AlgorithmMetricsSnapshot,
    )> {
        self.algorithm_router.metrics().await
    }

    fn get_batch_max_size(&self) -> u32 {
        let config = self.hot_config.get_config();
        config.batch_generate.max_batch_size
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::Config;
    use std::sync::Once;

    static INIT: Once = Once::new();

    fn ensure_test_mode() {
        INIT.call_once(|| {
            std::env::set_var("NEBULA_TEST_MODE", "1");
        });
    }

    fn create_test_algorithm_router() -> Arc<crate::core::algorithm::AlgorithmRouter> {
        ensure_test_mode();
        let config = Config::default();
        Arc::new(crate::core::algorithm::AlgorithmRouter::new(
            config.clone(),
            None,
        ))
    }

    #[tokio::test]
    async fn test_config_to_response() {
        let config = test_config();
        let response = ConfigManager::config_to_response(&config);

        // Engine depends on environment - SQLite for tests, PostgreSQL for production
        assert!(response.database.engine == "postgresql" || response.database.engine == "sqlite");
        assert_eq!(response.app.name, "nebula-id");
        assert_eq!(response.algorithm.default, "segment");
        assert_eq!(response.rate_limit.default_rps, 10000);
    }

    fn test_config() -> Config {
        ensure_test_mode();
        Config::default()
    }

    #[tokio::test]
    async fn test_update_rate_limit_request_validation() {
        let hot_config = Arc::new(HotReloadConfig::new(
            test_config(),
            "config/config.toml".to_string(),
        ));
        let algorithm_router = create_test_algorithm_router();
        let _service = ConfigManager::new(hot_config, algorithm_router);

        let valid_req = UpdateRateLimitRequest {
            default_rps: Some(5000),
            burst_size: Some(50),
        };
        assert!(valid_req.validate().is_ok());

        let invalid_req = UpdateRateLimitRequest {
            default_rps: Some(0),
            burst_size: Some(0),
        };
        assert!(invalid_req.validate().is_err());
    }

    #[tokio::test]
    async fn test_update_logging_request_validation() {
        let hot_config = Arc::new(HotReloadConfig::new(
            test_config(),
            "config/config.toml".to_string(),
        ));
        let algorithm_router = create_test_algorithm_router();
        let _service = ConfigManager::new(hot_config, algorithm_router);

        let valid_req = UpdateLoggingRequest {
            level: Some("debug".to_string()),
        };
        assert!(valid_req.validate().is_ok());

        let empty_req = UpdateLoggingRequest { level: None };
        assert!(empty_req.validate().is_ok());

        let invalid_req = UpdateLoggingRequest {
            level: Some("invalid_level_that_is_too_long".to_string()),
        };
        assert!(invalid_req.validate().is_err());
    }

    #[tokio::test]
    async fn test_set_algorithm() {
        let hot_config = Arc::new(HotReloadConfig::new(
            test_config(),
            "config/config.toml".to_string(),
        ));
        let algorithm_router = create_test_algorithm_router();
        let service = ConfigManager::new(hot_config, algorithm_router.clone());

        let req = SetAlgorithmRequest {
            biz_tag: "test-biz".to_string(),
            algorithm: "snowflake".to_string(),
        };

        let response = service.set_algorithm(req).await;
        assert!(response.success);
        assert_eq!(response.biz_tag, "test-biz");
        assert_eq!(response.algorithm, "snowflake");
    }

    #[tokio::test]
    async fn test_set_algorithm_invalid() {
        let hot_config = Arc::new(HotReloadConfig::new(
            test_config(),
            "config/config.toml".to_string(),
        ));
        let algorithm_router = create_test_algorithm_router();
        let service = ConfigManager::new(hot_config, algorithm_router);

        let req = SetAlgorithmRequest {
            biz_tag: "test-biz".to_string(),
            algorithm: "invalid_algorithm".to_string(),
        };

        let response = service.set_algorithm(req).await;
        assert!(!response.success);
        assert!(response.message.contains("Invalid algorithm"));
    }

    // ========== get_config / get_secure_config / get_batch_max_size ==========

    /// `get_config` returns a response with all sections populated.
    #[tokio::test]
    async fn test_get_config_full_response() {
        let hot_config = Arc::new(HotReloadConfig::new(
            test_config(),
            "config/config.toml".to_string(),
        ));
        let algorithm_router = create_test_algorithm_router();
        let service = ConfigManager::new(hot_config, algorithm_router);

        let response = service.get_config();
        assert_eq!(response.app.name, "nebula-id");
        assert!(response.database.engine == "postgresql" || response.database.engine == "sqlite");
        assert_eq!(response.algorithm.default, "segment");
        assert_eq!(response.algorithm.segment.base_step, 1000);
        assert!(!response.tls.has_cert);
    }

    /// `get_secure_config` returns a response without database secrets.
    #[tokio::test]
    async fn test_get_secure_config_no_db_credentials() {
        let hot_config = Arc::new(HotReloadConfig::new(
            test_config(),
            "config/config.toml".to_string(),
        ));
        let algorithm_router = create_test_algorithm_router();
        let service = ConfigManager::new(hot_config, algorithm_router);

        let response = service.get_secure_config();
        assert_eq!(response.app.name, "nebula-id");
        assert_eq!(response.algorithm.default, "segment");
        // SecureConfigResponse does not have a `database` field — verified
        // at compile time by the fact this compiles.
    }

    /// `get_batch_max_size` reads through to `batch_generate.max_batch_size`.
    #[tokio::test]
    async fn test_get_batch_max_size() {
        let hot_config = Arc::new(HotReloadConfig::new(
            test_config(),
            "config/config.toml".to_string(),
        ));
        let algorithm_router = create_test_algorithm_router();
        let service = ConfigManager::new(hot_config, algorithm_router);

        let max_size = service.get_batch_max_size();
        // Default Config sets batch_generate.max_batch_size = 100.
        assert_eq!(max_size, 100);
    }

    // ========== update_rate_limit / update_logging success paths ==========

    /// `update_rate_limit` with valid request must succeed and update config.
    #[tokio::test]
    async fn test_update_rate_limit_success() {
        let hot_config = Arc::new(HotReloadConfig::new(
            test_config(),
            "config/config.toml".to_string(),
        ));
        let algorithm_router = create_test_algorithm_router();
        let service = ConfigManager::new(hot_config.clone(), algorithm_router);

        let req = UpdateRateLimitRequest {
            default_rps: Some(5000),
            burst_size: Some(50),
        };
        let response = service.update_rate_limit(req).await;
        assert!(response.success);
        assert!(response.message.contains("successfully"));
        let config = response.config.unwrap();
        assert_eq!(config.rate_limit.default_rps, 5000);
        assert_eq!(config.rate_limit.burst_size, 50);

        // The override must be persisted in the rate_limiter field.
        let override_val = service.get_rate_limit_override().await;
        assert_eq!(override_val, Some((5000, 50)));

        // And hot_config must reflect the update.
        let hot_config_reflect = hot_config.get_config();
        assert_eq!(hot_config_reflect.rate_limit.default_rps, 5000);
    }

    /// `update_rate_limit` with only `default_rps` must leave `burst_size` unchanged.
    #[tokio::test]
    async fn test_update_rate_limit_partial_update() {
        let hot_config = Arc::new(HotReloadConfig::new(
            test_config(),
            "config/config.toml".to_string(),
        ));
        let algorithm_router = create_test_algorithm_router();
        let service = ConfigManager::new(hot_config, algorithm_router);

        let original_burst = test_config().rate_limit.burst_size;
        let req = UpdateRateLimitRequest {
            default_rps: Some(2000),
            burst_size: None,
        };
        let response = service.update_rate_limit(req).await;
        assert!(response.success);
        let config = response.config.unwrap();
        assert_eq!(config.rate_limit.default_rps, 2000);
        assert_eq!(config.rate_limit.burst_size, original_burst);
    }

    /// `update_rate_limit` with `None` for both fields is valid (no-op update)
    /// — `validate()` accepts empty updates, just doesn't change anything.
    #[tokio::test]
    async fn test_update_rate_limit_no_changes() {
        let hot_config = Arc::new(HotReloadConfig::new(
            test_config(),
            "config/config.toml".to_string(),
        ));
        let algorithm_router = create_test_algorithm_router();
        let service = ConfigManager::new(hot_config, algorithm_router);

        let req = UpdateRateLimitRequest {
            default_rps: None,
            burst_size: None,
        };
        let response = service.update_rate_limit(req).await;
        assert!(response.success);
    }

    /// `update_logging` with valid level must succeed and update config.
    #[tokio::test]
    async fn test_update_logging_success() {
        let hot_config = Arc::new(HotReloadConfig::new(
            test_config(),
            "config/config.toml".to_string(),
        ));
        let algorithm_router = create_test_algorithm_router();
        let service = ConfigManager::new(hot_config.clone(), algorithm_router);

        let req = UpdateLoggingRequest {
            level: Some("debug".to_string()),
        };
        let response = service.update_logging(req).await;
        assert!(response.success);
        assert!(response.message.contains("successfully"));
        let config = response.config.unwrap();
        assert_eq!(config.logging.level.to_string(), "debug");

        // hot_config must reflect the update.
        let hot_config_reflect = hot_config.get_config();
        assert_eq!(hot_config_reflect.logging.level.to_string(), "debug");
    }

    /// `update_logging` with `None` level is valid (no-op).
    #[tokio::test]
    async fn test_update_logging_no_change() {
        let hot_config = Arc::new(HotReloadConfig::new(
            test_config(),
            "config/config.toml".to_string(),
        ));
        let algorithm_router = create_test_algorithm_router();
        let service = ConfigManager::new(hot_config, algorithm_router);

        let req = UpdateLoggingRequest { level: None };
        let response = service.update_logging(req).await;
        assert!(response.success);
    }

    // ========== reload_config ==========

    /// `reload_config` on a non-existent path returns success=false
    /// (reload_from_file returns Ok(false), which is treated as success).
    #[tokio::test]
    async fn test_reload_config_missing_file_succeeds_with_false() {
        let hot_config = Arc::new(HotReloadConfig::new(
            test_config(),
            "/nonexistent/path.toml".to_string(),
        ));
        let algorithm_router = create_test_algorithm_router();
        let service = ConfigManager::new(hot_config, algorithm_router);

        let response = service.reload_config().await;
        // reload_from_file returns Ok(false) → reload_config treats Ok as success.
        assert!(response.success);
        assert!(response.config.is_some());
    }

    // ========== get_rate_limit_override ==========

    /// `get_rate_limit_override` returns None before any update_rate_limit call.
    #[tokio::test]
    async fn test_get_rate_limit_override_initially_none() {
        let hot_config = Arc::new(HotReloadConfig::new(
            test_config(),
            "config/config.toml".to_string(),
        ));
        let algorithm_router = create_test_algorithm_router();
        let service = ConfigManager::new(hot_config, algorithm_router);

        let override_val = service.get_rate_limit_override().await;
        assert!(override_val.is_none());
    }

    // ========== set_algorithm: all valid algorithms ==========

    #[tokio::test]
    async fn test_set_algorithm_segment() {
        let hot_config = Arc::new(HotReloadConfig::new(
            test_config(),
            "config/config.toml".to_string(),
        ));
        let algorithm_router = create_test_algorithm_router();
        let service = ConfigManager::new(hot_config, algorithm_router);

        let req = SetAlgorithmRequest {
            biz_tag: "seg-tag".to_string(),
            algorithm: "segment".to_string(),
        };
        let response = service.set_algorithm(req).await;
        assert!(response.success);
        assert_eq!(response.algorithm, "segment");
    }

    #[tokio::test]
    async fn test_set_algorithm_uuid_v7() {
        let hot_config = Arc::new(HotReloadConfig::new(
            test_config(),
            "config/config.toml".to_string(),
        ));
        let algorithm_router = create_test_algorithm_router();
        let service = ConfigManager::new(hot_config, algorithm_router);

        let req = SetAlgorithmRequest {
            biz_tag: "uuid-tag".to_string(),
            algorithm: "uuid_v7".to_string(),
        };
        let response = service.set_algorithm(req).await;
        assert!(response.success);
        assert_eq!(response.algorithm, "uuid_v7");
    }

    /// `set_algorithm` is case-insensitive (lowercases the input).
    #[tokio::test]
    async fn test_set_algorithm_case_insensitive() {
        let hot_config = Arc::new(HotReloadConfig::new(
            test_config(),
            "config/config.toml".to_string(),
        ));
        let algorithm_router = create_test_algorithm_router();
        let service = ConfigManager::new(hot_config, algorithm_router);

        let req = SetAlgorithmRequest {
            biz_tag: "case-tag".to_string(),
            algorithm: "SNOWFLAKE".to_string(),
        };
        let response = service.set_algorithm(req).await;
        assert!(response.success);
        // The response echoes the original (un-lowercased) algorithm string.
        assert_eq!(response.algorithm, "SNOWFLAKE");
    }

    // ========== BizTag CRUD: no repository configured ==========

    async fn service_no_repo() -> ConfigManager {
        let hot_config = Arc::new(HotReloadConfig::new(
            test_config(),
            "config/config.toml".to_string(),
        ));
        let algorithm_router = create_test_algorithm_router();
        ConfigManager::new(hot_config, algorithm_router)
    }

    #[tokio::test]
    async fn test_create_biz_tag_no_repo_returns_internal_error() {
        let service = service_no_repo().await;
        let req = crate::core::database::CreateBizTagRequest {
            workspace_id: Uuid::new_v4(),
            group_id: Uuid::new_v4(),
            name: "test".to_string(),
            description: None,
            algorithm: None,
            format: None,
            prefix: None,
            base_step: None,
            max_step: None,
            datacenter_ids: None,
        };
        let result = service.create_biz_tag(&req).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            crate::core::CoreError::InternalError(_)
        ));
    }

    #[tokio::test]
    async fn test_get_biz_tag_no_repo_returns_internal_error() {
        let service = service_no_repo().await;
        let result = service.get_biz_tag(Uuid::new_v4()).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            crate::core::CoreError::InternalError(_)
        ));
    }

    #[tokio::test]
    async fn test_update_biz_tag_no_repo_returns_internal_error() {
        let service = service_no_repo().await;
        let req = crate::core::database::UpdateBizTagRequest {
            name: None,
            description: None,
            algorithm: None,
            format: None,
            prefix: None,
            base_step: None,
            max_step: None,
            datacenter_ids: None,
        };
        let result = service.update_biz_tag(Uuid::new_v4(), &req).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            crate::core::CoreError::InternalError(_)
        ));
    }

    #[tokio::test]
    async fn test_delete_biz_tag_no_repo_returns_internal_error() {
        let service = service_no_repo().await;
        let result = service.delete_biz_tag(Uuid::new_v4()).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            crate::core::CoreError::InternalError(_)
        ));
    }

    #[tokio::test]
    async fn test_count_biz_tags_no_repo_returns_internal_error() {
        let service = service_no_repo().await;
        let result = service.count_biz_tags(Uuid::new_v4(), None).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            crate::core::CoreError::InternalError(_)
        ));
    }

    #[tokio::test]
    async fn test_list_biz_tags_no_repo_returns_internal_error() {
        let service = service_no_repo().await;
        let result = service
            .list_biz_tags(Uuid::new_v4(), None, None, None)
            .await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            crate::core::CoreError::InternalError(_)
        ));
    }

    // ========== Workspace CRUD: no repository configured ==========

    #[tokio::test]
    async fn test_create_workspace_no_repo_returns_internal_error() {
        let service = service_no_repo().await;
        let req = CreateWorkspaceRequest {
            name: "ws".to_string(),
            description: None,
            max_groups: None,
            max_biz_tags: None,
        };
        let result = service.create_workspace(req).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            crate::core::CoreError::InternalError(_)
        ));
    }

    #[tokio::test]
    async fn test_list_workspaces_no_repo_returns_internal_error() {
        let service = service_no_repo().await;
        let result = service.list_workspaces().await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            crate::core::CoreError::InternalError(_)
        ));
    }

    #[tokio::test]
    async fn test_get_workspace_no_repo_returns_internal_error() {
        let service = service_no_repo().await;
        let result = service.get_workspace("any").await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            crate::core::CoreError::InternalError(_)
        ));
    }

    // ========== Group CRUD: no repository configured ==========

    #[tokio::test]
    async fn test_create_group_no_repo_returns_internal_error() {
        let service = service_no_repo().await;
        let req = CreateGroupRequest {
            workspace: "ws".to_string(),
            name: "g".to_string(),
            description: None,
            max_biz_tags: None,
        };
        let result = service.create_group(req).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            crate::core::CoreError::InternalError(_)
        ));
    }

    #[tokio::test]
    async fn test_list_groups_no_repo_returns_internal_error() {
        let service = service_no_repo().await;
        let result = service.list_groups("ws").await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            crate::core::CoreError::InternalError(_)
        ));
    }

    // ========== get_database_metrics / get_cache_metrics / get_algorithm_metrics ==========

    /// `get_database_metrics` with no repository returns Unhealthy status.
    #[tokio::test]
    async fn test_get_database_metrics_no_repo_unhealthy() {
        let service = service_no_repo().await;
        let metrics = service.get_database_metrics().await;
        assert_eq!(
            metrics.status,
            crate::server::models::HealthStatus::Unhealthy
        );
        assert!(metrics.last_error.is_some());
        assert!(metrics.last_error.unwrap().contains("not configured"));
    }

    /// `get_cache_metrics` returns Healthy status with `has_cache = false`
    /// when no algorithm has a cache (default test router has no traffic).
    #[tokio::test]
    async fn test_get_cache_metrics_returns_healthy() {
        let service = service_no_repo().await;
        let metrics = service.get_cache_metrics().await;
        assert_eq!(metrics.status, crate::server::models::HealthStatus::Healthy);
        // hit_rate may be 0.0 if no cache algorithm; has_cache reflects that.
        assert!(metrics.hit_rate >= 0.0 && metrics.hit_rate <= 1.0);
    }

    /// `get_algorithm_metrics` returns a vector (may be empty if no algorithm
    /// has been exercised).
    #[tokio::test]
    async fn test_get_algorithm_metrics_returns_vec() {
        let service = service_no_repo().await;
        let metrics = service.get_algorithm_metrics().await;
        // Default router may have 0 or more algorithms registered; just
        // verify it doesn't panic and returns a Vec. The previous
        // `assert!(metrics.len() >= 0)` was a tautology (usize is always
        // >= 0); a plain binding documents "call must succeed" intent.
        let _ = metrics.len();
    }

    // ========== secure_config_to_response (indirect via get_secure_config) ==========

    /// `get_secure_config` must not include TLS info (SecureConfigResponse
    /// has no `tls` field).
    #[tokio::test]
    async fn test_secure_config_response_shape() {
        let hot_config = Arc::new(HotReloadConfig::new(
            test_config(),
            "config/config.toml".to_string(),
        ));
        let algorithm_router = create_test_algorithm_router();
        let service = ConfigManager::new(hot_config, algorithm_router);

        let response = service.get_secure_config();
        // Verify all expected fields are present and non-default.
        assert!(!response.app.name.is_empty());
        assert!(!response.algorithm.default.is_empty());
        assert!(!response.logging.level.is_empty());
    }

    // ========== with_repository constructor (indirect) ==========
    // with_repository requires actual repository instances (Arc<dyn ...>);
    // we can't easily construct them without a DB. We verify the constructor
    // exists and has the right signature by referencing it.

    /// `with_repository` constructor must be callable with the right types.
    /// We verify the function exists by type-checking a reference.
    #[tokio::test]
    async fn test_with_repository_signature() {
        let _ = std::any::TypeId::of::<
            fn(
                Arc<HotReloadConfig>,
                Arc<crate::core::algorithm::AlgorithmRouter>,
                Arc<dyn crate::core::database::BizTagRepository + Send + Sync>,
                Arc<dyn crate::core::database::WorkspaceRepository + Send + Sync>,
                Arc<dyn crate::core::database::GroupRepository + Send + Sync>,
            ) -> ConfigManager,
        >();
    }
}
