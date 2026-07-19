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
use serde::{Deserialize, Serialize};

#[cfg(not(feature = "etcd"))]
use super::{
    DistributedLock, EtcdClusterStatus, LockError, LockGuard, WorkerAllocatorError,
    WorkerIdAllocator,
};
#[cfg(not(feature = "etcd"))]
use async_trait::async_trait;
#[cfg(not(feature = "etcd"))]
use std::sync::atomic::{AtomicU16, Ordering};
#[cfg(not(feature = "etcd"))]
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

#[cfg(all(test, not(feature = "etcd")))]
mod tests {
    use super::{LocalWorkerAllocator, WorkerIdAllocator};

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
}
