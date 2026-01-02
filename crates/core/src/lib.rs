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

// Public API modules
pub mod algorithm;
pub mod auth;
pub mod cache;
pub mod config;
pub mod types;

// Internal implementation modules
pub(crate) mod config_management;
pub(crate) mod database;
pub(crate) mod dynamic_config;
pub(crate) mod monitoring;

// Coordinator module is pub to allow EtcdClusterHealthMonitor re-export,
// but NOT re-exported in public API (not in docs/crate root)
#[cfg(feature = "etcd")]
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

pub use cache::MultiLevelCache;

pub use config::{Config, TlsConfig};

// Re-export coordinator types that need to be accessed externally
#[cfg(feature = "etcd")]
pub use coordinator::EtcdClusterHealthMonitor;
