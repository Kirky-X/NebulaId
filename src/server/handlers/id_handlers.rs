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

//! ID generation / parsing handlers (rule 25: impl split into sub-module).

use crate::core::{CoreError, Id, Result};
use crate::server::models::{
    BatchGenerateRequest, BatchGenerateResponse, GenerateRequest, GenerateResponse,
    IdMetadataResponse, ParseRequest, ParseResponse,
};
use std::sync::atomic::Ordering;

impl super::ApiHandlers {
    pub async fn generate(&self, req: GenerateRequest) -> Result<GenerateResponse> {
        let start = std::time::Instant::now();

        tracing::debug!(
            "generate request: workspace={}, group={}, biz_tag={}",
            req.workspace,
            req.group,
            req.biz_tag
        );

        let result = if let Some(ref alg_str) = req.algorithm {
            let algorithm: crate::core::types::AlgorithmType = alg_str.parse()?;
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
                    .fetch_add(1, Ordering::SeqCst);
                return Err(e.clone());
            }
        };

        let elapsed = start.elapsed();
        self.metrics.total_requests.fetch_add(1, Ordering::SeqCst);
        self.metrics
            .successful_generations
            .fetch_add(1, Ordering::SeqCst);
        self.metrics
            .total_ids_generated
            .fetch_add(1, Ordering::SeqCst);

        let latency_ms = elapsed.as_millis() as u64;
        let current_avg = self.metrics.avg_latency_ms.load(Ordering::SeqCst);
        let new_avg = (current_avg + latency_ms) / 2;
        self.metrics.avg_latency_ms.store(new_avg, Ordering::SeqCst);

        let algorithm_name = if let Some(ref alg) = req.algorithm {
            alg.parse::<crate::core::types::AlgorithmType>()
                .map(|a| a.to_string())
                .unwrap_or_else(|_| "segment".to_string())
        } else {
            self.id_generator
                .get_algorithm_name(&req.workspace, &req.group, &req.biz_tag)
                .await
                .unwrap_or_else(|_| "segment".to_string())
        };

        Ok(GenerateResponse {
            id: id.to_string(),
            algorithm: algorithm_name,
            timestamp: chrono::Utc::now().to_rfc3339(),
        })
    }

    pub async fn batch_generate(&self, req: BatchGenerateRequest) -> Result<BatchGenerateResponse> {
        let start = std::time::Instant::now();

        let max_batch_size = self.config_service.get_batch_max_size();

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
            let algorithm: crate::core::types::AlgorithmType = alg_str.parse()?;
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

        let ids = match result {
            Ok(ids) => ids,
            Err(ref e) => {
                self.metrics
                    .failed_generations
                    .fetch_add(1, Ordering::SeqCst);
                return Err(e.clone());
            }
        };

        let elapsed = start.elapsed();
        self.metrics.total_requests.fetch_add(1, Ordering::SeqCst);
        self.metrics
            .successful_generations
            .fetch_add(1, Ordering::SeqCst);
        self.metrics
            .total_ids_generated
            .fetch_add(ids.len() as u64, Ordering::SeqCst);

        let latency_ms = elapsed.as_millis() as u64;
        let current_avg = self.metrics.avg_latency_ms.load(Ordering::SeqCst);
        let new_avg = (current_avg + latency_ms) / 2;
        self.metrics.avg_latency_ms.store(new_avg, Ordering::SeqCst);

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
}

#[cfg(test)]
mod tests {
    use crate::server::config::management::{ConfigManagementService, ConfigManager};
    use crate::server::config::HotReloadConfig;
    use crate::server::handlers::mock_generator::MockIdGenerator;
    use crate::server::models::{BatchGenerateRequest, GenerateRequest, ParseRequest};
    use std::sync::Arc;

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
}
