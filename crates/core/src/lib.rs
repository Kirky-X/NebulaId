mod algorithm;
mod auth;
mod cache;
mod config;
mod config_management;
mod coordinator;
mod database;
mod dynamic_config;
mod monitoring;
mod types;

#[cfg(test)]
pub mod tests;

pub use types::*;

pub use algorithm::{
    AlgorithmBuilder, AlgorithmMetricsSnapshot, GenerateContext, HealthStatus, IdAlgorithm,
    IdGenerator,
};

pub use types::{Id, IdBatch};

pub use auth::{AuthManager, Authenticator};

pub use cache::MultiLevelCache;

pub use config::{Config, TlsConfig};

pub use config_management::ConfigManagementService;

pub use coordinator::{EtcdClusterHealthMonitor, EtcdClusterStatus, LocalCacheEntry};

pub use database::{
    AlgorithmType, BizTag, BizTagRepository, CreateBizTagRequest, CreateGroupRequest,
    CreateWorkspaceRequest, Group, GroupRepository, IdFormat, UpdateBizTagRequest,
    UpdateGroupRequest, UpdateWorkspaceRequest, Workspace, WorkspaceRepository, WorkspaceStatus,
};

pub use dynamic_config::DynamicConfigService;
