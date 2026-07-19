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

//! Application container implementation.

use crate::core::types::CoreError;
use crate::infrastructure::{ConfigAdapter, ConfigProviderImpl, DatabaseAdapter};
use confers::interface::ConfigProvider;
use oxcache::backend::{MokaMemoryBackend, RedisBackend};
use oxcache::cache::{ChainCache, ChainLink};
use oxcache::Cache;
use std::sync::Arc;
use std::sync::OnceLock;

/// Application container for managing all dependencies.
///
/// This container follows the dependency injection pattern from di.md,
/// providing singleton management for infrastructure components and
/// lazy-loaded adapters for the feature layer.
///
/// # Construction Modes
///
/// The container supports three construction modes:
///
/// 1. `new()` - Creates container with default configuration (panics on error)
/// 2. `builder()` - Creates container with partial configuration
/// 3. `with_dependencies()` - Creates container with all dependencies injected
///
/// # Example
///
/// ```rust,ignore
/// // With full DI (recommended)
/// let container = AppContainer::with_dependencies(
///     config_provider,
///     cache,
///     connection_pool,
/// );
///
/// // With builder
/// let container = AppContainer::builder()
///     .config(config_provider)
///     .cache(cache)
///     .database(connection_pool)
///     .build();
/// ```
pub struct AppContainer {
    /// Infrastructure layer: Configuration provider
    config: Arc<dyn ConfigProvider>,
    /// Infrastructure layer: Cache backend (直接使用 oxcache Cache)
    cache: Arc<Cache<String, Vec<u8>>>,
    /// Infrastructure layer: Database connection pool
    database: Arc<dyn dbnexus::ConnectionPool>,

    /// Feature layer: Configuration adapter (lazy-loaded)
    config_adapter: OnceLock<ConfigAdapter>,
    /// Feature layer: Database adapter (lazy-loaded)
    database_adapter: OnceLock<DatabaseAdapter>,
}

impl AppContainer {
    /// Create a new container with dependencies injected.
    ///
    /// This is the primary construction mode for full DI support.
    ///
    /// # Arguments
    ///
    /// * `config` - Configuration provider from confers
    /// * `cache` - Cache backend from oxcache (`Cache<String, Vec<u8>>`)
    /// * `database` - Connection pool from dbnexus
    pub fn with_dependencies(
        config: Arc<dyn ConfigProvider>,
        cache: Arc<Cache<String, Vec<u8>>>,
        database: Arc<dyn dbnexus::ConnectionPool>,
    ) -> Self {
        Self {
            config,
            cache,
            database,
            config_adapter: OnceLock::new(),
            database_adapter: OnceLock::new(),
        }
    }

    /// Create a new container builder.
    ///
    /// Use the builder pattern for more flexible configuration.
    pub fn builder() -> AppContainerBuilder {
        AppContainerBuilder::new()
    }

    /// Create a container from a configuration file.
    ///
    /// This is a convenience method that loads configuration from a TOML file
    /// and initializes the container with default cache and database backends.
    ///
    /// # Arguments
    ///
    /// * `config_path` - Path to the configuration file (TOML format)
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let container = AppContainer::from_config_file("config/config.toml").await?;
    /// let segment_config = container.config_adapter().get_segment_config();
    /// ```
    pub async fn from_config_file(config_path: &str) -> Result<Self, CoreError> {
        // Load configuration
        let config_provider_impl = ConfigProviderImpl::builder()
            .file(config_path)
            .env()
            .build()
            .map_err(|e| CoreError::ConfigurationError(e.to_string()))?;

        // Convert to trait object
        let config_provider: Arc<dyn ConfigProvider> = Arc::new(config_provider_impl);
        let config_adapter = ConfigAdapter::new(Arc::clone(&config_provider));

        // Get database configuration
        let db_config = config_adapter.get_database_config();

        // Get Redis configuration for tiered cache
        let redis_config = config_adapter.get_redis_config();

        // Initialize tiered cache backend (L1 memory + L2 Redis) using oxcache.
        // oxcache 0.3.8 removed CacheBuilder::tiered; ChainCache now provides
        // multi-tier composition and implements CacheBackend, so it can be
        // wrapped by Cache<K, V> via backend_arc.
        let l1 = MokaMemoryBackend::builder().capacity(10000).build();
        let l2 = RedisBackend::new(&redis_config.url)
            .await
            .map_err(|e| CoreError::CacheError(e.to_string()))?;
        let chain = ChainCache::builder()
            .link(ChainLink::from_backend(l1))
            .link(ChainLink::from_backend(l2))
            .build();
        let cache: Cache<String, Vec<u8>> = Cache::builder()
            .backend_arc(Arc::new(chain))
            .build()
            .await
            .map_err(|e| CoreError::CacheError(e.to_string()))?;

        // Initialize database pool (using dbnexus)
        let database = dbnexus::DbPool::new(&db_config.url)
            .await
            .map_err(|e| CoreError::DatabaseError(e.to_string()))?;

        Ok(Self::with_dependencies(
            config_provider,
            Arc::new(cache),
            Arc::new(database),
        ))
    }

    /// Get the configuration adapter.
    ///
    /// The adapter is lazily initialized on first access.
    pub fn config_adapter(&self) -> &ConfigAdapter {
        self.config_adapter
            .get_or_init(|| ConfigAdapter::new(Arc::clone(&self.config)))
    }

    /// Get the cache backend directly.
    ///
    /// Returns the underlying oxcache Cache instance.
    pub fn cache_adapter(&self) -> &Cache<String, Vec<u8>> {
        &self.cache
    }

    /// Get the database adapter.
    ///
    /// The adapter is lazily initialized on first access.
    pub fn database_adapter(&self) -> &DatabaseAdapter {
        self.database_adapter
            .get_or_init(|| DatabaseAdapter::new(Arc::clone(&self.database)))
    }

    /// Get the underlying configuration provider.
    pub fn config(&self) -> &Arc<dyn ConfigProvider> {
        &self.config
    }

    /// Get the underlying cache backend.
    pub fn cache(&self) -> &Arc<Cache<String, Vec<u8>>> {
        &self.cache
    }

    /// Get the underlying database connection pool.
    pub fn database(&self) -> &Arc<dyn dbnexus::ConnectionPool> {
        &self.database
    }

    /// Get the complete configuration.
    ///
    /// This is a convenience method that assembles the full configuration.
    pub fn get_config(&self) -> crate::core::config::Config {
        self.config_adapter().get_config()
    }

    /// Check if all components are healthy.
    pub async fn health_check(&self) -> Result<bool, CoreError> {
        // Check cache health using the direct Cache API.
        // oxcache 0.3.8 health_check returns OxCacheResult<()>, so reaching
        // here without error means the cache is healthy.
        self.cache_adapter()
            .health_check()
            .await
            .map_err(CoreError::from)?;
        let cache_healthy = true;

        // Check database health
        let db_healthy = self.database_adapter().health_check().await?;

        Ok(cache_healthy && db_healthy)
    }
}

impl std::fmt::Debug for AppContainer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppContainer")
            .field("config", &"Arc<dyn ConfigProvider>")
            .field("cache", &"Arc<Cache<String, Vec<u8>>>")
            .field("database", &"Arc<ConnectionPool>")
            .finish()
    }
}

/// Builder for AppContainer.
///
/// This builder allows partial dependency injection.
/// All dependencies must be provided before building.
#[derive(Default)]
pub struct AppContainerBuilder {
    config: Option<Arc<dyn ConfigProvider>>,
    cache: Option<Arc<Cache<String, Vec<u8>>>>,
    database: Option<Arc<dyn dbnexus::ConnectionPool>>,
}

impl AppContainerBuilder {
    /// Create a new builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the configuration provider.
    pub fn config(mut self, config: Arc<dyn ConfigProvider>) -> Self {
        self.config = Some(config);
        self
    }

    /// Set the cache backend.
    pub fn cache(mut self, cache: Arc<Cache<String, Vec<u8>>>) -> Self {
        self.cache = Some(cache);
        self
    }

    /// Set the database connection pool.
    pub fn database(mut self, database: Arc<dyn dbnexus::ConnectionPool>) -> Self {
        self.database = Some(database);
        self
    }

    /// Build the AppContainer.
    ///
    /// # Panics
    ///
    /// Panics if any required dependency is missing.
    pub fn build(self) -> AppContainer {
        let config = self.config.expect("config provider is required");
        let cache = self.cache.expect("cache backend is required");
        let database = self.database.expect("database connection pool is required");

        AppContainer::with_dependencies(config, cache, database)
    }

    /// Try to build the AppContainer.
    ///
    /// Returns an error if any required dependency is missing.
    pub fn try_build(self) -> Result<AppContainer, CoreError> {
        let config = self.config.ok_or_else(|| {
            CoreError::ConfigurationError("config provider is required".to_string())
        })?;
        let cache = self.cache.ok_or_else(|| {
            CoreError::ConfigurationError("cache backend is required".to_string())
        })?;
        let database = self.database.ok_or_else(|| {
            CoreError::ConfigurationError("database connection pool is required".to_string())
        })?;

        Ok(AppContainer::with_dependencies(config, cache, database))
    }
}

impl std::fmt::Debug for AppContainerBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppContainerBuilder")
            .field("config", &self.config.is_some())
            .field("cache", &self.cache.is_some())
            .field("database", &self.database.is_some())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    // Tests would require mock implementations
    // Integration tests should verify full container functionality
}
