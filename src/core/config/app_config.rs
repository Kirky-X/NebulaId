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

/// 热更新相关配置（T011）。
#[derive(Debug, Clone, Deserialize, Serialize, Default, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct HotReloadSettings {
    /// 是否在启动时自动监视配置文件 mtime 并热加载。缺省 false：
    /// 行为与历史版本完全一致，仅能通过 POST /config/reload 手动触发。
    #[serde(default)]
    pub auto_watch_enabled: bool,
}

/// Complete application configuration
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields)]
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
    /// 热更新设置（T011：auto_watch_enabled 默认 false，缺省时零行为变化）
    #[serde(default)]
    pub hot_reload: HotReloadSettings,
    /// TLS settings
    pub tls: TlsConfig,
    /// Batch generation settings
    pub batch_generate: BatchGenerateConfig,
}

/// 使用 confers 从 TOML 字符串解析 Config。
///
/// confers 不直接支持 serde 反序列化，需通过 `AnnotatedValue → serde_json::Value
/// → Config` 两步转换。统一封装避免调用点重复（DRY）。
pub(crate) fn parse_toml_config(content: &str, source_id: &str) -> ConfigResult<Config> {
    let annotated = confers::parse_content(
        content,
        confers::Format::Toml,
        confers::SourceId::new(source_id),
        None,
    )
    .map_err(|e| ConfigError::InvalidValue(e.to_string()))?;
    serde_json::from_value(annotated.to_json())
        .map_err(|e| ConfigError::InvalidValue(e.to_string()))
}

/// 启动期配置的来源判定（T014）。
///
/// 调用方据此决定是否需要输出"正在使用内置默认值"的显式告警：`Loaded` 之外只有
/// `DefaultsBecauseMissing` 一种可能，不存在"静默用默认值"的第三种状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartupConfig {
    /// 已按 `path` 成功加载并通过校验
    Loaded { path: String },
    /// 未显式指定配置文件且该路径确实不存在 → 使用内置默认值
    DefaultsBecauseMissing,
}

/// 按 design 判定矩阵解析启动配置（确定性、无启发式）。
///
/// - 文件存在但解析/校验失败 → 原样 `Err`（消息含缺失或未知字段名）→ 启动失败
/// - 文件不存在且 `explicit_path` → `Err(FileNotFound)` → 运维指定的文件必须存在
/// - 文件不存在且非 `explicit_path` → `Ok((Config::default(), DefaultsBecauseMissing))`
/// - 权限/IO 类失败（非 `NotFound`）→ 原样 `Err(FileError)`，不得降级为默认值
///
/// 环境变量覆盖由 `Config::load_from_env()?` 单独承担（其消息已含变量名），
/// 不在本函数职责内。
///
/// # Errors
///
/// 见上述矩阵：除"默认路径且文件不存在"外，任何失败都原样上抛。
pub fn resolve_startup_config(
    path: &str,
    explicit_path: bool,
) -> ConfigResult<(Config, StartupConfig)> {
    match Config::load_from_file(path) {
        Ok(config) => Ok((
            config,
            StartupConfig::Loaded {
                path: path.to_string(),
            },
        )),
        Err(ConfigError::FileNotFound(_)) if !explicit_path => {
            Ok((Config::default(), StartupConfig::DefaultsBecauseMissing))
        }
        Err(e) => Err(e),
    }
}

impl Config {
    /// Load configuration from file with environment variable expansion
    /// Supports ${VAR_NAME} syntax for environment variable substitution
    ///
    /// # Errors
    ///
    /// * [`ConfigError::FileNotFound`] - 文件确实不存在，payload 为传入路径。
    ///   这是唯一允许回落到内置默认值的情形（见 `resolve_startup_config`，T014）。
    /// * [`ConfigError::FileError`] - 读取失败但原因不是缺失（权限、路径是目录、磁盘错误）。
    /// * [`ConfigError::InvalidValue`] - TOML 解析、字段缺失/未知、或 `validate` 不通过。
    pub fn load_from_file(path: &str) -> ConfigResult<Self> {
        // T013：只有 `NotFound` 才代表"文件不存在"，其余 IO 失败（权限、路径是目录、
        // 磁盘错误）必须保持 FileError，避免被启动期判定误当作可降级为默认值的缺失。
        let content = std::fs::read_to_string(path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                ConfigError::FileNotFound(path.to_string())
            } else {
                ConfigError::FileError(format!("{}: {}", path, e))
            }
        })?;

        let expanded = Self::expand_env_vars(&content);

        tracing::debug!(
            event = "config_expanded",
            content_len = content.len(),
            "{}",
            t!("log.core.config.app_config.config_expanded")
        );
        if let Some(auth_start) = expanded.find("[auth]") {
            // 按字符而非字节截断：`find` 返回的下标必定落在字符边界上，但
            // `auth_start + 100` 不一定 —— 配置文件含中文注释或值时，字节切片会
            // 在解析之前 panic（与日志级别无关）。
            let auth_section: String = expanded[auth_start..].chars().take(100).collect();
            tracing::debug!(event = "auth_section", auth_section = %auth_section);
        }

        let config: Config = parse_toml_config(&expanded, "config")?;

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

        if self.app.shutdown_timeout_seconds == 0 {
            return Err(ConfigError::InvalidValue(
                "Shutdown timeout must be greater than 0 seconds".to_string(),
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

        if !["segment", "snowflake", "uuid_v8"].contains(&self.algorithm.default.as_str()) {
            return Err(ConfigError::InvalidValue(
                "Default algorithm must be one of: segment, snowflake, uuid_v8".to_string(),
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
        if other.app.shutdown_timeout_seconds != 30 {
            self.app.shutdown_timeout_seconds = other.app.shutdown_timeout_seconds;
        }
        if other.hot_reload.auto_watch_enabled {
            self.hot_reload.auto_watch_enabled = true;
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
        self.algorithm.uuid_v8 = other.algorithm.uuid_v8;

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

    /// shutdown_timeout_seconds 往返：TOML 显式值保留；字段缺省时 serde default 兜底 30；
    /// merge 仅在非默认值时覆盖（T002 回归钉）
    #[test]
    fn shutdown_timeout_roundtrip_default_and_merge() {
        // 1) 序列化往返保留显式值
        let mut original = Config::default();
        original.app.shutdown_timeout_seconds = 45;
        let toml_content = toml::to_string(&original).expect("序列化 Config 应成功");
        assert!(toml_content.contains("shutdown_timeout_seconds"));

        let temp = tempfile::NamedTempFile::new().expect("创建临时文件应成功");
        std::fs::write(temp.path(), &toml_content).expect("写入临时文件应成功");
        let loaded = Config::load_from_file(temp.path().to_str().unwrap()).unwrap();
        assert_eq!(loaded.app.shutdown_timeout_seconds, 45);
        assert!(loaded.validate().is_ok());

        // 2) 字段从 TOML 缺省 → serde default 兜底 30
        //    （必须整行删除而不是改名：T016 之后未知键会直接报错，改名不再等价于"字段缺省"）
        let without_field = toml_content.replace("shutdown_timeout_seconds = 45\n", "");
        assert!(
            !without_field.contains("shutdown_timeout"),
            "测试前提：该键必须整行消失"
        );
        let reloaded: Config = toml::from_str(&without_field).expect("缺省字段应走 serde default");
        assert_eq!(reloaded.app.shutdown_timeout_seconds, 30);

        // 3) merge：默认值(30)不覆盖，非默认值覆盖
        let mut base = Config::default();
        base.merge(Config::default());
        assert_eq!(base.app.shutdown_timeout_seconds, 30);

        let mut target = Config::default();
        let mut override_cfg = Config::default();
        override_cfg.app.shutdown_timeout_seconds = 60;
        target.merge(override_cfg);
        assert_eq!(target.app.shutdown_timeout_seconds, 60);

        // 4) validate 拒绝 0 值
        let mut invalid = Config::default();
        invalid.app.shutdown_timeout_seconds = 0;
        assert!(invalid.validate().is_err());
    }

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

    /// 路径不存在时应返回 `FileNotFound`（T013：与 IO 类失败区分开）
    #[test]
    fn test_load_from_file_missing_path_returns_file_not_found() {
        let result = Config::load_from_file("/nonexistent/path/no/such/file.toml");
        assert!(
            matches!(result, Err(ConfigError::FileNotFound(_))),
            "不存在的路径必须映射为 FileNotFound，实际为 {:?}",
            result
        );
    }

    /// 传入目录时读文件失败属于 IO 错误而非"文件不存在"，必须保持 `FileError`，
    /// 否则权限/IO 类失败会被误判为文件缺失而在 `DefaultsBecauseMissing` 分支被静默降级。
    ///
    /// 跨平台实测：Windows 读取目录返回 `PermissionDenied`（os error 5），
    /// Linux 返回 `IsADirectory` —— 两者都不是 `NotFound`，因此两条路径都落到 FileError。
    #[test]
    fn test_load_from_file_directory_path_still_returns_file_error() {
        let dir = tempfile::tempdir().expect("创建临时目录应成功");
        let result = Config::load_from_file(dir.path().to_str().unwrap());
        assert!(
            matches!(result, Err(ConfigError::FileError(_))),
            "目录路径应返回 FileError 而非 FileNotFound，实际为 {:?}",
            result
        );
    }

    /// T014 判定矩阵前提：文件存在且合法 → `Loaded { path }`，且值来自文件而非默认值。
    #[test]
    fn test_resolve_startup_config_returns_loaded_with_path() {
        let mut original = Config::default();
        original.app.http_port = 18080;
        original.auth.key_rotation_grace_period_seconds = 3600;
        let toml_content = toml::to_string(&original).expect("序列化 Config 应成功");

        let temp = tempfile::NamedTempFile::new().expect("创建临时文件应成功");
        let path = temp.path().to_str().unwrap().to_string();
        std::fs::write(&path, &toml_content).expect("写入临时文件应成功");

        let (config, source) =
            resolve_startup_config(&path, true).expect("合法配置文件应当加载成功（含显式路径）");
        assert_eq!(source, StartupConfig::Loaded { path: path.clone() });
        assert_eq!(config.app.http_port, 18080, "值必须来自文件");
        assert_eq!(config.auth.key_rotation_grace_period_seconds, 3600);
    }

    /// T014 判定矩阵第 1 行：文件存在但解析失败 —— 错误必须原样上抛，且点名出错的键。
    /// 形状复现历史事故：配置里把 `[algorithm.uuid_v8]` 误写成 `[algorithm.uuid_v7]`，
    /// 此前会被静默降级为默认值并"正常启动"。
    ///
    /// T016 加上 `deny_unknown_fields` 后，误写的段名在反序列化阶段就以 `unknown field`
    /// 暴露 —— 比原先的 `missing field uuid_v8` 更早，且直接点名误写的键。
    #[test]
    fn test_resolve_startup_config_propagates_parse_error_with_field_name() {
        let original = Config::default();
        let toml_content = toml::to_string(&original).expect("序列化 Config 应成功");
        let typo = toml_content.replace("algorithm.uuid_v8", "algorithm.uuid_v7");
        assert_ne!(typo, toml_content, "测试前提：确实改写了段名");

        let temp = tempfile::NamedTempFile::new().expect("创建临时文件应成功");
        std::fs::write(temp.path(), &typo).expect("写入临时文件应成功");

        let result = resolve_startup_config(temp.path().to_str().unwrap(), true);
        match result {
            Err(ConfigError::InvalidValue(msg)) => {
                assert!(
                    msg.contains("unknown field")
                        && msg.contains("uuid_v7")
                        && msg.contains("uuid_v8"),
                    "解析错误必须点名误写字段和期望字段，实际消息：{}",
                    msg
                );
            }
            other => panic!(
                "文件存在但解析失败时必须 Err(InvalidValue)，实际为 {:?}",
                other
            ),
        }
    }

    /// 与上一个测试互补：`deny_unknown_fields` 不得吞掉"必填字段整段缺失"这一类诊断。
    /// 删掉必填的 `[algorithm.uuid_v8]` 段后，错误消息必须点名缺失字段。
    #[test]
    fn test_resolve_startup_config_propagates_missing_field_error() {
        let original = Config::default();
        let toml_content = toml::to_string(&original).expect("序列化 Config 应成功");
        let missing = toml_content.replace("[algorithm.uuid_v8]\nenabled = true\n", "");
        assert_ne!(missing, toml_content, "测试前提：确实删除了整个段");

        let temp = tempfile::NamedTempFile::new().expect("创建临时文件应成功");
        std::fs::write(temp.path(), &missing).expect("写入临时文件应成功");

        let result = resolve_startup_config(temp.path().to_str().unwrap(), true);
        match result {
            Err(ConfigError::InvalidValue(msg)) => {
                assert!(
                    msg.contains("missing field") && msg.contains("uuid_v8"),
                    "解析错误必须点名缺失字段，实际消息：{}",
                    msg
                );
            }
            other => panic!("必填字段缺失时必须 Err(InvalidValue)，实际为 {:?}", other),
        }
    }

    /// T014 判定矩阵第 1 行的另一半：文件能解析但 `validate` 不通过 —— 同样必须原样
    /// 上抛并点名违规项，而不是降级为默认值（`http_port = 0` 是最常见的手误形状）。
    #[test]
    fn test_resolve_startup_config_propagates_validation_error() {
        let mut original = Config::default();
        original.app.http_port = 0;
        let toml_content = toml::to_string(&original).expect("序列化 Config 应成功");

        let temp = tempfile::NamedTempFile::new().expect("创建临时文件应成功");
        std::fs::write(temp.path(), &toml_content).expect("写入临时文件应成功");

        let result = resolve_startup_config(temp.path().to_str().unwrap(), true);
        match result {
            Err(ConfigError::InvalidValue(msg)) => {
                assert!(
                    msg.contains("HTTP port"),
                    "校验错误必须点名违规项，实际消息：{}",
                    msg
                );
            }
            other => panic!("校验失败时必须 Err(InvalidValue)，实际为 {:?}", other),
        }
    }

    /// T014 判定矩阵第 2 行：`--config` 显式给出的路径不存在 → 启动失败。
    #[test]
    fn test_resolve_startup_config_missing_explicit_path_errors() {
        let result = resolve_startup_config("/nonexistent/path/explicit.toml", true);
        match result {
            Err(ConfigError::FileNotFound(msg)) => {
                assert!(
                    msg.contains("/nonexistent/path/explicit.toml"),
                    "错误消息必须点名运维指定的路径，实际：{}",
                    msg
                );
            }
            other => panic!("显式路径不存在时必须 Err(FileNotFound)，实际为 {:?}", other),
        }
    }

    /// T014 判定矩阵第 3 行：默认路径且文件不存在 → 回落内置默认值（开箱可跑）。
    #[test]
    fn test_resolve_startup_config_missing_default_path_falls_back_to_defaults() {
        let (config, source) = resolve_startup_config("/nonexistent/path/default.toml", false)
            .expect("未显式指定路径且文件不存在时应回落到内置默认值");
        assert_eq!(source, StartupConfig::DefaultsBecauseMissing);
        let default = Config::default();
        assert_eq!(config.app.http_port, default.app.http_port);
        assert_eq!(config.auth.enabled, default.auth.enabled);
        assert_eq!(
            config.auth.key_rotation_grace_period_seconds,
            default.auth.key_rotation_grace_period_seconds
        );
    }

    /// T014 判定矩阵第 4 行：权限/IO 类失败（非 NotFound）不得被当作"文件缺失"降级。
    #[test]
    fn test_resolve_startup_config_io_error_never_falls_back_to_defaults() {
        let dir = tempfile::tempdir().expect("创建临时目录应成功");
        let result = resolve_startup_config(dir.path().to_str().unwrap(), false);
        assert!(
            matches!(result, Err(ConfigError::FileError(_))),
            "非 NotFound 的 IO 失败必须原样 Err，实际为 {:?}",
            result
        );
    }

    /// T016 各用例统一注入的未知键名（取值本身无语义，只用于在错误消息里定位）。
    const UNKNOWN_KEY: &str = "__bogus_key_for_test";

    /// T016 测试夹具：在序列化后的默认配置里，向指定表头注入一个未知键。
    ///
    /// `header` 形如 `[app]` 或 `[algorithm.segment]`；返回的文本除这一个键外与
    /// 默认配置完全等价，因此断言失败时唯一的可能就是该未知键没被拒绝。
    fn toml_with_unknown_key(header: &str) -> String {
        let base = toml::to_string(&Config::default()).expect("序列化 Config 应成功");
        // 首部补换行，使第一个表头（无前导 \n）也能被同一个 needle 命中
        let padded = format!("\n{}", base);
        let needle = format!("\n{}\n", header);
        let injected = padded.replace(&needle, &format!("{}{} = true\n", needle, UNKNOWN_KEY));
        assert_ne!(
            injected, padded,
            "测试前提：默认序列化文本中应存在表头 {}",
            header
        );
        injected.replacen("\n", "", 1)
    }

    /// 断言指定 TOML 文本因未知键而加载失败，且消息点名该键。
    fn assert_rejected_unknown_key(toml_content: &str, case: &str) {
        let temp = tempfile::NamedTempFile::new().expect("创建临时文件应成功");
        std::fs::write(temp.path(), toml_content).expect("写入临时文件应成功");

        match Config::load_from_file(temp.path().to_str().unwrap()) {
            Err(ConfigError::InvalidValue(msg)) => assert!(
                msg.contains("unknown field") && msg.contains(UNKNOWN_KEY),
                "{}：必须因未知键返回 unknown field，实际消息：{}",
                case,
                msg
            ),
            Ok(_) => panic!("{}：未知键必须导致加载失败，实际返回 Ok", case),
            Err(other) => panic!("{}：应返回 InvalidValue，实际为 {:?}", case, other),
        }
    }

    /// T016 新契约：顶层未知表必须被拒绝（覆盖 `Config` 自身的 deny_unknown_fields）。
    ///
    /// 本例替换旧测试 `load_from_file_ignores_unknown_fields_like_toml_crate` ——
    /// 旧断言固化的正是被废弃的"静默忽略未知键"契约：段名拼错的整段配置会被无声丢弃。
    #[test]
    fn load_from_file_rejects_unknown_fields() {
        let base = toml::to_string(&Config::default()).expect("序列化 Config 应成功");
        // 未知键必须写在第一个表头之前，否则它会归属到上一个表而不是文档根
        assert_rejected_unknown_key(&format!("{} = true\n\n{}", UNKNOWN_KEY, base), "顶层未知键");
    }

    /// T016 新契约：每一个配置段内的未知键都必须被拒绝。
    ///
    /// 逐段构造而非只测一段，是因为 `deny_unknown_fields` 必须落在全部 17 个结构体上
    /// —— 只在 `Config` 加属性时，`[app]` 里的拼错叶键仍会被静默忽略。
    #[test]
    fn load_from_file_rejects_unknown_key_in_every_section() {
        let sections = [
            "[app]",
            "[database]",
            "[redis]",
            "[etcd]",
            "[auth]",
            "[algorithm]",
            "[algorithm.segment]",
            "[algorithm.snowflake]",
            "[algorithm.uuid_v8]",
            "[monitoring]",
            "[logging]",
            "[rate_limit]",
            "[tls]",
            "[batch_generate]",
            "[hot_reload]",
        ];
        for header in sections {
            assert_rejected_unknown_key(&toml_with_unknown_key(header), header);
        }

        // `[[auth.api_keys]]` 的每一条是独立结构体，默认序列化里是空数组，
        // 因此单独构造一条带未知键的完整条目。
        let entry = format!(
            "{{ key_id = \"k\", key_secret = \"s\", workspace = \"global\", \
             role = \"user\", rate_limit = 1, name = \"n\", {} = true }}",
            UNKNOWN_KEY
        );
        let base = toml::to_string(&Config::default()).expect("序列化 Config 应成功");
        let injected = base.replacen("api_keys = []", &format!("api_keys = [{}]", entry), 1);
        assert_ne!(
            injected, base,
            "测试前提：默认序列化文本中应存在 api_keys = []"
        );
        assert_rejected_unknown_key(&injected, "[[auth.api_keys]] 条目");
    }

    /// 回归：`[auth]` 段之后出现多字节字符时，加载不得 panic。
    ///
    /// 原实现在 `tracing::debug!` 之前就对 `[auth_start..auth_start+100]` 做字节切片，
    /// 边界落在字符中间会 panic（`is not a char boundary`），且与日志级别无关。
    /// 本用例把中文注释长度算准，使第 100 字节必定落在某个汉字的中间：
    /// `[auth]\n` 占 7 字节，注释行 `# ` 占 2 字节，其后每字 3 字节 →
    /// 偏移 100 即注释内第 93 字节 = 2 + 30×3 + 1，落在第 31 个汉字内部。
    #[test]
    fn load_from_file_with_non_ascii_comments_does_not_panic() {
        let base = toml::to_string(&Config::default()).expect("序列化 Config 应成功");
        let comment = format!("# {}\n", "密".repeat(80));
        let injected = base.replacen("\n[auth]\n", &format!("\n[auth]\n{}", comment), 1);
        assert_ne!(injected, base, "测试前提：默认序列化文本中应存在 [auth] 段");

        let temp = tempfile::NamedTempFile::new().expect("创建临时文件应成功");
        std::fs::write(temp.path(), &injected).expect("写入临时文件应成功");

        let config = Config::load_from_file(temp.path().to_str().unwrap())
            .expect("非 ASCII 注释不得影响配置加载");
        assert_eq!(config.auth.enabled, Config::default().auth.enabled);
    }

    /// T016 回归：仓库随附的**服务端**配置必须在严格模式下干净加载。
    ///
    /// 失败时修配置文件，不得放宽 `deny_unknown_fields`。
    ///
    /// 范围说明：任务原文点名的第三份 `config/test_config.toml` **不在清单内** —— 它的
    /// 唯一消费方是 `tests/lib.sh`（bash 测试夹具），段形状是 `[api]`/`[workspace]`/
    /// `[test]`/`[concurrency]`/`[performance]`，既没有 `[app]` 也没有 `[database]`，
    /// 因此服务端加载器从来读不了它（在加严格模式之前就会因 `[auth]` 缺 `enabled` 而
    /// 失败）。把它纳入"必须干净加载"要么需要为 shell 夹具发明服务端配置面，要么需要
    /// 放宽严格模式 —— 两者都不是正确方向，故按真相排除并留档。
    #[test]
    fn test_shipped_config_files_load_cleanly() {
        for path in ["config/config.toml", "config/config_test.toml"] {
            assert!(
                std::path::Path::new(path).exists(),
                "测试前提：仓库内应存在随附配置 {}",
                path
            );
            match Config::load_from_file(path) {
                Ok(_) => {}
                Err(e) => panic!(
                    "随附配置 {} 必须能干净加载（若为未知键，修配置文件而非放宽属性）：{}",
                    path, e
                ),
            }
        }
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
            "Default algorithm must be one of: segment, snowflake, uuid_v8",
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
        other.algorithm.uuid_v8.enabled = false;
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
        assert!(!base.algorithm.uuid_v8.enabled);
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
        base.algorithm.default = "uuid_v8".to_string();
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
        assert_eq!(base.algorithm.default, "uuid_v8");
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

/// T011：hot_reload.auto_watch_enabled 缺省 false、显式 true 往返保留、
/// merge 仅 true 覆盖（false 不回退已开启状态）。
#[test]
fn hot_reload_auto_watch_default_and_merge() {
    // 以完整默认配置的序列化产物为底稿，保证所有必填段齐全
    let base = toml::to_string(&Config::default()).expect("序列化默认配置应成功");
    assert!(
        base.contains("auto_watch_enabled = false"),
        "缺省应序列化为 false"
    );

    let flipped = base.replace("auto_watch_enabled = false", "auto_watch_enabled = true");
    let parsed: Config = toml::from_str(&flipped).expect("显式 true 应可反序列化");
    assert!(parsed.hot_reload.auto_watch_enabled);

    let mut on = Config::default();
    on.hot_reload.auto_watch_enabled = true;
    let mut target = Config::default();
    target.merge(on);
    assert!(target.hot_reload.auto_watch_enabled);

    // merge(false) 不得关闭已开启的开关
    let mut still_on = Config::default();
    still_on.hot_reload.auto_watch_enabled = true;
    let later_off = Config::default();
    still_on.merge(later_off);
    assert!(
        still_on.hot_reload.auto_watch_enabled,
        "merge(false) 不得关闭已开启的开关"
    );
}
