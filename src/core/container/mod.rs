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

//! Application container for dependency injection.
//!
//! This module provides a centralized container for managing all application
//! dependencies, following the dependency injection pattern defined in di.md.
//!
//! # Architecture
//!
//! ```text
//! AppContainer
//!     │
//!     ├── Infrastructure Layer (singleton)
//!     │   ├── config: Arc<dyn ConfigProvider>
//!     │   ├── cache: Arc<Cache<String, Vec<u8>>>  (oxcache)
//!     │   └── database: Arc<dyn ConnectionPool>
//!     │
//!     └── Feature Layer (lazy-loaded)
//!         ├── config_adapter: OnceCell<ConfigAdapter>
//!         └── database_adapter: OnceCell<DatabaseAdapter>
//! ```
//!
//! # Usage
//!
//! ```rust,ignore
//! use crate::core::container::AppContainer;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Create container with config file
//!     let container = AppContainer::new("config/config.toml").await?;
//!     
//!     // Access adapters
//!     let config = container.config_adapter();
//!     let cache = container.cache_adapter();
//!     
//!     Ok(())
//! }
//! ```

mod app_container;

pub use app_container::AppContainer;
