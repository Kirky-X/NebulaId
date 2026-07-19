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

use super::{
    DistributedLock, EtcdClusterStatus, LocalCacheEntry, LockError, LockGuard,
    WorkerAllocatorError, WorkerIdAllocator,
};
use crate::core::config::EtcdConfig;

/// etcd 客户端操作抽象 trait。
///
/// 解耦业务逻辑与 `etcd_client::Client`，便于单元测试 mock。所有方法接收 `&self`
/// （生产实现通过 `EtcdClientWrapper` 的内部 Mutex 提供 interior mutability），
/// 因此可用 `Arc<dyn EtcdClientOps>` 共享与注入。
#[async_trait]
pub trait EtcdClientOps: Send + Sync {
    /// 获取 key 对应的 value。返回 `None` 表示 key 不存在。
    async fn kv_get(&self, key: &str) -> std::result::Result<Option<Vec<u8>>, EtcdError>;

    /// 删除 key。
    async fn kv_delete(&self, key: &str) -> std::result::Result<(), EtcdError>;

    /// 授予 TTL（秒）的 lease，返回 lease_id。
    async fn lease_grant(&self, ttl: i64) -> std::result::Result<i64, EtcdError>;

    /// 撤销 lease。
    async fn lease_revoke(&self, lease_id: i64) -> std::result::Result<(), EtcdError>;

    /// 原子 CAS：当 key 的 `create_revision == 0`（不存在）时写入 value 并关联 lease_id。
    /// 返回 `true` 表示成功，`false` 表示 key 已存在。
    async fn txn_check_create_rev_and_put(
        &self,
        key: &str,
        value: Vec<u8>,
        lease_id: i64,
    ) -> std::result::Result<bool, EtcdError>;

    /// 健康检查 ping：执行一个轻量级 etcd 操作验证连通性。
    async fn ping(&self) -> std::result::Result<(), EtcdError>;
}

/// etcd 操作错误类型。
#[derive(Debug, Clone, thiserror::Error)]
pub enum EtcdError {
    #[error("etcd network error: {0}")]
    Network(String),

    #[error("etcd key not found: {0}")]
    KeyNotFound(String),

    #[error("etcd lease invalid: {0}")]
    LeaseInvalid(String),

    #[error("etcd internal error: {0}")]
    Internal(String),
}

/// 生产环境 etcd 客户端封装。
///
/// `etcd_client::Client` 的所有方法接收 `&mut self`，无法直接满足 `EtcdClientOps`
/// 的 `&self` 契约。`EtcdClientWrapper` 用 `tokio::sync::Mutex` 提供 interior
/// mutability，使生产客户端可通过 `Arc<dyn EtcdClientOps>` 注入业务结构。
pub struct EtcdClientWrapper {
    inner: tokio::sync::Mutex<etcd_client::Client>,
}

impl EtcdClientWrapper {
    /// 连接 etcd 集群并创建客户端封装。
    pub async fn new(endpoints: Vec<String>) -> std::result::Result<Self, EtcdError> {
        let client = etcd_client::Client::connect(endpoints, None)
            .await
            .map_err(|e| EtcdError::Network(e.to_string()))?;
        Ok(Self {
            inner: tokio::sync::Mutex::new(client),
        })
    }
}

#[async_trait]
impl EtcdClientOps for EtcdClientWrapper {
    async fn kv_get(&self, key: &str) -> std::result::Result<Option<Vec<u8>>, EtcdError> {
        let mut client = self.inner.lock().await;
        let resp = client
            .get(key, None)
            .await
            .map_err(|e| EtcdError::Network(e.to_string()))?;
        if resp.kvs().is_empty() {
            Ok(None)
        } else {
            Ok(Some(resp.kvs()[0].value().to_vec()))
        }
    }

    async fn kv_delete(&self, key: &str) -> std::result::Result<(), EtcdError> {
        let mut client = self.inner.lock().await;
        client
            .delete(key, None)
            .await
            .map_err(|e| EtcdError::Network(e.to_string()))?;
        Ok(())
    }

    async fn lease_grant(&self, ttl: i64) -> std::result::Result<i64, EtcdError> {
        let mut client = self.inner.lock().await;
        let resp = client
            .lease_grant(ttl, None)
            .await
            .map_err(|e| EtcdError::LeaseInvalid(e.to_string()))?;
        Ok(resp.id())
    }

    async fn lease_revoke(&self, lease_id: i64) -> std::result::Result<(), EtcdError> {
        let mut client = self.inner.lock().await;
        client
            .lease_revoke(lease_id)
            .await
            .map_err(|e| EtcdError::LeaseInvalid(e.to_string()))?;
        Ok(())
    }

    async fn txn_check_create_rev_and_put(
        &self,
        key: &str,
        value: Vec<u8>,
        lease_id: i64,
    ) -> std::result::Result<bool, EtcdError> {
        use etcd_client::{Compare, CompareOp, PutOptions, Txn, TxnOp};

        let txn = Txn::new()
            .when(vec![Compare::create_revision(key, CompareOp::Equal, 0)])
            .and_then(vec![TxnOp::put(
                key,
                value,
                Some(PutOptions::new().with_lease(lease_id)),
            )])
            .or_else(vec![TxnOp::get(key, None)]);

        let mut client = self.inner.lock().await;
        let resp = client
            .txn(txn)
            .await
            .map_err(|e| EtcdError::Network(e.to_string()))?;
        Ok(resp.succeeded())
    }

    async fn ping(&self) -> std::result::Result<(), EtcdError> {
        let mut client = self.inner.lock().await;
        client
            .get("", None)
            .await
            .map_err(|e| EtcdError::Network(e.to_string()))?;
        Ok(())
    }
}

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
    /// 可选注入的 etcd 客户端：`Some` 时 `check_etcd_health` 走可 mock 路径，
    /// `None` 时每次健康检查新建 `etcd_client::Client`（生产默认）。
    client: Option<Arc<dyn EtcdClientOps>>,
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
            client: None,
        }
    }

    /// 用注入的 etcd 客户端构造监控器，使 `check_etcd_health` 走可测试路径。
    pub fn new_with_client(
        config: EtcdConfig,
        cache_file_path: String,
        client: Arc<dyn EtcdClientOps>,
    ) -> Self {
        Self {
            config,
            status: AtomicU8::new(EtcdClusterStatus::Healthy as u8),
            last_success: Arc::new(tokio::sync::Mutex::new(Instant::now())),
            failure_count: AtomicU64::new(0),
            consecutive_failures: AtomicU64::new(0),
            local_cache: Arc::new(RwLock::new(HashMap::new())),
            cache_file_path,
            is_using_cache: AtomicBool::new(false),
            client: Some(client),
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
            info!("{}", t!("log.core.coordinator.etcd.cluster_recovered"));
            if self.is_using_cache() {
                self.is_using_cache.store(false, Ordering::Relaxed);
                info!("{}", t!("log.core.coordinator.etcd.switched_back_to_etcd"));
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
                "{}",
                t!(
                    "log.core.coordinator.etcd.cluster_failed",
                    consecutive = consecutive
                )
            );
        } else if consecutive >= 3 {
            self.set_status(EtcdClusterStatus::Degraded);
            warn!(
                "{}",
                t!(
                    "log.core.coordinator.etcd.cluster_degraded",
                    consecutive = consecutive
                )
            );
        }
    }

    pub async fn load_local_cache(&self) -> crate::core::types::Result<()> {
        let path = Path::new(&self.cache_file_path);
        if !path.exists() {
            info!(
                "{}",
                t!(
                    "log.core.coordinator.etcd.cache_file_not_found",
                    path = self.cache_file_path
                )
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

        info!(
            "{}",
            t!(
                "log.core.coordinator.etcd.loaded_cache_entries",
                count = entry_count
            )
        );
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

        info!(
            "{}",
            t!(
                "log.core.coordinator.etcd.saved_cache_entries",
                count = entries.len()
            )
        );
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
        // 注入路径：用 `EtcdClientOps::ping` 验证连通性，可被 mock。
        // 与生产路径对齐：用 `connect_timeout_ms` 包裹 ping，避免健康检查挂死。
        if let Some(client) = &self.client {
            let timeout = Duration::from_millis(self.config.connect_timeout_ms);
            let ping_result = tokio::time::timeout(timeout, client.ping()).await;
            match ping_result {
                Ok(Ok(())) => {
                    self.record_success().await;
                    debug!(
                        "{}",
                        t!("log.core.coordinator.etcd.health_check_passed_injected")
                    );
                }
                Ok(Err(e)) => {
                    warn!(
                        "{}",
                        t!("log.core.coordinator.etcd.health_check_failed", error = e)
                    );
                    self.record_failure();
                }
                Err(_) => {
                    warn!(
                        "{}",
                        t!(
                            "log.core.coordinator.etcd.health_check_timeout_injected",
                            timeout_ms = self.config.connect_timeout_ms
                        )
                    );
                    self.record_failure();
                }
            }
            return;
        }

        // 生产默认路径：每次检查新建一个临时 client。
        use etcd_client::Client;

        if self.config.endpoints.is_empty() {
            warn!(
                "{}",
                t!("log.core.coordinator.etcd.no_endpoints_configured")
            );
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
                debug!("{}", t!("log.core.coordinator.etcd.health_check_passed"));
            }
            Ok(Err(e)) => {
                warn!(
                    "{}",
                    t!("log.core.coordinator.etcd.health_check_failed", error = e)
                );
                self.record_failure();
            }
            Err(_) => {
                warn!(
                    "{}",
                    t!("log.core.coordinator.etcd.health_check_timeout_default")
                );
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
                    error!(
                        "{}",
                        t!("log.core.coordinator.etcd.persist_cache_failed", error = e)
                    );
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
            client: self.client.clone(),
        }
    }
}

/// EtcdWorkerAllocator - 使用 etcd 的 Worker ID 分配器。
pub struct EtcdWorkerAllocator {
    client: Arc<dyn EtcdClientOps>,
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

    /// 用注入的 etcd 客户端构造分配器。调用方负责创建 `EtcdClientOps` 实例
    /// （生产环境用 `EtcdClientWrapper`，测试用 `MockEtcdClientOps`）。
    pub async fn new(
        client: Arc<dyn EtcdClientOps>,
        datacenter_id: u8,
        config: EtcdConfig,
    ) -> std::result::Result<Self, WorkerAllocatorError> {
        let allocator = Self {
            client,
            datacenter_id,
            allocated_id: AtomicU16::new(0),
            lease_id: AtomicI64::new(0),
            health_status: AtomicU8::new(0),
            config,
        };

        info!(
            "{}",
            t!(
                "log.core.coordinator.etcd.allocator_initialized",
                datacenter_id = datacenter_id
            )
        );
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
        let lease_id = self
            .client
            .lease_grant(30)
            .await
            .map_err(|e| WorkerAllocatorError::LeaseRenewalFailed(e.to_string()))?;

        self.lease_id.store(lease_id, Ordering::SeqCst);
        info!(
            "{}",
            t!(
                "log.core.coordinator.etcd.lease_granted",
                lease_id = lease_id
            )
        );
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

        // 预检查：key 已存在则跳过。kv_get 错误容错（继续尝试下一个 id）。
        match self.client.kv_get(&path).await {
            Ok(Some(_)) => return Ok(false),
            Ok(None) => {}
            Err(e) => {
                warn!(
                    "{}",
                    t!(
                        "log.core.coordinator.etcd.kv_get_failed",
                        path = path,
                        error = e
                    )
                );
                return Ok(false);
            }
        }

        // 原子 CAS：仅当 key 不存在（create_revision == 0）时写入。
        let succeeded = self
            .client
            .txn_check_create_rev_and_put(&path, value.into_bytes(), lease_id)
            .await
            .map_err(|e| WorkerAllocatorError::EtcdError(e.to_string()))?;

        Ok(succeeded)
    }

    async fn do_allocate(&self) -> std::result::Result<u16, WorkerAllocatorError> {
        let lease_id = self.grant_lease().await?;

        for worker_id in 0..=Self::MAX_WORKER_ID {
            match self.try_allocate_id(worker_id, lease_id).await {
                Ok(true) => {
                    self.allocated_id.store(worker_id, Ordering::SeqCst);
                    info!(
                        "{}",
                        t!(
                            "log.core.coordinator.etcd.worker_id_allocated",
                            worker_id = worker_id
                        )
                    );
                    return Ok(worker_id);
                }
                Ok(false) => continue,
                Err(e) => {
                    warn!(
                        "{}",
                        t!(
                            "log.core.coordinator.etcd.worker_id_allocate_failed",
                            worker_id = worker_id,
                            error = e
                        )
                    );
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
        if let Err(e) = self.client.kv_delete(&path).await {
            error!(
                "{}",
                t!(
                    "log.core.coordinator.etcd.worker_id_release_failed",
                    worker_id = worker_id,
                    error = e
                )
            );
            return Err(WorkerAllocatorError::EtcdError(e.to_string()));
        }
        self.allocated_id.store(0, Ordering::SeqCst);
        self.lease_id.store(0, Ordering::SeqCst);
        info!(
            "{}",
            t!(
                "log.core.coordinator.etcd.worker_id_released",
                worker_id = worker_id
            )
        );
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
    client: Arc<dyn EtcdClientOps>,
    lock_path_prefix: String,
}

impl EtcdDistributedLock {
    /// 用注入的 etcd 客户端创建分布式锁。调用方负责创建 `EtcdClientOps` 实例。
    ///
    /// # Arguments
    /// * `client` - etcd 客户端抽象
    /// * `lock_path_prefix` - 锁键前缀，例如 "/nebula/locks/"
    pub async fn new(
        client: Arc<dyn EtcdClientOps>,
        lock_path_prefix: String,
    ) -> std::result::Result<Self, LockError> {
        info!(
            "{}",
            t!(
                "log.core.coordinator.etcd.lock_initialized",
                prefix = lock_path_prefix
            )
        );

        Ok(Self {
            client,
            lock_path_prefix,
        })
    }

    /// 使用现有 `EtcdClientOps` 客户端创建分布式锁。
    pub fn with_client(client: Arc<dyn EtcdClientOps>, lock_path_prefix: String) -> Self {
        Self {
            client,
            lock_path_prefix,
        }
    }

    /// 构建完整的锁路径。
    fn lock_path(&self, key: &str) -> String {
        format!("{}{}", self.lock_path_prefix, key)
    }

    /// 尝试获取锁（内部实现）：授予 lease + 原子 CAS 写入。
    async fn try_acquire_lock(
        &self,
        key: &str,
        ttl_seconds: i64,
    ) -> std::result::Result<Option<i64>, LockError> {
        let lock_path = self.lock_path(key);

        let lease_id = self
            .client
            .lease_grant(ttl_seconds)
            .await
            .map_err(|e| LockError::EtcdError(e.to_string()))?;

        let lock_value = format!(
            "holder_pid={},ts={}",
            std::process::id(),
            chrono::Utc::now().timestamp()
        );

        let succeeded = self
            .client
            .txn_check_create_rev_and_put(&lock_path, lock_value.into_bytes(), lease_id)
            .await
            .map_err(|e| LockError::EtcdError(e.to_string()))?;

        if succeeded {
            Ok(Some(lease_id))
        } else {
            // CAS 失败（key 已存在），撤销刚才授予的 lease 避免泄漏。
            let _ = self.client.lease_revoke(lease_id).await;
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
                        "{}",
                        t!(
                            "log.core.coordinator.etcd.lock_acquired",
                            key = key,
                            lease_id = lease_id,
                            attempt = attempt
                        )
                    );

                    let guard = EtcdLockGuard {
                        lock: self.clone(),
                        key: key.to_string(),
                        lease_id,
                        lock_path: self.lock_path(key),
                        // L9 修复：初始化为未释放状态，Drop 时检查
                        released: Arc::new(AtomicBool::new(false)),
                    };

                    return Ok(Box::new(guard));
                }
                None => {
                    if attempt < MAX_RETRIES - 1 {
                        debug!(
                            "{}",
                            t!(
                                "log.core.coordinator.etcd.lock_already_held_retry",
                                key = key,
                                retry_delay_ms = RETRY_DELAY_MS,
                                attempt = attempt
                            )
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
///
/// L9 修复：实现 `Drop` trait，在 guard 被 drop 时自动 spawn 一个
/// 后台 task 撤销 lease 释放锁。若调用方已显式调用 `release()`，
/// `released` 标志位会阻止 Drop 中的二次释放。若 tokio runtime
/// 不可用（例如进程关闭阶段），Drop 仅记录 warning 日志，锁会
/// 在 lease TTL 到期后由 etcd 自动回收。
pub struct EtcdLockGuard {
    lock: EtcdDistributedLock,
    key: String,
    lease_id: i64,
    lock_path: String,
    /// L9 修复：标记是否已显式 release，防止 Drop 中的 double-release。
    /// AtomicBool 无需 &mut self，可在 Drop 中读取。
    released: Arc<AtomicBool>,
}

#[async_trait]
impl LockGuard for EtcdLockGuard {
    async fn release(&self) -> std::result::Result<(), LockError> {
        self.lock
            .client
            .lease_revoke(self.lease_id)
            .await
            .map_err(|e| LockError::ReleaseFailed {
                key: self.key.clone(),
                reason: e.to_string(),
            })?;

        // L9 修复：标记已释放，防止 Drop 中二次释放。
        self.released.store(true, Ordering::SeqCst);

        // R-algorithm-003: 输出 lock_path 用于运维诊断 etcd key 路径，同时消除 dead_code 警告。
        info!(
            lock_path = %self.lock_path,
            key = %self.key,
            lease_id = self.lease_id,
            "{}",
            t!("log.core.coordinator.etcd.lock_released")
        );

        Ok(())
    }
}

impl Drop for EtcdLockGuard {
    fn drop(&mut self) {
        // L9 修复：已显式 release 则跳过，避免 double-release。
        if self.released.load(Ordering::SeqCst) {
            return;
        }

        // L9 修复：未显式 release 时，尝试 spawn 后台 task 异步释放锁。
        // 这覆盖了调用方忘记显式 release 的场景（如早期 return / panic）。
        let lock = self.lock.clone();
        let key = self.key.clone();
        let lease_id = self.lease_id;
        let lock_path = self.lock_path.clone();

        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                handle.spawn(async move {
                    if let Err(e) = lock.client.lease_revoke(lease_id).await {
                        warn!(
                            lock_path = %lock_path,
                            key = %key,
                            lease_id = lease_id,
                            error = %e,
                            "{}",
                            t!("log.core.coordinator.etcd.lock_drop_release_failed")
                        );
                    } else {
                        debug!(
                            lock_path = %lock_path,
                            key = %key,
                            lease_id = lease_id,
                            "{}",
                            t!("log.core.coordinator.etcd.lock_drop_released")
                        );
                    }
                });
            }
            Err(_) => {
                // runtime 不可用（如进程关闭），lease TTL 会自动回收。
                warn!(
                    lock_path = %lock_path,
                    key = %key,
                    lease_id = lease_id,
                    "{}",
                    t!("log.core.coordinator.etcd.lock_drop_no_runtime")
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::EtcdConfig;
    use tempfile::NamedTempFile;

    mockall::mock! {
        pub EtcdClientOps {}
        #[async_trait::async_trait]
        impl crate::core::coordinator::EtcdClientOps for EtcdClientOps {
            async fn kv_get(&self, key: &str) -> std::result::Result<Option<Vec<u8>>, crate::core::coordinator::EtcdError>;
            async fn kv_delete(&self, key: &str) -> std::result::Result<(), crate::core::coordinator::EtcdError>;
            async fn lease_grant(&self, ttl: i64) -> std::result::Result<i64, crate::core::coordinator::EtcdError>;
            async fn lease_revoke(&self, lease_id: i64) -> std::result::Result<(), crate::core::coordinator::EtcdError>;
            async fn txn_check_create_rev_and_put(&self, key: &str, value: Vec<u8>, lease_id: i64) -> std::result::Result<bool, crate::core::coordinator::EtcdError>;
            async fn ping(&self) -> std::result::Result<(), crate::core::coordinator::EtcdError>;
        }
    }

    /// 把 mock 封装为 `Arc<dyn EtcdClientOps>`，方便各测试复用。
    fn mock_into_client(mock: MockEtcdClientOps) -> Arc<dyn EtcdClientOps> {
        Arc::new(mock)
    }

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

    // ===== EtcdWorkerAllocator tests (10) =====

    #[tokio::test]
    async fn test_allocator_new_succeeds() {
        let client = mock_into_client(MockEtcdClientOps::new());
        let allocator = EtcdWorkerAllocator::new(client, 1, EtcdConfig::default()).await;

        assert!(allocator.is_ok());
        let allocator = allocator.unwrap();
        // allocated_id 默认 0 → None；lease_id 默认 0；health_status 默认 0 → false
        assert_eq!(allocator.get_allocated_id(), None);
        assert!(!allocator.is_healthy());
    }

    #[tokio::test]
    async fn test_allocator_allocate_succeeds() {
        let mut mock = MockEtcdClientOps::new();
        // worker_id=0 的 key 已存在 → 跳过；其他 id 的 key 不存在 → 进入 CAS
        mock.expect_kv_get().returning(|key| {
            if key.ends_with("/0") {
                Ok(Some(b"taken".to_vec()))
            } else {
                Ok(None)
            }
        });
        mock.expect_lease_grant().returning(|_| Ok(123));
        mock.expect_txn_check_create_rev_and_put()
            .returning(|_, _, _| Ok(true));

        let allocator = EtcdWorkerAllocator::new(mock_into_client(mock), 1, EtcdConfig::default())
            .await
            .unwrap();

        let result = allocator.allocate().await;
        assert!(result.is_ok());
        let worker_id = result.unwrap();
        assert!(
            worker_id >= 1,
            "worker_id 应 >= 1 (id 0 被占用应跳过), 实际: {}",
            worker_id
        );
        assert_eq!(allocator.get_allocated_id(), Some(worker_id));
    }

    #[tokio::test]
    async fn test_allocator_allocate_no_available_id() {
        let mut mock = MockEtcdClientOps::new();
        mock.expect_kv_get().returning(|_| Ok(None)); // 所有 key 都不存在
        mock.expect_lease_grant().returning(|_| Ok(123));
        mock.expect_txn_check_create_rev_and_put()
            .returning(|_, _, _| Ok(false)); // CAS 全部失败（key 已被抢占）

        let allocator = EtcdWorkerAllocator::new(mock_into_client(mock), 1, EtcdConfig::default())
            .await
            .unwrap();

        let result = allocator.allocate().await;
        assert!(
            matches!(result, Err(WorkerAllocatorError::NoAvailableId)),
            "应返回 NoAvailableId, 实际: {:?}",
            result
        );
    }

    #[tokio::test]
    async fn test_allocator_allocate_lease_grant_fails() {
        let mut mock = MockEtcdClientOps::new();
        mock.expect_lease_grant()
            .returning(|_| Err(EtcdError::LeaseInvalid("grant failed".into())));

        let allocator = EtcdWorkerAllocator::new(mock_into_client(mock), 1, EtcdConfig::default())
            .await
            .unwrap();

        let result = allocator.allocate().await;
        assert!(
            matches!(result, Err(WorkerAllocatorError::LeaseRenewalFailed(_))),
            "应返回 LeaseRenewalFailed, 实际: {:?}",
            result
        );
    }

    #[tokio::test]
    async fn test_allocator_allocate_kv_get_network_error_continues() {
        let mut mock = MockEtcdClientOps::new();
        let mut seq = mockall::Sequence::new();
        // 第一次 kv_get (worker_id=0): 网络错误 → 容错跳过
        mock.expect_kv_get()
            .times(1)
            .in_sequence(&mut seq)
            .returning(|_| Err(EtcdError::Network("net".into())));
        // 后续 kv_get: Ok(None)
        mock.expect_kv_get().returning(|_| Ok(None));
        mock.expect_lease_grant().returning(|_| Ok(123));
        mock.expect_txn_check_create_rev_and_put()
            .returning(|_, _, _| Ok(true));

        let allocator = EtcdWorkerAllocator::new(mock_into_client(mock), 1, EtcdConfig::default())
            .await
            .unwrap();

        let result = allocator.allocate().await;
        assert!(result.is_ok());
        let worker_id = result.unwrap();
        assert!(
            worker_id >= 1,
            "kv_get 错误后应跳过 worker_id 0, 实际分配: {}",
            worker_id
        );
    }

    #[tokio::test]
    async fn test_allocator_release_succeeds() {
        let mut mock = MockEtcdClientOps::new();
        mock.expect_kv_get().returning(|key| {
            if key.ends_with("/0") {
                Ok(Some(b"taken".to_vec()))
            } else {
                Ok(None)
            }
        });
        mock.expect_lease_grant().returning(|_| Ok(123));
        mock.expect_txn_check_create_rev_and_put()
            .returning(|_, _, _| Ok(true));
        mock.expect_kv_delete().returning(|_| Ok(()));

        let allocator = EtcdWorkerAllocator::new(mock_into_client(mock), 1, EtcdConfig::default())
            .await
            .unwrap();

        let worker_id = allocator.allocate().await.unwrap();
        assert!(worker_id >= 1);
        assert_eq!(allocator.get_allocated_id(), Some(worker_id));

        let release_result = allocator.release(worker_id).await;
        assert!(release_result.is_ok());
        assert_eq!(
            allocator.get_allocated_id(),
            None,
            "release 后 allocated_id 应重置为 0 → None"
        );
    }

    #[tokio::test]
    async fn test_allocator_release_delete_fails() {
        let mut mock = MockEtcdClientOps::new();
        mock.expect_kv_get().returning(|key| {
            if key.ends_with("/0") {
                Ok(Some(b"taken".to_vec()))
            } else {
                Ok(None)
            }
        });
        mock.expect_lease_grant().returning(|_| Ok(123));
        mock.expect_txn_check_create_rev_and_put()
            .returning(|_, _, _| Ok(true));
        mock.expect_kv_delete()
            .returning(|_| Err(EtcdError::Network("delete failed".into())));

        let allocator = EtcdWorkerAllocator::new(mock_into_client(mock), 1, EtcdConfig::default())
            .await
            .unwrap();

        let worker_id = allocator.allocate().await.unwrap();
        let release_result = allocator.release(worker_id).await;
        assert!(
            matches!(release_result, Err(WorkerAllocatorError::EtcdError(_))),
            "应返回 EtcdError, 实际: {:?}",
            release_result
        );
    }

    #[tokio::test]
    async fn test_allocator_get_allocated_id_none() {
        let client = mock_into_client(MockEtcdClientOps::new());
        let allocator = EtcdWorkerAllocator::new(client, 1, EtcdConfig::default())
            .await
            .unwrap();

        assert_eq!(allocator.get_allocated_id(), None);
    }

    #[tokio::test]
    async fn test_allocator_get_allocated_id_some() {
        let mut mock = MockEtcdClientOps::new();
        mock.expect_kv_get().returning(|key| {
            if key.ends_with("/0") {
                Ok(Some(b"taken".to_vec()))
            } else {
                Ok(None)
            }
        });
        mock.expect_lease_grant().returning(|_| Ok(123));
        mock.expect_txn_check_create_rev_and_put()
            .returning(|_, _, _| Ok(true));

        let allocator = EtcdWorkerAllocator::new(mock_into_client(mock), 1, EtcdConfig::default())
            .await
            .unwrap();

        let worker_id = allocator.allocate().await.unwrap();
        assert!(worker_id >= 1);
        assert_eq!(allocator.get_allocated_id(), Some(worker_id));
    }

    #[tokio::test]
    async fn test_allocator_is_healthy_default_false() {
        let client = mock_into_client(MockEtcdClientOps::new());
        let allocator = EtcdWorkerAllocator::new(client, 1, EtcdConfig::default())
            .await
            .unwrap();

        assert!(!allocator.is_healthy(), "默认 health_status=0 应为 false");
    }

    // ===== EtcdDistributedLock tests (8) =====

    #[tokio::test]
    async fn test_lock_new_succeeds() {
        let client = mock_into_client(MockEtcdClientOps::new());
        let lock = EtcdDistributedLock::new(client, "/locks/".to_string()).await;

        assert!(lock.is_ok());
        let lock = lock.unwrap();
        assert!(lock.is_healthy(), "is_healthy 应始终返回 true");
    }

    #[tokio::test]
    async fn test_lock_acquire_succeeds_first_attempt() {
        let mut mock = MockEtcdClientOps::new();
        mock.expect_lease_grant().returning(|_| Ok(456));
        mock.expect_txn_check_create_rev_and_put()
            .returning(|_, _, _| Ok(true));
        // L9 修复后：guard drop 时会 spawn release task 调用 lease_revoke。
        // 用 `.returning` 不限制 times（0 次或多次），避免 fire-and-forget
        // task 时序不确定导致 mock 验证失败。
        mock.expect_lease_revoke().returning(|_| Ok(()));

        let lock = EtcdDistributedLock::new(mock_into_client(mock), "/locks/".to_string())
            .await
            .unwrap();

        let result = lock.acquire("key1", 5).await;
        assert!(result.is_ok());
        // guard 持有但不显式 release，drop 时 spawn release task
        let _guard = result.unwrap();
    }

    #[tokio::test]
    async fn test_lock_acquire_retries_on_conflict() {
        let mut mock = MockEtcdClientOps::new();
        mock.expect_lease_grant().times(2).returning(|_| Ok(456));

        let mut seq = mockall::Sequence::new();
        mock.expect_txn_check_create_rev_and_put()
            .times(1)
            .in_sequence(&mut seq)
            .returning(|_, _, _| Ok(false)); // 第一次冲突
        mock.expect_txn_check_create_rev_and_put()
            .times(1)
            .in_sequence(&mut seq)
            .returning(|_, _, _| Ok(true)); // 第二次成功

        // L9 修复后：冲突时撤销 1 次 + guard drop 时可能撤销 1 次。
        // 用 `.returning` 不限制 times，避免 fire-and-forget task 时序问题。
        mock.expect_lease_revoke().returning(|_| Ok(()));

        let lock = EtcdDistributedLock::new(mock_into_client(mock), "/locks/".to_string())
            .await
            .unwrap();

        let result = lock.acquire("key1", 5).await;
        assert!(result.is_ok(), "重试后应成功, 实际: {:?}", result.err());
    }

    #[tokio::test]
    async fn test_lock_acquire_fails_after_max_retries() {
        let mut mock = MockEtcdClientOps::new();
        mock.expect_lease_grant().times(3).returning(|_| Ok(456));
        mock.expect_txn_check_create_rev_and_put()
            .times(3)
            .returning(|_, _, _| Ok(false)); // 3 次全部冲突
        mock.expect_lease_revoke().times(3).returning(|_| Ok(())); // 每次撤销

        let lock = EtcdDistributedLock::new(mock_into_client(mock), "/locks/".to_string())
            .await
            .unwrap();

        let result = lock.acquire("key1", 5).await;
        assert!(
            matches!(result, Err(LockError::AcquireFailed { .. })),
            "应返回 AcquireFailed, 实际: {:?}",
            result.as_ref().err()
        );
    }

    #[tokio::test]
    async fn test_lock_acquire_lease_grant_fails() {
        let mut mock = MockEtcdClientOps::new();
        mock.expect_lease_grant()
            .returning(|_| Err(EtcdError::LeaseInvalid("grant failed".into())));

        let lock = EtcdDistributedLock::new(mock_into_client(mock), "/locks/".to_string())
            .await
            .unwrap();

        let result = lock.acquire("key1", 5).await;
        assert!(
            matches!(result, Err(LockError::EtcdError(_))),
            "应返回 EtcdError, 实际: {:?}",
            result.as_ref().err()
        );
    }

    #[tokio::test]
    async fn test_lock_is_healthy_always_true() {
        let client = mock_into_client(MockEtcdClientOps::new());
        let lock = EtcdDistributedLock::new(client, "/locks/".to_string())
            .await
            .unwrap();

        assert!(lock.is_healthy());
    }

    #[tokio::test]
    async fn test_lock_guard_release_succeeds() {
        let mut mock = MockEtcdClientOps::new();
        // L9 修复后：显式 release 成功 → released=true → drop 跳过，
        // 因此 lease_revoke 只会被调用 1 次。使用 `.times(1)` 验证此行为。
        mock.expect_lease_revoke().times(1).returning(|_| Ok(()));

        let lock = EtcdDistributedLock::new(mock_into_client(mock), "/locks/".to_string())
            .await
            .unwrap();

        let guard = EtcdLockGuard {
            lock,
            key: "test_key".to_string(),
            lease_id: 123,
            lock_path: "/locks/test_key".to_string(),
            released: Arc::new(AtomicBool::new(false)),
        };

        let result = guard.release().await;
        assert!(result.is_ok());
        // guard 在 test 结束时 drop，released=true → Drop 跳过 lease_revoke
    }

    #[tokio::test]
    async fn test_lock_guard_release_fails() {
        let mut mock = MockEtcdClientOps::new();
        // L9 修复后：显式 release 失败 → released 仍为 false → drop 时
        // spawn 后台 task 再次调用 lease_revoke。由于 spawn 是
        // fire-and-forget，时序不确定，用 `.returning` 不限制 times。
        mock.expect_lease_revoke()
            .returning(|_| Err(EtcdError::LeaseInvalid("revoke failed".into())));

        let lock = EtcdDistributedLock::new(mock_into_client(mock), "/locks/".to_string())
            .await
            .unwrap();

        let guard = EtcdLockGuard {
            lock,
            key: "test_key".to_string(),
            lease_id: 123,
            lock_path: "/locks/test_key".to_string(),
            released: Arc::new(AtomicBool::new(false)),
        };

        let result = guard.release().await;
        assert!(
            matches!(result, Err(LockError::ReleaseFailed { .. })),
            "应返回 ReleaseFailed, 实际: {:?}",
            result
        );
        // guard drop 时 spawn 后台 task 再次尝试 release（也会失败，仅 log warning）
    }

    // ===== EtcdClusterHealthMonitor with injected client tests (5) =====

    #[tokio::test]
    async fn test_health_monitor_new_with_client_succeeds() {
        let mut mock = MockEtcdClientOps::new();
        mock.expect_ping().times(1).returning(|| Ok(())); // 验证 client 被注入

        let config = EtcdConfig::default();
        let cache_file = NamedTempFile::new().expect("Failed to create temp file");
        let cache_path = cache_file.path().to_string_lossy().to_string();
        let monitor =
            EtcdClusterHealthMonitor::new_with_client(config, cache_path, mock_into_client(mock));

        assert_eq!(monitor.get_status(), EtcdClusterStatus::Healthy);
        assert!(!monitor.is_using_cache());

        // 调用 check_etcd_health 应走注入路径（调用 ping）
        monitor.check_etcd_health().await;
        assert_eq!(monitor.get_status(), EtcdClusterStatus::Healthy);
    }

    #[tokio::test]
    async fn test_health_monitor_check_with_client_healthy() {
        let mut mock = MockEtcdClientOps::new();
        mock.expect_ping().times(1).returning(|| Ok(()));

        let config = EtcdConfig::default();
        let cache_file = NamedTempFile::new().expect("Failed to create temp file");
        let cache_path = cache_file.path().to_string_lossy().to_string();
        let monitor =
            EtcdClusterHealthMonitor::new_with_client(config, cache_path, mock_into_client(mock));

        monitor.check_etcd_health().await;

        assert_eq!(monitor.get_status(), EtcdClusterStatus::Healthy);
        assert!(!monitor.is_using_cache());
    }

    #[tokio::test]
    async fn test_health_monitor_check_with_client_degraded() {
        let mut mock = MockEtcdClientOps::new();
        mock.expect_ping()
            .times(3)
            .returning(|| Err(EtcdError::Network("ping failed".into())));

        let config = EtcdConfig::default();
        let cache_file = NamedTempFile::new().expect("Failed to create temp file");
        let cache_path = cache_file.path().to_string_lossy().to_string();
        let monitor =
            EtcdClusterHealthMonitor::new_with_client(config, cache_path, mock_into_client(mock));

        for _ in 0..3 {
            monitor.check_etcd_health().await;
        }

        assert_eq!(monitor.get_status(), EtcdClusterStatus::Degraded);
        assert!(!monitor.is_using_cache(), "Degraded 状态不应启用本地缓存");
    }

    #[tokio::test]
    async fn test_health_monitor_check_with_client_failed() {
        let mut mock = MockEtcdClientOps::new();
        mock.expect_ping()
            .times(5)
            .returning(|| Err(EtcdError::Network("ping failed".into())));

        let config = EtcdConfig::default();
        let cache_file = NamedTempFile::new().expect("Failed to create temp file");
        let cache_path = cache_file.path().to_string_lossy().to_string();
        let monitor =
            EtcdClusterHealthMonitor::new_with_client(config, cache_path, mock_into_client(mock));

        for _ in 0..5 {
            monitor.check_etcd_health().await;
        }

        assert_eq!(monitor.get_status(), EtcdClusterStatus::Failed);
        assert!(monitor.is_using_cache(), "Failed 状态应启用本地缓存降级");
    }

    #[tokio::test]
    async fn test_health_monitor_check_with_client_recovered() {
        let mut mock = MockEtcdClientOps::new();
        let mut seq = mockall::Sequence::new();
        // 前 5 次 ping 失败 → Failed
        mock.expect_ping()
            .times(5)
            .in_sequence(&mut seq)
            .returning(|| Err(EtcdError::Network("down".into())));
        // 第 6 次 ping 成功 → 恢复 Healthy
        mock.expect_ping()
            .times(1)
            .in_sequence(&mut seq)
            .returning(|| Ok(()));

        let config = EtcdConfig::default();
        let cache_file = NamedTempFile::new().expect("Failed to create temp file");
        let cache_path = cache_file.path().to_string_lossy().to_string();
        let monitor =
            EtcdClusterHealthMonitor::new_with_client(config, cache_path, mock_into_client(mock));

        // 5 次失败 → Failed + cache
        for _ in 0..5 {
            monitor.check_etcd_health().await;
        }
        assert_eq!(monitor.get_status(), EtcdClusterStatus::Failed);
        assert!(monitor.is_using_cache());

        // 1 次成功 → 恢复
        monitor.check_etcd_health().await;
        assert_eq!(monitor.get_status(), EtcdClusterStatus::Healthy);
        assert!(!monitor.is_using_cache(), "恢复后应退出本地缓存模式");
    }

    // ===== EtcdError Display tests (2) =====

    #[test]
    fn test_etcd_error_display_network_and_lease_invalid() {
        let net_err = EtcdError::Network("conn refused".to_string());
        let display = net_err.to_string();
        assert!(
            display.contains("network"),
            "Network Display 应含 'network', 实际: {}",
            display
        );
        assert!(display.contains("conn refused"));

        let lease_err = EtcdError::LeaseInvalid("expired".to_string());
        let display = lease_err.to_string();
        assert!(
            display.contains("lease"),
            "LeaseInvalid Display 应含 'lease', 实际: {}",
            display
        );
        assert!(display.contains("expired"));
    }

    #[test]
    fn test_etcd_error_display_key_not_found_and_internal() {
        let nf_err = EtcdError::KeyNotFound("/workers/1".to_string());
        let display = nf_err.to_string();
        assert!(
            display.contains("not found"),
            "KeyNotFound Display 应含 'not found', 实际: {}",
            display
        );
        assert!(display.contains("/workers/1"));

        let int_err = EtcdError::Internal("unexpected".to_string());
        let display = int_err.to_string();
        assert!(
            display.contains("internal"),
            "Internal Display 应含 'internal', 实际: {}",
            display
        );
        assert!(display.contains("unexpected"));
    }
}
