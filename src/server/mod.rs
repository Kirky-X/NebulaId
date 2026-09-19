// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Server module - HTTP/gRPC 服务实现

// Public API modules (re-exported in lib.rs)
pub mod api_version;
pub mod grpc;
pub mod router;

// garrison 集成模块：仅在 garrison-auth 特性启用时编译
#[cfg(feature = "garrison-auth")]
pub mod auth;

// Internal implementation modules
// These are pub for binary target access but NOT part of the public library API
// Users should only use types re-exported in lib.rs
pub mod audit;
pub mod config;
pub mod handlers;
pub mod middleware;
pub mod models;
pub mod openapi;
pub mod rate_limit;
pub mod sdforge_adapter;

// Proto module (internal use only, but needed by binary target)
pub mod proto;

// Public API re-exports
pub use api_version::{api_version_middleware, ApiVersion, API_V1, API_V2, API_VERSION_HEADER};
pub use audit::AuditLogger;
pub use grpc::GrpcServer;
pub use router::create_router;
