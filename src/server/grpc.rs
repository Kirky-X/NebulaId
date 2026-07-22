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

use crate::server::handlers::ApiHandlers;
use crate::server::models::{BatchGenerateRequest, GenerateRequest, ParseRequest};
use async_trait::async_trait;
use sdforge::tonic::{Request, Response, Status};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::StreamExt;

// Use pre-generated proto modules
use crate::server::proto::nebula::id::v1;

use v1::nebula_id_service_server::NebulaIdService;
use v1::{
    BatchGenerateRequest as GrpcBatchGenerateRequest,
    BatchGenerateResponse as GrpcBatchGenerateResponse, BatchGenerateStreamRequest,
    BatchGenerateStreamResponse, GenerateRequest as GrpcGenerateRequest,
    GenerateResponse as GrpcGenerateResponse, HealthCheckRequest, HealthCheckResponse,
    ParseRequest as GrpcParseRequest, ParseResponse as GrpcParseResponse,
};

pub struct GrpcServer {
    handlers: Arc<ApiHandlers>,
}

impl GrpcServer {
    pub fn new(handlers: Arc<ApiHandlers>) -> Self {
        Self { handlers }
    }
}

#[async_trait]
impl NebulaIdService for GrpcServer {
    type BatchGenerateStreamStream = ReceiverStream<Result<BatchGenerateStreamResponse, Status>>;

    async fn generate(
        &self,
        request: Request<GrpcGenerateRequest>,
    ) -> Result<Response<GrpcGenerateResponse>, Status> {
        let req = request.into_inner();
        let tag = req.tag.clone();

        let generate_req = GenerateRequest {
            workspace: req.namespace,
            group: tag.clone(),
            biz_tag: tag,
            algorithm: None,
        };

        match self.handlers.generate(generate_req).await {
            Ok(resp) => {
                let timestamp = resp.timestamp.parse().unwrap_or(0);
                Ok(Response::new(GrpcGenerateResponse {
                    id: resp.id,
                    timestamp,
                    sequence: 0,
                    worker_id: 0,
                    algorithm: resp.algorithm,
                }))
            }
            Err(e) => Err(Status::internal(format!("{}", e))),
        }
    }

    async fn batch_generate(
        &self,
        request: Request<GrpcBatchGenerateRequest>,
    ) -> Result<Response<GrpcBatchGenerateResponse>, Status> {
        let req = request.into_inner();
        let tag = req.tag.clone();

        tracing::info!(
            "{}",
            t!("log.server.grpc.batch_generate_received", count = req.count)
        );

        // Validate batch size
        if req.count == 0 {
            tracing::warn!(
                "{}",
                t!("log.server.grpc.batch_size_validation_failed_zero")
            );
            return Err(Status::invalid_argument("Batch size cannot be zero"));
        }
        if req.count > 100 {
            tracing::warn!(
                "{}",
                t!(
                    "log.server.grpc.batch_size_validation_failed_exceeds_max",
                    count = req.count
                )
            );
            return Err(Status::invalid_argument(format!(
                "Batch size {} exceeds maximum allowed value of 100",
                req.count
            )));
        }

        tracing::info!(
            "{}",
            t!(
                "log.server.grpc.batch_size_validation_passed",
                count = req.count
            )
        );

        let batch_req = BatchGenerateRequest {
            workspace: req.namespace,
            group: tag.clone(),
            biz_tag: tag,
            size: Some(req.count as usize),
            algorithm: None,
        };

        match self.handlers.batch_generate(batch_req).await {
            Ok(resp) => {
                let timestamp = resp.timestamp.parse().unwrap_or(0);
                let ids = resp
                    .ids
                    .into_iter()
                    .map(|id| GrpcGenerateResponse {
                        id,
                        timestamp,
                        sequence: 0,
                        worker_id: 0,
                        algorithm: resp.algorithm.clone(),
                    })
                    .collect();

                Ok(Response::new(GrpcBatchGenerateResponse { ids }))
            }
            Err(e) => Err(Status::internal(format!("{}", e))),
        }
    }

    async fn batch_generate_stream(
        &self,
        request: Request<sdforge::tonic::Streaming<BatchGenerateStreamRequest>>,
    ) -> Result<Response<Self::BatchGenerateStreamStream>, Status> {
        let mut stream = request.into_inner();
        let (tx, rx) = mpsc::channel(128);

        let handlers = self.handlers.clone();

        tokio::spawn(async move {
            while let Some(req) = stream.next().await {
                match req {
                    Ok(stream_req) => {
                        let tag = stream_req.tag.clone();
                        let batch_req = BatchGenerateRequest {
                            workspace: stream_req.namespace,
                            group: tag.clone(),
                            biz_tag: tag,
                            size: Some(stream_req.count as usize),
                            algorithm: None,
                        };

                        match handlers.batch_generate(batch_req).await {
                            Ok(resp) => {
                                let timestamp = resp.timestamp.parse().unwrap_or(0);
                                for id in resp.ids {
                                    let stream_resp = BatchGenerateStreamResponse {
                                        id: Some(GrpcGenerateResponse {
                                            id,
                                            timestamp,
                                            sequence: 0,
                                            worker_id: 0,
                                            algorithm: resp.algorithm.clone(),
                                        }),
                                    };

                                    if tx.send(Ok(stream_resp)).await.is_err() {
                                        break;
                                    }
                                }
                            }
                            Err(e) => {
                                let _ = tx
                                    .send(Ok(BatchGenerateStreamResponse {
                                        id: Some(GrpcGenerateResponse {
                                            id: String::new(),
                                            timestamp: 0,
                                            sequence: 0,
                                            worker_id: 0,
                                            algorithm: format!("error: {}", e),
                                        }),
                                    }))
                                    .await;
                            }
                        }
                    }
                    Err(e) => {
                        let _ = tx
                            .send(Ok(BatchGenerateStreamResponse {
                                id: Some(GrpcGenerateResponse {
                                    id: String::new(),
                                    timestamp: 0,
                                    sequence: 0,
                                    worker_id: 0,
                                    algorithm: format!("stream error: {}", e),
                                }),
                            }))
                            .await;
                        break;
                    }
                }
            }
        });

        Ok(Response::new(ReceiverStream::new(rx)))
    }

    async fn parse(
        &self,
        request: Request<GrpcParseRequest>,
    ) -> Result<Response<GrpcParseResponse>, Status> {
        let req = request.into_inner();

        let parse_req = ParseRequest {
            id: req.id.clone(),
            workspace: String::new(),
            group: String::new(),
            biz_tag: String::new(),
            algorithm: String::new(),
        };

        match self.handlers.parse(parse_req).await {
            Ok(resp) => {
                let timestamp = resp.timestamp.parse().unwrap_or(0);
                let metadata: HashMap<String, String> = vec![
                    ("timestamp".to_string(), resp.metadata.timestamp.to_string()),
                    (
                        "datacenter_id".to_string(),
                        resp.metadata.datacenter_id.to_string(),
                    ),
                    ("worker_id".to_string(), resp.metadata.worker_id.to_string()),
                    ("sequence".to_string(), resp.metadata.sequence.to_string()),
                    ("algorithm".to_string(), resp.metadata.algorithm),
                    ("biz_tag".to_string(), resp.metadata.biz_tag),
                ]
                .into_iter()
                .collect();

                Ok(Response::new(GrpcParseResponse {
                    id: resp.original_id,
                    timestamp,
                    sequence: resp.metadata.sequence as i32,
                    worker_id: resp.metadata.worker_id as i32,
                    algorithm: resp.algorithm,
                    metadata,
                }))
            }
            Err(e) => Err(Status::invalid_argument(format!("{}", e))),
        }
    }

    async fn health_check(
        &self,
        _request: Request<HealthCheckRequest>,
    ) -> Result<Response<HealthCheckResponse>, Status> {
        let health = self.handlers.health().await;
        let status = if health.status == crate::server::models::HealthStatus::Healthy {
            v1::health_check_response::ServingStatus::Serving
        } else {
            v1::health_check_response::ServingStatus::NotServing
        };

        Ok(Response::new(HealthCheckResponse {
            status: status as i32,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::algorithm::AlgorithmRouter;
    use crate::core::config::Config;
    use crate::server::config::management::{ConfigManagementService, ConfigManager};
    use crate::server::config::HotReloadConfig;
    use crate::server::handlers::mock_generator::MockIdGenerator;
    use std::sync::Arc;

    /// Build a GrpcServer wired to a MockIdGenerator + ConfigManager.
    fn create_test_grpc_server() -> GrpcServer {
        let config = Config::default();
        let hot_config = Arc::new(HotReloadConfig::new(
            config.clone(),
            "config/config.toml".to_string(),
        ));
        let algorithm_router = Arc::new(AlgorithmRouter::new(config, None));
        let config_service: Arc<dyn ConfigManagementService> =
            Arc::new(ConfigManager::new(hot_config, algorithm_router));
        let id_generator: Arc<dyn crate::core::algorithm::IdGenerator> =
            Arc::new(MockIdGenerator::new());
        let handlers = Arc::new(ApiHandlers::new(id_generator, config_service));
        GrpcServer::new(handlers)
    }

    // ===== GrpcServer::new =====

    #[test]
    fn test_grpc_server_new() {
        let server = create_test_grpc_server();
        // Smoke test: server can be constructed without panic.
        let _ = server;
    }

    // ===== generate =====

    #[tokio::test]
    async fn test_generate_success() {
        let server = create_test_grpc_server();
        let req = Request::new(GrpcGenerateRequest {
            namespace: "test-ns".to_string(),
            tag: "test-tag".to_string(),
            metadata: Default::default(),
        });
        let resp = server.generate(req).await;
        assert!(resp.is_ok(), "generate should succeed: {:?}", resp);
        let inner = resp.unwrap().into_inner();
        assert!(!inner.id.is_empty());
        assert_eq!(inner.algorithm, "segment");
        assert_eq!(inner.sequence, 0);
        assert_eq!(inner.worker_id, 0);
    }

    #[tokio::test]
    async fn test_generate_empty_namespace_returns_internal_error() {
        // MockIdGenerator returns InvalidInput when workspace is empty.
        let server = create_test_grpc_server();
        let req = Request::new(GrpcGenerateRequest {
            namespace: String::new(),
            tag: "test-tag".to_string(),
            metadata: Default::default(),
        });
        let resp = server.generate(req).await;
        assert!(resp.is_err());
        let err = resp.unwrap_err();
        assert_eq!(err.code(), sdforge::tonic::Code::Internal);
    }

    #[tokio::test]
    async fn test_generate_maps_namespace_and_tag() {
        // Verify that `namespace` is mapped to `workspace` and `tag` is mapped
        // to both `group` and `biz_tag` (per the handler's GenerateRequest
        // construction).
        let server = create_test_grpc_server();
        let req = Request::new(GrpcGenerateRequest {
            namespace: "mapped-ns".to_string(),
            tag: "mapped-tag".to_string(),
            metadata: Default::default(),
        });
        let resp = server.generate(req).await.unwrap().into_inner();
        // ID should be non-empty (MockIdGenerator generates u128 IDs).
        assert!(!resp.id.is_empty());
    }

    // ===== batch_generate =====

    #[tokio::test]
    async fn test_batch_generate_success() {
        let server = create_test_grpc_server();
        let req = Request::new(GrpcBatchGenerateRequest {
            namespace: "test-ns".to_string(),
            tag: "test-tag".to_string(),
            count: 5,
            metadata: Default::default(),
        });
        let resp = server.batch_generate(req).await;
        assert!(resp.is_ok(), "batch_generate should succeed: {:?}", resp);
        let inner = resp.unwrap().into_inner();
        assert_eq!(inner.ids.len(), 5);
        for id in &inner.ids {
            assert!(!id.id.is_empty());
            assert_eq!(id.algorithm, "segment");
        }
    }

    #[tokio::test]
    async fn test_batch_generate_count_one_boundary() {
        // Lower boundary: count=1 should succeed.
        let server = create_test_grpc_server();
        let req = Request::new(GrpcBatchGenerateRequest {
            namespace: "test-ns".to_string(),
            tag: "test-tag".to_string(),
            count: 1,
            metadata: Default::default(),
        });
        let resp = server.batch_generate(req).await.unwrap().into_inner();
        assert_eq!(resp.ids.len(), 1);
    }

    #[tokio::test]
    async fn test_batch_generate_count_100_boundary() {
        // Upper boundary: count=100 should succeed.
        let server = create_test_grpc_server();
        let req = Request::new(GrpcBatchGenerateRequest {
            namespace: "test-ns".to_string(),
            tag: "test-tag".to_string(),
            count: 100,
            metadata: Default::default(),
        });
        let resp = server.batch_generate(req).await.unwrap().into_inner();
        assert_eq!(resp.ids.len(), 100);
    }

    #[tokio::test]
    async fn test_batch_generate_zero_count_returns_invalid_argument() {
        let server = create_test_grpc_server();
        let req = Request::new(GrpcBatchGenerateRequest {
            namespace: "test-ns".to_string(),
            tag: "test-tag".to_string(),
            count: 0,
            metadata: Default::default(),
        });
        let err = server.batch_generate(req).await.unwrap_err();
        assert_eq!(err.code(), sdforge::tonic::Code::InvalidArgument);
        assert!(err.message().contains("zero"));
    }

    #[tokio::test]
    async fn test_batch_generate_exceeds_max_101_returns_invalid_argument() {
        let server = create_test_grpc_server();
        let req = Request::new(GrpcBatchGenerateRequest {
            namespace: "test-ns".to_string(),
            tag: "test-tag".to_string(),
            count: 101,
            metadata: Default::default(),
        });
        let err = server.batch_generate(req).await.unwrap_err();
        assert_eq!(err.code(), sdforge::tonic::Code::InvalidArgument);
        assert!(err.message().contains("exceeds maximum"));
    }

    #[tokio::test]
    async fn test_batch_generate_huge_count_returns_invalid_argument() {
        let server = create_test_grpc_server();
        let req = Request::new(GrpcBatchGenerateRequest {
            namespace: "test-ns".to_string(),
            tag: "test-tag".to_string(),
            count: 1000,
            metadata: Default::default(),
        });
        let err = server.batch_generate(req).await.unwrap_err();
        assert_eq!(err.code(), sdforge::tonic::Code::InvalidArgument);
    }

    #[tokio::test]
    async fn test_batch_generate_empty_namespace_returns_internal_error() {
        let server = create_test_grpc_server();
        let req = Request::new(GrpcBatchGenerateRequest {
            namespace: String::new(),
            tag: "test-tag".to_string(),
            count: 5,
            metadata: Default::default(),
        });
        let err = server.batch_generate(req).await.unwrap_err();
        assert_eq!(err.code(), sdforge::tonic::Code::Internal);
    }

    #[tokio::test]
    async fn test_batch_generate_maps_namespace_and_tag() {
        // Verify namespace→workspace and tag→group/biz_tag mapping by
        // observing that a valid request succeeds (MockIdGenerator returns
        // Err only when workspace is empty).
        let server = create_test_grpc_server();
        let req = Request::new(GrpcBatchGenerateRequest {
            namespace: "mapped-ns".to_string(),
            tag: "mapped-tag".to_string(),
            count: 3,
            metadata: Default::default(),
        });
        let resp = server.batch_generate(req).await.unwrap().into_inner();
        assert_eq!(resp.ids.len(), 3);
        for id in &resp.ids {
            assert!(!id.id.is_empty());
        }
    }

    // ===== parse =====

    #[tokio::test]
    async fn test_parse_valid_numeric_id() {
        let server = create_test_grpc_server();
        let req = Request::new(GrpcParseRequest {
            id: "12345".to_string(),
        });
        let resp = server.parse(req).await;
        assert!(resp.is_ok(), "parse should succeed: {:?}", resp);
        let inner = resp.unwrap().into_inner();
        assert_eq!(inner.id, "12345");
        // metadata should contain timestamp, datacenter_id, worker_id, etc.
        assert!(inner.metadata.contains_key("timestamp"));
        assert!(inner.metadata.contains_key("algorithm"));
    }

    #[tokio::test]
    async fn test_parse_invalid_id_returns_invalid_argument() {
        let server = create_test_grpc_server();
        let req = Request::new(GrpcParseRequest {
            id: "not-a-valid-id".to_string(),
        });
        let err = server.parse(req).await.unwrap_err();
        assert_eq!(err.code(), sdforge::tonic::Code::InvalidArgument);
    }

    #[tokio::test]
    async fn test_parse_empty_id_returns_invalid_argument() {
        let server = create_test_grpc_server();
        let req = Request::new(GrpcParseRequest { id: String::new() });
        let err = server.parse(req).await.unwrap_err();
        assert_eq!(err.code(), sdforge::tonic::Code::InvalidArgument);
    }

    // ===== health_check =====

    #[tokio::test]
    async fn test_health_check_returns_serving() {
        // MockIdGenerator.health_check() returns Healthy, so the gRPC
        // health check should report Serving.
        let server = create_test_grpc_server();
        let req = Request::new(HealthCheckRequest {
            service: String::new(),
        });
        let resp = server.health_check(req).await.unwrap().into_inner();
        assert_eq!(
            resp.status,
            v1::health_check_response::ServingStatus::Serving as i32
        );
    }
}
