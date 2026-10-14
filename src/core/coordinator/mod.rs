// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Coordinator 模块：trait 与共享错误/状态类型在此定义。
//!
//! 实现拆入 `local`（无 etcd 时的进程内降级）与 `etcd`（生产实现）子模块。

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

/// Segment 号段分配分布式锁的 etcd 键前缀（服务端与嵌入方共用的单一来源）。
///
/// 公开而非 `pub(crate)`：消费方含同包 bin crate 读不到 `pub(crate)` 项，
/// 且 `etcd` / `sdk` 两种 feature 组合下都会触发 `dead_code`。
pub const SEGMENT_LOCK_PATH_PREFIX: &str = "nebulaid-segment-lock";

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
// LocalDistributedLock / LocalLockGuard 两种 feature 下均 re-export：
// not(etcd) 时唯一锁实现，etcd 时作 etcd 失败的 fallback。
// 其他 local 类型仅在 not(etcd) 下导出。
#[cfg(not(feature = "etcd"))]
pub use local::{EtcdClusterHealthMonitor, LocalWorkerAllocator};

pub use local::{LocalDistributedLock, LocalLockGuard};
