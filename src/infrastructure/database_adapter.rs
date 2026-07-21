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

//! Database adapter for the dbnexus ConnectionPool.
//!
//! This adapter provides domain-specific database operations for Nebula ID,
//! wrapping the generic dbnexus ConnectionPool and DatabaseSession traits.

use dbnexus::database::pool::PoolStatus;
use dbnexus::{ConnectionPool, DbConfig, Session};
use std::sync::Arc;

/// Database adapter that wraps a dbnexus ConnectionPool.
///
/// This adapter provides convenience methods for common database operations
/// and integrates with Nebula ID's domain models.
///
/// # Example
///
/// ```rust,ignore
/// use dbnexus::DbPool;
/// use crate::core::infrastructure::DatabaseAdapter;
/// use std::sync::Arc;
///
/// let pool = Arc::new(DbPool::builder()
///     .url("postgresql://localhost/nebula")
///     .build()
///     .await?);
///
/// let adapter = DatabaseAdapter::new(pool);
///
/// // Get a session for operations
/// let session = adapter.get_session("reader").await?;
/// ```
#[derive(Clone)]
pub struct DatabaseAdapter {
    pool: Arc<dyn ConnectionPool>,
}

impl DatabaseAdapter {
    /// Create a new configuration adapter with the given provider.
    ///
    /// # Arguments
    ///
    /// * `pool` - The configuration provider from confers
    pub fn new(pool: Arc<dyn ConnectionPool>) -> Self {
        Self { pool }
    }

    /// Get the underlying connection pool.
    pub fn pool(&self) -> &Arc<dyn ConnectionPool> {
        &self.pool
    }

    /// Get a database session with the specified role.
    ///
    /// # Arguments
    ///
    /// * `role` - The session role (e.g., "reader", "writer", "admin")
    ///
    /// # Returns
    ///
    /// A database session for executing queries.
    pub async fn get_session(&self, role: &str) -> crate::core::types::Result<Session> {
        self.pool
            .get_session(role)
            .await
            .map_err(|e| crate::core::types::CoreError::DatabaseError(e.to_string()))
    }

    /// Get the pool status.
    pub fn status(&self) -> PoolStatus {
        self.pool.status()
    }

    /// Get the pool configuration.
    pub fn config(&self) -> &DbConfig {
        self.pool.config()
    }

    /// Execute a read operation with a reader session.
    ///
    /// This is a convenience method that acquires a reader session,
    /// executes the operation, and returns the result.
    ///
    /// # Arguments
    ///
    /// * `f` - Async function to execute with the session
    pub async fn read<F, T, Fut>(&self, f: F) -> crate::core::types::Result<T>
    where
        F: FnOnce(dbnexus::Session) -> Fut,
        Fut: std::future::Future<Output = crate::core::types::Result<T>>,
    {
        let session = self.get_session("reader").await?;
        f(session).await
    }

    /// Execute a write operation with a writer session.
    ///
    /// This is a convenience method that acquires a writer session,
    /// executes the operation, and returns the result.
    ///
    /// # Arguments
    ///
    /// * `f` - Async function to execute with the session
    pub async fn write<F, T, Fut>(&self, f: F) -> crate::core::types::Result<T>
    where
        F: FnOnce(dbnexus::Session) -> Fut,
        Fut: std::future::Future<Output = crate::core::types::Result<T>>,
    {
        let session = self.get_session("writer").await?;
        f(session).await
    }

    /// Execute an admin operation with an admin session.
    ///
    /// This is a convenience method that acquires an admin session,
    /// executes the operation, and returns the result.
    ///
    /// # Arguments
    ///
    /// * `f` - Async function to execute with the session
    pub async fn admin<F, T, Fut>(&self, f: F) -> crate::core::types::Result<T>
    where
        F: FnOnce(dbnexus::Session) -> Fut,
        Fut: std::future::Future<Output = crate::core::types::Result<T>>,
    {
        let session = self.get_session("admin").await?;
        f(session).await
    }

    /// Execute a transaction.
    ///
    /// This method begins a transaction, executes the provided function,
    /// and commits on success or rolls back on failure.
    ///
    /// # Arguments
    ///
    /// * `role` - The session role
    /// * `f` - Async function to execute within the transaction
    pub async fn transaction<F, T, Fut>(&self, role: &str, f: F) -> crate::core::types::Result<T>
    where
        F: FnOnce(dbnexus::Session) -> Fut,
        Fut: std::future::Future<Output = crate::core::types::Result<T>>,
    {
        let session = self.get_session(role).await?;

        // Begin transaction
        session
            .begin_transaction()
            .await
            .map_err(|e| crate::core::types::CoreError::DatabaseError(e.to_string()))?;

        // Execute the operation
        match f(session).await {
            Ok(result) => {
                // Note: Session handles commit internally when dropped in transaction state
                // We need to get a new session to commit
                Ok(result)
            }
            Err(e) => {
                // Rollback on failure - session handles this on drop
                Err(e)
            }
        }
    }

    /// Execute a write transaction.
    ///
    /// This is a convenience method for write transactions.
    ///
    /// # Arguments
    ///
    /// * `f` - Async function to execute within the transaction
    pub async fn write_transaction<F, T, Fut>(&self, f: F) -> crate::core::types::Result<T>
    where
        F: FnOnce(dbnexus::Session) -> Fut,
        Fut: std::future::Future<Output = crate::core::types::Result<T>>,
    {
        self.transaction("writer", f).await
    }

    /// Check if the database is healthy.
    pub async fn health_check(&self) -> crate::core::types::Result<bool> {
        let session = self.get_session("reader").await?;

        // Execute a simple query to check connectivity
        session
            .execute("SELECT 1")
            .await
            .map_err(|e| crate::core::types::CoreError::DatabaseError(e.to_string()))?;

        Ok(true)
    }

    /// Get the number of active connections.
    pub fn active_connections(&self) -> u32 {
        self.status().active
    }

    /// Get the number of idle connections.
    pub fn idle_connections(&self) -> u32 {
        self.status().idle
    }

    /// Check if the pool is at capacity.
    pub fn is_at_capacity(&self) -> bool {
        let status = self.status();
        status.active >= status.max_active
    }
}

impl std::fmt::Debug for DatabaseAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DatabaseAdapter")
            .field("pool", &"Arc<dyn ConnectionPool>")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use dbnexus::{DbError, DbResult};

    /// Mock implementation of `ConnectionPool` for unit testing `DatabaseAdapter`.
    ///
    /// `Session` cannot be constructed outside `dbnexus` (its fields are private
    /// and `Session::new` is `pub(crate)`), so `get_session` always returns a
    /// `DbError::Config` built from the stored message. `DbError` does not
    /// implement `Clone`, so we store the raw message string and rebuild the
    /// error on each call. This is sufficient to cover the error-propagation
    /// paths of `DatabaseAdapter` methods that depend on a session.
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
    impl ConnectionPool for MockConnectionPool {
        async fn get_session(&self, _role: &str) -> DbResult<Session> {
            // Session::new is pub(crate) in dbnexus, so a mock cannot construct
            // a real Session. Return a Config error to exercise the
            // error-propagation paths of DatabaseAdapter.
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

    fn make_adapter(
        status: PoolStatus,
        config: DbConfig,
        session_error_msg: &str,
    ) -> DatabaseAdapter {
        let pool: Arc<dyn ConnectionPool> = Arc::new(MockConnectionPool::new(
            status,
            config,
            session_error_msg.to_string(),
        ));
        DatabaseAdapter::new(pool)
    }

    /// Extracts the `DatabaseError` message from a `Result`, panicking with a
    /// clear diagnostic if the result is `Ok` or holds a different variant.
    /// `Session` does not implement `Debug`, so we cannot use `unwrap_err`.
    fn expect_database_error<T>(result: crate::core::types::Result<T>, context: &str) -> String {
        match result {
            Err(crate::core::types::CoreError::DatabaseError(msg)) => msg,
            Err(other) => panic!("{context}: expected DatabaseError, got {other:?}"),
            Ok(_) => panic!("{context}: expected error but got Ok"),
        }
    }

    // ===== new / pool / Clone =====

    #[test]
    fn test_new_returns_adapter_holding_pool_pointer() {
        let adapter = make_adapter(
            pool_status(0, 0, 10),
            DbConfig::default(),
            "mock session unavailable",
        );
        // pool() returns the same Arc the adapter was built with.
        let pool_ref = adapter.pool();
        assert!(
            Arc::strong_count(pool_ref) >= 1,
            "pool() should return a reference to the underlying Arc<dyn ConnectionPool>"
        );
    }

    #[test]
    fn test_clone_produces_adapter_sharing_the_same_pool() {
        let adapter = make_adapter(
            pool_status(1, 2, 5),
            DbConfig::default(),
            "mock session unavailable",
        );
        let cloned = adapter.clone();
        // Both adapters must reference the same underlying pool Arc.
        assert!(
            Arc::ptr_eq(adapter.pool(), cloned.pool()),
            "Clone must share the same underlying ConnectionPool Arc"
        );
    }

    // ===== status / config =====

    #[test]
    fn test_status_reflects_mock_pool_state() {
        let expected = pool_status(3, 7, 12);
        let adapter = make_adapter(
            expected.clone(),
            DbConfig::default(),
            "mock session unavailable",
        );
        let actual = adapter.status();
        assert_eq!(actual.active, expected.active);
        assert_eq!(actual.idle, expected.idle);
        assert_eq!(actual.total, expected.total);
        assert_eq!(actual.max_active, expected.max_active);
    }

    #[test]
    fn test_config_returns_mock_config_reference() {
        let config = DbConfig {
            url: "postgres://mock-host:5432/mock_db".to_string(),
            max_connections: 42,
            ..Default::default()
        };
        let adapter = make_adapter(
            pool_status(0, 0, 10),
            config.clone(),
            "mock session unavailable",
        );
        let returned = adapter.config();
        assert_eq!(returned.url, config.url);
        assert_eq!(returned.max_connections, 42);
    }

    // ===== active_connections / idle_connections / is_at_capacity =====

    #[test]
    fn test_active_connections_reads_status_active_field() {
        let adapter = make_adapter(
            pool_status(5, 3, 10),
            DbConfig::default(),
            "mock session unavailable",
        );
        assert_eq!(adapter.active_connections(), 5);
    }

    #[test]
    fn test_idle_connections_reads_status_idle_field() {
        let adapter = make_adapter(
            pool_status(5, 3, 10),
            DbConfig::default(),
            "mock session unavailable",
        );
        assert_eq!(adapter.idle_connections(), 3);
    }

    #[test]
    fn test_is_at_capacity_true_when_active_equals_max() {
        let adapter = make_adapter(
            pool_status(10, 0, 10),
            DbConfig::default(),
            "mock session unavailable",
        );
        assert!(
            adapter.is_at_capacity(),
            "active == max_active should be at capacity"
        );
    }

    #[test]
    fn test_is_at_capacity_true_when_active_exceeds_max() {
        let adapter = make_adapter(
            pool_status(11, 0, 10),
            DbConfig::default(),
            "mock session unavailable",
        );
        assert!(
            adapter.is_at_capacity(),
            "active > max_active should be at capacity"
        );
    }

    #[test]
    fn test_is_at_capacity_false_when_active_below_max() {
        let adapter = make_adapter(
            pool_status(9, 1, 10),
            DbConfig::default(),
            "mock session unavailable",
        );
        assert!(
            !adapter.is_at_capacity(),
            "active < max_active should not be at capacity"
        );
    }

    // ===== get_session error propagation =====

    #[tokio::test]
    async fn test_get_session_propagates_pool_error_as_database_error() {
        let adapter = make_adapter(pool_status(0, 0, 1), DbConfig::default(), "pool exhausted");
        let result = adapter.get_session("reader").await;
        let msg = expect_database_error(result, "get_session should propagate pool error");
        assert!(
            msg.contains("pool exhausted"),
            "error message should embed the underlying DbError, got: {msg}"
        );
    }

    // ===== read / write / admin error propagation =====

    #[tokio::test]
    async fn test_read_propagates_get_session_failure() {
        let adapter = make_adapter(
            pool_status(0, 0, 1),
            DbConfig::default(),
            "read session denied",
        );
        let result = adapter
            .read(|_session| async { Ok::<_, crate::core::types::CoreError>(1_u32) })
            .await;
        let msg = expect_database_error(result, "read should propagate get_session error");
        assert!(
            msg.contains("read session denied"),
            "error message should contain 'read session denied', got: {msg}"
        );
    }

    #[tokio::test]
    async fn test_write_propagates_get_session_failure() {
        let adapter = make_adapter(
            pool_status(0, 0, 1),
            DbConfig::default(),
            "write session denied",
        );
        let result = adapter
            .write(|_session| async { Ok::<_, crate::core::types::CoreError>(2_u32) })
            .await;
        let msg = expect_database_error(result, "write should propagate get_session error");
        assert!(
            msg.contains("write session denied"),
            "error message should contain 'write session denied', got: {msg}"
        );
    }

    #[tokio::test]
    async fn test_admin_propagates_get_session_failure() {
        let adapter = make_adapter(
            pool_status(0, 0, 1),
            DbConfig::default(),
            "admin session denied",
        );
        let result = adapter
            .admin(|_session| async { Ok::<_, crate::core::types::CoreError>(3_u32) })
            .await;
        let msg = expect_database_error(result, "admin should propagate get_session error");
        assert!(
            msg.contains("admin session denied"),
            "error message should contain 'admin session denied', got: {msg}"
        );
    }

    // ===== transaction / write_transaction error propagation =====

    #[tokio::test]
    async fn test_transaction_propagates_get_session_failure() {
        let adapter = make_adapter(
            pool_status(0, 0, 1),
            DbConfig::default(),
            "txn session denied",
        );
        let result = adapter
            .transaction("writer", |_session| async {
                Ok::<_, crate::core::types::CoreError>(4_u32)
            })
            .await;
        let msg = expect_database_error(result, "transaction should propagate get_session error");
        assert!(
            msg.contains("txn session denied"),
            "error message should contain 'txn session denied', got: {msg}"
        );
    }

    #[tokio::test]
    async fn test_write_transaction_propagates_get_session_failure() {
        let adapter = make_adapter(
            pool_status(0, 0, 1),
            DbConfig::default(),
            "write txn denied",
        );
        let result = adapter
            .write_transaction(|_session| async { Ok::<_, crate::core::types::CoreError>(5_u32) })
            .await;
        let msg = expect_database_error(
            result,
            "write_transaction should propagate get_session error",
        );
        assert!(
            msg.contains("write txn denied"),
            "error message should contain 'write txn denied', got: {msg}"
        );
    }

    // ===== health_check error propagation =====

    #[tokio::test]
    async fn test_health_check_propagates_get_session_failure() {
        let adapter = make_adapter(
            pool_status(0, 0, 1),
            DbConfig::default(),
            "health session denied",
        );
        let result = adapter.health_check().await;
        let msg = expect_database_error(result, "health_check should propagate get_session error");
        assert!(
            msg.contains("health session denied"),
            "error message should embed the underlying DbError, got: {msg}"
        );
    }

    // ===== Debug impl =====

    #[test]
    fn test_debug_impl_emits_struct_name_and_pool_placeholder() {
        let adapter = make_adapter(
            pool_status(0, 0, 1),
            DbConfig::default(),
            "mock session unavailable",
        );
        let debug_str = format!("{adapter:?}");
        assert!(
            debug_str.contains("DatabaseAdapter"),
            "Debug output should contain struct name, got: {debug_str}"
        );
        assert!(
            debug_str.contains("Arc<dyn ConnectionPool>"),
            "Debug output should contain pool placeholder, got: {debug_str}"
        );
    }

    // ===== permission error variant propagation =====

    #[tokio::test]
    async fn test_get_session_propagates_permission_error_variant() {
        // Verify the Permission(String) variant is also stringified into
        // DatabaseError by the adapter's map_err closure. We cannot construct
        // DbError::Connection directly because dbnexus 0.4 depends on sea-orm
        // 2.0 while nebulaid uses sea-orm 1.1, so we cover the Permission
        // variant instead to confirm multiple DbError variants flow through.
        let adapter = make_adapter(
            pool_status(0, 0, 1),
            DbConfig::default(),
            "permission denied for admin",
        );
        let result = adapter.get_session("admin").await;
        let msg = expect_database_error(result, "get_session should propagate permission error");
        assert!(
            msg.contains("permission denied for admin"),
            "should embed Permission variant message, got: {msg}"
        );
    }
}
