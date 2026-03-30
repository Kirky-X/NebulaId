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

//! Infrastructure adapters for external components.
//!
//! This module provides adapters that wrap external infrastructure components
//! (confers, dbnexus) and provide domain-specific interfaces for
//! the Nebula ID feature layer.
//!
//! # Architecture
//!
//! ```text
//! Feature Layer (nebulaid::core)
//!     │
//!     ├── Algorithm ────────────────┐
//!     ├── Auth ─────────────────────┤
//!     │                              │
//!     │   infrastructure/            │
//!     │   ├── ConfigAdapter ─────────┼──► confers (ConfigProvider)
//!     │   └── DatabaseAdapter ───────┼──► dbnexus (ConnectionPool)
//!     │                              │
//! Infrastructure Layer                │
//!     │
//!     └── oxcache (直接使用 Cache API)
//! ```

pub mod config_adapter;
pub mod config_provider;
pub mod database_adapter;

pub use config_adapter::ConfigAdapter;
pub use config_provider::ConfigProviderImpl;
pub use database_adapter::DatabaseAdapter;
