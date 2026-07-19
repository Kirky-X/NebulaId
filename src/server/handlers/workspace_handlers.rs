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

//! Workspace / Group management handlers (rule 25 split).

use super::helpers::{map_db_error, map_uuid_error};
use crate::core::{CoreError, Result};
use crate::server::models::{
    naive_to_rfc3339, ApiKeyResponse, ApiKeyWithSecretResponse, CreateGroupRequest,
    CreateWorkspaceRequest, GroupListResponse, GroupResponse, UserApiKeyInfo,
    WorkspaceListResponse, WorkspaceResponse,
};

impl super::ApiHandlers {
    /// Create a new Workspace (auto-provisions a user API key).
    pub async fn create_workspace(&self, req: CreateWorkspaceRequest) -> Result<WorkspaceResponse> {
        let workspace: WorkspaceResponse = self
            .config_service
            .create_workspace(req.clone())
            .await
            .map_err(map_db_error)?;

        let repo = self
            .api_key_repo
            .as_ref()
            .ok_or_else(|| CoreError::NotFound("API key repository not configured".to_string()))?;

        let workspace_uuid = uuid::Uuid::parse_str(&workspace.id).map_err(map_uuid_error)?;

        let user_key_request = crate::core::database::CreateApiKeyRequest {
            workspace_id: Some(workspace_uuid),
            name: format!("{}-user-key", workspace.name),
            description: Some(format!("User API key for workspace: {}", workspace.name)),
            role: crate::core::database::ApiKeyRole::User,
            rate_limit: Some(10000),
            expires_at: None,
            key_secret: None,
            key_id: None,
        };

        let user_key = repo
            .create_api_key(&user_key_request)
            .await
            .map_err(map_db_error)?;

        Ok(WorkspaceResponse {
            id: workspace.id,
            name: workspace.name,
            description: workspace.description,
            status: workspace.status,
            max_groups: workspace.max_groups,
            max_biz_tags: workspace.max_biz_tags,
            created_at: workspace.created_at,
            updated_at: workspace.updated_at,
            user_api_key: Some(UserApiKeyInfo {
                key_id: user_key.key.key_id,
                key_secret: user_key.key_secret,
                key_prefix: user_key.key.key_prefix,
            }),
        })
    }

    /// Regenerate User API Key for a Workspace.
    pub async fn regenerate_user_api_key(
        &self,
        workspace_name: &str,
    ) -> Result<ApiKeyWithSecretResponse> {
        let workspace = self
            .config_service
            .get_workspace(workspace_name)
            .await
            .map_err(map_db_error)?
            .ok_or_else(|| {
                CoreError::NotFound(format!("Workspace '{}' not found", workspace_name))
            })?;

        let repo = self
            .api_key_repo
            .as_ref()
            .ok_or_else(|| CoreError::NotFound("API key repository not configured".to_string()))?;

        let workspace_uuid = uuid::Uuid::parse_str(&workspace.id).map_err(map_uuid_error)?;

        let existing_keys = repo
            .list_api_keys(workspace_uuid, Some(1000), Some(0))
            .await
            .map_err(map_db_error)?;

        for key in existing_keys {
            if key.role == crate::core::database::ApiKeyRole::User {
                repo.delete_api_key(key.id).await.map_err(map_db_error)?;
            }
        }

        let user_key_request = crate::core::database::CreateApiKeyRequest {
            workspace_id: Some(workspace_uuid),
            name: format!("{}-user-key", workspace.name),
            description: Some(format!("User API key for workspace: {}", workspace.name)),
            role: crate::core::database::ApiKeyRole::User,
            rate_limit: Some(10000),
            expires_at: None,
            key_secret: None,
            key_id: None,
        };

        let user_key = repo
            .create_api_key(&user_key_request)
            .await
            .map_err(map_db_error)?;

        Ok(ApiKeyWithSecretResponse {
            key: ApiKeyResponse {
                id: user_key.key.id.to_string(),
                key_id: user_key.key.key_id,
                key_prefix: user_key.key.key_prefix,
                name: user_key.key.name,
                description: user_key.key.description,
                role: match user_key.key.role {
                    crate::core::database::ApiKeyRole::Admin => "admin".to_string(),
                    crate::core::database::ApiKeyRole::User => "user".to_string(),
                    // LOW-1 修复：Anonymous 不会被持久化到数据库，这里只是穷尽匹配。
                    // 如果运行到这里说明数据库被外部直接写入了 Anonymous，返回错误标记。
                    crate::core::database::ApiKeyRole::Anonymous => {
                        return Err(crate::core::CoreError::InternalError(
                            "Anonymous role should not be persisted in database".to_string(),
                        ))
                    }
                },
                rate_limit: user_key.key.rate_limit,
                enabled: user_key.key.enabled,
                expires_at: user_key.key.expires_at.map(naive_to_rfc3339),
                created_at: naive_to_rfc3339(user_key.key.created_at),
            },
            key_secret: user_key.key_secret,
        })
    }

    /// List all Workspaces.
    pub async fn list_workspaces(&self) -> Result<WorkspaceListResponse> {
        // M5 修复：直接传播 CoreError，避免 `e.to_string()` 丢失类型信息
        // 导致 HTTP 响应全部变成 500。helpers.rs 的错误转换层会根据
        // CoreError 变体映射到正确的 HTTP 状态码。
        self.config_service.list_workspaces().await
    }

    /// Get Workspace by name.
    pub async fn get_workspace(&self, name: &str) -> Result<Option<WorkspaceResponse>> {
        self.config_service.get_workspace(name).await
    }

    /// Create a new Group.
    pub async fn create_group(&self, req: CreateGroupRequest) -> Result<GroupResponse> {
        self.config_service.create_group(req).await
    }

    /// List all Groups for a workspace.
    pub async fn list_groups(&self, workspace: &str) -> Result<GroupListResponse> {
        self.config_service.list_groups(workspace).await
    }
}
