// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Top-level Config aggregation and loading.

use super::audit::AuditConfig;
use super::{
    AlgorithmConfig, AuthConfig, BatchGenerateConfig, ConfigError, ConfigResult, DatabaseConfig,
    EtcdConfig, LogLevel, LoggingConfig, MonitoringConfig, NebulaIdConfig, RateLimitConfig,
    RedisConfig, TlsConfig,
};
use serde::{Deserialize, Serialize};
use std::str::FromStr;

/// 热更新相关配置。
#[derive(Debug, Clone, Deserialize, Serialize, Default, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct HotReloadSettings {
    /// 是否在启动时自动监视配置文件 mtime 并热加载。缺省 false：仅可经 POST /config/reload 手动触发。
    #[serde(default)]
    pub auto_watch_enabled: bool,
}

/// Complete application configuration
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Application settings
    pub app: NebulaIdConfig,
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
    /// 审计持久化设置（内存容量独立配置，生产默认文件持久化）
    #[serde(default)]
    pub audit: AuditConfig,
    /// 热更新设置（auto_watch_enabled 默认 false，缺省时零行为变化）
    #[serde(default)]
    pub hot_reload: HotReloadSettings,
    /// TLS settings
    pub tls: TlsConfig,
    /// Batch generation settings
    pub batch_generate: BatchGenerateConfig,
}

/// 使用 confers 从 TOML 字符串解析 Config。
///
/// confers 不直接支持 serde 反序列化，需 `AnnotatedValue → serde_json::Value → Config` 两步转换。
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

/// 配置文件原文 → 可用配置的公共管线：环境变量展开 → 解析 → 校验。
///
/// 启动期（[`Config::load_from_file`]）与运行期热重载必须共用本函数，
/// 保证两类入口对配置严格性的口径一致。
///
/// # Errors
///
/// 解析或校验失败时返回 [`ConfigError::InvalidValue`]。
pub(crate) fn config_from_file_content(content: &str, source_id: &str) -> ConfigResult<Config> {
    let expanded = Config::expand_env_vars(content);
    tracing::debug!(
        event = "config_expanded",
        content_len = content.len(),
        "{}",
        t!("log.core.config.app_config.config_expanded")
    );
    let config: Config = parse_toml_config(&expanded, source_id)?;
    config.validate()?;
    Ok(config)
}

/// 启动期配置的来源判定。
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
/// - 文件不存在且 `explicit_path` → `Err(FileNotFound)`：运维指定的文件必须存在
/// - 文件不存在且非 `explicit_path` → `Ok((Config::default(), DefaultsBecauseMissing))`
/// - 权限/IO 类失败（非 `NotFound`）→ 原样 `Err(FileError)`，不得降级为默认值
///
/// 环境变量覆盖由 `Config::load_from_env()` 单独承担（其消息已含变量名），不在本函数职责内。
///
/// # Errors
///
/// 除"默认路径且文件不存在"外，任何失败都原样上抛。
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
            // 内置默认值同样过 validate：server-only/剪裁组合下默认算法可能未随特性
            // 启用，fail-fast 到启动期而非第一个请求。
            let config = Config::default();
            config.validate()?;
            Ok((config, StartupConfig::DefaultsBecauseMissing))
        }
        Err(e) => Err(e),
    }
}

/// 环境变量布尔解析：`true`/`false`/`1`/`0`（大小写不敏感，忽略首尾空白）。
/// 非法值返回 `InvalidValue`（消息即变量名，与 `APP_HTTP_PORT` 等非法值口径一致）。
fn parse_env_bool(name: &str, raw: &str) -> ConfigResult<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "true" | "1" => Ok(true),
        "false" | "0" => Ok(false),
        _ => Err(ConfigError::InvalidValue(name.to_string())),
    }
}

impl Config {
    /// Load configuration from file with environment variable expansion.
    ///
    /// 支持 `${VAR_NAME}` 语法做环境变量替换。
    ///
    /// # Errors
    ///
    /// * [`ConfigError::FileNotFound`] - 文件确实不存在；唯一允许回落内置默认值的情形
    ///   （见 `resolve_startup_config`）。
    /// * [`ConfigError::FileError`] - 读取失败但原因不是缺失（权限、路径是目录、磁盘错误）。
    /// * [`ConfigError::InvalidValue`] - TOML 解析、字段缺失/未知、或 `validate` 不通过。
    pub fn load_from_file(path: &str) -> ConfigResult<Self> {
        // 只有 `NotFound` 才代表"文件不存在"，其余 IO 失败（权限、路径是目录、
        // 磁盘错误）必须保持 FileError，避免被启动期判定误当作可降级为默认值的缺失。
        let content = std::fs::read_to_string(path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                ConfigError::FileNotFound(path.to_string())
            } else {
                ConfigError::FileError(format!("{}: {}", path, e))
            }
        })?;

        let config = config_from_file_content(&content, "config")?;

        tracing::debug!(event = "toml_parsed", raw_auth_enabled = %format!("{:?}", config.auth.enabled), "{}", t!("log.core.config.app_config.toml_parsed"));
        tracing::debug!(event = "config_loaded", auth_enabled = %config.auth.enabled, "{}", t!("log.core.config.app_config.config_loaded"));

        Ok(config)
    }

    /// Validate configuration values are reasonable.
    ///
    /// 按资源域拆分执行子校验器；依序执行，首个失败即返回。
    pub fn validate(&self) -> ConfigResult<()> {
        self.validate_app()?;
        self.validate_database()?;
        self.validate_rate_limit()?;
        self.validate_algorithm()?;
        self.validate_batch()?;
        self.validate_auth_secrets()?;
        Ok(())
    }

    fn validate_app(&self) -> ConfigResult<()> {
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

        Ok(())
    }

    fn validate_database(&self) -> ConfigResult<()> {
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

        // 0 会经 `Duration::from_secs(0)` 直接传给连接池，连接一建立就被判空闲回收，
        // 等价于"永不复用连接"；与 `acquire_timeout_seconds` 同口径拒绝。
        if self.database.idle_timeout_seconds == 0 {
            return Err(ConfigError::InvalidValue(
                "Database idle_timeout_seconds must be greater than 0".to_string(),
            ));
        }

        // 0 会经 `Duration::from_secs(0)` 传给 `tokio::time::timeout`，
        // 热查询立即超时，等价于数据库全拒；与 `acquire_timeout_seconds` 同口径拒绝。
        if self.database.statement_timeout_secs == 0 {
            return Err(ConfigError::InvalidValue(
                "Database statement_timeout_secs must be greater than 0".to_string(),
            ));
        }

        Ok(())
    }

    fn validate_rate_limit(&self) -> ConfigResult<()> {
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

            // `saturating_mul`：`default_rps` 接近 `u32::MAX` 时普通乘法在 debug 构建
            // panic、release 回绕成更小的数，把本该拒绝的配置放行。
            if self.rate_limit.burst_size > self.rate_limit.default_rps.saturating_mul(10) {
                return Err(ConfigError::InvalidValue(
                    "Rate limit burst_size should not exceed 10x default_rps".to_string(),
                ));
            }
        }

        Ok(())
    }

    fn validate_algorithm(&self) -> ConfigResult<()> {
        // 串面唯一映射源：AlgorithmType::from_str（v4/v7 为真算法，别名面与 set_algorithm 同源）。
        if crate::core::types::AlgorithmType::from_str(&self.algorithm.default).is_err() {
            return Err(ConfigError::InvalidValue(
                "Default algorithm must be one of: segment, snowflake, uuid_v8, uuid_v4, uuid_v7"
                    .to_string(),
            ));
        }

        // 部署形态一致性 fail-fast：默认算法必须随本构建的特性一起启用，否则
        // server-only/剪裁组合能启动，但所有 generate 在请求期才显性报错。
        // 测试构建允许合成组合，不做拦截。
        #[cfg(all(not(test), not(feature = "segment")))]
        if self.algorithm.get_default_algorithm() == crate::core::types::AlgorithmType::Segment {
            return Err(ConfigError::InvalidValue(t!(
                "error.core.config.app_config.segment_feature_not_enabled"
            )));
        }
        #[cfg(all(not(test), not(feature = "snowflake")))]
        if self.algorithm.get_default_algorithm() == crate::core::types::AlgorithmType::Snowflake {
            return Err(ConfigError::InvalidValue(t!(
                "error.core.config.app_config.snowflake_feature_not_enabled"
            )));
        }
        #[cfg(all(not(test), not(feature = "uuid-v8")))]
        if self.algorithm.get_default_algorithm() == crate::core::types::AlgorithmType::UuidV8 {
            return Err(ConfigError::InvalidValue(t!(
                "error.core.config.app_config.uuid_v8_feature_not_enabled"
            )));
        }
        #[cfg(all(not(test), not(feature = "uuid-v4")))]
        if self.algorithm.get_default_algorithm() == crate::core::types::AlgorithmType::UuidV4 {
            return Err(ConfigError::InvalidValue(t!(
                "error.core.config.app_config.uuid_v4_feature_not_enabled"
            )));
        }
        #[cfg(all(not(test), not(feature = "uuid-v7")))]
        if self.algorithm.get_default_algorithm() == crate::core::types::AlgorithmType::UuidV7 {
            return Err(ConfigError::InvalidValue(t!(
                "error.core.config.app_config.uuid_v7_feature_not_enabled"
            )));
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

        self.validate_snowflake()
    }

    fn validate_snowflake(&self) -> ConfigResult<()> {
        // u32 求和：三位 u8 相加（如 100+100+60=260）在 debug 构建 panic、
        // release 回绕成 4 并通过 <64 校验，溢出要到算法侧 `1 << bits` 才炸。
        // 和 <64 即保证每个分量 < 64，移位量合法。
        let total_bits = u32::from(self.algorithm.snowflake.datacenter_id_bits)
            + u32::from(self.algorithm.snowflake.worker_id_bits)
            + u32::from(self.algorithm.snowflake.sequence_bits);

        if total_bits >= 64 {
            return Err(ConfigError::InvalidValue(t!(
                "error.core.config.app_config.total_bits_exceed_limit"
            )));
        }

        // ID 值必须落在各自位宽可表示域内：construct_id 直接 OR 不掩码，超界
        // dc_id/worker_id 会污染高位（时间戳）位造成跨 DC 碰撞。u8 位宽 ≥ 8 时
        // 全域合法；< 8 时移位量 ≤ 7，u16 移位安全（总位宽校验已拦截极端值）。
        let dc_bits = self.algorithm.snowflake.datacenter_id_bits;
        if dc_bits < 8 {
            let dc_limit = 1u16 << dc_bits;
            if u16::from(self.app.dc_id) >= dc_limit {
                return Err(ConfigError::InvalidValue(t!(
                    "error.core.config.app_config.dc_id_exceeds_bits",
                    dc_id = self.app.dc_id,
                    dc_bits = dc_bits,
                    max = dc_limit - 1
                )));
            }
        }

        let worker_bits = self.algorithm.snowflake.worker_id_bits;
        if worker_bits < 8 {
            let worker_limit = 1u16 << worker_bits;
            if u16::from(self.app.worker_id) >= worker_limit {
                return Err(ConfigError::InvalidValue(t!(
                    "error.core.config.app_config.worker_id_exceeds_bits",
                    worker_id = self.app.worker_id,
                    worker_bits = worker_bits,
                    max = worker_limit - 1
                )));
            }
        }

        if self.algorithm.snowflake.clock_drift_threshold_ms == 0 {
            return Err(ConfigError::InvalidValue(t!(
                "error.core.config.app_config.clock_drift_threshold_invalid"
            )));
        }

        // 混淆密钥：必须恰 16 个 hex 字符（= u64）。None 合法（未启用）。
        if let Some(hex) = &self.algorithm.obfuscation.key_hex {
            if hex.len() != 16 || u64::from_str_radix(hex, 16).is_err() {
                return Err(ConfigError::InvalidValue(t!(
                    "error.core.config.app_config.key_hex_invalid"
                )));
            }
        }

        Ok(())
    }

    fn validate_batch(&self) -> ConfigResult<()> {
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

    fn validate_auth_secrets(&self) -> ConfigResult<()> {
        // SAST 修复：宽限期上限校验（handler 层 clamp 是静默的，配置文件中的值与
        // 生效值不一致，运维无法从文件判断，fail-fast 比 silent clamp 更安全）。
        // `api_key_salt` 空串是 dev/test 的安全默认值（garrison 生产路径已 panic），
        // 不在 config validate 层拒绝，避免破坏测试。
        if self.auth.key_rotation_grace_period_seconds
            > crate::core::config::defaults::MAX_KEY_ROTATION_GRACE_PERIOD_SECONDS
        {
            return Err(ConfigError::InvalidValue(format!(
                "auth.key_rotation_grace_period_seconds must not exceed {} seconds (30 days)",
                crate::core::config::defaults::MAX_KEY_ROTATION_GRACE_PERIOD_SECONDS
            )));
        }

        // 未定义的环境变量在 `expand_env_vars` 中按设计保留字面量 `${VAR}`（见
        // `expand_env_vars_preserves_missing_var`）。字符串型字段的字面量是"看起来
        // 合法的值"：`database.password` 有建连层显性拒绝，而 auth 凭证没有任何等价
        // 防线 —— 变量缺失时盐值就是字面量，全部密钥哈希共享一个可预测 pepper。
        let mut unexpanded = Vec::new();
        if self.auth.api_key_salt.contains("${") {
            unexpanded.push("auth.api_key_salt");
        }
        if self
            .auth
            .api_keys
            .iter()
            .any(|entry| entry.key_secret.contains("${"))
        {
            unexpanded.push("auth.api_keys[].key_secret");
        }
        if !unexpanded.is_empty() {
            return Err(ConfigError::InvalidValue(format!(
                "Configuration values contain unexpanded environment variable references: {}",
                unexpanded.join(", ")
            )));
        }

        Ok(())
    }

    /// `${VAR}` / `${VAR:default}` 环境变量展开（confers `interpolation` 引擎）。
    /// `allow_unresolved` 保持既有设计：缺失变量保留 `${VAR}` 字面量，由
    /// `validate()` 对 auth 凭证字段显性报错。
    fn expand_env_vars(content: &str) -> String {
        static CONFIG: std::sync::OnceLock<confers::interpolation::InterpolationConfig> =
            std::sync::OnceLock::new();
        let interpolation_config =
            CONFIG.get_or_init(|| confers::interpolation::InterpolationConfig {
                allow_unresolved: true,
                ..confers::interpolation::InterpolationConfig::default()
            });
        confers::interpolation::interpolate_with_config(
            content,
            &|var| std::env::var(var).ok(),
            interpolation_config,
        )
        .unwrap_or_else(|e| {
            // 展开引擎报错（如病态嵌套）时保留原文并告警，与 allow_unresolved 口径
            // 一致，残留 `${` 由 validate() 兜底显性化。
            tracing::warn!(error = ?e, "config interpolation failed; keeping raw content");
            content.to_string()
        })
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

        // TLS 覆盖：容器/编排部署经环境变量启用 HTTPS，无需替换镜像内配置文件
        // （cert/key 由挂载卷提供）。布尔非法值显性报错，消息含变量名。
        if let Ok(raw) = std::env::var("TLS_ENABLED") {
            config.tls.enabled = parse_env_bool("TLS_ENABLED", &raw)?;
        }
        if let Ok(raw) = std::env::var("TLS_HTTP_ENABLED") {
            config.tls.http_enabled = parse_env_bool("TLS_HTTP_ENABLED", &raw)?;
        }
        if let Ok(raw) = std::env::var("TLS_GRPC_ENABLED") {
            config.tls.grpc_enabled = parse_env_bool("TLS_GRPC_ENABLED", &raw)?;
        }
        if let Ok(path) = std::env::var("TLS_CERT_PATH") {
            config.tls.cert_path = path;
        }
        if let Ok(path) = std::env::var("TLS_KEY_PATH") {
            config.tls.key_path = path;
        }
        if let Ok(path) = std::env::var("TLS_CA_PATH") {
            config.tls.ca_path = Some(path);
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

        // 以「是否等于内置默认值」判定显式覆盖：`other` 来自 `Config::load_from_env()`
        // 时未设 `ETCD_ENDPOINTS` 的字段等于默认值（空列表），不得反向覆盖文件里已写的
        // endpoints。显式单进程形态经 `ETCD_ENDPOINTS=""` 解析为空白项（≠ 默认值）→
        // 覆盖成立，由 `EtcdConfig::has_configured_endpoints` 判定为未配置。
        if other.etcd.endpoints != EtcdConfig::default().endpoints {
            self.etcd.endpoints = other.etcd.endpoints;
        }

        if !other.auth.api_keys.is_empty() {
            self.auth.api_keys = other.auth.api_keys;
        }

        // `key_rotation_grace_period_seconds` 不参与 merge：部署期一次决策，热重载
        // 变更会让进行中的轮换窗口产生不一致凭证判定；调整必须重启进程，由
        // `resolve_startup_config` 重新读取。

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

        // TLS：与 `tracing_enabled`/`auto_watch_enabled` 同款口径 —— 布尔以
        // 非默认值（true）为覆盖信号，路径仅非空覆盖；`false` 不反向关闭
        // 文件里已启用的 TLS（关闭须改文件，避免 env 意外降级明文）。
        if other.tls.enabled {
            self.tls.enabled = true;
        }
        if other.tls.http_enabled {
            self.tls.http_enabled = true;
        }
        if other.tls.grpc_enabled {
            self.tls.grpc_enabled = true;
        }
        if !other.tls.cert_path.is_empty() {
            self.tls.cert_path = other.tls.cert_path;
        }
        if !other.tls.key_path.is_empty() {
            self.tls.key_path = other.tls.key_path;
        }
        if other.tls.ca_path.is_some() {
            self.tls.ca_path = other.tls.ca_path;
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
    /// merge 仅在非默认值时覆盖（回归钉）
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
        //    （必须整行删除而不是改名：之后未知键会直接报错，改名不再等价于"字段缺省"）
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

    /// 路径不存在时应返回 `FileNotFound`（与 IO 类失败区分开）
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
    /// 否则会被误判为缺失而在 `DefaultsBecauseMissing` 分支被静默降级。
    ///
    /// 跨平台：Windows 读目录返回 `PermissionDenied`、Linux 返回 `IsADirectory` ——
    /// 两者都非 `NotFound`，因此都落到 FileError。
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

    /// 判定矩阵前提：文件存在且合法 → `Loaded { path }`，且值来自文件而非默认值。
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

    /// 判定矩阵第 1 行：文件存在但解析失败 —— 错误必须原样上抛，且点名出错/期望字段。
    /// 段名误写 `[algorithm.uuid_v7]` 会被 `deny_unknown_fields` 在反序列化阶段以
    /// `unknown field` 暴露，比 `missing field` 更早。
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

    /// 判定矩阵第 1 行的另一半：文件能解析但 `validate` 不通过 —— 同样必须原样
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

    /// 判定矩阵第 2 行：`--config` 显式给出的路径不存在 → 启动失败。
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

    /// 判定矩阵第 3 行：默认路径且文件不存在 → 回落内置默认值（开箱可跑）。
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

    /// 判定矩阵第 4 行：权限/IO 类失败（非 NotFound）不得被当作"文件缺失"降级。
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

    /// 判定矩阵第 4 行的另一半：显式路径下的 IO 失败同样必须 `Err(FileError)`，
    /// 且消息点名出问题的路径。
    #[test]
    fn test_resolve_startup_config_io_error_with_explicit_path_errors() {
        let dir = tempfile::tempdir().expect("创建临时目录应成功");
        let path = dir.path().to_str().unwrap().to_string();
        let result = resolve_startup_config(&path, true);
        match result {
            Err(ConfigError::FileError(msg)) => {
                assert!(
                    msg.contains(&path),
                    "FileError 消息必须点名出问题的路径，实际：{}",
                    msg
                );
            }
            other => panic!("显式路径读取失败时必须 Err(FileError)，实际为 {:?}", other),
        }
    }

    /// 判定矩阵第 1 行 × 非显式路径：默认路径文件损坏同样不得回落内置默认值 ——
    /// 发行包自带 `config/config.toml`，运维改坏它就落在这一格。
    #[test]
    fn test_resolve_startup_config_broken_file_without_explicit_path_errors() {
        let original = Config::default();
        let toml_content = toml::to_string(&original).expect("序列化 Config 应成功");
        let typo = toml_content.replace("algorithm.uuid_v8", "algorithm.uuid_v7");
        assert_ne!(typo, toml_content, "测试前提：确实改写了段名");

        let temp = tempfile::NamedTempFile::new().expect("创建临时文件应成功");
        let path = temp.path().to_str().unwrap().to_string();
        std::fs::write(&path, &typo).expect("写入临时文件应成功");

        let result = resolve_startup_config(&path, false);
        assert!(
            matches!(result, Err(ConfigError::InvalidValue(_))),
            "文件坏了但路径非显式指定时仍必须 Err(InvalidValue)，不得回落默认值，实际为 {:?}",
            result
        );
    }

    /// 判定矩阵第 2 行的对照组（补空）：非显式路径 + 文件合法 → 必须 `Loaded` 且
    /// 取值来自文件。缺了这一格就无法区分"回落默认值"和"读到了恰好等于默认值的文件"。
    #[test]
    fn test_resolve_startup_config_valid_file_without_explicit_path_loads() {
        let mut original = Config::default();
        original.app.http_port = 18081;
        let toml_content = toml::to_string(&original).expect("序列化 Config 应成功");

        let temp = tempfile::NamedTempFile::new().expect("创建临时文件应成功");
        let path = temp.path().to_str().unwrap().to_string();
        std::fs::write(&path, &toml_content).expect("写入临时文件应成功");

        let (config, source) = resolve_startup_config(&path, false)
            .expect("文件合法且存在时必须 Loaded，即使路径不是显式指定");
        assert_eq!(source, StartupConfig::Loaded { path: path.clone() });
        assert_eq!(config.app.http_port, 18081, "值必须来自文件而非内置默认值");
    }

    /// 未定义变量在 `expand_env_vars` 后保留字面量 `${VAR}`，auth 侧
    /// 凭证字段必须在 `validate` 被拒 —— 否则那段字面量会当作 pepper/密钥静默使用。
    #[test]
    fn validate_rejects_unexpanded_env_var_in_credential_fields() {
        let mut config = Config::default();
        config.auth.api_key_salt = "${NEBULA_SALT_MISSING}".to_string();
        match config.validate() {
            Err(ConfigError::InvalidValue(msg)) => {
                assert!(
                    msg.contains("auth.api_key_salt") && msg.contains("unexpanded"),
                    "错误必须点名未展开的字段，实际：{}",
                    msg
                );
            }
            other => panic!("未展开的盐值必须 Err(InvalidValue)，实际为 {:?}", other),
        }

        config.auth.api_key_salt = String::new();
        config.auth.api_keys = vec![ApiKeyEntry {
            key_id: "niad_1".to_string(),
            key_secret: "${NEBULA_KEY_SECRET}".to_string(),
            workspace: "global".to_string(),
            role: "user".to_string(),
            rate_limit: 100,
            name: "k".to_string(),
        }];
        let err = config
            .validate()
            .expect_err("未展开的 key_secret 同样必须被拒");
        assert!(
            matches!(&err, ConfigError::InvalidValue(msg) if msg.contains("auth.api_keys")),
            "实际为 {:?}",
            err
        );
    }

    /// snowflake 三个位数分量用 u8 相加会溢出（debug panic / release
    /// 回绕后通过校验、在算法侧移位 panic）。必须在校验阶段以 u32 求和并拒绝。
    #[test]
    fn validate_rejects_snowflake_bits_that_overflow_u8_sum() {
        let mut config = Config::default();
        // 100 + 100 + 60 = 260：u8 相加在 debug 构建直接 panic，在 release 回绕成 4。
        config.algorithm.snowflake.datacenter_id_bits = 100;
        config.algorithm.snowflake.worker_id_bits = 100;
        config.algorithm.snowflake.sequence_bits = 60;
        match config.validate() {
            Err(ConfigError::InvalidValue(msg)) => {
                assert!(msg.contains("bits"), "错误必须点名位数规则，实际：{}", msg);
            }
            other => panic!(
                "u8 溢出的位数组合必须 Err(InvalidValue)，实际为 {:?}",
                other
            ),
        }
    }

    /// dc_id 超出 datacenter_id_bits 位宽必须在配置校验期拒绝：
    /// construct_id 直接 OR 不掩码，超界值会污染高位时间戳位，造成跨 DC
    /// 组合碰撞（dc8@T 与 dc0@(T+1) 同 worker 同序列时 ID 完全相同）。
    #[test]
    fn validate_rejects_dc_id_overflowing_datacenter_bits() {
        let mut config = Config::default();
        // 默认位宽 3 → 上限 7；8 即越界，必须拒绝。
        config.app.dc_id = 8;
        match config.validate() {
            Err(ConfigError::InvalidValue(msg)) => {
                assert!(
                    msg.contains("dc_id") && msg.contains("3"),
                    "错误必须含实际值与位宽上限，实际：{}",
                    msg
                );
            }
            other => panic!("dc_id 超位宽必须 Err(InvalidValue)，实际为 {:?}", other),
        }
    }

    /// worker_id 超出 worker_id_bits 位宽同样必须在配置校验期拒绝。
    #[test]
    fn validate_rejects_worker_id_overflowing_worker_bits() {
        let mut config = Config::default();
        // 位宽降到 4 → 上限 15；worker_id=20 已越界。
        config.algorithm.snowflake.worker_id_bits = 4;
        config.app.worker_id = 20;
        match config.validate() {
            Err(ConfigError::InvalidValue(msg)) => {
                assert!(
                    msg.contains("worker_id") && msg.contains("4"),
                    "错误必须含实际值与位宽上限，实际：{}",
                    msg
                );
            }
            other => panic!("worker_id 超位宽必须 Err(InvalidValue)，实际为 {:?}", other),
        }
    }

    /// 位宽边界值必须放行：dc_id=7 + 3 位、worker_id=255 + 8 位
    ///（以及位宽 ≥ 8 时任意 u8 值）都是合法配置。
    #[test]
    fn validate_accepts_id_at_bit_width_boundary() {
        let mut config = Config::default();
        config.app.dc_id = 7;
        config.app.worker_id = 255;
        config
            .validate()
            .expect("边界值 dc_id=7/worker_id=255 配默认位宽必须通过");

        // 位宽 ≥ 8 时 dc_id/worker_id 的 u8 全域合法（含 255）。
        let mut wide = Config::default();
        wide.algorithm.snowflake.datacenter_id_bits = 8;
        wide.algorithm.snowflake.worker_id_bits = 12;
        wide.algorithm.snowflake.sequence_bits = 10;
        wide.app.dc_id = 255;
        wide.app.worker_id = 255;
        wide.validate()
            .expect("位宽 ≥ 8 时 u8 全域 dc_id/worker_id 必须通过");
    }

    /// 混淆密钥格式校验：16 hex 合法；长度错/非 hex 拒绝；None 放行。
    #[test]
    fn validate_obfuscation_key_hex_format() {
        let mut config = Config::default();
        config.algorithm.obfuscation.key_hex = Some("0011223344556677".to_string());
        config.validate().expect("合法 16 hex 密钥必须通过");

        let mut short = Config::default();
        short.algorithm.obfuscation.key_hex = Some("0011".to_string());
        assert!(matches!(
            short.validate(),
            Err(ConfigError::InvalidValue(ref m)) if m.contains("key_hex")
        ));

        let mut non_hex = Config::default();
        non_hex.algorithm.obfuscation.key_hex = Some("zz11223344556677".to_string());
        assert!(matches!(
            non_hex.validate(),
            Err(ConfigError::InvalidValue(ref m)) if m.contains("key_hex")
        ));

        let none = Config::default();
        none.validate().expect("None 密钥必须放行");
    }

    /// 密钥在 Debug 输出中必须 REDACTED（与 api_key_salt 同口径）。
    #[test]
    fn obfuscation_key_hex_is_redacted_in_debug() {
        let mut config = Config::default();
        config.algorithm.obfuscation.key_hex = Some("0011223344556677".to_string());
        let dump = format!("{:?}", config.algorithm.obfuscation);
        assert!(
            !dump.contains("0011223344556677"),
            "密钥明文泄漏到 Debug: {dump}"
        );
        assert!(dump.contains("[REDACTED]"));
    }

    /// 极端位宽（≥ 16）下位宽校验自身的移位不得 panic：
    /// `1u16 << 200` 会直接炸，校验逻辑必须先保证移位量合法。
    #[test]
    fn validate_bit_width_check_does_not_panic_on_extreme_bits() {
        let mut config = Config::default();
        config.algorithm.snowflake.datacenter_id_bits = 200;
        config.algorithm.snowflake.worker_id_bits = 100;
        config.algorithm.snowflake.sequence_bits = 60;
        // 总和 360 >= 64 由既有的 u32 求和校验拒绝；此处只断言不 panic 且 Err。
        assert!(config.validate().is_err());
    }

    /// `burst_size > default_rps * 10` 的乘法在 `default_rps` 接近
    /// `u32::MAX` 时溢出。改为 saturating_mul 后：按真实数学关系判定，且不再 panic。
    #[test]
    fn validate_rate_limit_burst_check_does_not_overflow() {
        let mut config = Config::default();
        config.rate_limit.enabled = true;
        // 真实关系：burst_size(u32::MAX) ≤ 10 × 1_000_000_000，因此必须放行。
        // 回绕后的乘积是 1_410_065_408，旧实现会在这里错误拒绝。
        config.rate_limit.default_rps = 1_000_000_000;
        config.rate_limit.burst_size = u32::MAX;
        config
            .validate()
            .expect("10×rps 超过 u32 上限时不得因乘法回绕而错误拒绝");

        // 反向：真实超限仍必须拒绝，saturating 不能把校验变成空转。
        let mut violating = Config::default();
        violating.rate_limit.enabled = true;
        violating.rate_limit.default_rps = 100;
        violating.rate_limit.burst_size = 10_001;
        let err = violating
            .validate()
            .expect_err("burst 超过 10×rps 时必须拒绝");
        assert!(
            matches!(&err, ConfigError::InvalidValue(msg) if msg.contains("burst_size")),
            "实际为 {:?}",
            err
        );
    }

    /// 凭证字段的 `Debug` 必须脱敏：任何 `{:?}`（日志、panic、断言输出）都不得含
    /// 明文密钥、pepper、口令或内嵌口令的 URL。
    #[test]
    fn config_debug_output_never_renders_credential_material() {
        const SECRET: &str = "super-secret-material";
        const PEPPER: &str = "pepper-material";
        const DB_PASS: &str = "db-password-material";
        const REDIS_PASS: &str = "redis-password-material";

        let mut config = Config::default();
        config.auth.api_key_salt = PEPPER.to_string();
        config.auth.api_keys = vec![ApiKeyEntry {
            key_id: "niad_debug".to_string(),
            key_secret: SECRET.to_string(),
            workspace: "global".to_string(),
            role: "user".to_string(),
            rate_limit: 100,
            name: "debug-key".to_string(),
        }];
        config.database.password = DB_PASS.to_string();
        config.database.url = format!("postgresql://idgen:{}@db:5432/idgen", DB_PASS);
        config.redis.url = format!("redis://:{}@cache:6379", REDIS_PASS);

        let rendered = format!("{:#?}", config);
        for material in [SECRET, PEPPER, DB_PASS, REDIS_PASS] {
            assert!(
                !rendered.contains(material),
                "`Debug` 输出泄漏了凭证材料 {}：{}",
                material,
                rendered
            );
        }
        // 反向：脱敏必须是"替换"而不是"整段消失"，否则调试时看不出字段是否存在。
        assert!(rendered.contains("ApiKeyEntry"));
        assert!(rendered.contains("niad_debug"));
        assert!(rendered.contains("[REDACTED]"));
        assert!(
            rendered.contains("postgresql://idgen:***@db:5432/idgen"),
            "URL 应只替换口令，实际：{}",
            rendered
        );
    }

    /// 重复段不得被"后写覆盖前写"静默接受 —— 那会让运维以为生效的
    /// 那一段其实被整段丢弃。
    #[test]
    fn duplicate_toml_section_is_not_silently_accepted() {
        let mut base = toml::to_string(&Config::default()).expect("序列化 Config 应成功");
        base.push_str("\n[app]\nhttp_port = 19999\n");

        let temp = tempfile::NamedTempFile::new().expect("创建临时文件应成功");
        std::fs::write(temp.path(), &base).expect("写入临时文件应成功");

        match Config::load_from_file(temp.path().to_str().unwrap()) {
            Err(e) => {
                // 预期形状：解析阶段就失败。
                assert!(
                    matches!(e, ConfigError::InvalidValue(_)),
                    "重复段应在解析阶段失败，实际为 {:?}",
                    e
                );
            }
            Ok(loaded) => panic!(
                "重复 `[app]` 段被静默接受（http_port={}），后写覆盖了前写",
                loaded.app.http_port
            ),
        }
    }

    /// 段名大小写变体（`[App]`）在 TOML 里是另一个键，必须由顶层
    /// `deny_unknown_fields` 拒绝，而不是当成"该段没写"回落默认值。
    #[test]
    fn section_name_case_variant_is_rejected_as_unknown_field() {
        let base = toml::to_string(&Config::default()).expect("序列化 Config 应成功");
        let mutated = base.replacen("[app]\n", "[App]\n", 1);
        assert_ne!(mutated, base, "测试前提：确实存在 `[app]` 段头");

        let temp = tempfile::NamedTempFile::new().expect("创建临时文件应成功");
        std::fs::write(temp.path(), &mutated).expect("写入临时文件应成功");

        match Config::load_from_file(temp.path().to_str().unwrap()) {
            Err(ConfigError::InvalidValue(msg)) => {
                assert!(
                    msg.contains("unknown field") && msg.contains("App"),
                    "必须点名误写的段名，实际消息：{}",
                    msg
                );
            }
            other => panic!("大小写变体段名必须 Err(InvalidValue)，实际为 {:?}", other),
        }
    }

    /// 各用例统一注入的未知键名（取值本身无语义，只用于在错误消息里定位）。
    const UNKNOWN_KEY: &str = "__bogus_key_for_test";

    /// 测试夹具：在序列化后的默认配置里，向指定表头注入一个未知键。
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

    /// 新契约：顶层未知表必须被拒绝（`Config` 自身的 deny_unknown_fields）。
    ///
    /// 段名拼错的整段配置不得被无声丢弃（替代旧测试固化的"静默忽略未知键"契约）。
    #[test]
    fn load_from_file_rejects_unknown_fields() {
        let base = toml::to_string(&Config::default()).expect("序列化 Config 应成功");
        // 未知键必须写在第一个表头之前，否则它会归属到上一个表而不是文档根
        assert_rejected_unknown_key(&format!("{} = true\n\n{}", UNKNOWN_KEY, base), "顶层未知键");
    }

    /// 新契约：每一个配置段内的未知键都必须被拒绝。
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

    /// 回归：`[auth]` 段之后出现多字节字符时，加载不得 panic（字节切片落在字符
    /// 中间会 `is not a char boundary` panic，且与日志级别无关）。
    ///
    /// 本用例构造中文注释使第 100 字节必定落在汉字中间：`[auth]\n` 占 7 字节、
    /// `# ` 占 2 字节，其后每字 3 字节 → 偏移 100 = 2 + 30×3 + 1，落在第 31 个汉字内部。
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

    /// 回归：仓库随附的**服务端**配置必须在严格模式下干净加载；失败修配置文件，
    /// 不得放宽 `deny_unknown_fields`。
    ///
    /// 范围：`config/test_config.toml` 的唯一消费方是 `tests/lib.sh`（bash 夹具，
    /// `[api]`/`[workspace]` 形状），服务端加载器读不了它，不在本清单内。
    #[test]
    fn test_shipped_config_files_load_cleanly() {
        let path = "config/config.toml";
        assert!(
            std::path::Path::new(path).exists(),
            "测试前提：仓库内应存在随附配置 {path}"
        );
        match Config::load_from_file(path) {
            Ok(_) => {}
            Err(e) => {
                panic!("随附配置 {path} 必须能干净加载（若为未知键，修配置文件而非放宽属性）：{e}")
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

    /// dc_id 超出实际位宽（datacenter_id_bits=3，上限 7）时校验失败：按位宽判定，
    /// 不放行 8..=31 的位溢出配置。
    #[test]
    fn validate_dc_id_over_default_width_fails() {
        let mut config = Config::default();
        config.app.dc_id = 32;
        assert_invalid_value(
            config.validate(),
            "app.dc_id 32 exceeds datacenter_id_bits=3",
        );
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

    /// database.statement_timeout_secs=0 时校验失败
    #[test]
    fn validate_statement_timeout_zero_fails() {
        let mut config = Config::default();
        config.database.statement_timeout_secs = 0;
        assert_invalid_value(
            config.validate(),
            "Database statement_timeout_secs must be greater than 0",
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
        config.rate_limit.burst_size = 101; // 101 > 100 = 10×10
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
            "Default algorithm must be one of: segment, snowflake, uuid_v8, uuid_v4, uuid_v7",
        );
    }

    /// uuid_v4 / uuid_v7 为合法默认算法。
    #[test]
    fn validate_algorithm_default_accepts_uuid_v4_v7() {
        for default in ["uuid_v4", "uuid_v7"] {
            let mut config = Config::default();
            config.algorithm.default = default.to_string();
            assert!(
                config.validate().is_ok(),
                "{default} must be a valid default algorithm"
            );
        }
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
        config.algorithm.snowflake.sequence_bits = 16;
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

    /// `${VAR:default}` 缺省值语法：变量缺失时取默认值，存在时环境变量优先。
    #[test]
    fn expand_env_vars_default_value_syntax() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _v = VarGuard::remove("NEBULA_TEST_EXPAND_DEFAULT");
        let result = Config::expand_env_vars("value=${NEBULA_TEST_EXPAND_DEFAULT:fallback}");
        assert_eq!(result, "value=fallback");

        let _v = VarGuard::set("NEBULA_TEST_EXPAND_DEFAULT", "from-env");
        let result = Config::expand_env_vars("value=${NEBULA_TEST_EXPAND_DEFAULT:fallback}");
        assert_eq!(result, "value=from-env");
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

    /// TLS_* 环境变量（容器/编排启用 HTTPS 的入口）应被加载到 tls 段。
    #[test]
    fn load_from_env_tls_overrides() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _a = VarGuard::set("TLS_ENABLED", "true");
        let _b = VarGuard::set("TLS_HTTP_ENABLED", "1");
        let _c = VarGuard::set("TLS_GRPC_ENABLED", "false");
        let _d = VarGuard::set("TLS_CERT_PATH", "/app/certs/server.crt");
        let _e = VarGuard::set("TLS_KEY_PATH", "/app/certs/server.key");
        let _f = VarGuard::set("TLS_CA_PATH", "/app/certs/ca.crt");

        let config = Config::load_from_env().unwrap();
        assert!(config.tls.enabled);
        assert!(config.tls.http_enabled);
        assert!(!config.tls.grpc_enabled);
        assert_eq!(config.tls.cert_path, "/app/certs/server.crt");
        assert_eq!(config.tls.key_path, "/app/certs/server.key");
        assert_eq!(config.tls.ca_path.as_deref(), Some("/app/certs/ca.crt"));
    }

    /// TLS_* 布尔非法值应返回 InvalidValue 且消息指明变量名。
    #[test]
    fn load_from_env_tls_enabled_invalid_fails() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _g = VarGuard::set("TLS_ENABLED", "maybe");
        let result = Config::load_from_env();
        assert_eq!(
            result.unwrap_err(),
            ConfigError::InvalidValue("TLS_ENABLED".to_string())
        );
    }

    /// merge 应把 env 的 TLS 覆盖应用到文件配置上；未设置的字段保持不变
    /// （`false` 不反向关闭文件里已启用的 TLS）。
    #[test]
    fn merge_applies_env_tls_overrides() {
        let _guard = ENV_LOCK.lock().unwrap();
        let mut base = Config::default();
        base.tls.enabled = true;
        base.tls.http_enabled = true;
        base.tls.grpc_enabled = true;
        base.tls.cert_path = "/file/server.crt".to_string();
        base.tls.key_path = "/file/server.key".to_string();

        let mut other = Config::default();
        other.tls.cert_path = "/env/server.crt".to_string();
        base.merge(other);

        assert!(
            base.tls.enabled,
            "env 未设 TLS_ENABLED 不得关闭文件中的 TLS"
        );
        assert!(base.tls.http_enabled);
        assert!(base.tls.grpc_enabled);
        assert_eq!(base.tls.cert_path, "/env/server.crt");
        assert_eq!(base.tls.key_path, "/file/server.key");
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

    /// 自定义 other 应覆盖 base 的所有可覆盖字段（覆盖各 if 的 true 分支）。
    ///
    /// 加 ENV_LOCK：`Config::default()` 读 `DATABASE_URL`，与并行 env 测试有时序竞争。
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

    /// 默认 other 不应覆盖 base 的自定义字段（覆盖各 if 的 false 分支）。
    ///
    /// 加 ENV_LOCK：`Config::default()` 读 `DATABASE_URL`，与并行 env 测试有时序竞争。
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
        // 显式设置触发 false 分支的字段（100 = merge 条件中的哨兵值）
        other.database.max_connections = 100;

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

    /// 显式「无 etcd」必须覆盖 base：`ETCD_ENDPOINTS=""` 经 `load_from_env` 解析为
    /// `[""]`（≠ 内置默认空列表）→ 覆盖成立，装配层按空白项＝未配置退回单进程本地锁。
    /// 配置文件形态 `endpoints = []` 直接作为 base 值，不经过本分支。
    ///
    /// 加 ENV_LOCK：`Config::default()` 读 `DATABASE_URL`，与并行 env 测试有时序竞争。
    #[test]
    fn merge_applies_explicit_empty_etcd_endpoints() {
        let _guard = ENV_LOCK.lock().unwrap();
        let mut base = Config::default();
        base.etcd.endpoints = vec!["custom:2379".to_string()];
        let mut other = Config::default();
        other.etcd.endpoints = vec![String::new()];
        base.merge(other);
        assert_eq!(base.etcd.endpoints, vec![String::new()]);
        assert!(!base.etcd.has_configured_endpoints());
    }

    /// 默认值（未显式设置）的 etcd.endpoints 不得覆盖 base：`load_from_env()`
    /// 未设 `ETCD_ENDPOINTS` 时 endpoints 等于内置默认值，此时文件配置是唯一来源。
    ///
    /// 加 ENV_LOCK：`Config::default()` 读 `DATABASE_URL`，与并行 env 测试有时序竞争。
    #[test]
    fn merge_preserves_file_etcd_endpoints_when_other_default() {
        let _guard = ENV_LOCK.lock().unwrap();
        let mut base = Config::default();
        base.etcd.endpoints = vec!["file-configured:2379".to_string()];
        let other = Config::default();
        assert_eq!(other.etcd.endpoints, EtcdConfig::default().endpoints);
        base.merge(other);
        assert_eq!(
            base.etcd.endpoints,
            vec!["file-configured:2379".to_string()]
        );
    }

    /// merge 相同的 database.url 不应覆盖（条件中 url != self.database.url 为 false）。
    ///
    /// 显式移除 DATABASE_URL：其他模块测试可能在本模块不持锁时设置该变量，
    /// 导致两次 `Config::default()` 返回不同 URL，破坏"相同不覆盖"的断言。
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

/// hot_reload.auto_watch_enabled 缺省 false、显式 true 往返保留、
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
