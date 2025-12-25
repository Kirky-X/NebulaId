use crate::config::EtcdConfig;
use crate::types::Result;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::fs;
use tokio::time::sleep;
use tracing::{debug, error, info, warn};

#[derive(Debug, Clone, PartialEq)]
pub enum EtcdClusterStatus {
    Healthy,
    Degraded,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalCacheEntry {
    pub key: String,
    pub value: String,
    pub version: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

pub struct EtcdClusterHealthMonitor {
    config: EtcdConfig,
    status: AtomicU8,
    last_success: Arc<tokio::sync::Mutex<Instant>>,
    failure_count: AtomicU64,
    consecutive_failures: AtomicU64,
    local_cache: DashMap<String, LocalCacheEntry>,
    cache_file_path: String,
    is_using_cache: AtomicBool,
}

impl EtcdClusterHealthMonitor {
    pub fn new(config: EtcdConfig, cache_file_path: String) -> Self {
        Self {
            config,
            status: AtomicU8::new(EtcdClusterStatus::Healthy as u8),
            last_success: Arc::new(tokio::sync::Mutex::new(Instant::now())),
            failure_count: AtomicU64::new(0),
            consecutive_failures: AtomicU64::new(0),
            local_cache: DashMap::new(),
            cache_file_path,
            is_using_cache: AtomicBool::new(false),
        }
    }

    pub fn get_status(&self) -> EtcdClusterStatus {
        match self.status.load(Ordering::Relaxed) {
            0 => EtcdClusterStatus::Healthy,
            1 => EtcdClusterStatus::Degraded,
            _ => EtcdClusterStatus::Failed,
        }
    }

    pub fn set_status(&self, status: EtcdClusterStatus) {
        self.status.store(status as u8, Ordering::Relaxed);
    }

    pub fn is_using_cache(&self) -> bool {
        self.is_using_cache.load(Ordering::Relaxed)
    }

    pub async fn record_success(&self) {
        *self.last_success.lock().await = Instant::now();
        self.consecutive_failures.store(0, Ordering::Relaxed);
        if self.get_status() != EtcdClusterStatus::Healthy {
            self.set_status(EtcdClusterStatus::Healthy);
            info!("Etcd cluster recovered to healthy state");
            if self.is_using_cache() {
                self.is_using_cache.store(false, Ordering::Relaxed);
                info!("Switched back to etcd cluster from local cache");
            }
        }
    }

    pub fn record_failure(&self) {
        self.failure_count.fetch_add(1, Ordering::Relaxed);
        let consecutive = self.consecutive_failures.fetch_add(1, Ordering::Relaxed) + 1;

        if consecutive >= 5 {
            self.set_status(EtcdClusterStatus::Failed);
            self.is_using_cache.store(true, Ordering::Relaxed);
            warn!(
                "Etcd cluster marked as failed after {} consecutive failures, using local cache",
                consecutive
            );
        } else if consecutive >= 3 {
            self.set_status(EtcdClusterStatus::Degraded);
            warn!(
                "Etcd cluster marked as degraded after {} consecutive failures",
                consecutive
            );
        }
    }

    pub async fn load_local_cache(&self) -> Result<()> {
        let path = Path::new(&self.cache_file_path);
        if !path.exists() {
            info!(
                "Local cache file not found at {}, will create on first write",
                self.cache_file_path
            );
            return Ok(());
        }

        let content = fs::read_to_string(&self.cache_file_path)
            .await
            .map_err(|e| {
                crate::CoreError::InternalError(format!("Failed to read cache file: {}", e))
            })?;

        let entries: Vec<LocalCacheEntry> = serde_json::from_str(&content).map_err(|e| {
            crate::CoreError::InternalError(format!("Failed to parse cache file: {}", e))
        })?;

        let entry_count = entries.len();

        for entry in entries {
            self.local_cache.insert(entry.key.clone(), entry);
        }

        info!("Loaded {} entries from local cache", entry_count);
        Ok(())
    }

    pub async fn save_local_cache(&self) -> Result<()> {
        let entries: Vec<LocalCacheEntry> = self
            .local_cache
            .iter()
            .map(|entry| entry.value().clone())
            .collect();

        let content = serde_json::to_string_pretty(&entries).map_err(|e| {
            crate::CoreError::InternalError(format!("Failed to serialize cache: {}", e))
        })?;

        fs::write(&self.cache_file_path, content)
            .await
            .map_err(|e| {
                crate::CoreError::InternalError(format!("Failed to write cache file: {}", e))
            })?;

        info!("Saved {} entries to local cache", entries.len());
        Ok(())
    }

    pub fn get_from_cache(&self, key: &str) -> Option<LocalCacheEntry> {
        self.local_cache.get(key).map(|v| v.value().clone())
    }

    pub fn put_to_cache(&self, key: String, value: String, version: i64) {
        let now = chrono::Utc::now().timestamp();
        let entry = LocalCacheEntry {
            key: key.clone(),
            value,
            version,
            created_at: now,
            updated_at: now,
        };
        self.local_cache.insert(key, entry);
    }

    pub fn delete_from_cache(&self, key: &str) {
        self.local_cache.remove(key);
    }

    pub async fn start_health_check(&self, check_interval: Duration) {
        let monitor = self.clone();
        tokio::spawn(async move {
            loop {
                sleep(check_interval).await;
                monitor.check_etcd_health().await;
            }
        });
    }

    async fn check_etcd_health(&self) {
        use etcd_client::Client;

        if self.config.endpoints.is_empty() {
            warn!("No etcd endpoints configured");
            return;
        }

        let endpoints = self.config.endpoints.clone();
        let timeout = Duration::from_millis(self.config.connect_timeout_ms);

        let health_check = tokio::time::timeout(timeout, async {
            let client = Client::connect(endpoints, None).await;
            client
        })
        .await;

        match health_check {
            Ok(Ok(_client)) => {
                self.record_success().await;
                debug!("Etcd cluster health check passed");
            }
            Ok(Err(e)) => {
                warn!("Etcd cluster health check failed: {}", e);
                self.record_failure();
            }
            Err(_) => {
                warn!("Etcd cluster health check timeout");
                self.record_failure();
            }
        }
    }

    pub async fn start_cache_persistence(&self, interval: Duration) {
        let monitor = self.clone();
        tokio::spawn(async move {
            loop {
                sleep(interval).await;
                if let Err(e) = monitor.save_local_cache().await {
                    error!("Failed to persist local cache: {}", e);
                }
            }
        });
    }
}

impl Clone for EtcdClusterHealthMonitor {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            status: AtomicU8::new(self.status.load(Ordering::Relaxed)),
            last_success: self.last_success.clone(),
            failure_count: AtomicU64::new(self.failure_count.load(Ordering::Relaxed)),
            consecutive_failures: AtomicU64::new(self.consecutive_failures.load(Ordering::Relaxed)),
            local_cache: self.local_cache.clone(),
            cache_file_path: self.cache_file_path.clone(),
            is_using_cache: AtomicBool::new(self.is_using_cache.load(Ordering::Relaxed)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_etcd_cluster_health_monitor() {
        let config = EtcdConfig::default();
        let cache_path = "/tmp/test_etcd_cache.json".to_string();
        let monitor = EtcdClusterHealthMonitor::new(config, cache_path);

        assert_eq!(monitor.get_status(), EtcdClusterStatus::Healthy);
        assert!(!monitor.is_using_cache());

        monitor.record_failure();
        assert_eq!(monitor.get_status(), EtcdClusterStatus::Healthy);

        monitor.record_failure();
        monitor.record_failure();
        assert_eq!(monitor.get_status(), EtcdClusterStatus::Degraded);

        monitor.record_failure();
        monitor.record_failure();
        assert_eq!(monitor.get_status(), EtcdClusterStatus::Failed);
        assert!(monitor.is_using_cache());

        monitor.record_success().await;
        assert_eq!(monitor.get_status(), EtcdClusterStatus::Healthy);
        assert!(!monitor.is_using_cache());
    }

    #[tokio::test]
    async fn test_local_cache_operations() {
        let config = EtcdConfig::default();
        let cache_path = "/tmp/test_etcd_cache_ops.json".to_string();
        let monitor = EtcdClusterHealthMonitor::new(config, cache_path);

        monitor.put_to_cache("test_key".to_string(), "test_value".to_string(), 1);
        let entry = monitor.get_from_cache("test_key");
        assert!(entry.is_some());
        assert_eq!(entry.unwrap().value, "test_value");

        monitor.delete_from_cache("test_key");
        let entry = monitor.get_from_cache("test_key");
        assert!(entry.is_none());
    }

    #[tokio::test]
    async fn test_cache_persistence() {
        let config = EtcdConfig::default();
        let cache_path = "/tmp/test_etcd_cache_persist.json".to_string();
        let monitor = EtcdClusterHealthMonitor::new(config.clone(), cache_path.clone());

        monitor.put_to_cache("key1".to_string(), "value1".to_string(), 1);
        monitor.put_to_cache("key2".to_string(), "value2".to_string(), 2);

        monitor.save_local_cache().await.unwrap();

        let monitor2 = EtcdClusterHealthMonitor::new(config, cache_path);
        monitor2.load_local_cache().await.unwrap();

        let entry1 = monitor2.get_from_cache("key1");
        let entry2 = monitor2.get_from_cache("key2");

        assert!(entry1.is_some());
        assert!(entry2.is_some());
        assert_eq!(entry1.unwrap().value, "value1");
        assert_eq!(entry2.unwrap().value, "value2");

        let _ = fs::remove_file("/tmp/test_etcd_cache_persist.json").await;
    }
}
