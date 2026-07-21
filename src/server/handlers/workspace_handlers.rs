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

        let repo = self.api_key_repo.as_ref().ok_or_else(|| {
            CoreError::NotFound(
                t!("api.error.handlers.workspace_handlers.api_key_repo_not_configured").to_string(),
            )
        })?;

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
                CoreError::NotFound(
                    t!(
                        "api.error.handlers.workspace_handlers.not_found",
                        name = workspace_name
                    )
                    .to_string(),
                )
            })?;

        let repo = self.api_key_repo.as_ref().ok_or_else(|| {
            CoreError::NotFound(
                t!("api.error.handlers.workspace_handlers.api_key_repo_not_configured").to_string(),
            )
        })?;

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

#[cfg(test)]
mod tests {
    use crate::core::database::{
        ApiKeyInfo, ApiKeyRepository, ApiKeyResponse as CoreApiKeyResponse, ApiKeyRole,
        ApiKeyWithSecret, BizTag,
    };
    use crate::core::types::AlgorithmType;
    use crate::core::{CoreError, Result};
    use crate::server::config::management::ConfigManagementService;
    use crate::server::handlers::mock_generator::MockIdGenerator;
    use crate::server::handlers::ApiHandlers;
    use crate::server::models::*;
    use async_trait::async_trait;
    use mockall::mock;
    use std::sync::Arc;
    use uuid::Uuid;

    mock! {
        pub WorkspaceTestService {}
        #[async_trait]
        impl ConfigManagementService for WorkspaceTestService {
            fn get_config(&self) -> ConfigResponse;
            fn get_secure_config(&self) -> SecureConfigResponse;
            fn get_batch_max_size(&self) -> u32;
            async fn update_rate_limit(&self, req: UpdateRateLimitRequest) -> UpdateConfigResponse;
            async fn update_logging(&self, req: UpdateLoggingRequest) -> UpdateConfigResponse;
            async fn reload_config(&self) -> UpdateConfigResponse;
            async fn get_rate_limit_override(&self) -> Option<(u32, u32)>;
            async fn set_algorithm(&self, req: SetAlgorithmRequest) -> SetAlgorithmResponse;
            async fn create_biz_tag(&self, request: &crate::core::database::CreateBizTagRequest) -> crate::core::Result<BizTag>;
            async fn get_biz_tag(&self, id: Uuid) -> crate::core::Result<Option<BizTag>>;
            async fn update_biz_tag(&self, id: Uuid, request: &crate::core::database::UpdateBizTagRequest) -> crate::core::Result<BizTag>;
            async fn delete_biz_tag(&self, id: Uuid) -> crate::core::Result<()>;
            async fn count_biz_tags(&self, workspace_id: Uuid, group_id: Option<Uuid>) -> crate::core::Result<u64>;
            async fn list_biz_tags(&self, workspace_id: Uuid, group_id: Option<Uuid>, limit: Option<u32>, offset: Option<u32>) -> crate::core::Result<Vec<BizTag>>;
            async fn create_workspace(&self, req: CreateWorkspaceRequest) -> crate::core::Result<WorkspaceResponse>;
            async fn list_workspaces(&self) -> crate::core::Result<WorkspaceListResponse>;
            async fn get_workspace(&self, name: &str) -> crate::core::Result<Option<WorkspaceResponse>>;
            async fn create_group(&self, req: CreateGroupRequest) -> crate::core::Result<GroupResponse>;
            async fn list_groups(&self, workspace: &str) -> crate::core::Result<GroupListResponse>;
            async fn get_database_metrics(&self) -> DatabaseMetrics;
            async fn get_cache_metrics(&self) -> CacheMetrics;
            async fn get_algorithm_metrics(&self) -> Vec<(AlgorithmType, crate::core::algorithm::AlgorithmMetricsSnapshot)>;
        }
    }

    mock! {
        pub WorkspaceTestRepo {}
        #[async_trait]
        impl ApiKeyRepository for WorkspaceTestRepo {
            async fn create_api_key(&self, request: &crate::core::database::CreateApiKeyRequest) -> Result<ApiKeyWithSecret>;
            async fn get_api_key_by_id(&self, key_id: &str) -> Result<Option<ApiKeyInfo>>;
            async fn validate_api_key(&self, key_id: &str, key_secret: &str) -> Result<Option<(Option<Uuid>, ApiKeyRole)>>;
            async fn list_api_keys(&self, workspace_id: Uuid, limit: Option<u32>, offset: Option<u32>) -> Result<Vec<ApiKeyInfo>>;
            async fn delete_api_key(&self, id: Uuid) -> Result<()>;
            async fn revoke_api_key(&self, id: Uuid) -> Result<()>;
            async fn update_last_used(&self, id: Uuid) -> Result<()>;
            async fn get_admin_api_key(&self, workspace_id: Uuid) -> Result<Option<ApiKeyInfo>>;
            async fn count_api_keys(&self, workspace_id: Uuid) -> Result<u64>;
            async fn rotate_api_key(&self, key_id: &str, grace_period_seconds: u64) -> Result<ApiKeyWithSecret>;
            async fn get_keys_older_than(&self, age_threshold_days: i64) -> Result<Vec<ApiKeyInfo>>;
        }
    }

    fn make_handlers_no_repo(mock: MockWorkspaceTestService) -> Arc<ApiHandlers> {
        let mock_gen = Arc::new(MockIdGenerator::new());
        let config_service: Arc<dyn ConfigManagementService> = Arc::new(mock);
        Arc::new(ApiHandlers::new(mock_gen, config_service))
    }

    fn make_handlers_with_repo(
        mock_config: MockWorkspaceTestService,
        mock_repo: MockWorkspaceTestRepo,
    ) -> Arc<ApiHandlers> {
        let mock_gen = Arc::new(MockIdGenerator::new());
        let config_service: Arc<dyn ConfigManagementService> = Arc::new(mock_config);
        let repo: Arc<dyn ApiKeyRepository> = Arc::new(mock_repo);
        Arc::new(ApiHandlers::with_api_key_repository(
            mock_gen,
            config_service,
            repo,
        ))
    }

    fn make_workspace_response() -> WorkspaceResponse {
        WorkspaceResponse {
            id: Uuid::new_v4().to_string(),
            name: "test-workspace".to_string(),
            description: Some("Test workspace".to_string()),
            status: "active".to_string(),
            max_groups: 10,
            max_biz_tags: 100,
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            user_api_key: None,
        }
    }

    fn make_api_key_with_secret(role: ApiKeyRole) -> ApiKeyWithSecret {
        ApiKeyWithSecret {
            key: CoreApiKeyResponse {
                id: Uuid::new_v4(),
                key_id: "nino_test-key-id".to_string(),
                key_prefix: "nino_".to_string(),
                name: "test-key".to_string(),
                description: Some("Test API key".to_string()),
                role,
                rate_limit: 10000,
                enabled: true,
                expires_at: None,
                created_at: chrono::Utc::now().naive_utc(),
            },
            key_secret: "test-secret-value-12345".to_string(),
        }
    }

    fn make_api_key_info(role: ApiKeyRole) -> ApiKeyInfo {
        ApiKeyInfo {
            id: Uuid::new_v4(),
            key_id: "nino_test-key-id".to_string(),
            key_prefix: "nino_".to_string(),
            role,
            workspace_id: Some(Uuid::new_v4()),
            name: "test-key".to_string(),
            description: Some("Test API key".to_string()),
            rate_limit: 10000,
            enabled: true,
            expires_at: None,
            last_used_at: None,
            created_at: chrono::Utc::now().naive_utc(),
        }
    }

    fn make_create_workspace_req() -> CreateWorkspaceRequest {
        CreateWorkspaceRequest {
            name: "test-workspace".to_string(),
            description: Some("Test workspace".to_string()),
            max_groups: Some(10),
            max_biz_tags: Some(100),
        }
    }

    // ===== create_workspace =====

    #[tokio::test]
    async fn test_create_workspace_happy_path() {
        let ws = make_workspace_response();
        let ws_clone = ws.clone();
        let key = make_api_key_with_secret(ApiKeyRole::User);
        let key_clone = key.clone();
        let expected_key_id = key.key.key_id.clone();

        let mut mock_config = MockWorkspaceTestService::new();
        mock_config
            .expect_create_workspace()
            .return_once(move |_| Ok(ws_clone));

        let mut mock_repo = MockWorkspaceTestRepo::new();
        mock_repo
            .expect_create_api_key()
            .return_once(move |_| Ok(key_clone));

        let handlers = make_handlers_with_repo(mock_config, mock_repo);
        let result = handlers.create_workspace(make_create_workspace_req()).await;
        assert!(result.is_ok());
        let response = result.unwrap();
        assert_eq!(response.name, "test-workspace");
        assert!(response.user_api_key.is_some());
        assert_eq!(response.user_api_key.unwrap().key_id, expected_key_id);
    }

    #[tokio::test]
    async fn test_create_workspace_no_repo_returns_not_found() {
        // 没有 api_key_repo 时应返回 NotFound 错误。
        let ws = make_workspace_response();
        let ws_clone = ws.clone();
        let mut mock_config = MockWorkspaceTestService::new();
        mock_config
            .expect_create_workspace()
            .return_once(move |_| Ok(ws_clone));

        let handlers = make_handlers_no_repo(mock_config);
        let result = handlers.create_workspace(make_create_workspace_req()).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            CoreError::NotFound(msg) => assert!(msg.contains("API key repository not configured")),
            other => panic!("Expected NotFound, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_create_workspace_db_error() {
        let mut mock_config = MockWorkspaceTestService::new();
        mock_config
            .expect_create_workspace()
            .return_once(|_| Err(CoreError::DatabaseError("connection refused".to_string())));
        // No repo expectations — should never be called.
        let mock_repo = MockWorkspaceTestRepo::new();
        let handlers = make_handlers_with_repo(mock_config, mock_repo);

        let result = handlers.create_workspace(make_create_workspace_req()).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            CoreError::DatabaseError(msg) => assert!(msg.contains("connection refused")),
            other => panic!("Expected DatabaseError, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_create_workspace_invalid_uuid_in_response() {
        // workspace.id 不是合法 UUID — 应当返回错误。
        let mut ws = make_workspace_response();
        ws.id = "not-a-uuid".to_string();

        let mut mock_config = MockWorkspaceTestService::new();
        mock_config
            .expect_create_workspace()
            .return_once(move |_| Ok(ws));
        let mock_repo = MockWorkspaceTestRepo::new();
        let handlers = make_handlers_with_repo(mock_config, mock_repo);

        let result = handlers.create_workspace(make_create_workspace_req()).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            CoreError::InvalidInput(msg) => assert!(msg.contains("Invalid UUID")),
            other => panic!("Expected InvalidInput, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_create_workspace_api_key_creation_fails() {
        let ws = make_workspace_response();
        let ws_clone = ws.clone();
        let mut mock_config = MockWorkspaceTestService::new();
        mock_config
            .expect_create_workspace()
            .return_once(move |_| Ok(ws_clone));

        let mut mock_repo = MockWorkspaceTestRepo::new();
        mock_repo.expect_create_api_key().return_once(|_| {
            Err(CoreError::DatabaseError(
                "api key insert failed".to_string(),
            ))
        });

        let handlers = make_handlers_with_repo(mock_config, mock_repo);
        let result = handlers.create_workspace(make_create_workspace_req()).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            CoreError::DatabaseError(msg) => assert!(msg.contains("api key insert failed")),
            other => panic!("Expected DatabaseError, got {:?}", other),
        }
    }

    // ===== regenerate_user_api_key =====

    #[tokio::test]
    async fn test_regenerate_user_api_key_happy_path() {
        let ws = make_workspace_response();
        let ws_name = ws.name.clone();
        let ws_id = ws.id.clone();
        let mut mock_config = MockWorkspaceTestService::new();
        mock_config
            .expect_get_workspace()
            .return_once(move |_| Ok(Some(ws)));

        let existing_key = make_api_key_info(ApiKeyRole::User);
        let existing_key_id = existing_key.id;

        let new_key = make_api_key_with_secret(ApiKeyRole::User);
        let new_key_clone = new_key.clone();
        let expected_secret = new_key.key_secret.clone();

        let mut mock_repo = MockWorkspaceTestRepo::new();
        mock_repo
            .expect_list_api_keys()
            .return_once(move |_, _, _| Ok(vec![existing_key]));
        mock_repo
            .expect_delete_api_key()
            .withf(move |id| *id == existing_key_id)
            .return_once(|_| Ok(()));
        mock_repo
            .expect_create_api_key()
            .return_once(move |_| Ok(new_key_clone));

        let handlers = make_handlers_with_repo(mock_config, mock_repo);
        let result = handlers.regenerate_user_api_key(&ws_name).await;
        assert!(result.is_ok());
        let response = result.unwrap();
        assert_eq!(response.key_secret, expected_secret);
        assert_eq!(response.key.role, "user");
        // 验证 workspace id 被正确解析（无 panic 即可）。
        let _ = ws_id;
    }

    #[tokio::test]
    async fn test_regenerate_user_api_key_workspace_not_found() {
        let mut mock_config = MockWorkspaceTestService::new();
        mock_config.expect_get_workspace().return_once(|_| Ok(None));
        let mock_repo = MockWorkspaceTestRepo::new();
        let handlers = make_handlers_with_repo(mock_config, mock_repo);

        let result = handlers.regenerate_user_api_key("nonexistent").await;
        assert!(result.is_err());
        match result.unwrap_err() {
            CoreError::NotFound(msg) => assert!(msg.contains("not found")),
            other => panic!("Expected NotFound, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_regenerate_user_api_key_no_repo() {
        let ws = make_workspace_response();
        let ws_clone = ws.clone();
        let mut mock_config = MockWorkspaceTestService::new();
        mock_config
            .expect_get_workspace()
            .return_once(move |_| Ok(Some(ws_clone)));
        let handlers = make_handlers_no_repo(mock_config);

        let result = handlers.regenerate_user_api_key("test-workspace").await;
        assert!(result.is_err());
        match result.unwrap_err() {
            CoreError::NotFound(msg) => assert!(msg.contains("API key repository not configured")),
            other => panic!("Expected NotFound, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_regenerate_user_api_key_list_error() {
        let ws = make_workspace_response();
        let ws_clone = ws.clone();
        let mut mock_config = MockWorkspaceTestService::new();
        mock_config
            .expect_get_workspace()
            .return_once(move |_| Ok(Some(ws_clone)));

        let mut mock_repo = MockWorkspaceTestRepo::new();
        mock_repo
            .expect_list_api_keys()
            .return_once(|_, _, _| Err(CoreError::DatabaseError("list failed".to_string())));

        let handlers = make_handlers_with_repo(mock_config, mock_repo);
        let result = handlers.regenerate_user_api_key("test-workspace").await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), CoreError::DatabaseError(_)));
    }

    #[tokio::test]
    async fn test_regenerate_user_api_key_create_error() {
        let ws = make_workspace_response();
        let ws_clone = ws.clone();
        let mut mock_config = MockWorkspaceTestService::new();
        mock_config
            .expect_get_workspace()
            .return_once(move |_| Ok(Some(ws_clone)));

        let mut mock_repo = MockWorkspaceTestRepo::new();
        mock_repo
            .expect_list_api_keys()
            .return_once(|_, _, _| Ok(Vec::new()));
        mock_repo
            .expect_create_api_key()
            .return_once(|_| Err(CoreError::DatabaseError("create failed".to_string())));

        let handlers = make_handlers_with_repo(mock_config, mock_repo);
        let result = handlers.regenerate_user_api_key("test-workspace").await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), CoreError::DatabaseError(_)));
    }

    #[tokio::test]
    async fn test_regenerate_user_api_key_skips_admin_keys() {
        // 只删除 User 角色的 key，Admin 角色应跳过。
        let ws = make_workspace_response();
        let ws_clone = ws.clone();
        let mut mock_config = MockWorkspaceTestService::new();
        mock_config
            .expect_get_workspace()
            .return_once(move |_| Ok(Some(ws_clone)));

        let admin_key = make_api_key_info(ApiKeyRole::Admin);
        let user_key = make_api_key_info(ApiKeyRole::User);
        let user_key_id = user_key.id;

        let new_key = make_api_key_with_secret(ApiKeyRole::User);

        let mut mock_repo = MockWorkspaceTestRepo::new();
        mock_repo
            .expect_list_api_keys()
            .return_once(move |_, _, _| Ok(vec![admin_key, user_key]));
        // 只期望删除 user key（不删除 admin key）。
        mock_repo
            .expect_delete_api_key()
            .withf(move |id| *id == user_key_id)
            .return_once(|_| Ok(()));
        mock_repo
            .expect_create_api_key()
            .return_once(move |_| Ok(new_key));

        let handlers = make_handlers_with_repo(mock_config, mock_repo);
        let result = handlers.regenerate_user_api_key("test-workspace").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_regenerate_user_api_key_delete_error() {
        let ws = make_workspace_response();
        let ws_clone = ws.clone();
        let mut mock_config = MockWorkspaceTestService::new();
        mock_config
            .expect_get_workspace()
            .return_once(move |_| Ok(Some(ws_clone)));

        let user_key = make_api_key_info(ApiKeyRole::User);

        let mut mock_repo = MockWorkspaceTestRepo::new();
        mock_repo
            .expect_list_api_keys()
            .return_once(move |_, _, _| Ok(vec![user_key]));
        mock_repo
            .expect_delete_api_key()
            .return_once(|_| Err(CoreError::DatabaseError("delete failed".to_string())));

        let handlers = make_handlers_with_repo(mock_config, mock_repo);
        let result = handlers.regenerate_user_api_key("test-workspace").await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), CoreError::DatabaseError(_)));
    }

    // ===== list_workspaces =====

    #[tokio::test]
    async fn test_list_workspaces_happy_path() {
        let ws = make_workspace_response();
        let ws_clone = ws.clone();
        let mut mock_config = MockWorkspaceTestService::new();
        mock_config.expect_list_workspaces().return_once(|| {
            Ok(WorkspaceListResponse {
                workspaces: vec![ws_clone],
                total: 1,
            })
        });
        let handlers = make_handlers_no_repo(mock_config);

        let result = handlers.list_workspaces().await;
        assert!(result.is_ok());
        let response = result.unwrap();
        assert_eq!(response.total, 1);
        assert_eq!(response.workspaces.len(), 1);
        assert_eq!(response.workspaces[0].name, "test-workspace");
    }

    #[tokio::test]
    async fn test_list_workspaces_empty() {
        let mut mock_config = MockWorkspaceTestService::new();
        mock_config.expect_list_workspaces().return_once(|| {
            Ok(WorkspaceListResponse {
                workspaces: Vec::new(),
                total: 0,
            })
        });
        let handlers = make_handlers_no_repo(mock_config);

        let result = handlers.list_workspaces().await;
        assert!(result.is_ok());
        let response = result.unwrap();
        assert_eq!(response.total, 0);
        assert!(response.workspaces.is_empty());
    }

    #[tokio::test]
    async fn test_list_workspaces_error() {
        let mut mock_config = MockWorkspaceTestService::new();
        mock_config
            .expect_list_workspaces()
            .return_once(|| Err(CoreError::InternalError("db err".to_string())));
        let handlers = make_handlers_no_repo(mock_config);

        let result = handlers.list_workspaces().await;
        assert!(result.is_err());
        // list_workspaces 直接传播 CoreError，不经过 map_db_error。
        assert!(matches!(result.unwrap_err(), CoreError::InternalError(_)));
    }

    // ===== get_workspace =====

    #[tokio::test]
    async fn test_get_workspace_happy_path() {
        let ws = make_workspace_response();
        let ws_clone = ws.clone();
        let expected_name = ws.name.clone();
        let mut mock_config = MockWorkspaceTestService::new();
        mock_config
            .expect_get_workspace()
            .return_once(move |_| Ok(Some(ws_clone)));
        let handlers = make_handlers_no_repo(mock_config);

        let result = handlers.get_workspace("test-workspace").await;
        assert!(result.is_ok());
        let response = result.unwrap();
        assert!(response.is_some());
        assert_eq!(response.unwrap().name, expected_name);
    }

    #[tokio::test]
    async fn test_get_workspace_not_found() {
        let mut mock_config = MockWorkspaceTestService::new();
        mock_config.expect_get_workspace().return_once(|_| Ok(None));
        let handlers = make_handlers_no_repo(mock_config);

        let result = handlers.get_workspace("nonexistent").await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_get_workspace_service_error() {
        let mut mock_config = MockWorkspaceTestService::new();
        mock_config
            .expect_get_workspace()
            .return_once(|_| Err(CoreError::InternalError("db err".to_string())));
        let handlers = make_handlers_no_repo(mock_config);

        let result = handlers.get_workspace("test").await;
        assert!(result.is_err());
    }

    // ===== create_group =====

    #[tokio::test]
    async fn test_create_group_happy_path() {
        let group = GroupResponse {
            id: Uuid::new_v4().to_string(),
            workspace_id: Uuid::new_v4().to_string(),
            workspace_name: "test-workspace".to_string(),
            name: "test-group".to_string(),
            description: Some("Test group".to_string()),
            max_biz_tags: 50,
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
        };
        let group_clone = group.clone();
        let expected_name = group.name.clone();
        let mut mock_config = MockWorkspaceTestService::new();
        mock_config
            .expect_create_group()
            .return_once(move |_| Ok(group_clone));
        let handlers = make_handlers_no_repo(mock_config);

        let req = CreateGroupRequest {
            workspace: "test-workspace".to_string(),
            name: "test-group".to_string(),
            description: Some("Test group".to_string()),
            max_biz_tags: Some(50),
        };
        let result = handlers.create_group(req).await;
        assert!(result.is_ok());
        let response = result.unwrap();
        assert_eq!(response.name, expected_name);
        assert_eq!(response.workspace_name, "test-workspace");
        assert_eq!(response.max_biz_tags, 50);
        assert_eq!(group.max_biz_tags, 50);
    }

    #[tokio::test]
    async fn test_create_group_service_error() {
        let mut mock_config = MockWorkspaceTestService::new();
        mock_config
            .expect_create_group()
            .return_once(|_| Err(CoreError::InternalError("db err".to_string())));
        let handlers = make_handlers_no_repo(mock_config);

        let req = CreateGroupRequest {
            workspace: "test-workspace".to_string(),
            name: "test-group".to_string(),
            description: None,
            max_biz_tags: None,
        };
        let result = handlers.create_group(req).await;
        assert!(result.is_err());
    }

    // ===== list_groups =====

    #[tokio::test]
    async fn test_list_groups_happy_path() {
        let group = GroupResponse {
            id: Uuid::new_v4().to_string(),
            workspace_id: Uuid::new_v4().to_string(),
            workspace_name: "test-workspace".to_string(),
            name: "test-group".to_string(),
            description: None,
            max_biz_tags: 50,
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
        };
        let group_clone = group.clone();
        let mut mock_config = MockWorkspaceTestService::new();
        mock_config.expect_list_groups().return_once(move |_| {
            Ok(GroupListResponse {
                groups: vec![group_clone],
                total: 1,
            })
        });
        let handlers = make_handlers_no_repo(mock_config);

        let result = handlers.list_groups("test-workspace").await;
        assert!(result.is_ok());
        let response = result.unwrap();
        assert_eq!(response.total, 1);
        assert_eq!(response.groups.len(), 1);
        assert_eq!(response.groups[0].name, "test-group");
        assert_eq!(group.max_biz_tags, 50);
    }

    #[tokio::test]
    async fn test_list_groups_empty() {
        let mut mock_config = MockWorkspaceTestService::new();
        mock_config.expect_list_groups().return_once(|_| {
            Ok(GroupListResponse {
                groups: Vec::new(),
                total: 0,
            })
        });
        let handlers = make_handlers_no_repo(mock_config);

        let result = handlers.list_groups("test-workspace").await;
        assert!(result.is_ok());
        let response = result.unwrap();
        assert_eq!(response.total, 0);
        assert!(response.groups.is_empty());
    }

    #[tokio::test]
    async fn test_list_groups_service_error() {
        let mut mock_config = MockWorkspaceTestService::new();
        mock_config
            .expect_list_groups()
            .return_once(|_| Err(CoreError::InternalError("db err".to_string())));
        let handlers = make_handlers_no_repo(mock_config);

        let result = handlers.list_groups("test-workspace").await;
        assert!(result.is_err());
    }
}
