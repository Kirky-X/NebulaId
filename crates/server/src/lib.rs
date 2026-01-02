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

//! Nebula ID Server

// Public API modules
pub mod grpc;
pub mod router;

// Internal implementation modules
// Note: These are pub (not pub(crate)) so they're accessible from the binary target
// but NOT re-exported in the public API
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
