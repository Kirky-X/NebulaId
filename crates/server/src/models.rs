use serde::{Deserialize, Serialize};
use validator::Validate;

#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct GenerateRequest {
    #[validate(length(min = 1, max = 64))]
    pub workspace: String,

    #[validate(length(min = 1, max = 64))]
    pub group: String,

    #[validate(length(min = 1, max = 64))]
    pub biz_tag: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerateResponse {
    pub id: String,
    pub algorithm: String,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct BatchGenerateRequest {
    #[validate(length(min = 1, max = 64))]
    pub workspace: String,

    #[validate(length(min = 1, max = 64))]
    pub group: String,

    #[validate(length(min = 1, max = 64))]
    pub biz_tag: String,

    #[validate(range(min = 1, max = 1000))]
    pub size: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchGenerateResponse {
    pub ids: Vec<String>,
    pub size: usize,
    pub algorithm: String,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthResponse {
    pub status: String,
    pub algorithm: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiMetricsResponse {
    pub total_requests: u64,
    pub successful_generations: u64,
    pub failed_generations: u64,
    pub total_ids_generated: u64,
    pub avg_latency_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub code: i32,
    pub message: String,
    pub details: Option<String>,
}

impl ErrorResponse {
    pub fn new(code: i32, message: String) -> Self {
        Self {
            code,
            message,
            details: None,
        }
    }

    pub fn with_details(code: i32, message: String, details: String) -> Self {
        Self {
            code,
            message,
            details: Some(details),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyInfo {
    pub key_id: String,
    pub workspace_id: String,
    pub created_at: String,
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsResponse {
    pub total_requests: u64,
    pub successful_generations: u64,
    pub failed_generations: u64,
    pub total_ids_generated: u64,
    pub avg_latency_ms: u64,
    pub uptime_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct ParseRequest {
    pub id: String,

    #[validate(length(min = 1, max = 64))]
    pub workspace: String,

    #[validate(length(min = 1, max = 64))]
    pub group: String,

    #[validate(length(min = 1, max = 64))]
    pub biz_tag: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParseResponse {
    pub original_id: String,
    pub numeric_value: String,
    pub algorithm: String,
    pub metadata: IdMetadataResponse,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdMetadataResponse {
    pub timestamp: u64,
    pub datacenter_id: u8,
    pub worker_id: u16,
    pub sequence: u16,
    pub algorithm: String,
    pub biz_tag: String,
}
