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

use crate::types::error::CoreError;
use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use chrono::{DateTime, Utc};
use lru::LruCache;
use parking_lot::Mutex;
use sha2::{Digest, Sha256};
use std::num::NonZeroUsize;
use std::sync::Arc;
use subtle::ConstantTimeEq;

#[derive(Debug, Clone)]
pub struct ApiKeyData {
    pub key_id: String,
    pub key_hash: String,
    pub workspace_id: String,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub permissions: Vec<String>,
    /// Cache entry creation time for TTL
    pub cached_at: DateTime<Utc>,
}

#[derive(Debug)]
pub struct AuthManager {
    /// LRU cache for API keys with size limit
    keys: Arc<Mutex<LruCache<String, ApiKeyData>>>,
    /// Salt for key hashing - prevents rainbow table attacks
    salt: String,
    /// Cache TTL in seconds (default: 5 minutes)
    cache_ttl_seconds: i64,
    /// Maximum cache size (default: 10000)
    max_cache_size: NonZeroUsize,
}

impl Default for AuthManager {
    fn default() -> Self {
        Self::new()
    }
}

impl AuthManager {
    pub fn new() -> Self {
        // Generate or load a secure salt for key hashing
        let salt = std::env::var("NEBULA_API_KEY_SALT").unwrap_or_else(|_err| {
            // Generate a random salt if not provided
            let salt_bytes: [u8; 32] = rand::random();
            hex::encode(salt_bytes)
        });

        // Cache configuration from environment variables
        let cache_ttl_seconds = std::env::var("NEBULA_CACHE_TTL_SECONDS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(300); // Default: 5 minutes

        let max_cache_size = std::env::var("NEBULA_MAX_CACHE_SIZE")
            .ok()
            .and_then(|s| s.parse().ok())
            .and_then(NonZeroUsize::new)
            .unwrap_or(NonZeroUsize::new(10000).unwrap()); // Default: 10000 entries

        Self {
            keys: Arc::new(Mutex::new(LruCache::new(max_cache_size))),
            salt,
            cache_ttl_seconds,
            max_cache_size,
        }
    }

    /// Create AuthManager with custom cache settings
    pub fn with_cache_settings(cache_ttl_seconds: i64, max_cache_size: usize) -> Self {
        let salt = std::env::var("NEBULA_API_KEY_SALT").unwrap_or_else(|_err| {
            let salt_bytes: [u8; 32] = rand::random();
            hex::encode(salt_bytes)
        });

        Self {
            keys: Arc::new(Mutex::new(LruCache::new(
                NonZeroUsize::new(max_cache_size).unwrap_or(NonZeroUsize::new(10000).unwrap()),
            ))),
            salt,
            cache_ttl_seconds,
            max_cache_size: NonZeroUsize::new(max_cache_size)
                .unwrap_or(NonZeroUsize::new(10000).unwrap()),
        }
    }

    pub async fn add_key(
        &self,
        key_id: String,
        key_secret: String,
        workspace_id: String,
        expires_at: Option<DateTime<Utc>>,
        permissions: Vec<String>,
    ) -> String {
        let key_hash = self.hash_key(&key_id, &key_secret);

        let key_data = ApiKeyData {
            key_id: key_id.clone(),
            key_hash,
            workspace_id,
            enabled: true,
            created_at: Utc::now(),
            expires_at,
            permissions,
            cached_at: Utc::now(),
        };

        let mut cache = self.keys.lock();
        cache.put(key_id.clone(), key_data);

        key_id
    }

    pub async fn validate_key(&self, key_id: &str, key_secret: &str) -> Option<String> {
        let mut cache = self.keys.lock();

        // Check if key exists in cache
        if let Some(key_data) = cache.get_mut(key_id) {
            // Check if cache entry has expired
            let cache_age = Utc::now().signed_duration_since(key_data.cached_at);
            if cache_age.num_seconds() > self.cache_ttl_seconds {
                // Cache entry expired, remove it
                cache.pop(key_id);
                return None;
            }

            // Check if key is enabled
            if !key_data.enabled {
                return None;
            }

            // Check if key has expired
            if let Some(expires_at) = key_data.expires_at {
                if expires_at < Utc::now() {
                    // Key expired, remove from cache
                    cache.pop(key_id);
                    return None;
                }
            }

            let expected_hash = self.hash_key(key_id, key_secret);
            // Use constant-time comparison to prevent timing attacks
            if expected_hash
                .as_bytes()
                .ct_eq(key_data.key_hash.as_bytes())
                .into()
            {
                // Update cache timestamp for LRU
                key_data.cached_at = Utc::now();
                return Some(key_data.workspace_id.clone());
            }
        }

        None
    }

    pub async fn revoke_key(&self, key_id: &str) -> bool {
        let mut cache = self.keys.lock();
        if let Some(key_data) = cache.get_mut(key_id) {
            key_data.enabled = false;
            true
        } else {
            false
        }
    }

    pub async fn list_keys(&self) -> Vec<(String, bool, Option<DateTime<Utc>>)> {
        let cache = self.keys.lock();
        cache
            .iter()
            .map(|(key_id, key_data)| (key_id.clone(), key_data.enabled, key_data.expires_at))
            .collect()
    }

    /// Remove expired keys from cache
    pub async fn cleanup_expired_keys(&self) -> usize {
        let mut cache = self.keys.lock();
        let now = Utc::now();
        let mut removed = 0;

        // Collect expired keys
        let expired_keys: Vec<String> = cache
            .iter()
            .filter(|(_key_id, key_data)| {
                // Check cache TTL
                let cache_age = now.signed_duration_since(key_data.cached_at);
                if cache_age.num_seconds() > self.cache_ttl_seconds {
                    return true;
                }

                // Check key expiration
                if let Some(expires_at) = key_data.expires_at {
                    if expires_at < now {
                        return true;
                    }
                }

                false
            })
            .map(|(key_id, _key_data)| key_id.clone())
            .collect();

        // Remove expired keys
        for key_id in expired_keys {
            cache.pop(&key_id);
            removed += 1;
        }

        removed
    }

    /// Get cache statistics
    pub async fn cache_stats(&self) -> CacheStats {
        let cache = self.keys.lock();
        CacheStats {
            current_size: cache.len(),
            max_size: self.max_cache_size.get(),
            ttl_seconds: self.cache_ttl_seconds,
        }
    }

    /// Clear all cached keys
    pub async fn clear_cache(&self) {
        let mut cache = self.keys.lock();
        cache.clear();
    }

    fn hash_key(&self, key_id: &str, key_secret: &str) -> String {
        // Use salt to prevent rainbow table attacks
        let input = format!("{}:{}:{}", self.salt, key_id, key_secret);
        let mut hasher = Sha256::new();
        hasher.update(input.as_bytes());
        let result = hasher.finalize();
        STANDARD.encode(result)
    }
}

#[derive(Debug, Clone)]
pub struct CacheStats {
    pub current_size: usize,
    pub max_size: usize,
    pub ttl_seconds: i64,
}

#[async_trait]
pub trait Authenticator: Send + Sync {
    async fn authenticate(&self, key_id: &str, key_secret: &str) -> Result<String, CoreError>;
}

#[async_trait]
impl Authenticator for AuthManager {
    async fn authenticate(&self, key_id: &str, key_secret: &str) -> Result<String, CoreError> {
        self.validate_key(key_id, key_secret)
            .await
            .ok_or_else(|| CoreError::AuthenticationError("Invalid API key".to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_api_key_creation_and_validation() {
        let manager = AuthManager::new();

        let key_id = manager
            .add_key(
                "test-key-id".to_string(),
                "test-key-secret".to_string(),
                "workspace-1".to_string(),
                None,
                vec!["read".to_string(), "write".to_string()],
            )
            .await;

        assert!(!key_id.is_empty());

        let workspace_id = manager.validate_key(&key_id, "test-key-secret").await;
        assert_eq!(workspace_id, Some("workspace-1".to_string()));

        let invalid = manager.validate_key(&key_id, "wrong-secret").await;
        assert_eq!(invalid, None);
    }

    #[tokio::test]
    async fn test_key_revocation() {
        let manager = AuthManager::new();

        let key_id = manager
            .add_key(
                "revokable-key".to_string(),
                "secret".to_string(),
                "workspace-1".to_string(),
                None,
                vec![],
            )
            .await;

        assert!(manager.validate_key(&key_id, "secret").await.is_some());

        manager.revoke_key(&key_id).await;

        assert!(manager.validate_key(&key_id, "secret").await.is_none());
    }

    #[tokio::test]
    async fn test_cache_ttl() {
        // Create manager with very short TTL (1 second)
        let manager = AuthManager::with_cache_settings(1, 100);

        let key_id = manager
            .add_key(
                "ttl-key".to_string(),
                "secret".to_string(),
                "workspace-1".to_string(),
                None,
                vec![],
            )
            .await;

        // Key should be valid immediately
        assert!(manager.validate_key(&key_id, "secret").await.is_some());

        // Wait for TTL to expire
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

        // Key should be invalid after TTL expiration
        assert!(manager.validate_key(&key_id, "secret").await.is_none());
    }

    #[tokio::test]
    async fn test_cache_size_limit() {
        // Create manager with small cache size (3)
        let manager = AuthManager::with_cache_settings(300, 3);

        // Add 5 keys
        let mut key_ids = Vec::new();
        for i in 0..5 {
            let key_id = manager
                .add_key(
                    format!("key-{}", i),
                    "secret".to_string(),
                    format!("workspace-{}", i),
                    None,
                    vec![],
                )
                .await;
            key_ids.push(key_id);
        }

        // Only the last 3 keys should be in cache
        assert!(manager.validate_key(&key_ids[2], "secret").await.is_some());
        assert!(manager.validate_key(&key_ids[3], "secret").await.is_some());
        assert!(manager.validate_key(&key_ids[4], "secret").await.is_some());

        // First 2 keys should be evicted
        assert!(manager.validate_key(&key_ids[0], "secret").await.is_none());
        assert!(manager.validate_key(&key_ids[1], "secret").await.is_none());
    }

    #[tokio::test]
    async fn test_cleanup_expired_keys() {
        let manager = AuthManager::with_cache_settings(1, 100);

        // Add a key
        let key_id = manager
            .add_key(
                "cleanup-key".to_string(),
                "secret".to_string(),
                "workspace-1".to_string(),
                None,
                vec![],
            )
            .await;

        // Wait for TTL to expire
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

        // Clean up expired keys
        let removed = manager.cleanup_expired_keys().await;
        assert_eq!(removed, 1);

        // Key should no longer be valid
        assert!(manager.validate_key(&key_id, "secret").await.is_none());
    }

    #[tokio::test]
    async fn test_cache_stats() {
        let manager = AuthManager::new();

        // Add some keys
        for i in 0..5 {
            manager
                .add_key(
                    format!("key-{}", i),
                    "secret".to_string(),
                    format!("workspace-{}", i),
                    None,
                    vec![],
                )
                .await;
        }

        let stats = manager.cache_stats().await;
        assert_eq!(stats.current_size, 5);
        assert_eq!(stats.max_size, 10000);
        assert_eq!(stats.ttl_seconds, 300);
    }

    #[tokio::test]
    async fn test_clear_cache() {
        let manager = AuthManager::new();

        // Add some keys
        for i in 0..5 {
            manager
                .add_key(
                    format!("key-{}", i),
                    "secret".to_string(),
                    format!("workspace-{}", i),
                    None,
                    vec![],
                )
                .await;
        }

        // Clear cache
        manager.clear_cache().await;

        // All keys should be removed
        let stats = manager.cache_stats().await;
        assert_eq!(stats.current_size, 0);
    }
}
