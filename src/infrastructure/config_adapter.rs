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

//! Configuration adapter for the confers ConfigProvider.
//!
//! This adapter bridges the gap between confers' generic key-value configuration
//! interface and Nebula ID's domain-specific configuration structures.

use crate::core::config::{
    AlgorithmConfig, AppConfig, AuthConfig, BatchGenerateConfig, DatabaseConfig, EtcdConfig,
    LogLevel, LoggingConfig, MonitoringConfig, RateLimitConfig, SegmentAlgorithmConfig,
    SnowflakeAlgorithmConfig, TlsConfig, UuidV7Config,
};
// ARCH-MED-002 修复：统一引用 auth 模块的常量，避免默认值重复定义。
use crate::core::config::auth::DEFAULT_KEY_ROTATION_GRACE_PERIOD_SECONDS;
use confers::interface::{ConfigProvider, ConfigProviderExt};
use std::sync::Arc;

/// Configuration adapter that wraps a confers ConfigProvider.
///
/// This adapter provides domain-specific configuration access methods
/// that map confers key-value pairs to Nebula ID's configuration structures.
///
/// # Key Mapping Convention
///
/// Configuration keys follow dot-notation pattern:
/// - `app.name` - Application name
/// - `algorithm.segment.base_step` - Segment algorithm base step
/// - `database.url` - Database connection URL
///
/// # Example
///
/// ```rust,ignore
/// use confers::Config;
/// use crate::core::infrastructure::ConfigAdapter;
/// use std::sync::Arc;
///
/// let config = Arc::new(Config::from_file("config.toml")?);
/// let adapter = ConfigAdapter::new(config);
///
/// let segment_config = adapter.get_segment_config();
/// println!("Base step: {}", segment_config.base_step);
/// ```
#[derive(Clone)]
pub struct ConfigAdapter {
    provider: Arc<dyn ConfigProvider>,
}

impl ConfigAdapter {
    /// Create a new configuration adapter with the given provider.
    ///
    /// # Arguments
    ///
    /// * `provider` - The configuration provider from confers
    pub fn new(provider: Arc<dyn ConfigProvider>) -> Self {
        Self { provider }
    }

    /// Get the underlying configuration provider.
    pub fn provider(&self) -> &Arc<dyn ConfigProvider> {
        &self.provider
    }

    /// Get the application configuration.
    ///
    /// Keys:
    /// - `app.name` - Application name
    /// - `app.host` - Server host
    /// - `app.http_port` - HTTP port
    /// - `app.grpc_port` - gRPC port
    /// - `app.dc_id` - Datacenter ID (0-31)
    /// - `app.worker_id` - Worker ID (0-255)
    pub fn get_app_config(&self) -> AppConfig {
        AppConfig {
            name: self
                .provider
                .get_string("app.name")
                .unwrap_or_else(|| "nebula-id".to_string()),
            host: self
                .provider
                .get_string("app.host")
                .unwrap_or_else(|| "0.0.0.0".to_string()),
            http_port: self.provider.get_int("app.http_port").unwrap_or(8080) as u16,
            grpc_port: self.provider.get_int("app.grpc_port").unwrap_or(9091) as u16,
            dc_id: self.provider.get_int("app.dc_id").unwrap_or(0) as u8,
            worker_id: self.provider.get_int("app.worker_id").unwrap_or(0) as u8,
        }
    }

    /// Get the segment algorithm configuration.
    ///
    /// Keys:
    /// - `algorithm.segment.base_step` - Base step size
    /// - `algorithm.segment.min_step` - Minimum step size
    /// - `algorithm.segment.max_step` - Maximum step size
    /// - `algorithm.segment.switch_threshold` - Dynamic adjustment threshold
    pub fn get_segment_config(&self) -> SegmentAlgorithmConfig {
        SegmentAlgorithmConfig {
            base_step: self
                .provider
                .get_int("algorithm.segment.base_step")
                .unwrap_or(1000) as u64,
            min_step: self
                .provider
                .get_int("algorithm.segment.min_step")
                .unwrap_or(500) as u64,
            max_step: self
                .provider
                .get_int("algorithm.segment.max_step")
                .unwrap_or(100000) as u64,
            switch_threshold: self
                .provider
                .get_float("algorithm.segment.switch_threshold")
                .unwrap_or(0.1),
        }
    }

    /// Get the snowflake algorithm configuration.
    ///
    /// Keys:
    /// - `algorithm.snowflake.datacenter_id_bits` - Bits for datacenter ID
    /// - `algorithm.snowflake.worker_id_bits` - Bits for worker ID
    /// - `algorithm.snowflake.sequence_bits` - Bits for sequence
    /// - `algorithm.snowflake.clock_drift_threshold_ms` - Clock drift threshold
    pub fn get_snowflake_config(&self) -> SnowflakeAlgorithmConfig {
        SnowflakeAlgorithmConfig {
            datacenter_id_bits: self
                .provider
                .get_int("algorithm.snowflake.datacenter_id_bits")
                .unwrap_or(3) as u8,
            worker_id_bits: self
                .provider
                .get_int("algorithm.snowflake.worker_id_bits")
                .unwrap_or(8) as u8,
            sequence_bits: self
                .provider
                .get_int("algorithm.snowflake.sequence_bits")
                .unwrap_or(10) as u8,
            clock_drift_threshold_ms: self
                .provider
                .get_int("algorithm.snowflake.clock_drift_threshold_ms")
                .unwrap_or(1000) as u64,
        }
    }

    /// Get the UUID v7 configuration.
    ///
    /// Keys:
    /// - `algorithm.uuid_v7.enabled` - Enable UUID v7 generation
    pub fn get_uuid_v7_config(&self) -> UuidV7Config {
        UuidV7Config {
            enabled: self
                .provider
                .get_bool("algorithm.uuid_v7.enabled")
                .unwrap_or(true),
        }
    }

    /// Get the complete algorithm configuration.
    ///
    /// Keys:
    /// - `algorithm.default` - Default algorithm type
    /// - Plus all segment, snowflake, and uuid_v7 keys
    pub fn get_algorithm_config(&self) -> AlgorithmConfig {
        AlgorithmConfig {
            default: self
                .provider
                .get_string("algorithm.default")
                .unwrap_or_else(|| "segment".to_string()),
            segment: self.get_segment_config(),
            snowflake: self.get_snowflake_config(),
            uuid_v7: self.get_uuid_v7_config(),
        }
    }

    /// Get the authentication configuration.
    ///
    /// Keys:
    /// - `auth.enabled` - Enable authentication
    /// - `auth.cache_ttl_seconds` - Cache TTL
    /// - `auth.api_key_salt` - Salt for API key hashing
    ///
    /// Phase 9 T043 (HIGH H1 / tiangang HIGH-1) — `api_key_salt` no
    /// longer falls back to a hard-coded value. If unset, returns an
    /// empty string; `AuthManager::from_env()` enforces the
    /// production-must-set-env rule (panic on missing salt in release
    /// builds, random salt in dev builds).
    pub fn get_auth_config(&self) -> AuthConfig {
        AuthConfig {
            enabled: self.provider.get_bool("auth.enabled").unwrap_or(true),
            cache_ttl_seconds: self
                .provider
                .get_int("auth.cache_ttl_seconds")
                .unwrap_or(300) as u64,
            api_keys: vec![], // API keys are typically loaded from database
            api_key_salt: self
                .provider
                .get_string("auth.api_key_salt")
                .or_else(|| std::env::var("NEBULA_API_KEY_SALT").ok())
                .unwrap_or_default(),
            // L16 修复：从配置读取密钥轮换宽限期，默认 7 天。
            key_rotation_grace_period_seconds: self
                .provider
                .get_int("auth.key_rotation_grace_period_seconds")
                .map(|v| v as u64)
                .unwrap_or(DEFAULT_KEY_ROTATION_GRACE_PERIOD_SECONDS),
        }
    }

    /// Get the database configuration.
    ///
    /// Keys:
    /// - `database.url` - Database connection URL
    /// - `database.max_connections` - Maximum connections
    /// - `database.min_connections` - Minimum connections
    /// - `database.acquire_timeout_seconds` - Acquisition timeout
    /// - `database.idle_timeout_seconds` - Idle timeout
    ///
    /// Phase 9 T043 (HIGH H2 / tiangang MEDIUM-1) — no hard-coded
    /// production fallback. Dev/test keeps `postgresql://localhost/nebula`
    /// for convenience; release builds require `DATABASE_URL` /
    /// `database.url` to be set explicitly (the `AppConfig` validator
    /// in `core/config/app.rs` rejects empty URLs in production).
    pub fn get_database_config(&self) -> DatabaseConfig {
        let url = self.provider.get_string("database.url").unwrap_or_else(|| {
            std::env::var("DATABASE_URL").unwrap_or_else(|_| {
                if cfg!(debug_assertions) {
                    "postgresql://localhost/nebula".to_string()
                } else {
                    String::new()
                }
            })
        });

        DatabaseConfig {
            url,
            max_connections: self
                .provider
                .get_int("database.max_connections")
                .unwrap_or(100) as u32,
            min_connections: self
                .provider
                .get_int("database.min_connections")
                .unwrap_or(10) as u32,
            acquire_timeout_seconds: self
                .provider
                .get_int("database.acquire_timeout_seconds")
                .unwrap_or(30) as u64,
            idle_timeout_seconds: self
                .provider
                .get_int("database.idle_timeout_seconds")
                .unwrap_or(300) as u64,
            ..Default::default()
        }
    }

    /// Get the etcd configuration.
    ///
    /// Keys:
    /// - `etcd.endpoints` - Comma-separated list of endpoints
    /// - `etcd.connect_timeout_ms` - Connection timeout
    /// - `etcd.watch_timeout_ms` - Watch timeout
    ///
    /// Phase 9 T043 (HIGH H2) — `localhost:2379` is only used as a
    /// dev convenience. Release builds return an empty endpoint list
    /// so misconfiguration fails loudly instead of silently connecting
    /// to an unauthenticated local etcd.
    pub fn get_etcd_config(&self) -> EtcdConfig {
        let endpoints_str = self
            .provider
            .get_string("etcd.endpoints")
            .unwrap_or_else(|| {
                std::env::var("ETCD_ENDPOINTS").unwrap_or_else(|_| {
                    if cfg!(debug_assertions) {
                        "localhost:2379".to_string()
                    } else {
                        String::new()
                    }
                })
            });

        EtcdConfig {
            endpoints: endpoints_str
                .split(',')
                .filter(|s| !s.trim().is_empty())
                .map(String::from)
                .collect(),
            connect_timeout_ms: self
                .provider
                .get_int("etcd.connect_timeout_ms")
                .unwrap_or(5000) as u64,
            watch_timeout_ms: self
                .provider
                .get_int("etcd.watch_timeout_ms")
                .unwrap_or(5000) as u64,
        }
    }

    /// Get the monitoring configuration.
    ///
    /// Keys:
    /// - `monitoring.metrics_enabled` - Enable Prometheus metrics
    /// - `monitoring.metrics_path` - Metrics endpoint path
    /// - `monitoring.tracing_enabled` - Enable OpenTelemetry tracing
    /// - `monitoring.otlp_endpoint` - OTLP collector endpoint
    pub fn get_monitoring_config(&self) -> MonitoringConfig {
        MonitoringConfig {
            metrics_enabled: self
                .provider
                .get_bool("monitoring.metrics_enabled")
                .unwrap_or(true),
            metrics_path: self
                .provider
                .get_string("monitoring.metrics_path")
                .unwrap_or_else(|| "/metrics".to_string()),
            tracing_enabled: self
                .provider
                .get_bool("monitoring.tracing_enabled")
                .unwrap_or(false),
            otlp_endpoint: self
                .provider
                .get_string("monitoring.otlp_endpoint")
                .unwrap_or_default(),
        }
    }

    /// Get the logging configuration.
    ///
    /// Keys:
    /// - `logging.level` - Log level (trace/debug/info/warn/error)
    /// - `logging.format` - Log format (json/pretty)
    /// - `logging.include_location` - Include source location
    pub fn get_logging_config(&self) -> LoggingConfig {
        let level_str = self
            .provider
            .get_string("logging.level")
            .unwrap_or_else(|| "info".to_string());

        let format_str = self
            .provider
            .get_string("logging.format")
            .unwrap_or_else(|| "json".to_string());

        LoggingConfig {
            level: LogLevel::from(level_str.as_str()),
            format: crate::core::config::LogFormat::from(format_str.as_str()),
            include_location: self
                .provider
                .get_bool("logging.include_location")
                .unwrap_or(true),
        }
    }

    /// Get the rate limiting configuration.
    ///
    /// Keys:
    /// - `rate_limit.enabled` - Enable rate limiting
    /// - `rate_limit.default_rps` - Default requests per second
    /// - `rate_limit.burst_size` - Burst size
    pub fn get_rate_limit_config(&self) -> RateLimitConfig {
        RateLimitConfig {
            enabled: self.provider.get_bool("rate_limit.enabled").unwrap_or(true),
            default_rps: self
                .provider
                .get_int("rate_limit.default_rps")
                .unwrap_or(10000) as u32,
            burst_size: self
                .provider
                .get_int("rate_limit.burst_size")
                .unwrap_or(100) as u32,
        }
    }

    /// Get the TLS configuration.
    ///
    /// Keys:
    /// - `tls.enabled` - Enable TLS
    /// - `tls.cert_path` - Certificate path
    /// - `tls.key_path` - Private key path
    /// - `tls.ca_path` - CA certificate path (optional)
    /// - `tls.http_enabled` - Enable TLS for HTTP
    /// - `tls.grpc_enabled` - Enable TLS for gRPC
    pub fn get_tls_config(&self) -> TlsConfig {
        TlsConfig {
            enabled: self.provider.get_bool("tls.enabled").unwrap_or(false),
            cert_path: self
                .provider
                .get_string("tls.cert_path")
                .unwrap_or_default(),
            key_path: self.provider.get_string("tls.key_path").unwrap_or_default(),
            ca_path: self.provider.get_string("tls.ca_path"),
            http_enabled: self.provider.get_bool("tls.http_enabled").unwrap_or(false),
            grpc_enabled: self.provider.get_bool("tls.grpc_enabled").unwrap_or(false),
            ..Default::default()
        }
    }

    /// Get the batch generation configuration.
    ///
    /// Keys:
    /// - `batch_generate.max_batch_size` - Maximum batch size
    pub fn get_batch_generate_config(&self) -> BatchGenerateConfig {
        BatchGenerateConfig {
            max_batch_size: self
                .provider
                .get_int("batch_generate.max_batch_size")
                .unwrap_or(100) as u32,
        }
    }

    /// Get Redis configuration.
    ///
    /// Keys:
    /// - `redis.url` - Redis connection URL
    /// - `redis.pool_size` - Connection pool size
    /// - `redis.key_prefix` - Key prefix for cache entries
    /// - `redis.ttl_seconds` - Default TTL in seconds
    ///
    /// Phase 9 T043 (HIGH H2) — `redis://localhost:6379` is dev-only.
    /// Release builds require `REDIS_URL` / `redis.url` to be set
    /// explicitly so misconfiguration fails loudly instead of silently
    /// connecting to an unauthenticated local Redis.
    pub fn get_redis_config(&self) -> crate::core::config::RedisConfig {
        crate::core::config::RedisConfig {
            url: self.provider.get_string("redis.url").unwrap_or_else(|| {
                std::env::var("REDIS_URL").unwrap_or_else(|_| {
                    if cfg!(debug_assertions) {
                        "redis://localhost:6379".to_string()
                    } else {
                        String::new()
                    }
                })
            }),
            pool_size: self.provider.get_int("redis.pool_size").unwrap_or(16) as u32,
            key_prefix: self
                .provider
                .get_string("redis.key_prefix")
                .unwrap_or_else(|| "nebula:id:".to_string()),
            ttl_seconds: self.provider.get_int("redis.ttl_seconds").unwrap_or(600) as u64,
        }
    }

    /// Get the complete configuration.
    ///
    /// This method assembles all configuration sections into a single Config struct.
    pub fn get_config(&self) -> crate::core::config::Config {
        crate::core::config::Config {
            app: self.get_app_config(),
            database: self.get_database_config(),
            redis: self.get_redis_config(),
            etcd: self.get_etcd_config(),
            auth: self.get_auth_config(),
            algorithm: self.get_algorithm_config(),
            monitoring: self.get_monitoring_config(),
            logging: self.get_logging_config(),
            rate_limit: self.get_rate_limit_config(),
            tls: self.get_tls_config(),
            batch_generate: self.get_batch_generate_config(),
        }
    }

    /// Get a raw string value from the provider.
    pub fn get_string(&self, key: &str) -> Option<String> {
        self.provider.get_string(key)
    }

    /// Get a raw integer value from the provider.
    pub fn get_int(&self, key: &str) -> Option<i64> {
        self.provider.get_int(key)
    }

    /// Get a raw boolean value from the provider.
    pub fn get_bool(&self, key: &str) -> Option<bool> {
        self.provider.get_bool(key)
    }

    /// Get a raw float value from the provider.
    pub fn get_float(&self, key: &str) -> Option<f64> {
        self.provider.get_float(key)
    }
}

impl std::fmt::Debug for ConfigAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConfigAdapter")
            .field("provider", &"Arc<dyn ConfigProvider>")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use confers::types::ConfigValue;

    /// Mock ConfigProvider for testing
    struct MockConfigProvider {
        values: std::collections::HashMap<String, confers::types::AnnotatedValue>,
    }

    impl MockConfigProvider {
        fn new() -> Self {
            Self {
                values: std::collections::HashMap::new(),
            }
        }

        #[allow(dead_code)]
        fn with_string(mut self, key: &str, value: &str) -> Self {
            self.values.insert(
                key.to_string(),
                confers::types::AnnotatedValue::from(ConfigValue::string(value.to_string())),
            );
            self
        }

        #[allow(dead_code)]
        fn with_int(mut self, key: &str, value: i64) -> Self {
            self.values.insert(
                key.to_string(),
                confers::types::AnnotatedValue::from(ConfigValue::integer(value)),
            );
            self
        }

        #[allow(dead_code)]
        fn with_bool(mut self, key: &str, value: bool) -> Self {
            self.values.insert(
                key.to_string(),
                confers::types::AnnotatedValue::from(ConfigValue::bool(value)),
            );
            self
        }
    }

    impl ConfigProvider for MockConfigProvider {
        fn get_raw(&self, key: &str) -> Option<&confers::types::AnnotatedValue> {
            self.values.get(key)
        }

        fn keys(&self) -> Vec<String> {
            self.values.keys().cloned().collect()
        }
    }

    #[test]
    fn test_get_segment_config_defaults() {
        let provider = Arc::new(MockConfigProvider::new());
        let adapter = ConfigAdapter::new(provider);

        let config = adapter.get_segment_config();
        assert_eq!(config.base_step, 1000);
        assert_eq!(config.min_step, 500);
        assert_eq!(config.max_step, 100000);
    }

    #[test]
    fn test_get_segment_config_custom() {
        let provider = Arc::new(
            MockConfigProvider::new()
                .with_int("algorithm.segment.base_step", 2000)
                .with_int("algorithm.segment.min_step", 1000)
                .with_int("algorithm.segment.max_step", 200000),
        );
        let adapter = ConfigAdapter::new(provider);

        let config = adapter.get_segment_config();
        assert_eq!(config.base_step, 2000);
        assert_eq!(config.min_step, 1000);
        assert_eq!(config.max_step, 200000);
    }

    #[test]
    fn test_get_snowflake_config_defaults() {
        let provider = Arc::new(MockConfigProvider::new());
        let adapter = ConfigAdapter::new(provider);

        let config = adapter.get_snowflake_config();
        assert_eq!(config.datacenter_id_bits, 3);
        assert_eq!(config.worker_id_bits, 8);
        assert_eq!(config.sequence_bits, 10);
    }

    #[test]
    fn test_get_app_config_defaults() {
        let provider = Arc::new(MockConfigProvider::new());
        let adapter = ConfigAdapter::new(provider);

        let config = adapter.get_app_config();
        assert_eq!(config.name, "nebula-id");
        assert_eq!(config.host, "0.0.0.0");
        assert_eq!(config.http_port, 8080);
    }
}
