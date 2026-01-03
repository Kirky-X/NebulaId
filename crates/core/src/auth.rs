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
use dashmap::DashMap;
use sha2::{Digest, Sha256};
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
}

#[derive(Debug)]
pub struct AuthManager {
    keys: Arc<DashMap<String, ApiKeyData>>,
    /// Salt for key hashing - prevents rainbow table attacks
    salt: String,
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

        Self {
            keys: Arc::new(DashMap::new()),
            salt,
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
        };

        self.keys.insert(key_id.clone(), key_data);

        key_id
    }

    pub async fn validate_key(&self, key_id: &str, key_secret: &str) -> Option<String> {
        if let Some(key_data) = self.keys.get(key_id) {
            let key_data = key_data.value();

            if !key_data.enabled {
                return None;
            }

            if let Some(expires_at) = key_data.expires_at {
                if expires_at < Utc::now() {
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
                return Some(key_data.workspace_id.clone());
            }
        }

        None
    }

    pub async fn revoke_key(&self, key_id: &str) -> bool {
        if let Some(mut key_data) = self.keys.get_mut(key_id) {
            key_data.enabled = false;
            true
        } else {
            false
        }
    }

    pub async fn list_keys(&self) -> Vec<(String, bool, Option<DateTime<Utc>>)> {
        self.keys
            .iter()
            .map(|entry| (entry.key().clone(), entry.enabled, entry.expires_at))
            .collect()
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
}
