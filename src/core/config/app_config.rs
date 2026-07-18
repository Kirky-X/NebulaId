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

//! Top-level Config aggregation and loading.

use super::{
    AlgorithmConfig, AppConfig, AuthConfig, BatchGenerateConfig, ConfigError, ConfigResult,
    DatabaseConfig, EtcdConfig, LoggingConfig, LogLevel, MonitoringConfig, RateLimitConfig,
    RedisConfig, TlsConfig,
};
use serde::{Deserialize, Serialize};

/// Complete application configuration
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct Config {
    /// Application settings
    pub app: AppConfig,
    /// Database settings
    pub database: DatabaseConfig,
    /// Redis cache settings
    #[serde(default)]
    pub redis: RedisConfig,
    /// etcd settings
    pub etcd: EtcdConfig,
    /// Authentication settings
    pub auth: AuthConfig,
    /// Algorithm settings
    pub algorithm: AlgorithmConfig,
    /// Monitoring settings
    pub monitoring: MonitoringConfig,
    /// Logging settings
    pub logging: LoggingConfig,
    /// Rate limiting settings
    pub rate_limit: RateLimitConfig,
    /// TLS settings
    pub tls: TlsConfig,
    /// Batch generation settings
    pub batch_generate: BatchGenerateConfig,
}

impl Config {
    /// Load configuration from file with environment variable expansion
    /// Supports ${VAR_NAME} syntax for environment variable substitution
    pub fn load_from_file(path: &str) -> ConfigResult<Self> {
        let content =
            std::fs::read_to_string(path).map_err(|e| ConfigError::FileError(e.to_string()))?;

        let expanded = Self::expand_env_vars(&content);

        tracing::debug!(
            event = "config_expanded",
            content_len = content.len(),
            "Configuration expanded"
        );
        if let Some(auth_start) = expanded.find("[auth]") {
            let auth_section = &expanded[auth_start..(auth_start + 100).min(expanded.len())];
            tracing::debug!(event = "auth_section", auth_section = %auth_section);
        }

        let config: Config =
            toml::from_str(&expanded).map_err(|e| ConfigError::InvalidValue(e.to_string()))?;

        tracing::debug!(event = "toml_parsed", raw_auth_enabled = %format!("{:?}", config.auth.enabled), "Raw parsed auth enabled");
        tracing::debug!(event = "config_loaded", auth_enabled = %config.auth.enabled, "Auth configuration loaded");

        config.validate()?;

        Ok(config)
    }

    /// Validate configuration values are reasonable
    pub fn validate(&self) -> ConfigResult<()> {
        if self.app.http_port == 0 {
            return Err(ConfigError::InvalidValue(
                "HTTP port must be between 1 and 65535".to_string(),
            ));
        }

        if self.app.grpc_port == 0 {
            return Err(ConfigError::InvalidValue(
                "gRPC port must be between 1 and 65535".to_string(),
            ));
        }

        if self.app.dc_id > 31 {
            return Err(ConfigError::InvalidValue(
                "Datacenter ID must be between 0 and 31".to_string(),
            ));
        }

        if self.database.max_connections == 0 {
            return Err(ConfigError::InvalidValue(
                "Database max_connections must be greater than 0".to_string(),
            ));
        }

        if self.database.min_connections > self.database.max_connections {
            return Err(ConfigError::InvalidValue(
                "Database min_connections cannot be greater than max_connections".to_string(),
            ));
        }

        if self.database.acquire_timeout_seconds == 0 {
            return Err(ConfigError::InvalidValue(
                "Database acquire_timeout_seconds must be greater than 0".to_string(),
            ));
        }

        if self.rate_limit.enabled {
            if self.rate_limit.default_rps == 0 {
                return Err(ConfigError::InvalidValue(
                    "Rate limit default_rps must be greater than 0 when enabled".to_string(),
                ));
            }

            if self.rate_limit.burst_size == 0 {
                return Err(ConfigError::InvalidValue(
                    "Rate limit burst_size must be greater than 0 when enabled".to_string(),
                ));
            }

            if self.rate_limit.burst_size > self.rate_limit.default_rps * 10 {
                return Err(ConfigError::InvalidValue(
                    "Rate limit burst_size should not exceed 10x default_rps".to_string(),
                ));
            }
        }

        if !["segment", "snowflake", "uuid_v7", "uuid_v4"]
            .contains(&self.algorithm.default.as_str())
        {
            return Err(ConfigError::InvalidValue(
                "Default algorithm must be one of: segment, snowflake, uuid_v7, uuid_v4"
                    .to_string(),
            ));
        }

        if self.algorithm.segment.min_step > self.algorithm.segment.max_step {
            return Err(ConfigError::InvalidValue(
                "Segment min_step cannot be greater than max_step".to_string(),
            ));
        }

        if self.algorithm.segment.base_step < self.algorithm.segment.min_step
            || self.algorithm.segment.base_step > self.algorithm.segment.max_step
        {
            return Err(ConfigError::InvalidValue(
                "Segment base_step must be between min_step and max_step".to_string(),
            ));
        }

        if self.algorithm.segment.switch_threshold < 0.0
            || self.algorithm.segment.switch_threshold > 1.0
        {
            return Err(ConfigError::InvalidValue(
                "Segment switch_threshold must be between 0.0 and 1.0".to_string(),
            ));
        }

        let total_bits = self.algorithm.snowflake.datacenter_id_bits
            + self.algorithm.snowflake.worker_id_bits
            + self.algorithm.snowflake.sequence_bits;

        if total_bits >= 64 {
            return Err(ConfigError::InvalidValue(
                "Snowflake total bits (datacenter_id_bits + worker_id_bits + sequence_bits) must be less than 64".to_string(),
            ));
        }

        if self.algorithm.snowflake.clock_drift_threshold_ms == 0 {
            return Err(ConfigError::InvalidValue(
                "Snowflake clock_drift_threshold_ms must be greater than 0".to_string(),
            ));
        }

        if self.batch_generate.max_batch_size == 0 {
            return Err(ConfigError::InvalidValue(
                "Batch generate max_batch_size must be greater than 0".to_string(),
            ));
        }

        if self.batch_generate.max_batch_size > 10000 {
            return Err(ConfigError::InvalidValue(
                "Batch generate max_batch_size should not exceed 10000".to_string(),
            ));
        }

        Ok(())
    }

    /// Expand environment variables in config content
    /// Pattern: ${VAR_NAME} -> value of VAR_NAME
    fn expand_env_vars(content: &str) -> String {
        static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
        let re = RE.get_or_init(|| {
            regex::Regex::new(r"\$\{([^}]+)\}")
                .expect("BUG: Hardcoded regex pattern should never fail")
        });
        re.replace_all(content, |caps: &regex::Captures| {
            let var_name = &caps[1];
            std::env::var(var_name).unwrap_or_else(|_| caps[0].to_string())
        })
        .to_string()
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
        if let Ok(worker_id) = std::env::var("WORKER_ID") {
            config.app.worker_id = worker_id
                .parse()
                .map_err(|_| ConfigError::InvalidValue("WORKER_ID".to_string()))?;
        }

        if let Ok(url) = std::env::var("DATABASE_URL") {
            config.database.url = url;
        }

        if let Ok(endpoints) = std::env::var("ETCD_ENDPOINTS") {
            config.etcd.endpoints = endpoints.split(',').map(String::from).collect();
        }

        if let Ok(level) = std::env::var("RUST_LOG") {
            config.logging.level = LogLevel::from(level);
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

        if !other.database.url.is_empty() && other.database.url != self.database.url {
            self.database.url = other.database.url;
        }
        if other.database.max_connections != 100 {
            self.database.max_connections = other.database.max_connections;
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

        if other.logging.level != LogLevel::Info {
            self.logging.level = other.logging.level;
        }
    }
}
