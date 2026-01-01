//! Nebula ID Server

// Public API modules
pub mod grpc;
pub mod router;

// Internal implementation modules
pub(crate) mod audit;
pub(crate) mod audit_middleware;
pub(crate) mod config_hot_reload;
pub(crate) mod config_management;
pub(crate) mod handlers;
pub(crate) mod middleware;
pub(crate) mod models;
pub(crate) mod rate_limit;
pub(crate) mod rate_limit_middleware;
pub(crate) mod server_config;
pub(crate) mod tls_server;

// Public API re-exports
pub use audit::{AuditEvent, AuditEventType, AuditLogger};
pub use grpc::GrpcServer;
pub use router::create_router;
