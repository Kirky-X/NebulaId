use std::sync::Arc;
use uuid::Uuid;

use crate::database::{
    BizTag, BizTagRepository, CreateBizTagRequest, CreateGroupRequest, CreateWorkspaceRequest,
    Group, GroupRepository, UpdateBizTagRequest, UpdateGroupRequest, UpdateWorkspaceRequest,
    Workspace, WorkspaceRepository,
};
use crate::dynamic_config::{DynamicConfigRequest, DynamicConfigResponse, DynamicConfigService};
use crate::types::Result;

/// 配置管理服务，提供对工作空间、组和业务标签的管理功能
///
/// 该服务提供对ID生成系统的配置管理，包括工作空间、组和业务标签的CRUD操作，
/// 以及动态配置更新功能。
pub struct ConfigManagementService<R>
where
    R: WorkspaceRepository + GroupRepository + BizTagRepository + Send + Sync,
{
    repository: Arc<R>,
}

impl<R> ConfigManagementService<R>
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
            return Err(crate::CoreError::InvalidInput(
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
                return Err(crate::CoreError::InvalidInput(
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
            return Err(crate::CoreError::InvalidInput(
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
                return Err(crate::CoreError::InvalidInput(
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
            return Err(crate::CoreError::InvalidInput(
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
                return Err(crate::CoreError::InvalidInput(
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
            return Err(crate::CoreError::InvalidInput(
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
                return Err(crate::CoreError::InvalidInput(
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
    use crate::database::{
        AlgorithmType, BizTag, CreateBizTagRequest, CreateGroupRequest, CreateWorkspaceRequest,
        Group, IdFormat, UpdateBizTagRequest, UpdateGroupRequest, UpdateWorkspaceRequest,
        Workspace, WorkspaceStatus,
    };
    use mockall::{mock, predicate};
    use std::sync::Arc;
    use uuid::Uuid;

    // 创建模拟仓库实现用于测试
    mock! {
        pub TestRepository {}

        #[async_trait]
        impl WorkspaceRepository for TestRepository {
            async fn create_workspace(&self, workspace: &CreateWorkspaceRequest) -> Result<Workspace>;
            async fn get_workspace(&self, id: Uuid) -> Result<Option<Workspace>>;
            async fn get_workspace_by_name(&self, name: &str) -> Result<Option<Workspace>>;
            async fn update_workspace(&self, id: Uuid, workspace: &UpdateWorkspaceRequest) -> Result<Workspace>;
            async fn delete_workspace(&self, id: Uuid) -> Result<()>;
            async fn list_workspaces(&self, limit: Option<u32>, offset: Option<u32>) -> Result<Vec<Workspace>>;
        }

        #[async_trait]
        impl GroupRepository for TestRepository {
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
        impl BizTagRepository for TestRepository {
            async fn create_biz_tag(&self, biz_tag: &CreateBizTagRequest) -> Result<BizTag>;
            async fn get_biz_tag(&self, id: Uuid) -> Result<Option<BizTag>>;
            async fn get_biz_tag_by_workspace_group_and_name(&self, workspace_id: Uuid, group_id: Uuid, name: &str) -> Result<Option<BizTag>>;
            async fn update_biz_tag(&self, id: Uuid, biz_tag: &UpdateBizTagRequest) -> Result<BizTag>;
            async fn delete_biz_tag(&self, id: Uuid) -> Result<()>;
            async fn list_biz_tags(&self, workspace_id: Uuid, group_id: Option<Uuid>, limit: Option<u32>, offset: Option<u32>) -> Result<Vec<BizTag>>;
            async fn list_biz_tags_by_workspace_group(&self, workspace_id: Uuid, group_id: Uuid) -> Result<Vec<BizTag>>;
            async fn count_biz_tags_by_group(&self, group_id: Uuid) -> Result<u64>;
        }
    }

    #[tokio::test]
    async fn test_create_workspace_with_valid_request() {
        let mut mock_repo = MockTestRepository::new();
        let workspace_request = CreateWorkspaceRequest {
            name: "Test Workspace".to_string(),
            description: Some("Test Description".to_string()),
            max_groups: Some(10),
            max_biz_tags: Some(100),
        };

        let expected_workspace = Workspace {
            id: Uuid::new_v4(),
            name: "Test Workspace".to_string(),
            description: Some("Test Description".to_string()),
            status: WorkspaceStatus::Active,
            max_groups: 10,
            max_biz_tags: 100,
            created_at: chrono::Utc::now().naive_utc(),
            updated_at: chrono::Utc::now().naive_utc(),
        };

        mock_repo
            .expect_create_workspace()
            .with(predicate::eq(workspace_request.clone()))
            .times(1)
            .returning(move |_| Ok(expected_workspace.clone()));

        let service = ConfigManagementService::new(Arc::new(mock_repo));
        let result = service.create_workspace(&workspace_request).await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_create_workspace_with_empty_name_should_fail() {
        let mock_repo = MockTestRepository::new();
        let workspace_request = CreateWorkspaceRequest {
            name: "".to_string(),
            description: Some("Test Description".to_string()),
            max_groups: Some(10),
            max_biz_tags: Some(100),
        };

        let service = ConfigManagementService::new(Arc::new(mock_repo));
        let result = service.create_workspace(&workspace_request).await;

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            crate::CoreError::InvalidInput(_)
        ));
    }

    #[tokio::test]
    async fn test_create_group_with_valid_request() {
        let mut mock_repo = MockTestRepository::new();
        let group_request = CreateGroupRequest {
            workspace_id: Uuid::new_v4(),
            name: "Test Group".to_string(),
            description: Some("Test Description".to_string()),
            max_biz_tags: Some(50),
        };

        let expected_group = Group {
            id: Uuid::new_v4(),
            workspace_id: group_request.workspace_id,
            name: "Test Group".to_string(),
            description: Some("Test Description".to_string()),
            max_biz_tags: 50,
            created_at: chrono::Utc::now().naive_utc(),
            updated_at: chrono::Utc::now().naive_utc(),
        };

        mock_repo
            .expect_create_group()
            .with(predicate::eq(group_request.clone()))
            .times(1)
            .returning(move |_| Ok(expected_group.clone()));

        let service = ConfigManagementService::new(Arc::new(mock_repo));
        let result = service.create_group(&group_request).await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_create_biz_tag_with_valid_request() {
        let mut mock_repo = MockTestRepository::new();
        let biz_tag_request = CreateBizTagRequest {
            workspace_id: Uuid::new_v4(),
            group_id: Uuid::new_v4(),
            name: "Test BizTag".to_string(),
            description: Some("Test Description".to_string()),
            algorithm: Some(AlgorithmType::Segment),
            format: Some(IdFormat::Numeric),
            prefix: Some("TEST".to_string()),
            base_step: Some(100),
            max_step: Some(1000),
            datacenter_ids: Some(vec![0]),
        };

        let expected_biz_tag = BizTag {
            id: Uuid::new_v4(),
            workspace_id: biz_tag_request.workspace_id,
            group_id: biz_tag_request.group_id,
            name: "Test BizTag".to_string(),
            description: Some("Test Description".to_string()),
            algorithm: AlgorithmType::Segment,
            format: IdFormat::Numeric,
            prefix: "TEST".to_string(),
            base_step: 100,
            max_step: 1000,
            datacenter_ids: vec![0],
            created_at: chrono::Utc::now().naive_utc(),
            updated_at: chrono::Utc::now().naive_utc(),
        };

        mock_repo
            .expect_create_biz_tag()
            .with(predicate::eq(biz_tag_request.clone()))
            .times(1)
            .returning(move |_| Ok(expected_biz_tag.clone()));

        let service = ConfigManagementService::new(Arc::new(mock_repo));
        let result = service.create_biz_tag(&biz_tag_request).await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_update_workspace_with_valid_request() {
        let mut mock_repo = MockTestRepository::new();
        let workspace_id = Uuid::new_v4();
        let update_request = UpdateWorkspaceRequest {
            name: Some("Updated Workspace".to_string()),
            description: Some("Updated Description".to_string()),
            status: None,
            max_groups: Some(20),
            max_biz_tags: Some(200),
        };

        let expected_workspace = Workspace {
            id: workspace_id,
            name: "Updated Workspace".to_string(),
            description: Some("Updated Description".to_string()),
            status: WorkspaceStatus::Active,
            max_groups: 20,
            max_biz_tags: 200,
            created_at: chrono::Utc::now().naive_utc(),
            updated_at: chrono::Utc::now().naive_utc(),
        };

        mock_repo
            .expect_update_workspace()
            .with(
                predicate::eq(workspace_id),
                predicate::eq(update_request.clone()),
            )
            .times(1)
            .returning(move |_, _| Ok(expected_workspace.clone()));

        let service = ConfigManagementService::new(Arc::new(mock_repo));
        let result = service
            .update_workspace(workspace_id, &update_request)
            .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_update_workspace_with_empty_name_should_fail() {
        let mock_repo = MockTestRepository::new();
        let workspace_id = Uuid::new_v4();
        let update_request = UpdateWorkspaceRequest {
            name: Some("".to_string()),
            description: Some("Updated Description".to_string()),
            status: None,
            max_groups: Some(20),
            max_biz_tags: Some(200),
        };

        let service = ConfigManagementService::new(Arc::new(mock_repo));
        let result = service
            .update_workspace(workspace_id, &update_request)
            .await;

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            crate::CoreError::InvalidInput(_)
        ));
    }

    #[tokio::test]
    async fn test_get_workspace() {
        let mut mock_repo = MockTestRepository::new();
        let workspace_id = Uuid::new_v4();
        let expected_workspace = Some(Workspace {
            id: workspace_id,
            name: "Test Workspace".to_string(),
            description: Some("Test Description".to_string()),
            status: WorkspaceStatus::Active,
            max_groups: 10,
            max_biz_tags: 100,
            created_at: chrono::Utc::now().naive_utc(),
            updated_at: chrono::Utc::now().naive_utc(),
        });

        mock_repo
            .expect_get_workspace()
            .with(predicate::eq(workspace_id))
            .times(1)
            .returning(move |_| Ok(expected_workspace.clone()));

        let service = ConfigManagementService::new(Arc::new(mock_repo));
        let result = service.get_workspace(workspace_id).await;

        assert!(result.is_ok());
        assert!(result.unwrap().is_some());
    }

    #[tokio::test]
    async fn test_delete_workspace() {
        let mut mock_repo = MockTestRepository::new();
        let workspace_id = Uuid::new_v4();

        mock_repo
            .expect_delete_workspace()
            .with(predicate::eq(workspace_id))
            .times(1)
            .returning(move |_| Ok(()));

        let service = ConfigManagementService::new(Arc::new(mock_repo));
        let result = service.delete_workspace(workspace_id).await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_update_group_with_valid_request() {
        let mut mock_repo = MockTestRepository::new();
        let group_id = Uuid::new_v4();
        let workspace_id = Uuid::new_v4();
        let update_request = UpdateGroupRequest {
            name: Some("Updated Group".to_string()),
            description: Some("Updated Description".to_string()),
            max_biz_tags: Some(50),
        };

        let expected_group = Group {
            id: group_id,
            workspace_id,
            name: "Updated Group".to_string(),
            description: Some("Updated Description".to_string()),
            max_biz_tags: 50,
            created_at: chrono::Utc::now().naive_utc(),
            updated_at: chrono::Utc::now().naive_utc(),
        };

        mock_repo
            .expect_update_group()
            .with(
                predicate::eq(group_id),
                predicate::eq(update_request.clone()),
            )
            .times(1)
            .returning(move |_, _| Ok(expected_group.clone()));

        let service = ConfigManagementService::new(Arc::new(mock_repo));
        let result = service.update_group(group_id, &update_request).await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_update_group_with_empty_name_should_fail() {
        let mock_repo = MockTestRepository::new();
        let group_id = Uuid::new_v4();
        let update_request = UpdateGroupRequest {
            name: Some("".to_string()),
            description: Some("Updated Description".to_string()),
            max_biz_tags: Some(50),
        };

        let service = ConfigManagementService::new(Arc::new(mock_repo));
        let result = service.update_group(group_id, &update_request).await;

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            crate::CoreError::InvalidInput(_)
        ));
    }

    #[tokio::test]
    async fn test_update_biz_tag_with_valid_request() {
        let mut mock_repo = MockTestRepository::new();
        let biz_tag_id = Uuid::new_v4();
        let update_request = UpdateBizTagRequest {
            name: Some("Updated BizTag".to_string()),
            description: Some("Updated Description".to_string()),
            algorithm: Some(AlgorithmType::Snowflake),
            format: Some(IdFormat::Prefixed),
            prefix: Some("UPD".to_string()),
            base_step: Some(200),
            max_step: Some(2000),
            datacenter_ids: Some(vec![1, 2]),
        };

        let expected_biz_tag = BizTag {
            id: biz_tag_id,
            workspace_id: Uuid::new_v4(),
            group_id: Uuid::new_v4(),
            name: "Updated BizTag".to_string(),
            description: Some("Updated Description".to_string()),
            algorithm: AlgorithmType::Snowflake,
            format: IdFormat::Prefixed,
            prefix: "UPD".to_string(),
            base_step: 200,
            max_step: 2000,
            datacenter_ids: vec![1, 2],
            created_at: chrono::Utc::now().naive_utc(),
            updated_at: chrono::Utc::now().naive_utc(),
        };

        mock_repo
            .expect_update_biz_tag()
            .with(
                predicate::eq(biz_tag_id),
                predicate::eq(update_request.clone()),
            )
            .times(1)
            .returning(move |_, _| Ok(expected_biz_tag.clone()));

        let service = ConfigManagementService::new(Arc::new(mock_repo));
        let result = service.update_biz_tag(biz_tag_id, &update_request).await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_update_biz_tag_with_empty_name_should_fail() {
        let mock_repo = MockTestRepository::new();
        let biz_tag_id = Uuid::new_v4();
        let update_request = UpdateBizTagRequest {
            name: Some("".to_string()),
            description: Some("Updated Description".to_string()),
            algorithm: Some(AlgorithmType::Snowflake),
            format: Some(IdFormat::Prefixed),
            prefix: Some("UPD".to_string()),
            base_step: Some(200),
            max_step: Some(2000),
            datacenter_ids: Some(vec![1, 2]),
        };

        let service = ConfigManagementService::new(Arc::new(mock_repo));
        let result = service.update_biz_tag(biz_tag_id, &update_request).await;

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            crate::CoreError::InvalidInput(_)
        ));
    }

    #[tokio::test]
    async fn test_update_biz_tag_config_with_valid_request() {
        let mut mock_repo = MockTestRepository::new();
        let workspace_id = Uuid::new_v4();
        let group_id = Uuid::new_v4();
        let request = DynamicConfigRequest {
            workspace_id,
            group_id: Some(group_id),
            biz_tag: "test_biz_tag".to_string(),
            algorithm: Some(AlgorithmType::Segment),
            format: Some(IdFormat::Numeric),
            prefix: Some("TEST".to_string()),
            base_step: Some(100),
            max_step: Some(1000),
            datacenter_ids: Some(vec![0]),
        };

        let _expected_response = DynamicConfigResponse {
            workspace_id,
            group_id,
            biz_tag_id: Uuid::new_v4(),
            biz_tag: "test_biz_tag".to_string(),
            algorithm: AlgorithmType::Segment,
            format: IdFormat::Numeric,
            prefix: "TEST".to_string(),
            base_step: 100,
            max_step: 1000,
            datacenter_ids: vec![0],
        };

        // Mock the get_biz_tag_by_workspace_group_and_name call
        mock_repo
            .expect_get_biz_tag_by_workspace_group_and_name()
            .with(
                predicate::eq(workspace_id),
                predicate::eq(group_id),
                predicate::eq("test_biz_tag".to_string()),
            )
            .times(1)
            .returning(move |_, _, _| {
                Ok(Some(BizTag {
                    id: Uuid::new_v4(),
                    workspace_id,
                    group_id,
                    name: "test_biz_tag".to_string(),
                    description: Some("Test BizTag".to_string()),
                    algorithm: AlgorithmType::Segment,
                    format: IdFormat::Numeric,
                    prefix: "OLD".to_string(),
                    base_step: 50,
                    max_step: 500,
                    datacenter_ids: vec![0],
                    created_at: chrono::Utc::now().naive_utc(),
                    updated_at: chrono::Utc::now().naive_utc(),
                }))
            });

        // Mock the update_biz_tag call
        mock_repo
            .expect_update_biz_tag()
            .times(1)
            .returning(move |_, _| {
                Ok(BizTag {
                    id: Uuid::new_v4(),
                    workspace_id,
                    group_id,
                    name: "test_biz_tag".to_string(),
                    description: Some("Test BizTag".to_string()),
                    algorithm: AlgorithmType::Segment,
                    format: IdFormat::Numeric,
                    prefix: "TEST".to_string(),
                    base_step: 100,
                    max_step: 1000,
                    datacenter_ids: vec![0],
                    created_at: chrono::Utc::now().naive_utc(),
                    updated_at: chrono::Utc::now().naive_utc(),
                })
            });

        let service = ConfigManagementService::new(Arc::new(mock_repo));
        let result = service.update_biz_tag_config(&request).await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_update_biz_tag_config_with_empty_biz_tag_name_should_fail() {
        let mock_repo = MockTestRepository::new();
        let request = DynamicConfigRequest {
            workspace_id: Uuid::new_v4(),
            group_id: Some(Uuid::new_v4()),
            biz_tag: "".to_string(),
            algorithm: Some(AlgorithmType::Segment),
            format: Some(IdFormat::Numeric),
            prefix: Some("TEST".to_string()),
            base_step: Some(100),
            max_step: Some(1000),
            datacenter_ids: Some(vec![0]),
        };

        let service = ConfigManagementService::new(Arc::new(mock_repo));
        let result = service.update_biz_tag_config(&request).await;

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            crate::CoreError::InvalidInput(_)
        ));
    }

    #[tokio::test]
    async fn test_batch_update_biz_tag_config_with_valid_requests() {
        let mut mock_repo = MockTestRepository::new();
        let workspace_id = Uuid::new_v4();
        let group_id = Uuid::new_v4();
        let requests = vec![
            DynamicConfigRequest {
                workspace_id,
                group_id: Some(group_id),
                biz_tag: "test_biz_tag1".to_string(),
                algorithm: Some(AlgorithmType::Segment),
                format: Some(IdFormat::Numeric),
                prefix: Some("TEST".to_string()),
                base_step: Some(100),
                max_step: Some(1000),
                datacenter_ids: Some(vec![0]),
            },
            DynamicConfigRequest {
                workspace_id,
                group_id: Some(group_id),
                biz_tag: "test_biz_tag2".to_string(),
                algorithm: Some(AlgorithmType::Snowflake),
                format: Some(IdFormat::Prefixed),
                prefix: Some("SNOW".to_string()),
                base_step: Some(200),
                max_step: Some(2000),
                datacenter_ids: Some(vec![1]),
            },
        ];

        mock_repo
            .expect_get_biz_tag_by_workspace_group_and_name()
            .with(
                predicate::eq(workspace_id),
                predicate::eq(group_id),
                predicate::eq("test_biz_tag1".to_string()),
            )
            .times(1)
            .returning(move |_, _, _| {
                Ok(Some(BizTag {
                    id: Uuid::new_v4(),
                    workspace_id,
                    group_id,
                    name: "test_biz_tag1".to_string(),
                    description: Some("Test BizTag".to_string()),
                    algorithm: AlgorithmType::Segment,
                    format: IdFormat::Numeric,
                    prefix: "TEST".to_string(),
                    base_step: 100,
                    max_step: 1000,
                    datacenter_ids: vec![0],
                    created_at: chrono::Utc::now().naive_utc(),
                    updated_at: chrono::Utc::now().naive_utc(),
                }))
            });

        mock_repo
            .expect_get_biz_tag_by_workspace_group_and_name()
            .with(
                predicate::eq(workspace_id),
                predicate::eq(group_id),
                predicate::eq("test_biz_tag2".to_string()),
            )
            .times(1)
            .returning(move |_, _, _| {
                Ok(Some(BizTag {
                    id: Uuid::new_v4(),
                    workspace_id,
                    group_id,
                    name: "test_biz_tag2".to_string(),
                    description: Some("Test BizTag".to_string()),
                    algorithm: AlgorithmType::Snowflake,
                    format: IdFormat::Prefixed,
                    prefix: "SNOW".to_string(),
                    base_step: 200,
                    max_step: 2000,
                    datacenter_ids: vec![1],
                    created_at: chrono::Utc::now().naive_utc(),
                    updated_at: chrono::Utc::now().naive_utc(),
                }))
            });

        mock_repo
            .expect_update_biz_tag()
            .times(2)
            .returning(|_, _| {
                Ok(BizTag {
                    id: Uuid::new_v4(),
                    workspace_id: Uuid::new_v4(),
                    group_id: Uuid::new_v4(),
                    name: "test_biz_tag".to_string(),
                    description: Some("Test BizTag".to_string()),
                    algorithm: AlgorithmType::Segment,
                    format: IdFormat::Numeric,
                    prefix: "TEST".to_string(),
                    base_step: 100,
                    max_step: 1000,
                    datacenter_ids: vec![0],
                    created_at: chrono::Utc::now().naive_utc(),
                    updated_at: chrono::Utc::now().naive_utc(),
                })
            });

        let service = ConfigManagementService::new(Arc::new(mock_repo));
        let result = service.batch_update_biz_tag_config(requests).await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn test_batch_update_biz_tag_config_with_empty_request_list_should_fail() {
        let mock_repo = MockTestRepository::new();
        let requests = vec![];

        let service = ConfigManagementService::new(Arc::new(mock_repo));
        let result = service.batch_update_biz_tag_config(requests).await;

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            crate::CoreError::InvalidInput(_)
        ));
    }

    #[tokio::test]
    async fn test_batch_update_biz_tag_config_with_empty_biz_tag_name_should_fail() {
        let mock_repo = MockTestRepository::new();
        let workspace_id = Uuid::new_v4();
        let group_id = Uuid::new_v4();
        let requests = vec![DynamicConfigRequest {
            workspace_id,
            group_id: Some(group_id),
            biz_tag: "".to_string(),
            algorithm: Some(AlgorithmType::Segment),
            format: Some(IdFormat::Numeric),
            prefix: Some("TEST".to_string()),
            base_step: Some(100),
            max_step: Some(1000),
            datacenter_ids: Some(vec![0]),
        }];

        let service = ConfigManagementService::new(Arc::new(mock_repo));
        let result = service.batch_update_biz_tag_config(requests).await;

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            crate::CoreError::InvalidInput(_)
        ));
    }

    #[tokio::test]
    async fn test_list_workspaces() {
        let mut mock_repo = MockTestRepository::new();
        let expected_workspaces = vec![
            Workspace {
                id: Uuid::new_v4(),
                name: "Test Workspace 1".to_string(),
                description: Some("Test Description 1".to_string()),
                status: WorkspaceStatus::Active,
                max_groups: 10,
                max_biz_tags: 100,
                created_at: chrono::Utc::now().naive_utc(),
                updated_at: chrono::Utc::now().naive_utc(),
            },
            Workspace {
                id: Uuid::new_v4(),
                name: "Test Workspace 2".to_string(),
                description: Some("Test Description 2".to_string()),
                status: WorkspaceStatus::Active,
                max_groups: 10,
                max_biz_tags: 100,
                created_at: chrono::Utc::now().naive_utc(),
                updated_at: chrono::Utc::now().naive_utc(),
            },
        ];

        mock_repo
            .expect_list_workspaces()
            .with(predicate::eq(Some(10)), predicate::eq(Some(0)))
            .times(1)
            .returning(move |_, _| Ok(expected_workspaces.clone()));

        let service = ConfigManagementService::new(Arc::new(mock_repo));
        let result = service.list_workspaces(Some(10), Some(0)).await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn test_list_groups() {
        let mut mock_repo = MockTestRepository::new();
        let workspace_id = Uuid::new_v4();
        let expected_groups = vec![
            Group {
                id: Uuid::new_v4(),
                workspace_id,
                name: "Test Group 1".to_string(),
                description: Some("Test Description 1".to_string()),
                max_biz_tags: 50,
                created_at: chrono::Utc::now().naive_utc(),
                updated_at: chrono::Utc::now().naive_utc(),
            },
            Group {
                id: Uuid::new_v4(),
                workspace_id,
                name: "Test Group 2".to_string(),
                description: Some("Test Description 2".to_string()),
                max_biz_tags: 50,
                created_at: chrono::Utc::now().naive_utc(),
                updated_at: chrono::Utc::now().naive_utc(),
            },
        ];

        mock_repo
            .expect_list_groups()
            .with(
                predicate::eq(workspace_id),
                predicate::eq(Some(10)),
                predicate::eq(Some(0)),
            )
            .times(1)
            .returning(move |_, _, _| Ok(expected_groups.clone()));

        let service = ConfigManagementService::new(Arc::new(mock_repo));
        let result = service.list_groups(workspace_id, Some(10), Some(0)).await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn test_list_biz_tags() {
        let mut mock_repo = MockTestRepository::new();
        let workspace_id = Uuid::new_v4();
        let group_id = Uuid::new_v4();
        let expected_biz_tags = vec![
            BizTag {
                id: Uuid::new_v4(),
                workspace_id,
                group_id,
                name: "Test BizTag 1".to_string(),
                description: Some("Test Description 1".to_string()),
                algorithm: AlgorithmType::Segment,
                format: IdFormat::Numeric,
                prefix: "TEST".to_string(),
                base_step: 100,
                max_step: 1000,
                datacenter_ids: vec![0],
                created_at: chrono::Utc::now().naive_utc(),
                updated_at: chrono::Utc::now().naive_utc(),
            },
            BizTag {
                id: Uuid::new_v4(),
                workspace_id,
                group_id,
                name: "Test BizTag 2".to_string(),
                description: Some("Test Description 2".to_string()),
                algorithm: AlgorithmType::Snowflake,
                format: IdFormat::Prefixed,
                prefix: "SNOW".to_string(),
                base_step: 200,
                max_step: 2000,
                datacenter_ids: vec![1],
                created_at: chrono::Utc::now().naive_utc(),
                updated_at: chrono::Utc::now().naive_utc(),
            },
        ];

        mock_repo
            .expect_list_biz_tags()
            .with(
                predicate::eq(workspace_id),
                predicate::eq(Some(group_id)),
                predicate::eq(Some(10)),
                predicate::eq(Some(0)),
            )
            .times(1)
            .returning(move |_, _, _, _| Ok(expected_biz_tags.clone()));

        let service = ConfigManagementService::new(Arc::new(mock_repo));
        let result = service
            .list_biz_tags(workspace_id, Some(group_id), Some(10), Some(0))
            .await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap().len(), 2);
    }
}
