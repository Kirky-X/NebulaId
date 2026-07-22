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

use sdforge::utoipa::ToSchema;
use serde::{Deserialize, Serialize};
use serde_with::serde_as;
use validator::Validate;

/// Health status of the system
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ToSchema)]
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

#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
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

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct GenerateResponse {
    pub id: String,
    pub algorithm: String,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
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

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BatchGenerateResponse {
    pub ids: Vec<String>,
    pub size: usize,
    pub algorithm: String,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct HealthResponse {
    pub status: HealthStatus,
    pub algorithm: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ReadyResponse {
    pub ready: bool,
    pub database: bool,
    pub cache: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ApiMetricsResponse {
    pub total_requests: u64,
    pub successful_generations: u64,
    pub failed_generations: u64,
    pub total_ids_generated: u64,
    pub avg_latency_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, ToSchema)]
/// 结构化 API 错误码（用于增强的错误处理）
pub enum ApiErrorCode {
    // 认证错误 (1xxx)
    Unauthorized = 1001,
    Forbidden = 1002,
    InvalidApiKey = 1003,
    ApiKeyExpired = 1004,
    ApiKeyDisabled = 1005,

    // 资源错误 (2xxx)
    WorkspaceNotFound = 2001,
    GroupNotFound = 2002,
    BizTagNotFound = 2003,
    ResourceAlreadyExists = 2004,

    // 验证错误 (3xxx)
    InvalidInput = 3001,
    ValidationError = 3002,
    MissingRequiredField = 3003,
    InvalidUuid = 3004,

    // 限流错误 (4xxx)
    RateLimitExceeded = 4001,

    // 服务器错误 (5xxx)
    InternalError = 5001,
    DatabaseError = 5002,
    CacheError = 5003,
    ServiceUnavailable = 5004,
}

impl std::fmt::Display for ApiErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:04}", *self as i32)
    }
}

/// Error message constants for consistent error responses
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorMessage {
    // Authentication errors
    InvalidApiKey,
    ApiKeyExpired,
    ApiKeyDisabled,

    // Resource errors
    WorkspaceNotFound,
    GroupNotFound,
    BizTagNotFound,

    // Validation errors
    InvalidInput,
    InvalidIdFormat,
    InvalidAlgorithmType,

    // Rate limiting
    RateLimitExceeded,

    // Server errors
    InternalError,
    DatabaseError,
    CacheError,
    ServiceUnavailable,
    AlgorithmError,
    ConfigurationError,
}

impl ErrorMessage {
    /// Get the user-friendly error message
    pub fn message(&self) -> &'static str {
        match self {
            // Authentication
            ErrorMessage::InvalidApiKey => "Invalid API key signature",
            ErrorMessage::ApiKeyExpired => "API key has expired",
            ErrorMessage::ApiKeyDisabled => "API key has been disabled",

            // Resources
            ErrorMessage::WorkspaceNotFound => "Workspace not found",
            ErrorMessage::GroupNotFound => "Group not found",
            ErrorMessage::BizTagNotFound => "Biz tag not found",

            // Validation
            ErrorMessage::InvalidInput => "Invalid input",
            ErrorMessage::InvalidIdFormat => "Invalid ID format",
            ErrorMessage::InvalidAlgorithmType => "Invalid algorithm type",

            // Rate limiting
            ErrorMessage::RateLimitExceeded => "Rate limit exceeded",

            // Server errors
            ErrorMessage::InternalError => "Internal server error",
            ErrorMessage::DatabaseError => "Database operation failed",
            ErrorMessage::CacheError => "Cache service unavailable",
            ErrorMessage::ServiceUnavailable => "Service unavailable",
            ErrorMessage::AlgorithmError => "ID generation algorithm error",
            ErrorMessage::ConfigurationError => "Configuration error",
        }
    }

    /// Get error message with context (for development/debugging)
    pub fn with_context(&self, context: &str) -> String {
        match self {
            ErrorMessage::InvalidInput
            | ErrorMessage::InvalidIdFormat
            | ErrorMessage::InvalidAlgorithmType
            | ErrorMessage::WorkspaceNotFound
            | ErrorMessage::GroupNotFound
            | ErrorMessage::BizTagNotFound
            | ErrorMessage::DatabaseError
            | ErrorMessage::CacheError
            | ErrorMessage::ConfigurationError
            | ErrorMessage::AlgorithmError => {
                format!("{}: {}", self.message(), context)
            }
            _ => self.message().to_string(),
        }
    }
}

///增强的 API 错误响应（包含结构化错误码）
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ApiErrorResponse {
    pub code: String,            // 错误码，如 "1001"
    pub message: String,         // 用户友好的错误消息
    pub details: Option<String>, // 详细信息（可选，生产环境隐藏）
    pub request_id: String,      // 请求追踪 ID
    pub timestamp: i64,          // 错误发生时间
}

impl ApiErrorResponse {
    pub fn new(code: ApiErrorCode, message: String) -> Self {
        Self {
            code: code.to_string(),
            message,
            details: None,
            request_id: uuid::Uuid::new_v4().to_string(),
            timestamp: chrono::Utc::now().timestamp_millis(),
        }
    }

    pub fn with_details(mut self, details: String) -> Self {
        self.details = Some(details);
        self
    }
}

/// 从旧的 ErrorResponse 转换为新的 ApiErrorResponse
impl From<ErrorResponse> for ApiErrorResponse {
    fn from(err: ErrorResponse) -> Self {
        let code = match err.code {
            401 => ApiErrorCode::Unauthorized,
            403 => ApiErrorCode::Forbidden,
            404 => ApiErrorCode::WorkspaceNotFound, // 默认资源错误
            429 => ApiErrorCode::RateLimitExceeded,
            500 => ApiErrorCode::InternalError,
            _ => ApiErrorCode::InternalError,
        };

        Self::new(code, err.message).with_details(
            err.details
                .unwrap_or_else(|| "No additional details".to_string()),
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ApiKeyInfo {
    pub key_id: String,
    pub workspace_id: String,
    pub created_at: String,
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
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

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DatabaseMetrics {
    pub status: HealthStatus,
    pub connection_pool: ConnectionPoolMetrics,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ConnectionPoolMetrics {
    pub active_connections: u32,
    pub idle_connections: u32,
    pub max_connections: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CacheMetrics {
    pub status: HealthStatus,
    /// L15 修复：缓存命中率。`hit_rate = 0.0` 在 `has_cache = false` 时
    /// 表示「无缓存概念」，不是「命中率 0%」。客户端必须先检查 `has_cache`
    /// 再决定是否展示 `hit_rate`。
    pub hit_rate: f64,
    /// ARCH-MED-004 修复：明确表达「当前部署是否有缓存算法」。
    /// `false` 时 `hit_rate` 字段无意义（恒为 0.0），客户端不应展示。
    /// `true` 时 `hit_rate` 是所有缓存算法的平均命中率。
    pub has_cache: bool,
    pub memory_usage_mb: Option<u64>,
    pub key_count: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AlgorithmMetrics {
    pub algorithm: String,
    pub status: HealthStatus,
    pub total_generated: u64,
    pub total_failed: u64,
    /// L15 修复：`None` 表示该算法无缓存概念，`Some(rate)` 表示真实命中率。
    pub cache_hit_rate: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct ParseRequest {
    pub id: String,

    #[validate(length(min = 1, max = 64))]
    pub workspace: String,

    #[validate(length(min = 1, max = 64))]
    pub group: String,

    #[validate(length(min = 1, max = 64))]
    pub biz_tag: String,

    #[validate(length(min = 0, max = 32))]
    #[serde(default)]
    pub algorithm: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ParseResponse {
    pub original_id: String,
    pub numeric_value: String,
    pub algorithm: String,
    pub metadata: IdMetadataResponse,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct IdMetadataResponse {
    pub timestamp: u64,
    pub datacenter_id: u8,
    pub worker_id: u16,
    pub sequence: u16,
    pub algorithm: String,
    pub biz_tag: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ConfigResponse {
    pub app: AppConfigInfo,
    pub database: DatabaseConfigInfo,
    pub algorithm: AlgorithmConfigInfo,
    pub monitoring: MonitoringConfigInfo,
    pub logging: LoggingConfigInfo,
    pub rate_limit: RateLimitConfigInfo,
    pub tls: TlsConfigInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AppConfigInfo {
    pub name: String,
    pub host: String,
    pub http_port: u16,
    pub grpc_port: u16,
    pub dc_id: u8,
    pub worker_id: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
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

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SecureConfigResponse {
    pub app: AppConfigInfo,
    pub algorithm: AlgorithmConfigInfo,
    pub monitoring: MonitoringConfigInfo,
    pub logging: LoggingConfigInfo,
    pub rate_limit: RateLimitConfigInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AlgorithmConfigInfo {
    pub default: String,
    pub segment: SegmentConfigInfo,
    pub snowflake: SnowflakeConfigInfo,
    pub uuid_v7: UuidV7ConfigInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SegmentConfigInfo {
    pub base_step: u64,
    pub min_step: u64,
    pub max_step: u64,
    pub switch_threshold: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SnowflakeConfigInfo {
    pub datacenter_id_bits: u8,
    pub worker_id_bits: u8,
    pub sequence_bits: u8,
    pub clock_drift_threshold_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UuidV7ConfigInfo {
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MonitoringConfigInfo {
    pub metrics_enabled: bool,
    pub metrics_path: String,
    pub tracing_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LoggingConfigInfo {
    pub level: String,
    pub format: String,
    pub include_location: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RateLimitConfigInfo {
    pub enabled: bool,
    pub default_rps: u32,
    pub burst_size: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TlsConfigInfo {
    pub enabled: bool,
    pub http_enabled: bool,
    pub grpc_enabled: bool,
    pub has_cert: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct UpdateRateLimitRequest {
    #[validate(range(min = 1, max = 1000000))]
    pub default_rps: Option<u32>,

    #[validate(range(min = 1, max = 1000))]
    pub burst_size: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct UpdateLoggingRequest {
    #[validate(length(min = 1, max = 20))]
    pub level: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct SetAlgorithmRequest {
    #[validate(length(min = 1, max = 64))]
    pub biz_tag: String,

    #[validate(length(min = 1, max = 20))]
    pub algorithm: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SetAlgorithmResponse {
    pub success: bool,
    pub biz_tag: String,
    pub algorithm: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateConfigResponse {
    pub success: bool,
    pub message: String,
    pub config: Option<ConfigResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ApiInfoResponse {
    pub name: String,
    pub version: String,
    pub description: String,
    pub endpoints: Vec<String>,
}

// ========== BizTag Models ==========

#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct CreateBizTagRequest {
    #[serde_as(as = "serde_with::DisplayFromStr")]
    pub workspace_id: uuid::Uuid,

    #[serde_as(as = "serde_with::DisplayFromStr")]
    pub group_id: uuid::Uuid,

    #[validate(length(min = 1, max = 64))]
    pub name: String,

    #[validate(length(max = 512))]
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

#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct UpdateBizTagRequest {
    #[validate(length(min = 1, max = 64))]
    pub name: Option<String>,

    #[validate(length(max = 512))]
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

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
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

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BizTagListResponse {
    pub biz_tags: Vec<BizTagResponse>,
    pub total: u64,
    pub page: u64,
    pub page_size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
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
#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
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

/// Shared utility: Convert `DateTime<FixedOffset>` to RFC3339 formatted string
pub fn datetime_to_rfc3339(dt: chrono::DateTime<chrono::FixedOffset>) -> String {
    dt.to_rfc3339()
}

// ========== API Key Models ==========

#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
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

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
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

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ApiKeyWithSecretResponse {
    pub key: ApiKeyResponse,
    pub key_secret: String, // Only returned on creation
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ApiKeyListResponse {
    pub api_keys: Vec<ApiKeyResponse>,
    pub total: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RevokeApiKeyResponse {
    pub success: bool,
    pub message: String,
}

// ========== Workspace Models ==========

#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct CreateWorkspaceRequest {
    #[validate(length(min = 1, max = 64))]
    pub name: String,

    pub description: Option<String>,

    #[validate(range(min = 1, max = 1000))]
    pub max_groups: Option<i32>,

    #[validate(range(min = 1, max = 10000))]
    pub max_biz_tags: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UserApiKeyInfo {
    pub key_id: String,
    pub key_secret: String,
    pub key_prefix: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
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

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct WorkspaceListResponse {
    pub workspaces: Vec<WorkspaceResponse>,
    pub total: u64,
}

// ========== Group Models ==========

#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
pub struct CreateGroupRequest {
    #[validate(length(min = 1, max = 64))]
    pub workspace: String, // workspace name

    #[validate(length(min = 1, max = 64))]
    pub name: String,

    pub description: Option<String>,

    #[validate(range(min = 1, max = 1000))]
    pub max_biz_tags: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
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

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct GroupListResponse {
    pub groups: Vec<GroupResponse>,
    pub total: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    // ========== HealthStatus::Display ==========

    #[test]
    fn test_health_status_display_healthy() {
        assert_eq!(HealthStatus::Healthy.to_string(), "healthy");
    }

    #[test]
    fn test_health_status_display_degraded() {
        assert_eq!(HealthStatus::Degraded.to_string(), "degraded");
    }

    #[test]
    fn test_health_status_display_unhealthy() {
        assert_eq!(HealthStatus::Unhealthy.to_string(), "unhealthy");
    }

    // ========== HealthStatus::From<&str> ==========

    #[test]
    fn test_health_status_from_str_known_values() {
        assert_eq!(HealthStatus::from("healthy"), HealthStatus::Healthy);
        assert_eq!(HealthStatus::from("degraded"), HealthStatus::Degraded);
        assert_eq!(HealthStatus::from("unhealthy"), HealthStatus::Unhealthy);
    }

    #[test]
    fn test_health_status_from_str_unknown_defaults_to_unhealthy() {
        // Unknown strings must fall back to Unhealthy (fail-closed).
        assert_eq!(HealthStatus::from("unknown"), HealthStatus::Unhealthy);
        assert_eq!(HealthStatus::from(""), HealthStatus::Unhealthy);
        // Case-sensitive: "HEALTHY" is not "healthy".
        assert_eq!(HealthStatus::from("HEALTHY"), HealthStatus::Unhealthy);
    }

    // ========== HealthStatus::From<String> ==========

    #[test]
    fn test_health_status_from_string_delegates_to_str() {
        assert_eq!(
            HealthStatus::from("healthy".to_string()),
            HealthStatus::Healthy
        );
        assert_eq!(
            HealthStatus::from("degraded".to_string()),
            HealthStatus::Degraded
        );
        assert_eq!(
            HealthStatus::from("unhealthy".to_string()),
            HealthStatus::Unhealthy
        );
        assert_eq!(
            HealthStatus::from("bogus".to_string()),
            HealthStatus::Unhealthy
        );
    }

    // ========== HealthStatus serde round-trip ==========

    #[test]
    fn test_health_status_serde_roundtrip_preserves_variants() {
        for status in [
            HealthStatus::Healthy,
            HealthStatus::Degraded,
            HealthStatus::Unhealthy,
        ] {
            let json = serde_json::to_string(&status).unwrap();
            let back: HealthStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(back, status);
        }
    }

    // ========== ErrorResponse ==========

    #[test]
    fn test_error_response_new_omits_details() {
        let resp = ErrorResponse::new(404, "Not Found".to_string());
        assert_eq!(resp.code, 404);
        assert_eq!(resp.message, "Not Found");
        assert_eq!(resp.details, None);
    }

    #[test]
    fn test_error_response_with_details_attaches_details() {
        let resp = ErrorResponse::with_details(500, "Internal".to_string(), "db down".to_string());
        assert_eq!(resp.code, 500);
        assert_eq!(resp.message, "Internal");
        assert_eq!(resp.details, Some("db down".to_string()));
    }

    #[test]
    fn test_error_response_serde_roundtrip_with_details() {
        let resp = ErrorResponse::with_details(400, "Bad".to_string(), "missing field".to_string());
        let json = serde_json::to_string(&resp).unwrap();
        let back: ErrorResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(back.code, 400);
        assert_eq!(back.message, "Bad");
        assert_eq!(back.details, Some("missing field".to_string()));
    }

    #[test]
    fn test_error_response_serde_roundtrip_without_details() {
        let resp = ErrorResponse::new(404, "Not Found".to_string());
        let json = serde_json::to_string(&resp).unwrap();
        let back: ErrorResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(back.code, 404);
        assert_eq!(back.message, "Not Found");
        assert_eq!(back.details, None);
    }

    // ========== ApiErrorCode::Display ==========

    #[test]
    fn test_api_error_code_display_all_variants() {
        assert_eq!(ApiErrorCode::Unauthorized.to_string(), "1001");
        assert_eq!(ApiErrorCode::Forbidden.to_string(), "1002");
        assert_eq!(ApiErrorCode::InvalidApiKey.to_string(), "1003");
        assert_eq!(ApiErrorCode::ApiKeyExpired.to_string(), "1004");
        assert_eq!(ApiErrorCode::ApiKeyDisabled.to_string(), "1005");
        assert_eq!(ApiErrorCode::WorkspaceNotFound.to_string(), "2001");
        assert_eq!(ApiErrorCode::GroupNotFound.to_string(), "2002");
        assert_eq!(ApiErrorCode::BizTagNotFound.to_string(), "2003");
        assert_eq!(ApiErrorCode::ResourceAlreadyExists.to_string(), "2004");
        assert_eq!(ApiErrorCode::InvalidInput.to_string(), "3001");
        assert_eq!(ApiErrorCode::ValidationError.to_string(), "3002");
        assert_eq!(ApiErrorCode::MissingRequiredField.to_string(), "3003");
        assert_eq!(ApiErrorCode::InvalidUuid.to_string(), "3004");
        assert_eq!(ApiErrorCode::RateLimitExceeded.to_string(), "4001");
        assert_eq!(ApiErrorCode::InternalError.to_string(), "5001");
        assert_eq!(ApiErrorCode::DatabaseError.to_string(), "5002");
        assert_eq!(ApiErrorCode::CacheError.to_string(), "5003");
        assert_eq!(ApiErrorCode::ServiceUnavailable.to_string(), "5004");
    }

    // ========== ApiErrorResponse ==========

    #[test]
    fn test_api_error_response_new_populates_request_id_and_timestamp() {
        let resp = ApiErrorResponse::new(ApiErrorCode::Unauthorized, "Unauthorized".to_string());
        assert_eq!(resp.code, "1001");
        assert_eq!(resp.message, "Unauthorized");
        assert_eq!(resp.details, None);
        // request_id is a UUID string (non-empty).
        assert!(!resp.request_id.is_empty());
        // timestamp is millis since epoch — must be positive.
        assert!(resp.timestamp > 0);
    }

    #[test]
    fn test_api_error_response_with_details_chain_attaches_details() {
        let resp = ApiErrorResponse::new(ApiErrorCode::Forbidden, "Forbidden".to_string())
            .with_details("admin role required".to_string());
        assert_eq!(resp.code, "1002");
        assert_eq!(resp.details, Some("admin role required".to_string()));
    }

    #[test]
    fn test_api_error_response_serde_roundtrip() {
        let resp = ApiErrorResponse::new(ApiErrorCode::InvalidInput, "bad".to_string())
            .with_details("ctx".to_string());
        let json = serde_json::to_string(&resp).unwrap();
        let back: ApiErrorResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(back.code, resp.code);
        assert_eq!(back.message, resp.message);
        assert_eq!(back.details, resp.details);
        assert_eq!(back.request_id, resp.request_id);
        assert_eq!(back.timestamp, resp.timestamp);
    }

    // ========== From<ErrorResponse> for ApiErrorResponse ==========

    #[test]
    fn test_from_error_response_maps_401_to_unauthorized() {
        let err = ErrorResponse::new(401, "Unauthorized".to_string());
        let api_err: ApiErrorResponse = err.into();
        assert_eq!(api_err.code, "1001");
        assert_eq!(api_err.message, "Unauthorized");
        // No details on the source -> default placeholder string.
        assert_eq!(api_err.details, Some("No additional details".to_string()));
    }

    #[test]
    fn test_from_error_response_maps_403_to_forbidden() {
        let err = ErrorResponse::new(403, "Forbidden".to_string());
        let api_err: ApiErrorResponse = err.into();
        assert_eq!(api_err.code, "1002");
    }

    #[test]
    fn test_from_error_response_maps_404_to_workspace_not_found() {
        let err = ErrorResponse::new(404, "Not Found".to_string());
        let api_err: ApiErrorResponse = err.into();
        assert_eq!(api_err.code, "2001");
    }

    #[test]
    fn test_from_error_response_maps_429_to_rate_limit_exceeded() {
        let err = ErrorResponse::new(429, "Too Many".to_string());
        let api_err: ApiErrorResponse = err.into();
        assert_eq!(api_err.code, "4001");
    }

    #[test]
    fn test_from_error_response_maps_500_to_internal_error() {
        let err = ErrorResponse::new(500, "Internal".to_string());
        let api_err: ApiErrorResponse = err.into();
        assert_eq!(api_err.code, "5001");
    }

    #[test]
    fn test_from_error_response_maps_unknown_code_to_internal_error() {
        let err = ErrorResponse::new(418, "Teapot".to_string());
        let api_err: ApiErrorResponse = err.into();
        assert_eq!(api_err.code, "5001");
    }

    #[test]
    fn test_from_error_response_preserves_existing_details() {
        let err =
            ErrorResponse::with_details(401, "Unauthorized".to_string(), "key expired".to_string());
        let api_err: ApiErrorResponse = err.into();
        assert_eq!(api_err.details, Some("key expired".to_string()));
    }

    // ========== ErrorMessage::message ==========

    #[test]
    fn test_error_message_returns_user_friendly_strings() {
        assert_eq!(
            ErrorMessage::InvalidApiKey.message(),
            "Invalid API key signature"
        );
        assert_eq!(ErrorMessage::ApiKeyExpired.message(), "API key has expired");
        assert_eq!(
            ErrorMessage::ApiKeyDisabled.message(),
            "API key has been disabled"
        );
        assert_eq!(
            ErrorMessage::WorkspaceNotFound.message(),
            "Workspace not found"
        );
        assert_eq!(ErrorMessage::GroupNotFound.message(), "Group not found");
        assert_eq!(ErrorMessage::BizTagNotFound.message(), "Biz tag not found");
        assert_eq!(ErrorMessage::InvalidInput.message(), "Invalid input");
        assert_eq!(ErrorMessage::InvalidIdFormat.message(), "Invalid ID format");
        assert_eq!(
            ErrorMessage::InvalidAlgorithmType.message(),
            "Invalid algorithm type"
        );
        assert_eq!(
            ErrorMessage::RateLimitExceeded.message(),
            "Rate limit exceeded"
        );
        assert_eq!(
            ErrorMessage::InternalError.message(),
            "Internal server error"
        );
        assert_eq!(
            ErrorMessage::DatabaseError.message(),
            "Database operation failed"
        );
        assert_eq!(
            ErrorMessage::CacheError.message(),
            "Cache service unavailable"
        );
        assert_eq!(
            ErrorMessage::ServiceUnavailable.message(),
            "Service unavailable"
        );
        assert_eq!(
            ErrorMessage::AlgorithmError.message(),
            "ID generation algorithm error"
        );
        assert_eq!(
            ErrorMessage::ConfigurationError.message(),
            "Configuration error"
        );
    }

    // ========== ErrorMessage::with_context (context-appending branch) ==========

    #[test]
    fn test_error_message_with_context_invalid_input_appends_context() {
        assert_eq!(
            ErrorMessage::InvalidInput.with_context("missing field"),
            "Invalid input: missing field"
        );
    }

    #[test]
    fn test_error_message_with_context_invalid_id_format_appends_context() {
        assert_eq!(
            ErrorMessage::InvalidIdFormat.with_context("bad uuid"),
            "Invalid ID format: bad uuid"
        );
    }

    #[test]
    fn test_error_message_with_context_invalid_algorithm_type_appends_context() {
        assert_eq!(
            ErrorMessage::InvalidAlgorithmType.with_context("foo"),
            "Invalid algorithm type: foo"
        );
    }

    #[test]
    fn test_error_message_with_context_resource_errors_append_context() {
        assert_eq!(
            ErrorMessage::WorkspaceNotFound.with_context("ws-1"),
            "Workspace not found: ws-1"
        );
        assert_eq!(
            ErrorMessage::GroupNotFound.with_context("g-1"),
            "Group not found: g-1"
        );
        assert_eq!(
            ErrorMessage::BizTagNotFound.with_context("tag-1"),
            "Biz tag not found: tag-1"
        );
    }

    #[test]
    fn test_error_message_with_context_server_errors_append_context() {
        assert_eq!(
            ErrorMessage::DatabaseError.with_context("conn refused"),
            "Database operation failed: conn refused"
        );
        assert_eq!(
            ErrorMessage::CacheError.with_context("redis down"),
            "Cache service unavailable: redis down"
        );
        assert_eq!(
            ErrorMessage::ConfigurationError.with_context("invalid toml"),
            "Configuration error: invalid toml"
        );
        assert_eq!(
            ErrorMessage::AlgorithmError.with_context("snowflake"),
            "ID generation algorithm error: snowflake"
        );
    }

    #[test]
    fn test_error_message_with_context_passthrough_variants_ignore_context() {
        // These variants fall into the `_ =>` arm and must NOT append context.
        assert_eq!(
            ErrorMessage::InvalidApiKey.with_context("ctx"),
            "Invalid API key signature"
        );
        assert_eq!(
            ErrorMessage::ApiKeyExpired.with_context("ctx"),
            "API key has expired"
        );
        assert_eq!(
            ErrorMessage::ApiKeyDisabled.with_context("ctx"),
            "API key has been disabled"
        );
        assert_eq!(
            ErrorMessage::RateLimitExceeded.with_context("ctx"),
            "Rate limit exceeded"
        );
        assert_eq!(
            ErrorMessage::InternalError.with_context("ctx"),
            "Internal server error"
        );
        assert_eq!(
            ErrorMessage::ServiceUnavailable.with_context("ctx"),
            "Service unavailable"
        );
    }

    // ========== PaginationParams / GroupListParams serde defaults ==========

    #[test]
    fn test_pagination_params_uses_defaults_when_empty() {
        // Exercises default_page (returns 1) and default_page_size (returns 20).
        let params: PaginationParams = serde_json::from_str("{}").unwrap();
        assert_eq!(params.page, 1);
        assert_eq!(params.page_size, 20);
        assert_eq!(params.workspace_id, None);
    }

    #[test]
    fn test_pagination_params_with_explicit_values() {
        let json = r#"{"page":3,"page_size":50,"workspace_id":"ws-1"}"#;
        let params: PaginationParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.page, 3);
        assert_eq!(params.page_size, 50);
        assert_eq!(params.workspace_id, Some("ws-1".to_string()));
    }

    #[test]
    fn test_group_list_params_uses_defaults() {
        let json = r#"{"workspace":"test"}"#;
        let params: GroupListParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.workspace, "test");
        assert_eq!(params.page, 1);
        assert_eq!(params.page_size, 20);
    }

    // ========== datetime_to_rfc3339 ==========

    #[test]
    fn test_datetime_to_rfc3339_preserves_offset() {
        let dt = chrono::DateTime::parse_from_rfc3339("2026-07-20T12:30:45+08:00").unwrap();
        let rfc = datetime_to_rfc3339(dt);
        assert_eq!(rfc, "2026-07-20T12:30:45+08:00");
    }

    #[test]
    fn test_datetime_to_rfc3339_utc() {
        let dt = chrono::DateTime::parse_from_rfc3339("2026-01-15T08:00:00+00:00").unwrap();
        let rfc = datetime_to_rfc3339(dt);
        assert_eq!(rfc, "2026-01-15T08:00:00+00:00");
    }

    // ========== naive_to_rfc3339 (additional robustness) ==========

    #[test]
    fn test_naive_to_rfc3339_returns_valid_rfc3339_with_utc_offset() {
        let dt = chrono::NaiveDate::from_ymd_opt(2026, 7, 20)
            .unwrap()
            .and_hms_opt(12, 30, 45)
            .unwrap();
        let rfc = naive_to_rfc3339(dt);
        let parsed = chrono::DateTime::parse_from_rfc3339(&rfc).unwrap();
        assert_eq!(parsed.timestamp(), dt.and_utc().timestamp());
        // NaiveDateTime is interpreted as UTC, so offset must be +00:00.
        // `local_minus_utc()` is an inherent method on `FixedOffset`;
        // the redundant `.fix()` call was removed (chrono 0.4.45 dropped
        // the `Offset::fix` method that required the `Offset` trait import).
        assert_eq!(parsed.offset().local_minus_utc(), 0);
    }

    // ========== Request struct validation (Validate derive) ==========

    #[test]
    fn test_generate_request_validation_accepts_valid_input() {
        let req = GenerateRequest {
            workspace: "ws".to_string(),
            group: "g".to_string(),
            biz_tag: "tag".to_string(),
            algorithm: None,
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn test_generate_request_validation_rejects_empty_fields() {
        let req = GenerateRequest {
            workspace: String::new(),
            group: "g".to_string(),
            biz_tag: "tag".to_string(),
            algorithm: None,
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn test_batch_generate_request_validation_rejects_out_of_range_size() {
        let req = BatchGenerateRequest {
            workspace: "ws".to_string(),
            group: "g".to_string(),
            biz_tag: "tag".to_string(),
            size: Some(0),
            algorithm: None,
        };
        assert!(req.validate().is_err());

        let req2 = BatchGenerateRequest {
            workspace: "ws".to_string(),
            group: "g".to_string(),
            biz_tag: "tag".to_string(),
            size: Some(101),
            algorithm: None,
        };
        assert!(req2.validate().is_err());
    }

    #[test]
    fn test_batch_generate_request_validation_accepts_valid_size() {
        let req = BatchGenerateRequest {
            workspace: "ws".to_string(),
            group: "g".to_string(),
            biz_tag: "tag".to_string(),
            size: Some(50),
            algorithm: None,
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn test_parse_request_validation_accepts_empty_algorithm() {
        // algorithm field has #[serde(default)] and #[validate(length(min = 0, max = 32))]
        let req = ParseRequest {
            id: "123".to_string(),
            workspace: "ws".to_string(),
            group: "g".to_string(),
            biz_tag: "tag".to_string(),
            algorithm: String::new(),
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn test_parse_request_validation_rejects_empty_workspace() {
        let req = ParseRequest {
            id: "123".to_string(),
            workspace: String::new(),
            group: "g".to_string(),
            biz_tag: "tag".to_string(),
            algorithm: String::new(),
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn test_update_rate_limit_request_validation_rejects_out_of_range() {
        let req = UpdateRateLimitRequest {
            default_rps: Some(0),
            burst_size: None,
        };
        assert!(req.validate().is_err());

        let req2 = UpdateRateLimitRequest {
            default_rps: None,
            burst_size: Some(0),
        };
        assert!(req2.validate().is_err());
    }

    #[test]
    fn test_update_rate_limit_request_validation_accepts_in_range() {
        let req = UpdateRateLimitRequest {
            default_rps: Some(100),
            burst_size: Some(50),
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn test_update_logging_request_validation_rejects_empty_level() {
        let req = UpdateLoggingRequest {
            level: Some(String::new()),
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn test_update_logging_request_validation_accepts_valid_level() {
        let req = UpdateLoggingRequest {
            level: Some("info".to_string()),
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn test_set_algorithm_request_validation_rejects_empty_biz_tag() {
        let req = SetAlgorithmRequest {
            biz_tag: String::new(),
            algorithm: "segment".to_string(),
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn test_set_algorithm_request_validation_accepts_valid_input() {
        let req = SetAlgorithmRequest {
            biz_tag: "tag".to_string(),
            algorithm: "segment".to_string(),
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn test_create_workspace_request_validation_rejects_invalid_max_groups() {
        let req = CreateWorkspaceRequest {
            name: "ws".to_string(),
            description: None,
            max_groups: Some(0),
            max_biz_tags: None,
        };
        assert!(req.validate().is_err());

        let req2 = CreateWorkspaceRequest {
            name: "ws".to_string(),
            description: None,
            max_groups: None,
            max_biz_tags: Some(10001),
        };
        assert!(req2.validate().is_err());
    }

    #[test]
    fn test_create_group_request_validation_rejects_empty_workspace() {
        let req = CreateGroupRequest {
            workspace: String::new(),
            name: "g".to_string(),
            description: None,
            max_biz_tags: None,
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn test_create_api_key_request_validation_accepts_admin_without_workspace() {
        let req = CreateApiKeyRequest {
            workspace_id: None,
            name: "admin-key".to_string(),
            description: None,
            role: Some("admin".to_string()),
            rate_limit: Some(1000),
            expires_at: None,
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn test_create_api_key_request_validation_rejects_out_of_range_rate_limit() {
        let req = CreateApiKeyRequest {
            workspace_id: None,
            name: "k".to_string(),
            description: None,
            role: Some("user".to_string()),
            rate_limit: Some(50), // below min=100
            expires_at: None,
        };
        assert!(req.validate().is_err());
    }
}
