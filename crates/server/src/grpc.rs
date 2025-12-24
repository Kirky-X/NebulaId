use crate::handlers::ApiHandlers;
use crate::models::{
    BatchGenerateRequest, GenerateRequest,
    ParseRequest,
};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tonic::{Code, Request, Response, Status};

pub mod nebula_id {
    tonic::include_proto!("nebula.id.v1");
}

use nebula_id::nebula_id_service_server::NebulaIdService;
use nebula_id::{
    BatchGenerateRequest as GrpcBatchGenerateRequest, BatchGenerateResponse as GrpcBatchGenerateResponse,
    GenerateRequest as GrpcGenerateRequest, GenerateResponse as GrpcGenerateResponse,
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
            },
            Err(e) => Err(Status::new(Code::Internal, e.to_string())),
        }
    }

    async fn batch_generate(
        &self,
        request: Request<GrpcBatchGenerateRequest>,
    ) -> Result<Response<GrpcBatchGenerateResponse>, Status> {
        let req = request.into_inner();
        let tag = req.tag.clone();

        let batch_req = BatchGenerateRequest {
            workspace: req.namespace,
            group: tag.clone(),
            biz_tag: tag,
            size: Some(req.count as usize),
        };

        match self.handlers.batch_generate(batch_req).await {
            Ok(resp) => {
                let timestamp = resp.timestamp.parse().unwrap_or(0);
                let ids = resp.ids.into_iter().map(|id| GrpcGenerateResponse {
                    id,
                    timestamp,
                    sequence: 0,
                    worker_id: 0,
                    algorithm: resp.algorithm.clone(),
                }).collect();

                Ok(Response::new(GrpcBatchGenerateResponse { ids }))
            },
            Err(e) => Err(Status::new(Code::Internal, e.to_string())),
        }
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
        };

        match self.handlers.parse(parse_req).await {
            Ok(resp) => {
                let timestamp = resp.timestamp.parse().unwrap_or(0);
                let metadata: HashMap<String, String> = vec![
                    ("timestamp".to_string(), resp.metadata.timestamp.to_string()),
                    ("datacenter_id".to_string(), resp.metadata.datacenter_id.to_string()),
                    ("worker_id".to_string(), resp.metadata.worker_id.to_string()),
                    ("sequence".to_string(), resp.metadata.sequence.to_string()),
                    ("algorithm".to_string(), resp.metadata.algorithm),
                    ("biz_tag".to_string(), resp.metadata.biz_tag),
                ].into_iter().collect();

                Ok(Response::new(GrpcParseResponse {
                    id: resp.original_id,
                    timestamp,
                    sequence: resp.metadata.sequence as i32,
                    worker_id: resp.metadata.worker_id as i32,
                    algorithm: resp.algorithm,
                    metadata,
                }))
            },
            Err(e) => Err(Status::new(Code::InvalidArgument, e.to_string())),
        }
    }
}
