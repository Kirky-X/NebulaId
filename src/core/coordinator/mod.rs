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

//! Coordinator module for Nebula ID.
//!
//! Trait + shared error/status types live here; concrete implementations
//! are split into `local` (no-etcd stub) and `etcd` (full) sub-modules
//! (rule 25: mod.rs 只放 trait + pub re-export).

use async_trait::async_trait;

pub mod etcd;
pub mod local;

pub use local::LocalCacheEntry;

#[derive(Debug, Clone, PartialEq)]
pub enum EtcdClusterStatus {
    Healthy,
    Degraded,
    Failed,
}

#[async_trait]
pub trait WorkerIdAllocator: Send + Sync {
    async fn allocate(&self) -> std::result::Result<u16, WorkerAllocatorError>;
    async fn release(&self, worker_id: u16) -> std::result::Result<(), WorkerAllocatorError>;
    fn get_allocated_id(&self) -> Option<u16>;
    fn is_healthy(&self) -> bool;
}

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

#[async_trait]
pub trait DistributedLock: Send + Sync {
    async fn acquire(
        &self,
        key: &str,
        ttl_seconds: u64,
    ) -> std::result::Result<Box<dyn LockGuard>, LockError>;
    fn is_healthy(&self) -> bool;
}

#[async_trait]
pub trait LockGuard: Send + Sync {
    async fn release(&self) -> std::result::Result<(), LockError>;
}

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

#[cfg(feature = "etcd")]
pub use etcd::{
    EtcdClientOps, EtcdClientWrapper, EtcdClusterHealthMonitor, EtcdDistributedLock, EtcdError,
    EtcdLockGuard, EtcdWorkerAllocator,
};
// `LocalDistributedLock` / `LocalLockGuard` 在两种 feature 下都 re-export：
// - `not(etcd)`：作为唯一的分布式锁实现
// - `etcd`：作为 etcd 失败时的 fallback（main.rs 用）
// 其他 local 类型（LocalWorkerAllocator 等）仅在 `not(etcd)` 下导出。
#[cfg(not(feature = "etcd"))]
pub use local::{EtcdClusterHealthMonitor, LocalWorkerAllocator};

pub use local::{LocalDistributedLock, LocalLockGuard};
