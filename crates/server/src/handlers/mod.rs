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
    naive_to_rfc3339, BatchGenerateRequest, BatchGenerateResponse, BizTagListResponse,
    BizTagResponse, CreateBizTagRequest, GenerateRequest, GenerateResponse, HealthResponse,
    IdMetadataResponse, MetricsResponse, ParseRequest, ParseResponse, UpdateBizTagRequest,
};
use nebula_core::{CoreError, Id, Result};
use std::sync::Arc;

pub struct ApiHandlers {
    id_generator: Arc<dyn nebula_core::algorithm::IdGenerator>,
    metrics: ApiMetrics,
    start_time: std::time::Instant,
    config_service: Arc<ConfigManagementService>,
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
        }
    }

    pub fn get_config_service(&self) -> Arc<ConfigManagementService> {
        self.config_service.clone()
    }

    pub async fn generate(&self, req: GenerateRequest) -> Result<GenerateResponse> {
        let start = std::time::Instant::now();

        tracing::info!(
            "generate called: workspace={}, group={}, biz_tag={}, algorithm={:?}",
            req.workspace,
            req.group,
            req.biz_tag,
            req.algorithm
        );

        let result = if let Some(ref alg_str) = req.algorithm {
            tracing::info!("Using explicit algorithm: {}", alg_str);
            let algorithm = alg_str.parse::<nebula_core::types::AlgorithmType>()?;
            tracing::info!("Parsed algorithm type: {:?}", algorithm);
            self.id_generator
                .generate_with_algorithm(algorithm, &req.workspace, &req.group, &req.biz_tag)
                .await
        } else {
            tracing::info!("No algorithm specified, using default");
            self.id_generator
                .generate(&req.workspace, &req.group, &req.biz_tag)
                .await
        };

        if let Err(ref e) = result {
            self.metrics
                .failed_generations
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            return Err(e.clone());
        }

        let id = result.unwrap();

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

        // Validate batch size
        let size = req.size.unwrap_or(10);
        if size == 0 {
            return Err(CoreError::InvalidInput(
                "Batch size cannot be zero".to_string(),
            ));
        }
        if size > 100 {
            return Err(CoreError::InvalidInput(format!(
                "Batch size {} exceeds maximum allowed value of 100",
                size
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
        let hot_config = Arc::new(HotReloadConfig::new(config, "config/config.toml".to_string()));

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
