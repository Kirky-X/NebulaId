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
    pub fn new(id_generator: Arc<dyn nebula_core::algorithm::IdGenerator>) -> Self {
        Self {
            id_generator,
            metrics: Arc::new(RwLock::new(ApiMetrics::default())),
            start_time: std::time::Instant::now(),
        }
    }

    pub async fn generate(&self, req: GenerateRequest) -> Result<GenerateResponse> {
        let start = std::time::Instant::now();

        let result = self
            .id_generator
            .generate(&req.workspace, &req.group, &req.biz_tag)
            .await;

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

        Ok(GenerateResponse {
            id: id.to_string(),
            algorithm: self
                .id_generator
                .get_algorithm_name(&req.workspace, &req.group, &req.biz_tag)
                .await
                .unwrap_or_default(),
            timestamp: chrono::Utc::now().to_rfc3339(),
        })
    }

    pub async fn batch_generate(&self, req: BatchGenerateRequest) -> Result<BatchGenerateResponse> {
        let start = std::time::Instant::now();

        let result = self
            .id_generator
            .batch_generate(
                &req.workspace,
                &req.group,
                &req.biz_tag,
                req.size.unwrap_or(10),
            )
            .await;

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

        let algorithm = self
            .id_generator
            .get_algorithm_name(&req.workspace, &req.group, &req.biz_tag)
            .await
            .unwrap_or_else(|_| "unknown".to_string());

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
        let sequence = (value & 0xFFF) as u16;
        let worker_id = ((value >> 12) & 0x3FF) as u16;
        let datacenter_id = ((value >> 22) & 0x1F) as u8;
        let timestamp = (value >> 27) as u64;

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
                unix.0 as u64 * 1000 + (unix.1 / 1_000_000) as u64
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
    use super::*;
    use nebula_core::algorithm::AlgorithmRouter;
    use nebula_core::config::Config;

    #[tokio::test]
    async fn test_generate_response() {
        let config = Config::default();
        let mut router = AlgorithmRouter::new(config);
        router.initialize().await.unwrap();
        let handlers = ApiHandlers::new(Arc::new(router));

        let req = GenerateRequest {
            workspace: "test-workspace".to_string(),
            group: "test-group".to_string(),
            biz_tag: "test-tag".to_string(),
        };

        let response = handlers.generate(req).await.unwrap();
        assert!(!response.id.is_empty());
        assert!(!response.algorithm.is_empty());
    }
}
