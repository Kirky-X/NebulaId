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

//! Nebula ID - Enterprise-grade distributed ID generation system
//!
//! This crate provides a unified API for distributed ID generation with support for
//! multiple algorithms (Segment, Snowflake, UUID v4/v7) and features like
//! distributed coordination, caching, and monitoring.
//!
//! # Architecture
//!
//! - [`core`] - Core business logic for ID generation algorithms
//! - [`server`] - HTTP/gRPC server implementations
//!
//! # Usage
//!
//! ```rust
//! use nebulaid::core::{Config, AppContainer};
//! ```

// Core namespace - 核心业务逻辑
pub mod core;

// Server namespace - HTTP/gRPC 服务
pub mod server;

// Internal implementation modules
pub(crate) mod infrastructure;
