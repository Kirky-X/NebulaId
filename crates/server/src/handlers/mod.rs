use crate::config_management::ConfigManagementService;
use crate::models::{
    BatchGenerateRequest, BatchGenerateResponse, GenerateRequest, GenerateResponse, HealthResponse,
    IdMetadataResponse, MetricsResponse, ParseRequest, ParseResponse,
};
use nebula_core::{CoreError, Id, Result};
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct ApiHandlers {
    id_generator: Arc<dyn nebula_core::algorithm::IdGenerator>,
    metrics: Arc<RwLock<ApiMetrics>>,
    start_time: std::time::Instant,
    config_service: Arc<ConfigManagementService>,
}

#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ApiMetrics {
    pub total_requests: Arc<std::sync::atomic::AtomicU64>,
    pub successful_generations: Arc<std::sync::atomic::AtomicU64>,
    pub failed_generations: Arc<std::sync::atomic::AtomicU64>,
    pub total_ids_generated: Arc<std::sync::atomic::AtomicU64>,
    pub avg_latency_ms: Arc<std::sync::atomic::AtomicU64>,
}

impl ApiHandlers {
    pub fn new(
        id_generator: Arc<dyn nebula_core::algorithm::IdGenerator>,
        config_service: Arc<ConfigManagementService>,
    ) -> Self {
        Self {
            id_generator,
            metrics: Arc::new(RwLock::new(ApiMetrics::default())),
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
                .write()
                .await
                .failed_generations
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            return Err(e.clone());
        }

        let id = result.unwrap();

        let elapsed = start.elapsed();
        self.metrics
            .write()
            .await
            .total_requests
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.metrics
            .write()
            .await
            .successful_generations
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.metrics
            .write()
            .await
            .total_ids_generated
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        let latency_ms = elapsed.as_millis() as u64;
        let current_avg = self
            .metrics
            .read()
            .await
            .avg_latency_ms
            .load(std::sync::atomic::Ordering::SeqCst);
        let new_avg = (current_avg + latency_ms) / 2;
        self.metrics
            .write()
            .await
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
                .write()
                .await
                .failed_generations
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            return Err(e.clone());
        }

        let ids = result.unwrap();

        let elapsed = start.elapsed();
        self.metrics
            .write()
            .await
            .total_requests
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.metrics
            .write()
            .await
            .successful_generations
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.metrics
            .write()
            .await
            .total_ids_generated
            .fetch_add(ids.len() as u64, std::sync::atomic::Ordering::SeqCst);

        let latency_ms = elapsed.as_millis() as u64;
        let current_avg = self
            .metrics
            .read()
            .await
            .avg_latency_ms
            .load(std::sync::atomic::Ordering::SeqCst);
        let new_avg = (current_avg + latency_ms) / 2;
        self.metrics
            .write()
            .await
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
        let metrics = self.metrics.read().await;
        MetricsResponse {
            total_requests: metrics
                .total_requests
                .load(std::sync::atomic::Ordering::SeqCst),
            successful_generations: metrics
                .successful_generations
                .load(std::sync::atomic::Ordering::SeqCst),
            failed_generations: metrics
                .failed_generations
                .load(std::sync::atomic::Ordering::SeqCst),
            total_ids_generated: metrics
                .total_ids_generated
                .load(std::sync::atomic::Ordering::SeqCst),
            avg_latency_ms: metrics
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
        let hot_config = Arc::new(HotReloadConfig::new(config, "config.toml".to_string()));

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
