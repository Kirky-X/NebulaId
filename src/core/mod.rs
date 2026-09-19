// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Core module - 核心业务逻辑

// Public API modules (re-exported in lib.rs)
pub mod algorithm;
pub mod auth;
pub mod config;
pub mod database;
pub mod i18n;
pub mod monitoring;
pub mod types;

// Coordinator module - conditionally compiled based on feature flags
pub mod coordinator;

#[cfg(test)]
mod tests;

// Public API re-exports
pub use types::*;

pub use algorithm::{
    AlgorithmBuilder, AlgorithmMetricsSnapshot, GenerateContext, HealthStatus, IdAlgorithm,
    IdGenerator,
};

pub use types::{Id, IdBatch};

pub use config::{Config, TlsConfig};

// Re-export oxcache types for convenience
pub use oxcache::Cache;

// Re-export coordinator types that need to be accessed externally
#[cfg(feature = "etcd")]
pub use coordinator::{
    DistributedLock, EtcdClusterHealthMonitor, EtcdDistributedLock, EtcdLockGuard, LockError,
    LockGuard,
};

#[cfg(not(feature = "etcd"))]
pub use coordinator::{
    DistributedLock, LocalDistributedLock, LocalLockGuard, LockError, LockGuard,
};
