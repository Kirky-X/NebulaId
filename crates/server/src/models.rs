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

use serde::{Deserialize, Serialize};
use serde_with::serde_as;
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub database: Option<String>,
    pub max_connections: u32,
    pub min_connections: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedisConfigInfo {
    #[serde(skip_serializing)]
    pub url: Option<String>, // Never expose URL
    pub pool_size: u32,
    pub key_prefix: String,
    pub ttl_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecureConfigResponse {
    pub app: AppConfigInfo,
    pub algorithm: AlgorithmConfigInfo,
    pub monitoring: MonitoringConfigInfo,
    pub logging: LoggingConfigInfo,
    pub rate_limit: RateLimitConfigInfo,
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

// ========== BizTag Models ==========

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct CreateBizTagRequest {
    #[serde_as(as = "serde_with::DisplayFromStr")]
    pub workspace_id: uuid::Uuid,

    #[serde_as(as = "serde_with::DisplayFromStr")]
    pub group_id: uuid::Uuid,

    #[validate(length(min = 1, max = 64))]
    pub name: String,

    pub description: Option<String>,

    #[validate(length(min = 1, max = 20))]
    pub algorithm: Option<String>,

    #[validate(length(min = 1, max = 20))]
    pub format: Option<String>,

    #[validate(length(max = 50))]
    pub prefix: Option<String>,

    #[validate(range(min = 1, max = 1000000))]
    pub base_step: Option<i32>,

    #[validate(range(min = 1, max = 10000000))]
    pub max_step: Option<i32>,

    pub datacenter_ids: Option<Vec<i32>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct UpdateBizTagRequest {
    #[validate(length(min = 1, max = 64))]
    pub name: Option<String>,

    pub description: Option<String>,

    #[validate(length(min = 1, max = 20))]
    pub algorithm: Option<String>,

    #[validate(length(min = 1, max = 20))]
    pub format: Option<String>,

    #[validate(length(max = 50))]
    pub prefix: Option<String>,

    #[validate(range(min = 1, max = 1000000))]
    pub base_step: Option<i32>,

    #[validate(range(min = 1, max = 10000000))]
    pub max_step: Option<i32>,

    pub datacenter_ids: Option<Vec<i32>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BizTagResponse {
    pub id: String,
    pub workspace_id: String,
    pub group_id: String,
    pub name: String,
    pub description: Option<String>,
    pub algorithm: String,
    pub format: String,
    pub prefix: String,
    pub base_step: i32,
    pub max_step: i32,
    pub datacenter_ids: Vec<i32>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BizTagListResponse {
    pub biz_tags: Vec<BizTagResponse>,
    pub total: u64,
    pub page: u64,
    pub page_size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct PaginationParams {
    #[serde(default = "default_page")]
    pub page: u64,

    #[serde(default = "default_page_size")]
    #[validate(range(min = 1, max = 100))]
    pub page_size: u64,
}

fn default_page() -> u64 {
    1
}

fn default_page_size() -> u64 {
    20
}

/// Shared utility: Convert NaiveDateTime to RFC3339 formatted string
pub fn naive_to_rfc3339(dt: chrono::NaiveDateTime) -> String {
    chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(dt, chrono::Utc).to_rfc3339()
}
