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

/// Health status of the system
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum HealthStatus {
    /// System is operating normally
    Healthy,
    /// System is degraded but still operational
    Degraded,
    /// System is not operational
    Unhealthy,
}

impl std::fmt::Display for HealthStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HealthStatus::Healthy => write!(f, "healthy"),
            HealthStatus::Degraded => write!(f, "degraded"),
            HealthStatus::Unhealthy => write!(f, "unhealthy"),
        }
    }
}

impl From<&str> for HealthStatus {
    fn from(s: &str) -> Self {
        match s {
            "healthy" => HealthStatus::Healthy,
            "degraded" => HealthStatus::Degraded,
            "unhealthy" => HealthStatus::Unhealthy,
            _ => HealthStatus::Unhealthy,
        }
    }
}

impl From<String> for HealthStatus {
    fn from(s: String) -> Self {
        s.as_str().into()
    }
}

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
    pub status: HealthStatus,
    pub algorithm: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadyResponse {
    pub ready: bool,
    pub database: bool,
    pub cache: bool,
    pub message: String,
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
    pub database: DatabaseMetrics,
    pub cache: CacheMetrics,
    pub algorithms: Vec<AlgorithmMetrics>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseMetrics {
    pub status: HealthStatus,
    pub connection_pool: ConnectionPoolMetrics,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionPoolMetrics {
    pub active_connections: u32,
    pub idle_connections: u32,
    pub max_connections: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheMetrics {
    pub status: HealthStatus,
    pub hit_rate: f64,
    pub memory_usage_mb: Option<u64>,
    pub key_count: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlgorithmMetrics {
    pub algorithm: String,
    pub status: HealthStatus,
    pub total_generated: u64,
    pub total_failed: u64,
    pub cache_hit_rate: f64,
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
    pub workspace_id: Option<String>, // Optional: filter by workspace
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

/// Query params for listing groups (requires workspace parameter)
#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct GroupListParams {
    pub workspace: String,
    #[serde(default = "default_page")]
    pub page: u64,
    #[serde(default = "default_page_size")]
    #[validate(range(min = 1, max = 100))]
    pub page_size: u64,
}

/// Shared utility: Convert NaiveDateTime to RFC3339 formatted string
pub fn naive_to_rfc3339(dt: chrono::NaiveDateTime) -> String {
    chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(dt, chrono::Utc).to_rfc3339()
}

/// Shared utility: Convert DateTime<FixedOffset> to RFC3339 formatted string
pub fn datetime_to_rfc3339(dt: chrono::DateTime<chrono::FixedOffset>) -> String {
    dt.to_rfc3339()
}

// ========== API Key Models ==========

#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct CreateApiKeyRequest {
    pub workspace_id: Option<String>, // Optional: None for admin keys, required for user keys
    #[validate(length(min = 1, max = 64))]
    pub name: String,

    pub description: Option<String>,

    #[validate(length(min = 1, max = 20))]
    pub role: Option<String>, // "admin" or "user"

    #[validate(range(min = 100, max = 1000000))]
    pub rate_limit: Option<i32>,

    pub expires_at: Option<String>, // RFC3339 format
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyResponse {
    pub id: String,
    pub key_id: String,
    pub key_prefix: String,
    pub name: String,
    pub description: Option<String>,
    pub role: String,
    pub rate_limit: i32,
    pub enabled: bool,
    pub expires_at: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyWithSecretResponse {
    pub key: ApiKeyResponse,
    pub key_secret: String, // Only returned on creation
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyListResponse {
    pub api_keys: Vec<ApiKeyResponse>,
    pub total: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RevokeApiKeyResponse {
    pub success: bool,
    pub message: String,
}

// ========== Workspace Models ==========

#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct CreateWorkspaceRequest {
    #[validate(length(min = 1, max = 64))]
    pub name: String,

    pub description: Option<String>,

    #[validate(range(min = 1, max = 1000))]
    pub max_groups: Option<i32>,

    #[validate(range(min = 1, max = 10000))]
    pub max_biz_tags: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserApiKeyInfo {
    pub key_id: String,
    pub key_secret: String,
    pub key_prefix: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceResponse {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub status: String,
    pub max_groups: i32,
    pub max_biz_tags: i32,
    pub created_at: String,
    pub updated_at: String,
    pub user_api_key: Option<UserApiKeyInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceListResponse {
    pub workspaces: Vec<WorkspaceResponse>,
    pub total: u64,
}

// ========== Group Models ==========

#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct CreateGroupRequest {
    #[validate(length(min = 1, max = 64))]
    pub workspace: String, // workspace name

    #[validate(length(min = 1, max = 64))]
    pub name: String,

    pub description: Option<String>,

    #[validate(range(min = 1, max = 1000))]
    pub max_biz_tags: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupResponse {
    pub id: String,
    pub workspace_id: String,
    pub workspace_name: String,
    pub name: String,
    pub description: Option<String>,
    pub max_biz_tags: i32,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupListResponse {
    pub groups: Vec<GroupResponse>,
    pub total: u64,
}
