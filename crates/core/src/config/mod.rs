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

use crate::types::AlgorithmType;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ConfigError {
    #[error("Missing required configuration: {}", _0)]
    MissingRequired(String),

    #[error("Invalid configuration value: {}", _0)]
    InvalidValue(String),

    #[error("Configuration file error: {}", _0)]
    FileError(String),
}

pub type ConfigResult<T> = std::result::Result<T, ConfigError>;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AppConfig {
    pub name: String,
    pub host: String,
    pub http_port: u16,
    pub grpc_port: u16,
    pub dc_id: u8,
    pub worker_id: u8,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            name: "nebula-id".to_string(),
            host: "0.0.0.0".to_string(),
            http_port: 8080,
            grpc_port: 9091,
            dc_id: 0,
            worker_id: 0,
        }
    }
}

impl AppConfig {
    pub fn http_addr(&self) -> SocketAddr {
        format!("{}:{}", self.host, self.http_port).parse().unwrap()
    }

    pub fn grpc_addr(&self) -> SocketAddr {
        format!("{}:{}", self.host, self.grpc_port).parse().unwrap()
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DatabaseConfig {
    pub engine: String,
    pub url: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub database: String,
    pub max_connections: u32,
    pub min_connections: u32,
    pub acquire_timeout_seconds: u64,
    pub idle_timeout_seconds: u64,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            engine: "postgresql".to_string(),
            url: std::env::var("NEBULA_DATABASE_URL").unwrap_or_else(|_| {
                "postgresql://idgen:CHANGE_ME@localhost:5432/idgen".to_string()
            }),
            host: "localhost".to_string(),
            port: 5432,
            username: "idgen".to_string(),
            password: std::env::var("NEBULA_DATABASE_PASSWORD")
                .unwrap_or_else(|_| "CHANGE_ME".to_string()),
            database: "idgen".to_string(),
            max_connections: 1200, // Increased to support 1M QPS target
            min_connections: 100,  // Increased to maintain warm connections
            acquire_timeout_seconds: 5,
            idle_timeout_seconds: 300,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RedisConfig {
    pub url: String,
    pub pool_size: u32,
    pub key_prefix: String,
    pub ttl_seconds: u64,
}

impl Default for RedisConfig {
    fn default() -> Self {
        Self {
            url: "redis://localhost:6379".to_string(),
            pool_size: 50,
            key_prefix: "nebula:id:".to_string(),
            ttl_seconds: 600,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EtcdConfig {
    pub endpoints: Vec<String>,
    pub connect_timeout_ms: u64,
    pub watch_timeout_ms: u64,
}

impl Default for EtcdConfig {
    fn default() -> Self {
        Self {
            endpoints: vec!["etcd:2379".to_string()],
            connect_timeout_ms: 5000,
            watch_timeout_ms: 5000,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ApiKeyEntry {
    pub key: String,
    pub workspace: String,
    pub rate_limit: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AuthConfig {
    pub enabled: bool,
    pub cache_ttl_seconds: u64,
    pub api_keys: Vec<ApiKeyEntry>,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            cache_ttl_seconds: 300,
            api_keys: vec![], // Empty by default, should be configured via environment
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SegmentAlgorithmConfig {
    pub base_step: u64,
    pub min_step: u64,
    pub max_step: u64,
    pub switch_threshold: f64,
}

impl Default for SegmentAlgorithmConfig {
    fn default() -> Self {
        Self {
            base_step: 1000,
            min_step: 500,
            max_step: 100000,
            switch_threshold: 0.1,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SnowflakeAlgorithmConfig {
    pub datacenter_id_bits: u8,
    pub worker_id_bits: u8,
    pub sequence_bits: u8,
    pub clock_drift_threshold_ms: u64,
}

impl SnowflakeAlgorithmConfig {
    pub fn datacenter_id_mask(&self) -> u64 {
        (1 << self.datacenter_id_bits) - 1
    }

    pub fn worker_id_mask(&self) -> u64 {
        (1 << self.worker_id_bits) - 1
    }

    pub fn sequence_mask(&self) -> u64 {
        (1 << self.sequence_bits) - 1
    }

    pub fn timestamp_bits(&self) -> u8 {
        64 - self.datacenter_id_bits - self.worker_id_bits - self.sequence_bits
    }
}

impl Default for SnowflakeAlgorithmConfig {
    fn default() -> Self {
        Self {
            datacenter_id_bits: 3,
            worker_id_bits: 8,
            sequence_bits: 10,
            clock_drift_threshold_ms: 1000,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UuidV7Config {
    pub enabled: bool,
}

impl Default for UuidV7Config {
    fn default() -> Self {
        Self { enabled: true }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AlgorithmConfig {
    pub default: String,
    pub segment: SegmentAlgorithmConfig,
    pub snowflake: SnowflakeAlgorithmConfig,
    pub uuid_v7: UuidV7Config,
}

impl Default for AlgorithmConfig {
    fn default() -> Self {
        Self {
            default: "segment".to_string(),
            segment: SegmentAlgorithmConfig::default(),
            snowflake: SnowflakeAlgorithmConfig::default(),
            uuid_v7: UuidV7Config::default(),
        }
    }
}

impl AlgorithmConfig {
    pub fn get_default_algorithm(&self) -> AlgorithmType {
        self.default.parse().unwrap_or(AlgorithmType::Segment)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MonitoringConfig {
    pub metrics_enabled: bool,
    pub metrics_path: String,
    pub tracing_enabled: bool,
    pub otlp_endpoint: String,
}

impl Default for MonitoringConfig {
    fn default() -> Self {
        Self {
            metrics_enabled: true,
            metrics_path: "/metrics".to_string(),
            tracing_enabled: false,
            otlp_endpoint: "".to_string(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LoggingConfig {
    pub level: String,
    pub format: String,
    pub include_location: bool,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: "info".to_string(),
            format: "json".to_string(),
            include_location: true,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RateLimitConfig {
    pub enabled: bool,
    pub default_rps: u32,
    pub burst_size: u32,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            default_rps: 10000,
            burst_size: 100,
        }
    }
}

/// TLS 版本配置
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum TlsVersion {
    /// TLS 1.2
    Tls12,
    /// TLS 1.3 (推荐)
    #[default]
    Tls13,
}

impl std::fmt::Display for TlsVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TlsVersion::Tls12 => write!(f, "TLSv1.2"),
            TlsVersion::Tls13 => write!(f, "TLSv1.3"),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TlsConfig {
    pub enabled: bool,
    pub cert_path: String,
    pub key_path: String,
    pub ca_path: Option<String>,
    pub http_enabled: bool,
    pub grpc_enabled: bool,
    /// 最低 TLS 版本 (默认: TLS 1.3)
    pub min_tls_version: TlsVersion,
    /// ALPN 协议列表，用于 HTTP/2 支持
    pub alpn_protocols: Vec<String>,
}

impl Default for TlsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            cert_path: "".to_string(),
            key_path: "".to_string(),
            ca_path: None,
            http_enabled: false,
            grpc_enabled: false,
            min_tls_version: TlsVersion::Tls13,
            alpn_protocols: vec!["h2".to_string(), "http/1.1".to_string()],
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct Config {
    pub app: AppConfig,
    pub database: DatabaseConfig,
    pub redis: RedisConfig,
    pub etcd: EtcdConfig,
    pub auth: AuthConfig,
    pub algorithm: AlgorithmConfig,
    pub monitoring: MonitoringConfig,
    pub logging: LoggingConfig,
    pub rate_limit: RateLimitConfig,
    pub tls: TlsConfig,
}

impl Config {
    pub fn load_from_file(path: &str) -> ConfigResult<Self> {
        let content =
            std::fs::read_to_string(path).map_err(|e| ConfigError::FileError(e.to_string()))?;

        toml::from_str(&content).map_err(|e| ConfigError::InvalidValue(e.to_string()))
    }

    pub fn load_from_env() -> ConfigResult<Self> {
        let mut config = Config::default();

        if let Ok(host) = std::env::var("APP_HOST") {
            config.app.host = host;
        }
        if let Ok(port) = std::env::var("APP_HTTP_PORT") {
            config.app.http_port = port
                .parse()
                .map_err(|_| ConfigError::InvalidValue("APP_HTTP_PORT".to_string()))?;
        }
        if let Ok(port) = std::env::var("APP_GRPC_PORT") {
            config.app.grpc_port = port
                .parse()
                .map_err(|_| ConfigError::InvalidValue("APP_GRPC_PORT".to_string()))?;
        }
        if let Ok(dc_id) = std::env::var("DC_ID") {
            config.app.dc_id = dc_id
                .parse()
                .map_err(|_| ConfigError::InvalidValue("DC_ID".to_string()))?;
        }

        if let Ok(url) = std::env::var("DATABASE_URL") {
            config.database.url = url;
        }

        if let Ok(url) = std::env::var("REDIS_URL") {
            config.redis.url = url;
        }

        if let Ok(endpoints) = std::env::var("ETCD_ENDPOINTS") {
            config.etcd.endpoints = endpoints.split(',').map(String::from).collect();
        }

        if let Ok(level) = std::env::var("RUST_LOG") {
            config.logging.level = level;
        }

        Ok(config)
    }

    pub fn merge(&mut self, other: Config) {
        if other.app.host != "0.0.0.0" {
            self.app.host = other.app.host;
        }
        if other.app.http_port != 8080 {
            self.app.http_port = other.app.http_port;
        }
        if other.app.grpc_port != 9091 {
            self.app.grpc_port = other.app.grpc_port;
        }
        if other.app.dc_id != 0 {
            self.app.dc_id = other.app.dc_id;
        }
        if other.app.worker_id != 0 {
            self.app.worker_id = other.app.worker_id;
        }

        if !other.database.url.contains("localhost") {
            self.database.url = other.database.url;
        }
        if other.database.max_connections != 100 {
            self.database.max_connections = other.database.max_connections;
        }

        if !other.redis.url.contains("localhost") {
            self.redis.url = other.redis.url;
        }
        if other.redis.pool_size != 50 {
            self.redis.pool_size = other.redis.pool_size;
        }

        if !other.etcd.endpoints.is_empty() {
            self.etcd.endpoints = other.etcd.endpoints;
        }

        if !other.auth.api_keys.is_empty() {
            self.auth.api_keys = other.auth.api_keys;
        }

        if other.algorithm.default != "segment" {
            self.algorithm.default = other.algorithm.default;
        }
        self.algorithm.segment = other.algorithm.segment;
        self.algorithm.snowflake = other.algorithm.snowflake;
        self.algorithm.uuid_v7 = other.algorithm.uuid_v7;

        if other.monitoring.metrics_path != "/metrics" {
            self.monitoring.metrics_path = other.monitoring.metrics_path;
        }
        if other.monitoring.tracing_enabled {
            self.monitoring.tracing_enabled = true;
        }
        if !other.monitoring.otlp_endpoint.is_empty() {
            self.monitoring.otlp_endpoint = other.monitoring.otlp_endpoint;
        }

        if other.logging.level != "info" {
            self.logging.level = other.logging.level;
        }
    }
}
