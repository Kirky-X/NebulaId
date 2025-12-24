use crate::types::error::CoreError;
use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

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
    keys: Arc<RwLock<HashMap<String, ApiKeyData>>>,
}

impl AuthManager {
    pub fn new() -> Self {
        Self {
            keys: Arc::new(RwLock::new(HashMap::new())),
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
        let key_hash = Self::hash_key(&key_id, &key_secret);

        let key_data = ApiKeyData {
            key_id: key_id.clone(),
            key_hash,
            workspace_id,
            enabled: true,
            created_at: Utc::now(),
            expires_at,
            permissions,
        };

        let mut keys = self.keys.write().await;
        keys.insert(key_id.clone(), key_data);

        key_id
    }

    pub async fn validate_key(&self, key_id: &str, key_secret: &str) -> Option<String> {
        let keys = self.keys.read().await;

        if let Some(key_data) = keys.get(key_id) {
            if !key_data.enabled {
                return None;
            }

            if let Some(expires_at) = key_data.expires_at {
                if expires_at < Utc::now() {
                    return None;
                }
            }

            let expected_hash = Self::hash_key(key_id, key_secret);
            if expected_hash == key_data.key_hash {
                return Some(key_data.workspace_id.clone());
            }
        }

        None
    }

    pub async fn revoke_key(&self, key_id: &str) -> bool {
        let mut keys = self.write_lock().await;
        if let Some(key_data) = keys.get_mut(key_id) {
            key_data.enabled = false;
            true
        } else {
            false
        }
    }

    pub async fn list_keys(&self) -> Vec<(String, bool, Option<DateTime<Utc>>)> {
        let keys = self.keys.read().await;
        keys.iter()
            .map(|(id, data)| (id.clone(), data.enabled, data.expires_at))
            .collect()
    }

    fn hash_key(key_id: &str, key_secret: &str) -> String {
        let input = format!("{}:{}", key_id, key_secret);
        let mut hasher = Sha256::new();
        hasher.update(input.as_bytes());
        let result = hasher.finalize();
        STANDARD.encode(result)
    }

    async fn write_lock(&self) -> tokio::sync::RwLockWriteGuard<'_, HashMap<String, ApiKeyData>> {
        self.keys.write().await
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
