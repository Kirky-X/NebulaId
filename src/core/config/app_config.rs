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
    DatabaseConfig, EtcdConfig, LogLevel, LoggingConfig, MonitoringConfig, RateLimitConfig,
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
            "{}",
            t!("log.core.config.app_config.config_expanded")
        );
        if let Some(auth_start) = expanded.find("[auth]") {
            let auth_section = &expanded[auth_start..(auth_start + 100).min(expanded.len())];
            tracing::debug!(event = "auth_section", auth_section = %auth_section);
        }

        let config: Config =
            toml::from_str(&expanded).map_err(|e| ConfigError::InvalidValue(e.to_string()))?;

        tracing::debug!(event = "toml_parsed", raw_auth_enabled = %format!("{:?}", config.auth.enabled), "{}", t!("log.core.config.app_config.toml_parsed"));
        tracing::debug!(event = "config_loaded", auth_enabled = %config.auth.enabled, "{}", t!("log.core.config.app_config.config_loaded"));

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::ApiKeyEntry;
    use std::sync::Mutex;

    /// 串行化所有涉及环境变量的测试，避免并行测试污染
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// 环境变量 RAII 守卫：在作用域内修改，离开时恢复原始值
    struct VarGuard {
        key: &'static str,
        original: Option<String>,
    }

    impl VarGuard {
        /// 设置环境变量并记录原始值
        fn set(key: &'static str, value: &str) -> Self {
            let original = std::env::var(key).ok();
            std::env::set_var(key, value);
            Self { key, original }
        }

        /// 删除环境变量并记录原始值
        fn remove(key: &'static str) -> Self {
            let original = std::env::var(key).ok();
            std::env::remove_var(key);
            Self { key, original }
        }
    }

    impl Drop for VarGuard {
        fn drop(&mut self) {
            match &self.original {
                Some(v) => std::env::set_var(self.key, v),
                None => std::env::remove_var(self.key),
            }
        }
    }

    /// 断言 validate 返回 InvalidValue 错误，且消息包含指定子串
    fn assert_invalid_value(result: ConfigResult<()>, expected_substring: &str) {
        match result {
            Err(ConfigError::InvalidValue(msg)) => {
                assert!(
                    msg.contains(expected_substring),
                    "错误消息应包含 '{}', 实际为: {}",
                    expected_substring,
                    msg
                );
            }
            other => panic!("期望 InvalidValue 错误, 实际为: {:?}", other),
        }
    }

    // ==================== load_from_file 测试 ====================

    /// 从有效 TOML 文件加载配置应成功，并保留字段值
    #[test]
    fn load_from_file_valid_config_succeeds() {
        let original = Config::default();
        let toml_content = toml::to_string(&original).expect("序列化 Config 应成功");

        let temp = tempfile::NamedTempFile::new().expect("创建临时文件应成功");
        std::fs::write(temp.path(), &toml_content).expect("写入临时文件应成功");

        let loaded = Config::load_from_file(temp.path().to_str().unwrap()).unwrap();
        assert_eq!(loaded.app.host, original.app.host);
        assert_eq!(loaded.app.http_port, original.app.http_port);
        assert_eq!(loaded.app.grpc_port, original.app.grpc_port);
        assert_eq!(loaded.app.dc_id, original.app.dc_id);
        assert_eq!(loaded.algorithm.default, original.algorithm.default);
        assert_eq!(
            loaded.database.max_connections,
            original.database.max_connections
        );
    }

    /// 文件不存在时应返回 FileError
    #[test]
    fn load_from_file_missing_path_returns_file_error() {
        let result = Config::load_from_file("/nonexistent/path/no/such/file.toml");
        assert!(matches!(result, Err(ConfigError::FileError(_))));
    }

    /// TOML 解析错误应返回 InvalidValue
    #[test]
    fn load_from_file_invalid_toml_returns_invalid_value() {
        let temp = tempfile::NamedTempFile::new().expect("创建临时文件应成功");
        std::fs::write(temp.path(), "this is = = invalid toml [[").unwrap();
        let result = Config::load_from_file(temp.path().to_str().unwrap());
        assert!(matches!(result, Err(ConfigError::InvalidValue(_))));
    }

    /// 配置校验失败时应返回 InvalidValue
    #[test]
    fn load_from_file_validation_failure_returns_invalid_value() {
        let mut invalid = Config::default();
        invalid.app.http_port = 0;
        let toml_content = toml::to_string(&invalid).expect("序列化 Config 应成功");

        let temp = tempfile::NamedTempFile::new().expect("创建临时文件应成功");
        std::fs::write(temp.path(), &toml_content).unwrap();

        let result = Config::load_from_file(temp.path().to_str().unwrap());
        assert!(matches!(result, Err(ConfigError::InvalidValue(_))));
    }

    /// 配置文件中的 ${VAR} 占位符应被环境变量值替换
    #[test]
    fn load_from_file_expands_env_vars() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _v = VarGuard::set("NEBULA_TEST_EXPAND_VAR", "postgres://expanded:5432/db");

        let original = Config::default();
        let mut toml_content = toml::to_string(&original).expect("序列化 Config 应成功");
        let original_url_line = format!("url = \"{}\"", original.database.url);
        assert!(
            toml_content.contains(&original_url_line),
            "TOML 内容应包含 database.url 行"
        );
        toml_content =
            toml_content.replace(&original_url_line, "url = \"${NEBULA_TEST_EXPAND_VAR}\"");

        let temp = tempfile::NamedTempFile::new().expect("创建临时文件应成功");
        std::fs::write(temp.path(), &toml_content).unwrap();

        let loaded = Config::load_from_file(temp.path().to_str().unwrap()).unwrap();
        assert_eq!(loaded.database.url, "postgres://expanded:5432/db");
    }

    // ==================== validate 测试 ====================

    /// 默认配置应通过校验
    #[test]
    fn validate_default_config_passes() {
        let config = Config::default();
        assert!(config.validate().is_ok());
    }

    /// http_port=0 时校验失败
    #[test]
    fn validate_http_port_zero_fails() {
        let mut config = Config::default();
        config.app.http_port = 0;
        assert_invalid_value(config.validate(), "HTTP port must be between 1 and 65535");
    }

    /// grpc_port=0 时校验失败
    #[test]
    fn validate_grpc_port_zero_fails() {
        let mut config = Config::default();
        config.app.grpc_port = 0;
        assert_invalid_value(config.validate(), "gRPC port must be between 1 and 65535");
    }

    /// dc_id>31 时校验失败
    #[test]
    fn validate_dc_id_over_31_fails() {
        let mut config = Config::default();
        config.app.dc_id = 32;
        assert_invalid_value(config.validate(), "Datacenter ID must be between 0 and 31");
    }

    /// database.max_connections=0 时校验失败
    #[test]
    fn validate_database_max_connections_zero_fails() {
        let mut config = Config::default();
        config.database.max_connections = 0;
        assert_invalid_value(
            config.validate(),
            "Database max_connections must be greater than 0",
        );
    }

    /// database.min_connections > max_connections 时校验失败
    #[test]
    fn validate_min_connections_greater_than_max_fails() {
        let mut config = Config::default();
        config.database.min_connections = 100;
        config.database.max_connections = 50;
        assert_invalid_value(
            config.validate(),
            "Database min_connections cannot be greater than max_connections",
        );
    }

    /// database.acquire_timeout_seconds=0 时校验失败
    #[test]
    fn validate_acquire_timeout_zero_fails() {
        let mut config = Config::default();
        config.database.acquire_timeout_seconds = 0;
        assert_invalid_value(
            config.validate(),
            "Database acquire_timeout_seconds must be greater than 0",
        );
    }

    /// rate_limit 启用且 default_rps=0 时校验失败
    #[test]
    fn validate_rate_limit_enabled_default_rps_zero_fails() {
        let mut config = Config::default();
        config.rate_limit.enabled = true;
        config.rate_limit.default_rps = 0;
        assert_invalid_value(
            config.validate(),
            "Rate limit default_rps must be greater than 0 when enabled",
        );
    }

    /// rate_limit 启用且 burst_size=0 时校验失败
    #[test]
    fn validate_rate_limit_enabled_burst_size_zero_fails() {
        let mut config = Config::default();
        config.rate_limit.enabled = true;
        config.rate_limit.default_rps = 100;
        config.rate_limit.burst_size = 0;
        assert_invalid_value(
            config.validate(),
            "Rate limit burst_size must be greater than 0 when enabled",
        );
    }

    /// rate_limit 启用且 burst_size > 10x default_rps 时校验失败
    #[test]
    fn validate_rate_limit_burst_size_exceeds_10x_default_rps_fails() {
        let mut config = Config::default();
        config.rate_limit.enabled = true;
        config.rate_limit.default_rps = 10;
        config.rate_limit.burst_size = 101; // 10 * 10 = 100, 101 > 100
        assert_invalid_value(
            config.validate(),
            "Rate limit burst_size should not exceed 10x default_rps",
        );
    }

    /// rate_limit 禁用时跳过 default_rps 和 burst_size 校验
    #[test]
    fn validate_rate_limit_disabled_skips_rate_checks() {
        let mut config = Config::default();
        config.rate_limit.enabled = false;
        config.rate_limit.default_rps = 0;
        config.rate_limit.burst_size = 0;
        assert!(config.validate().is_ok());
    }

    /// algorithm.default 无效时校验失败
    #[test]
    fn validate_algorithm_default_invalid_fails() {
        let mut config = Config::default();
        config.algorithm.default = "invalid_algo".to_string();
        assert_invalid_value(
            config.validate(),
            "Default algorithm must be one of: segment, snowflake, uuid_v7, uuid_v4",
        );
    }

    /// segment.min_step > max_step 时校验失败
    #[test]
    fn validate_segment_min_step_greater_than_max_step_fails() {
        let mut config = Config::default();
        config.algorithm.segment.min_step = 200000;
        config.algorithm.segment.max_step = 100000;
        assert_invalid_value(
            config.validate(),
            "Segment min_step cannot be greater than max_step",
        );
    }

    /// segment.base_step < min_step 时校验失败
    #[test]
    fn validate_segment_base_step_below_min_fails() {
        let mut config = Config::default();
        config.algorithm.segment.base_step = 100;
        config.algorithm.segment.min_step = 500;
        assert_invalid_value(
            config.validate(),
            "Segment base_step must be between min_step and max_step",
        );
    }

    /// segment.base_step > max_step 时校验失败
    #[test]
    fn validate_segment_base_step_above_max_fails() {
        let mut config = Config::default();
        config.algorithm.segment.base_step = 200000;
        config.algorithm.segment.max_step = 100000;
        assert_invalid_value(
            config.validate(),
            "Segment base_step must be between min_step and max_step",
        );
    }

    /// segment.switch_threshold < 0 时校验失败
    #[test]
    fn validate_segment_switch_threshold_negative_fails() {
        let mut config = Config::default();
        config.algorithm.segment.switch_threshold = -0.1;
        assert_invalid_value(
            config.validate(),
            "Segment switch_threshold must be between 0.0 and 1.0",
        );
    }

    /// segment.switch_threshold > 1 时校验失败
    #[test]
    fn validate_segment_switch_threshold_above_one_fails() {
        let mut config = Config::default();
        config.algorithm.segment.switch_threshold = 1.5;
        assert_invalid_value(
            config.validate(),
            "Segment switch_threshold must be between 0.0 and 1.0",
        );
    }

    /// snowflake 各位之和 >= 64 时校验失败
    #[test]
    fn validate_snowflake_total_bits_over_64_fails() {
        let mut config = Config::default();
        config.algorithm.snowflake.datacenter_id_bits = 32;
        config.algorithm.snowflake.worker_id_bits = 16;
        config.algorithm.snowflake.sequence_bits = 16; // 32 + 16 + 16 = 64
        assert_invalid_value(config.validate(), "Snowflake total bits");
    }

    /// snowflake.clock_drift_threshold_ms=0 时校验失败
    #[test]
    fn validate_snowflake_clock_drift_zero_fails() {
        let mut config = Config::default();
        config.algorithm.snowflake.clock_drift_threshold_ms = 0;
        assert_invalid_value(
            config.validate(),
            "Snowflake clock_drift_threshold_ms must be greater than 0",
        );
    }

    /// batch_generate.max_batch_size=0 时校验失败
    #[test]
    fn validate_batch_max_size_zero_fails() {
        let mut config = Config::default();
        config.batch_generate.max_batch_size = 0;
        assert_invalid_value(
            config.validate(),
            "Batch generate max_batch_size must be greater than 0",
        );
    }

    /// batch_generate.max_batch_size > 10000 时校验失败
    #[test]
    fn validate_batch_max_size_above_10000_fails() {
        let mut config = Config::default();
        config.batch_generate.max_batch_size = 10001;
        assert_invalid_value(
            config.validate(),
            "Batch generate max_batch_size should not exceed 10000",
        );
    }

    // ==================== expand_env_vars 测试 ====================

    /// 存在的环境变量应被替换为对应值
    #[test]
    fn expand_env_vars_replaces_existing_var() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _v = VarGuard::set("NEBULA_TEST_EXPAND_EXISTING", "hello");
        let result = Config::expand_env_vars("value=${NEBULA_TEST_EXPAND_EXISTING}");
        assert_eq!(result, "value=hello");
    }

    /// 不存在的环境变量应保留原占位符文本
    #[test]
    fn expand_env_vars_preserves_missing_var() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _v = VarGuard::remove("NEBULA_TEST_EXPAND_MISSING");
        let result = Config::expand_env_vars("value=${NEBULA_TEST_EXPAND_MISSING}");
        assert_eq!(result, "value=${NEBULA_TEST_EXPAND_MISSING}");
    }

    /// 多个环境变量应同时被替换
    #[test]
    fn expand_env_vars_replaces_multiple_vars() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _v1 = VarGuard::set("NEBULA_TEST_MULTI_A", "foo");
        let _v2 = VarGuard::set("NEBULA_TEST_MULTI_B", "bar");
        let result = Config::expand_env_vars("${NEBULA_TEST_MULTI_A}-${NEBULA_TEST_MULTI_B}");
        assert_eq!(result, "foo-bar");
    }

    /// 无占位符的内容应保持不变
    #[test]
    fn expand_env_vars_no_vars_unchanged() {
        let result = Config::expand_env_vars("plain text without vars");
        assert_eq!(result, "plain text without vars");
    }

    // ==================== load_from_env 测试 ====================

    /// 无环境变量时返回默认配置
    #[test]
    fn load_from_env_no_vars_returns_default() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _g1 = VarGuard::remove("APP_HOST");
        let _g2 = VarGuard::remove("APP_HTTP_PORT");
        let _g3 = VarGuard::remove("APP_GRPC_PORT");
        let _g4 = VarGuard::remove("DC_ID");
        let _g5 = VarGuard::remove("WORKER_ID");
        let _g6 = VarGuard::remove("DATABASE_URL");
        let _g7 = VarGuard::remove("ETCD_ENDPOINTS");
        let _g8 = VarGuard::remove("RUST_LOG");

        let config = Config::load_from_env().unwrap();
        assert_eq!(config.app.host, "0.0.0.0");
        assert_eq!(config.app.http_port, 8080);
        assert_eq!(config.app.grpc_port, 9091);
        assert_eq!(config.app.dc_id, 0);
        assert_eq!(config.app.worker_id, 0);
    }

    /// APP_HOST 环境变量应被加载到 app.host
    #[test]
    fn load_from_env_app_host() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _g = VarGuard::set("APP_HOST", "192.168.1.1");
        let config = Config::load_from_env().unwrap();
        assert_eq!(config.app.host, "192.168.1.1");
    }

    /// APP_HTTP_PORT 有效值应被加载
    #[test]
    fn load_from_env_app_http_port_valid() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _g = VarGuard::set("APP_HTTP_PORT", "9000");
        let config = Config::load_from_env().unwrap();
        assert_eq!(config.app.http_port, 9000);
    }

    /// APP_HTTP_PORT 无效值应返回 InvalidValue 错误
    #[test]
    fn load_from_env_app_http_port_invalid_fails() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _g = VarGuard::set("APP_HTTP_PORT", "not-a-number");
        let result = Config::load_from_env();
        assert_eq!(
            result.unwrap_err(),
            ConfigError::InvalidValue("APP_HTTP_PORT".to_string())
        );
    }

    /// APP_GRPC_PORT 有效值应被加载
    #[test]
    fn load_from_env_app_grpc_port_valid() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _g = VarGuard::set("APP_GRPC_PORT", "9092");
        let config = Config::load_from_env().unwrap();
        assert_eq!(config.app.grpc_port, 9092);
    }

    /// APP_GRPC_PORT 无效值应返回 InvalidValue 错误
    #[test]
    fn load_from_env_app_grpc_port_invalid_fails() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _g = VarGuard::set("APP_GRPC_PORT", "abc");
        let result = Config::load_from_env();
        assert_eq!(
            result.unwrap_err(),
            ConfigError::InvalidValue("APP_GRPC_PORT".to_string())
        );
    }

    /// DC_ID 有效值应被加载
    #[test]
    fn load_from_env_dc_id_valid() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _g = VarGuard::set("DC_ID", "15");
        let config = Config::load_from_env().unwrap();
        assert_eq!(config.app.dc_id, 15);
    }

    /// DC_ID 无效值应返回 InvalidValue 错误
    #[test]
    fn load_from_env_dc_id_invalid_fails() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _g = VarGuard::set("DC_ID", "not-a-number");
        let result = Config::load_from_env();
        assert_eq!(
            result.unwrap_err(),
            ConfigError::InvalidValue("DC_ID".to_string())
        );
    }

    /// WORKER_ID 有效值应被加载
    #[test]
    fn load_from_env_worker_id_valid() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _g = VarGuard::set("WORKER_ID", "200");
        let config = Config::load_from_env().unwrap();
        assert_eq!(config.app.worker_id, 200);
    }

    /// WORKER_ID 无效值应返回 InvalidValue 错误
    #[test]
    fn load_from_env_worker_id_invalid_fails() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _g = VarGuard::set("WORKER_ID", "xyz");
        let result = Config::load_from_env();
        assert_eq!(
            result.unwrap_err(),
            ConfigError::InvalidValue("WORKER_ID".to_string())
        );
    }

    /// DATABASE_URL 环境变量应被加载到 database.url
    #[test]
    fn load_from_env_database_url() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _g = VarGuard::set("DATABASE_URL", "postgres://test:5432/testdb");
        let config = Config::load_from_env().unwrap();
        assert_eq!(config.database.url, "postgres://test:5432/testdb");
    }

    /// ETCD_ENDPOINTS 环境变量应按逗号拆分加载
    #[test]
    fn load_from_env_etcd_endpoints() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _g = VarGuard::set("ETCD_ENDPOINTS", "etcd1:2379,etcd2:2379,etcd3:2379");
        let config = Config::load_from_env().unwrap();
        assert_eq!(
            config.etcd.endpoints,
            vec![
                "etcd1:2379".to_string(),
                "etcd2:2379".to_string(),
                "etcd3:2379".to_string()
            ]
        );
    }

    /// RUST_LOG 环境变量应被加载为对应日志级别
    #[test]
    fn load_from_env_rust_log() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _g = VarGuard::set("RUST_LOG", "debug");
        let config = Config::load_from_env().unwrap();
        assert_eq!(config.logging.level, LogLevel::Debug);
    }

    // ==================== merge 测试 ====================

    /// 自定义 other 应覆盖 base 的所有可覆盖字段（覆盖各 if 的 true 分支）
    ///
    /// 加 ENV_LOCK：`Config::default()` 内部读取 `DATABASE_URL` 环境变量，
    /// 与 `load_from_env_*` 测试并行运行时会因 env var 时序导致 panic。
    #[test]
    fn merge_with_custom_other_overrides_all_fields() {
        let _guard = ENV_LOCK.lock().unwrap();
        let mut base = Config::default();
        let mut other = Config::default();
        other.app.host = "1.2.3.4".to_string();
        other.app.http_port = 7000;
        other.app.grpc_port = 8000;
        other.app.dc_id = 7;
        other.app.worker_id = 14;
        other.database.url = "postgres://custom-host:5432/custom_db".to_string();
        other.database.max_connections = 200;
        other.etcd.endpoints = vec!["etcd1:2379".to_string(), "etcd2:2379".to_string()];
        let api_key = ApiKeyEntry {
            key_id: "k1".to_string(),
            key_secret: "s1".to_string(),
            workspace: "w1".to_string(),
            role: "admin".to_string(),
            rate_limit: 100,
            name: "n1".to_string(),
        };
        other.auth.api_keys = vec![api_key.clone()];
        other.algorithm.default = "snowflake".to_string();
        other.algorithm.segment.base_step = 9999;
        other.algorithm.snowflake.sequence_bits = 12;
        other.algorithm.uuid_v7.enabled = false;
        other.monitoring.metrics_path = "/custom_metrics".to_string();
        other.monitoring.tracing_enabled = true;
        other.monitoring.otlp_endpoint = "http://otlp:4317".to_string();
        other.logging.level = LogLevel::Debug;

        base.merge(other);

        assert_eq!(base.app.host, "1.2.3.4");
        assert_eq!(base.app.http_port, 7000);
        assert_eq!(base.app.grpc_port, 8000);
        assert_eq!(base.app.dc_id, 7);
        assert_eq!(base.app.worker_id, 14);
        assert_eq!(base.database.url, "postgres://custom-host:5432/custom_db");
        assert_eq!(base.database.max_connections, 200);
        assert_eq!(
            base.etcd.endpoints,
            vec!["etcd1:2379".to_string(), "etcd2:2379".to_string()]
        );
        assert_eq!(base.auth.api_keys.len(), 1);
        assert_eq!(base.auth.api_keys[0].key_id, "k1");
        assert_eq!(base.algorithm.default, "snowflake");
        assert_eq!(base.algorithm.segment.base_step, 9999);
        assert_eq!(base.algorithm.snowflake.sequence_bits, 12);
        assert!(!base.algorithm.uuid_v7.enabled);
        assert_eq!(base.monitoring.metrics_path, "/custom_metrics");
        assert!(base.monitoring.tracing_enabled);
        assert_eq!(base.monitoring.otlp_endpoint, "http://otlp:4317");
        assert_eq!(base.logging.level, LogLevel::Debug);
    }

    /// 默认 other 不应覆盖 base 的自定义字段（覆盖各 if 的 false 分支）
    ///
    /// 加 ENV_LOCK：`Config::default()` 内部读取 `DATABASE_URL` 环境变量，
    /// 与 `load_from_env_*` 测试并行运行时会因 env var 时序导致 panic。
    #[test]
    fn merge_with_default_other_preserves_base_customizations() {
        let _guard = ENV_LOCK.lock().unwrap();
        let mut base = Config::default();
        base.app.host = "custom.host".to_string();
        base.app.http_port = 7777;
        base.app.grpc_port = 8888;
        base.app.dc_id = 5;
        base.app.worker_id = 10;
        base.database.max_connections = 50;
        base.algorithm.default = "uuid_v7".to_string();
        base.monitoring.metrics_path = "/custom".to_string();
        base.monitoring.tracing_enabled = true;
        base.monitoring.otlp_endpoint = "http://custom".to_string();
        base.logging.level = LogLevel::Warn;

        let mut other = Config::default();
        // 显式设置触发 false 分支的字段
        other.database.max_connections = 100; // == 100 → 不覆盖
        other.etcd.endpoints = vec![]; // 空 → 不覆盖
                                       // database.url: other 用默认值，与 base 相同 → 不覆盖
                                       // auth.api_keys: 默认空 → 不覆盖
                                       // algorithm.default: 默认 "segment" → 不覆盖
                                       // monitoring.metrics_path: 默认 "/metrics" → 不覆盖
                                       // monitoring.tracing_enabled: 默认 false → 不覆盖
                                       // monitoring.otlp_endpoint: 默认空 → 不覆盖
                                       // logging.level: 默认 Info → 不覆盖

        base.merge(other);

        assert_eq!(base.app.host, "custom.host");
        assert_eq!(base.app.http_port, 7777);
        assert_eq!(base.app.grpc_port, 8888);
        assert_eq!(base.app.dc_id, 5);
        assert_eq!(base.app.worker_id, 10);
        assert_eq!(base.database.max_connections, 50);
        assert_eq!(base.algorithm.default, "uuid_v7");
        assert_eq!(base.monitoring.metrics_path, "/custom");
        assert!(base.monitoring.tracing_enabled);
        assert_eq!(base.monitoring.otlp_endpoint, "http://custom");
        assert_eq!(base.logging.level, LogLevel::Warn);
    }

    /// merge 空的 etcd.endpoints 不应覆盖 base 的非空 endpoints
    ///
    /// 加 ENV_LOCK：`Config::default()` 内部读取 `DATABASE_URL` 环境变量，
    /// 与 `load_from_env_*` 测试并行运行时会因 env var 时序导致 panic
    /// （`std::env::var("DATABASE_URL").unwrap()` 在 `.is_ok()` 通过后失败）。
    #[test]
    fn merge_preserves_etcd_endpoints_when_other_empty() {
        let _guard = ENV_LOCK.lock().unwrap();
        let mut base = Config::default();
        base.etcd.endpoints = vec!["custom:2379".to_string()];
        let mut other = Config::default();
        other.etcd.endpoints = vec![];
        base.merge(other);
        assert_eq!(base.etcd.endpoints, vec!["custom:2379".to_string()]);
    }

    /// merge 相同的 database.url 不应覆盖（条件中 url != self.database.url 为 false）
    ///
    /// 加 ENV_LOCK：`Config::default()` 内部读取 `DATABASE_URL` 环境变量，
    /// 若不串行化，与 `load_from_env_database_url` 并行运行时会出现两次
    /// `Config::default()` 返回不同 URL（一次 postgres 一次 sqlite::memory:），
    /// 导致 merge 后 URL 被覆盖，破坏 "url 相同不覆盖" 的断言。
    #[test]
    fn merge_preserves_database_url_when_same() {
        let _guard = ENV_LOCK.lock().unwrap();
        // 显式移除 DATABASE_URL：ENV_LOCK 只串行化本模块测试，
        // 其他模块（app.rs::tests 等）的测试可能在不持有此锁的情况下
        // 设置 DATABASE_URL，导致两次 Config::default() 返回不同 URL。
        let _url_guard = VarGuard::remove("DATABASE_URL");
        let mut base = Config::default();
        let original_url = base.database.url.clone();
        let other = Config::default(); // 相同的 url
        base.merge(other);
        assert_eq!(base.database.url, original_url);
    }
}
