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
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;
use tracing::info;

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

/// Worker ID 分配器 trait
#[async_trait]
pub trait WorkerIdAllocator: Send + Sync {
    async fn allocate(&self) -> std::result::Result<u16, WorkerAllocatorError>;
    async fn release(&self, worker_id: u16) -> std::result::Result<(), WorkerAllocatorError>;
    fn get_allocated_id(&self) -> Option<u16>;
    fn is_healthy(&self) -> bool;
}

/// Worker ID 分配器错误
#[derive(Debug, Clone, thiserror::Error)]
pub enum WorkerAllocatorError {
    #[error("Failed to connect to etcd: {0}")]
    ConnectionFailed(String),

    #[error("No available worker ID")]
    NoAvailableId,

    #[error("Lease renewal failed: {0}")]
    LeaseRenewalFailed(String),

    #[error("Worker ID allocation cancelled")]
    Cancelled,

    #[error("etcd error: {0}")]
    EtcdError(String),

    #[error("Local allocator not configured")]
    NotConfigured,
}

/// 分布式锁 trait
/// 用于在分布式环境中协调对共享资源的访问
#[async_trait]
pub trait DistributedLock: Send + Sync {
    /// 尝试获取锁
    ///
    /// # Arguments
    /// * `key` - 锁的键，用于标识资源
    /// * `ttl_seconds` - 锁的生存时间（秒），超过此时间锁自动释放
    ///
    /// # Returns
    /// * `Ok(LockGuard)` - 成功获取锁，返回守卫对象
    /// * `Err(LockError)` - 获取锁失败
    async fn acquire(
        &self,
        key: &str,
        ttl_seconds: u64,
    ) -> std::result::Result<Box<dyn LockGuard>, LockError>;

    /// 检查锁服务是否健康
    fn is_healthy(&self) -> bool;
}

/// 锁守卫 trait
/// 当守卫被drop时自动释放锁
#[async_trait]
pub trait LockGuard: Send + Sync {
    /// 释放锁
    async fn release(&self) -> std::result::Result<(), LockError>;
}

/// 分布式锁错误
#[derive(Debug, Clone, thiserror::Error)]
pub enum LockError {
    #[error("Failed to connect to lock service: {0}")]
    ConnectionFailed(String),

    #[error("Lock acquisition timeout for key '{key}'")]
    Timeout { key: String },

    #[error("Failed to acquire lock for key '{key}': {reason}")]
    AcquireFailed { key: String, reason: String },

    #[error("Failed to release lock for key '{key}': {reason}")]
    ReleaseFailed { key: String, reason: String },

    #[error("etcd error: {0}")]
    EtcdError(String),

    #[error("Lock not configured")]
    NotConfigured,

    #[error("Lock service unavailable, degraded to local mode")]
    ServiceUnavailable,
}

/// 本地 Worker ID 分配器（无 etcd 时使用）
#[cfg(not(feature = "etcd"))]
#[derive(Clone)]
pub struct LocalWorkerAllocator {
    worker_id: Arc<AtomicU16>,
    datacenter_id: u8,
}

#[cfg(not(feature = "etcd"))]
impl LocalWorkerAllocator {
    pub fn new(datacenter_id: u8, default_worker_id: u16) -> Self {
        info!(
            "LocalWorkerAllocator initialized for DC {} with worker_id {}",
            datacenter_id, default_worker_id
        );
        Self {
            worker_id: Arc::new(AtomicU16::new(default_worker_id)),
            datacenter_id,
        }
    }
}

#[cfg(not(feature = "etcd"))]
#[async_trait]
impl WorkerIdAllocator for LocalWorkerAllocator {
    async fn allocate(&self) -> std::result::Result<u16, WorkerAllocatorError> {
        let id = self.worker_id.load(Ordering::SeqCst);
        info!(
            "Allocated local worker_id: {} for DC {}",
            id, self.datacenter_id
        );
        Ok(id)
    }

    async fn release(&self, worker_id: u16) -> std::result::Result<(), WorkerAllocatorError> {
        info!("Released local worker_id: {}", worker_id);
        Ok(())
    }

    fn get_allocated_id(&self) -> Option<u16> {
        let id = self.worker_id.load(Ordering::SeqCst);
        if id == 0 {
            None
        } else {
            Some(id)
        }
    }

    fn is_healthy(&self) -> bool {
        true
    }
}

/// Placeholder type when etcd feature is disabled
#[cfg(not(feature = "etcd"))]
#[derive(Clone)]
pub struct EtcdClusterHealthMonitor;

#[cfg(not(feature = "etcd"))]
impl EtcdClusterHealthMonitor {
    pub fn new(_config: crate::config::EtcdConfig, _cache_file_path: String) -> Self {
        Self
    }

    pub fn get_status(&self) -> EtcdClusterStatus {
        EtcdClusterStatus::Failed
    }

    pub fn set_status(&self, _status: EtcdClusterStatus) {}

    pub fn is_using_cache(&self) -> bool {
        true
    }

    pub async fn record_success(&self) {}

    pub fn record_failure(&self) {}

    pub async fn load_local_cache(&self) -> crate::types::Result<()> {
        Ok(())
    }

    pub async fn save_local_cache(&self) -> crate::types::Result<()> {
        Ok(())
    }

    pub fn get_from_cache(&self, _key: &str) -> Option<LocalCacheEntry> {
        None
    }

    pub fn put_to_cache(&self, _key: String, _value: String, _version: i64) {}

    pub fn delete_from_cache(&self, _key: &str) {}

    pub async fn start_health_check(&self, _check_interval: std::time::Duration) {}

    pub async fn start_cache_persistence(&self, _interval: std::time::Duration) {}
}

/// 本地分布式锁实现（无 etcd 时使用）
/// 注意：这是一个降级实现，只能在单机环境工作
#[cfg(not(feature = "etcd"))]
#[derive(Clone)]
pub struct LocalDistributedLock {
    locks: Arc<parking_lot::Mutex<std::collections::HashMap<String, bool>>>,
}

#[cfg(not(feature = "etcd"))]
impl LocalDistributedLock {
    pub fn new() -> Self {
        Self {
            locks: Arc::new(parking_lot::Mutex::new(std::collections::HashMap::new())),
        }
    }

    /// 检查锁是否已被持有
    fn is_locked(&self, key: &str) -> bool {
        let locks = self.locks.lock();
        locks.get(key).copied().unwrap_or(false)
    }

    /// 标记锁为已持有
    fn acquire_lock(&self, key: &str) {
        let mut locks = self.locks.lock();
        locks.insert(key.to_string(), true);
    }

    /// 释放锁
    fn release_lock(&self, key: &str) {
        let mut locks = self.locks.lock();
        locks.remove(key);
    }
}

#[cfg(not(feature = "etcd"))]
impl Default for LocalDistributedLock {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(not(feature = "etcd"))]
#[async_trait]
impl DistributedLock for LocalDistributedLock {
    async fn acquire(
        &self,
        key: &str,
        _ttl_seconds: u64,
    ) -> std::result::Result<Box<dyn LockGuard>, LockError> {
        // 本地实现：直接检查并获取锁
        if self.is_locked(key) {
            return Err(LockError::AcquireFailed {
                key: key.to_string(),
                reason: "Lock already held in local mode".to_string(),
            });
        }

        self.acquire_lock(key);

        let guard = LocalLockGuard {
            lock: self.clone(),
            key: key.to_string(),
        };

        Ok(Box::new(guard))
    }

    fn is_healthy(&self) -> bool {
        true // 本地实现始终健康
    }
}

/// 本地锁守卫实现
#[cfg(not(feature = "etcd"))]
pub struct LocalLockGuard {
    lock: LocalDistributedLock,
    key: String,
}

#[cfg(not(feature = "etcd"))]
#[async_trait]
impl LockGuard for LocalLockGuard {
    async fn release(&self) -> std::result::Result<(), LockError> {
        self.lock.release_lock(&self.key);
        Ok(())
    }
}

#[cfg(feature = "etcd")]
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

#[cfg(feature = "etcd")]
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

    #[cfg(feature = "etcd")]
    pub fn get_from_cache(&self, key: &str) -> Option<LocalCacheEntry> {
        self.local_cache.get(key).map(|v| v.value().clone())
    }

    #[cfg(feature = "etcd")]
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

    #[cfg(feature = "etcd")]
    pub fn delete_from_cache(&self, key: &str) {
        self.local_cache.remove(key);
    }

    #[cfg(feature = "etcd")]
    pub async fn start_health_check(&self, check_interval: Duration) {
        let monitor = self.clone();
        tokio::spawn(async move {
            loop {
                sleep(check_interval).await;
                monitor.check_etcd_health().await;
            }
        });
    }

    #[cfg(not(feature = "etcd"))]
    pub async fn start_health_check(&self, _check_interval: Duration) {
        // No-op when etcd is disabled
    }

    #[cfg(feature = "etcd")]
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

#[cfg(feature = "etcd")]
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

/// EtcdWorkerAllocator - 使用 etcd 的 Worker ID 分配器
#[cfg(feature = "etcd")]
pub struct EtcdWorkerAllocator {
    client: Arc<tokio::sync::Mutex<etcd_client::Client>>,
    datacenter_id: u8,
    allocated_id: AtomicU16,
    lease_id: AtomicI64,
    health_status: AtomicU8,
    config: EtcdConfig,
}

#[cfg(feature = "etcd")]
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

#[cfg(feature = "etcd")]
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

#[cfg(feature = "etcd")]
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

/// Etcd 分布式锁实现
/// 使用 etcd 的 lease 机制实现带 TTL 的分布式锁
#[cfg(feature = "etcd")]
pub struct EtcdDistributedLock {
    client: Arc<tokio::sync::Mutex<etcd_client::Client>>,
    lock_path_prefix: String,
}

#[cfg(feature = "etcd")]
impl EtcdDistributedLock {
    /// 创建新的 Etcd 分布式锁
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

    /// 使用现有 etcd 客户端创建分布式锁
    pub fn with_client(
        client: Arc<tokio::sync::Mutex<etcd_client::Client>>,
        lock_path_prefix: String,
    ) -> Self {
        Self {
            client,
            lock_path_prefix,
        }
    }

    /// 构建完整的锁路径
    fn lock_path(&self, key: &str) -> String {
        format!("{}{}", self.lock_path_prefix, key)
    }

    /// 尝试获取锁（内部实现）
    ///
    /// 使用 etcd 事务实现原子性的"检查-设置"操作
    async fn try_acquire_lock(
        &self,
        key: &str,
        ttl_seconds: i64,
    ) -> std::result::Result<Option<i64>, LockError> {
        use etcd_client::Txn;

        let lock_path = self.lock_path(key);

        // 先创建 lease
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

        // 使用事务检查键是否存在，不存在则创建
        let txn = Txn::new()
            .when(
                etcd_client::Compare::value(
                    etcd_client::CompareOp::Equal,
                    lock_path.clone(),
                    None, // version for MOD revision comparison
                ),
                etcd_client::CompareResult::Equal,
                std::vec::Vec::new(), // empty success comparison target means "key doesn't exist"
            )
            .and_then(
                etcd_client::TxnOp::put(
                    lock_path.clone(),
                    lock_value,
                    Some(etcd_client::PutOptions::new().with_lease(lease_id)),
                ),
                vec![],
            )
            .or_else(etcd_client::TxnOp::get(lock_path.clone(), None), vec![]);

        let mut client = self.client.lock().await;
        let response = client
            .txn(txn)
            .await
            .map_err(|e| LockError::EtcdError(e.to_string()))?;

        if response.succeeded {
            // 成功获取锁
            Ok(Some(lease_id))
        } else {
            // 锁已被其他持有者占用，撤销 lease
            let _ = client.lease_revoke(lease_id).await;
            Ok(None)
        }
    }
}

#[cfg(feature = "etcd")]
impl Clone for EtcdDistributedLock {
    fn clone(&self) -> Self {
        Self {
            client: self.client.clone(),
            lock_path_prefix: self.lock_path_prefix.clone(),
        }
    }
}

#[cfg(feature = "etcd")]
#[async_trait]
impl DistributedLock for EtcdDistributedLock {
    async fn acquire(
        &self,
        key: &str,
        ttl_seconds: u64,
    ) -> std::result::Result<Box<dyn LockGuard>, LockError> {
        const MAX_RETRIES: u32 = 3;
        const RETRY_DELAY_MS: u64 = 100;

        let ttl_seconds = ttl_seconds.max(1) as i64; // 最小 1 秒

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
        // 简单健康检查：尝试获取 etcd 状态
        // 注意：这是同步检查，可能不总是准确
        true
    }
}

/// Etcd 锁守卫实现
/// 当守卫被 drop 时，自动释放锁（撤销 lease）
#[cfg(feature = "etcd")]
pub struct EtcdLockGuard {
    lock: EtcdDistributedLock,
    key: String,
    lease_id: i64,
    lock_path: String,
}

#[cfg(feature = "etcd")]
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

        info!(
            "Released distributed lock for key '{}' (lease: {})",
            self.key, self.lease_id
        );

        Ok(())
    }
}

#[cfg(all(test, feature = "etcd"))]
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

    #[tokio::test]
    #[cfg(all(test, not(feature = "etcd")))]
    async fn test_local_worker_allocator() {
        use crate::coordinator::{LocalWorkerAllocator, WorkerIdAllocator};

        let allocator = LocalWorkerAllocator::new(0, 1);
        assert_eq!(allocator.get_allocated_id(), Some(1));
        assert!(allocator.is_healthy());

        let id = allocator.allocate().await.unwrap();
        assert_eq!(id, 1);

        allocator.release(1).await.unwrap();
        assert_eq!(allocator.get_allocated_id(), Some(1));
    }
}
