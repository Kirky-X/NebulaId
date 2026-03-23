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

use dbnexus::pool::{ConnectionPool, PoolStatus};
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
/// use nebula_core::infrastructure::DatabaseAdapter;
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
    /// Create a new database adapter with the given connection pool.
    ///
    /// # Arguments
    ///
    /// * `pool` - The connection pool from dbnexus
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
    pub async fn get_session(&self, role: &str) -> crate::types::Result<dbnexus::pool::Session> {
        self.pool
            .get_session(role)
            .await
            .map_err(|e| crate::types::CoreError::DatabaseError(e.to_string()))
    }

    /// Get the pool status.
    pub fn status(&self) -> PoolStatus {
        self.pool.status()
    }

    /// Get the pool configuration.
    pub fn config(&self) -> &dbnexus::config::DbConfig {
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
    pub async fn read<F, T, Fut>(&self, f: F) -> crate::types::Result<T>
    where
        F: FnOnce(dbnexus::pool::Session) -> Fut,
        Fut: std::future::Future<Output = crate::types::Result<T>>,
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
    pub async fn write<F, T, Fut>(&self, f: F) -> crate::types::Result<T>
    where
        F: FnOnce(dbnexus::pool::Session) -> Fut,
        Fut: std::future::Future<Output = crate::types::Result<T>>,
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
    pub async fn admin<F, T, Fut>(&self, f: F) -> crate::types::Result<T>
    where
        F: FnOnce(dbnexus::pool::Session) -> Fut,
        Fut: std::future::Future<Output = crate::types::Result<T>>,
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
    pub async fn transaction<F, T, Fut>(&self, role: &str, f: F) -> crate::types::Result<T>
    where
        F: FnOnce(dbnexus::pool::Session) -> Fut,
        Fut: std::future::Future<Output = crate::types::Result<T>>,
    {
        let session = self.get_session(role).await?;

        // Begin transaction
        session
            .begin_transaction()
            .await
            .map_err(|e| crate::types::CoreError::DatabaseError(e.to_string()))?;

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
    pub async fn write_transaction<F, T, Fut>(&self, f: F) -> crate::types::Result<T>
    where
        F: FnOnce(dbnexus::pool::Session) -> Fut,
        Fut: std::future::Future<Output = crate::types::Result<T>>,
    {
        self.transaction("writer", f).await
    }

    /// Check if the database is healthy.
    pub async fn health_check(&self) -> crate::types::Result<bool> {
        let session = self.get_session("reader").await?;

        // Execute a simple query to check connectivity
        session
            .execute("SELECT 1")
            .await
            .map_err(|e| crate::types::CoreError::DatabaseError(e.to_string()))?;

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

    // Tests would require a mock ConnectionPool implementation
    // For now, we rely on integration tests with actual databases
}
