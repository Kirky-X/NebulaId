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
