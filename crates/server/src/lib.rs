//! Nebula ID Server

// Public API modules
pub mod grpc;
pub mod router;

// Internal implementation modules
pub mod audit;
pub mod audit_middleware;
pub mod config_hot_reload;
pub mod config_management;
pub mod handlers;
pub mod middleware;
pub mod models;
pub mod rate_limit;
pub mod rate_limit_middleware;
pub mod server_config;
pub mod tls_server;

// Public API re-exports
pub use audit::{AuditEvent, AuditEventType, AuditLogger};
pub use grpc::GrpcServer;
pub use router::create_router;
