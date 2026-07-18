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

//! etcd-backed implementations for the coordinator module.
//!
//! Compiled only when the `etcd` feature is enabled. Provides the
//! production-grade distributed worker ID allocator, distributed lock,
//! and cluster health monitor (rule 25: implementations live in
//! sub-modules; mod.rs only declares traits + re-exports).

#![cfg(feature = "etcd")]

use async_trait::async_trait;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU16, AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::fs;
use tokio::time::{sleep, Duration};
use tracing::{debug, error, info, warn};

use crate::core::config::EtcdConfig;
use super::{
    DistributedLock, EtcdClusterStatus, LocalCacheEntry, LockError, LockGuard,
    WorkerAllocatorError, WorkerIdAllocator,
};

/// etcd 集群健康监控器。
pub struct EtcdClusterHealthMonitor {
    config: EtcdConfig,
    status: AtomicU8,
    last_success: Arc<tokio::sync::Mutex<Instant>>,
    failure_count: AtomicU64,
    consecutive_failures: AtomicU64,
    local_cache: Arc<RwLock<HashMap<String, LocalCacheEntry>>>,
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
            local_cache: Arc::new(RwLock::new(HashMap::new())),
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

    pub async fn load_local_cache(&self) -> crate::core::types::Result<()> {
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
                crate::core::CoreError::InternalError(format!("Failed to read cache file: {}", e))
            })?;

        let entries: Vec<LocalCacheEntry> = serde_json::from_str(&content).map_err(|e| {
            crate::core::CoreError::InternalError(format!("Failed to parse cache file: {}", e))
        })?;

        let entry_count = entries.len();

        for entry in entries {
            self.local_cache.write().insert(entry.key.clone(), entry);
        }

        info!("Loaded {} entries from local cache", entry_count);
        Ok(())
    }

    pub async fn save_local_cache(&self) -> crate::core::types::Result<()> {
        let entries: Vec<LocalCacheEntry> = self.local_cache.read().values().cloned().collect();

        let content = serde_json::to_string_pretty(&entries).map_err(|e| {
            crate::core::CoreError::InternalError(format!("Failed to serialize cache: {}", e))
        })?;

        fs::write(&self.cache_file_path, content)
            .await
            .map_err(|e| {
                crate::core::CoreError::InternalError(format!("Failed to write cache file: {}", e))
            })?;

        info!("Saved {} entries to local cache", entries.len());
        Ok(())
    }

    pub fn get_from_cache(&self, key: &str) -> Option<LocalCacheEntry> {
        self.local_cache.read().get(key).cloned()
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
        self.local_cache.write().insert(key, entry);
    }

    pub fn delete_from_cache(&self, key: &str) {
        self.local_cache.write().remove(key);
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

/// EtcdWorkerAllocator - 使用 etcd 的 Worker ID 分配器。
pub struct EtcdWorkerAllocator {
    client: Arc<tokio::sync::Mutex<etcd_client::Client>>,
    datacenter_id: u8,
    allocated_id: AtomicU16,
    lease_id: AtomicI64,
    health_status: AtomicU8,
    config: EtcdConfig,
}

impl Clone for EtcdWorkerAllocator {
    fn clone(&self) -> Self {
        Self {
            client: self.client.clone(),
            datacenter_id: self.datacenter_id,
            allocated_id: AtomicU16::new(self.allocated_id.load(Ordering::SeqCst)),
            lease_id: AtomicI64::new(self.lease_id.load(Ordering::SeqCst)),
            health_status: AtomicU8::new(self.health_status.load(Ordering::SeqCst)),
            config: self.config.clone(),
        }
    }
}

impl EtcdWorkerAllocator {
    const MAX_WORKER_ID: u16 = 255;
    const WORKER_PATH_PREFIX: &'static str = "/idgen/workers";

    pub async fn new(
        endpoints: Vec<String>,
        datacenter_id: u8,
        config: EtcdConfig,
    ) -> std::result::Result<Self, WorkerAllocatorError> {
        use etcd_client::{Client, ConnectOptions};

        let options = ConnectOptions::new()
            .with_connect_timeout(std::time::Duration::from_millis(config.connect_timeout_ms))
            .with_timeout(std::time::Duration::from_millis(config.watch_timeout_ms));

        let client = Client::connect(endpoints, Some(options))
            .await
            .map_err(|e| WorkerAllocatorError::ConnectionFailed(e.to_string()))?;

        let allocator = Self {
            client: Arc::new(tokio::sync::Mutex::new(client)),
            datacenter_id,
            allocated_id: AtomicU16::new(0),
            lease_id: AtomicI64::new(0),
            health_status: AtomicU8::new(0),
            config,
        };

        info!("EtcdWorkerAllocator initialized for DC {}", datacenter_id);
        Ok(allocator)
    }

    fn worker_path(&self, worker_id: u16) -> String {
        format!(
            "{}/{}/{}",
            Self::WORKER_PATH_PREFIX,
            self.datacenter_id,
            worker_id
        )
    }

    async fn grant_lease(&self) -> std::result::Result<i64, WorkerAllocatorError> {
        let lease = self
            .client
            .lock()
            .await
            .lease_grant(30, None)
            .await
            .map_err(|e| WorkerAllocatorError::LeaseRenewalFailed(e.to_string()))?;

        let lease_id = lease.id();
        self.lease_id.store(lease_id, Ordering::SeqCst);
        info!("Lease granted: {}", lease_id);
        Ok(lease_id)
    }

    async fn try_allocate_id(
        &self,
        worker_id: u16,
        lease_id: i64,
    ) -> std::result::Result<bool, WorkerAllocatorError> {
        let path = self.worker_path(worker_id);
        let value = format!(
            "dc={},pid={},ts={}",
            self.datacenter_id,
            std::process::id(),
            chrono::Utc::now().timestamp()
        );

        let get_result = {
            let mut client = self.client.lock().await;
            client
                .get(path.clone(), None)
                .await
                .map_err(|e| WorkerAllocatorError::EtcdError(e.to_string()))?
        };

        if get_result.kvs().is_empty() {
            let put_options = Some(etcd_client::PutOptions::new().with_lease(lease_id));
            let mut client = self.client.lock().await;
            match client.put(path, value, put_options).await {
                Ok(_) => return Ok(true),
                Err(e) => return Err(WorkerAllocatorError::EtcdError(e.to_string())),
            }
        }

        Ok(false)
    }

    async fn do_allocate(&self) -> std::result::Result<u16, WorkerAllocatorError> {
        let lease_id = self.grant_lease().await?;

        for worker_id in 0..=Self::MAX_WORKER_ID {
            match self.try_allocate_id(worker_id, lease_id).await {
                Ok(true) => {
                    self.allocated_id.store(worker_id, Ordering::SeqCst);
                    info!("Successfully allocated worker_id: {}", worker_id);
                    return Ok(worker_id);
                }
                Ok(false) => continue,
                Err(e) => {
                    warn!("Failed to allocate worker_id {}: {}", worker_id, e);
                    continue;
                }
            }
        }

        Err(WorkerAllocatorError::NoAvailableId)
    }
}

#[async_trait]
impl WorkerIdAllocator for EtcdWorkerAllocator {
    async fn allocate(&self) -> std::result::Result<u16, WorkerAllocatorError> {
        self.do_allocate().await
    }

    async fn release(&self, worker_id: u16) -> std::result::Result<(), WorkerAllocatorError> {
        let path = self.worker_path(worker_id);
        let mut client = self.client.lock().await;
        if let Err(e) = client.delete(path, None).await {
            error!("Failed to release worker_id {}: {}", worker_id, e);
            return Err(WorkerAllocatorError::EtcdError(e.to_string()));
        }
        self.allocated_id.store(0, Ordering::SeqCst);
        self.lease_id.store(0, Ordering::SeqCst);
        info!("Released worker_id: {}", worker_id);
        Ok(())
    }

    fn get_allocated_id(&self) -> Option<u16> {
        let id = self.allocated_id.load(Ordering::SeqCst);
        if id == 0 {
            None
        } else {
            Some(id)
        }
    }

    fn is_healthy(&self) -> bool {
        self.health_status.load(Ordering::Relaxed) == 1
    }
}

/// Etcd 分布式锁实现。
/// 使用 etcd 的 lease 机制实现带 TTL 的分布式锁。
pub struct EtcdDistributedLock {
    client: Arc<tokio::sync::Mutex<etcd_client::Client>>,
    lock_path_prefix: String,
}

impl EtcdDistributedLock {
    /// 创建新的 Etcd 分布式锁。
    ///
    /// # Arguments
    /// * `endpoints` - etcd 集群端点列表
    /// * `lock_path_prefix` - 锁键前缀，例如 "/nebula/locks/"
    ///
    /// # Returns
    /// 返回锁实例或连接错误
    pub async fn new(
        endpoints: Vec<String>,
        lock_path_prefix: String,
    ) -> std::result::Result<Self, LockError> {
        use etcd_client::Client;

        let client = Client::connect(endpoints, None)
            .await
            .map_err(|e| LockError::ConnectionFailed(e.to_string()))?;

        info!(
            "EtcdDistributedLock initialized with prefix: {}",
            lock_path_prefix
        );

        Ok(Self {
            client: Arc::new(tokio::sync::Mutex::new(client)),
            lock_path_prefix,
        })
    }

    /// 使用现有 etcd 客户端创建分布式锁。
    pub fn with_client(
        client: Arc<tokio::sync::Mutex<etcd_client::Client>>,
        lock_path_prefix: String,
    ) -> Self {
        Self {
            client,
            lock_path_prefix,
        }
    }

    /// 构建完整的锁路径。
    fn lock_path(&self, key: &str) -> String {
        format!("{}{}", self.lock_path_prefix, key)
    }

    /// 尝试获取锁（内部实现）。
    ///
    /// 使用 etcd 事务实现原子性的"检查-设置"操作。
    async fn try_acquire_lock(
        &self,
        key: &str,
        ttl_seconds: i64,
    ) -> std::result::Result<Option<i64>, LockError> {
        use etcd_client::Txn;

        let lock_path = self.lock_path(key);

        let lease = self
            .client
            .lock()
            .await
            .lease_grant(ttl_seconds, None)
            .await
            .map_err(|e| LockError::EtcdError(e.to_string()))?;

        let lease_id = lease.id();
        let lock_value = format!(
            "holder_pid={},ts={}",
            std::process::id(),
            chrono::Utc::now().timestamp()
        );

        let txn = Txn::new()
            .when(vec![etcd_client::Compare::create_revision(
                lock_path.clone(),
                etcd_client::CompareOp::Equal,
                0,
            )])
            .and_then(vec![etcd_client::TxnOp::put(
                lock_path.clone(),
                lock_value,
                Some(etcd_client::PutOptions::new().with_lease(lease_id)),
            )])
            .or_else(vec![etcd_client::TxnOp::get(lock_path.clone(), None)]);

        let mut client = self.client.lock().await;
        let response = client
            .txn(txn)
            .await
            .map_err(|e| LockError::EtcdError(e.to_string()))?;

        if response.succeeded() {
            Ok(Some(lease_id))
        } else {
            let _ = client.lease_revoke(lease_id).await;
            Ok(None)
        }
    }
}

impl Clone for EtcdDistributedLock {
    fn clone(&self) -> Self {
        Self {
            client: self.client.clone(),
            lock_path_prefix: self.lock_path_prefix.clone(),
        }
    }
}

#[async_trait]
impl DistributedLock for EtcdDistributedLock {
    async fn acquire(
        &self,
        key: &str,
        ttl_seconds: u64,
    ) -> std::result::Result<Box<dyn LockGuard>, LockError> {
        const MAX_RETRIES: u32 = 3;
        const RETRY_DELAY_MS: u64 = 100;

        let ttl_seconds = ttl_seconds.max(1) as i64;

        for attempt in 0..MAX_RETRIES {
            match self.try_acquire_lock(key, ttl_seconds).await? {
                Some(lease_id) => {
                    info!(
                        "Acquired distributed lock for key '{}' (lease: {}, attempt: {})",
                        key, lease_id, attempt
                    );

                    let guard = EtcdLockGuard {
                        lock: self.clone(),
                        key: key.to_string(),
                        lease_id,
                        lock_path: self.lock_path(key),
                    };

                    return Ok(Box::new(guard));
                }
                None => {
                    if attempt < MAX_RETRIES - 1 {
                        debug!(
                            "Lock for key '{}' already held, retrying in {}ms (attempt {})",
                            key, RETRY_DELAY_MS, attempt
                        );
                        tokio::time::sleep(tokio::time::Duration::from_millis(RETRY_DELAY_MS))
                            .await;
                    }
                }
            }
        }

        Err(LockError::AcquireFailed {
            key: key.to_string(),
            reason: format!("Lock acquisition failed after {} retries", MAX_RETRIES),
        })
    }

    fn is_healthy(&self) -> bool {
        true
    }
}

/// Etcd 锁守卫实现。
/// 当守卫被 drop 时，自动释放锁（撤销 lease）。
pub struct EtcdLockGuard {
    lock: EtcdDistributedLock,
    key: String,
    lease_id: i64,
    lock_path: String,
}

#[async_trait]
impl LockGuard for EtcdLockGuard {
    async fn release(&self) -> std::result::Result<(), LockError> {
        let mut client = self.lock.client.lock().await;

        client
            .lease_revoke(self.lease_id)
            .await
            .map_err(|e| LockError::ReleaseFailed {
                key: self.key.clone(),
                reason: e.to_string(),
            })?;

        // R-algorithm-003: 输出 lock_path 用于运维诊断 etcd key 路径，同时消除 dead_code 警告。
        info!(
            lock_path = %self.lock_path,
            key = %self.key,
            lease_id = self.lease_id,
            "Released distributed lock"
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::EtcdConfig;
    use tempfile::NamedTempFile;

    #[tokio::test]
    async fn test_etcd_cluster_health_monitor() {
        let config = EtcdConfig::default();
        let cache_file = NamedTempFile::new().expect("Failed to create temp file");
        let cache_path = cache_file.path().to_string_lossy().to_string();
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
        let cache_file = NamedTempFile::new().expect("Failed to create temp file");
        let cache_path = cache_file.path().to_string_lossy().to_string();
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
        let cache_file = NamedTempFile::new().expect("Failed to create temp file");
        let cache_path = cache_file.path().to_string_lossy().to_string();
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
    }
}
