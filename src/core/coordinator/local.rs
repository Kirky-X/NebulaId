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

//! Local fallback implementations for the coordinator module.
//!
//! These types are compiled when the `etcd` feature is disabled. They
//! provide stub/in-process behavior so the rest of the system can run
//! without an external etcd cluster (rule 25: implementations live in
//! sub-modules; mod.rs only declares traits + re-exports).

// LocalCacheEntry is shared with the etcd sub-module via `super::LocalCacheEntry`,
// so it must compile under both feature flags. All other items below are
// `#[cfg(not(feature = "etcd"))]` and their imports follow the same gate.
//
// 例外：`LocalDistributedLock` / `LocalLockGuard` 也在两种 feature 下编译，
// 因为 etcd 模式下 `main.rs` 需要 `LocalDistributedLock` 作为 etcd 失败时的
// 进程内 fallback（避免单点故障导致服务完全不可用）。
use serde::{Deserialize, Serialize};

// Items needed only by the no-etcd stub implementations.
#[cfg(not(feature = "etcd"))]
use super::{EtcdClusterStatus, WorkerAllocatorError, WorkerIdAllocator};
// LocalDistributedLock / LocalLockGuard 在两种 feature 下都需要，故无条件导入。
use super::{DistributedLock, LockError, LockGuard};
// LocalDistributedLock 的 impl 需要 async_trait，无条件导入。
use async_trait::async_trait;
#[cfg(not(feature = "etcd"))]
use std::sync::atomic::{AtomicU16, Ordering};
// LocalDistributedLock 需要 Arc，无条件导入。
use std::sync::Arc;
#[cfg(not(feature = "etcd"))]
use tracing::info;

/// 本地缓存条目（同时被 etcd 实现复用，故在 mod.rs re-export）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalCacheEntry {
    pub key: String,
    pub value: String,
    pub version: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

/// 本地 Worker ID 分配器（无 etcd 时使用）。
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
            "{}",
            t!(
                "log.core.coordinator.local.allocator_initialized",
                datacenter_id = datacenter_id,
                worker_id = default_worker_id
            )
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
            "{}",
            t!(
                "log.core.coordinator.local.worker_allocated",
                worker_id = id,
                datacenter_id = self.datacenter_id
            )
        );
        Ok(id)
    }

    async fn release(&self, worker_id: u16) -> std::result::Result<(), WorkerAllocatorError> {
        info!(
            "{}",
            t!(
                "log.core.coordinator.local.worker_released",
                worker_id = worker_id
            )
        );
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

/// Placeholder type when etcd feature is disabled.
#[cfg(not(feature = "etcd"))]
#[derive(Clone)]
pub struct EtcdClusterHealthMonitor;

#[cfg(not(feature = "etcd"))]
impl EtcdClusterHealthMonitor {
    pub fn new(_config: crate::core::config::EtcdConfig, _cache_file_path: String) -> Self {
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

    pub async fn load_local_cache(&self) -> crate::core::types::Result<()> {
        Ok(())
    }

    pub async fn save_local_cache(&self) -> crate::core::types::Result<()> {
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

/// 本地分布式锁实现（无 etcd 时使用）。
/// 注意：这是一个降级实现，只能在单机环境工作。
///
/// 在 etcd feature 下也编译：当 `EtcdClientWrapper` 或 `EtcdDistributedLock`
/// 创建失败时，`main.rs` 用它作为 fallback，避免服务完全不可用。
#[derive(Clone)]
pub struct LocalDistributedLock {
    locks: Arc<parking_lot::Mutex<std::collections::HashMap<String, bool>>>,
}

impl LocalDistributedLock {
    pub fn new() -> Self {
        Self {
            locks: Arc::new(parking_lot::Mutex::new(std::collections::HashMap::new())),
        }
    }

    fn is_locked(&self, key: &str) -> bool {
        let locks = self.locks.lock();
        locks.get(key).copied().unwrap_or(false)
    }

    fn acquire_lock(&self, key: &str) {
        let mut locks = self.locks.lock();
        locks.insert(key.to_string(), true);
    }

    fn release_lock(&self, key: &str) {
        let mut locks = self.locks.lock();
        locks.remove(key);
    }
}

impl Default for LocalDistributedLock {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl DistributedLock for LocalDistributedLock {
    async fn acquire(
        &self,
        key: &str,
        _ttl_seconds: u64,
    ) -> std::result::Result<Box<dyn LockGuard>, LockError> {
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
        true
    }
}

/// 本地锁守卫实现。
pub struct LocalLockGuard {
    lock: LocalDistributedLock,
    key: String,
}

#[async_trait]
impl LockGuard for LocalLockGuard {
    async fn release(&self) -> std::result::Result<(), LockError> {
        self.lock.release_lock(&self.key);
        Ok(())
    }
}

// ============================================================================
// Tests
// ============================================================================
// LocalDistributedLock / LocalLockGuard / LocalCacheEntry 在两种 feature 下都
// 编译，因此其单元测试放在 `#[cfg(test)]` 模块中，确保 `--features etcd` 下
// 也能跑（提高覆盖率）。LocalWorkerAllocator / EtcdClusterHealthMonitor stub
// 仅在 `not(feature = "etcd")` 下编译，其测试沿用原有 cfg gate。

#[cfg(test)]
mod tests {
    use super::{DistributedLock, LocalCacheEntry, LocalDistributedLock, LockError};

    /// `LocalDistributedLock::new()` 返回一个健康的锁实例。
    #[tokio::test]
    async fn test_local_distributed_lock_acquire_release() {
        let lock = LocalDistributedLock::new();
        assert!(lock.is_healthy());

        let guard = lock.acquire("test-key", 30).await.unwrap();
        guard.release().await.unwrap();
    }

    /// 重复 acquire 同一 key 必须失败，返回 `LockError::AcquireFailed`。
    /// 验证本地互斥语义。
    #[tokio::test]
    async fn test_local_distributed_lock_double_acquire_fails() {
        let lock = LocalDistributedLock::new();
        let _guard = lock.acquire("shared-key", 30).await.unwrap();

        let result = lock.acquire("shared-key", 30).await;
        assert!(result.is_err(), "second acquire must fail");
        // `Box<dyn LockGuard>` doesn't implement `Debug`, so we cannot use
        // `unwrap_err()`. Use `if let` to destructure the error variant
        // without requiring `Debug` on the `Ok` type.
        if let Err(LockError::AcquireFailed { key, reason }) = result {
            assert_eq!(key, "shared-key");
            assert!(reason.contains("Lock already held"));
        } else {
            panic!("Expected AcquireFailed, got a different variant");
        }
    }

    /// 释放后可以再次 acquire 同一 key。
    #[tokio::test]
    async fn test_local_distributed_lock_reacquire_after_release() {
        let lock = LocalDistributedLock::new();
        let guard = lock.acquire("cycle-key", 30).await.unwrap();
        guard.release().await.unwrap();

        let guard2 = lock.acquire("cycle-key", 30).await;
        assert!(guard2.is_ok());
        guard2.unwrap().release().await.unwrap();
    }

    /// 不同 key 之间互不影响。
    #[tokio::test]
    async fn test_local_distributed_lock_independent_keys() {
        let lock = LocalDistributedLock::new();
        let guard_a = lock.acquire("key-a", 30).await.unwrap();
        let guard_b = lock.acquire("key-b", 30).await.unwrap();
        guard_a.release().await.unwrap();
        guard_b.release().await.unwrap();
    }

    /// `Default::default()` 等价于 `new()`。
    #[test]
    fn test_local_distributed_lock_default() {
        let lock = LocalDistributedLock::default();
        assert!(lock.is_healthy());
    }

    /// `Clone` 必须共享底层状态（Arc<Mutex<...>>）。
    /// 克隆的实例必须能观察到原实例已持有的锁。
    #[tokio::test]
    async fn test_local_distributed_lock_clone_shares_state() {
        let lock = LocalDistributedLock::new();
        let lock_clone = lock.clone();

        let _guard = lock.acquire("clone-key", 30).await.unwrap();
        // 克隆必须观察到锁已被持有 — 共享 Arc 状态。
        let result = lock_clone.acquire("clone-key", 30).await;
        assert!(result.is_err(), "clone must observe shared lock state");
        // Avoid `unwrap_err()` — `Box<dyn LockGuard>` does not implement Debug.
        assert!(
            matches!(result, Err(LockError::AcquireFailed { .. })),
            "Expected AcquireFailed"
        );
    }

    /// `is_healthy()` 恒为 `true`（本地降级实现总是"健康"）。
    #[test]
    fn test_local_distributed_lock_is_healthy_always_true() {
        let lock = LocalDistributedLock::new();
        assert!(lock.is_healthy());

        // 即使持有锁，is_healthy 仍为 true。
        let lock2 = LocalDistributedLock::new();
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let _guard = lock2.acquire("h", 1).await.unwrap();
            assert!(lock2.is_healthy());
        });
    }

    /// `LocalCacheEntry` 的 Clone + Serialize/Deserialize round-trip。
    #[test]
    fn test_local_cache_entry_clone_serialize() {
        let entry = LocalCacheEntry {
            key: "k".to_string(),
            value: "v".to_string(),
            version: 3,
            created_at: 100,
            updated_at: 200,
        };
        let cloned = entry.clone();
        assert_eq!(cloned.key, entry.key);
        assert_eq!(cloned.value, entry.value);
        assert_eq!(cloned.version, entry.version);
        assert_eq!(cloned.created_at, entry.created_at);
        assert_eq!(cloned.updated_at, entry.updated_at);

        // JSON round-trip
        let json = serde_json::to_string(&entry).unwrap();
        let deserialized: LocalCacheEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.key, entry.key);
        assert_eq!(deserialized.version, entry.version);
        assert_eq!(deserialized.created_at, entry.created_at);
    }

    /// `LocalCacheEntry` 字段可以使用任意字符串与 i64 值。
    #[test]
    fn test_local_cache_entry_arbitrary_values() {
        let entry = LocalCacheEntry {
            key: "".to_string(),
            value: "value with spaces & symbols!".to_string(),
            version: i64::MAX,
            created_at: 0,
            updated_at: i64::MIN,
        };
        let json = serde_json::to_string(&entry).unwrap();
        let deserialized: LocalCacheEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.key, "");
        assert_eq!(deserialized.version, i64::MAX);
        assert_eq!(deserialized.updated_at, i64::MIN);
    }
}

#[cfg(all(test, not(feature = "etcd")))]
mod no_etcd_tests {
    use super::{
        DistributedLock, EtcdClusterHealthMonitor, EtcdClusterStatus, LocalDistributedLock,
        LocalWorkerAllocator, WorkerIdAllocator,
    };
    use crate::core::config::EtcdConfig;
    use std::time::Duration;

    // ===== LocalWorkerAllocator =====

    #[tokio::test]
    async fn test_local_worker_allocator() {
        let allocator = LocalWorkerAllocator::new(0, 1);
        assert_eq!(allocator.get_allocated_id(), Some(1));
        assert!(allocator.is_healthy());

        let id = allocator.allocate().await.unwrap();
        assert_eq!(id, 1);

        allocator.release(1).await.unwrap();
        assert_eq!(allocator.get_allocated_id(), Some(1));
    }

    /// `default_worker_id = 0` 时 `get_allocated_id` 返回 `None`
    /// （0 是"未初始化"哨兵值）。
    #[tokio::test]
    async fn test_local_worker_allocator_zero_id_returns_none() {
        let allocator = LocalWorkerAllocator::new(1, 0);
        assert_eq!(allocator.get_allocated_id(), None);
        assert!(allocator.is_healthy());

        // allocate 仍返回 0（配置值），但 get_allocated_id 视 0 为未初始化。
        let id = allocator.allocate().await.unwrap();
        assert_eq!(id, 0);
        // 仍为 None（0 被视为未初始化）。
        assert_eq!(allocator.get_allocated_id(), None);
    }

    /// `release` 一个从未分配的 id 应当是 no-op，不报错。
    #[tokio::test]
    async fn test_local_worker_allocator_release_unallocated() {
        let allocator = LocalWorkerAllocator::new(0, 1);
        // Releasing an id we never allocated — should not error.
        allocator.release(999).await.unwrap();
        // 原分配 id 不受影响。
        assert_eq!(allocator.get_allocated_id(), Some(1));
    }

    /// `allocate()` 多次调用返回相同 id（stateless allocator）。
    #[tokio::test]
    async fn test_local_worker_allocator_allocate_idempotent() {
        let allocator = LocalWorkerAllocator::new(2, 42);
        let id1 = allocator.allocate().await.unwrap();
        let id2 = allocator.allocate().await.unwrap();
        let id3 = allocator.allocate().await.unwrap();
        assert_eq!(id1, 42);
        assert_eq!(id2, 42);
        assert_eq!(id3, 42);
    }

    /// `Clone` 后两个 allocator 共享同一 worker_id（Arc<AtomicU16>）。
    #[tokio::test]
    async fn test_local_worker_allocator_clone_shares_state() {
        let allocator = LocalWorkerAllocator::new(0, 7);
        let cloned = allocator.clone();
        assert_eq!(cloned.get_allocated_id(), Some(7));

        // 两者返回相同的 id。
        let id1 = allocator.allocate().await.unwrap();
        let id2 = cloned.allocate().await.unwrap();
        assert_eq!(id1, id2);
        assert_eq!(id1, 7);
    }

    /// `is_healthy()` 恒为 `true`（本地降级实现总是"健康"）。
    #[test]
    fn test_local_worker_allocator_is_healthy_always_true() {
        let allocator = LocalWorkerAllocator::new(0, 1);
        assert!(allocator.is_healthy());
    }

    // ===== EtcdClusterHealthMonitor (no-etcd stub) =====

    /// no-etcd stub 总是返回 `Failed` 状态和 `is_using_cache = true`
    /// （降级模式）。
    #[tokio::test]
    async fn test_etcd_cluster_health_monitor_stub_status() {
        let config = EtcdConfig::default();
        let monitor = EtcdClusterHealthMonitor::new(config, "cache.json".to_string());

        assert_eq!(monitor.get_status(), EtcdClusterStatus::Failed);
        assert!(monitor.is_using_cache());
    }

    /// `set_status` 在 stub 中是 no-op — 调用 `set_status(Healthy)` 后
    /// `get_status` 仍返回 `Failed`。
    #[test]
    fn test_etcd_cluster_health_monitor_stub_set_status_noop() {
        let config = EtcdConfig::default();
        let monitor = EtcdClusterHealthMonitor::new(config, "cache.json".to_string());
        monitor.set_status(EtcdClusterStatus::Healthy);
        assert_eq!(monitor.get_status(), EtcdClusterStatus::Failed);

        monitor.set_status(EtcdClusterStatus::Degraded);
        assert_eq!(monitor.get_status(), EtcdClusterStatus::Failed);
    }

    /// `record_success` / `record_failure` 是 no-op，不应 panic。
    #[tokio::test]
    async fn test_etcd_cluster_health_monitor_stub_record_noops() {
        let config = EtcdConfig::default();
        let monitor = EtcdClusterHealthMonitor::new(config, "cache.json".to_string());
        monitor.record_success().await;
        monitor.record_failure();
        // 多次调用也应当稳定。
        monitor.record_success().await;
        monitor.record_failure();
        monitor.record_failure();
    }

    /// `load_local_cache` / `save_local_cache` 返回 `Ok(())`（stub）。
    #[tokio::test]
    async fn test_etcd_cluster_health_monitor_stub_cache_io() {
        let config = EtcdConfig::default();
        let monitor = EtcdClusterHealthMonitor::new(config, "cache.json".to_string());
        monitor.load_local_cache().await.unwrap();
        monitor.save_local_cache().await.unwrap();
    }

    /// `get_from_cache` 恒返回 `None`；`put_to_cache` / `delete_from_cache`
    /// 是 no-op（stub 不真正存储）。
    #[tokio::test]
    async fn test_etcd_cluster_health_monitor_stub_cache_ops() {
        let config = EtcdConfig::default();
        let monitor = EtcdClusterHealthMonitor::new(config, "cache.json".to_string());
        assert!(monitor.get_from_cache("any-key").is_none());
        monitor.put_to_cache("k".to_string(), "v".to_string(), 1);
        // 仍为 None — stub 不真正存储。
        assert!(monitor.get_from_cache("k").is_none());
        monitor.delete_from_cache("k");
        assert!(monitor.get_from_cache("k").is_none());
    }

    /// `start_health_check` / `start_cache_persistence` 接受任意 duration
    /// 并立即返回（no-op stub）。
    #[tokio::test]
    async fn test_etcd_cluster_health_monitor_stub_background_tasks() {
        let config = EtcdConfig::default();
        let monitor = EtcdClusterHealthMonitor::new(config, "cache.json".to_string());
        // 用 timeout 包裹以保证测试有界（no-op 应立即完成）。
        let _ = tokio::time::timeout(
            Duration::from_millis(100),
            monitor.start_health_check(Duration::from_secs(60)),
        )
        .await;
        let _ = tokio::time::timeout(
            Duration::from_millis(100),
            monitor.start_cache_persistence(Duration::from_secs(60)),
        )
        .await;
    }

    /// `EtcdClusterHealthMonitor` 是 `Clone`（derive）。
    #[test]
    fn test_etcd_cluster_health_monitor_stub_clone() {
        let config = EtcdConfig::default();
        let monitor = EtcdClusterHealthMonitor::new(config, "cache.json".to_string());
        let _cloned = monitor.clone();
        // 克隆后状态一致。
        assert_eq!(monitor.get_status(), EtcdClusterStatus::Failed);
    }

    /// 使用不同的 cache_file_path 不影响 stub 行为（参数被忽略）。
    #[tokio::test]
    async fn test_etcd_cluster_health_monitor_stub_different_paths() {
        let config = EtcdConfig::default();
        let m1 = EtcdClusterHealthMonitor::new(config.clone(), "a.json".to_string());
        let m2 = EtcdClusterHealthMonitor::new(config, "b.json".to_string());

        assert_eq!(m1.get_status(), m2.get_status());
        assert_eq!(m1.is_using_cache(), m2.is_using_cache());
        assert!(m1.get_from_cache("x").is_none());
        assert!(m2.get_from_cache("x").is_none());
    }

    /// 综合验证：LocalDistributedLock 在 no-etcd 模式下也可用
    /// （它无条件编译，但此测试确保 cfg gate 不影响其行为）。
    #[tokio::test]
    async fn test_local_distributed_lock_available_without_etcd() {
        let lock = LocalDistributedLock::new();
        let guard = lock.acquire("no-etcd-key", 5).await.unwrap();
        guard.release().await.unwrap();
        // 同一 key 可再次 acquire。
        let guard2 = lock.acquire("no-etcd-key", 5).await.unwrap();
        guard2.release().await.unwrap();
    }
}
