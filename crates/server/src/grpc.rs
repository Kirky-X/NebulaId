use crate::handlers::ApiHandlers;
use crate::models::{BatchGenerateRequest, GenerateRequest, ParseRequest};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::StreamExt;
use tonic::{Code, Request, Response, Status};

pub mod nebula_id {
    tonic::include_proto!("nebula.id.v1");
}

use nebula_id::nebula_id_service_server::NebulaIdService;
use nebula_id::{
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
            Err(e) => Err(Status::new(Code::Internal, e.to_string())),
        }
    }

    async fn batch_generate(
        &self,
        request: Request<GrpcBatchGenerateRequest>,
    ) -> Result<Response<GrpcBatchGenerateResponse>, Status> {
        let req = request.into_inner();
        let tag = req.tag.clone();

        tracing::info!(
            "Received gRPC batch_generate request with count: {}",
            req.count
        );

        // Validate batch size
        if req.count == 0 {
            tracing::warn!("Batch size validation failed: count is 0");
            return Err(Status::invalid_argument("Batch size cannot be zero"));
        }
        if req.count > 100 {
            tracing::warn!(
                "Batch size validation failed: count {} exceeds maximum 100",
                req.count
            );
            return Err(Status::invalid_argument(format!(
                "Batch size {} exceeds maximum allowed value of 100",
                req.count
            )));
        }

        tracing::info!("Batch size validation passed: {}", req.count);

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
            Err(e) => Err(Status::new(Code::Internal, e.to_string())),
        }
    }

    async fn batch_generate_stream(
        &self,
        request: Request<tonic::Streaming<BatchGenerateStreamRequest>>,
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
            Err(e) => Err(Status::new(Code::InvalidArgument, e.to_string())),
        }
    }

    async fn health_check(
        &self,
        _request: Request<HealthCheckRequest>,
    ) -> Result<Response<HealthCheckResponse>, Status> {
        let health = self.handlers.health().await;
        let status = if health.status == "healthy" {
            nebula_id::health_check_response::ServingStatus::Serving
        } else {
            nebula_id::health_check_response::ServingStatus::NotServing
        };

        Ok(Response::new(HealthCheckResponse {
            status: status as i32,
        }))
    }
}
