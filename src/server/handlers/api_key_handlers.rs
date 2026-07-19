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

//! API Key management handlers + `KeyRotationHandle` (rule 25 split).

use super::helpers::map_db_error;
use crate::core::database::{ApiKeyRole, CreateApiKeyRequest as CoreCreateApiKeyRequest};
use crate::core::{CoreError, Result};
use crate::server::models::{
    naive_to_rfc3339, ApiKeyListResponse, ApiKeyResponse, ApiKeyWithSecretResponse,
    CreateApiKeyRequest, RevokeApiKeyResponse,
};

/// Handle for managing the key rotation background task.
#[derive(Clone, Debug)]
pub struct KeyRotationHandle {
    pub(super) shutdown_tx: tokio::sync::watch::Sender<bool>,
}

impl KeyRotationHandle {
    /// Signal the key rotation task to stop.
    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(true);
    }
}

impl super::ApiHandlers {
    /// Create a new API Key (admin only).
    pub async fn create_api_key(
        &self,
        workspace_id: Option<uuid::Uuid>,
        req: CreateApiKeyRequest,
    ) -> Result<ApiKeyWithSecretResponse> {
        let repo = self
            .api_key_repo
            .as_ref()
            .ok_or_else(|| CoreError::NotFound("API key repository not configured".to_string()))?;

        let role = match req.role.as_deref() {
            Some("admin") => ApiKeyRole::Admin,
            Some("user") | None => ApiKeyRole::User,
            Some(r) => {
                return Err(CoreError::AuthenticationError(format!(
                    "Invalid role: {}",
                    r
                )))
            }
        };

        if role == ApiKeyRole::Admin {
            let existing_keys = repo
                .list_api_keys(uuid::Uuid::nil(), Some(1000), Some(0))
                .await
                .map_err(map_db_error)?;

            let has_admin = existing_keys
                .iter()
                .any(|k| k.role == crate::core::database::ApiKeyRole::Admin);

            if has_admin {
                tracing::warn!(
                    event = "admin_key_creation",
                    workspace_id = ?workspace_id,
                    "{}",
                    t!("log.server.handlers.api_key_handlers.creating_additional_admin_key")
                );
            }
        }

        if role == ApiKeyRole::User {
            let ws_id = workspace_id.ok_or_else(|| {
                CoreError::InvalidInput("workspace_id is required for user keys".to_string())
            })?;

            let existing_keys = repo
                .list_api_keys(ws_id, Some(1000), Some(0))
                .await
                .map_err(map_db_error)?;

            let has_user_key = existing_keys
                .iter()
                .any(|k| k.role == crate::core::database::ApiKeyRole::User);

            if has_user_key {
                return Err(CoreError::AuthenticationError(format!(
                    "User API key already exists for workspace: {}",
                    ws_id
                )));
            }
        }

        let expires_at = match &req.expires_at {
            Some(ts) => Some(
                chrono::DateTime::parse_from_rfc3339(ts)
                    .map_err(|_| {
                        CoreError::InvalidIdFormat("Invalid expires_at format".to_string())
                    })?
                    .with_timezone(&chrono::Utc)
                    .naive_utc(),
            ),
            None => None,
        };

        let core_req = CoreCreateApiKeyRequest {
            workspace_id,
            name: req.name,
            description: req.description,
            role,
            rate_limit: req.rate_limit,
            expires_at,
            key_secret: None,
            key_id: None,
        };

        let key_with_secret = repo.create_api_key(&core_req).await.map_err(map_db_error)?;

        Ok(ApiKeyWithSecretResponse {
            key: ApiKeyResponse {
                id: key_with_secret.key.id.to_string(),
                key_id: key_with_secret.key.key_id,
                key_prefix: key_with_secret.key.key_prefix,
                name: key_with_secret.key.name,
                description: key_with_secret.key.description,
                role: key_with_secret.key.role.to_string(),
                rate_limit: key_with_secret.key.rate_limit,
                enabled: key_with_secret.key.enabled,
                expires_at: key_with_secret.key.expires_at.map(naive_to_rfc3339),
                created_at: naive_to_rfc3339(key_with_secret.key.created_at),
            },
            key_secret: key_with_secret.key_secret,
        })
    }

    /// List API Keys for a workspace (admin only).
    pub async fn list_api_keys(
        &self,
        workspace_id: uuid::Uuid,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> Result<ApiKeyListResponse> {
        let repo = self
            .api_key_repo
            .as_ref()
            .ok_or_else(|| CoreError::NotFound("API key repository not configured".to_string()))?;

        let keys = repo
            .list_api_keys(workspace_id, limit, offset)
            .await
            .map_err(map_db_error)?;

        let responses: Vec<ApiKeyResponse> = keys
            .into_iter()
            .map(|k| ApiKeyResponse {
                id: k.id.to_string(),
                key_id: k.key_id,
                key_prefix: k.key_prefix,
                name: k.name,
                description: k.description,
                role: k.role.to_string(),
                rate_limit: k.rate_limit,
                enabled: k.enabled,
                expires_at: k.expires_at.map(naive_to_rfc3339),
                created_at: naive_to_rfc3339(k.created_at),
            })
            .collect();

        let total = repo
            .count_api_keys(workspace_id)
            .await
            .map_err(map_db_error)?;

        Ok(ApiKeyListResponse {
            api_keys: responses,
            total,
        })
    }

    /// Revoke (delete) an API Key (admin only).
    pub async fn revoke_api_key(&self, id: uuid::Uuid) -> Result<RevokeApiKeyResponse> {
        let repo = self
            .api_key_repo
            .as_ref()
            .ok_or_else(|| CoreError::NotFound("API key repository not configured".to_string()))?;

        let key_info = repo
            .get_api_key_by_id(&id.to_string())
            .await
            .map_err(map_db_error)?;

        if let Some(key) = key_info {
            if key.role == crate::core::database::ApiKeyRole::Admin {
                let existing_keys = repo
                    .list_api_keys(uuid::Uuid::nil(), Some(1000), Some(0))
                    .await
                    .map_err(map_db_error)?;

                let admin_count = existing_keys
                    .iter()
                    .filter(|k| k.role == crate::core::database::ApiKeyRole::Admin)
                    .count();

                if admin_count <= 1 {
                    return Err(CoreError::AuthenticationError(
                        "Cannot revoke the last admin key".to_string(),
                    ));
                }
            }
        }

        repo.delete_api_key(id).await.map_err(map_db_error)?;

        Ok(RevokeApiKeyResponse {
            success: true,
            message: format!("API key {} revoked successfully", id),
        })
    }

    /// Rotate an API Key (generate new secret, keep old key active during grace period).
    pub async fn rotate_api_key(&self, key_id: &str) -> Result<ApiKeyWithSecretResponse> {
        use crate::server::models::ApiKeyResponse;

        if key_id.is_empty() {
            return Err(CoreError::InvalidInput(
                "key_id cannot be empty".to_string(),
            ));
        }

        let repo = self
            .api_key_repo
            .as_ref()
            .ok_or_else(|| CoreError::NotFound("API key repository not configured".to_string()))?;

        const GRACE_PERIOD_SECONDS: u64 = 7 * 24 * 60 * 60;

        let key_with_secret = repo
            .rotate_api_key(key_id, GRACE_PERIOD_SECONDS)
            .await
            .map_err(map_db_error)?;

        tracing::info!(event = "api_key_rotated", key_id = key_id);

        Ok(ApiKeyWithSecretResponse {
            key: ApiKeyResponse {
                id: key_with_secret.key.id.to_string(),
                key_id: key_with_secret.key.key_id,
                key_prefix: key_with_secret.key.key_prefix,
                name: key_with_secret.key.name,
                description: key_with_secret.key.description,
                role: key_with_secret.key.role.to_string(),
                rate_limit: key_with_secret.key.rate_limit,
                enabled: key_with_secret.key.enabled,
                expires_at: key_with_secret.key.expires_at.map(naive_to_rfc3339),
                created_at: naive_to_rfc3339(key_with_secret.key.created_at),
            },
            key_secret: key_with_secret.key_secret,
        })
    }
}
