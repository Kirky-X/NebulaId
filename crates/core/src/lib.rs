// Public API modules
pub mod algorithm;
pub mod auth;
pub mod cache;
pub mod config;
pub mod types;

// Internal implementation modules
pub(crate) mod config_management;
pub(crate) mod coordinator;
pub(crate) mod database;
pub(crate) mod dynamic_config;
pub(crate) mod monitoring;

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
