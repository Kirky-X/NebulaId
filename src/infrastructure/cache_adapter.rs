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

//! Cache adapter for the oxcache CacheBackend.
//!
//! This adapter provides domain-specific cache operations for Nebula ID,
//! wrapping the generic oxcache CacheBackend trait.

use oxcache::backend::CacheBackend;
use serde::{de::DeserializeOwned, Serialize};
use std::sync::Arc;

/// Cache adapter that wraps an oxcache CacheBackend.
///
/// This adapter provides type-safe serialization/deserialization of
/// cached values, with domain-specific convenience methods.
///
/// # Example
///
/// ```rust,ignore
/// use oxcache::Cache;
/// use crate::core::infrastructure::CacheAdapter;
/// use std::sync::Arc;
///
/// let cache = Arc::new(Cache::memory(10000).await?);
/// let adapter = CacheAdapter::new(cache);
///
/// // Store and retrieve typed values
/// adapter.set("key", &my_data, Some(Duration::from_secs(300))).await?;
/// let data: Option<MyData> = adapter.get("key").await?;
/// ```
#[derive(Clone)]
pub struct CacheAdapter {
    backend: Arc<dyn CacheBackend>,
    key_prefix: String,
}

impl CacheAdapter {
    /// Create a new cache adapter with the given backend.
    ///
    /// # Arguments
    ///
    /// * `backend` - The cache backend from oxcache
    pub fn new(backend: Arc<dyn CacheBackend>) -> Self {
        Self {
            backend,
            key_prefix: "nebula:".to_string(),
        }
    }

    /// Create a new cache adapter with a custom key prefix.
    ///
    /// # Arguments
    ///
    /// * `backend` - The cache backend from oxcache
    /// * `key_prefix` - Prefix for all cache keys
    pub fn with_prefix(backend: Arc<dyn CacheBackend>, key_prefix: impl Into<String>) -> Self {
        Self {
            backend,
            key_prefix: key_prefix.into(),
        }
    }

    /// Get the underlying cache backend.
    pub fn backend(&self) -> &Arc<dyn CacheBackend> {
        &self.backend
    }

    /// Get the key prefix used by this adapter.
    pub fn key_prefix(&self) -> &str {
        &self.key_prefix
    }

    /// Build a prefixed cache key.
    fn build_key(&self, key: &str) -> String {
        format!("{}{}", self.key_prefix, key)
    }

    /// Get a typed value from the cache.
    ///
    /// # Arguments
    ///
    /// * `key` - The cache key (without prefix)
    ///
    /// # Returns
    ///
    /// * `Ok(Some(value))` - Value found and deserialized
    /// * `Ok(None)` - Key not found
    /// * `Err` - Deserialization failed
    pub async fn get<T: DeserializeOwned>(
        &self,
        key: &str,
    ) -> crate::core::types::Result<Option<T>> {
        let full_key = self.build_key(key);
        let bytes = self
            .backend
            .get(&full_key)
            .await
            .map_err(|e| crate::core::types::CoreError::CacheError(e.to_string()))?;

        match bytes {
            Some(data) => {
                let value = serde_json::from_slice(&data).map_err(|e| {
                    crate::core::types::CoreError::CacheError(format!(
                        "Deserialization failed: {}",
                        e
                    ))
                })?;
                Ok(Some(value))
            }
            None => Ok(None),
        }
    }

    /// Set a typed value in the cache.
    ///
    /// # Arguments
    ///
    /// * `key` - The cache key (without prefix)
    /// * `value` - The value to cache
    /// * `ttl` - Optional time-to-live
    pub async fn set<T: Serialize>(
        &self,
        key: &str,
        value: &T,
        ttl: Option<std::time::Duration>,
    ) -> crate::core::types::Result<()> {
        let full_key = self.build_key(key);
        let bytes = serde_json::to_vec(value).map_err(|e| {
            crate::core::types::CoreError::CacheError(format!("Serialization failed: {}", e))
        })?;

        self.backend
            .set(&full_key, bytes, ttl)
            .await
            .map_err(|e| crate::core::types::CoreError::CacheError(e.to_string()))?;

        Ok(())
    }

    /// Delete a value from the cache.
    ///
    /// # Arguments
    ///
    /// * `key` - The cache key (without prefix)
    pub async fn delete(&self, key: &str) -> crate::core::types::Result<()> {
        let full_key = self.build_key(key);
        self.backend
            .delete(&full_key)
            .await
            .map_err(|e| crate::core::types::CoreError::CacheError(e.to_string()))?;

        Ok(())
    }

    /// Check if a key exists in the cache.
    ///
    /// # Arguments
    ///
    /// * `key` - The cache key (without prefix)
    pub async fn exists(&self, key: &str) -> crate::core::types::Result<bool> {
        let full_key = self.build_key(key);
        self.backend
            .exists(&full_key)
            .await
            .map_err(|e| crate::core::types::CoreError::CacheError(e.to_string()))
    }

    /// Get the TTL for a key.
    ///
    /// # Arguments
    ///
    /// * `key` - The cache key (without prefix)
    pub async fn ttl(&self, key: &str) -> crate::core::types::Result<Option<std::time::Duration>> {
        let full_key = self.build_key(key);
        self.backend
            .ttl(&full_key)
            .await
            .map_err(|e| crate::core::types::CoreError::CacheError(e.to_string()))
    }

    /// Set a new TTL for an existing key.
    ///
    /// # Arguments
    ///
    /// * `key` - The cache key (without prefix)
    /// * `ttl` - The new TTL
    ///
    /// # Returns
    ///
    /// * `Ok(true)` - TTL updated successfully
    /// * `Ok(false)` - Key does not exist
    pub async fn expire(
        &self,
        key: &str,
        ttl: std::time::Duration,
    ) -> crate::core::types::Result<bool> {
        let full_key = self.build_key(key);
        self.backend
            .expire(&full_key, ttl)
            .await
            .map_err(|e| crate::core::types::CoreError::CacheError(e.to_string()))
    }

    /// Get raw bytes from the cache.
    ///
    /// # Arguments
    ///
    /// * `key` - The cache key (without prefix)
    pub async fn get_raw(&self, key: &str) -> crate::core::types::Result<Option<Vec<u8>>> {
        let full_key = self.build_key(key);
        self.backend
            .get(&full_key)
            .await
            .map_err(|e| crate::core::types::CoreError::CacheError(e.to_string()))
    }

    /// Set raw bytes in the cache.
    ///
    /// # Arguments
    ///
    /// * `key` - The cache key (without prefix)
    /// * `value` - The raw bytes to cache
    /// * `ttl` - Optional time-to-live
    pub async fn set_raw(
        &self,
        key: &str,
        value: Vec<u8>,
        ttl: Option<std::time::Duration>,
    ) -> crate::core::types::Result<()> {
        let full_key = self.build_key(key);
        self.backend
            .set(&full_key, value, ttl)
            .await
            .map_err(|e| crate::core::types::CoreError::CacheError(e.to_string()))?;

        Ok(())
    }

    /// Clear all entries in the cache.
    pub async fn clear(&self) -> crate::core::types::Result<()> {
        self.backend
            .clear()
            .await
            .map_err(|e| crate::core::types::CoreError::CacheError(e.to_string()))
    }

    /// Check if the cache backend is healthy.
    pub async fn health_check(&self) -> crate::core::types::Result<bool> {
        self.backend
            .health_check()
            .await
            .map_err(|e| crate::core::types::CoreError::CacheError(e.to_string()))
    }

    /// Get the number of entries in the cache.
    pub async fn len(&self) -> crate::core::types::Result<u64> {
        self.backend
            .len()
            .await
            .map_err(|e| crate::core::types::CoreError::CacheError(e.to_string()))
    }

    /// Check if the cache is empty.
    pub async fn is_empty(&self) -> crate::core::types::Result<bool> {
        self.backend
            .is_empty()
            .await
            .map_err(|e| crate::core::types::CoreError::CacheError(e.to_string()))
    }
}

impl std::fmt::Debug for CacheAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CacheAdapter")
            .field("backend", &"Arc<dyn CacheBackend>")
            .field("key_prefix", &self.key_prefix)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    // Tests would require a mock CacheBackend implementation
    // For now, we rely on integration tests with actual backends
}
