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

use crate::core::config::Config;
use crate::core::types::id::AlgorithmType;
use crate::core::types::Result;
use arc_swap::ArcSwap;
use std::path::Path;
use std::sync::{Arc, RwLock};
use tokio::fs;
use tokio::time::{interval, Duration};
use tracing::{error, info, warn};

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
        // M10 修复：原实现先检查锁中毒再 `write().unwrap()`，逻辑矛盾
        // （锁中毒时第二次 write 仍会 Err，unwrap 会 panic）。
        // 改为复用 guard，与 `update_config` (line 228-240) 的正确模式一致。
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

        let new_config = match toml::from_str::<Config>(&content) {
            Ok(c) => c,
            Err(e) => {
                error!(
                    "{}",
                    t!(
                        "log.server.config.hot_reload.parse_config_failed",
                        error = e
                    )
                );
                return Ok(false);
            }
        };

        let old_config = self.config.load().as_ref().clone();
        self.config.store(Arc::new(new_config.clone()));

        // 使用map_err处理可能的锁中毒情况
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
        let mut interval = interval(Duration::from_millis(interval_ms));
        let mut last_modified = None;

        loop {
            interval.tick().await;

            if let Ok(metadata) = fs::metadata(&self.config_path).await {
                let current_modified = metadata.modified().ok();

                if last_modified.is_none() || current_modified != last_modified {
                    last_modified = current_modified;

                    if let Err(e) = self.reload_config().await {
                        error!(
                            "{}",
                            t!("log.server.config.hot_reload.reload_error", error = e)
                        );
                    }
                }
            }
        }
    }

    pub fn update_config(&self, new_config: Config) {
        let old_config = self.config.load().as_ref().clone();
        self.config.store(Arc::new(new_config.clone()));

        // 使用map_err处理可能的锁中毒情况
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
        // 使用map_err处理可能的锁中毒情况，避免panic
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
        // 使用map_err处理可能的锁中毒情况，避免panic
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

pub async fn watch_config_file<P: AsRef<Path>>(
    path: P,
    callback: impl Fn(Config) + Send + Sync + 'static,
) {
    let hot_config = HotReloadConfig::new(
        Config::load_from_file(path.as_ref().to_str().unwrap_or("config/config.toml"))
            .unwrap_or_default(),
        path.as_ref()
            .to_str()
            .unwrap_or("config/config.toml")
            .to_string(),
    );

    hot_config.add_reload_callback(callback);

    hot_config.watch(1000).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::TempDir;

    /// Setup test environment - must be called at the start of each test
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
grpc_port = 50051
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

[algorithm.uuid_v7]
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
grpc_port = 50051
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

[algorithm.uuid_v7]
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
}
