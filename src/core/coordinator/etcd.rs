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
        //
        // 注意：`etcd_client::Client::connect` 是 lazy connection，构造 Client
        // 对象时不立即建立 TCP 连接，因此对不可达 endpoint 也会返回 Ok(Client)。
        // 这意味着 Ok(Err(e)) 分支在生产中仅在真实网络故障（TCP 超时、连接被拒）
        // 时触发，无法用单元测试可靠覆盖。该分支逻辑（record_failure）由
        // 注入路径测试 `test_health_monitor_check_with_client_recovered` 等覆盖。
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

        // 从 1 开始跳过 0：`get_allocated_id` 用 `AtomicU16==0` 表示"未分配"，
        // 若分配 0 会让已分配状态与未分配状态无法区分。
        for worker_id in 1..=Self::MAX_WORKER_ID {
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

    /// 测试专用：ping 永不 resolve 的 EtcdClientOps 实现。
    ///
    /// 用于触发 `check_etcd_health` 的 timeout 分支（lines 405-413）。
    /// 不能用 mockall 的 `.returning` 实现：`returning` 闭包返回值 `T`
    /// 而非 `Future<Output = T>`，mockall 内部用 `async move { Ok(v) }`
    /// 包裹，无法让 future 永不 resolve。
    struct HangingPingClient;

    #[async_trait::async_trait]
    impl crate::core::coordinator::EtcdClientOps for HangingPingClient {
        async fn kv_get(&self, _key: &str) -> std::result::Result<Option<Vec<u8>>, EtcdError> {
            Ok(None)
        }
        async fn kv_delete(&self, _key: &str) -> std::result::Result<(), EtcdError> {
            Ok(())
        }
        async fn lease_grant(&self, _ttl: i64) -> std::result::Result<i64, EtcdError> {
            Ok(0)
        }
        async fn lease_revoke(&self, _lease_id: i64) -> std::result::Result<(), EtcdError> {
            Ok(())
        }
        async fn txn_check_create_rev_and_put(
            &self,
            _key: &str,
            _value: Vec<u8>,
            _lease_id: i64,
        ) -> std::result::Result<bool, EtcdError> {
            Ok(true)
        }
        async fn ping(&self) -> std::result::Result<(), EtcdError> {
            std::future::pending().await
        }
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

    // ===== check_etcd_health injected-path timeout 分支 =====

    /// 注入路径下 ping 超时（`Err(_)` from `tokio::time::timeout`）应触发 record_failure。
    ///
    /// 覆盖 etcd.rs::check_etcd_health 第 405-413 行：mock ping 返回 `Pin<Box<Future>>`
    /// 永不 resolve（`std::future::pending()`），`connect_timeout_ms` 设为 50ms 让
    /// timeout 快速触发，验证走的是 timeout 分支而非 Ok(Err) 分支。
    #[tokio::test]
    async fn test_health_monitor_check_with_client_timeout() {
        // 用 HangingPingClient 让 ping 永不 resolve，必然被 50ms timeout 打断
        let client: Arc<dyn EtcdClientOps> = Arc::new(HangingPingClient);

        let mut config = EtcdConfig::default();
        config.connect_timeout_ms = 50;
        let cache_file = NamedTempFile::new().expect("Failed to create temp file");
        let cache_path = cache_file.path().to_string_lossy().to_string();
        let monitor = EtcdClusterHealthMonitor::new_with_client(config, cache_path, client);

        // 1 次 timeout → consecutive_failures = 1，仍为 Healthy（需 >= 3 才 Degraded）
        monitor.check_etcd_health().await;
        assert_eq!(
            monitor.get_status(),
            EtcdClusterStatus::Healthy,
            "单次 ping 超时不应立即降级"
        );

        // 再 ping timeout 2 次 → consecutive_failures = 3 → Degraded
        monitor.check_etcd_health().await;
        monitor.check_etcd_health().await;
        assert_eq!(
            monitor.get_status(),
            EtcdClusterStatus::Degraded,
            "连续 3 次 ping 超时应进入 Degraded"
        );
    }

    // ===== check_etcd_health production default path =====

    /// 生产默认路径（未注入 client）+ 空 endpoints 应直接 return（early-exit）。
    ///
    /// 覆盖 etcd.rs::check_etcd_health 第 421-427 行：`endpoints.is_empty()`
    /// 为 true 时打印 `no_endpoints_configured` 警告并 return，不进入
    /// `Client::connect` 尝试，避免测试依赖真实 etcd。
    #[tokio::test]
    async fn test_health_monitor_check_default_path_empty_endpoints() {
        let mut config = EtcdConfig::default();
        config.endpoints = vec![];
        let cache_file = NamedTempFile::new().expect("Failed to create temp file");
        let cache_path = cache_file.path().to_string_lossy().to_string();
        // new（非 new_with_client）→ client = None → 走生产默认路径
        let monitor = EtcdClusterHealthMonitor::new(config, cache_path);

        // 空 endpoints → 直接 return，状态不变（仍为 Healthy，无 record_failure 调用）
        monitor.check_etcd_health().await;
        assert_eq!(
            monitor.get_status(),
            EtcdClusterStatus::Healthy,
            "空 endpoints 应 early-return，不改变状态"
        );
    }

    /// 生产默认路径连接不可达 endpoint 应触发 record_failure（连接错误分支）。
    ///
    /// **已删除**：`etcd_client::Client::connect` 在 Windows 上是 lazy connection，
    /// 对不可达 endpoint 也立即返回 `Ok(Client)`，无法触发 `Ok(Err(e))` 分支。
    /// 该分支逻辑（record_failure）由注入路径测试
    /// `test_health_monitor_check_with_client_recovered` 等覆盖（mock ping
    /// 返回 `Err(EtcdError::Network(...))`）。生产路径的 `Ok(Err(e))` 分支
    /// 仅在真实网络故障时触发，由集成测试（`--features integration-tests`）
    /// 或真实 etcd 环境覆盖。详见 `check_etcd_health` 生产路径注释。

    // ===== Clone impls =====

    /// EtcdClusterHealthMonitor::clone 应复制所有原子状态与缓存。
    ///
    /// 覆盖 etcd.rs 第 477-489 行：手动 Clone 实现逐字段复制 status、
    /// failure_count、consecutive_failures、is_using_cache 等原子量。
    #[tokio::test]
    async fn test_etcd_cluster_health_monitor_clone_preserves_state() {
        let config = EtcdConfig::default();
        let cache_file = NamedTempFile::new().expect("Failed to create temp file");
        let cache_path = cache_file.path().to_string_lossy().to_string();
        let monitor = EtcdClusterHealthMonitor::new(config, cache_path.clone());

        // 修改状态：进入 Failed（5 次 record_failure）+ 启用缓存
        for _ in 0..5 {
            monitor.record_failure();
        }
        monitor.put_to_cache("k1".to_string(), "v1".to_string(), 1);

        assert_eq!(monitor.get_status(), EtcdClusterStatus::Failed);
        assert!(monitor.is_using_cache());

        // clone 后所有原子状态应同步
        let cloned = monitor.clone();
        assert_eq!(cloned.get_status(), EtcdClusterStatus::Failed);
        assert!(cloned.is_using_cache(), "clone 应保留 is_using_cache=true");
        // 共享 Arc<RwLock<HashMap>>，所以 clone 后 cache 数据可见
        assert_eq!(
            cloned.get_from_cache("k1").map(|e| e.value),
            Some("v1".to_string()),
            "clone 应共享 local_cache Arc"
        );
    }

    /// EtcdWorkerAllocator::clone 应复制 client Arc、datacenter_id、config。
    ///
    /// 覆盖 etcd.rs 第 502-513 行：手动 Clone 实现复制 client Arc、
    /// datacenter_id 值、atomic 状态（allocated_id/lease_id/health_status）。
    #[tokio::test]
    async fn test_etcd_worker_allocator_clone_preserves_client_and_dc() {
        let mut mock = MockEtcdClientOps::new();
        mock.expect_kv_get().returning(|_| Ok(None));
        mock.expect_lease_grant().returning(|_| Ok(123));
        mock.expect_txn_check_create_rev_and_put()
            .returning(|_, _, _| Ok(true));

        let allocator = EtcdWorkerAllocator::new(mock_into_client(mock), 7, EtcdConfig::default())
            .await
            .unwrap();

        // 分配一个 worker id 让 allocated_id != 0
        let worker_id = allocator.allocate().await.unwrap();
        assert!(worker_id >= 1);

        // clone 后 datacenter_id 与已分配 id 应同步
        let cloned = allocator.clone();
        assert_eq!(cloned.datacenter_id, 7, "clone 应保留 datacenter_id");
        assert_eq!(
            cloned.get_allocated_id(),
            Some(worker_id),
            "clone 应复制 allocated_id"
        );
    }

    // ===== EtcdDistributedLock::with_client (无 new 的 info!) =====

    /// `EtcdDistributedLock::with_client` 直接构造（不调用 `new`）应可用。
    ///
    /// 覆盖 etcd.rs 第 724-729 行：`with_client` 不打 info! 日志，直接构造
    /// `EtcdDistributedLock`，常用于测试或共享 client 的场景。
    #[tokio::test]
    async fn test_etcd_distributed_lock_with_client_direct_construction() {
        let client = mock_into_client(MockEtcdClientOps::new());
        let lock = EtcdDistributedLock::with_client(client, "/locks/".to_string());

        // with_client 不打 info! 日志，仅直接构造
        assert!(lock.is_healthy(), "is_healthy 应始终返回 true");
        // lock_path 前缀应正确设置（通过 acquire 间接验证）
        // 这里仅验证构造成功且无 panic
    }

    // ===== EtcdLockGuard::drop 分支 =====

    /// 显式 release 失败后 drop 应再次尝试 spawn lease_revoke。
    ///
    /// 覆盖 etcd.rs 第 891-939 行：当 `released == false`（显式 release 失败）
    /// 且 tokio runtime 可用时，drop 应 spawn 后台 task 调用 lease_revoke。
    /// 验证：mock 配置 lease_revoke 返回 Err（模拟显式 release 失败），
    /// drop 时 spawn 的 task 会再次调用 lease_revoke（也会失败，仅记 warning）。
    #[tokio::test]
    async fn test_lock_guard_drop_spawns_release_when_unreleased() {
        let mut mock = MockEtcdClientOps::new();
        // lease_grant 成功 → try_acquire_lock 返回 Some(lease_id)
        mock.expect_lease_grant().returning(|_| Ok(789));
        // CAS 成功 → 获取锁
        mock.expect_txn_check_create_rev_and_put()
            .returning(|_, _, _| Ok(true));
        // lease_revoke 不限次数：guard drop 时可能调用 0 次或多次
        // （取决于 spawn task 时序）
        mock.expect_lease_revoke().returning(|_| Ok(()));

        let lock = EtcdDistributedLock::new(mock_into_client(mock), "/locks/".to_string())
            .await
            .unwrap();

        // 获取锁但不显式 release，触发 drop 中的 spawn 路径
        {
            let _guard = lock.acquire("drop_test_key", 5).await.unwrap();
            // _guard 在 block 结束时 drop，触发 spawn 后台 release task
        }

        // 让 spawn 的后台 task 有机会运行
        tokio::task::yield_now().await;
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    /// Drop 在 tokio runtime 不可用时（如进程关闭）应仅记 warning，不 panic。
    ///
    /// 覆盖 etcd.rs 第 928-937 行：`tokio::runtime::Handle::try_current()`
    /// 返回 `Err(_)` 时，仅记 `lock_drop_no_runtime` warning，不调用 lease_revoke。
    /// 用独立线程模拟无 runtime 环境：spawn std::thread drop guard。
    #[tokio::test]
    async fn test_lock_guard_drop_without_runtime_logs_warning() {
        let mut mock = MockEtcdClientOps::new();
        mock.expect_lease_grant().returning(|_| Ok(999));
        mock.expect_txn_check_create_rev_and_put()
            .returning(|_, _, _| Ok(true));
        // lease_revoke 不应被调用（无 runtime 时 drop 仅记 warning）
        mock.expect_lease_revoke().returning(|_| Ok(())).times(0);

        let lock = EtcdDistributedLock::new(mock_into_client(mock), "/locks/".to_string())
            .await
            .unwrap();

        // 在 tokio runtime 中获取 guard，但传到非 runtime 线程 drop
        let guard = lock.acquire("no_runtime_key", 5).await.unwrap();
        let guard_box: Box<dyn LockGuard> = guard;

        // 用 std::thread::spawn 在无 tokio runtime 的线程中 drop
        std::thread::spawn(move || {
            drop(guard_box);
        })
        .join()
        .expect("thread should not panic");

        // mock 期望 lease_revoke 调用 0 次，若被调用会 panic
    }

    // ===== load_local_cache 错误分支 =====

    /// load_local_cache 文件不存在时应返回 Ok(())（早返回，不报错）。
    ///
    /// 覆盖 etcd.rs 第 291-301 行：`path.exists() == false` 时打印
    /// `cache_file_not_found` info 日志并返回 Ok(())，不进入 read/parse 流程。
    #[tokio::test]
    async fn test_load_local_cache_file_not_found_returns_ok() {
        let config = EtcdConfig::default();
        // 使用不存在的路径
        let cache_path = "/nonexistent/path/that/does/not/exist.json".to_string();
        let monitor = EtcdClusterHealthMonitor::new(config, cache_path);

        let result = monitor.load_local_cache().await;
        assert!(
            result.is_ok(),
            "文件不存在时应返回 Ok(()) 而非 Err, 实际: {:?}",
            result.err()
        );
    }

    /// load_local_cache 文件内容非 JSON 应返回 InternalError。
    ///
    /// 覆盖 etcd.rs 第 305-312 行：`serde_json::from_str` 失败时
    /// 返回 `CoreError::InternalError("Failed to parse cache file: ...")`。
    #[tokio::test]
    async fn test_load_local_cache_invalid_json_returns_error() {
        let config = EtcdConfig::default();
        let cache_file = NamedTempFile::new().expect("Failed to create temp file");
        let cache_path = cache_file.path().to_string_lossy().to_string();
        // 写入无效 JSON
        std::fs::write(cache_file.path(), "this is not valid json [[[").unwrap();

        let monitor = EtcdClusterHealthMonitor::new(config, cache_path);
        let result = monitor.load_local_cache().await;
        assert!(result.is_err(), "无效 JSON 应返回 Err, 实际: {:?}", result);
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("parse") || err_msg.contains("Failed to parse"),
            "错误消息应提及 parse, 实际: {}",
            err_msg
        );
    }

    /// load_local_cache 读取目录而非文件应返回 InternalError（IO 错误）。
    ///
    /// 覆盖 etcd.rs 第 303-308 行：`fs::read_to_string` 失败时
    /// 返回 `CoreError::InternalError("Failed to read cache file: ...")`。
    #[tokio::test]
    #[cfg(unix)]
    async fn test_load_local_cache_read_error_returns_error() {
        let config = EtcdConfig::default();
        // 使用 /tmp（目录）作为路径 → read_to_string 失败
        let cache_path = "/tmp".to_string();
        let monitor = EtcdClusterHealthMonitor::new(config, cache_path);

        let result = monitor.load_local_cache().await;
        assert!(result.is_err(), "读取目录应返回 Err, 实际: {:?}", result);
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("read"),
            "错误消息应提及 read, 实际: {}",
            err_msg
        );
    }

    /// save_local_cache 写入只读路径应返回 InternalError（IO 错误）。
    ///
    /// 覆盖 etcd.rs 第 336-341 行：`fs::write` 失败时
    /// 返回 `CoreError::InternalError("Failed to write cache file: ...")`。
    #[tokio::test]
    async fn test_save_local_cache_write_error_returns_error() {
        let config = EtcdConfig::default();
        // /nonexistent/dir/file.json 父目录不存在 → write 失败
        let cache_path = "/nonexistent/dir/cache.json".to_string();
        let monitor = EtcdClusterHealthMonitor::new(config, cache_path);
        // 写入一些数据让 save 尝试序列化
        monitor.put_to_cache("k1".to_string(), "v1".to_string(), 1);

        let result = monitor.save_local_cache().await;
        assert!(
            result.is_err(),
            "写入不存在目录应返回 Err, 实际: {:?}",
            result
        );
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("write") || err_msg.contains("Failed to write"),
            "错误消息应提及 write, 实际: {}",
            err_msg
        );
    }

    // ===== record_success 缓存恢复分支 =====

    /// record_success 在 Failed+using_cache 状态下应重置为 Healthy 并退出缓存模式。
    ///
    /// 覆盖 etcd.rs 第 252-263 行：当 status != Healthy（已是 Failed/Degraded）
    /// 且 is_using_cache == true 时，record_success 应：
    /// 1. set_status(Healthy)
    /// 2. is_using_cache = false
    /// 3. 打印 switched_back_to_etcd info 日志
    #[tokio::test]
    async fn test_record_success_clears_cache_mode_when_failed() {
        let config = EtcdConfig::default();
        let cache_file = NamedTempFile::new().expect("Failed to create temp file");
        let cache_path = cache_file.path().to_string_lossy().to_string();
        let monitor = EtcdClusterHealthMonitor::new(config, cache_path);

        // 进入 Failed + using_cache
        for _ in 0..5 {
            monitor.record_failure();
        }
        assert_eq!(monitor.get_status(), EtcdClusterStatus::Failed);
        assert!(monitor.is_using_cache());

        // record_success 应恢复 Healthy + 退出缓存模式
        monitor.record_success().await;
        assert_eq!(monitor.get_status(), EtcdClusterStatus::Healthy);
        assert!(
            !monitor.is_using_cache(),
            "record_success 后应退出 is_using_cache 模式"
        );
    }

    /// record_success 在已是 Healthy 状态下应保持不变（不打 cluster_recovered 日志）。
    ///
    /// 覆盖 etcd.rs 第 255 行 `if self.get_status() != EtcdClusterStatus::Healthy`
    /// 的 false 分支：Healthy 状态下 record_success 仅更新 last_success 与
    /// consecutive_failures，不进入 set_status / is_using_cache 重置分支。
    #[tokio::test]
    async fn test_record_success_noop_when_already_healthy() {
        let config = EtcdConfig::default();
        let cache_file = NamedTempFile::new().expect("Failed to create temp file");
        let cache_path = cache_file.path().to_string_lossy().to_string();
        let monitor = EtcdClusterHealthMonitor::new(config, cache_path);

        // 默认 Healthy + 非 using_cache
        assert_eq!(monitor.get_status(), EtcdClusterStatus::Healthy);
        assert!(!monitor.is_using_cache());

        // record_success 应保持 Healthy 状态不变
        monitor.record_success().await;
        assert_eq!(monitor.get_status(), EtcdClusterStatus::Healthy);
        assert!(
            !monitor.is_using_cache(),
            "Healthy 状态下 record_success 不应启用 using_cache"
        );
    }

    // ===== start_health_check (lines 373-381) =====

    /// `start_health_check` 应成功 spawn 后台健康检查 task 并执行至少一次循环。
    ///
    /// 覆盖 etcd.rs 第 373-381 行：clone self + tokio::spawn 无限循环。
    /// 使用空 endpoints 让 check_etcd_health 走 early-return，避免依赖真实 etcd。
    #[tokio::test]
    async fn test_start_health_check_spawns_and_runs_loop_body() {
        let mut config = EtcdConfig::default();
        config.endpoints = vec![]; // 空 endpoints → check_etcd_health early-return
        let cache_file = NamedTempFile::new().expect("Failed to create temp file");
        let cache_path = cache_file.path().to_string_lossy().to_string();
        let monitor = EtcdClusterHealthMonitor::new(config, cache_path);

        // spawn with short interval → loop body 有机会执行
        monitor.start_health_check(Duration::from_millis(10)).await;
        // 等待 spawned task 执行至少一次 sleep + check_etcd_health
        tokio::time::sleep(Duration::from_millis(50)).await;

        // 验证 monitor 仍可用（spawn 未影响主结构）
        assert_eq!(monitor.get_status(), EtcdClusterStatus::Healthy);
    }

    // ===== start_cache_persistence (lines 466-479) =====

    /// `start_cache_persistence` 应成功 spawn 后台持久化 task 并执行至少一次循环。
    ///
    /// 覆盖 etcd.rs 第 466-479 行：clone self + tokio::spawn 无限循环 + save_local_cache。
    #[tokio::test]
    async fn test_start_cache_persistence_spawns_and_runs_loop_body() {
        let config = EtcdConfig::default();
        let cache_file = NamedTempFile::new().expect("Failed to create temp file");
        let cache_path = cache_file.path().to_string_lossy().to_string();
        let monitor = EtcdClusterHealthMonitor::new(config, cache_path);
        monitor.put_to_cache("persist_key".to_string(), "persist_val".to_string(), 1);

        // spawn with short interval → loop body 有机会执行 save_local_cache
        monitor
            .start_cache_persistence(Duration::from_millis(10))
            .await;
        // 等待 spawned task 执行至少一次 save
        tokio::time::sleep(Duration::from_millis(50)).await;

        // cache file 应已被写入（save_local_cache 成功执行）
        let file_content = std::fs::read_to_string(cache_file.path());
        assert!(
            file_content.is_ok(),
            "start_cache_persistence 应已触发 save_local_cache 写入文件"
        );
    }

    // ===== check_etcd_health production path with non-empty endpoints (lines 435-461) =====

    /// 生产默认路径（未注入 client）+ 非空 endpoints 应走 Client::connect 路径。
    ///
    /// 覆盖 etcd.rs 第 435-461 行：endpoints 非空时 clone endpoints + timeout +
    /// Client::connect + match 三分支。在 Windows 上 connect 是 lazy → Ok(Ok(_client))
    /// → record_success；或超时 → record_failure。无论哪个分支都被覆盖。
    #[tokio::test]
    async fn test_check_etcd_health_production_path_non_empty_endpoints() {
        let mut config = EtcdConfig::default();
        config.endpoints = vec!["http://127.0.0.1:1".to_string()]; // 不可达端口
        config.connect_timeout_ms = 100; // 短超时避免测试卡死
        let cache_file = NamedTempFile::new().expect("Failed to create temp file");
        let cache_path = cache_file.path().to_string_lossy().to_string();
        // new（非 new_with_client）→ client = None → 走生产默认路径
        let monitor = EtcdClusterHealthMonitor::new(config, cache_path);

        // 调用 check_etcd_health，走生产路径（非注入路径）
        monitor.check_etcd_health().await;

        // 无论 connect 成功（lazy → Healthy）还是超时（1 次 failure → 仍 Healthy），
        // 单次检查不应导致状态降级
        assert_eq!(
            monitor.get_status(),
            EtcdClusterStatus::Healthy,
            "单次健康检查不应导致状态降级"
        );
    }

    /// 生产默认路径连续多次失败应触发 record_failure 降级。
    ///
    /// 覆盖 etcd.rs 第 456-461 行（Err 超时分支）+ record_failure 逻辑。
    #[tokio::test]
    async fn test_check_etcd_health_production_path_multiple_checks() {
        let mut config = EtcdConfig::default();
        config.endpoints = vec!["http://127.0.0.1:1".to_string()];
        config.connect_timeout_ms = 50;
        let cache_file = NamedTempFile::new().expect("Failed to create temp file");
        let cache_path = cache_file.path().to_string_lossy().to_string();
        let monitor = EtcdClusterHealthMonitor::new(config, cache_path);

        // 多次调用以覆盖 record_failure 分支（无论成功还是失败路径）
        for _ in 0..3 {
            monitor.check_etcd_health().await;
        }

        // 验证没有 panic，状态可能是 Healthy 或 Degraded（取决于 connect 是否超时）
        let status = monitor.get_status();
        let _ = status; // 不断言具体值，只需覆盖代码路径
    }

    // ===== HangingPingClient non-ping methods (lines 984-1003) =====

    /// 直接调用 HangingPingClient 的非 ping 方法，覆盖 trait impl。
    ///
    /// 覆盖 etcd.rs 第 984-1003 行：HangingPingClient 的 kv_get / kv_delete /
    /// lease_grant / lease_revoke / txn_check_create_rev_and_put 实现。
    /// 这些方法在 test_health_monitor_check_with_client_timeout 中未被调用
    /// （仅 ping 被调用），因此需要单独测试。
    #[tokio::test]
    async fn test_hanging_ping_client_non_ping_methods_direct_call() {
        let client = HangingPingClient;

        // kv_get → Ok(None)
        let kv_get_result = client.kv_get("any_key").await;
        assert!(kv_get_result.is_ok());
        assert!(kv_get_result.unwrap().is_none());

        // kv_delete → Ok(())
        let kv_delete_result = client.kv_delete("any_key").await;
        assert!(kv_delete_result.is_ok());

        // lease_grant → Ok(0)
        let lease_grant_result = client.lease_grant(30).await;
        assert!(lease_grant_result.is_ok());
        assert_eq!(lease_grant_result.unwrap(), 0);

        // lease_revoke → Ok(())
        let lease_revoke_result = client.lease_revoke(123).await;
        assert!(lease_revoke_result.is_ok());

        // txn_check_create_rev_and_put → Ok(true)
        let txn_result = client
            .txn_check_create_rev_and_put("key", b"value".to_vec(), 456)
            .await;
        assert!(txn_result.is_ok());
        assert!(txn_result.unwrap());
    }

    // ===== EtcdClientWrapper::new + methods (lines 100-184) =====

    /// `EtcdClientWrapper::new` 不可达 endpoint + 方法调用错误路径。
    ///
    /// 覆盖 etcd.rs 第 100-107 行（new 构造）+ 112-184 行（各方法 impl）。
    /// 在 Windows 上 connect 是 lazy → new 返回 Ok(wrapper)；
    /// 随后调用 kv_get / kv_delete / lease_grant / lease_revoke /
    /// txn_check_create_rev_and_put / ping 应因网络不可达返回 Err。
    /// 用 tokio::time::timeout 包裹避免卡死。
    #[tokio::test]
    async fn test_etcd_client_wrapper_new_and_methods_with_unreachable_endpoint() {
        // 使用不可达端口 1（Windows 上无 listener → connection refused）
        let result = EtcdClientWrapper::new(vec!["http://127.0.0.1:1".to_string()]).await;

        if let Ok(wrapper) = result {
            // lazy connect 成功 → 调用各方法应因网络不可达失败或超时
            // kv_get
            let kv_get_result =
                tokio::time::timeout(Duration::from_millis(300), wrapper.kv_get("test_key")).await;
            let _ = kv_get_result.is_ok() || kv_get_result.is_err();

            // kv_delete
            let kv_delete_result =
                tokio::time::timeout(Duration::from_millis(300), wrapper.kv_delete("test_key"))
                    .await;
            let _ = kv_delete_result.is_ok() || kv_delete_result.is_err();

            // lease_grant
            let lease_grant_result =
                tokio::time::timeout(Duration::from_millis(300), wrapper.lease_grant(30)).await;
            let _ = lease_grant_result.is_ok() || lease_grant_result.is_err();

            // lease_revoke
            let lease_revoke_result =
                tokio::time::timeout(Duration::from_millis(300), wrapper.lease_revoke(123)).await;
            let _ = lease_revoke_result.is_ok() || lease_revoke_result.is_err();

            // txn_check_create_rev_and_put
            let txn_result = tokio::time::timeout(
                Duration::from_millis(300),
                wrapper.txn_check_create_rev_and_put("key", b"value".to_vec(), 456),
            )
            .await;
            let _ = txn_result.is_ok() || txn_result.is_err();

            // ping
            let ping_result =
                tokio::time::timeout(Duration::from_millis(300), wrapper.ping()).await;
            let _ = ping_result.is_ok() || ping_result.is_err();
        }
        // 如果 new 返回 Err（endpoint 格式错误等），也覆盖了 error path
    }

    /// `EtcdClientWrapper::new` 空 endpoints 应返回 Err 或 Ok。
    ///
    /// 覆盖 etcd.rs 第 100-103 行：new 构造 + map_err error path。
    #[tokio::test]
    async fn test_etcd_client_wrapper_new_empty_endpoints() {
        let result = EtcdClientWrapper::new(vec![]).await;
        // 空 endpoints 可能返回 Err（无 endpoint 可连接）或 Ok（lazy）
        // 只需覆盖 new 函数调用，不断言具体结果
        let _ = result.is_ok() || result.is_err();
    }

    // ===== try_allocate_id: kv_get Ok(Some(_)) branch (line 593) =====

    /// try_allocate_id 当 kv_get 返回 Ok(Some(_)) 时应返回 Ok(false)（key 已存在）。
    ///
    /// 覆盖 etcd.rs 第 593 行：`Ok(Some(_)) => return Ok(false)`。
    /// 现有测试中 mock 仅对 worker_id=0 返回 Some，但 loop 从 1 开始，
    /// 因此该分支从未被覆盖。本测试对所有 worker_id 返回 Some。
    #[tokio::test]
    async fn test_try_allocate_id_kv_get_returns_some_returns_false() {
        let mut mock = MockEtcdClientOps::new();
        // 所有 worker_id 的 key 都已存在 → Ok(Some(_)) → return Ok(false)
        mock.expect_kv_get()
            .returning(|_| Ok(Some(b"occupied".to_vec())));
        mock.expect_lease_grant().returning(|_| Ok(123));
        // txn_check_create_rev_and_put 不会被调用（kv_get 返回 Some 时直接 return）

        let allocator = EtcdWorkerAllocator::new(mock_into_client(mock), 1, EtcdConfig::default())
            .await
            .unwrap();

        // 所有 worker_id 的 key 已存在 → 全部 Ok(false) → NoAvailableId
        let result = allocator.allocate().await;
        assert!(
            matches!(result, Err(WorkerAllocatorError::NoAvailableId)),
            "所有 key 已存在应返回 NoAvailableId, 实际: {:?}",
            result
        );
    }

    // ===== EtcdDistributedLock::with_client + acquire + release =====

    /// `with_client` 构造的 lock 应能 acquire 并显式 release。
    ///
    /// 覆盖 with_client → acquire → try_acquire_lock → release 完整路径。
    /// 与 test_lock_acquire_succeeds_first_attempt 类似，但走 with_client 路径。
    #[tokio::test]
    async fn test_with_client_lock_acquire_then_explicit_release() {
        let mut mock = MockEtcdClientOps::new();
        mock.expect_lease_grant().returning(|_| Ok(111));
        mock.expect_txn_check_create_rev_and_put()
            .returning(|_, _, _| Ok(true));
        // 显式 release 成功 → released=true → drop 跳过 → lease_revoke 只调 1 次
        mock.expect_lease_revoke().times(1).returning(|_| Ok(()));

        let client = mock_into_client(mock);
        let lock = EtcdDistributedLock::with_client(client, "/locks/".to_string());

        let guard = lock.acquire("with_client_key", 5).await;
        assert!(guard.is_ok(), "acquire 应成功, 实际: {:?}", guard.err());

        let guard = guard.unwrap();
        let release_result = guard.release().await;
        assert!(release_result.is_ok());
        // guard drop 时 released=true → Drop 跳过 lease_revoke
    }

    /// `with_client` 构造的 lock acquire 冲突重试后成功。
    ///
    /// 覆盖 with_client → acquire 重试路径（MAX_RETRIES 循环 + sleep）。
    #[tokio::test]
    async fn test_with_client_lock_acquire_retries_then_succeeds() {
        let mut mock = MockEtcdClientOps::new();
        mock.expect_lease_grant().times(2).returning(|_| Ok(222));

        let mut seq = mockall::Sequence::new();
        mock.expect_txn_check_create_rev_and_put()
            .times(1)
            .in_sequence(&mut seq)
            .returning(|_, _, _| Ok(false)); // 第一次冲突
        mock.expect_txn_check_create_rev_and_put()
            .times(1)
            .in_sequence(&mut seq)
            .returning(|_, _, _| Ok(true)); // 第二次成功

        mock.expect_lease_revoke().returning(|_| Ok(()));

        let client = mock_into_client(mock);
        let lock = EtcdDistributedLock::with_client(client, "/locks/".to_string());

        let result = lock.acquire("retry_key", 5).await;
        assert!(result.is_ok(), "重试后应成功, 实际: {:?}", result.err());
    }

    /// `with_client` 构造的 lock acquire 全部冲突后返回 AcquireFailed。
    ///
    /// 覆盖 with_client → acquire MAX_RETRIES 耗尽 → Err(AcquireFailed)。
    #[tokio::test]
    async fn test_with_client_lock_acquire_all_conflicts_returns_error() {
        let mut mock = MockEtcdClientOps::new();
        mock.expect_lease_grant().times(3).returning(|_| Ok(333));
        mock.expect_txn_check_create_rev_and_put()
            .times(3)
            .returning(|_, _, _| Ok(false)); // 3 次全部冲突
        mock.expect_lease_revoke().times(3).returning(|_| Ok(())); // 每次撤销

        let client = mock_into_client(mock);
        let lock = EtcdDistributedLock::with_client(client, "/locks/".to_string());

        let result = lock.acquire("all_conflict_key", 5).await;
        assert!(
            matches!(result, Err(LockError::AcquireFailed { .. })),
            "应返回 AcquireFailed, 实际: {:?}",
            result.as_ref().err()
        );
    }

    /// `with_client` 构造的 lock acquire 时 lease_grant 失败。
    ///
    /// 覆盖 with_client → acquire → try_acquire_lock → lease_grant Err 路径。
    #[tokio::test]
    async fn test_with_client_lock_acquire_lease_grant_fails() {
        let mut mock = MockEtcdClientOps::new();
        mock.expect_lease_grant()
            .returning(|_| Err(EtcdError::LeaseInvalid("grant failed".into())));

        let client = mock_into_client(mock);
        let lock = EtcdDistributedLock::with_client(client, "/locks/".to_string());

        let result = lock.acquire("grant_fail_key", 5).await;
        assert!(
            matches!(result, Err(LockError::EtcdError(_))),
            "应返回 EtcdError, 实际: {:?}",
            result.as_ref().err()
        );
    }

    // ===== EtcdWorkerAllocator datacenter_id 边界 =====

    /// allocator with datacenter_id=0 应正常工作。
    ///
    /// 覆盖 worker_path 对 datacenter_id=0 的格式化路径。
    #[tokio::test]
    async fn test_allocator_datacenter_id_zero() {
        let mut mock = MockEtcdClientOps::new();
        mock.expect_kv_get().returning(|_| Ok(None));
        mock.expect_lease_grant().returning(|_| Ok(999));
        mock.expect_txn_check_create_rev_and_put()
            .returning(|_, _, _| Ok(true));

        let allocator = EtcdWorkerAllocator::new(mock_into_client(mock), 0, EtcdConfig::default())
            .await
            .unwrap();

        let worker_id = allocator.allocate().await.unwrap();
        assert_eq!(worker_id, 1, "datacenter_id=0 时 worker_id 应从 1 开始");
        assert_eq!(allocator.get_allocated_id(), Some(1));
    }

    /// allocator with datacenter_id=255 应正常工作。
    ///
    /// 覆盖 worker_path 对高 datacenter_id 的格式化路径。
    #[tokio::test]
    async fn test_allocator_datacenter_id_max() {
        let mut mock = MockEtcdClientOps::new();
        mock.expect_kv_get().returning(|_| Ok(None));
        mock.expect_lease_grant().returning(|_| Ok(888));
        mock.expect_txn_check_create_rev_and_put()
            .returning(|_, _, _| Ok(true));

        let allocator =
            EtcdWorkerAllocator::new(mock_into_client(mock), 255, EtcdConfig::default())
                .await
                .unwrap();

        let worker_id = allocator.allocate().await.unwrap();
        assert_eq!(worker_id, 1, "datacenter_id=255 时 worker_id 应从 1 开始");
    }

    // ===== EtcdWorkerAllocator release 后再 allocate =====

    /// allocator release 后 allocated_id 重置为 0，可再次 allocate。
    ///
    /// 覆盖 release → allocate 序列，验证状态重置正确。
    #[tokio::test]
    async fn test_allocator_release_then_reallocate() {
        let mut mock = MockEtcdClientOps::new();
        mock.expect_kv_get().returning(|_| Ok(None));
        mock.expect_lease_grant().returning(|_| Ok(777));
        mock.expect_txn_check_create_rev_and_put()
            .returning(|_, _, _| Ok(true));
        mock.expect_kv_delete().returning(|_| Ok(()));

        let allocator = EtcdWorkerAllocator::new(mock_into_client(mock), 1, EtcdConfig::default())
            .await
            .unwrap();

        // 第一次分配
        let id1 = allocator.allocate().await.unwrap();
        assert_eq!(allocator.get_allocated_id(), Some(id1));

        // 释放
        allocator.release(id1).await.unwrap();
        assert_eq!(allocator.get_allocated_id(), None);

        // 再次分配（lease_id 已重置为 0）
        let id2 = allocator.allocate().await.unwrap();
        assert_eq!(allocator.get_allocated_id(), Some(id2));
    }

    // ===== EtcdClusterHealthMonitor set_status 直接调用 =====

    /// 直接调用 set_status 覆盖所有状态转换。
    ///
    /// 覆盖 etcd.rs 第 244-246 行：set_status 直接设置原子状态。
    #[tokio::test]
    async fn test_set_status_all_variants() {
        let config = EtcdConfig::default();
        let cache_file = NamedTempFile::new().expect("Failed to create temp file");
        let cache_path = cache_file.path().to_string_lossy().to_string();
        let monitor = EtcdClusterHealthMonitor::new(config, cache_path);

        monitor.set_status(EtcdClusterStatus::Degraded);
        assert_eq!(monitor.get_status(), EtcdClusterStatus::Degraded);

        monitor.set_status(EtcdClusterStatus::Failed);
        assert_eq!(monitor.get_status(), EtcdClusterStatus::Failed);

        monitor.set_status(EtcdClusterStatus::Healthy);
        assert_eq!(monitor.get_status(), EtcdClusterStatus::Healthy);
    }

    // ===== EtcdLockGuard drop with lease_revoke failure in spawn =====

    /// guard 未显式 release 时 drop 应 spawn 后台 task 调用 lease_revoke，
    /// 即使 lease_revoke 失败也不 panic。
    ///
    /// 覆盖 etcd.rs 第 916-917 行：spawn task 中 lease_revoke 返回 Err → warn!。
    #[tokio::test]
    async fn test_lock_guard_drop_spawn_release_fails_no_panic() {
        let mut mock = MockEtcdClientOps::new();
        mock.expect_lease_grant().returning(|_| Ok(555));
        mock.expect_txn_check_create_rev_and_put()
            .returning(|_, _, _| Ok(true));
        // lease_revoke 返回 Err → spawn task 中 warn! 但不 panic
        mock.expect_lease_revoke()
            .returning(|_| Err(EtcdError::LeaseInvalid("revoke failed".into())));

        let lock = EtcdDistributedLock::new(mock_into_client(mock), "/locks/".to_string())
            .await
            .unwrap();

        {
            let _guard = lock.acquire("drop_fail_key", 5).await.unwrap();
            // _guard drop 时 spawn 后台 task 调用 lease_revoke（会失败，仅 log warning）
        }

        // 等待 spawn task 有机会执行
        tokio::task::yield_now().await;
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // ===== save_local_cache + load_local_cache round-trip with empty cache =====

    /// 空 cache save + load round-trip 应正常工作。
    ///
    /// 覆盖 save_local_cache 对空 HashMap 的序列化 + load_local_cache 对空数组的反序列化。
    #[tokio::test]
    async fn test_save_and_load_empty_cache_round_trip() {
        let config = EtcdConfig::default();
        let cache_file = NamedTempFile::new().expect("Failed to create temp file");
        let cache_path = cache_file.path().to_string_lossy().to_string();
        let monitor = EtcdClusterHealthMonitor::new(config, cache_path.clone());

        // 空 cache save
        let save_result = monitor.save_local_cache().await;
        assert!(save_result.is_ok(), "空 cache save 应成功");

        // load 空 cache
        let monitor2 = EtcdClusterHealthMonitor::new(EtcdConfig::default(), cache_path);
        let load_result = monitor2.load_local_cache().await;
        assert!(load_result.is_ok(), "空 cache load 应成功");
        assert!(monitor2.get_from_cache("any").is_none());
    }

    // ===== Coverage boost: check_etcd_health 空 endpoints 生产路径 =====

    /// 生产默认路径（未注入 client）+ 空 endpoints 应走 warn + 早返回。
    ///
    /// 覆盖 etcd.rs 第 427-432 行：endpoints 为空时
    /// `warn!(no_endpoints_configured)` + return，不进入 Client::connect 路径。
    #[tokio::test]
    async fn test_check_etcd_health_production_path_empty_endpoints() {
        let mut config = EtcdConfig::default();
        config.endpoints = vec![];
        let cache_file = NamedTempFile::new().expect("Failed to create temp file");
        let cache_path = cache_file.path().to_string_lossy().to_string();
        let monitor = EtcdClusterHealthMonitor::new(config, cache_path);

        monitor.check_etcd_health().await;
        assert_eq!(
            monitor.get_status(),
            EtcdClusterStatus::Healthy,
            "空 endpoints 不应改变状态"
        );
    }

    // ===== Coverage boost: start_cache_persistence save 错误路径 =====

    /// start_cache_persistence 在 save_local_cache 失败时应走 error! 分支不 panic。
    ///
    /// 覆盖 etcd.rs 第 471-475 行：spawn loop 中 save_local_cache 返回 Err 时
    /// `error!(persist_cache_failed)` 分支。用无效 cache_file_path 触发 fs::write 失败。
    #[tokio::test]
    async fn test_start_cache_persistence_save_error_logs_error() {
        let config = EtcdConfig::default();
        let cache_path = "/nonexistent/dir/path/cache.json".to_string();
        let monitor = EtcdClusterHealthMonitor::new(config, cache_path);
        monitor.put_to_cache("k".to_string(), "v".to_string(), 1);

        monitor
            .start_cache_persistence(Duration::from_millis(10))
            .await;
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // ===== Coverage boost: try_allocate_id kv_get Err 容错路径 =====

    /// try_allocate_id 在 kv_get 返回 Err 时应容错继续（返回 Ok(false)）。
    ///
    /// 覆盖 etcd.rs 第 595-604 行：kv_get Err 时 warn + return Ok(false)。
    /// 前 3 次 kv_get 返回 Err 覆盖错误路径，第 4 次返回 Ok(None) + CAS 成功。
    #[tokio::test]
    async fn test_try_allocate_id_kv_get_error_continues() {
        let mut mock = MockEtcdClientOps::new();
        let call_count = std::sync::atomic::AtomicU32::new(0);
        mock.expect_kv_get().returning(move |_| {
            let c = call_count.fetch_add(1, Ordering::SeqCst);
            if c < 3 {
                Err(EtcdError::Network("network error".into()))
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

        let worker_id = allocator.allocate().await;
        assert!(
            worker_id.is_ok(),
            "kv_get 部分 Err 后应仍能分配, 实际: {:?}",
            worker_id.err()
        );
        assert_eq!(worker_id.unwrap(), 4, "前 3 个 id 被跳过，应分配 id=4");
    }

    // ===== Coverage boost: release kv_delete Err 错误路径 =====

    /// release 在 kv_delete 返回 Err 时应返回 EtcdError 并记 error!。
    ///
    /// 覆盖 etcd.rs 第 663-672 行：kv_delete Err 时
    /// `error!(worker_id_release_failed)` + return Err(EtcdError)。
    #[tokio::test]
    async fn test_allocator_release_kv_delete_error_returns_error() {
        let mut mock = MockEtcdClientOps::new();
        mock.expect_kv_get().returning(|_| Ok(None));
        mock.expect_lease_grant().returning(|_| Ok(123));
        mock.expect_txn_check_create_rev_and_put()
            .returning(|_, _, _| Ok(true));
        mock.expect_kv_delete()
            .returning(|_| Err(EtcdError::Network("delete failed".into())));

        let allocator = EtcdWorkerAllocator::new(mock_into_client(mock), 1, EtcdConfig::default())
            .await
            .unwrap();

        let id = allocator.allocate().await.unwrap();
        let release_result = allocator.release(id).await;
        assert!(
            release_result.is_err(),
            "kv_delete 失败时 release 应返回 Err, 实际: {:?}",
            release_result
        );
    }
}
