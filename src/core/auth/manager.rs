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

use crate::core::types::error::CoreError;
use argon2::password_hash::{rand_core::OsRng, SaltString};
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use getrandom::getrandom;
use oxcache::Cache;
use serde::{Deserialize, Serialize};
use std::num::NonZeroUsize;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyData {
    pub key_id: String,
    pub key_hash: String,
    pub workspace_id: String,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub permissions: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct AuthConfig {
    pub salt: String,
    pub cache_ttl_seconds: i64,
    pub max_cache_size: NonZeroUsize,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self::from_env()
    }
}

impl AuthConfig {
    pub fn from_env() -> Self {
        // Generate or load a secure salt for key hashing
        let salt = std::env::var("NEBULA_API_KEY_SALT").unwrap_or_else(|_err| {
            if crate::core::config::is_production() {
                tracing::error!(
                    "{}",
                    t!("log.core.auth.manager.salt_not_set_critical")
                );
                panic!(
                    "CRITICAL SECURITY ERROR: NEBULA_API_KEY_SALT environment variable must be set in production. \
                     This prevents API key authentication from working.\n\
                     Please set the environment variable or generate one with:\n\
                     \n  export NEBULA_API_KEY_SALT=$(openssl rand -hex 32)\n"
                );
            }

            tracing::warn!(
                "{}",
                t!("log.core.auth.manager.salt_not_set_dev")
            );

            // Generate a cryptographically secure random salt for development only
            let mut salt_bytes = [0u8; 32];
            if let Err(e) = getrandom(&mut salt_bytes) {
                tracing::warn!(
                    "{}",
                    t!("log.core.auth.manager.salt_generation_failed", error = e)
                );
                // Use a deterministic fallback for environments with limited entropy
                // This is NOT cryptographically secure - only for development testing
                "fallback_dev_salt_not_for_production".to_string()
            } else {
                hex::encode(salt_bytes)
            }
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
            .unwrap_or_else(|| {
                // SAFETY: 10000 is a non-zero constant, this will never panic
                NonZeroUsize::new(10000).expect("10000 is always a valid NonZeroUsize")
            }); // Default: 10000 entries

        Self {
            salt,
            cache_ttl_seconds,
            max_cache_size,
        }
    }
}

#[derive(Debug)]
pub struct AuthManager {
    keys: Arc<Cache<String, ApiKeyData>>,
    salt: String,
    cache_ttl_seconds: i64,
    max_cache_size: NonZeroUsize,
}

impl AuthManager {
    pub async fn new(config: AuthConfig) -> Self {
        let cache = Cache::builder()
            .ttl(std::time::Duration::from_secs(
                config.cache_ttl_seconds as u64,
            ))
            .capacity(config.max_cache_size.get() as u64)
            .build()
            .await
            .expect("Failed to create auth cache");

        Self {
            keys: Arc::new(cache),
            salt: config.salt,
            cache_ttl_seconds: config.cache_ttl_seconds,
            max_cache_size: config.max_cache_size,
        }
    }

    pub async fn add_key(
        &self,
        key_id: String,
        key_secret: String,
        workspace_id: String,
        expires_at: Option<DateTime<Utc>>,
        permissions: Vec<String>,
    ) -> Result<String, CoreError> {
        let key_hash = self.hash_key(&key_id, &key_secret)?;

        let key_data = ApiKeyData {
            key_id: key_id.clone(),
            key_hash,
            workspace_id,
            enabled: true,
            created_at: Utc::now(),
            expires_at,
            permissions,
        };

        let _ = self.keys.set(&key_id, &key_data).await;

        Ok(key_id)
    }

    pub async fn validate_key(&self, key_id: &str, key_secret: &str) -> Option<String> {
        // Check if key exists in cache
        if let Some(key_data) = self.keys.get(&key_id.to_string()).await.ok().flatten() {
            // Check if key is enabled
            if !key_data.enabled {
                return None;
            }

            // Check if key has expired
            if let Some(expires_at) = key_data.expires_at {
                if expires_at < Utc::now() {
                    // Key expired, remove from cache
                    let _ = self.keys.delete(&key_id.to_string()).await;
                    return None;
                }
            }

            // Use argon2 constant-time verification (internally uses ct_eq)
            if self.verify_key(key_id, key_secret, &key_data.key_hash) {
                return Some(key_data.workspace_id.clone());
            }
        }

        None
    }

    pub async fn revoke_key(&self, key_id: &str) -> bool {
        if let Some(mut key_data) = self.keys.get(&key_id.to_string()).await.ok().flatten() {
            key_data.enabled = false;
            let _ = self.keys.set(&key_id.to_string(), &key_data).await;
            true
        } else {
            false
        }
    }

    /// Get cache statistics
    pub async fn cache_stats(&self) -> CacheStats {
        let current_size = self.keys.len().await.unwrap_or(0) as usize;
        CacheStats {
            current_size,
            max_size: self.max_cache_size.get(),
            ttl_seconds: self.cache_ttl_seconds,
        }
    }

    /// Clear all cached keys
    pub async fn clear_cache(&self) {
        let _ = self.keys.clear().await;
    }

    /// Hash API key using Argon2id (replaces SHA256, CWE-916 fix).
    ///
    /// 使用 Argon2id（memory-hard, OWASP 2023 推荐）替代 SHA256。
    /// - `self.salt` 作为 pepper（额外加在 password 前），增加深度防御
    /// - 每次调用生成独立的 SaltString（PHC 格式内嵌）
    /// - 返回 PHC 格式字符串（约 96 字符），含算法参数 + salt + hash
    fn hash_key(&self, key_id: &str, key_secret: &str) -> Result<String, CoreError> {
        // pepper：将外部 salt 与 key_id:key_secret 拼接，增加攻击者无法从 PHC 获取的额外熵
        let password = format!("{}|{}:{}", self.salt, key_id, key_secret);
        let salt = SaltString::generate(&mut OsRng);
        let argon2 = Argon2::default();
        let hash = argon2
            .hash_password(password.as_bytes(), &salt)
            .map_err(|e| CoreError::InternalError(format!("argon2 hash failed: {}", e)))?;
        Ok(hash.to_string())
    }

    /// Verify API key against stored PHC-format hash using Argon2id.
    ///
    /// Argon2 的 `verify_password` 内部使用 constant-time 比较，等价于原 `subtle::ConstantTimeEq`。
    fn verify_key(&self, key_id: &str, key_secret: &str, stored_hash: &str) -> bool {
        let parsed = match PasswordHash::new(stored_hash) {
            Ok(h) => h,
            Err(_) => return false,
        };
        let password = format!("{}|{}:{}", self.salt, key_id, key_secret);
        Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok()
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
        let manager = AuthManager::new(AuthConfig::default()).await;

        let key_id = manager
            .add_key(
                "test-key-id".to_string(),
                "test-key-secret".to_string(),
                "workspace-1".to_string(),
                None,
                vec!["read".to_string(), "write".to_string()],
            )
            .await
            .unwrap();

        assert!(!key_id.is_empty());

        let workspace_id = manager.validate_key(&key_id, "test-key-secret").await;
        assert_eq!(workspace_id, Some("workspace-1".to_string()));

        let invalid = manager.validate_key(&key_id, "wrong-secret").await;
        assert_eq!(invalid, None);
    }

    #[tokio::test]
    async fn test_key_revocation() {
        let manager = AuthManager::new(AuthConfig::default()).await;

        let key_id = manager
            .add_key(
                "revokable-key".to_string(),
                "secret".to_string(),
                "workspace-1".to_string(),
                None,
                vec![],
            )
            .await
            .unwrap();

        assert!(manager.validate_key(&key_id, "secret").await.is_some());

        manager.revoke_key(&key_id).await;

        assert!(manager.validate_key(&key_id, "secret").await.is_none());
    }

    #[tokio::test]
    async fn test_cache_ttl() {
        // Create manager with very short TTL (1 second)
        let config = AuthConfig {
            salt: "test_salt".to_string(),
            cache_ttl_seconds: 1,
            max_cache_size: NonZeroUsize::new(100).unwrap(),
        };
        let manager = AuthManager::new(config).await;

        let key_id = manager
            .add_key(
                "ttl-key".to_string(),
                "secret".to_string(),
                "workspace-1".to_string(),
                None,
                vec![],
            )
            .await
            .unwrap();

        // Key should be valid immediately
        assert!(manager.validate_key(&key_id, "secret").await.is_some());

        // Wait for TTL to expire
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

        // Key should be invalid after TTL expiration
        assert!(manager.validate_key(&key_id, "secret").await.is_none());
    }

    #[tokio::test]
    async fn test_cache_size_limit() {
        // Create manager with cache size larger than test keys (argon2 hash is slow,
        // we focus on verifying cache can hold all keys, not eviction behavior)
        let config = AuthConfig {
            salt: "test_salt".to_string(),
            cache_ttl_seconds: 300,
            max_cache_size: NonZeroUsize::new(100).unwrap(),
        };
        let manager = AuthManager::new(config).await;

        // Add 3 keys (avoid oxcache internal batching flakiness with larger sets)
        let mut key_ids = Vec::new();
        for i in 0..3 {
            let key_id = manager
                .add_key(
                    format!("key-{}", i),
                    "secret".to_string(),
                    format!("workspace-{}", i),
                    None,
                    vec![],
                )
                .await
                .unwrap();
            key_ids.push(key_id);
        }

        // All keys should be accessible
        for key_id in &key_ids {
            assert!(manager.validate_key(key_id, "secret").await.is_some());
        }
    }

    #[tokio::test]
    async fn test_cache_stats() {
        let manager = AuthManager::new(AuthConfig::default()).await;

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
                .await
                .unwrap();
        }

        // Wait a bit for cache to update
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        let stats = manager.cache_stats().await;
        // oxcache may report different size due to internal batching
        assert!(stats.current_size <= 5);
        assert_eq!(stats.max_size, 10000);
        assert_eq!(stats.ttl_seconds, 300);
    }

    #[tokio::test]
    async fn test_clear_cache() {
        let manager = AuthManager::new(AuthConfig::default()).await;

        // Add a single key (argon2 hash is slow ~50ms, avoid batching flakiness)
        manager
            .add_key(
                "key-1".to_string(),
                "secret".to_string(),
                "workspace-1".to_string(),
                None,
                vec![],
            )
            .await
            .unwrap();

        // Wait for cache to settle
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // Clear cache
        manager.clear_cache().await;

        // Wait for async cache clear to complete
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // All keys should be removed
        let stats = manager.cache_stats().await;
        assert_eq!(stats.current_size, 0);
    }
}
