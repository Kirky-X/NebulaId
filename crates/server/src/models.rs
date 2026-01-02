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

    #[validate(length(min = 1, max = 20))]
    pub algorithm: Option<String>,
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

    #[validate(range(min = 1, max = 100))]
    pub size: Option<usize>,

    #[validate(length(min = 1, max = 20))]
    pub algorithm: Option<String>,
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

    #[validate(length(min = 1, max = 32))]
    #[serde(default)]
    pub algorithm: String,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigResponse {
    pub app: AppConfigInfo,
    pub database: DatabaseConfigInfo,
    pub redis: RedisConfigInfo,
    pub algorithm: AlgorithmConfigInfo,
    pub monitoring: MonitoringConfigInfo,
    pub logging: LoggingConfigInfo,
    pub rate_limit: RateLimitConfigInfo,
    pub tls: TlsConfigInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfigInfo {
    pub name: String,
    pub host: String,
    pub http_port: u16,
    pub grpc_port: u16,
    pub dc_id: u8,
    pub worker_id: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseConfigInfo {
    pub engine: String,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub max_connections: u32,
    pub min_connections: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedisConfigInfo {
    pub url: String,
    pub pool_size: u32,
    pub key_prefix: String,
    pub ttl_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlgorithmConfigInfo {
    pub default: String,
    pub segment: SegmentConfigInfo,
    pub snowflake: SnowflakeConfigInfo,
    pub uuid_v7: UuidV7ConfigInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SegmentConfigInfo {
    pub base_step: u64,
    pub min_step: u64,
    pub max_step: u64,
    pub switch_threshold: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnowflakeConfigInfo {
    pub datacenter_id_bits: u8,
    pub worker_id_bits: u8,
    pub sequence_bits: u8,
    pub clock_drift_threshold_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UuidV7ConfigInfo {
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitoringConfigInfo {
    pub metrics_enabled: bool,
    pub metrics_path: String,
    pub tracing_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfigInfo {
    pub level: String,
    pub format: String,
    pub include_location: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitConfigInfo {
    pub enabled: bool,
    pub default_rps: u32,
    pub burst_size: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TlsConfigInfo {
    pub enabled: bool,
    pub http_enabled: bool,
    pub grpc_enabled: bool,
    pub has_cert: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct UpdateRateLimitRequest {
    #[validate(range(min = 1, max = 1000000))]
    pub default_rps: Option<u32>,

    #[validate(range(min = 1, max = 1000))]
    pub burst_size: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct UpdateLoggingRequest {
    #[validate(length(min = 1, max = 20))]
    pub level: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct SetAlgorithmRequest {
    #[validate(length(min = 1, max = 64))]
    pub biz_tag: String,

    #[validate(length(min = 1, max = 20))]
    pub algorithm: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetAlgorithmResponse {
    pub success: bool,
    pub biz_tag: String,
    pub algorithm: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateConfigResponse {
    pub success: bool,
    pub message: String,
    pub config: Option<ConfigResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiInfoResponse {
    pub name: String,
    pub version: String,
    pub description: String,
    pub endpoints: Vec<String>,
}
