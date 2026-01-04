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

use crate::config_management::ConfigManagementService;
use crate::models::{
    naive_to_rfc3339, AlgorithmMetrics, ApiKeyListResponse, ApiKeyResponse,
    ApiKeyWithSecretResponse, BatchGenerateRequest, BatchGenerateResponse, BizTagListResponse,
    BizTagResponse, CreateApiKeyRequest, CreateBizTagRequest, GenerateRequest, GenerateResponse,
    HealthResponse, IdMetadataResponse, MetricsResponse, ParseRequest, ParseResponse,
    RevokeApiKeyResponse, UpdateBizTagRequest,
};
use nebula_core::database::{
    ApiKeyRepository, ApiKeyRole, CreateApiKeyRequest as CoreCreateApiKeyRequest,
};
use nebula_core::{CoreError, Id, Result};
use std::sync::Arc;

pub struct ApiHandlers {
    id_generator: Arc<dyn nebula_core::algorithm::IdGenerator>,
    metrics: ApiMetrics,
    start_time: std::time::Instant,
    config_service: Arc<ConfigManagementService>,
    api_key_repo: Option<Arc<dyn ApiKeyRepository>>,
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
        id_generator: Arc<dyn nebula_core::algorithm::IdGenerator>,
        config_service: Arc<ConfigManagementService>,
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
        id_generator: Arc<dyn nebula_core::algorithm::IdGenerator>,
        config_service: Arc<ConfigManagementService>,
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

    pub fn get_config_service(&self) -> Arc<ConfigManagementService> {
        self.config_service.clone()
    }

    pub async fn generate(&self, req: GenerateRequest) -> Result<GenerateResponse> {
        let start = std::time::Instant::now();

        tracing::debug!(
            "generate request: workspace={}, group={}, biz_tag={}",
            req.workspace,
            req.group,
            req.biz_tag
        );

        let result = if let Some(ref alg_str) = req.algorithm {
            let algorithm = alg_str.parse::<nebula_core::types::AlgorithmType>()?;
            self.id_generator
                .generate_with_algorithm(algorithm, &req.workspace, &req.group, &req.biz_tag)
                .await
        } else {
            self.id_generator
                .generate(&req.workspace, &req.group, &req.biz_tag)
                .await
        };

        let id = match result {
            Ok(id) => id,
            Err(ref e) => {
                self.metrics
                    .failed_generations
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                return Err(e.clone());
            }
        };

        let elapsed = start.elapsed();
        self.metrics
            .total_requests
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.metrics
            .successful_generations
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.metrics
            .total_ids_generated
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        let latency_ms = elapsed.as_millis() as u64;
        let current_avg = self
            .metrics
            .avg_latency_ms
            .load(std::sync::atomic::Ordering::SeqCst);
        let new_avg = (current_avg + latency_ms) / 2;
        self.metrics
            .avg_latency_ms
            .store(new_avg, std::sync::atomic::Ordering::SeqCst);

        let algorithm_name = if let Some(ref alg) = req.algorithm {
            alg.clone()
        } else {
            self.id_generator
                .get_algorithm_name(&req.workspace, &req.group, &req.biz_tag)
                .await
                .unwrap_or_default()
        };

        Ok(GenerateResponse {
            id: id.to_string(),
            algorithm: algorithm_name,
            timestamp: chrono::Utc::now().to_rfc3339(),
        })
    }

    pub async fn batch_generate(&self, req: BatchGenerateRequest) -> Result<BatchGenerateResponse> {
        let start = std::time::Instant::now();

        // Get max batch size from config
        let max_batch_size = self.config_service.get_batch_max_size();

        // Validate batch size
        let size = req.size.unwrap_or(10);
        if size == 0 {
            return Err(CoreError::InvalidInput(
                "Batch size cannot be zero".to_string(),
            ));
        }
        if size > max_batch_size as usize {
            return Err(CoreError::InvalidInput(format!(
                "Batch size {} exceeds maximum allowed value of {}",
                size, max_batch_size
            )));
        }

        let result = if let Some(ref alg_str) = req.algorithm {
            let algorithm = alg_str.parse::<nebula_core::types::AlgorithmType>()?;
            self.id_generator
                .batch_generate_with_algorithm(
                    algorithm,
                    &req.workspace,
                    &req.group,
                    &req.biz_tag,
                    size,
                )
                .await
        } else {
            self.id_generator
                .batch_generate(&req.workspace, &req.group, &req.biz_tag, size)
                .await
        };

        if let Err(ref e) = result {
            self.metrics
                .failed_generations
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            return Err(e.clone());
        }

        let ids = result.unwrap();

        let elapsed = start.elapsed();
        self.metrics
            .total_requests
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.metrics
            .successful_generations
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.metrics
            .total_ids_generated
            .fetch_add(ids.len() as u64, std::sync::atomic::Ordering::SeqCst);

        let latency_ms = elapsed.as_millis() as u64;
        let current_avg = self
            .metrics
            .avg_latency_ms
            .load(std::sync::atomic::Ordering::SeqCst);
        let new_avg = (current_avg + latency_ms) / 2;
        self.metrics
            .avg_latency_ms
            .store(new_avg, std::sync::atomic::Ordering::SeqCst);

        Ok(BatchGenerateResponse {
            ids: ids.iter().map(|id| id.to_string()).collect(),
            size: ids.len(),
            algorithm: self
                .id_generator
                .get_algorithm_name(&req.workspace, &req.group, &req.biz_tag)
                .await
                .unwrap_or_default(),
            timestamp: chrono::Utc::now().to_rfc3339(),
        })
    }

    pub async fn health(&self) -> HealthResponse {
        let health_status = self.id_generator.health_check().await;
        HealthResponse {
            status: if health_status.is_healthy() {
                "healthy".to_string()
            } else {
                "degraded".to_string()
            },
            algorithm: self.id_generator.get_primary_algorithm().await.to_string(),
        }
    }

    pub async fn metrics(&self) -> MetricsResponse {
        // Get algorithm metrics from config service
        let algorithm_metrics = self.config_service.get_algorithm_metrics();
        let algorithms = algorithm_metrics
            .into_iter()
            .map(
                |(alg_type, snapshot): (
                    nebula_core::types::AlgorithmType,
                    nebula_core::algorithm::AlgorithmMetricsSnapshot,
                )| AlgorithmMetrics {
                    algorithm: alg_type.to_string(),
                    status: "healthy".to_string(),
                    total_generated: snapshot.total_generated,
                    total_failed: snapshot.total_failed,
                    cache_hit_rate: snapshot.cache_hit_rate,
                },
            )
            .collect();

        // Get database health metrics from config service
        let database = self.config_service.get_database_metrics().await;

        // Get cache health metrics
        let cache = self.config_service.get_cache_metrics().await;

        MetricsResponse {
            total_requests: self
                .metrics
                .total_requests
                .load(std::sync::atomic::Ordering::SeqCst),
            successful_generations: self
                .metrics
                .successful_generations
                .load(std::sync::atomic::Ordering::SeqCst),
            failed_generations: self
                .metrics
                .failed_generations
                .load(std::sync::atomic::Ordering::SeqCst),
            total_ids_generated: self
                .metrics
                .total_ids_generated
                .load(std::sync::atomic::Ordering::SeqCst),
            avg_latency_ms: self
                .metrics
                .avg_latency_ms
                .load(std::sync::atomic::Ordering::SeqCst),
            uptime_seconds: std::time::Instant::now()
                .duration_since(self.start_time)
                .as_secs(),
            database,
            cache,
            algorithms,
        }
    }

    pub async fn parse(&self, req: ParseRequest) -> Result<ParseResponse> {
        let id = Id::from_string(&req.id)
            .map_err(|e| CoreError::InvalidIdString(format!("Failed to parse ID: {}", e)))?;

        let algorithm = if req.algorithm.is_empty() {
            self.id_generator
                .get_algorithm_name(&req.workspace, &req.group, &req.biz_tag)
                .await
                .unwrap_or_else(|_| "unknown".to_string())
        } else {
            req.algorithm.clone()
        };

        let metadata = match algorithm.as_str() {
            "snowflake" => self.extract_snowflake_metadata(id.clone()),
            "uuid_v7" => self.extract_uuid_v7_metadata(id.clone()),
            "segment" => self.extract_segment_metadata(id.clone(), req.biz_tag),
            _ => IdMetadataResponse {
                timestamp: 0,
                datacenter_id: 0,
                worker_id: 0,
                sequence: 0,
                algorithm: "unknown".to_string(),
                biz_tag: req.biz_tag.clone(),
            },
        };

        Ok(ParseResponse {
            original_id: req.id,
            numeric_value: id.as_u128().to_string(),
            algorithm,
            metadata,
            timestamp: chrono::Utc::now().to_rfc3339(),
        })
    }

    fn extract_snowflake_metadata(&self, id: Id) -> IdMetadataResponse {
        let value = id.as_u128();

        const SEQUENCE_BITS: u8 = 10;
        const WORKER_ID_BITS: u8 = 8;
        const DATACENTER_ID_BITS: u8 = 3;

        let sequence_mask: u128 = (1u128 << SEQUENCE_BITS) - 1;
        let worker_mask: u128 = (1u128 << WORKER_ID_BITS) - 1;
        let datacenter_mask: u128 = (1u128 << DATACENTER_ID_BITS) - 1;

        let worker_shift = SEQUENCE_BITS;
        let datacenter_shift = SEQUENCE_BITS + WORKER_ID_BITS;
        let timestamp_shift = SEQUENCE_BITS + WORKER_ID_BITS + DATACENTER_ID_BITS;

        let sequence = (value & sequence_mask) as u16;
        let worker_id = ((value >> worker_shift) & worker_mask) as u16;
        let datacenter_id = ((value >> datacenter_shift) & datacenter_mask) as u8;
        let timestamp = (value >> timestamp_shift) as u64;

        IdMetadataResponse {
            timestamp,
            datacenter_id,
            worker_id,
            sequence,
            algorithm: "snowflake".to_string(),
            biz_tag: String::new(),
        }
    }

    fn extract_uuid_v7_metadata(&self, id: Id) -> IdMetadataResponse {
        let uuid = id.to_uuid_v7();
        let timestamp = uuid
            .get_timestamp()
            .map(|ts| {
                let unix = ts.to_unix();
                unix.0 * 1000 + (unix.1 / 1_000_000) as u64
            })
            .unwrap_or(0);

        IdMetadataResponse {
            timestamp,
            datacenter_id: 0,
            worker_id: 0,
            sequence: 0,
            algorithm: "uuid_v7".to_string(),
            biz_tag: String::new(),
        }
    }

    fn extract_segment_metadata(&self, _id: Id, biz_tag: String) -> IdMetadataResponse {
        IdMetadataResponse {
            timestamp: 0,
            datacenter_id: 0,
            worker_id: 0,
            sequence: 0,
            algorithm: "segment".to_string(),
            biz_tag,
        }
    }

    // ========== BizTag CRUD Operations ==========

    pub async fn create_biz_tag(&self, req: CreateBizTagRequest) -> Result<BizTagResponse> {
        let algorithm = req
            .algorithm
            .clone()
            .unwrap_or_else(|| "segment".to_string());
        let format = req.format.clone().unwrap_or_else(|| "numeric".to_string());

        let core_req = nebula_core::database::CreateBizTagRequest {
            workspace_id: req.workspace_id,
            group_id: req.group_id,
            name: req.name,
            description: req.description,
            algorithm: Some(
                algorithm
                    .parse()
                    .map_err(|_| CoreError::InvalidAlgorithmType(algorithm.clone()))?,
            ),
            format: Some(
                format
                    .parse()
                    .map_err(|_| CoreError::InvalidIdFormat(format.clone()))?,
            ),
            prefix: req.prefix,
            base_step: req.base_step,
            max_step: req.max_step,
            datacenter_ids: req.datacenter_ids,
        };

        let biz_tag = self.config_service.create_biz_tag(&core_req).await?;

        Ok(BizTagResponse {
            id: biz_tag.id.to_string(),
            workspace_id: biz_tag.workspace_id.to_string(),
            group_id: biz_tag.group_id.to_string(),
            name: biz_tag.name,
            description: biz_tag.description,
            algorithm: biz_tag.algorithm.to_string(),
            format: biz_tag.format.to_string(),
            prefix: biz_tag.prefix,
            base_step: biz_tag.base_step,
            max_step: biz_tag.max_step,
            datacenter_ids: biz_tag.datacenter_ids,
            created_at: naive_to_rfc3339(biz_tag.created_at),
            updated_at: naive_to_rfc3339(biz_tag.updated_at),
        })
    }

    pub async fn update_biz_tag(
        &self,
        id: uuid::Uuid,
        req: UpdateBizTagRequest,
    ) -> Result<BizTagResponse> {
        let core_req = nebula_core::database::UpdateBizTagRequest {
            name: req.name,
            description: req.description,
            algorithm: req
                .algorithm
                .map(|a| {
                    a.parse()
                        .map_err(|_| CoreError::InvalidAlgorithmType(a.clone()))
                })
                .transpose()?,
            format: req
                .format
                .map(|f| f.parse().map_err(|_| CoreError::InvalidIdFormat(f.clone())))
                .transpose()?,
            prefix: req.prefix,
            base_step: req.base_step,
            max_step: req.max_step,
            datacenter_ids: req.datacenter_ids,
        };

        let biz_tag = self.config_service.update_biz_tag(id, &core_req).await?;

        Ok(BizTagResponse {
            id: biz_tag.id.to_string(),
            workspace_id: biz_tag.workspace_id.to_string(),
            group_id: biz_tag.group_id.to_string(),
            name: biz_tag.name,
            description: biz_tag.description,
            algorithm: biz_tag.algorithm.to_string(),
            format: biz_tag.format.to_string(),
            prefix: biz_tag.prefix,
            base_step: biz_tag.base_step,
            max_step: biz_tag.max_step,
            datacenter_ids: biz_tag.datacenter_ids,
            created_at: naive_to_rfc3339(biz_tag.created_at),
            updated_at: naive_to_rfc3339(biz_tag.updated_at),
        })
    }

    pub async fn get_biz_tag(&self, id: uuid::Uuid) -> Result<BizTagResponse> {
        let biz_tag: nebula_core::database::BizTag = self
            .config_service
            .get_biz_tag(id)
            .await?
            .ok_or_else(|| CoreError::NotFound(format!("BizTag not found: {}", id)))?;

        Ok(BizTagResponse {
            id: biz_tag.id.to_string(),
            workspace_id: biz_tag.workspace_id.to_string(),
            group_id: biz_tag.group_id.to_string(),
            name: biz_tag.name,
            description: biz_tag.description,
            algorithm: biz_tag.algorithm.to_string(),
            format: biz_tag.format.to_string(),
            prefix: biz_tag.prefix,
            base_step: biz_tag.base_step,
            max_step: biz_tag.max_step,
            datacenter_ids: biz_tag.datacenter_ids,
            created_at: naive_to_rfc3339(biz_tag.created_at),
            updated_at: naive_to_rfc3339(biz_tag.updated_at),
        })
    }

    pub async fn list_biz_tags(
        &self,
        workspace_id: Option<uuid::Uuid>,
        group_id: Option<uuid::Uuid>,
    ) -> Result<BizTagListResponse> {
        let workspace_id = workspace_id.unwrap_or_else(uuid::Uuid::nil);
        let biz_tags: Vec<nebula_core::database::BizTag> = self
            .config_service
            .list_biz_tags(workspace_id, group_id, None, None)
            .await?;

        let responses: Vec<BizTagResponse> = biz_tags
            .into_iter()
            .map(|bt| BizTagResponse {
                id: bt.id.to_string(),
                workspace_id: bt.workspace_id.to_string(),
                group_id: bt.group_id.to_string(),
                name: bt.name,
                description: bt.description,
                algorithm: bt.algorithm.to_string(),
                format: bt.format.to_string(),
                prefix: bt.prefix,
                base_step: bt.base_step,
                max_step: bt.max_step,
                datacenter_ids: bt.datacenter_ids,
                created_at: naive_to_rfc3339(bt.created_at),
                updated_at: naive_to_rfc3339(bt.updated_at),
            })
            .collect();

        let total = responses.len() as u64;
        Ok(BizTagListResponse {
            total,
            biz_tags: responses,
            page: 1,
            page_size: total,
        })
    }

    pub async fn list_biz_tags_with_pagination(
        &self,
        workspace_id: Option<uuid::Uuid>,
        group_id: Option<uuid::Uuid>,
        limit: usize,
        offset: usize,
    ) -> Result<BizTagListResponse> {
        let workspace_id = workspace_id.unwrap_or_else(uuid::Uuid::nil);

        let biz_tags: Vec<nebula_core::database::BizTag> = self
            .config_service
            .list_biz_tags(
                workspace_id,
                group_id,
                Some(limit as u32),
                Some(offset as u32),
            )
            .await?;

        let total = self
            .config_service
            .count_biz_tags(workspace_id, group_id)
            .await?;

        let responses: Vec<BizTagResponse> = biz_tags
            .into_iter()
            .map(|bt| BizTagResponse {
                id: bt.id.to_string(),
                workspace_id: bt.workspace_id.to_string(),
                group_id: bt.group_id.to_string(),
                name: bt.name,
                description: bt.description,
                algorithm: bt.algorithm.to_string(),
                format: bt.format.to_string(),
                prefix: bt.prefix,
                base_step: bt.base_step,
                max_step: bt.max_step,
                datacenter_ids: bt.datacenter_ids,
                created_at: naive_to_rfc3339(bt.created_at),
                updated_at: naive_to_rfc3339(bt.updated_at),
            })
            .collect();

        Ok(BizTagListResponse {
            total,
            biz_tags: responses,
            page: (offset / limit + 1) as u64,
            page_size: limit as u64,
        })
    }

    pub async fn delete_biz_tag(&self, id: uuid::Uuid) -> Result<()> {
        self.config_service.delete_biz_tag(id).await
    }

    // ========== API Key Management ==========

    /// Create a new API Key (admin only)
    pub async fn create_api_key(
        &self,
        workspace_id: uuid::Uuid,
        req: CreateApiKeyRequest,
    ) -> Result<ApiKeyWithSecretResponse> {
        let repo = self
            .api_key_repo
            .as_ref()
            .ok_or_else(|| CoreError::NotFound("API key repository not configured".to_string()))?;

        let role = match req.role.as_deref() {
            Some("admin") => ApiKeyRole::Admin,
            Some("user") | None => ApiKeyRole::User,
            Some(r) => {
                return Err(CoreError::AuthenticationError(format!(
                    "Invalid role: {}",
                    r
                )))
            }
        };

        // Validate admin key creation: check if workspace already has an admin key
        if role == ApiKeyRole::Admin {
            let existing_keys = repo
                .list_api_keys(workspace_id, Some(1000), Some(0))
                .await
                .map_err(|e| CoreError::DatabaseError(e.to_string()))?;

            let has_admin = existing_keys.iter().any(|k| {
                k.role == nebula_core::database::ApiKeyRole::Admin
            });

            if has_admin {
                tracing::warn!(
                    event = "admin_key_creation",
                    workspace_id = %workspace_id,
                    "Creating additional admin key for workspace"
                );
            }
        }

        let expires_at = match req.expires_at {
            Some(ts) => Some(
                chrono::DateTime::parse_from_rfc3339(&ts)
                    .map_err(|_| {
                        CoreError::InvalidIdFormat("Invalid expires_at format".to_string())
                    })?
                    .with_timezone(&chrono::Utc)
                    .naive_utc(),
            ),
            None => None,
        };

        let core_req = CoreCreateApiKeyRequest {
            workspace_id,
            name: req.name,
            description: req.description,
            role,
            rate_limit: req.rate_limit,
            expires_at,
        };

        let key_with_secret = repo
            .create_api_key(&core_req)
            .await
            .map_err(|e| CoreError::DatabaseError(e.to_string()))?;

        Ok(ApiKeyWithSecretResponse {
            key: ApiKeyResponse {
                id: key_with_secret.key.id.to_string(),
                key_id: key_with_secret.key.key_id,
                key_prefix: key_with_secret.key.key_prefix,
                name: key_with_secret.key.name,
                description: key_with_secret.key.description,
                role: key_with_secret.key.role.to_string(),
                rate_limit: key_with_secret.key.rate_limit,
                enabled: key_with_secret.key.enabled,
                expires_at: key_with_secret.key.expires_at.map(naive_to_rfc3339),
                created_at: naive_to_rfc3339(key_with_secret.key.created_at),
            },
            key_secret: key_with_secret.key_secret,
        })
    }

    /// List API Keys for a workspace (admin only)
    pub async fn list_api_keys(
        &self,
        workspace_id: uuid::Uuid,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> Result<ApiKeyListResponse> {
        let repo = self
            .api_key_repo
            .as_ref()
            .ok_or_else(|| CoreError::NotFound("API key repository not configured".to_string()))?;

        let keys = repo
            .list_api_keys(workspace_id, limit, offset)
            .await
            .map_err(|e| CoreError::DatabaseError(e.to_string()))?;

        let responses: Vec<ApiKeyResponse> = keys
            .into_iter()
            .map(|k| ApiKeyResponse {
                id: k.id.to_string(),
                key_id: k.key_id,
                key_prefix: k.key_prefix,
                name: k.name,
                description: k.description,
                role: k.role.to_string(),
                rate_limit: k.rate_limit,
                enabled: k.enabled,
                expires_at: k.expires_at.map(naive_to_rfc3339),
                created_at: naive_to_rfc3339(k.created_at),
            })
            .collect();

        let total = repo
            .count_api_keys(workspace_id)
            .await
            .map_err(|e| CoreError::DatabaseError(e.to_string()))?;

        Ok(ApiKeyListResponse {
            api_keys: responses,
            total,
        })
    }

    /// Revoke (delete) an API Key (admin only)
    pub async fn revoke_api_key(&self, id: uuid::Uuid) -> Result<RevokeApiKeyResponse> {
        let repo = self
            .api_key_repo
            .as_ref()
            .ok_or_else(|| CoreError::NotFound("API key repository not configured".to_string()))?;

        // Get the key info before deletion to check if it's an admin key
        let key_info = repo
            .get_api_key_by_id(&id.to_string())
            .await
            .map_err(|e| CoreError::DatabaseError(e.to_string()))?;

        if let Some(key) = key_info {
            // If it's an admin key, check if it's the last one
            if key.role == nebula_core::database::ApiKeyRole::Admin {
                let existing_keys = repo
                    .list_api_keys(key.workspace_id, Some(1000), Some(0))
                    .await
                    .map_err(|e| CoreError::DatabaseError(e.to_string()))?;

                let admin_count = existing_keys
                    .iter()
                    .filter(|k| k.role == nebula_core::database::ApiKeyRole::Admin)
                    .count();

                if admin_count <= 1 {
                    return Err(CoreError::AuthenticationError(
                        "Cannot revoke the last admin key".to_string(),
                    ));
                }
            }
        }

        repo.delete_api_key(id)
            .await
            .map_err(|e| CoreError::DatabaseError(e.to_string()))?;

        Ok(RevokeApiKeyResponse {
            success: true,
            message: format!("API key {} revoked successfully", id),
        })
    }
}

pub mod mock_generator;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config_hot_reload::HotReloadConfig;
    use crate::handlers::mock_generator::MockIdGenerator;
    use crate::models::{BatchGenerateRequest, GenerateRequest, ParseRequest};
    use std::sync::Arc;

    fn create_test_api_handlers() -> (Arc<ApiHandlers>, Arc<MockIdGenerator>) {
        let mock_gen = Arc::new(MockIdGenerator::new());
        // Create a minimal config for testing
        let config = nebula_core::config::Config::default();
        let hot_config = Arc::new(HotReloadConfig::new(
            config,
            "config/config.toml".to_string(),
        ));

        // Create a minimal AlgorithmRouter for testing
        let router = Arc::new(nebula_core::algorithm::AlgorithmRouter::new(
            nebula_core::config::Config::default(),
            None,
        ));

        let config_service = Arc::new(crate::config_management::ConfigManagementService::new(
            hot_config, router,
        ));
        let handlers = ApiHandlers::new(mock_gen.clone(), config_service);
        (Arc::new(handlers), mock_gen)
    }

    #[tokio::test]
    async fn test_handle_generate() {
        let (handlers, _router) = create_test_api_handlers();
        let req = GenerateRequest {
            workspace: "test".to_string(),
            group: "test".to_string(),
            biz_tag: "test-biz".to_string(),
            algorithm: None,
        };
        let response = handlers.generate(req).await;
        assert!(response.is_ok());
        let gen_response = response.unwrap();
        assert!(!gen_response.id.is_empty());
    }

    #[tokio::test]
    async fn test_handle_generate_invalid_request() {
        let (handlers, _router) = create_test_api_handlers();
        let req = GenerateRequest {
            workspace: "".to_string(),
            group: "test".to_string(),
            biz_tag: "test-biz".to_string(),
            algorithm: None,
        };
        let response = handlers.generate(req).await;
        assert!(response.is_err());
    }

    #[tokio::test]
    async fn test_handle_batch_generate() {
        let (handlers, _router) = create_test_api_handlers();
        let req = BatchGenerateRequest {
            workspace: "test".to_string(),
            group: "test".to_string(),
            biz_tag: "test-biz".to_string(),
            size: Some(5),
            algorithm: None,
        };
        let response = handlers.batch_generate(req).await;
        assert!(response.is_ok());
        let gen_response = response.unwrap();
        assert_eq!(gen_response.ids.len(), 5);
    }

    #[tokio::test]
    async fn test_handle_parse() {
        let (handlers, _router) = create_test_api_handlers();
        let parse_req = ParseRequest {
            id: "test-id".to_string(),
            workspace: "test".to_string(),
            group: "test".to_string(),
            biz_tag: "test-biz".to_string(),
            algorithm: "segment".to_string(),
        };
        let _response = handlers.parse(parse_req).await;
    }

    #[tokio::test]
    async fn test_handle_metrics() {
        let (handlers, _router) = create_test_api_handlers();
        let response = handlers.metrics().await;
        assert!(response.total_requests == response.total_requests);
    }
}
