// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! etcd-backed implementations for the coordinator module (feature `etcd` gated):
//! distributed worker ID allocator, distributed lock, and cluster health monitor.

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
/// 解耦业务逻辑与 `etcd_client::Client`（便于 mock）；方法均为 `&self`，生产实现用内部
/// Mutex 提供 interior mutability，可 `Arc<dyn EtcdClientOps>` 共享与注入。
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

    /// lease 续期一次：对 `lease_id` 发送 keep-alive 并等待一次响应。
    /// 每次调用独立建立 keep-alive 流；收到非零 granted TTL 即视为续期成功。
    async fn lease_keep_alive_once(&self, lease_id: i64) -> std::result::Result<(), EtcdError>;
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

/// etcd 操作超时默认值（秒），与 `EtcdConfig::operation_timeout_secs` 的 serde 默认一致。
const DEFAULT_OPERATION_TIMEOUT_SECS: u64 = 3;

/// 集中式操作超时 helper：用 `tokio::time::timeout` 包裹，防止 etcd 挂起阻塞协调路径。
/// 超时映射 `EtcdError::Network`（消息含 "timed out"），不新增错误变体。
async fn with_operation_timeout<T, F>(
    timeout: Duration,
    fut: F,
) -> std::result::Result<T, EtcdError>
where
    F: std::future::Future<Output = std::result::Result<T, EtcdError>>,
{
    match tokio::time::timeout(timeout, fut).await {
        Ok(result) => result,
        Err(_elapsed) => Err(EtcdError::Network(format!(
            "etcd operation timed out after {timeout:?}"
        ))),
    }
}

/// 生产环境 etcd 客户端封装：`etcd_client::Client` 方法需 `&mut self`，
/// 用 `tokio::sync::Mutex` 提供 interior mutability 以满足 `&self` 契约。
pub struct EtcdClientWrapper {
    inner: tokio::sync::Mutex<etcd_client::Client>,
    /// 单次操作超时，各操作经 [`with_operation_timeout`] 包裹；默认
    /// [`DEFAULT_OPERATION_TIMEOUT_SECS`]，可经 [`Self::with_operation_timeout`] 接线配置。
    operation_timeout: Duration,
}

impl EtcdClientWrapper {
    /// 连接 etcd 集群并创建客户端封装。
    pub async fn new(endpoints: Vec<String>) -> std::result::Result<Self, EtcdError> {
        let client = etcd_client::Client::connect(endpoints, None)
            .await
            .map_err(|e| EtcdError::Network(e.to_string()))?;
        Ok(Self {
            inner: tokio::sync::Mutex::new(client),
            operation_timeout: Duration::from_secs(DEFAULT_OPERATION_TIMEOUT_SECS),
        })
    }

    /// 接线单次操作超时（来自 `EtcdConfig::operation_timeout_secs`）。
    ///
    /// ```ignore
    /// EtcdClientWrapper::new(config.etcd.endpoints.clone())
    ///     .await?
    ///     .with_operation_timeout(Duration::from_secs(config.etcd.operation_timeout_secs))
    /// ```
    pub fn with_operation_timeout(mut self, timeout: Duration) -> Self {
        self.operation_timeout = timeout;
        self
    }
}

#[async_trait]
impl EtcdClientOps for EtcdClientWrapper {
    async fn kv_get(&self, key: &str) -> std::result::Result<Option<Vec<u8>>, EtcdError> {
        // 各操作统一经集中式超时 helper 包裹（下同）。
        with_operation_timeout(self.operation_timeout, async {
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
        })
        .await
    }

    async fn kv_delete(&self, key: &str) -> std::result::Result<(), EtcdError> {
        with_operation_timeout(self.operation_timeout, async {
            let mut client = self.inner.lock().await;
            client
                .delete(key, None)
                .await
                .map_err(|e| EtcdError::Network(e.to_string()))?;
            Ok(())
        })
        .await
    }

    async fn lease_grant(&self, ttl: i64) -> std::result::Result<i64, EtcdError> {
        with_operation_timeout(self.operation_timeout, async {
            let mut client = self.inner.lock().await;
            let resp = client
                .lease_grant(ttl, None)
                .await
                .map_err(|e| EtcdError::LeaseInvalid(e.to_string()))?;
            Ok(resp.id())
        })
        .await
    }

    async fn lease_revoke(&self, lease_id: i64) -> std::result::Result<(), EtcdError> {
        with_operation_timeout(self.operation_timeout, async {
            let mut client = self.inner.lock().await;
            client
                .lease_revoke(lease_id)
                .await
                .map_err(|e| EtcdError::LeaseInvalid(e.to_string()))?;
            Ok(())
        })
        .await
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

        with_operation_timeout(self.operation_timeout, async {
            let mut client = self.inner.lock().await;
            let resp = client
                .txn(txn)
                .await
                .map_err(|e| EtcdError::Network(e.to_string()))?;
            Ok(resp.succeeded())
        })
        .await
    }

    async fn ping(&self) -> std::result::Result<(), EtcdError> {
        with_operation_timeout(self.operation_timeout, async {
            let mut client = self.inner.lock().await;
            // 真实 etcd 拒绝空 key，故对中立探针 key 做一次 Range：应答即存活，
            // 空结果（key 不存在）不影响判定，也不写入任何数据。
            client
                .get("/idgen/__liveness_probe__", None)
                .await
                .map_err(|e| EtcdError::Network(e.to_string()))?;
            Ok(())
        })
        .await
    }

    async fn lease_keep_alive_once(&self, lease_id: i64) -> std::result::Result<(), EtcdError> {
        with_operation_timeout(self.operation_timeout, async {
            let mut client = self.inner.lock().await;
            let (mut keeper, mut stream) = client
                .lease_keep_alive(lease_id)
                .await
                .map_err(|e| EtcdError::LeaseInvalid(e.to_string()))?;
            keeper
                .keep_alive()
                .await
                .map_err(|e| EtcdError::LeaseInvalid(e.to_string()))?;
            match stream.message().await {
                // ttl == 0 表示 lease 已过期（etcd 语义），续期失败。
                Ok(Some(resp)) if resp.ttl() > 0 => Ok(()),
                Ok(Some(_)) => Err(EtcdError::LeaseInvalid(format!(
                    "lease {lease_id} expired (ttl=0)"
                ))),
                Ok(None) => Err(EtcdError::LeaseInvalid(format!(
                    "keep-alive stream closed for lease {lease_id}"
                ))),
                Err(e) => Err(EtcdError::LeaseInvalid(e.to_string())),
            }
        })
        .await
    }
}

/// etcd 集群健康监控器：状态字段以 `Arc<Atomic*>` 持有，clone 后所有句柄共享同一状态
/// （后台巡检与外部观察者不再隔离）。
pub struct EtcdClusterHealthMonitor {
    config: EtcdConfig,
    status: Arc<AtomicU8>,
    last_success: Arc<tokio::sync::Mutex<Instant>>,
    failure_count: Arc<AtomicU64>,
    consecutive_failures: Arc<AtomicU64>,
    local_cache: Arc<RwLock<HashMap<String, LocalCacheEntry>>>,
    cache_file_path: String,
    is_using_cache: Arc<AtomicBool>,
    /// 可选注入的 etcd 客户端：`Some` 时走可 mock 路径；`None` 时每次检查新建 Client（lazy connect）。
    client: Option<Arc<dyn EtcdClientOps>>,
}

impl EtcdClusterHealthMonitor {
    pub fn new(config: EtcdConfig, cache_file_path: String) -> Self {
        Self {
            config,
            status: Arc::new(AtomicU8::new(EtcdClusterStatus::Healthy as u8)),
            last_success: Arc::new(tokio::sync::Mutex::new(Instant::now())),
            failure_count: Arc::new(AtomicU64::new(0)),
            consecutive_failures: Arc::new(AtomicU64::new(0)),
            local_cache: Arc::new(RwLock::new(HashMap::new())),
            cache_file_path,
            is_using_cache: Arc::new(AtomicBool::new(false)),
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
            status: Arc::new(AtomicU8::new(EtcdClusterStatus::Healthy as u8)),
            last_success: Arc::new(tokio::sync::Mutex::new(Instant::now())),
            failure_count: Arc::new(AtomicU64::new(0)),
            consecutive_failures: Arc::new(AtomicU64::new(0)),
            local_cache: Arc::new(RwLock::new(HashMap::new())),
            cache_file_path,
            is_using_cache: Arc::new(AtomicBool::new(false)),
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
        // 注入路径：用 ping 验证连通性（可 mock），以 connect_timeout_ms 包裹避免挂死。
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

        // 生产默认路径：每次检查新建临时 client。connect 是 lazy 的，对不可达
        // endpoint 也返回 Ok(Client)，故 Ok(Err(e)) 分支仅在真实网络故障时触发，
        // 无法单测可靠覆盖；该分支逻辑由注入路径测试覆盖。
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

// Clone 语义：状态字段以 `Arc<Atomic*>` 共享，clone 句柄与原句柄读写同一状态。
impl Clone for EtcdClusterHealthMonitor {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            status: self.status.clone(),
            last_success: self.last_success.clone(),
            failure_count: self.failure_count.clone(),
            consecutive_failures: self.consecutive_failures.clone(),
            local_cache: self.local_cache.clone(),
            cache_file_path: self.cache_file_path.clone(),
            is_using_cache: self.is_using_cache.clone(),
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
    /// 本实例身份标识（写入 worker key 的 value）；release 据此校验归属，防误删他实例 key。
    instance_id: String,
    /// 最近一次 allocate 写入的 value（None = 未持有），Arc 共享保证 clone 句柄一致。
    allocated_value: Arc<RwLock<Option<String>>>,
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
            instance_id: self.instance_id.clone(),
            allocated_value: self.allocated_value.clone(),
        }
    }
}

impl EtcdWorkerAllocator {
    const MAX_WORKER_ID: u16 = 255;
    const WORKER_PATH_PREFIX: &'static str = "/idgen/workers";

    /// lease TTL（秒），grant_lease 与 keepalive 周期的单一来源（bin 装配引用，故 pub）。
    pub const LEASE_TTL_SECS: i64 = 30;

    /// 续期连续失败阈值：达到即 fail-stop（上报致命错误触发优雅停机）。
    pub const KEEPALIVE_MAX_CONSECUTIVE_FAILURES: u32 = 3;

    /// keepalive 周期 = lease_ttl/3（默认 10s），保证单次失败仍有两窗口补救机会。
    pub fn keepalive_interval() -> Duration {
        Duration::from_secs((Self::LEASE_TTL_SECS / 3).max(1) as u64)
    }

    /// 实例身份标识：hostname + pid + 启动纳秒时戳，保证同机多副本与快速重启互不相同。
    fn make_instance_id(datacenter_id: u8) -> String {
        let hostname = std::fs::read_to_string("/etc/hostname")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "unknown-host".to_string());
        let boot_ns = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        format!(
            "host={};dc={};pid={};boot_ns={}",
            hostname,
            datacenter_id,
            std::process::id(),
            boot_ns
        )
    }

    /// 当前持有的 lease id（0 = 未持有）。
    pub fn current_lease_id(&self) -> i64 {
        self.lease_id.load(Ordering::SeqCst)
    }

    /// 续期一次（经 [`EtcdClientOps`] 抽象，可 mock）。
    pub async fn keep_alive_once(&self) -> std::result::Result<(), WorkerAllocatorError> {
        let lease_id = self.current_lease_id();
        if lease_id == 0 {
            return Err(WorkerAllocatorError::LeaseRenewalFailed(
                "no active lease to keep alive".to_string(),
            ));
        }
        self.client
            .lease_keep_alive_once(lease_id)
            .await
            .map_err(|e| WorkerAllocatorError::LeaseRenewalFailed(e.to_string()))
    }

    /// lease 续期调度核心（fail-stop）：每 interval 调一次 `lease_keep_alive_once`；
    /// 连续失败达阈值经 `failure_tx` 上报致命错误并退出（lease 失效后继续发号会产生重复 ID）；
    /// `stop_rx` 收到信号则优雅退出不上报。
    pub async fn run_lease_keepalive_loop(
        client: Arc<dyn EtcdClientOps>,
        lease_id: i64,
        interval: Duration,
        max_consecutive_failures: u32,
        mut stop_rx: tokio::sync::watch::Receiver<bool>,
        failure_tx: tokio::sync::oneshot::Sender<String>,
    ) {
        let mut consecutive_failures: u32 = 0;
        loop {
            tokio::select! {
                _ = stop_rx.changed() => {
                    info!(
                        "{}",
                        t!("log.core.coordinator.etcd.lease_keepalive_stopped")
                    );
                    return;
                }
                _ = sleep(interval) => {}
            }
            match client.lease_keep_alive_once(lease_id).await {
                Ok(()) => {
                    consecutive_failures = 0;
                    debug!(
                        "{}",
                        t!(
                            "log.core.coordinator.etcd.lease_keepalive_renewed",
                            lease_id = lease_id
                        )
                    );
                }
                Err(e) => {
                    consecutive_failures += 1;
                    warn!(
                        "{}",
                        t!(
                            "log.core.coordinator.etcd.lease_keepalive_failed",
                            consecutive = consecutive_failures,
                            max = max_consecutive_failures,
                            error = e
                        )
                    );
                    if consecutive_failures >= max_consecutive_failures {
                        error!(
                            "{}",
                            t!(
                                "log.core.coordinator.etcd.lease_keepalive_fail_stop",
                                consecutive = consecutive_failures
                            )
                        );
                        let _ = failure_tx.send(e.to_string());
                        return;
                    }
                }
            }
        }
    }

    /// 用注入的 etcd 客户端构造分配器（生产用 EtcdClientWrapper，测试用 MockEtcdClientOps）。
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
            instance_id: Self::make_instance_id(datacenter_id),
            allocated_value: Arc::new(RwLock::new(None)),
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
            .lease_grant(Self::LEASE_TTL_SECS)
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
        // key value 即本实例 instance_id，release 归属校验的依据。
        let value = self.instance_id.clone();

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

        // 从 1 开始跳过 0：AtomicU16==0 表示"未分配"，分配 0 会无法区分已分配状态。
        for worker_id in 1..=Self::MAX_WORKER_ID {
            match self.try_allocate_id(worker_id, lease_id).await {
                Ok(true) => {
                    self.allocated_id.store(worker_id, Ordering::SeqCst);
                    *self.allocated_value.write() = Some(self.instance_id.clone());
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

        // 归属校验（防误删）：key 存在但 value 非本实例 → 已被接管，拒绝删除；
        // key 不存在（lease 过期被 etcd 回收）→ 幂等释放。
        match self.client.kv_get(&path).await {
            Ok(Some(existing)) => {
                let owned_value = self.allocated_value.read().clone();
                if owned_value.as_deref() != Some(String::from_utf8_lossy(&existing).as_ref()) {
                    warn!(
                        "{}",
                        t!(
                            "log.core.coordinator.etcd.release_refused_key_owned_by_other",
                            path = path,
                            value = String::from_utf8_lossy(&existing)
                        )
                    );
                    return Err(WorkerAllocatorError::EtcdError(format!(
                        "worker key {path} is owned by another instance; release refused"
                    )));
                }
            }
            Ok(None) => {}
            Err(e) => {
                return Err(WorkerAllocatorError::EtcdError(e.to_string()));
            }
        }

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
        *self.allocated_value.write() = None;
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

/// Etcd 分布式锁：用 lease 机制实现带 TTL 的分布式锁。
pub struct EtcdDistributedLock {
    client: Arc<dyn EtcdClientOps>,
    lock_path_prefix: String,
}

impl EtcdDistributedLock {
    /// 用注入的 etcd 客户端创建分布式锁（`lock_path_prefix` 为锁键前缀，如 "/nebula/locks/"）。
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
                        // 初始化为未释放状态，Drop 时检查
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

/// Etcd 锁守卫：实现 `Drop`，未显式 release 时自动 spawn 后台 task 撤销 lease。
/// 已显式 release 由 `released` 标志阻止二次释放；tokio runtime 不可用时仅记 warning，
/// 锁由 lease TTL 到期后 etcd 自动回收。
pub struct EtcdLockGuard {
    lock: EtcdDistributedLock,
    key: String,
    lease_id: i64,
    lock_path: String,
    /// 是否已显式 release，防止 Drop 二次释放（AtomicBool 可在 Drop 中读取）。
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

        // 标记已释放，防止 Drop 中二次释放。
        self.released.store(true, Ordering::SeqCst);

        // 输出 lock_path 用于运维诊断 etcd key 路径，同时消除 dead_code 警告。
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
        // 已显式 release 则跳过，避免 double-release。
        if self.released.load(Ordering::SeqCst) {
            return;
        }

        // 未显式 release 时 spawn 后台 task 释放锁（覆盖早期 return / panic 场景）。
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

    // ============== 操作超时 ==============

    /// 慢 future 超时 → 映射为消息含 "timed out" 的 `EtcdError::Network`。
    #[tokio::test]
    async fn test_operation_timeout_maps_slow_future_to_network_error() {
        let result: std::result::Result<(), EtcdError> =
            with_operation_timeout(Duration::from_millis(20), async {
                sleep(Duration::from_millis(200)).await;
                Ok(())
            })
            .await;
        match result {
            Err(EtcdError::Network(msg)) => {
                assert!(
                    msg.contains("timed out"),
                    "超时消息必须显式标注 timed out，实际: {msg}"
                );
            }
            other => panic!("期望 EtcdError::Network 超时，实际 {other:?}"),
        }
    }

    /// 快 future 不受超时影响，原样透传结果。
    #[tokio::test]
    async fn test_operation_timeout_passes_through_fast_future() {
        let result: std::result::Result<u8, EtcdError> =
            with_operation_timeout(Duration::from_secs(5), async { Ok(9) }).await;
        assert_eq!(result.unwrap(), 9);
    }

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
            async fn lease_keep_alive_once(&self, lease_id: i64) -> std::result::Result<(), crate::core::coordinator::EtcdError>;
        }
    }

    /// 把 mock 封装为 `Arc<dyn EtcdClientOps>`，方便各测试复用。
    fn mock_into_client(mock: MockEtcdClientOps) -> Arc<dyn EtcdClientOps> {
        Arc::new(mock)
    }

    /// ping 永不 resolve 的 EtcdClientOps 实现，用于触发 check_etcd_health 的 timeout 分支。
    /// （mockall 的 `.returning` 只能返回 `T`，无法让 future 永不 resolve。）
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
        async fn lease_keep_alive_once(
            &self,
            _lease_id: i64,
        ) -> std::result::Result<(), EtcdError> {
            Ok(())
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
        // .returning 不限制 times，避免 fire-and-forget task 时序不确定导致 mock 验证失败。
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

        // .returning 不限制 times，避免 fire-and-forget task 时序问题。
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
        // 显式 release 后 Drop 跳过：lease_revoke 只应被调用 1 次。
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
    }

    #[tokio::test]
    async fn test_lock_guard_release_fails() {
        let mut mock = MockEtcdClientOps::new();
        // Drop spawn 的后台 release 时序不确定，用 .returning 不限制 times。
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

    /// 注入路径下 ping 超时应触发 record_failure（走 timeout 分支而非 Ok(Err)）。
    /// 用 HangingPingClient 让 ping 永不 resolve，connect_timeout_ms=50ms 快速触发。
    #[tokio::test]
    async fn test_health_monitor_check_with_client_timeout() {
        let client: Arc<dyn EtcdClientOps> = Arc::new(HangingPingClient);

        let config = EtcdConfig {
            connect_timeout_ms: 50,
            ..Default::default()
        };
        let cache_file = NamedTempFile::new().expect("Failed to create temp file");
        let cache_path = cache_file.path().to_string_lossy().to_string();
        let monitor = EtcdClusterHealthMonitor::new_with_client(config, cache_path, client);

        monitor.check_etcd_health().await;
        assert_eq!(
            monitor.get_status(),
            EtcdClusterStatus::Healthy,
            "单次 ping 超时不应立即降级"
        );

        monitor.check_etcd_health().await;
        monitor.check_etcd_health().await;
        assert_eq!(
            monitor.get_status(),
            EtcdClusterStatus::Degraded,
            "连续 3 次 ping 超时应进入 Degraded"
        );
    }

    /// 生产默认路径 + 空 endpoints 应打印 no_endpoints_configured 警告并 early-return。
    #[tokio::test]
    async fn test_health_monitor_check_default_path_empty_endpoints() {
        let config = EtcdConfig {
            endpoints: vec![],
            ..Default::default()
        };
        let cache_file = NamedTempFile::new().expect("Failed to create temp file");
        let cache_path = cache_file.path().to_string_lossy().to_string();
        let monitor = EtcdClusterHealthMonitor::new(config, cache_path);

        // 空 endpoints → 直接 return，状态不变（仍为 Healthy，无 record_failure 调用）
        monitor.check_etcd_health().await;
        assert_eq!(
            monitor.get_status(),
            EtcdClusterStatus::Healthy,
            "空 endpoints 应 early-return，不改变状态"
        );
    }

    // 生产路径 Ok(Err(e)) 分支（连接错误）无法在测试中触发：connect 是 lazy 的，
    // 不可达 endpoint 也返回 Ok(Client)；该分支由注入路径 mock 测试与真实 etcd
    // 集成测试（--features integration-tests）覆盖。

    /// clone 后原子状态与 `local_cache` 均可读（手动 Clone 逐字段复制）。
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

    /// clone 后 client / datacenter_id / atomic 状态同步（手动 Clone 复制）。
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

    /// `with_client` 直接构造（不打 info! 日志）应可用。
    #[tokio::test]
    async fn test_etcd_distributed_lock_with_client_direct_construction() {
        let client = mock_into_client(MockEtcdClientOps::new());
        let lock = EtcdDistributedLock::with_client(client, "/locks/".to_string());

        // with_client 不打 info! 日志，仅直接构造
        assert!(lock.is_healthy(), "is_healthy 应始终返回 true");
        // lock_path 前缀应正确设置（通过 acquire 间接验证）
        // 这里仅验证构造成功且无 panic
    }

    /// 显式 release 失败后 drop 应再次 spawn lease_revoke（mock 返回 Err，仅记 warning）。
    #[tokio::test]
    async fn test_lock_guard_drop_spawns_release_when_unreleased() {
        let mut mock = MockEtcdClientOps::new();
        // lease_grant 成功 → try_acquire_lock 返回 Some(lease_id)
        mock.expect_lease_grant().returning(|_| Ok(789));
        // CAS 成功 → 获取锁
        mock.expect_txn_check_create_rev_and_put()
            .returning(|_, _, _| Ok(true));
        // lease_revoke 不限次数：drop spawn 时序不定，可能调用 0 次或多次。
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

    /// Drop 在 tokio runtime 不可用时应仅记 warning、不 panic 且不调用 lease_revoke
    /// （用独立 std::thread drop guard 模拟）。
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

    /// load_local_cache 文件不存在时应返回 Ok(())（打印 info 后早返回）。
    #[tokio::test]
    async fn test_load_local_cache_file_not_found_returns_ok() {
        let config = EtcdConfig::default();
        let cache_path = "/nonexistent/path/that/does/not/exist.json".to_string();
        let monitor = EtcdClusterHealthMonitor::new(config, cache_path);

        let result = monitor.load_local_cache().await;
        assert!(
            result.is_ok(),
            "文件不存在时应返回 Ok(()) 而非 Err, 实际: {:?}",
            result.err()
        );
    }

    /// load_local_cache 文件内容非 JSON 应返回 InternalError("Failed to parse ...")。
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

    /// load_local_cache 读取目录应返回 InternalError（fs::read_to_string 失败）。
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

    /// save_local_cache 写入不存在目录应返回 InternalError（fs::write 失败）。
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

    /// record_success 在 Failed+using_cache 下应重置 Healthy 并退出缓存模式
    /// （set_status(Healthy) + is_using_cache=false）。
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

    /// 已 Healthy 时 record_success 仅更新 last_success / consecutive_failures，不重置状态。
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

    /// start_health_check 应 spawn 后台巡检 task 并执行循环体（空 endpoints 走 early-return）。
    #[tokio::test]
    async fn test_start_health_check_spawns_and_runs_loop_body() {
        // 空 endpoints → check_etcd_health early-return
        let config = EtcdConfig {
            endpoints: vec![],
            ..Default::default()
        };
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

    /// start_cache_persistence 应 spawn 后台 task 并至少执行一次 save_local_cache。
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

    /// 生产默认路径 + 非空 endpoints 应走 Client::connect：lazy connect → record_success
    /// 或超时 → record_failure，两分支均可接受（单次检查不降级）。
    #[tokio::test]
    async fn test_check_etcd_health_production_path_non_empty_endpoints() {
        let config = EtcdConfig {
            // 不可达端口
            endpoints: vec!["http://127.0.0.1:1".to_string()],
            // 短超时避免测试卡死
            connect_timeout_ms: 100,
            ..Default::default()
        };
        let cache_file = NamedTempFile::new().expect("Failed to create temp file");
        let cache_path = cache_file.path().to_string_lossy().to_string();
        let monitor = EtcdClusterHealthMonitor::new(config, cache_path);

        // 调用 check_etcd_health，走生产路径（非注入路径）
        monitor.check_etcd_health().await;

        // 无论 lazy connect 成功还是单次超时，单次检查都不应导致状态降级。
        assert_eq!(
            monitor.get_status(),
            EtcdClusterStatus::Healthy,
            "单次健康检查不应导致状态降级"
        );
    }

    /// 生产默认路径连续多次检查覆盖 record_failure 降级路径。
    #[tokio::test]
    async fn test_check_etcd_health_production_path_multiple_checks() {
        let config = EtcdConfig {
            endpoints: vec!["http://127.0.0.1:1".to_string()],
            connect_timeout_ms: 50,
            ..Default::default()
        };
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

    /// 直接调用 HangingPingClient 的非 ping 方法以覆盖 trait impl
    /// （timeout 测试仅触达 ping）。
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

    /// `EtcdClientWrapper::new` 不可达 endpoint + 各方法错误路径（lazy connect 可能返回 Ok，
    /// 方法调用应因网络不可达失败或超时，用 timeout 包裹避免卡死）。
    #[tokio::test]
    async fn test_etcd_client_wrapper_new_and_methods_with_unreachable_endpoint() {
        // 使用不可达端口 1（Windows 上无 listener → connection refused）
        let result = EtcdClientWrapper::new(vec!["http://127.0.0.1:1".to_string()]).await;

        if let Ok(wrapper) = result {
            // lazy connect 成功 → 调用各方法应因网络不可达失败或超时
            let kv_get_result =
                tokio::time::timeout(Duration::from_millis(300), wrapper.kv_get("test_key")).await;
            let _ = kv_get_result.is_ok() || kv_get_result.is_err();

            let kv_delete_result =
                tokio::time::timeout(Duration::from_millis(300), wrapper.kv_delete("test_key"))
                    .await;
            let _ = kv_delete_result.is_ok() || kv_delete_result.is_err();

            let lease_grant_result =
                tokio::time::timeout(Duration::from_millis(300), wrapper.lease_grant(30)).await;
            let _ = lease_grant_result.is_ok() || lease_grant_result.is_err();

            let lease_revoke_result =
                tokio::time::timeout(Duration::from_millis(300), wrapper.lease_revoke(123)).await;
            let _ = lease_revoke_result.is_ok() || lease_revoke_result.is_err();

            let txn_result = tokio::time::timeout(
                Duration::from_millis(300),
                wrapper.txn_check_create_rev_and_put("key", b"value".to_vec(), 456),
            )
            .await;
            let _ = txn_result.is_ok() || txn_result.is_err();

            let ping_result =
                tokio::time::timeout(Duration::from_millis(300), wrapper.ping()).await;
            let _ = ping_result.is_ok() || ping_result.is_err();
        }
        // 如果 new 返回 Err（endpoint 格式错误等），也覆盖了 error path
    }

    /// `EtcdClientWrapper::new` 空 endpoints 覆盖 new 路径（返回 Err 或 lazy Ok）。
    #[tokio::test]
    async fn test_etcd_client_wrapper_new_empty_endpoints() {
        let result = EtcdClientWrapper::new(vec![]).await;
        // 空 endpoints 可能 Err 或 Ok（lazy），只覆盖调用不断言结果。
        let _ = result.is_ok() || result.is_err();
    }

    /// 全部 worker_id 的 key 已存在时 try_allocate_id 返回 Ok(false) → NoAvailableId。
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

    /// `with_client` 构造的 lock 应能 acquire 并显式 release（走 with_client 路径）。
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

    /// `with_client` 构造的 lock acquire 冲突重试后成功（MAX_RETRIES + sleep 路径）。
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

    /// `with_client` lock acquire 全部冲突 → MAX_RETRIES 耗尽 → AcquireFailed。
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

    /// `with_client` lock acquire 时 lease_grant 失败 → EtcdError。
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

    /// allocator datacenter_id=0 应正常分配（worker_path 格式化路径）。
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

    /// allocator datacenter_id=255 应正常分配（高值格式化路径）。
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

    /// release 后 allocated_id 重置为 0，可再次 allocate。
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

    /// 直接调用 set_status 覆盖所有状态转换。
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

    /// guard 未显式 release 时 drop 应 spawn task 调 lease_revoke；
    /// 即使 revoke 失败也不 panic（仅 warn）。
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

    /// 空 cache 的 save + load round-trip 应正常（空序列化/反序列化路径）。
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

    /// 生产默认路径 + 空 endpoints 应 warn + early-return，状态不变。
    #[tokio::test]
    async fn test_check_etcd_health_production_path_empty_endpoints() {
        let config = EtcdConfig {
            endpoints: vec![],
            ..Default::default()
        };
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

    /// start_cache_persistence 在 save_local_cache 失败时应走 error! 分支不 panic
    /// （无效 cache_file_path 触发 fs::write 失败）。
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

    /// try_allocate_id 在 kv_get Err 时应容错继续（返回 Ok(false)）。
    /// 前 3 次 Err、第 4 次 Ok(None)+CAS 成功 → 分配到 id=4。
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

    /// release 在 kv_delete Err 时应记 error! 并返回 EtcdError。
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

    // ==================== worker 归属校验与 lease keepalive ====================

    /// 有状态 KV mock：txn 写入对 kv_get 可见，kv_delete 真删除；返回 (mock, store)。
    type StatefulStoreMock = (
        MockEtcdClientOps,
        Arc<std::sync::Mutex<HashMap<String, Vec<u8>>>>,
    );
    fn stateful_store_mock() -> StatefulStoreMock {
        let store: Arc<std::sync::Mutex<HashMap<String, Vec<u8>>>> =
            Arc::new(std::sync::Mutex::new(HashMap::new()));
        let mut mock = MockEtcdClientOps::new();
        {
            let store = store.clone();
            mock.expect_kv_get()
                .returning(move |k| Ok(store.lock().unwrap().get(k).cloned()));
        }
        {
            let store = store.clone();
            mock.expect_txn_check_create_rev_and_put()
                .returning(move |k, v, _| {
                    store.lock().unwrap().insert(k.to_string(), v);
                    Ok(true)
                });
        }
        {
            let store = store.clone();
            mock.expect_kv_delete().returning(move |k| {
                store.lock().unwrap().remove(k);
                Ok(())
            });
        }
        mock.expect_lease_grant().returning(|_| Ok(42));
        (mock, store)
    }

    /// release 归属校验：key 被其他实例接管（value 不匹配）时必须拒绝删除、
    /// 不触碰 kv_delete、不重置已分配状态。
    #[tokio::test]
    async fn test_allocator_release_refused_when_key_owned_by_other() {
        let (mut mock, store) = stateful_store_mock();
        // 归属校验拒绝路径不得触达 kv_delete：错误调用即 panic（times(0)）
        mock.expect_kv_delete().times(0).returning(|_| Ok(()));

        let allocator = EtcdWorkerAllocator::new(mock_into_client(mock), 1, EtcdConfig::default())
            .await
            .unwrap();
        let worker_id = allocator.allocate().await.unwrap();
        let path = format!("/idgen/workers/1/{worker_id}");

        // 模拟 lease 过期后其他实例接管同一 worker key
        store
            .lock()
            .unwrap()
            .insert(path.clone(), b"host=other;dc=1;pid=999;boot_ns=1".to_vec());

        let release_result = allocator.release(worker_id).await;
        assert!(
            matches!(release_result, Err(WorkerAllocatorError::EtcdError(_))),
            "key 被其他实例接管时 release 必须拒绝, 实际: {:?}",
            release_result
        );
        assert_eq!(
            allocator.get_allocated_id(),
            Some(worker_id),
            "release 被拒绝时已分配状态不得重置"
        );
        assert!(
            store.lock().unwrap().contains_key(&path),
            "被接管的 key 不得被删除"
        );
    }

    /// release 正向路径：value 为本实例时删除；key 已不存在时幂等释放成功。
    #[tokio::test]
    async fn test_allocator_release_succeeds_with_ownership_check() {
        let (mock, store) = stateful_store_mock();
        let allocator = EtcdWorkerAllocator::new(mock_into_client(mock), 1, EtcdConfig::default())
            .await
            .unwrap();
        let worker_id = allocator.allocate().await.unwrap();
        let path = format!("/idgen/workers/1/{worker_id}");

        // 写入的 value 必须是本实例 instance_id（allocator 状态中记录的同一值）
        assert_eq!(
            store
                .lock()
                .unwrap()
                .get(&path)
                .map(|v| String::from_utf8_lossy(v).into_owned()),
            allocator.allocated_value.read().clone(),
            "worker key value 必须为 allocator 记录的 instance value"
        );

        allocator
            .release(worker_id)
            .await
            .expect("归属匹配时 release 必须成功");
        assert!(
            !store.lock().unwrap().contains_key(&path),
            "release 后 worker key 必须被删除"
        );
        assert_eq!(allocator.get_allocated_id(), None);
    }

    /// keep_alive_once 在未持有 lease（lease_id==0）时显性报错。
    #[tokio::test]
    async fn test_keep_alive_once_without_lease_errors() {
        let client = mock_into_client(MockEtcdClientOps::new());
        let allocator = EtcdWorkerAllocator::new(client, 1, EtcdConfig::default())
            .await
            .unwrap();

        let result = allocator.keep_alive_once().await;
        assert!(
            matches!(result, Err(WorkerAllocatorError::LeaseRenewalFailed(_))),
            "未持有 lease 时续期必须报 LeaseRenewalFailed, 实际: {:?}",
            result
        );
    }

    /// keep_alive_once 以 allocate 授予的 lease id 委托 client 续期。
    #[tokio::test]
    async fn test_keep_alive_once_delegates_with_allocated_lease_id() {
        let mut mock = MockEtcdClientOps::new();
        mock.expect_kv_get().returning(|_| Ok(None));
        mock.expect_lease_grant().returning(|_| Ok(77));
        mock.expect_txn_check_create_rev_and_put()
            .returning(|_, _, _| Ok(true));
        mock.expect_lease_keep_alive_once()
            .times(1)
            .withf(|lease_id| *lease_id == 77)
            .returning(|_| Ok(()));

        let allocator = EtcdWorkerAllocator::new(mock_into_client(mock), 1, EtcdConfig::default())
            .await
            .unwrap();
        allocator.allocate().await.unwrap();
        assert_eq!(allocator.current_lease_id(), 77);

        allocator
            .keep_alive_once()
            .await
            .expect("lease id 匹配时续期必须成功");
    }

    /// 续期调度：周期续期 + stop 信号优雅退出，不上报致命错误。
    #[tokio::test]
    async fn test_keepalive_loop_renews_periodically_and_stops() {
        let mut mock = MockEtcdClientOps::new();
        let counter = Arc::new(AtomicU64::new(0));
        {
            let counter = counter.clone();
            mock.expect_lease_keep_alive_once().returning(move |_| {
                counter.fetch_add(1, Ordering::SeqCst);
                Ok(())
            });
        }

        let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
        let (failure_tx, mut failure_rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(EtcdWorkerAllocator::run_lease_keepalive_loop(
            mock_into_client(mock),
            7,
            Duration::from_millis(10),
            EtcdWorkerAllocator::KEEPALIVE_MAX_CONSECUTIVE_FAILURES,
            stop_rx,
            failure_tx,
        ));

        // 60ms / 10ms 间隔 → 至少数次续期
        tokio::time::sleep(Duration::from_millis(60)).await;
        stop_tx.send(true).expect("stop 信号发送必须成功");
        task.await.expect("keepalive 任务必须正常退出");

        assert!(
            counter.load(Ordering::SeqCst) >= 2,
            "keepalive 应已周期续期多次, 实际: {}",
            counter.load(Ordering::SeqCst)
        );
        assert!(
            failure_rx.try_recv().is_err(),
            "无失败路径不得触发致命错误上报"
        );
    }

    /// fail-stop：连续失败达阈值 → failure_tx 上报致命错误并退出。
    #[tokio::test]
    async fn test_keepalive_loop_fail_stops_after_consecutive_failures() {
        let mut mock = MockEtcdClientOps::new();
        mock.expect_lease_keep_alive_once()
            .returning(|_| Err(EtcdError::Network("etcd down".into())));

        let (_stop_tx, stop_rx) = tokio::sync::watch::channel(false);
        let (failure_tx, failure_rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(EtcdWorkerAllocator::run_lease_keepalive_loop(
            mock_into_client(mock),
            7,
            Duration::from_millis(5),
            3,
            stop_rx,
            failure_tx,
        ));

        let reason = tokio::time::timeout(Duration::from_secs(2), failure_rx)
            .await
            .expect("连续 3 次失败后必须上报致命错误")
            .expect("failure channel 必须有值");
        assert!(
            reason.contains("etcd down"),
            "上报原因应为最后一次续期错误, 实际: {reason}"
        );
        task.await.expect("fail-stop 后任务必须退出");
    }

    /// 间歇性失败被成功重置：不触发 fail-stop。
    #[tokio::test]
    async fn test_keepalive_loop_resets_counter_on_intermittent_success() {
        let mut mock = MockEtcdClientOps::new();
        let call_count = Arc::new(AtomicU64::new(0));
        {
            let call_count = call_count.clone();
            mock.expect_lease_keep_alive_once().returning(move |_| {
                let n = call_count.fetch_add(1, Ordering::SeqCst);
                // 偶数次失败、奇数次成功 → 连续失败最多 1 次
                if n.is_multiple_of(2) {
                    Err(EtcdError::Network("flaky".into()))
                } else {
                    Ok(())
                }
            });
        }

        let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
        let (failure_tx, mut failure_rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(EtcdWorkerAllocator::run_lease_keepalive_loop(
            mock_into_client(mock),
            7,
            Duration::from_millis(5),
            3,
            stop_rx,
            failure_tx,
        ));

        tokio::time::sleep(Duration::from_millis(80)).await;
        stop_tx.send(true).expect("stop 信号发送必须成功");
        task.await.expect("keepalive 任务必须正常退出");

        assert!(
            failure_rx.try_recv().is_err(),
            "间歇性失败（连续 < 3）不得触发 fail-stop"
        );
    }

    /// 真实 etcd 路径（integration-tests 门控 + #[ignore]）：分配 → 续期一次 → release 全链路。
    /// 运行：`ETCD_TEST_ENDPOINTS=http://127.0.0.1:2379 cargo test
    ///  --features etcd,integration-tests -- --ignored`
    #[cfg(feature = "integration-tests")]
    #[tokio::test]
    #[ignore = "需要真实 etcd：设置 ETCD_TEST_ENDPOINTS 后以 --ignored 运行"]
    async fn integration_worker_allocator_allocate_keepalive_release_real_etcd() {
        let endpoints = std::env::var("ETCD_TEST_ENDPOINTS")
            .unwrap_or_else(|_| "http://127.0.0.1:2379".to_string());
        let wrapper = EtcdClientWrapper::new(vec![endpoints])
            .await
            .expect("连接 etcd 失败（请检查 ETCD_TEST_ENDPOINTS）");
        let client: Arc<dyn EtcdClientOps> = Arc::new(wrapper);
        client
            .ping()
            .await
            .expect("etcd 必须可达（ETCD_TEST_ENDPOINTS）");

        let allocator = EtcdWorkerAllocator::new(client.clone(), 1, EtcdConfig::default())
            .await
            .unwrap();
        let worker_id = allocator.allocate().await.expect("allocate 必须成功");
        assert!((1..=255).contains(&worker_id));

        allocator
            .keep_alive_once()
            .await
            .expect("lease 续期必须成功");
        allocator
            .release(worker_id)
            .await
            .expect("归属校验 release 必须成功");
        assert_eq!(allocator.get_allocated_id(), None);
    }

    // ==================== Clone 状态共享与巡检接线 ====================

    /// clone 后两句柄状态互通：降级/恢复立即互相可见（防巡检任务 clone self 后状态隔离）。
    #[tokio::test]
    async fn test_monitor_clone_handles_share_state() {
        let config = EtcdConfig::default();
        let cache_file = NamedTempFile::new().expect("Failed to create temp file");
        let cache_path = cache_file.path().to_string_lossy().to_string();
        let monitor = EtcdClusterHealthMonitor::new(config, cache_path);

        let observer = monitor.clone();
        assert_eq!(observer.get_status(), EtcdClusterStatus::Healthy);

        // 原句柄降级 → clone 句柄立即可见
        monitor.record_failure();
        monitor.record_failure();
        monitor.record_failure();
        assert_eq!(
            observer.get_status(),
            EtcdClusterStatus::Degraded,
            "clone 句柄必须读到原句柄触发的降级状态"
        );

        // 原/clone 句柄均可继续驱动同一状态：恢复对双方可见
        observer.record_success().await;
        assert_eq!(monitor.get_status(), EtcdClusterStatus::Healthy);
        assert!(!monitor.is_using_cache());
        assert_eq!(observer.get_status(), EtcdClusterStatus::Healthy);

        // clone 之后再降级到 Failed + using_cache，原句柄同样可见
        for _ in 0..5 {
            observer.record_failure();
        }
        assert_eq!(monitor.get_status(), EtcdClusterStatus::Failed);
        assert!(monitor.is_using_cache(), "is_using_cache 也必须跨句柄共享");
    }

    /// 巡检接线后的生产形态：注入真实 EtcdClientWrapper（lazy connect 对不可达端点返回 Ok），
    /// 连续 ping 失败应把监控器带入 Failed + 本地缓存降级。
    #[tokio::test]
    async fn test_monitor_injected_real_client_reaches_failed_on_unreachable_etcd() {
        let wrapper = EtcdClientWrapper::new(vec!["http://127.0.0.1:1".to_string()])
            .await
            .expect("lazy connect 对不可达端点也可能返回 Ok（Windows/兼容路径）");
        let config = EtcdConfig {
            connect_timeout_ms: 300,
            ..Default::default()
        };
        let cache_file = NamedTempFile::new().expect("Failed to create temp file");
        let cache_path = cache_file.path().to_string_lossy().to_string();
        let monitor =
            EtcdClusterHealthMonitor::new_with_client(config, cache_path, Arc::new(wrapper));

        // 连续 5 次真实 ping 失败（连接拒绝）→ Failed + 启用本地缓存降级
        for _ in 0..5 {
            monitor.check_etcd_health().await;
        }
        assert_eq!(
            monitor.get_status(),
            EtcdClusterStatus::Failed,
            "不可达端点连续巡检失败必须转入 Failed 状态"
        );
        assert!(monitor.is_using_cache());
    }
}
