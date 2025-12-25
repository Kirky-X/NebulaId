use nebula_core::config::Config;
use nebula_core::types::Result;
use std::path::Path;
use std::sync::{Arc, RwLock};
use tokio::fs;
use tokio::time::{interval, Duration};
use tracing::{error, info, warn};

#[derive(Clone)]
pub struct HotReloadConfig {
    config: Arc<RwLock<Config>>,
    config_path: String,
    reload_callbacks: Arc<RwLock<Vec<Arc<dyn Fn(Config) + Send + Sync>>>>,
    audit_logger: Option<Arc<super::audit::AuditLogger>>,
}

impl HotReloadConfig {
    pub fn new(config: Config, config_path: String) -> Self {
        Self {
            config: Arc::new(RwLock::new(config)),
            config_path,
            reload_callbacks: Arc::new(RwLock::new(Vec::new())),
            audit_logger: None,
        }
    }

    pub fn with_audit_logger(mut self, audit_logger: Arc<super::audit::AuditLogger>) -> Self {
        self.audit_logger = Some(audit_logger);
        self
    }

    pub fn get_config(&self) -> Config {
        self.config.read().unwrap().clone()
    }

    pub fn add_reload_callback<F>(&self, callback: F)
    where
        F: Fn(Config) + Send + Sync + 'static,
    {
        self.reload_callbacks
            .write()
            .unwrap()
            .push(Arc::new(callback));
    }

    async fn reload_config(&self) -> Result<bool> {
        let config_path = self.config_path.clone();
        let content = match fs::read_to_string(&config_path).await {
            Ok(c) => c,
            Err(e) => {
                warn!("Failed to read config file: {}", e);
                return Ok(false);
            }
        };

        let new_config = match toml::from_str::<Config>(&content) {
            Ok(c) => c,
            Err(e) => {
                error!("Failed to parse config file: {}", e);
                return Ok(false);
            }
        };

        let old_config = {
            let mut config = self.config.write().unwrap();
            let old = config.clone();
            *config = new_config.clone();
            old
        };

        let callbacks: Vec<_> = {
            let callbacks = self.reload_callbacks.read().unwrap();
            callbacks.iter().cloned().collect()
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

        info!("Configuration hot-reloaded from {}", config_path);
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
                        error!("Error during config reload: {}", e);
                    }
                }
            }
        }
    }

    pub fn update_config(&self, new_config: Config) {
        let mut config = self.config.write().unwrap();
        let old_config = config.clone();
        *config = new_config.clone();
        drop(config);

        let callbacks = self.reload_callbacks.read().unwrap();
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
                let _ = logger.log_config_change(
                    None,
                    "api_update".to_string(),
                    "system".to_string(),
                    details,
                );
            }
        }

        info!("Configuration updated programmatically");
    }

    pub async fn reload_from_file(&self) -> Result<bool> {
        self.reload_config().await
    }
}

pub async fn watch_config_file<P: AsRef<Path>>(
    path: P,
    callback: impl Fn(Config) + Send + Sync + 'static,
) {
    let hot_config = HotReloadConfig::new(
        Config::load_from_file(path.as_ref().to_str().unwrap_or("config.toml")).unwrap_or_default(),
        path.as_ref().to_str().unwrap_or("config.toml").to_string(),
    );

    hot_config.add_reload_callback(callback);

    hot_config.watch(1000).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_hot_reload_config() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.toml");

        let initial_content = r#"[app]
name = "test"
host = "127.0.0.1"
http_port = 8080
grpc_port = 50051
dc_id = 1
worker_id = 1

[database]
engine = "postgresql"
url = "postgresql://idgen:idgen123@localhost:5432/idgen"
host = "localhost"
port = 5432
username = "idgen"
password = "idgen123"
database = "idgen"
max_connections = 10
min_connections = 1
acquire_timeout_seconds = 5
idle_timeout_seconds = 300

[redis]
url = "redis://localhost:6379"
pool_size = 10
key_prefix = "nebula:id:"
ttl_seconds = 600

[etcd]
endpoints = ["http://localhost:2379"]
connect_timeout_ms = 5000
watch_timeout_ms = 5000

[auth]
enabled = true
cache_ttl_seconds = 300
api_keys = [{ key = "test-api-key", workspace = "test", rate_limit = 10000 }]

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
url = "postgresql://idgen:idgen123@localhost:5432/idgen"
host = "localhost"
port = 5432
username = "idgen"
password = "idgen123"
database = "idgen"
max_connections = 10
min_connections = 1
acquire_timeout_seconds = 5
idle_timeout_seconds = 300

[redis]
url = "redis://localhost:6379"
pool_size = 10
key_prefix = "nebula:id:"
ttl_seconds = 600

[etcd]
endpoints = ["http://localhost:2379"]
connect_timeout_ms = 5000
watch_timeout_ms = 5000

[auth]
enabled = true
cache_ttl_seconds = 300
api_keys = [{ key = "test-api-key", workspace = "test", rate_limit = 10000 }]

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
"#;
        std::fs::write(&config_path, updated_content).unwrap();

        hot_config.reload_from_file().await.unwrap();

        assert!(*callback_triggered.lock().unwrap());
    }

    #[tokio::test]
    async fn test_get_config() {
        let hot_config = HotReloadConfig::new(Config::default(), "config.toml".to_string());

        let config = hot_config.get_config();
        assert_eq!(config.app.name, "nebula-id");
    }

    #[tokio::test]
    async fn test_update_config() {
        let hot_config = HotReloadConfig::new(Config::default(), "config.toml".to_string());

        let mut new_config = Config::default();
        new_config.app.name = "new-name".to_string();

        hot_config.update_config(new_config.clone());

        let retrieved = hot_config.get_config();
        assert_eq!(retrieved.app.name, "new-name");
    }
}
