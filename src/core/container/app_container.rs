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
use std::any::TypeId;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::OnceLock;
use trait_kit::core::{AsyncAutoBuilder, ModuleMeta};
use trait_kit::AsyncKit;

// ============================================================================
// trait-kit 模块定义 — 替代手写的 DI 构建逻辑
// ============================================================================

/// 配置文件路径（用于 AsyncKit::set_config 传入 from_config_file 的参数）
#[derive(Clone)]
struct ConfigFilePath(String);

/// ConfigModule — 产出 `Arc<dyn ConfigProvider>`，无依赖
struct ConfigModule;

impl ModuleMeta for ConfigModule {
    const NAME: &'static str = "config";
    fn dependencies() -> &'static [(&'static str, TypeId)] {
        &[]
    }
}

impl AsyncAutoBuilder for ConfigModule {
    type Capability = Arc<dyn ConfigProvider>;
    type Error = CoreError;

    fn build<'a>(
        kit: &'a AsyncKit,
    ) -> Pin<Box<dyn Future<Output = Result<Self::Capability, Self::Error>> + Send + 'a>> {
        Box::pin(async move {
            let path = kit
                .config::<ConfigFilePath>()
                .map_err(|e| CoreError::ConfigurationError(e.to_string()))?;
            let config_provider_impl = ConfigProviderImpl::builder()
                .file(&path.0)
                .env()
                .build()
                .map_err(|e| CoreError::ConfigurationError(e.to_string()))?;
            Ok(Arc::new(config_provider_impl) as Arc<dyn ConfigProvider>)
        })
    }
}

/// CacheModule — 产出 `Arc<Cache<String, Vec<u8>>>`，依赖 ConfigModule
struct CacheModule;

impl ModuleMeta for CacheModule {
    const NAME: &'static str = "cache";
    fn dependencies() -> &'static [(&'static str, TypeId)] {
        static DEPS: &[(&str, TypeId)] = &[("config", TypeId::of::<ConfigModule>())];
        DEPS
    }
}

impl AsyncAutoBuilder for CacheModule {
    type Capability = Arc<Cache<String, Vec<u8>>>;
    type Error = CoreError;

    fn build<'a>(
        kit: &'a AsyncKit,
    ) -> Pin<Box<dyn Future<Output = Result<Self::Capability, Self::Error>> + Send + 'a>> {
        Box::pin(async move {
            let config_provider = kit
                .require::<ConfigModule>()
                .map_err(|e| CoreError::ConfigurationError(e.to_string()))?;
            let config_adapter = ConfigAdapter::new(config_provider);
            let redis_config = config_adapter.get_redis_config();

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
            Ok(Arc::new(cache))
        })
    }
}

/// DatabaseModule — 产出 `Arc<dyn dbnexus::ConnectionPool>`，依赖 ConfigModule
struct DatabaseModule;

impl ModuleMeta for DatabaseModule {
    const NAME: &'static str = "database";
    fn dependencies() -> &'static [(&'static str, TypeId)] {
        static DEPS: &[(&str, TypeId)] = &[("config", TypeId::of::<ConfigModule>())];
        DEPS
    }
}

impl AsyncAutoBuilder for DatabaseModule {
    type Capability = Arc<dyn dbnexus::ConnectionPool>;
    type Error = CoreError;

    fn build<'a>(
        kit: &'a AsyncKit,
    ) -> Pin<Box<dyn Future<Output = Result<Self::Capability, Self::Error>> + Send + 'a>> {
        Box::pin(async move {
            let config_provider = kit
                .require::<ConfigModule>()
                .map_err(|e| CoreError::ConfigurationError(e.to_string()))?;
            let config_adapter = ConfigAdapter::new(config_provider);
            let db_config = config_adapter.get_database_config();

            let database = dbnexus::DbPool::new(&db_config.url)
                .await
                .map_err(|e| CoreError::DatabaseError(e.to_string()))?;
            Ok(Arc::new(database) as Arc<dyn dbnexus::ConnectionPool>)
        })
    }
}

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
        // 使用 trait-kit 的 AsyncKit 管理依赖构建拓扑：
        // ConfigModule（无依赖）→ CacheModule/DatabaseModule（依赖 ConfigModule）
        // AsyncKit::build() 自动按拓扑序构建，替代原手写的线性构建逻辑
        let mut kit = AsyncKit::new();
        kit.set_config(ConfigFilePath(config_path.to_string()));
        kit.register::<ConfigModule>()?;
        kit.register::<CacheModule>()?;
        kit.register::<DatabaseModule>()?;
        let kit = kit.build().await?;

        let config = kit.require::<ConfigModule>()?;
        let cache = kit.require::<CacheModule>()?;
        let database = kit.require::<DatabaseModule>()?;

        Ok(Self::with_dependencies(config, cache, database))
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
    use super::*;
    use async_trait::async_trait;
    use dbnexus::database::pool::PoolStatus;
    use dbnexus::{DbConfig, DbError, DbResult, Session};

    // ===== Mock ConfigProvider (minimal, mirrors config_adapter.rs pattern) =====

    struct MockConfigProvider {
        values: std::collections::HashMap<String, confers::types::AnnotatedValue>,
    }

    impl MockConfigProvider {
        fn new() -> Self {
            Self {
                values: std::collections::HashMap::new(),
            }
        }
    }

    impl ConfigProvider for MockConfigProvider {
        fn get_raw(&self, key: &str) -> Option<&confers::types::AnnotatedValue> {
            self.values.get(key)
        }

        fn keys(&self) -> Vec<String> {
            self.values.keys().cloned().collect()
        }
    }

    // ===== Mock ConnectionPool (mirrors database_adapter.rs pattern) =====

    struct MockConnectionPool {
        status: PoolStatus,
        config: DbConfig,
        session_error_msg: String,
    }

    impl MockConnectionPool {
        fn new(status: PoolStatus, config: DbConfig, session_error_msg: String) -> Self {
            Self {
                status,
                config,
                session_error_msg,
            }
        }
    }

    #[async_trait]
    impl dbnexus::ConnectionPool for MockConnectionPool {
        async fn get_session(&self, _role: &str) -> DbResult<Session> {
            Err(DbError::Config(self.session_error_msg.clone()))
        }

        fn status(&self) -> PoolStatus {
            self.status.clone()
        }

        fn config(&self) -> &DbConfig {
            &self.config
        }
    }

    fn pool_status(active: u32, idle: u32, max_active: u32) -> PoolStatus {
        PoolStatus {
            total: active + idle,
            active,
            idle,
            wait_count: 0,
            max_waiters: 0,
            borrow_count: 0,
            max_active,
        }
    }

    // ===== Test fixtures =====

    fn make_mock_config() -> Arc<dyn ConfigProvider> {
        Arc::new(MockConfigProvider::new())
    }

    fn make_mock_db_pool() -> Arc<dyn dbnexus::ConnectionPool> {
        Arc::new(MockConnectionPool::new(
            pool_status(0, 0, 10),
            DbConfig::default(),
            "mock session unavailable".to_string(),
        ))
    }

    async fn make_test_cache() -> Arc<Cache<String, Vec<u8>>> {
        let l1 = MokaMemoryBackend::builder().capacity(100).build();
        let cache: Cache<String, Vec<u8>> = Cache::builder()
            .backend_arc(Arc::new(l1))
            .build()
            .await
            .expect("cache build should succeed with MokaMemoryBackend");
        Arc::new(cache)
    }

    async fn make_container() -> AppContainer {
        AppContainer::with_dependencies(
            make_mock_config(),
            make_test_cache().await,
            make_mock_db_pool(),
        )
    }

    // ===== with_dependencies / accessors =====

    #[tokio::test]
    async fn test_with_dependencies_stores_all_providers() {
        let config = make_mock_config();
        let cache = make_test_cache().await;
        let db = make_mock_db_pool();
        let container = AppContainer::with_dependencies(
            Arc::clone(&config),
            Arc::clone(&cache),
            Arc::clone(&db),
        );
        assert!(
            Arc::ptr_eq(container.config(), &config),
            "config() should return the same Arc"
        );
        assert!(
            Arc::ptr_eq(container.cache(), &cache),
            "cache() should return the same Arc"
        );
        assert!(
            Arc::ptr_eq(container.database(), &db),
            "database() should return the same Arc"
        );
    }

    #[tokio::test]
    async fn test_cache_adapter_returns_reference_to_cache() {
        let container = make_container().await;
        let cache_ref = container.cache_adapter();
        // Verify the reference is usable (compiles + doesn't panic).
        let _ = cache_ref;
    }

    #[tokio::test]
    async fn test_config_adapter_lazily_initializes_and_is_idempotent() {
        let container = make_container().await;
        let adapter1 = container.config_adapter();
        let adapter2 = container.config_adapter();
        assert!(
            std::ptr::eq(adapter1, adapter2),
            "OnceLock should return the same reference on subsequent calls"
        );
    }

    #[tokio::test]
    async fn test_database_adapter_lazily_initializes_and_is_idempotent() {
        let container = make_container().await;
        let adapter1 = container.database_adapter();
        let adapter2 = container.database_adapter();
        assert!(
            std::ptr::eq(adapter1, adapter2),
            "OnceLock should return the same reference on subsequent calls"
        );
    }

    #[tokio::test]
    async fn test_get_config_delegates_to_config_adapter() {
        // With empty MockConfigProvider, ConfigAdapter returns defaults.
        // Default app.name should be "nebula-id" per config_adapter.rs tests.
        let container = make_container().await;
        let config = container.get_config();
        assert_eq!(
            config.app.name, "nebula-id",
            "get_config should return defaults from empty provider"
        );
    }

    #[tokio::test]
    async fn test_health_check_returns_err_when_db_session_unavailable() {
        // MockConnectionPool.get_session always returns Err, so
        // DatabaseAdapter::health_check returns Err, propagated by
        // AppContainer::health_check.
        let container = make_container().await;
        let result = container.health_check().await;
        match result {
            Err(CoreError::DatabaseError(msg)) => {
                assert!(
                    msg.contains("mock session unavailable"),
                    "should propagate DB session error, got: {msg}"
                );
            }
            other => panic!("expected DatabaseError from DB session unavailable, got {other:?}"),
        }
    }

    // ===== Debug impls =====

    #[tokio::test]
    async fn test_app_container_debug_emits_struct_name() {
        let container = make_container().await;
        let debug = format!("{container:?}");
        assert!(
            debug.contains("AppContainer"),
            "Debug should contain struct name, got: {debug}"
        );
        assert!(
            debug.contains("Arc<dyn ConfigProvider>"),
            "Debug should contain config field placeholder, got: {debug}"
        );
    }

    #[test]
    fn test_app_container_builder_debug_reports_set_fields() {
        let builder = AppContainerBuilder::new();
        let debug = format!("{builder:?}");
        assert!(
            debug.contains("AppContainerBuilder"),
            "Debug should contain builder struct name, got: {debug}"
        );
        // No fields set, all should be false
        assert!(
            debug.contains("false"),
            "Debug should report unset fields as false, got: {debug}"
        );
    }

    // ===== builder() / build() / try_build() =====

    #[test]
    fn test_builder_new_returns_default_builder() {
        let builder = AppContainerBuilder::new();
        // Try building without any deps - should panic, but we'll use try_build
        let result = builder.try_build();
        assert!(result.is_err(), "empty builder should fail try_build");
    }

    #[tokio::test]
    async fn test_builder_config_setter_stores_provider() {
        let cache = make_test_cache().await;
        let db = make_mock_db_pool();
        let config = make_mock_config();
        let container = AppContainer::builder()
            .config(config)
            .cache(cache)
            .database(db)
            .build();
        // Verify build succeeded by accessing a field
        let _ = container.config();
    }

    #[tokio::test]
    async fn test_builder_cache_setter_stores_backend() {
        let config = make_mock_config();
        let cache = make_test_cache().await;
        let db = make_mock_db_pool();
        let container = AppContainer::builder()
            .cache(cache)
            .config(config)
            .database(db)
            .build();
        let _ = container.cache();
    }

    #[tokio::test]
    async fn test_builder_database_setter_stores_pool() {
        let config = make_mock_config();
        let cache = make_test_cache().await;
        let db = make_mock_db_pool();
        let container = AppContainer::builder()
            .database(db)
            .config(config)
            .cache(cache)
            .build();
        let _ = container.database();
    }

    #[tokio::test]
    #[should_panic(expected = "config provider is required")]
    async fn test_builder_build_panics_when_config_missing() {
        let cache = make_test_cache().await;
        let db = make_mock_db_pool();
        let _ = AppContainer::builder().cache(cache).database(db).build();
    }

    #[tokio::test]
    #[should_panic(expected = "cache backend is required")]
    async fn test_builder_build_panics_when_cache_missing() {
        let config = make_mock_config();
        let db = make_mock_db_pool();
        let _ = AppContainer::builder().config(config).database(db).build();
    }

    #[tokio::test]
    #[should_panic(expected = "database connection pool is required")]
    async fn test_builder_build_panics_when_database_missing() {
        let config = make_mock_config();
        let cache = make_test_cache().await;
        let _ = AppContainer::builder().config(config).cache(cache).build();
    }

    #[tokio::test]
    async fn test_builder_try_build_returns_config_error_when_config_missing() {
        let cache = make_test_cache().await;
        let db = make_mock_db_pool();
        let result = AppContainer::builder()
            .cache(cache)
            .database(db)
            .try_build();
        match result {
            Err(CoreError::ConfigurationError(msg)) => {
                assert!(
                    msg.contains("config provider"),
                    "should mention config provider, got: {msg}"
                );
            }
            other => panic!("expected ConfigurationError, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_builder_try_build_returns_config_error_when_cache_missing() {
        let config = make_mock_config();
        let db = make_mock_db_pool();
        let result = AppContainer::builder()
            .config(config)
            .database(db)
            .try_build();
        match result {
            Err(CoreError::ConfigurationError(msg)) => {
                assert!(msg.contains("cache"), "should mention cache, got: {msg}");
            }
            other => panic!("expected ConfigurationError, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_builder_try_build_returns_config_error_when_database_missing() {
        let config = make_mock_config();
        let cache = make_test_cache().await;
        let result = AppContainer::builder()
            .config(config)
            .cache(cache)
            .try_build();
        match result {
            Err(CoreError::ConfigurationError(msg)) => {
                assert!(
                    msg.contains("database"),
                    "should mention database, got: {msg}"
                );
            }
            other => panic!("expected ConfigurationError, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_builder_try_build_succeeds_when_all_deps_provided() {
        let config = make_mock_config();
        let cache = make_test_cache().await;
        let db = make_mock_db_pool();
        let result = AppContainer::builder()
            .config(config)
            .cache(cache)
            .database(db)
            .try_build();
        assert!(
            result.is_ok(),
            "try_build should succeed when all deps provided: {result:?}"
        );
    }

    #[tokio::test]
    async fn test_builder_debug_shows_set_fields_after_population() {
        let config = make_mock_config();
        let cache = make_test_cache().await;
        let db = make_mock_db_pool();
        let builder = AppContainer::builder()
            .config(config)
            .cache(cache)
            .database(db);
        let debug = format!("{builder:?}");
        assert!(
            debug.contains("true"),
            "Debug should report set fields as true after population, got: {debug}"
        );
    }
}
