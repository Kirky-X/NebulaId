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

//! Core module - 核心业务逻辑

// Public API modules (re-exported in lib.rs)
pub mod algorithm;
pub mod auth;
pub mod config;
pub mod container;
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

pub use auth::{AuthManager, Authenticator};

pub use container::AppContainer;

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
