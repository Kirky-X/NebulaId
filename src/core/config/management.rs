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

#![allow(clippy::type_complexity)]
#![allow(dead_code)]

use std::sync::Arc;
use uuid::Uuid;

use crate::core::config::dynamic::{
    DynamicConfigRequest, DynamicConfigResponse, DynamicConfigService,
};
use crate::core::database::{
    BizTag, BizTagRepository, CreateBizTagRequest, CreateGroupRequest, CreateWorkspaceRequest,
    Group, GroupRepository, UpdateBizTagRequest, UpdateGroupRequest, UpdateWorkspaceRequest,
    Workspace, WorkspaceRepository,
};
use crate::core::types::Result;

/// 配置管理服务，提供对工作空间、组和业务标签的管理功能
///
/// 该服务提供对ID生成系统的配置管理，包括工作空间、组和业务标签的CRUD操作，
/// 以及动态配置更新功能。
pub struct WorkspaceConfigManager<R>
where
    R: WorkspaceRepository + GroupRepository + BizTagRepository + Send + Sync,
{
    repository: Arc<R>,
}

impl<R> WorkspaceConfigManager<R>
where
    R: WorkspaceRepository + GroupRepository + BizTagRepository + Send + Sync,
{
    /// 创建新的配置管理服务实例
    ///
    /// # Arguments
    /// * `repository` - 数据库仓库实现
    ///
    /// # Returns
    /// 返回配置管理服务实例
    pub fn new(repository: Arc<R>) -> Self {
        Self { repository }
    }

    /// 创建工作空间
    ///
    /// # Arguments
    /// * `request` - 创建工作空间的请求参数
    ///
    /// # Returns
    /// 返回创建的工作空间信息
    ///
    /// # Errors
    /// 当请求参数无效或数据库操作失败时返回错误
    pub async fn create_workspace(&self, request: &CreateWorkspaceRequest) -> Result<Workspace> {
        // 验证请求参数
        if request.name.trim().is_empty() {
            return Err(crate::core::CoreError::InvalidInput(
                "Workspace name cannot be empty".to_string(),
            ));
        }

        self.repository.create_workspace(request).await
    }

    /// 根据ID获取工作空间
    ///
    /// # Arguments
    /// * `id` - 工作空间ID
    ///
    /// # Returns
    /// 返回可选的工作空间信息，如果不存在则返回None
    pub async fn get_workspace(&self, id: Uuid) -> Result<Option<Workspace>> {
        self.repository.get_workspace(id).await
    }

    /// 更新工作空间
    ///
    /// # Arguments
    /// * `id` - 工作空间ID
    /// * `request` - 更新工作空间的请求参数
    ///
    /// # Returns
    /// 返回更新后的工作空间信息
    ///
    /// # Errors
    /// 当工作空间不存在或请求参数无效时返回错误
    pub async fn update_workspace(
        &self,
        id: Uuid,
        request: &UpdateWorkspaceRequest,
    ) -> Result<Workspace> {
        // 验证请求参数
        if let Some(ref name) = request.name {
            if name.trim().is_empty() {
                return Err(crate::core::CoreError::InvalidInput(
                    "Workspace name cannot be empty".to_string(),
                ));
            }
        }

        self.repository.update_workspace(id, request).await
    }

    /// 删除工作空间
    ///
    /// # Arguments
    /// * `id` - 工作空间ID
    ///
    /// # Returns
    /// 删除成功返回空结果
    ///
    /// # Errors
    /// 当工作空间不存在时返回错误
    pub async fn delete_workspace(&self, id: Uuid) -> Result<()> {
        self.repository.delete_workspace(id).await
    }

    /// 列出工作空间
    ///
    /// # Arguments
    /// * `limit` - 限制返回结果数量
    /// * `offset` - 偏移量
    ///
    /// # Returns
    /// 返回工作空间列表
    pub async fn list_workspaces(
        &self,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> Result<Vec<Workspace>> {
        self.repository.list_workspaces(limit, offset).await
    }

    /// 创建组
    ///
    /// # Arguments
    /// * `request` - 创建组的请求参数
    ///
    /// # Returns
    /// 返回创建的组信息
    ///
    /// # Errors
    /// 当请求参数无效或数据库操作失败时返回错误
    pub async fn create_group(&self, request: &CreateGroupRequest) -> Result<Group> {
        // 验证请求参数
        if request.name.trim().is_empty() {
            return Err(crate::core::CoreError::InvalidInput(
                "Group name cannot be empty".to_string(),
            ));
        }

        self.repository.create_group(request).await
    }

    /// 根据ID获取组
    ///
    /// # Arguments
    /// * `id` - 组ID
    ///
    /// # Returns
    /// 返回可选的组信息，如果不存在则返回None
    pub async fn get_group(&self, id: Uuid) -> Result<Option<Group>> {
        self.repository.get_group(id).await
    }

    /// 更新组
    ///
    /// # Arguments
    /// * `id` - 组ID
    /// * `request` - 更新组的请求参数
    ///
    /// # Returns
    /// 返回更新后的组信息
    ///
    /// # Errors
    /// 当组不存在或请求参数无效时返回错误
    pub async fn update_group(&self, id: Uuid, request: &UpdateGroupRequest) -> Result<Group> {
        // 验证请求参数
        if let Some(ref name) = request.name {
            if name.trim().is_empty() {
                return Err(crate::core::CoreError::InvalidInput(
                    "Group name cannot be empty".to_string(),
                ));
            }
        }

        self.repository.update_group(id, request).await
    }

    /// 删除组
    ///
    /// # Arguments
    /// * `id` - 组ID
    ///
    /// # Returns
    /// 删除成功返回空结果
    ///
    /// # Errors
    /// 当组不存在时返回错误
    pub async fn delete_group(&self, id: Uuid) -> Result<()> {
        self.repository.delete_group(id).await
    }

    /// 列出组
    ///
    /// # Arguments
    /// * `workspace_id` - 工作空间ID
    /// * `limit` - 限制返回结果数量
    /// * `offset` - 偏移量
    ///
    /// # Returns
    /// 返回组列表
    pub async fn list_groups(
        &self,
        workspace_id: Uuid,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> Result<Vec<Group>> {
        self.repository
            .list_groups(workspace_id, limit, offset)
            .await
    }

    /// 创建业务标签
    ///
    /// # Arguments
    /// * `request` - 创建业务标签的请求参数
    ///
    /// # Returns
    /// 返回创建的业务标签信息
    ///
    /// # Errors
    /// 当请求参数无效或数据库操作失败时返回错误
    pub async fn create_biz_tag(&self, request: &CreateBizTagRequest) -> Result<BizTag> {
        // 验证请求参数
        if request.name.trim().is_empty() {
            return Err(crate::core::CoreError::InvalidInput(
                "BizTag name cannot be empty".to_string(),
            ));
        }

        self.repository.create_biz_tag(request).await
    }

    /// 根据ID获取业务标签
    ///
    /// # Arguments
    /// * `id` - 业务标签ID
    ///
    /// # Returns
    /// 返回可选的业务标签信息，如果不存在则返回None
    pub async fn get_biz_tag(&self, id: Uuid) -> Result<Option<BizTag>> {
        self.repository.get_biz_tag(id).await
    }

    /// 更新业务标签
    ///
    /// # Arguments
    /// * `id` - 业务标签ID
    /// * `request` - 更新业务标签的请求参数
    ///
    /// # Returns
    /// 返回更新后的业务标签信息
    ///
    /// # Errors
    /// 当业务标签不存在或请求参数无效时返回错误
    pub async fn update_biz_tag(&self, id: Uuid, request: &UpdateBizTagRequest) -> Result<BizTag> {
        // 验证请求参数
        if let Some(ref name) = request.name {
            if name.trim().is_empty() {
                return Err(crate::core::CoreError::InvalidInput(
                    "BizTag name cannot be empty".to_string(),
                ));
            }
        }

        self.repository.update_biz_tag(id, request).await
    }

    /// 删除业务标签
    ///
    /// # Arguments
    /// * `id` - 业务标签ID
    ///
    /// # Returns
    /// 删除成功返回空结果
    ///
    /// # Errors
    /// 当业务标签不存在时返回错误
    pub async fn delete_biz_tag(&self, id: Uuid) -> Result<()> {
        self.repository.delete_biz_tag(id).await
    }

    /// 列出业务标签
    ///
    /// # Arguments
    /// * `workspace_id` - 工作空间ID
    /// * `group_id` - 组ID（可选）
    /// * `limit` - 限制返回结果数量
    /// * `offset` - 偏移量
    ///
    /// # Returns
    /// 返回业务标签列表
    pub async fn list_biz_tags(
        &self,
        workspace_id: Uuid,
        group_id: Option<uuid::Uuid>,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> Result<Vec<BizTag>> {
        self.repository
            .list_biz_tags(workspace_id, group_id, limit, offset)
            .await
    }

    /// 更新业务标签的配置参数（动态配置）
    ///
    /// # Arguments
    /// * `request` - 动态配置更新请求
    ///
    /// # Returns
    /// 返回更新后的配置信息
    ///
    /// # Errors
    /// 当业务标签不存在或请求参数无效时返回错误
    pub async fn update_biz_tag_config(
        &self,
        request: &DynamicConfigRequest,
    ) -> Result<DynamicConfigResponse> {
        // 验证请求参数
        if request.biz_tag.trim().is_empty() {
            return Err(crate::core::CoreError::InvalidInput(
                "BizTag name cannot be empty".to_string(),
            ));
        }

        let dynamic_config_service = DynamicConfigService::new(Arc::clone(&self.repository));
        dynamic_config_service.update_config(request).await
    }

    /// 批量更新配置参数
    ///
    /// # Arguments
    /// * `requests` - 动态配置更新请求列表
    ///
    /// # Returns
    /// 返回更新后的配置信息列表
    ///
    /// # Errors
    /// 当任一业务标签不存在或请求参数无效时返回错误
    pub async fn batch_update_biz_tag_config(
        &self,
        requests: Vec<DynamicConfigRequest>,
    ) -> Result<Vec<DynamicConfigResponse>> {
        // 验证请求参数
        for request in &requests {
            if request.biz_tag.trim().is_empty() {
                return Err(crate::core::CoreError::InvalidInput(
                    "BizTag name cannot be empty".to_string(),
                ));
            }
        }

        let dynamic_config_service = DynamicConfigService::new(Arc::clone(&self.repository));
        dynamic_config_service.batch_update_config(requests).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::database::{
        BizTag, CreateBizTagRequest, CreateGroupRequest, CreateWorkspaceRequest, Group,
        UpdateBizTagRequest, UpdateGroupRequest, UpdateWorkspaceRequest, Workspace, WorkspaceStatus,
    };
    use crate::core::types::id::{AlgorithmType, IdFormat};
    use crate::core::types::Result;
    use async_trait::async_trait;
    use mockall::{mock, predicate};
    use uuid::Uuid;

    // 同时实现三个 repository trait 的 mock，用于测试 WorkspaceConfigManager
    mock! {
        pub Repository {}

        #[async_trait]
        impl WorkspaceRepository for Repository {
            async fn create_workspace(&self, workspace: &CreateWorkspaceRequest) -> Result<Workspace>;
            async fn get_workspace(&self, id: Uuid) -> Result<Option<Workspace>>;
            async fn get_workspace_by_name(&self, name: &str) -> Result<Option<Workspace>>;
            async fn update_workspace(&self, id: Uuid, workspace: &UpdateWorkspaceRequest) -> Result<Workspace>;
            async fn delete_workspace(&self, id: Uuid) -> Result<()>;
            async fn list_workspaces(&self, limit: Option<u32>, offset: Option<u32>) -> Result<Vec<Workspace>>;
            async fn get_workspace_with_groups(&self, id: Uuid) -> Result<Option<(Workspace, Vec<Group>)>>;
            async fn get_workspace_with_groups_and_biz_tags(&self, id: Uuid) -> Result<Option<(Workspace, Vec<(Group, Vec<BizTag>)>)>>;
        }

        #[async_trait]
        impl GroupRepository for Repository {
            async fn create_group(&self, group: &CreateGroupRequest) -> Result<Group>;
            async fn get_group(&self, id: Uuid) -> Result<Option<Group>>;
            async fn get_group_by_workspace_and_name(&self, workspace_id: Uuid, name: &str) -> Result<Option<Group>>;
            async fn update_group(&self, id: Uuid, group: &UpdateGroupRequest) -> Result<Group>;
            async fn delete_group(&self, id: Uuid) -> Result<()>;
            async fn list_groups(&self, workspace_id: Uuid, limit: Option<u32>, offset: Option<u32>) -> Result<Vec<Group>>;
            async fn get_group_with_biz_tags(&self, id: Uuid) -> Result<Option<(Group, Vec<BizTag>)>>;
            async fn delete_group_with_biz_tags(&self, id: Uuid) -> Result<()>;
        }

        #[async_trait]
        impl BizTagRepository for Repository {
            async fn create_biz_tag(&self, biz_tag: &CreateBizTagRequest) -> Result<BizTag>;
            async fn get_biz_tag(&self, id: Uuid) -> Result<Option<BizTag>>;
            async fn get_biz_tag_by_workspace_group_and_name(&self, workspace_id: Uuid, group_id: Uuid, name: &str) -> Result<Option<BizTag>>;
            async fn update_biz_tag(&self, id: Uuid, biz_tag: &UpdateBizTagRequest) -> Result<BizTag>;
            async fn delete_biz_tag(&self, id: Uuid) -> Result<()>;
            async fn list_biz_tags(&self, workspace_id: Uuid, group_id: Option<Uuid>, limit: Option<u32>, offset: Option<u32>) -> Result<Vec<BizTag>>;
            async fn list_biz_tags_by_workspace_group(&self, workspace_id: Uuid, group_id: Uuid) -> Result<Vec<BizTag>>;
            async fn count_biz_tags_by_group(&self, group_id: Uuid) -> Result<u64>;
            async fn count_biz_tags(&self, workspace_id: Uuid, group_id: Option<Uuid>) -> Result<u64>;
            async fn health_check(&self) -> Result<()>;
        }
    }

    fn now_naive() -> chrono::NaiveDateTime {
        chrono::Utc::now().naive_utc()
    }

    fn sample_workspace(id: Uuid, name: &str) -> Workspace {
        Workspace {
            id,
            name: name.to_string(),
            description: Some("测试工作空间".to_string()),
            status: WorkspaceStatus::Active,
            max_groups: 10,
            max_biz_tags: 100,
            created_at: now_naive(),
            updated_at: now_naive(),
        }
    }

    fn sample_group(id: Uuid, workspace_id: Uuid, name: &str) -> Group {
        Group {
            id,
            workspace_id,
            name: name.to_string(),
            description: Some("测试组".to_string()),
            max_biz_tags: 50,
            created_at: now_naive(),
            updated_at: now_naive(),
        }
    }

    fn sample_biz_tag(id: Uuid, workspace_id: Uuid, group_id: Uuid, name: &str) -> BizTag {
        BizTag {
            id,
            workspace_id,
            group_id,
            name: name.to_string(),
            description: Some("测试业务标签".to_string()),
            algorithm: AlgorithmType::Segment,
            format: IdFormat::Numeric,
            prefix: String::new(),
            base_step: 100,
            max_step: 1000,
            datacenter_ids: vec![0],
            created_at: now_naive(),
            updated_at: now_naive(),
        }
    }

    // ============ new ============

    /// 验证 new() 正常构造实例，repository 字段被正确持有（通过 Arc 引用计数验证）
    #[tokio::test]
    async fn test_new_manager_construction_succeeds() {
        let repo = Arc::new(MockRepository::new());
        let manager = WorkspaceConfigManager::new(Arc::clone(&repo));
        // manager 持有一个 Arc<R>，故 strong_count == 2
        assert_eq!(Arc::strong_count(&repo), 2);
        // manager drop 后引用计数应回到 1
        drop(manager);
        assert_eq!(Arc::strong_count(&repo), 1);
    }

    // ============ create_workspace ============

    /// 验证 create_workspace 在合法请求下透传到 repository 并返回工作空间
    #[tokio::test]
    async fn test_create_workspace_with_valid_request_forwards_to_repository() {
        let mut repo = MockRepository::new();
        let workspace_id = Uuid::new_v4();

        repo.expect_create_workspace()
            .with(predicate::eq(CreateWorkspaceRequest {
                name: "alpha".to_string(),
                description: Some("测试工作空间".to_string()),
                max_groups: Some(10),
                max_biz_tags: Some(100),
            }))
            .times(1)
            .returning(move |_| Ok(sample_workspace(workspace_id, "alpha")));

        let manager = WorkspaceConfigManager::new(Arc::new(repo));
        let request = CreateWorkspaceRequest {
            name: "alpha".to_string(),
            description: Some("测试工作空间".to_string()),
            max_groups: Some(10),
            max_biz_tags: Some(100),
        };

        let result = manager.create_workspace(&request).await;
        assert!(result.is_ok());
        let ws = result.unwrap();
        assert_eq!(ws.id, workspace_id);
        assert_eq!(ws.name, "alpha");
        assert_eq!(ws.status, WorkspaceStatus::Active);
    }

    /// 验证 create_workspace 在空名称时返回 InvalidInput 且不触达 repository
    #[tokio::test]
    async fn test_create_workspace_with_empty_name_returns_invalid_input() {
        let mut repo = MockRepository::new();
        // 即便误调用也应失败：times(0) 确保未触达
        repo.expect_create_workspace().times(0);

        let manager = WorkspaceConfigManager::new(Arc::new(repo));
        let request = CreateWorkspaceRequest {
            name: String::new(),
            description: None,
            max_groups: None,
            max_biz_tags: None,
        };

        let result = manager.create_workspace(&request).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            crate::core::CoreError::InvalidInput(_)
        ));
    }

    /// 验证 create_workspace 在仅含空白的名称时同样返回 InvalidInput
    #[tokio::test]
    async fn test_create_workspace_with_whitespace_name_returns_invalid_input() {
        let mut repo = MockRepository::new();
        repo.expect_create_workspace().times(0);

        let manager = WorkspaceConfigManager::new(Arc::new(repo));
        let request = CreateWorkspaceRequest {
            name: "   \t ".to_string(),
            description: None,
            max_groups: None,
            max_biz_tags: None,
        };

        let result = manager.create_workspace(&request).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            crate::core::CoreError::InvalidInput(_)
        ));
    }

    /// 验证 create_workspace 将 repository 错误原样透传
    #[tokio::test]
    async fn test_create_workspace_propagates_repository_error() {
        let mut repo = MockRepository::new();
        repo.expect_create_workspace()
            .returning(|_| Err(crate::core::CoreError::DatabaseError("db down".to_string())));

        let manager = WorkspaceConfigManager::new(Arc::new(repo));
        let request = CreateWorkspaceRequest {
            name: "alpha".to_string(),
            description: None,
            max_groups: None,
            max_biz_tags: None,
        };

        let result = manager.create_workspace(&request).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            crate::core::CoreError::DatabaseError(_)
        ));
    }

    // ============ get_workspace ============

    /// 验证 get_workspace 在找到时返回 Some
    #[tokio::test]
    async fn test_get_workspace_returns_some_when_found() {
        let mut repo = MockRepository::new();
        let workspace_id = Uuid::new_v4();
        repo.expect_get_workspace()
            .with(predicate::eq(workspace_id))
            .times(1)
            .returning(move |_| Ok(Some(sample_workspace(workspace_id, "alpha"))));

        let manager = WorkspaceConfigManager::new(Arc::new(repo));
        let result = manager.get_workspace(workspace_id).await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_some());
    }

    /// 验证 get_workspace 在未找到时返回 None
    #[tokio::test]
    async fn test_get_workspace_returns_none_when_not_found() {
        let mut repo = MockRepository::new();
        repo.expect_get_workspace()
            .returning(|_| Ok(None));

        let manager = WorkspaceConfigManager::new(Arc::new(repo));
        let result = manager.get_workspace(Uuid::new_v4()).await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    /// 验证 get_workspace 透传 repository 错误
    #[tokio::test]
    async fn test_get_workspace_propagates_repository_error() {
        let mut repo = MockRepository::new();
        repo.expect_get_workspace()
            .returning(|_| Err(crate::core::CoreError::DatabaseError("conn lost".to_string())));

        let manager = WorkspaceConfigManager::new(Arc::new(repo));
        let result = manager.get_workspace(Uuid::new_v4()).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            crate::core::CoreError::DatabaseError(_)
        ));
    }

    // ============ update_workspace ============

    /// 验证 update_workspace 在合法请求下透传 repository 并返回更新后的工作空间
    #[tokio::test]
    async fn test_update_workspace_with_valid_request_forwards_to_repository() {
        let mut repo = MockRepository::new();
        let workspace_id = Uuid::new_v4();
        repo.expect_update_workspace()
            .with(predicate::eq(workspace_id), predicate::always())
            .times(1)
            .returning(move |_, _| Ok(sample_workspace(workspace_id, "renamed")));

        let manager = WorkspaceConfigManager::new(Arc::new(repo));
        let request = UpdateWorkspaceRequest {
            name: Some("renamed".to_string()),
            description: None,
            status: None,
            max_groups: None,
            max_biz_tags: None,
        };

        let result = manager.update_workspace(workspace_id, &request).await;
        assert!(result.is_ok());
        let ws = result.unwrap();
        assert_eq!(ws.id, workspace_id);
        assert_eq!(ws.name, "renamed");
    }

    /// 验证 update_workspace 在 Some(空字符串) 名称时返回 InvalidInput
    #[tokio::test]
    async fn test_update_workspace_with_empty_name_returns_invalid_input() {
        let mut repo = MockRepository::new();
        repo.expect_update_workspace().times(0);

        let manager = WorkspaceConfigManager::new(Arc::new(repo));
        let request = UpdateWorkspaceRequest {
            name: Some(String::new()),
            description: None,
            status: None,
            max_groups: None,
            max_biz_tags: None,
        };

        let result = manager.update_workspace(Uuid::new_v4(), &request).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            crate::core::CoreError::InvalidInput(_)
        ));
    }

    /// 验证 update_workspace 在 Some(仅空白) 名称时返回 InvalidInput
    #[tokio::test]
    async fn test_update_workspace_with_whitespace_name_returns_invalid_input() {
        let mut repo = MockRepository::new();
        repo.expect_update_workspace().times(0);

        let manager = WorkspaceConfigManager::new(Arc::new(repo));
        let request = UpdateWorkspaceRequest {
            name: Some("  \t ".to_string()),
            description: None,
            status: None,
            max_groups: None,
            max_biz_tags: None,
        };

        let result = manager.update_workspace(Uuid::new_v4(), &request).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            crate::core::CoreError::InvalidInput(_)
        ));
    }

    /// 验证 update_workspace 在 name=None 时跳过名称验证直接调用 repository
    #[tokio::test]
    async fn test_update_workspace_with_none_name_skips_validation() {
        let mut repo = MockRepository::new();
        let workspace_id = Uuid::new_v4();
        repo.expect_update_workspace()
            .with(predicate::eq(workspace_id), predicate::always())
            .times(1)
            .returning(move |_, _| Ok(sample_workspace(workspace_id, "unchanged")));

        let manager = WorkspaceConfigManager::new(Arc::new(repo));
        let request = UpdateWorkspaceRequest {
            name: None,
            description: Some("仅更新描述".to_string()),
            status: Some(WorkspaceStatus::Suspended),
            max_groups: Some(20),
            max_biz_tags: Some(200),
        };

        let result = manager.update_workspace(workspace_id, &request).await;
        assert!(result.is_ok());
        let ws = result.unwrap();
        assert_eq!(ws.id, workspace_id);
    }

    /// 验证 update_workspace 透传 repository 错误
    #[tokio::test]
    async fn test_update_workspace_propagates_repository_error() {
        let mut repo = MockRepository::new();
        repo.expect_update_workspace()
            .returning(|_, _| Err(crate::core::CoreError::NotFound("Workspace not found: x".to_string())));

        let manager = WorkspaceConfigManager::new(Arc::new(repo));
        let request = UpdateWorkspaceRequest {
            name: Some("renamed".to_string()),
            description: None,
            status: None,
            max_groups: None,
            max_biz_tags: None,
        };

        let result = manager.update_workspace(Uuid::new_v4(), &request).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            crate::core::CoreError::NotFound(_)
        ));
    }

    // ============ delete_workspace ============

    /// 验证 delete_workspace 在存在时透传 repository 并返回 Ok
    #[tokio::test]
    async fn test_delete_workspace_calls_repository_and_returns_ok() {
        let mut repo = MockRepository::new();
        let workspace_id = Uuid::new_v4();
        repo.expect_delete_workspace()
            .with(predicate::eq(workspace_id))
            .times(1)
            .returning(|_| Ok(()));

        let manager = WorkspaceConfigManager::new(Arc::new(repo));
        let result = manager.delete_workspace(workspace_id).await;
        assert!(result.is_ok());
    }

    /// 验证 delete_workspace 透传 NotFound 错误
    #[tokio::test]
    async fn test_delete_workspace_propagates_not_found_error() {
        let mut repo = MockRepository::new();
        repo.expect_delete_workspace()
            .returning(|_| Err(crate::core::CoreError::NotFound("Workspace not found: x".to_string())));

        let manager = WorkspaceConfigManager::new(Arc::new(repo));
        let result = manager.delete_workspace(Uuid::new_v4()).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            crate::core::CoreError::NotFound(_)
        ));
    }

    // ============ list_workspaces ============

    /// 验证 list_workspaces 透传 repository 返回的列表
    #[tokio::test]
    async fn test_list_workspaces_returns_repository_list() {
        let mut repo = MockRepository::new();
        let ws1 = sample_workspace(Uuid::new_v4(), "alpha");
        let ws2 = sample_workspace(Uuid::new_v4(), "beta");
        let expected = vec![ws1.clone(), ws2.clone()];

        repo.expect_list_workspaces()
            .with(predicate::eq(None), predicate::eq(None))
            .times(1)
            .returning(move |_, _| Ok(expected.clone()));

        let manager = WorkspaceConfigManager::new(Arc::new(repo));
        let result = manager.list_workspaces(None, None).await;
        assert!(result.is_ok());
        let list = result.unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name, "alpha");
        assert_eq!(list[1].name, "beta");
    }

    /// 验证 list_workspaces 正确传递 limit/offset 参数
    #[tokio::test]
    async fn test_list_workspaces_forwards_limit_and_offset() {
        let mut repo = MockRepository::new();
        repo.expect_list_workspaces()
            .with(predicate::eq(Some(10u32)), predicate::eq(Some(5u32)))
            .times(1)
            .returning(|_, _| Ok(vec![]));

        let manager = WorkspaceConfigManager::new(Arc::new(repo));
        let result = manager.list_workspaces(Some(10), Some(5)).await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    /// 验证 list_workspaces 透传 repository 错误
    #[tokio::test]
    async fn test_list_workspaces_propagates_repository_error() {
        let mut repo = MockRepository::new();
        repo.expect_list_workspaces()
            .returning(|_, _| Err(crate::core::CoreError::DatabaseError("unavailable".to_string())));

        let manager = WorkspaceConfigManager::new(Arc::new(repo));
        let result = manager.list_workspaces(None, None).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            crate::core::CoreError::DatabaseError(_)
        ));
    }

    // ============ create_group ============

    /// 验证 create_group 在合法请求下透传 repository
    #[tokio::test]
    async fn test_create_group_with_valid_request_forwards_to_repository() {
        let mut repo = MockRepository::new();
        let workspace_id = Uuid::new_v4();
        let group_id = Uuid::new_v4();

        repo.expect_create_group()
            .with(predicate::eq(CreateGroupRequest {
                workspace_id,
                name: "g1".to_string(),
                description: None,
                max_biz_tags: Some(50),
            }))
            .times(1)
            .returning(move |_| Ok(sample_group(group_id, workspace_id, "g1")));

        let manager = WorkspaceConfigManager::new(Arc::new(repo));
        let request = CreateGroupRequest {
            workspace_id,
            name: "g1".to_string(),
            description: None,
            max_biz_tags: Some(50),
        };

        let result = manager.create_group(&request).await;
        assert!(result.is_ok());
        let group = result.unwrap();
        assert_eq!(group.id, group_id);
        assert_eq!(group.workspace_id, workspace_id);
        assert_eq!(group.name, "g1");
    }

    /// 验证 create_group 在空名称时返回 InvalidInput
    #[tokio::test]
    async fn test_create_group_with_empty_name_returns_invalid_input() {
        let mut repo = MockRepository::new();
        repo.expect_create_group().times(0);

        let manager = WorkspaceConfigManager::new(Arc::new(repo));
        let request = CreateGroupRequest {
            workspace_id: Uuid::new_v4(),
            name: String::new(),
            description: None,
            max_biz_tags: None,
        };

        let result = manager.create_group(&request).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            crate::core::CoreError::InvalidInput(_)
        ));
    }

    /// 验证 create_group 在仅空白名称时返回 InvalidInput
    #[tokio::test]
    async fn test_create_group_with_whitespace_name_returns_invalid_input() {
        let mut repo = MockRepository::new();
        repo.expect_create_group().times(0);

        let manager = WorkspaceConfigManager::new(Arc::new(repo));
        let request = CreateGroupRequest {
            workspace_id: Uuid::new_v4(),
            name: " \n\t ".to_string(),
            description: None,
            max_biz_tags: None,
        };

        let result = manager.create_group(&request).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            crate::core::CoreError::InvalidInput(_)
        ));
    }

    /// 验证 create_group 透传 repository 错误
    #[tokio::test]
    async fn test_create_group_propagates_repository_error() {
        let mut repo = MockRepository::new();
        repo.expect_create_group()
            .returning(|_| Err(crate::core::CoreError::NotFound("Workspace not found".to_string())));

        let manager = WorkspaceConfigManager::new(Arc::new(repo));
        let request = CreateGroupRequest {
            workspace_id: Uuid::new_v4(),
            name: "g1".to_string(),
            description: None,
            max_biz_tags: None,
        };

        let result = manager.create_group(&request).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            crate::core::CoreError::NotFound(_)
        ));
    }

    // ============ get_group ============

    /// 验证 get_group 在找到时返回 Some
    #[tokio::test]
    async fn test_get_group_returns_some_when_found() {
        let mut repo = MockRepository::new();
        let group_id = Uuid::new_v4();
        let workspace_id = Uuid::new_v4();
        repo.expect_get_group()
            .with(predicate::eq(group_id))
            .times(1)
            .returning(move |_| Ok(Some(sample_group(group_id, workspace_id, "g1"))));

        let manager = WorkspaceConfigManager::new(Arc::new(repo));
        let result = manager.get_group(group_id).await;
        assert!(result.is_ok());
        let opt = result.unwrap();
        assert!(opt.is_some());
        assert_eq!(opt.unwrap().id, group_id);
    }

    /// 验证 get_group 在未找到时返回 None
    #[tokio::test]
    async fn test_get_group_returns_none_when_not_found() {
        let mut repo = MockRepository::new();
        repo.expect_get_group().returning(|_| Ok(None));

        let manager = WorkspaceConfigManager::new(Arc::new(repo));
        let result = manager.get_group(Uuid::new_v4()).await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    // ============ update_group ============

    /// 验证 update_group 在合法请求下透传 repository
    #[tokio::test]
    async fn test_update_group_with_valid_request_forwards_to_repository() {
        let mut repo = MockRepository::new();
        let group_id = Uuid::new_v4();
        let workspace_id = Uuid::new_v4();
        repo.expect_update_group()
            .with(predicate::eq(group_id), predicate::always())
            .times(1)
            .returning(move |_, _| Ok(sample_group(group_id, workspace_id, "renamed")));

        let manager = WorkspaceConfigManager::new(Arc::new(repo));
        let request = UpdateGroupRequest {
            name: Some("renamed".to_string()),
            description: None,
            max_biz_tags: None,
        };

        let result = manager.update_group(group_id, &request).await;
        assert!(result.is_ok());
        let group = result.unwrap();
        assert_eq!(group.id, group_id);
        assert_eq!(group.name, "renamed");
    }

    /// 验证 update_group 在 Some(空字符串) 名称时返回 InvalidInput
    #[tokio::test]
    async fn test_update_group_with_empty_name_returns_invalid_input() {
        let mut repo = MockRepository::new();
        repo.expect_update_group().times(0);

        let manager = WorkspaceConfigManager::new(Arc::new(repo));
        let request = UpdateGroupRequest {
            name: Some(String::new()),
            description: None,
            max_biz_tags: None,
        };

        let result = manager.update_group(Uuid::new_v4(), &request).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            crate::core::CoreError::InvalidInput(_)
        ));
    }

    /// 验证 update_group 在 Some(仅空白) 名称时返回 InvalidInput
    #[tokio::test]
    async fn test_update_group_with_whitespace_name_returns_invalid_input() {
        let mut repo = MockRepository::new();
        repo.expect_update_group().times(0);

        let manager = WorkspaceConfigManager::new(Arc::new(repo));
        let request = UpdateGroupRequest {
            name: Some(" \t ".to_string()),
            description: None,
            max_biz_tags: None,
        };

        let result = manager.update_group(Uuid::new_v4(), &request).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            crate::core::CoreError::InvalidInput(_)
        ));
    }

    /// 验证 update_group 在 name=None 时跳过验证直接调用 repository
    #[tokio::test]
    async fn test_update_group_with_none_name_skips_validation() {
        let mut repo = MockRepository::new();
        let group_id = Uuid::new_v4();
        let workspace_id = Uuid::new_v4();
        repo.expect_update_group()
            .with(predicate::eq(group_id), predicate::always())
            .times(1)
            .returning(move |_, _| Ok(sample_group(group_id, workspace_id, "unchanged")));

        let manager = WorkspaceConfigManager::new(Arc::new(repo));
        let request = UpdateGroupRequest {
            name: None,
            description: Some("仅更新描述".to_string()),
            max_biz_tags: Some(100),
        };

        let result = manager.update_group(group_id, &request).await;
        assert!(result.is_ok());
    }

    /// 验证 update_group 透传 repository 错误
    #[tokio::test]
    async fn test_update_group_propagates_repository_error() {
        let mut repo = MockRepository::new();
        repo.expect_update_group()
            .returning(|_, _| Err(crate::core::CoreError::NotFound("Group not found".to_string())));

        let manager = WorkspaceConfigManager::new(Arc::new(repo));
        let request = UpdateGroupRequest {
            name: Some("x".to_string()),
            description: None,
            max_biz_tags: None,
        };

        let result = manager.update_group(Uuid::new_v4(), &request).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            crate::core::CoreError::NotFound(_)
        ));
    }

    // ============ delete_group ============

    /// 验证 delete_group 透传 repository 并返回 Ok
    #[tokio::test]
    async fn test_delete_group_calls_repository_and_returns_ok() {
        let mut repo = MockRepository::new();
        let group_id = Uuid::new_v4();
        repo.expect_delete_group()
            .with(predicate::eq(group_id))
            .times(1)
            .returning(|_| Ok(()));

        let manager = WorkspaceConfigManager::new(Arc::new(repo));
        let result = manager.delete_group(group_id).await;
        assert!(result.is_ok());
    }

    /// 验证 delete_group 透传 NotFound 错误
    #[tokio::test]
    async fn test_delete_group_propagates_not_found_error() {
        let mut repo = MockRepository::new();
        repo.expect_delete_group()
            .returning(|_| Err(crate::core::CoreError::NotFound("Group not found".to_string())));

        let manager = WorkspaceConfigManager::new(Arc::new(repo));
        let result = manager.delete_group(Uuid::new_v4()).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            crate::core::CoreError::NotFound(_)
        ));
    }

    // ============ list_groups ============

    /// 验证 list_groups 透传 repository 返回的列表，并正确传递 workspace_id
    #[tokio::test]
    async fn test_list_groups_returns_repository_list() {
        let mut repo = MockRepository::new();
        let workspace_id = Uuid::new_v4();
        let g1 = sample_group(Uuid::new_v4(), workspace_id, "g1");
        let g2 = sample_group(Uuid::new_v4(), workspace_id, "g2");
        let expected = vec![g1.clone(), g2.clone()];

        repo.expect_list_groups()
            .with(predicate::eq(workspace_id), predicate::eq(None), predicate::eq(None))
            .times(1)
            .returning(move |_, _, _| Ok(expected.clone()));

        let manager = WorkspaceConfigManager::new(Arc::new(repo));
        let result = manager.list_groups(workspace_id, None, None).await;
        assert!(result.is_ok());
        let list = result.unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name, "g1");
        assert_eq!(list[1].name, "g2");
    }

    /// 验证 list_groups 正确传递 limit/offset
    #[tokio::test]
    async fn test_list_groups_forwards_limit_and_offset() {
        let mut repo = MockRepository::new();
        let workspace_id = Uuid::new_v4();
        repo.expect_list_groups()
            .with(
                predicate::eq(workspace_id),
                predicate::eq(Some(20u32)),
                predicate::eq(Some(5u32)),
            )
            .times(1)
            .returning(|_, _, _| Ok(vec![]));

        let manager = WorkspaceConfigManager::new(Arc::new(repo));
        let result = manager.list_groups(workspace_id, Some(20), Some(5)).await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    // ============ create_biz_tag ============

    /// 验证 create_biz_tag 在合法请求下透传 repository
    #[tokio::test]
    async fn test_create_biz_tag_with_valid_request_forwards_to_repository() {
        let mut repo = MockRepository::new();
        let workspace_id = Uuid::new_v4();
        let group_id = Uuid::new_v4();
        let biz_tag_id = Uuid::new_v4();

        repo.expect_create_biz_tag()
            .with(predicate::eq(CreateBizTagRequest {
                workspace_id,
                group_id,
                name: "tag1".to_string(),
                description: None,
                algorithm: Some(AlgorithmType::Segment),
                format: Some(IdFormat::Numeric),
                prefix: None,
                base_step: Some(100),
                max_step: Some(1000),
                datacenter_ids: Some(vec![0]),
            }))
            .times(1)
            .returning(move |_| Ok(sample_biz_tag(biz_tag_id, workspace_id, group_id, "tag1")));

        let manager = WorkspaceConfigManager::new(Arc::new(repo));
        let request = CreateBizTagRequest {
            workspace_id,
            group_id,
            name: "tag1".to_string(),
            description: None,
            algorithm: Some(AlgorithmType::Segment),
            format: Some(IdFormat::Numeric),
            prefix: None,
            base_step: Some(100),
            max_step: Some(1000),
            datacenter_ids: Some(vec![0]),
        };

        let result = manager.create_biz_tag(&request).await;
        assert!(result.is_ok());
        let tag = result.unwrap();
        assert_eq!(tag.id, biz_tag_id);
        assert_eq!(tag.name, "tag1");
        assert_eq!(tag.algorithm, AlgorithmType::Segment);
    }

    /// 验证 create_biz_tag 在空名称时返回 InvalidInput
    #[tokio::test]
    async fn test_create_biz_tag_with_empty_name_returns_invalid_input() {
        let mut repo = MockRepository::new();
        repo.expect_create_biz_tag().times(0);

        let manager = WorkspaceConfigManager::new(Arc::new(repo));
        let request = CreateBizTagRequest {
            workspace_id: Uuid::new_v4(),
            group_id: Uuid::new_v4(),
            name: String::new(),
            description: None,
            algorithm: None,
            format: None,
            prefix: None,
            base_step: None,
            max_step: None,
            datacenter_ids: None,
        };

        let result = manager.create_biz_tag(&request).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            crate::core::CoreError::InvalidInput(_)
        ));
    }

    /// 验证 create_biz_tag 在仅空白名称时返回 InvalidInput
    #[tokio::test]
    async fn test_create_biz_tag_with_whitespace_name_returns_invalid_input() {
        let mut repo = MockRepository::new();
        repo.expect_create_biz_tag().times(0);

        let manager = WorkspaceConfigManager::new(Arc::new(repo));
        let request = CreateBizTagRequest {
            workspace_id: Uuid::new_v4(),
            group_id: Uuid::new_v4(),
            name: "  \n ".to_string(),
            description: None,
            algorithm: None,
            format: None,
            prefix: None,
            base_step: None,
            max_step: None,
            datacenter_ids: None,
        };

        let result = manager.create_biz_tag(&request).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            crate::core::CoreError::InvalidInput(_)
        ));
    }

    /// 验证 create_biz_tag 透传 repository 错误
    #[tokio::test]
    async fn test_create_biz_tag_propagates_repository_error() {
        let mut repo = MockRepository::new();
        repo.expect_create_biz_tag()
            .returning(|_| Err(crate::core::CoreError::NotFound("Workspace not found".to_string())));

        let manager = WorkspaceConfigManager::new(Arc::new(repo));
        let request = CreateBizTagRequest {
            workspace_id: Uuid::new_v4(),
            group_id: Uuid::new_v4(),
            name: "tag1".to_string(),
            description: None,
            algorithm: None,
            format: None,
            prefix: None,
            base_step: None,
            max_step: None,
            datacenter_ids: None,
        };

        let result = manager.create_biz_tag(&request).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            crate::core::CoreError::NotFound(_)
        ));
    }

    // ============ get_biz_tag ============

    /// 验证 get_biz_tag 在找到时返回 Some
    #[tokio::test]
    async fn test_get_biz_tag_returns_some_when_found() {
        let mut repo = MockRepository::new();
        let biz_tag_id = Uuid::new_v4();
        let workspace_id = Uuid::new_v4();
        let group_id = Uuid::new_v4();
        repo.expect_get_biz_tag()
            .with(predicate::eq(biz_tag_id))
            .times(1)
            .returning(move |_| Ok(Some(sample_biz_tag(biz_tag_id, workspace_id, group_id, "tag1"))));

        let manager = WorkspaceConfigManager::new(Arc::new(repo));
        let result = manager.get_biz_tag(biz_tag_id).await;
        assert!(result.is_ok());
        let opt = result.unwrap();
        assert!(opt.is_some());
        assert_eq!(opt.unwrap().id, biz_tag_id);
    }

    /// 验证 get_biz_tag 在未找到时返回 None
    #[tokio::test]
    async fn test_get_biz_tag_returns_none_when_not_found() {
        let mut repo = MockRepository::new();
        repo.expect_get_biz_tag().returning(|_| Ok(None));

        let manager = WorkspaceConfigManager::new(Arc::new(repo));
        let result = manager.get_biz_tag(Uuid::new_v4()).await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    // ============ update_biz_tag ============

    /// 验证 update_biz_tag 在合法请求下透传 repository
    #[tokio::test]
    async fn test_update_biz_tag_with_valid_request_forwards_to_repository() {
        let mut repo = MockRepository::new();
        let biz_tag_id = Uuid::new_v4();
        let workspace_id = Uuid::new_v4();
        let group_id = Uuid::new_v4();
        repo.expect_update_biz_tag()
            .with(predicate::eq(biz_tag_id), predicate::always())
            .times(1)
            .returning(move |_, _| {
                Ok(sample_biz_tag(biz_tag_id, workspace_id, group_id, "renamed"))
            });

        let manager = WorkspaceConfigManager::new(Arc::new(repo));
        let request = UpdateBizTagRequest {
            name: Some("renamed".to_string()),
            description: None,
            algorithm: None,
            format: None,
            prefix: None,
            base_step: None,
            max_step: None,
            datacenter_ids: None,
        };

        let result = manager.update_biz_tag(biz_tag_id, &request).await;
        assert!(result.is_ok());
        let tag = result.unwrap();
        assert_eq!(tag.id, biz_tag_id);
        assert_eq!(tag.name, "renamed");
    }

    /// 验证 update_biz_tag 在 Some(空字符串) 名称时返回 InvalidInput
    #[tokio::test]
    async fn test_update_biz_tag_with_empty_name_returns_invalid_input() {
        let mut repo = MockRepository::new();
        repo.expect_update_biz_tag().times(0);

        let manager = WorkspaceConfigManager::new(Arc::new(repo));
        let request = UpdateBizTagRequest {
            name: Some(String::new()),
            description: None,
            algorithm: None,
            format: None,
            prefix: None,
            base_step: None,
            max_step: None,
            datacenter_ids: None,
        };

        let result = manager.update_biz_tag(Uuid::new_v4(), &request).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            crate::core::CoreError::InvalidInput(_)
        ));
    }

    /// 验证 update_biz_tag 在 Some(仅空白) 名称时返回 InvalidInput
    #[tokio::test]
    async fn test_update_biz_tag_with_whitespace_name_returns_invalid_input() {
        let mut repo = MockRepository::new();
        repo.expect_update_biz_tag().times(0);

        let manager = WorkspaceConfigManager::new(Arc::new(repo));
        let request = UpdateBizTagRequest {
            name: Some(" \t ".to_string()),
            description: None,
            algorithm: None,
            format: None,
            prefix: None,
            base_step: None,
            max_step: None,
            datacenter_ids: None,
        };

        let result = manager.update_biz_tag(Uuid::new_v4(), &request).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            crate::core::CoreError::InvalidInput(_)
        ));
    }

    /// 验证 update_biz_tag 在 name=None 时跳过验证
    #[tokio::test]
    async fn test_update_biz_tag_with_none_name_skips_validation() {
        let mut repo = MockRepository::new();
        let biz_tag_id = Uuid::new_v4();
        let workspace_id = Uuid::new_v4();
        let group_id = Uuid::new_v4();
        repo.expect_update_biz_tag()
            .with(predicate::eq(biz_tag_id), predicate::always())
            .times(1)
            .returning(move |_, _| {
                Ok(sample_biz_tag(biz_tag_id, workspace_id, group_id, "unchanged"))
            });

        let manager = WorkspaceConfigManager::new(Arc::new(repo));
        let request = UpdateBizTagRequest {
            name: None,
            description: Some("仅更新描述".to_string()),
            algorithm: Some(AlgorithmType::Snowflake),
            format: Some(IdFormat::Prefixed),
            prefix: Some("PRE".to_string()),
            base_step: Some(200),
            max_step: Some(2000),
            datacenter_ids: Some(vec![0, 1]),
        };

        let result = manager.update_biz_tag(biz_tag_id, &request).await;
        assert!(result.is_ok());
    }

    /// 验证 update_biz_tag 透传 repository 错误
    #[tokio::test]
    async fn test_update_biz_tag_propagates_repository_error() {
        let mut repo = MockRepository::new();
        repo.expect_update_biz_tag()
            .returning(|_, _| Err(crate::core::CoreError::NotFound("BizTag not found".to_string())));

        let manager = WorkspaceConfigManager::new(Arc::new(repo));
        let request = UpdateBizTagRequest {
            name: Some("x".to_string()),
            description: None,
            algorithm: None,
            format: None,
            prefix: None,
            base_step: None,
            max_step: None,
            datacenter_ids: None,
        };

        let result = manager.update_biz_tag(Uuid::new_v4(), &request).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            crate::core::CoreError::NotFound(_)
        ));
    }

    // ============ delete_biz_tag ============

    /// 验证 delete_biz_tag 透传 repository 并返回 Ok
    #[tokio::test]
    async fn test_delete_biz_tag_calls_repository_and_returns_ok() {
        let mut repo = MockRepository::new();
        let biz_tag_id = Uuid::new_v4();
        repo.expect_delete_biz_tag()
            .with(predicate::eq(biz_tag_id))
            .times(1)
            .returning(|_| Ok(()));

        let manager = WorkspaceConfigManager::new(Arc::new(repo));
        let result = manager.delete_biz_tag(biz_tag_id).await;
        assert!(result.is_ok());
    }

    /// 验证 delete_biz_tag 透传 NotFound 错误
    #[tokio::test]
    async fn test_delete_biz_tag_propagates_not_found_error() {
        let mut repo = MockRepository::new();
        repo.expect_delete_biz_tag()
            .returning(|_| Err(crate::core::CoreError::NotFound("BizTag not found".to_string())));

        let manager = WorkspaceConfigManager::new(Arc::new(repo));
        let result = manager.delete_biz_tag(Uuid::new_v4()).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            crate::core::CoreError::NotFound(_)
        ));
    }

    // ============ list_biz_tags ============

    /// 验证 list_biz_tags 在指定 group_id 时透传 repository 返回的列表
    #[tokio::test]
    async fn test_list_biz_tags_with_group_id_returns_repository_list() {
        let mut repo = MockRepository::new();
        let workspace_id = Uuid::new_v4();
        let group_id = Uuid::new_v4();
        let t1 = sample_biz_tag(Uuid::new_v4(), workspace_id, group_id, "t1");
        let t2 = sample_biz_tag(Uuid::new_v4(), workspace_id, group_id, "t2");
        let expected = vec![t1.clone(), t2.clone()];

        repo.expect_list_biz_tags()
            .with(
                predicate::eq(workspace_id),
                predicate::eq(Some(group_id)),
                predicate::eq(None),
                predicate::eq(None),
            )
            .times(1)
            .returning(move |_, _, _, _| Ok(expected.clone()));

        let manager = WorkspaceConfigManager::new(Arc::new(repo));
        let result = manager
            .list_biz_tags(workspace_id, Some(group_id), None, None)
            .await;
        assert!(result.is_ok());
        let list = result.unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name, "t1");
        assert_eq!(list[1].name, "t2");
    }

    /// 验证 list_biz_tags 在 group_id=None 时跨组返回
    #[tokio::test]
    async fn test_list_biz_tags_without_group_id_returns_all_in_workspace() {
        let mut repo = MockRepository::new();
        let workspace_id = Uuid::new_v4();
        let group_id = Uuid::new_v4();
        let tag = sample_biz_tag(Uuid::new_v4(), workspace_id, group_id, "t1");

        repo.expect_list_biz_tags()
            .with(
                predicate::eq(workspace_id),
                predicate::eq(None),
                predicate::eq(Some(50u32)),
                predicate::eq(Some(0u32)),
            )
            .times(1)
            .returning(move |_, _, _, _| Ok(vec![tag.clone()]));

        let manager = WorkspaceConfigManager::new(Arc::new(repo));
        let result = manager
            .list_biz_tags(workspace_id, None, Some(50), Some(0))
            .await;
        assert!(result.is_ok());
        let list = result.unwrap();
        assert_eq!(list.len(), 1);
    }

    // ============ update_biz_tag_config ============

    /// 验证 update_biz_tag_config 在合法请求下委托给 DynamicConfigService 并返回响应
    #[tokio::test]
    async fn test_update_biz_tag_config_with_valid_request_delegates_to_service() {
        let mut repo = MockRepository::new();
        let workspace_id = Uuid::new_v4();
        let group_id = Uuid::new_v4();
        let biz_tag_id = Uuid::new_v4();

        let existing = sample_biz_tag(biz_tag_id, workspace_id, group_id, "tag1");
        let mut updated = existing.clone();
        updated.algorithm = AlgorithmType::Snowflake;
        updated.format = IdFormat::Prefixed;
        updated.prefix = "PRE".to_string();
        updated.base_step = 200;
        updated.max_step = 2000;
        updated.datacenter_ids = vec![0, 1];

        repo.expect_get_biz_tag_by_workspace_group_and_name()
            .with(
                predicate::eq(workspace_id),
                predicate::eq(group_id),
                predicate::eq("tag1".to_string()),
            )
            .times(1)
            .returning(move |_, _, _| Ok(Some(existing.clone())));

        repo.expect_update_biz_tag()
            .times(1)
            .returning(move |_, _| Ok(updated.clone()));

        let manager = WorkspaceConfigManager::new(Arc::new(repo));
        let request = DynamicConfigRequest {
            workspace_id,
            group_id: Some(group_id),
            biz_tag: "tag1".to_string(),
            algorithm: Some(AlgorithmType::Snowflake),
            format: Some(IdFormat::Prefixed),
            prefix: Some("PRE".to_string()),
            base_step: Some(200),
            max_step: Some(2000),
            datacenter_ids: Some(vec![0, 1]),
        };

        let result = manager.update_biz_tag_config(&request).await;
        assert!(result.is_ok());
        let resp = result.unwrap();
        assert_eq!(resp.workspace_id, workspace_id);
        assert_eq!(resp.group_id, group_id);
        assert_eq!(resp.biz_tag_id, biz_tag_id);
        assert_eq!(resp.biz_tag, "tag1");
        assert_eq!(resp.algorithm, AlgorithmType::Snowflake);
        assert_eq!(resp.format, IdFormat::Prefixed);
        assert_eq!(resp.prefix, "PRE");
        assert_eq!(resp.base_step, 200);
        assert_eq!(resp.max_step, 2000);
        assert_eq!(resp.datacenter_ids, vec![0, 1]);
    }

    /// 验证 update_biz_tag_config 在空 biz_tag 时返回 InvalidInput 且不触达 repository
    #[tokio::test]
    async fn test_update_biz_tag_config_with_empty_biz_tag_returns_invalid_input() {
        let mut repo = MockRepository::new();
        repo.expect_get_biz_tag_by_workspace_group_and_name().times(0);
        repo.expect_update_biz_tag().times(0);

        let manager = WorkspaceConfigManager::new(Arc::new(repo));
        let request = DynamicConfigRequest {
            workspace_id: Uuid::new_v4(),
            group_id: Some(Uuid::new_v4()),
            biz_tag: "  ".to_string(),
            algorithm: None,
            format: None,
            prefix: None,
            base_step: None,
            max_step: None,
            datacenter_ids: None,
        };

        let result = manager.update_biz_tag_config(&request).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            crate::core::CoreError::InvalidInput(_)
        ));
    }

    /// 验证 update_biz_tag_config 在 group_id=None 时由 service 返回 InvalidInput
    #[tokio::test]
    async fn test_update_biz_tag_config_without_group_id_returns_invalid_input() {
        let mut repo = MockRepository::new();
        repo.expect_get_biz_tag_by_workspace_group_and_name().times(0);
        repo.expect_update_biz_tag().times(0);

        let manager = WorkspaceConfigManager::new(Arc::new(repo));
        let request = DynamicConfigRequest {
            workspace_id: Uuid::new_v4(),
            group_id: None,
            biz_tag: "tag1".to_string(),
            algorithm: None,
            format: None,
            prefix: None,
            base_step: None,
            max_step: None,
            datacenter_ids: None,
        };

        let result = manager.update_biz_tag_config(&request).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            crate::core::CoreError::InvalidInput(_)
        ));
    }

    /// 验证 update_biz_tag_config 在 biz_tag 不存在时返回 NotFound
    #[tokio::test]
    async fn test_update_biz_tag_config_returns_not_found_when_biz_tag_missing() {
        let mut repo = MockRepository::new();
        let workspace_id = Uuid::new_v4();
        let group_id = Uuid::new_v4();

        repo.expect_get_biz_tag_by_workspace_group_and_name()
            .with(
                predicate::eq(workspace_id),
                predicate::eq(group_id),
                predicate::eq("missing".to_string()),
            )
            .times(1)
            .returning(|_, _, _| Ok(None));
        repo.expect_update_biz_tag().times(0);

        let manager = WorkspaceConfigManager::new(Arc::new(repo));
        let request = DynamicConfigRequest {
            workspace_id,
            group_id: Some(group_id),
            biz_tag: "missing".to_string(),
            algorithm: None,
            format: None,
            prefix: None,
            base_step: None,
            max_step: None,
            datacenter_ids: None,
        };

        let result = manager.update_biz_tag_config(&request).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            crate::core::CoreError::NotFound(_)
        ));
    }

    // ============ batch_update_biz_tag_config ============

    /// 验证 batch_update_biz_tag_config 在空列表时返回 InvalidInput
    #[tokio::test]
    async fn test_batch_update_biz_tag_config_with_empty_list_returns_invalid_input() {
        let mut repo = MockRepository::new();
        repo.expect_get_biz_tag_by_workspace_group_and_name().times(0);
        repo.expect_update_biz_tag().times(0);

        let manager = WorkspaceConfigManager::new(Arc::new(repo));
        let result = manager.batch_update_biz_tag_config(vec![]).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            crate::core::CoreError::InvalidInput(_)
        ));
    }

    /// 验证 batch_update_biz_tag_config 在含空 biz_tag 时立即返回 InvalidInput
    #[tokio::test]
    async fn test_batch_update_biz_tag_config_with_empty_biz_tag_returns_invalid_input() {
        let mut repo = MockRepository::new();
        repo.expect_get_biz_tag_by_workspace_group_and_name().times(0);
        repo.expect_update_biz_tag().times(0);

        let manager = WorkspaceConfigManager::new(Arc::new(repo));
        let request = DynamicConfigRequest {
            workspace_id: Uuid::new_v4(),
            group_id: Some(Uuid::new_v4()),
            biz_tag: "  ".to_string(),
            algorithm: None,
            format: None,
            prefix: None,
            base_step: None,
            max_step: None,
            datacenter_ids: None,
        };

        let result = manager.batch_update_biz_tag_config(vec![request]).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            crate::core::CoreError::InvalidInput(_)
        ));
    }

    /// 验证 batch_update_biz_tag_config 在 group_id=None 时返回 InvalidInput
    #[tokio::test]
    async fn test_batch_update_biz_tag_config_without_group_id_returns_invalid_input() {
        let mut repo = MockRepository::new();
        repo.expect_get_biz_tag_by_workspace_group_and_name().times(0);
        repo.expect_update_biz_tag().times(0);

        let manager = WorkspaceConfigManager::new(Arc::new(repo));
        let request = DynamicConfigRequest {
            workspace_id: Uuid::new_v4(),
            group_id: None,
            biz_tag: "tag1".to_string(),
            algorithm: None,
            format: None,
            prefix: None,
            base_step: None,
            max_step: None,
            datacenter_ids: None,
        };

        let result = manager.batch_update_biz_tag_config(vec![request]).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            crate::core::CoreError::InvalidInput(_)
        ));
    }

    /// 验证 batch_update_biz_tag_config 在合法批量请求下委托给 DynamicConfigService 完成全部更新
    #[tokio::test]
    async fn test_batch_update_biz_tag_config_with_valid_requests_delegates_to_service() {
        let mut repo = MockRepository::new();
        let workspace_id = Uuid::new_v4();
        let group_id = Uuid::new_v4();
        let biz_tag_id_1 = Uuid::new_v4();
        let biz_tag_id_2 = Uuid::new_v4();

        let existing1 = sample_biz_tag(biz_tag_id_1, workspace_id, group_id, "tag1");
        let existing2 = sample_biz_tag(biz_tag_id_2, workspace_id, group_id, "tag2");

        repo.expect_get_biz_tag_by_workspace_group_and_name()
            .with(
                predicate::eq(workspace_id),
                predicate::eq(group_id),
                predicate::eq("tag1".to_string()),
            )
            .times(1)
            .returning(move |_, _, _| Ok(Some(existing1.clone())));

        repo.expect_get_biz_tag_by_workspace_group_and_name()
            .with(
                predicate::eq(workspace_id),
                predicate::eq(group_id),
                predicate::eq("tag2".to_string()),
            )
            .times(1)
            .returning(move |_, _, _| Ok(Some(existing2.clone())));

        // 两次 update_biz_tag 调用，分别返回更新后的值
        repo.expect_update_biz_tag()
            .times(1)
            .with(predicate::eq(biz_tag_id_1), predicate::always())
            .returning(move |_, _| Ok(sample_biz_tag(biz_tag_id_1, workspace_id, group_id, "tag1")));
        repo.expect_update_biz_tag()
            .times(1)
            .with(predicate::eq(biz_tag_id_2), predicate::always())
            .returning(move |_, _| Ok(sample_biz_tag(biz_tag_id_2, workspace_id, group_id, "tag2")));

        let manager = WorkspaceConfigManager::new(Arc::new(repo));
        let requests = vec![
            DynamicConfigRequest {
                workspace_id,
                group_id: Some(group_id),
                biz_tag: "tag1".to_string(),
                algorithm: Some(AlgorithmType::Segment),
                format: Some(IdFormat::Numeric),
                prefix: None,
                base_step: Some(100),
                max_step: Some(1000),
                datacenter_ids: Some(vec![0]),
            },
            DynamicConfigRequest {
                workspace_id,
                group_id: Some(group_id),
                biz_tag: "tag2".to_string(),
                algorithm: Some(AlgorithmType::Snowflake),
                format: Some(IdFormat::Prefixed),
                prefix: Some("P".to_string()),
                base_step: Some(200),
                max_step: Some(2000),
                datacenter_ids: Some(vec![1]),
            },
        ];

        let result = manager.batch_update_biz_tag_config(requests).await;
        assert!(result.is_ok());
        let responses = result.unwrap();
        assert_eq!(responses.len(), 2);
        assert_eq!(responses[0].biz_tag, "tag1");
        assert_eq!(responses[1].biz_tag, "tag2");
        assert_eq!(responses[0].biz_tag_id, biz_tag_id_1);
        assert_eq!(responses[1].biz_tag_id, biz_tag_id_2);
    }

    /// 验证 batch_update_biz_tag_config 在中途遇到 BizTag 不存在时返回 NotFound
    #[tokio::test]
    async fn test_batch_update_biz_tag_config_propagates_not_found_midway() {
        let mut repo = MockRepository::new();
        let workspace_id = Uuid::new_v4();
        let group_id = Uuid::new_v4();
        let biz_tag_id_1 = Uuid::new_v4();

        let existing1 = sample_biz_tag(biz_tag_id_1, workspace_id, group_id, "tag1");

        repo.expect_get_biz_tag_by_workspace_group_and_name()
            .with(
                predicate::eq(workspace_id),
                predicate::eq(group_id),
                predicate::eq("tag1".to_string()),
            )
            .times(1)
            .returning(move |_, _, _| Ok(Some(existing1.clone())));

        repo.expect_update_biz_tag()
            .times(1)
            .returning(move |_, _| Ok(sample_biz_tag(biz_tag_id_1, workspace_id, group_id, "tag1")));

        // 第二个 BizTag 不存在
        repo.expect_get_biz_tag_by_workspace_group_and_name()
            .with(
                predicate::eq(workspace_id),
                predicate::eq(group_id),
                predicate::eq("missing".to_string()),
            )
            .times(1)
            .returning(|_, _, _| Ok(None));

        let manager = WorkspaceConfigManager::new(Arc::new(repo));
        let requests = vec![
            DynamicConfigRequest {
                workspace_id,
                group_id: Some(group_id),
                biz_tag: "tag1".to_string(),
                algorithm: None,
                format: None,
                prefix: None,
                base_step: None,
                max_step: None,
                datacenter_ids: None,
            },
            DynamicConfigRequest {
                workspace_id,
                group_id: Some(group_id),
                biz_tag: "missing".to_string(),
                algorithm: None,
                format: None,
                prefix: None,
                base_step: None,
                max_step: None,
                datacenter_ids: None,
            },
        ];

        let result = manager.batch_update_biz_tag_config(requests).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            crate::core::CoreError::NotFound(_)
        ));
    }
}
