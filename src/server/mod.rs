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

//! Server module - HTTP/gRPC 服务实现

// Public API modules (re-exported in lib.rs)
pub mod api_version;
pub mod grpc;
pub mod router;

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
