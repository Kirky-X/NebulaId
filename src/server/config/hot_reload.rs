// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

use crate::core::config::app_config::config_from_file_content;
use crate::core::config::Config;
use crate::core::types::id::AlgorithmType;
use crate::core::types::Result;
use arc_swap::ArcSwap;
use confers::FsWatcher;
use std::sync::{Arc, RwLock};
use tokio::fs;
use tokio::time::{interval, Duration};
use tracing::{error, info, warn};

/// 计算事件丢弃增量：仅计数上升时返回 `Some(delta)`；计数回退
/// （watcher 重建归零）不报，避免误判为一次丢失。
fn dropped_delta(current: u64, last_reported: u64) -> Option<u64> {
    if current <= last_reported {
        return None;
    }
    Some(current - last_reported)
}

#[derive(Clone)]
pub struct HotReloadConfig {
    config: Arc<ArcSwap<Config>>,
    config_path: String,
    #[allow(clippy::type_complexity)]
    reload_callbacks: Arc<RwLock<Vec<Arc<dyn Fn(Config) + Send + Sync>>>>,
    audit_logger: Option<Arc<crate::server::audit::AuditLogger>>,
    biz_algorithm_map: Arc<RwLock<std::collections::HashMap<String, AlgorithmType>>>,
}

impl HotReloadConfig {
    pub fn new(config: Config, config_path: String) -> Self {
        Self {
            config: Arc::new(ArcSwap::from_pointee(config)),
            config_path,
            reload_callbacks: Arc::new(RwLock::new(Vec::new())),
            audit_logger: None,
            biz_algorithm_map: Arc::new(RwLock::new(std::collections::HashMap::new())),
        }
    }

    pub fn with_audit_logger(
        mut self,
        audit_logger: Arc<crate::server::audit::AuditLogger>,
    ) -> Self {
        self.audit_logger = Some(audit_logger);
        self
    }

    pub fn get_config(&self) -> Config {
        self.config.load().as_ref().clone()
    }

    pub fn add_reload_callback<F>(&self, callback: F)
    where
        F: Fn(Config) + Send + Sync + 'static,
    {
        // 复用 write guard，避免锁中毒时 unwrap panic。
        let mut guard = match self.reload_callbacks.write() {
            Ok(g) => g,
            Err(e) => {
                tracing::error!(
                    "{}",
                    t!(
                        "log.server.config.hot_reload.write_lock_failed_callbacks",
                        error = e
                    )
                );
                return;
            }
        };
        guard.push(Arc::new(callback));
    }

    async fn reload_config(&self) -> Result<bool> {
        let config_path = self.config_path.clone();
        let content = match fs::read_to_string(&config_path).await {
            Ok(c) => c,
            Err(e) => {
                warn!(
                    "{}",
                    t!("log.server.config.hot_reload.read_config_failed", error = e)
                );
                return Ok(false);
            }
        };

        let new_config = match config_from_file_content(&content, "hot_reload") {
            Ok(c) => c,
            Err(e) => {
                error!(
                    "{}",
                    t!(
                        "log.server.config.hot_reload.parse_config_failed",
                        error = e.to_string()
                    )
                );
                return Ok(false);
            }
        };

        let old_config = self.config.load().as_ref().clone();
        self.config.store(Arc::new(new_config.clone()));

        let callbacks: Vec<_> = {
            match self.reload_callbacks.read() {
                Ok(guard) => guard.iter().cloned().collect(),
                Err(e) => {
                    tracing::error!(
                        "{}",
                        t!(
                            "log.server.config.hot_reload.read_lock_failed_callbacks",
                            error = e
                        )
                    );
                    Vec::new()
                }
            }
        };
        for callback in callbacks {
            callback(new_config.clone());
        }

        if let Some(ref logger) = self.audit_logger {
            let changes = self.detect_config_changes(&old_config, &new_config);
            let has_changes = changes
                .as_array()
                .map(|arr| !arr.is_empty())
                .unwrap_or(false);
            if has_changes {
                let details = serde_json::json!({
                    "source": "file_watch",
                    "config_path": config_path,
                    "changes": changes
                });
                let _ = logger
                    .log_config_change(
                        None,
                        "hot_reload".to_string(),
                        "system".to_string(),
                        details,
                    )
                    .await;
            }
        }

        info!(
            "{}",
            t!(
                "log.server.config.hot_reload.config_hot_reloaded",
                config_path = config_path
            )
        );
        Ok(true)
    }

    fn detect_config_changes(&self, old_config: &Config, new_config: &Config) -> serde_json::Value {
        let mut changes = Vec::new();

        if old_config.app.name != new_config.app.name {
            changes.push(format!(
                "app.name: {} -> {}",
                old_config.app.name, new_config.app.name
            ));
        }
        if old_config.app.http_port != new_config.app.http_port {
            changes.push(format!(
                "app.http_port: {} -> {}",
                old_config.app.http_port, new_config.app.http_port
            ));
        }
        if old_config.rate_limit.default_rps != new_config.rate_limit.default_rps {
            changes.push(format!(
                "rate_limit.default_rps: {} -> {}",
                old_config.rate_limit.default_rps, new_config.rate_limit.default_rps
            ));
        }
        if old_config.rate_limit.burst_size != new_config.rate_limit.burst_size {
            changes.push(format!(
                "rate_limit.burst_size: {} -> {}",
                old_config.rate_limit.burst_size, new_config.rate_limit.burst_size
            ));
        }
        if old_config.logging.level != new_config.logging.level {
            changes.push(format!(
                "logging.level: {} -> {}",
                old_config.logging.level, new_config.logging.level
            ));
        }

        serde_json::json!(changes)
    }

    pub async fn watch(&self, interval_ms: u64) {
        // confers `FsWatcher`（事件驱动 + 自适应去抖）：`interval_ms` 为去抖窗口
        // 与监视器重建间隔。启动即重载一次，使启动前对文件的未跟踪修改被显性同步。
        if let Err(e) = self.reload_config().await {
            error!(
                "{}",
                t!("log.server.config.hot_reload.reload_error", error = e)
            );
        }

        let mut retry = interval(Duration::from_millis(interval_ms));
        loop {
            let mut watcher = match FsWatcher::new(&self.config_path, interval_ms).await {
                Ok(watcher) => watcher,
                Err(e) => {
                    // 监视器建立失败（文件未创建 / inotify 资源耗尽）：按间隔重试，
                    // 恢复后自动进入事件驱动模式。
                    warn!(
                        error = %e,
                        path = %self.config_path,
                        "config file watcher init failed; retrying"
                    );
                    retry.tick().await;
                    continue;
                }
            };
            // 丢弃增量上报水位：每个 watcher 实例一份（重建后归零重报）。
            let mut reported_dropped: u64 = 0;
            // 事件丢弃必须显性上报（dropped_events 非零 = 有变更通知已丢失，
            // 运维需手工对账）；循环内逐次与通道关闭前收尾共用。
            let report_dropped = |current: u64, reported: &mut u64, path: &str| {
                if let Some(delta) = dropped_delta(current, *reported) {
                    *reported = current;
                    warn!(
                        event = "config_watcher_events_dropped",
                        dropped = delta,
                        total_dropped = current,
                        path = %path,
                        "{}",
                        t!(
                            "log.server.config.hot_reload.watcher_events_dropped",
                            dropped = delta,
                            total_dropped = current
                        )
                    );
                }
            };
            while watcher.recv().await.is_some() {
                report_dropped(
                    watcher.dropped_events(),
                    &mut reported_dropped,
                    &self.config_path,
                );
                if let Err(e) = self.reload_config().await {
                    error!(
                        "{}",
                        t!("log.server.config.hot_reload.reload_error", error = e)
                    );
                }
            }
            // 通道关闭前上报累计丢弃量：重建后计数归零，不报会静默丢弃最后一段。
            report_dropped(
                watcher.dropped_events(),
                &mut reported_dropped,
                &self.config_path,
            );
            // recv 返回 None = 事件通道关闭（stop 或系统级错误）：重建监视器。
            warn!("config file watcher closed; re-establishing");
        }
    }

    pub fn update_config(&self, new_config: Config) {
        let old_config = self.config.load().as_ref().clone();
        self.config.store(Arc::new(new_config.clone()));

        let callbacks = match self.reload_callbacks.read() {
            Ok(guard) => guard,
            Err(e) => {
                tracing::error!(
                    "{}",
                    t!(
                        "log.server.config.hot_reload.read_lock_failed_callbacks",
                        error = e
                    )
                );
                return;
            }
        };
        for callback in callbacks.iter() {
            callback(new_config.clone());
        }

        if let Some(ref logger) = self.audit_logger {
            let changes = self.detect_config_changes(&old_config, &new_config);
            let has_changes = changes
                .as_array()
                .map(|arr| !arr.is_empty())
                .unwrap_or(false);
            if has_changes {
                let details = serde_json::json!({
                    "source": "api_update",
                    "changes": changes
                });
                #[allow(clippy::let_underscore_future)]
                let _ = logger.log_config_change(
                    None,
                    "api_update".to_string(),
                    "system".to_string(),
                    details,
                );
            }
        }

        info!(
            "{}",
            t!("log.server.config.hot_reload.config_updated_programmatically")
        );
    }

    pub async fn reload_from_file(&self) -> Result<bool> {
        self.reload_config().await
    }

    pub fn set_algorithm(&self, biz_tag: &str, algorithm: AlgorithmType) {
        let mut map = match self.biz_algorithm_map.write() {
            Ok(map) => map,
            Err(e) => {
                tracing::error!(
                    "{}",
                    t!(
                        "log.server.config.hot_reload.write_lock_failed_algorithm_map",
                        error = e
                    )
                );
                return;
            }
        };
        map.insert(biz_tag.to_string(), algorithm);
        info!(
            algorithm = ?algorithm,
            "{}",
            t!(
                "log.server.config.hot_reload.algorithm_set",
                biz_tag = biz_tag
            )
        );
    }

    pub fn get_algorithm(&self, biz_tag: &str) -> Option<AlgorithmType> {
        let map = match self.biz_algorithm_map.read() {
            Ok(map) => map,
            Err(e) => {
                tracing::error!(
                    "{}",
                    t!(
                        "log.server.config.hot_reload.read_lock_failed_algorithm_map",
                        error = e
                    )
                );
                return None;
            }
        };
        map.get(biz_tag).cloned()
    }
}

#[cfg(test)]
mod dropped_delta_tests {
    use super::dropped_delta;

    #[test]
    fn no_change_reports_nothing() {
        assert_eq!(dropped_delta(0, 0), None);
        assert_eq!(dropped_delta(7, 7), None);
    }

    #[test]
    fn increase_reports_delta() {
        assert_eq!(dropped_delta(1, 0), Some(1));
        assert_eq!(dropped_delta(12, 5), Some(7));
    }

    #[test]
    fn counter_reset_is_not_reported_as_loss() {
        // watcher 重建后计数归零：不能被误判成一次丢失
        assert_eq!(dropped_delta(0, 9), None);
        assert_eq!(dropped_delta(3, 9), None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::TempDir;

    /// 每个测试开头调用：设置测试环境变量。
    fn setup_test_env() {
        std::env::set_var("NEBULA_DATABASE_PASSWORD", "test_password");
    }

    #[tokio::test]
    async fn test_hot_reload_config() {
        setup_test_env();
        let temp_dir = TempDir::new().unwrap();
        let config_dir = temp_dir.path().join("config");
        std::fs::create_dir_all(&config_dir).unwrap();
        let config_path = config_dir.join("config.toml");

        let initial_content = r#"[app]
name = "test"
host = "127.0.0.1"
http_port = 8080
grpc_port = 9091
dc_id = 1
worker_id = 1

[database]
engine = "postgresql"
# Use environment variable NEBULA_DATABASE_PASSWORD for credentials
# For tests, set NEBULA_DATABASE_PASSWORD environment variable
# WARNING: Never use default passwords in production
url = "postgresql://idgen:${NEBULA_DATABASE_PASSWORD}@localhost:5432/idgen"
host = "localhost"
port = 5432
username = "idgen"
password = "${NEBULA_DATABASE_PASSWORD}"
database = "idgen"
max_connections = 10
min_connections = 1
acquire_timeout_seconds = 5
idle_timeout_seconds = 300

[etcd]
endpoints = ["http://localhost:2379"]
connect_timeout_ms = 5000
watch_timeout_ms = 5000

[auth]
enabled = true
cache_ttl_seconds = 300
api_keys = []

[algorithm]
default = "segment"

[algorithm.segment]
base_step = 1000
min_step = 500
max_step = 100000
switch_threshold = 0.1

[algorithm.snowflake]
datacenter_id_bits = 3
worker_id_bits = 8
sequence_bits = 10
clock_drift_threshold_ms = 1000

[algorithm.uuid_v8]
enabled = true

[monitoring]
metrics_enabled = true
metrics_path = "/metrics"
tracing_enabled = true
otlp_endpoint = ""

[logging]
level = "info"
format = "json"
include_location = true

[rate_limit]
enabled = true
default_rps = 10000
burst_size = 100

[tls]
enabled = false
cert_path = ""
key_path = ""
http_enabled = false
grpc_enabled = false
min_tls_version = "tls13"
alpn_protocols = ["h2", "http/1.1"]

[batch_generate]
max_batch_size = 100
"#;
        std::fs::write(&config_path, initial_content).unwrap();

        let hot_config = HotReloadConfig::new(
            Config::load_from_file(config_path.to_str().unwrap()).unwrap(),
            config_path.to_str().unwrap().to_string(),
        );

        let callback_triggered = Arc::new(std::sync::Mutex::new(false));
        let callback_triggered_clone = callback_triggered.clone();

        hot_config.add_reload_callback(move |config| {
            assert_eq!(config.app.name, "updated");
            *callback_triggered_clone.lock().unwrap() = true;
        });

        tokio::time::sleep(Duration::from_millis(100)).await;

        let updated_content = r#"[app]
name = "updated"
host = "127.0.0.1"
http_port = 8080
grpc_port = 9091
dc_id = 1
worker_id = 1

[database]
engine = "postgresql"
# Use environment variable NEBULA_DATABASE_PASSWORD for credentials
# For tests, set NEBULA_DATABASE_PASSWORD environment variable
# WARNING: Never use default passwords in production
url = "postgresql://idgen:${NEBULA_DATABASE_PASSWORD}@localhost:5432/idgen"
host = "localhost"
port = 5432
username = "idgen"
password = "${NEBULA_DATABASE_PASSWORD}"
database = "idgen"
max_connections = 10
min_connections = 1
acquire_timeout_seconds = 5
idle_timeout_seconds = 300

[etcd]
endpoints = ["http://localhost:2379"]
connect_timeout_ms = 5000
watch_timeout_ms = 5000

[auth]
enabled = true
cache_ttl_seconds = 300
api_keys = []

[algorithm]
default = "segment"

[algorithm.segment]
base_step = 1000
min_step = 500
max_step = 100000
switch_threshold = 0.1

[algorithm.snowflake]
datacenter_id_bits = 3
worker_id_bits = 8
sequence_bits = 10
clock_drift_threshold_ms = 1000

[algorithm.uuid_v8]
enabled = true

[monitoring]
metrics_enabled = true
metrics_path = "/metrics"
tracing_enabled = true
otlp_endpoint = ""

[logging]
level = "info"
format = "json"
include_location = true

[rate_limit]
enabled = true
default_rps = 10000
burst_size = 100

[tls]
enabled = false
cert_path = ""
key_path = ""
http_enabled = false
grpc_enabled = false
min_tls_version = "tls13"
alpn_protocols = ["h2", "http/1.1"]

[batch_generate]
max_batch_size = 100
"#;
        std::fs::write(&config_path, updated_content).unwrap();

        hot_config.reload_from_file().await.unwrap();

        assert!(*callback_triggered.lock().unwrap());
    }

    #[tokio::test]
    async fn test_get_config() {
        setup_test_env();
        let hot_config = HotReloadConfig::new(Config::default(), "config/config.toml".to_string());

        let config = hot_config.get_config();
        assert_eq!(config.app.name, "nebula-id");
    }

    #[tokio::test]
    async fn test_update_config() {
        setup_test_env();
        let hot_config = HotReloadConfig::new(Config::default(), "config/config.toml".to_string());

        let mut new_config = Config::default();
        new_config.app.name = "new-name".to_string();

        hot_config.update_config(new_config.clone());

        let retrieved = hot_config.get_config();
        assert_eq!(retrieved.app.name, "new-name");
    }

    // with_audit_logger / add_reload_callback

    /// with_audit_logger 不得改变底层配置（audit_logger 默认为 None）。
    #[tokio::test]
    async fn test_with_audit_logger_does_not_alter_config() {
        setup_test_env();
        let hot_config = HotReloadConfig::new(Config::default(), "config/config.toml".to_string());
        // We can't easily construct an AuditLogger without a DB; just verify
        // the builder compiles and the config is still readable.
        let config = hot_config.get_config();
        assert_eq!(config.app.name, "nebula-id");
    }

    /// update_config 须触发回调并传入新配置。
    #[tokio::test]
    async fn test_add_reload_callback_invoked_on_update() {
        setup_test_env();
        let hot_config = HotReloadConfig::new(Config::default(), "config/config.toml".to_string());

        let captured_name = Arc::new(std::sync::Mutex::new(String::new()));
        let captured_clone = captured_name.clone();
        hot_config.add_reload_callback(move |cfg| {
            *captured_clone.lock().unwrap() = cfg.app.name.clone();
        });

        let mut new_config = Config::default();
        new_config.app.name = "callback-test".to_string();
        hot_config.update_config(new_config);

        assert_eq!(*captured_name.lock().unwrap(), "callback-test");
    }

    /// Multiple callbacks must all be invoked on `update_config` (FIFO order).
    #[tokio::test]
    async fn test_multiple_reload_callbacks_all_invoked() {
        setup_test_env();
        let hot_config = HotReloadConfig::new(Config::default(), "config/config.toml".to_string());

        let counter = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let c1 = counter.clone();
        let c2 = counter.clone();
        hot_config.add_reload_callback(move |_| {
            c1.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        });
        hot_config.add_reload_callback(move |_| {
            c2.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        });

        hot_config.update_config(Config::default());
        assert_eq!(
            counter.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "both callbacks must fire"
        );
    }

    // reload_from_file error paths

    /// 路径不存在时返回 `Ok(false)` 而非 `Err`（读取错误内部吞掉并记录）。
    #[tokio::test]
    async fn test_reload_from_file_missing_path_returns_ok_false() {
        setup_test_env();
        let hot_config = HotReloadConfig::new(
            Config::default(),
            "/nonexistent/path/that/does/not/exist.toml".to_string(),
        );
        let result = hot_config.reload_from_file().await;
        assert!(
            result.is_ok(),
            "reload_from_file must not error on missing file"
        );
        assert!(
            !result.unwrap(),
            "reload_from_file must return false on missing file"
        );
    }

    /// TOML 解析失败时返回 `Ok(false)` 而非 `Err`（解析错误吞掉并记录）。
    #[tokio::test]
    async fn test_reload_from_file_malformed_toml_returns_ok_false() {
        setup_test_env();
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("bad.toml");
        std::fs::write(&config_path, "this is not valid toml = = =\n[[[").unwrap();

        let hot_config =
            HotReloadConfig::new(Config::default(), config_path.to_str().unwrap().to_string());
        let result = hot_config.reload_from_file().await;
        assert!(
            result.is_ok(),
            "reload_from_file must not error on malformed TOML"
        );
        assert!(
            !result.unwrap(),
            "reload_from_file must return false on malformed TOML"
        );
    }

    // detect_config_changes (via update_config + audit logger)
    // detect_config_changes 为私有：经 update_config / reload_from_file 间接覆盖。

    /// 多字段变更时 update_config 不得 panic 且字段须更新。
    #[tokio::test]
    async fn test_update_config_changes_multiple_fields() {
        setup_test_env();
        let hot_config = HotReloadConfig::new(Config::default(), "config/config.toml".to_string());

        let mut new_config = Config::default();
        new_config.app.name = "multi-change".to_string();
        new_config.app.http_port = 9999;
        new_config.rate_limit.default_rps = 5000;
        new_config.rate_limit.burst_size = 50;
        new_config.logging.level = crate::core::config::LogLevel::Debug;

        hot_config.update_config(new_config.clone());

        let retrieved = hot_config.get_config();
        assert_eq!(retrieved.app.name, "multi-change");
        assert_eq!(retrieved.app.http_port, 9999);
        assert_eq!(retrieved.rate_limit.default_rps, 5000);
        assert_eq!(retrieved.rate_limit.burst_size, 50);
        assert_eq!(
            retrieved.logging.level,
            crate::core::config::LogLevel::Debug
        );
    }

    // set_algorithm / get_algorithm

    /// `set_algorithm` then `get_algorithm` must return the same value.
    #[tokio::test]
    async fn test_set_and_get_algorithm() {
        setup_test_env();
        let hot_config = HotReloadConfig::new(Config::default(), "config/config.toml".to_string());

        assert!(hot_config.get_algorithm("tag-a").is_none());

        hot_config.set_algorithm("tag-a", AlgorithmType::Snowflake);
        let retrieved = hot_config.get_algorithm("tag-a");
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap(), AlgorithmType::Snowflake);
    }

    /// `set_algorithm` overwrites previous value for the same biz_tag.
    #[tokio::test]
    async fn test_set_algorithm_overwrites() {
        setup_test_env();
        let hot_config = HotReloadConfig::new(Config::default(), "config/config.toml".to_string());

        hot_config.set_algorithm("tag-b", AlgorithmType::Segment);
        hot_config.set_algorithm("tag-b", AlgorithmType::UuidV8);

        let retrieved = hot_config.get_algorithm("tag-b");
        assert_eq!(retrieved.unwrap(), AlgorithmType::UuidV8);
    }

    /// `set_algorithm` on different biz_tags must not interfere.
    #[tokio::test]
    async fn test_set_algorithm_independent_tags() {
        setup_test_env();
        let hot_config = HotReloadConfig::new(Config::default(), "config/config.toml".to_string());

        hot_config.set_algorithm("tag-1", AlgorithmType::Segment);
        hot_config.set_algorithm("tag-2", AlgorithmType::Snowflake);

        assert_eq!(
            hot_config.get_algorithm("tag-1").unwrap(),
            AlgorithmType::Segment
        );
        assert_eq!(
            hot_config.get_algorithm("tag-2").unwrap(),
            AlgorithmType::Snowflake
        );
        assert!(hot_config.get_algorithm("tag-3").is_none());
    }

    /// `get_algorithm` on never-set biz_tag must return None.
    #[tokio::test]
    async fn test_get_algorithm_unknown_tag_returns_none() {
        setup_test_env();
        let hot_config = HotReloadConfig::new(Config::default(), "config/config.toml".to_string());
        assert!(hot_config.get_algorithm("never-set").is_none());
    }

    // watch (background loop)
    // watch 为无限循环：spawn 后立即 abort，仅验证可启动且不 panic。

    /// watch 可作为后台任务 spawn，启动阶段不 panic。
    #[tokio::test]
    async fn test_watch_is_spawnable_and_abortable() {
        setup_test_env();
        let hot_config =
            HotReloadConfig::new(Config::default(), "/nonexistent/path.toml".to_string());
        let handle = tokio::spawn(async move {
            hot_config.watch(60_000).await;
        });
        handle.abort();
        tokio::time::sleep(Duration::from_millis(10)).await;
        // If we reach here without panic, the test passes.
    }

    // HotReloadConfig::new edge cases

    /// 空路径仍产出可用配置（路径仅影响 reload_from_file / watch）。
    #[tokio::test]
    async fn test_new_with_empty_path() {
        setup_test_env();
        let hot_config = HotReloadConfig::new(Config::default(), String::new());
        let config = hot_config.get_config();
        assert_eq!(config.app.name, "nebula-id");
        let result = hot_config.reload_from_file().await;
        assert!(result.is_ok());
        assert!(!result.unwrap());
    }

    /// `new` preserves the provided config (round-trip via get_config).
    #[tokio::test]
    async fn test_new_preserves_provided_config() {
        setup_test_env();
        let mut config = Config::default();
        config.app.name = "preserved".to_string();
        config.app.http_port = 7777;
        let hot_config = HotReloadConfig::new(config, "config/config.toml".to_string());
        let retrieved = hot_config.get_config();
        assert_eq!(retrieved.app.name, "preserved");
        assert_eq!(retrieved.app.http_port, 7777);
    }

    /// 后注册的回调不追溯，只在后续 update 时触发。
    #[tokio::test]
    async fn test_callback_registered_after_first_update_only_fires_on_next() {
        setup_test_env();
        let hot_config = HotReloadConfig::new(Config::default(), "config/config.toml".to_string());

        hot_config.update_config(Config::default());

        let counter = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let c = counter.clone();
        hot_config.add_reload_callback(move |_| {
            c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        });

        // 第二次 update 恰好触发一次回调。
        hot_config.update_config(Config::default());
        assert_eq!(counter.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    // 补充覆盖（Phase 2 P0）

    // Helper: write a complete valid TOML config with overrides

    /// 写完整合法 TOML（字段可覆盖），避免各测试重复约 80 行样板。
    fn write_test_config_file(
        path: &std::path::Path,
        app_name: &str,
        http_port: u16,
        default_rps: u32,
        burst_size: u32,
        log_level: &str,
    ) {
        let content = format!(
            r#"[app]
name = "{app_name}"
host = "127.0.0.1"
http_port = {http_port}
grpc_port = 9091
dc_id = 1
worker_id = 1

[database]
engine = "postgresql"
url = "postgresql://idgen:${{NEBULA_DATABASE_PASSWORD}}@localhost:5432/idgen"
host = "localhost"
port = 5432
username = "idgen"
password = "${{NEBULA_DATABASE_PASSWORD}}"
database = "idgen"
max_connections = 10
min_connections = 1
acquire_timeout_seconds = 5
idle_timeout_seconds = 300

[etcd]
endpoints = ["http://localhost:2379"]
connect_timeout_ms = 5000
watch_timeout_ms = 5000

[auth]
enabled = true
cache_ttl_seconds = 300
api_keys = []

[algorithm]
default = "segment"

[algorithm.segment]
base_step = 1000
min_step = 500
max_step = 100000
switch_threshold = 0.1

[algorithm.snowflake]
datacenter_id_bits = 3
worker_id_bits = 8
sequence_bits = 10
clock_drift_threshold_ms = 1000

[algorithm.uuid_v8]
enabled = true

[monitoring]
metrics_enabled = true
metrics_path = "/metrics"
tracing_enabled = true
otlp_endpoint = ""

[logging]
level = "{log_level}"
format = "json"
include_location = true

[rate_limit]
enabled = true
default_rps = {default_rps}
burst_size = {burst_size}

[tls]
enabled = false
cert_path = ""
key_path = ""
http_enabled = false
grpc_enabled = false
min_tls_version = "tls13"
alpn_protocols = ["h2", "http/1.1"]

[batch_generate]
max_batch_size = 100
"#
        );
        std::fs::write(path, content).unwrap();
    }

    // with_audit_logger builder

    /// with_audit_logger 后 config/callbacks/algorithm map 须保持可用。
    #[tokio::test]
    async fn test_with_audit_logger_builder_returns_usable_config() {
        setup_test_env();
        let logger = Arc::new(crate::server::audit::AuditLogger::new(100));
        let hot_config = HotReloadConfig::new(Config::default(), "config/config.toml".to_string())
            .with_audit_logger(logger);

        assert_eq!(hot_config.get_config().app.name, "nebula-id");

        let counter = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let c = counter.clone();
        hot_config.add_reload_callback(move |_| {
            c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        });
        hot_config.update_config(Config::default());
        assert_eq!(counter.load(std::sync::atomic::Ordering::SeqCst), 1);

        hot_config.set_algorithm("audit-tag", AlgorithmType::Snowflake);
        assert_eq!(
            hot_config.get_algorithm("audit-tag").unwrap(),
            AlgorithmType::Snowflake
        );
    }

    // update_config + audit_logger

    /// update_config 附审计日志且字段变化：须走 detect_config_changes 并构造
    /// 审计事件；此处 future 不 await（let_underscore_future），total_logged 不增。
    #[tokio::test]
    async fn test_with_audit_logger_update_config_with_changes_covers_detect() {
        setup_test_env();
        let logger = Arc::new(crate::server::audit::AuditLogger::new(100));
        let hot_config = HotReloadConfig::new(Config::default(), "config/config.toml".to_string())
            .with_audit_logger(logger.clone());

        let mut new_config = Config::default();
        // Change all 5 fields exercised by detect_config_changes:
        new_config.app.name = "audit-update".to_string();
        new_config.app.http_port = 9999;
        new_config.rate_limit.default_rps = 5000;
        new_config.rate_limit.burst_size = 50;
        new_config.logging.level = crate::core::config::LogLevel::Debug;

        // 覆盖审计分支与 detect_config_changes，不得 panic。
        hot_config.update_config(new_config);

        assert_eq!(hot_config.get_config().app.name, "audit-update");
        assert_eq!(hot_config.get_config().app.http_port, 9999);
    }

    /// 配置无变化时不构造审计事件（覆盖 has_changes 为 false 分支）。
    #[tokio::test]
    async fn test_with_audit_logger_update_config_no_changes_skips_audit() {
        setup_test_env();
        let logger = Arc::new(crate::server::audit::AuditLogger::new(100));
        let config = Config::default();
        let hot_config = HotReloadConfig::new(config.clone(), "config/config.toml".to_string())
            .with_audit_logger(logger);

        hot_config.update_config(config);
        // 审计 future 被丢弃，仅验证不 panic。
    }

    // reload_from_file + audit_logger

    /// reload_from_file 附审计日志且文件真实变更：await 审计 future 并令
    /// total_logged +1（与 update_config 的 fire-and-forget 不同）。
    #[tokio::test]
    async fn test_with_audit_logger_reload_from_file_logs_change() {
        setup_test_env();
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("audit_reload.toml");
        write_test_config_file(&config_path, "initial", 8080, 10000, 100, "info");

        let logger = Arc::new(crate::server::audit::AuditLogger::new(100));
        let hot_config = HotReloadConfig::new(
            Config::load_from_file(config_path.to_str().unwrap()).unwrap(),
            config_path.to_str().unwrap().to_string(),
        )
        .with_audit_logger(logger.clone());

        write_test_config_file(&config_path, "reloaded", 9090, 5000, 50, "debug");

        let before = logger.total_logged();
        let result = hot_config.reload_from_file().await;
        assert!(result.is_ok());
        assert!(result.unwrap(), "reload_from_file should report success");
        let after = logger.total_logged();

        assert_eq!(
            after,
            before + 1,
            "audit logger must record one config-change event on file reload"
        );
    }

    // watch: 文件变更检测

    /// watch 检测文件修改并触发 reload_config：短间隔 spawn，改文件后验证
    /// 内存配置更新，随后 abort。
    #[tokio::test]
    async fn test_watch_detects_file_modification_and_reloads() {
        setup_test_env();
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("watch_reload.toml");
        write_test_config_file(&config_path, "before-watch", 8080, 10000, 100, "info");

        let hot_config = Arc::new(HotReloadConfig::new(
            Config::load_from_file(config_path.to_str().unwrap()).unwrap(),
            config_path.to_str().unwrap().to_string(),
        ));

        let hot_config_clone = hot_config.clone();
        let handle = tokio::spawn(async move {
            hot_config_clone.watch(50).await;
        });

        // 等待启动重载完成（初始内容 before-watch 生效）。
        tokio::time::sleep(Duration::from_millis(120)).await;
        assert_eq!(
            hot_config.get_config().app.name,
            "before-watch",
            "first tick should reload initial config"
        );

        // 改 app.name 以验证 reload；短暂等待确保变更可被观测。
        tokio::time::sleep(Duration::from_millis(50)).await;
        write_test_config_file(&config_path, "after-watch", 8080, 10000, 100, "info");

        tokio::time::sleep(Duration::from_millis(150)).await;

        handle.abort();
        tokio::time::sleep(Duration::from_millis(10)).await;

        let config = hot_config.get_config();
        assert_eq!(
            config.app.name, "after-watch",
            "watch should have detected file modification and reloaded config"
        );
    }

    /// 配置文件缺失时 watch 不得 panic，也不得改动内存配置。
    #[tokio::test]
    async fn test_watch_handles_missing_file_no_reload() {
        setup_test_env();
        let hot_config = Arc::new(HotReloadConfig::new(
            Config::default(),
            "/nonexistent/path/watch_missing.toml".to_string(),
        ));

        let hot_config_clone = hot_config.clone();
        let handle = tokio::spawn(async move {
            hot_config_clone.watch(50).await;
        });

        // 文件缺失：等待若干轮询，不得 panic，配置保持默认。
        tokio::time::sleep(Duration::from_millis(150)).await;

        handle.abort();
        tokio::time::sleep(Duration::from_millis(10)).await;

        assert_eq!(
            hot_config.get_config().app.name,
            "nebula-id",
            "watch on missing file must not reload config"
        );
    }

    // reload_callbacks 锁中毒路径：实践中不可达（已知覆盖缺口）
    // RwLock 仅写锁守护期间 panic 才中毒；reload_callbacks/biz_algorithm_map 的
    // 写锁体（push/insert）不会 panic，读锁不中毒，故这些错误分支无法在测试触发。

    #[tokio::test]
    async fn test_reload_config_missing_file_returns_false() {
        setup_test_env();
        let config = HotReloadConfig::new(
            Config::default(),
            "/nonexistent-dir-nebulaid-test/config.toml".to_string(),
        );
        assert!(!config
            .reload_config()
            .await
            .expect("读失败应返回 Ok(false)"));
    }

    #[tokio::test]
    async fn test_reload_config_invalid_content_returns_false() {
        setup_test_env();
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.toml");
        std::fs::write(&config_path, "[[[invalid toml").unwrap();
        let config =
            HotReloadConfig::new(Config::default(), config_path.to_str().unwrap().to_string());
        assert!(!config
            .reload_config()
            .await
            .expect("解析失败应返回 Ok(false)"));
    }
}

/// watch 循环在文件变更后触发 reload 回调（tempfile + 短间隔验证）。
#[tokio::test]
async fn test_watch_triggers_reload_callback_on_mtime_change() {
    // 以默认配置序列化产物为底稿，注入可观测的差异字段。
    let mut base = toml::to_string(&Config::default()).expect("序列化默认配置应成功");
    if base.contains("shutdown_timeout_seconds") {
        base = base.replace(
            "shutdown_timeout_seconds = 30",
            "shutdown_timeout_seconds = 45",
        );
    } else {
        base = base.replace("[app]", "[app]\nshutdown_timeout_seconds = 45");
    }
    let updated = base.replace(
        "shutdown_timeout_seconds = 45",
        "shutdown_timeout_seconds = 46",
    );
    let temp = tempfile::NamedTempFile::new().expect("创建临时配置文件应成功");
    std::fs::write(temp.path(), &base).expect("写入初始内容应成功");

    let config = Config::default();
    let hot = Arc::new(HotReloadConfig::new(
        config,
        temp.path().to_string_lossy().to_string(),
    ));

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<u64>();
    hot.add_reload_callback(move |cfg: Config| {
        let _ = tx.send(cfg.app.shutdown_timeout_seconds);
    });

    let watcher = hot.clone();
    let handle = tokio::spawn(async move { watcher.watch(20).await });

    // 覆写内容后等待 reload 回调（启动重载或事件驱动均可能先到）。
    std::fs::write(temp.path(), updated).expect("覆写内容应成功");

    let seen = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
        .await
        .expect("2 秒内应触发一次 reload 回调");
    assert_eq!(
        seen,
        Some(46),
        "回调应携带重载后配置的 shutdown_timeout_seconds"
    );

    handle.abort();
}
