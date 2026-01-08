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

#![allow(dead_code)]

use async_trait::async_trait;
use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use rand::Rng;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QuerySelect, Set,
    TransactionTrait,
};
use serde_json::to_string;
use sha2::Digest;
use subtle::ConstantTimeEq;
use tracing::{debug, info};
use uuid::Uuid;

use crate::coordinator::{LockError, LockGuard};
use crate::database::api_key_entity::{
    ActiveModel as ApiKeyActiveModel, ApiKey as ApiKeyInfo, ApiKeyResponse, ApiKeyRole,
    ApiKeyWithSecret, Column as ApiKeyColumn, CreateApiKeyRequest, Entity as ApiKeyEntity,
    Model as ApiKeyModel,
};
use crate::database::biz_tag_entity::{
    ActiveModel as BizTagActiveModel, Column as BizTagColumn, Entity as BizTagEntity,
};
use crate::database::group_entity::{
    ActiveModel as GroupActiveModel, Column as GroupColumn, Entity as GroupEntity,
};
use crate::database::segment_entity::{
    ActiveModel as SegmentActiveModel, Column as SegmentColumn, Entity as SegmentEntity,
};
use crate::database::workspace_entity::{
    ActiveModel as WorkspaceActiveModel, Column as WorkspaceColumn, Entity as WorkspaceEntity,
    Workspace,
};
use crate::types::{Result, SegmentInfo};

#[async_trait]
pub trait SegmentRepository: Send + Sync {
    async fn get_segment(&self, workspace_id: &str, biz_tag: &str) -> Result<Option<SegmentInfo>>;
    async fn allocate_segment(
        &self,
        workspace_id: &str,
        biz_tag: &str,
        step: i32,
    ) -> Result<SegmentInfo>;
    async fn allocate_segment_with_dc(
        &self,
        workspace_id: &str,
        biz_tag: &str,
        step: i32,
        dc_id: i32,
    ) -> Result<SegmentInfo>;
    async fn update_segment(
        &self,
        workspace_id: &str,
        biz_tag: &str,
        current_id: i64,
        max_id: i64,
    ) -> Result<()>;
    async fn create_segment(
        &self,
        workspace_id: &str,
        biz_tag: &str,
        start_id: i64,
        max_id: i64,
        step: i32,
        delta: i32,
    ) -> Result<SegmentInfo>;
    async fn list_segments(&self, workspace_id: &str) -> Result<Vec<SegmentInfo>>;
    async fn delete_segment(&self, workspace_id: &str, biz_tag: &str) -> Result<()>;
}

#[async_trait]
pub trait WorkspaceRepository: Send + Sync {
    async fn create_workspace(&self, workspace: &CreateWorkspaceRequest) -> Result<Workspace>;
    async fn get_workspace(&self, id: Uuid) -> Result<Option<Workspace>>;
    async fn get_workspace_by_name(&self, name: &str) -> Result<Option<Workspace>>;
    async fn update_workspace(
        &self,
        id: Uuid,
        workspace: &UpdateWorkspaceRequest,
    ) -> Result<Workspace>;
    async fn delete_workspace(&self, id: Uuid) -> Result<()>;
    async fn list_workspaces(
        &self,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> Result<Vec<Workspace>>;
    async fn get_workspace_with_groups(&self, id: Uuid) -> Result<Option<(Workspace, Vec<Group>)>>;
    async fn get_workspace_with_groups_and_biz_tags(
        &self,
        id: Uuid,
    ) -> Result<Option<(Workspace, Vec<(Group, Vec<BizTag>)>)>>;
}

#[async_trait]
pub trait GroupRepository: Send + Sync {
    async fn create_group(&self, group: &CreateGroupRequest) -> Result<Group>;
    async fn get_group(&self, id: Uuid) -> Result<Option<Group>>;
    async fn get_group_by_workspace_and_name(
        &self,
        workspace_id: Uuid,
        name: &str,
    ) -> Result<Option<Group>>;
    async fn update_group(&self, id: Uuid, group: &UpdateGroupRequest) -> Result<Group>;
    async fn delete_group(&self, id: Uuid) -> Result<()>;
    async fn list_groups(
        &self,
        workspace_id: Uuid,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> Result<Vec<Group>>;
    async fn get_group_with_biz_tags(&self, id: Uuid) -> Result<Option<(Group, Vec<BizTag>)>>;
    async fn delete_group_with_biz_tags(&self, id: Uuid) -> Result<()>;
}

#[async_trait]
pub trait BizTagRepository: Send + Sync {
    async fn create_biz_tag(&self, biz_tag: &CreateBizTagRequest) -> Result<BizTag>;
    async fn get_biz_tag(&self, id: Uuid) -> Result<Option<BizTag>>;
    async fn get_biz_tag_by_workspace_group_and_name(
        &self,
        workspace_id: Uuid,
        group_id: Uuid,
        name: &str,
    ) -> Result<Option<BizTag>>;
    async fn update_biz_tag(&self, id: Uuid, biz_tag: &UpdateBizTagRequest) -> Result<BizTag>;
    async fn delete_biz_tag(&self, id: Uuid) -> Result<()>;
    async fn list_biz_tags(
        &self,
        workspace_id: Uuid,
        group_id: Option<Uuid>,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> Result<Vec<BizTag>>;
    async fn list_biz_tags_by_workspace_group(
        &self,
        workspace_id: Uuid,
        group_id: Uuid,
    ) -> Result<Vec<BizTag>>;
    async fn count_biz_tags_by_group(&self, group_id: Uuid) -> Result<u64>;
    async fn count_biz_tags(&self, workspace_id: Uuid, group_id: Option<Uuid>) -> Result<u64>;
    async fn health_check(&self) -> Result<()>;
}

#[async_trait]
pub trait ApiKeyRepository: Send + Sync {
    async fn create_api_key(&self, request: &CreateApiKeyRequest) -> Result<ApiKeyWithSecret>;
    async fn get_api_key_by_id(&self, key_id: &str) -> Result<Option<ApiKeyInfo>>;
    async fn validate_api_key(
        &self,
        key_id: &str,
        key_secret: &str,
    ) -> Result<Option<(Option<Uuid>, ApiKeyRole)>>; // workspace_id is Uuid
    async fn list_api_keys(
        &self,
        workspace_id: Uuid,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> Result<Vec<ApiKeyInfo>>;
    async fn delete_api_key(&self, id: Uuid) -> Result<()>;
    async fn revoke_api_key(&self, id: Uuid) -> Result<()>;
    async fn update_last_used(&self, id: Uuid) -> Result<()>; // Changed from String to Uuid
    async fn get_admin_api_key(&self, workspace_id: Uuid) -> Result<Option<ApiKeyInfo>>;
    async fn count_api_keys(&self, workspace_id: Uuid) -> Result<u64>;

    /// 轮换 API Key（生成新密钥，保持旧密钥在宽限期内有效）
    async fn rotate_api_key(
        &self,
        key_id: &str,
        grace_period_seconds: u64,
    ) -> Result<ApiKeyWithSecret>;

    /// 获取需要轮换的密钥列表（基于创建时间）
    async fn get_keys_older_than(&self, age_threshold_days: i64) -> Result<Vec<ApiKeyInfo>>;
}

use crate::database::biz_tag_entity::{BizTag, CreateBizTagRequest, UpdateBizTagRequest};
use crate::database::group_entity::{CreateGroupRequest, Group, UpdateGroupRequest};
use crate::database::workspace_entity::{CreateWorkspaceRequest, UpdateWorkspaceRequest};

pub struct SeaOrmRepository {
    db: sea_orm::DatabaseConnection,
    /// 分布式锁（可选，用于 segment 分配）
    #[cfg(feature = "etcd")]
    distributed_lock: Option<std::sync::Arc<dyn crate::coordinator::DistributedLock + Send + Sync>>,
    /// 本地分布式锁（无 etcd 时使用）
    #[cfg(not(feature = "etcd"))]
    distributed_lock: Option<std::sync::Arc<dyn crate::coordinator::DistributedLock + Send + Sync>>,
}

impl SeaOrmRepository {
    pub fn new(db: sea_orm::DatabaseConnection) -> Self {
        Self {
            db,
            distributed_lock: None,
        }
    }

    /// 设置分布式锁
    #[cfg(feature = "etcd")]
    pub fn with_lock(
        mut self,
        lock: std::sync::Arc<dyn crate::coordinator::DistributedLock + Send + Sync>,
    ) -> Self {
        self.distributed_lock = Some(lock);
        self
    }

    /// 设置分布式锁
    #[cfg(not(feature = "etcd"))]
    pub fn with_lock(
        mut self,
        lock: std::sync::Arc<dyn crate::coordinator::DistributedLock + Send + Sync>,
    ) -> Self {
        self.distributed_lock = Some(lock);
        self
    }

    /// 构建用于 segment 分配的分布式锁键
    fn segment_lock_key(&self, workspace_id: &str, biz_tag: &str, dc_id: Option<i32>) -> String {
        match dc_id {
            Some(dc) => format!("segment:{}:{}:dc:{}", workspace_id, biz_tag, dc),
            None => format!("segment:{}:{}", workspace_id, biz_tag),
        }
    }
}

#[async_trait]
impl WorkspaceRepository for SeaOrmRepository {
    async fn create_workspace(&self, workspace: &CreateWorkspaceRequest) -> Result<Workspace> {
        let new_workspace = WorkspaceActiveModel {
            id: Set(uuid::Uuid::new_v4()),
            name: Set(workspace.name.clone()),
            description: Set(workspace.description.clone()),
            status: Set(super::workspace_entity::WorkspaceStatus::Active.to_string()),
            max_groups: Set(workspace.max_groups.unwrap_or(10)), // 默认值
            max_biz_tags: Set(workspace.max_biz_tags.unwrap_or(100)), // 默认值
            created_at: Set(chrono::Utc::now().naive_utc()),
            updated_at: Set(chrono::Utc::now().naive_utc()),
        };

        let inserted = new_workspace
            .insert(&self.db)
            .await
            .map_err(|e| crate::CoreError::DatabaseError(e.to_string()))?;

        Ok(inserted.into())
    }

    async fn get_workspace(&self, id: Uuid) -> Result<Option<Workspace>> {
        let result = WorkspaceEntity::find_by_id(id)
            .one(&self.db)
            .await
            .map_err(|e| crate::CoreError::DatabaseError(e.to_string()))?;

        Ok(result.map(|m| m.into()))
    }

    async fn get_workspace_by_name(&self, name: &str) -> Result<Option<Workspace>> {
        let result = WorkspaceEntity::find()
            .filter(WorkspaceColumn::Name.eq(name))
            .one(&self.db)
            .await
            .map_err(|e| crate::CoreError::DatabaseError(e.to_string()))?;

        Ok(result.map(|m| m.into()))
    }

    async fn update_workspace(
        &self,
        id: Uuid,
        workspace: &UpdateWorkspaceRequest,
    ) -> Result<Workspace> {
        let existing = WorkspaceEntity::find_by_id(id)
            .one(&self.db)
            .await
            .map_err(|e| crate::CoreError::DatabaseError(e.to_string()))?;

        if existing.is_none() {
            return Err(crate::CoreError::NotFound(format!(
                "Workspace not found: {}",
                id
            )));
        }

        let existing = existing.unwrap();
        let updated = WorkspaceActiveModel {
            id: Set(existing.id),
            name: Set(workspace.name.clone().unwrap_or(existing.name)),
            description: Set(workspace.description.clone().or(existing.description)),
            status: Set(workspace
                .status
                .clone()
                .map(|s| s.into())
                .unwrap_or(existing.status)),
            max_groups: Set(workspace.max_groups.unwrap_or(existing.max_groups)),
            max_biz_tags: Set(workspace.max_biz_tags.unwrap_or(existing.max_biz_tags)),
            updated_at: Set(chrono::Utc::now().naive_utc()),
            ..Default::default()
        };

        let result = updated
            .update(&self.db)
            .await
            .map_err(|e| crate::CoreError::DatabaseError(e.to_string()))?;

        Ok(result.into())
    }

    async fn delete_workspace(&self, id: Uuid) -> Result<()> {
        let result = WorkspaceEntity::delete_by_id(id)
            .exec(&self.db)
            .await
            .map_err(|e| crate::CoreError::DatabaseError(e.to_string()))?;

        if result.rows_affected == 0 {
            return Err(crate::CoreError::NotFound(format!(
                "Workspace not found: {}",
                id
            )));
        }

        Ok(())
    }

    async fn list_workspaces(
        &self,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> Result<Vec<Workspace>> {
        let mut query = WorkspaceEntity::find();

        if let Some(limit) = limit {
            query = query.limit(limit as u64);
        }

        if let Some(offset) = offset {
            query = query.offset(offset as u64);
        }

        let results = query
            .all(&self.db)
            .await
            .map_err(|e| crate::CoreError::DatabaseError(e.to_string()))?;

        Ok(results.into_iter().map(|m| m.into()).collect())
    }

    async fn get_workspace_with_groups(&self, id: Uuid) -> Result<Option<(Workspace, Vec<Group>)>> {
        let workspace = WorkspaceEntity::find_by_id(id)
            .one(&self.db)
            .await
            .map_err(|e| crate::CoreError::DatabaseError(e.to_string()))?;

        if workspace.is_none() {
            return Ok(None);
        }

        let workspace: Workspace = workspace.unwrap().into();

        let groups = GroupEntity::find()
            .filter(GroupColumn::WorkspaceId.eq(id))
            .all(&self.db)
            .await
            .map_err(|e| crate::CoreError::DatabaseError(e.to_string()))?;

        let groups: Vec<Group> = groups.into_iter().map(|g| g.into()).collect();

        Ok(Some((workspace, groups)))
    }

    async fn get_workspace_with_groups_and_biz_tags(
        &self,
        id: Uuid,
    ) -> Result<Option<(Workspace, Vec<(Group, Vec<BizTag>)>)>> {
        // 使用预加载一次性获取 workspace, groups 和 biz_tags
        let workspace_with_relations = WorkspaceEntity::find_by_id(id)
            .find_also_related(GroupEntity)
            .all(&self.db)
            .await
            .map_err(|e| crate::CoreError::DatabaseError(e.to_string()))?;

        if workspace_with_relations.is_empty() {
            return Ok(None);
        }

        // 获取 workspace
        let (workspace_entity, _) = &workspace_with_relations[0];
        let workspace: Workspace = workspace_entity.clone().into();

        // 收集所有 group IDs
        let group_ids: Vec<Uuid> = workspace_with_relations
            .iter()
            .filter_map(|(_, group_opt)| group_opt.as_ref().map(|g| g.id))
            .collect();

        // 一次性查询所有 biz_tags
        let all_biz_tags_models: Vec<crate::database::biz_tag_entity::Model> = BizTagEntity::find()
            .filter(BizTagColumn::GroupId.is_in(group_ids))
            .all(&self.db)
            .await
            .map_err(|e| crate::CoreError::DatabaseError(e.to_string()))?;

        // 按 group_id 组织 biz_tags
        let mut biz_tags_by_group: std::collections::HashMap<Uuid, Vec<BizTag>> =
            std::collections::HashMap::new();
        for biz_tag_model in all_biz_tags_models {
            biz_tags_by_group
                .entry(biz_tag_model.group_id)
                .or_default()
                .push(biz_tag_model.into());
        }

        // 构建结果
        let mut result_groups: Vec<(Group, Vec<BizTag>)> = Vec::new();
        for (_, group_opt) in workspace_with_relations.iter() {
            if let Some(group) = group_opt {
                let biz_tags = biz_tags_by_group
                    .get(&group.id)
                    .cloned()
                    .unwrap_or_default();
                result_groups.push((group.clone().into(), biz_tags));
            }
        }

        Ok(Some((workspace, result_groups)))
    }
}

#[async_trait]
impl GroupRepository for SeaOrmRepository {
    async fn create_group(&self, group: &CreateGroupRequest) -> Result<Group> {
        // 检查工作空间是否存在
        let workspace_exists = WorkspaceEntity::find_by_id(group.workspace_id)
            .one(&self.db)
            .await
            .map_err(|e| crate::CoreError::DatabaseError(e.to_string()))?
            .is_some();

        if !workspace_exists {
            return Err(crate::CoreError::NotFound(format!(
                "Workspace not found: {}",
                group.workspace_id
            )));
        }

        let new_group = GroupActiveModel {
            id: Set(uuid::Uuid::new_v4()),
            workspace_id: Set(group.workspace_id),
            name: Set(group.name.clone()),
            description: Set(group.description.clone()),
            max_biz_tags: Set(group.max_biz_tags.unwrap_or(50)), // 默认值
            created_at: Set(chrono::Utc::now().naive_utc()),
            updated_at: Set(chrono::Utc::now().naive_utc()),
        };

        let inserted = new_group
            .insert(&self.db)
            .await
            .map_err(|e| crate::CoreError::DatabaseError(e.to_string()))?;

        Ok(inserted.into())
    }

    async fn get_group(&self, id: Uuid) -> Result<Option<Group>> {
        let result = GroupEntity::find_by_id(id)
            .one(&self.db)
            .await
            .map_err(|e| crate::CoreError::DatabaseError(e.to_string()))?;

        Ok(result.map(|m| m.into()))
    }

    async fn get_group_by_workspace_and_name(
        &self,
        workspace_id: Uuid,
        name: &str,
    ) -> Result<Option<Group>> {
        let result = GroupEntity::find()
            .filter(GroupColumn::WorkspaceId.eq(workspace_id))
            .filter(GroupColumn::Name.eq(name))
            .one(&self.db)
            .await
            .map_err(|e| crate::CoreError::DatabaseError(e.to_string()))?;

        Ok(result.map(|m| m.into()))
    }

    async fn update_group(&self, id: Uuid, group: &UpdateGroupRequest) -> Result<Group> {
        let existing = GroupEntity::find_by_id(id)
            .one(&self.db)
            .await
            .map_err(|e| crate::CoreError::DatabaseError(e.to_string()))?;

        if existing.is_none() {
            return Err(crate::CoreError::NotFound(format!(
                "Group not found: {}",
                id
            )));
        }

        let existing = existing.unwrap();
        let updated = GroupActiveModel {
            id: Set(existing.id),
            name: Set(group.name.clone().unwrap_or(existing.name)),
            description: Set(group.description.clone().or(existing.description)),
            max_biz_tags: Set(group.max_biz_tags.unwrap_or(existing.max_biz_tags)),
            updated_at: Set(chrono::Utc::now().naive_utc()),
            ..Default::default()
        };

        let result = updated
            .update(&self.db)
            .await
            .map_err(|e| crate::CoreError::DatabaseError(e.to_string()))?;

        Ok(result.into())
    }

    async fn delete_group(&self, id: Uuid) -> Result<()> {
        let result = GroupEntity::delete_by_id(id)
            .exec(&self.db)
            .await
            .map_err(|e| crate::CoreError::DatabaseError(e.to_string()))?;

        if result.rows_affected == 0 {
            return Err(crate::CoreError::NotFound(format!(
                "Group not found: {}",
                id
            )));
        }

        Ok(())
    }

    async fn list_groups(
        &self,
        workspace_id: Uuid,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> Result<Vec<Group>> {
        let mut query = GroupEntity::find().filter(GroupColumn::WorkspaceId.eq(workspace_id));

        if let Some(limit) = limit {
            query = query.limit(limit as u64);
        }

        if let Some(offset) = offset {
            query = query.offset(offset as u64);
        }

        let results = query
            .all(&self.db)
            .await
            .map_err(|e| crate::CoreError::DatabaseError(e.to_string()))?;

        Ok(results.into_iter().map(|m| m.into()).collect())
    }

    async fn get_group_with_biz_tags(&self, id: Uuid) -> Result<Option<(Group, Vec<BizTag>)>> {
        let group = GroupEntity::find_by_id(id)
            .one(&self.db)
            .await
            .map_err(|e| crate::CoreError::DatabaseError(e.to_string()))?;

        if group.is_none() {
            return Ok(None);
        }

        let group: Group = group.unwrap().into();

        let biz_tags = BizTagEntity::find()
            .filter(BizTagColumn::GroupId.eq(id))
            .all(&self.db)
            .await
            .map_err(|e| crate::CoreError::DatabaseError(e.to_string()))?;

        let biz_tags: Vec<BizTag> = biz_tags.into_iter().map(|b| b.into()).collect();

        Ok(Some((group, biz_tags)))
    }

    async fn delete_group_with_biz_tags(&self, id: Uuid) -> Result<()> {
        let txn = self
            .db
            .begin()
            .await
            .map_err(|e| crate::CoreError::DatabaseError(e.to_string()))?;

        let biz_tags = BizTagEntity::find()
            .filter(BizTagColumn::GroupId.eq(id))
            .all(&txn)
            .await
            .map_err(|e| crate::CoreError::DatabaseError(e.to_string()))?;

        for biz_tag in biz_tags {
            BizTagEntity::delete_by_id(biz_tag.id)
                .exec(&txn)
                .await
                .map_err(|e| crate::CoreError::DatabaseError(e.to_string()))?;
        }

        GroupEntity::delete_by_id(id)
            .exec(&txn)
            .await
            .map_err(|e| crate::CoreError::DatabaseError(e.to_string()))?;

        txn.commit()
            .await
            .map_err(|e| crate::CoreError::DatabaseError(e.to_string()))?;

        Ok(())
    }
}

#[async_trait]
impl BizTagRepository for SeaOrmRepository {
    async fn create_biz_tag(&self, biz_tag: &CreateBizTagRequest) -> Result<BizTag> {
        // 检查工作空间和组是否存在
        let workspace_exists = WorkspaceEntity::find_by_id(biz_tag.workspace_id)
            .one(&self.db)
            .await
            .map_err(|e| crate::CoreError::DatabaseError(e.to_string()))?
            .is_some();

        if !workspace_exists {
            return Err(crate::CoreError::NotFound(format!(
                "Workspace not found: {}",
                biz_tag.workspace_id
            )));
        }

        let group_exists = GroupEntity::find_by_id(biz_tag.group_id)
            .one(&self.db)
            .await
            .map_err(|e| crate::CoreError::DatabaseError(e.to_string()))?
            .is_some();

        if !group_exists {
            return Err(crate::CoreError::NotFound(format!(
                "Group not found: {}",
                biz_tag.group_id
            )));
        }

        let datacenter_ids_json = biz_tag.datacenter_ids.clone().unwrap_or(vec![0]);
        let datacenter_ids_str =
            to_string(&datacenter_ids_json).unwrap_or_else(|_| "[]".to_string());

        let new_biz_tag = BizTagActiveModel {
            id: Set(uuid::Uuid::new_v4()),
            workspace_id: Set(biz_tag.workspace_id),
            group_id: Set(biz_tag.group_id),
            name: Set(biz_tag.name.clone()),
            description: Set(biz_tag.description.clone()),
            algorithm: Set(biz_tag
                .algorithm
                .unwrap_or(crate::types::id::AlgorithmType::Segment)
                .into()),
            format: Set(biz_tag
                .format
                .clone()
                .unwrap_or(crate::types::id::IdFormat::Numeric)
                .into()),
            prefix: Set(biz_tag.prefix.clone().unwrap_or_default()),
            base_step: Set(biz_tag.base_step.unwrap_or(100)),
            max_step: Set(biz_tag.max_step.unwrap_or(1000)),
            datacenter_ids: Set(datacenter_ids_str),
            created_at: Set(chrono::Utc::now().naive_utc()),
            updated_at: Set(chrono::Utc::now().naive_utc()),
        };

        let inserted = new_biz_tag
            .insert(&self.db)
            .await
            .map_err(|e| crate::CoreError::DatabaseError(e.to_string()))?;

        Ok(inserted.into())
    }

    async fn get_biz_tag(&self, id: Uuid) -> Result<Option<BizTag>> {
        let result = BizTagEntity::find_by_id(id)
            .one(&self.db)
            .await
            .map_err(|e| crate::CoreError::DatabaseError(e.to_string()))?;

        Ok(result.map(|m| m.into()))
    }

    async fn get_biz_tag_by_workspace_group_and_name(
        &self,
        workspace_id: Uuid,
        group_id: Uuid,
        name: &str,
    ) -> Result<Option<BizTag>> {
        let result = BizTagEntity::find()
            .filter(BizTagColumn::WorkspaceId.eq(workspace_id))
            .filter(BizTagColumn::GroupId.eq(group_id))
            .filter(BizTagColumn::Name.eq(name))
            .one(&self.db)
            .await
            .map_err(|e| crate::CoreError::DatabaseError(e.to_string()))?;

        Ok(result.map(|m| m.into()))
    }

    async fn update_biz_tag(&self, id: Uuid, biz_tag: &UpdateBizTagRequest) -> Result<BizTag> {
        let existing = BizTagEntity::find_by_id(id)
            .one(&self.db)
            .await
            .map_err(|e| crate::CoreError::DatabaseError(e.to_string()))?;

        if existing.is_none() {
            return Err(crate::CoreError::NotFound(format!(
                "BizTag not found: {}",
                id
            )));
        }

        let existing = existing.unwrap();

        let datacenter_ids = if let Some(ids) = &biz_tag.datacenter_ids {
            to_string(ids).unwrap_or_else(|_| existing.datacenter_ids.clone())
        } else {
            existing.datacenter_ids.clone()
        };

        let updated = BizTagActiveModel {
            id: Set(existing.id),
            name: Set(biz_tag.name.clone().unwrap_or(existing.name)),
            description: Set(biz_tag.description.clone().or(existing.description)),
            algorithm: Set(biz_tag
                .algorithm
                .map(|a| a.into())
                .unwrap_or(existing.algorithm)),
            format: Set(biz_tag
                .format
                .clone()
                .map(|f| f.into())
                .unwrap_or(existing.format)),
            prefix: Set(biz_tag.prefix.clone().unwrap_or(existing.prefix)),
            base_step: Set(biz_tag.base_step.unwrap_or(existing.base_step)),
            max_step: Set(biz_tag.max_step.unwrap_or(existing.max_step)),
            datacenter_ids: Set(datacenter_ids),
            updated_at: Set(chrono::Utc::now().naive_utc()),
            ..Default::default()
        };

        let result = updated
            .update(&self.db)
            .await
            .map_err(|e| crate::CoreError::DatabaseError(e.to_string()))?;

        Ok(result.into())
    }

    async fn delete_biz_tag(&self, id: Uuid) -> Result<()> {
        let result = BizTagEntity::delete_by_id(id)
            .exec(&self.db)
            .await
            .map_err(|e| crate::CoreError::DatabaseError(e.to_string()))?;

        if result.rows_affected == 0 {
            return Err(crate::CoreError::NotFound(format!(
                "BizTag not found: {}",
                id
            )));
        }

        Ok(())
    }

    async fn list_biz_tags(
        &self,
        workspace_id: Uuid,
        group_id: Option<Uuid>,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> Result<Vec<BizTag>> {
        let mut query = BizTagEntity::find().filter(BizTagColumn::WorkspaceId.eq(workspace_id));

        if let Some(group_id) = group_id {
            query = query.filter(BizTagColumn::GroupId.eq(group_id));
        }

        if let Some(limit) = limit {
            query = query.limit(limit as u64);
        }

        if let Some(offset) = offset {
            query = query.offset(offset as u64);
        }

        let results = query
            .all(&self.db)
            .await
            .map_err(|e| crate::CoreError::DatabaseError(e.to_string()))?;

        Ok(results.into_iter().map(|m| m.into()).collect())
    }

    async fn list_biz_tags_by_workspace_group(
        &self,
        workspace_id: Uuid,
        group_id: Uuid,
    ) -> Result<Vec<BizTag>> {
        let results = BizTagEntity::find()
            .filter(BizTagColumn::WorkspaceId.eq(workspace_id))
            .filter(BizTagColumn::GroupId.eq(group_id))
            .all(&self.db)
            .await
            .map_err(|e| crate::CoreError::DatabaseError(e.to_string()))?;

        Ok(results.into_iter().map(|m| m.into()).collect())
    }

    async fn count_biz_tags_by_group(&self, group_id: Uuid) -> Result<u64> {
        let count = BizTagEntity::find()
            .filter(BizTagColumn::GroupId.eq(group_id))
            .count(&self.db)
            .await
            .map_err(|e| crate::CoreError::DatabaseError(e.to_string()))?;

        Ok(count)
    }

    async fn count_biz_tags(&self, workspace_id: Uuid, group_id: Option<Uuid>) -> Result<u64> {
        let mut query = BizTagEntity::find().filter(BizTagColumn::WorkspaceId.eq(workspace_id));

        if let Some(group_id) = group_id {
            query = query.filter(BizTagColumn::GroupId.eq(group_id));
        }

        let count = query
            .count(&self.db)
            .await
            .map_err(|e| crate::CoreError::DatabaseError(e.to_string()))?;

        Ok(count)
    }

    async fn health_check(&self) -> Result<()> {
        // Database connection is already established
        // Return Ok if we can reach this point
        Ok(())
    }
}

#[async_trait]
impl ApiKeyRepository for SeaOrmRepository {
    async fn create_api_key(&self, request: &CreateApiKeyRequest) -> Result<ApiKeyWithSecret> {
        // Validate key_secret length if provided (prevent DoS attacks)
        if let Some(ref secret) = request.key_secret {
            if secret.len() < 16 || secret.len() > 128 {
                return Err(crate::CoreError::InvalidInput(
                    "key_secret must be between 16 and 128 characters".to_string(),
                ));
            }
        }

        let uuid = Uuid::new_v4();
        let key_id = uuid.to_string();
        // Use provided secret or generate a new one
        let key_secret = request.key_secret.clone().unwrap_or_else(generate_secret);
        let key_secret_hash = hash_secret(&key_secret);
        let prefix = match request.role {
            ApiKeyRole::Admin => "niad_",
            ApiKeyRole::User => "nino_",
        };

        // Store full key_id with prefix for consistency
        let full_key_id = format!("{}{}", prefix, key_id);

        // Calculate expiration: use provided or default to 30 days from now
        let now = chrono::Utc::now();
        let expires_at = request.expires_at.or_else(|| {
            now.naive_utc()
                .checked_add_signed(chrono::Duration::days(30))
        });

        let new_key = ApiKeyActiveModel {
            id: Set(Uuid::new_v4()),
            key_id: Set(full_key_id.clone()),
            key_secret_hash: Set(key_secret_hash),
            key_prefix: Set(prefix.to_string()),
            role: Set(request.role.clone().into()),
            workspace_id: Set(request.workspace_id),
            name: Set(request.name.clone()),
            description: Set(request.description.clone()),
            rate_limit: Set(request.rate_limit.unwrap_or(10000)),
            enabled: Set(true),
            expires_at: Set(expires_at),
            last_used_at: Set(None),
            created_at: Set(now.naive_utc()),
            updated_at: Set(now.naive_utc()),
        };

        let inserted = new_key
            .insert(&self.db)
            .await
            .map_err(|e| crate::CoreError::DatabaseError(e.to_string()))?;

        let response = ApiKeyWithSecret {
            key: ApiKeyResponse {
                id: inserted.id,
                key_id: inserted.key_id,
                key_prefix: inserted.key_prefix,
                name: inserted.name,
                description: inserted.description,
                role: inserted.role.into(),
                rate_limit: inserted.rate_limit,
                enabled: inserted.enabled,
                expires_at: inserted.expires_at,
                created_at: inserted.created_at,
            },
            key_secret,
        };

        Ok(response)
    }

    async fn get_api_key_by_id(&self, key_id: &str) -> Result<Option<ApiKeyInfo>> {
        let result = ApiKeyEntity::find()
            .filter(ApiKeyColumn::KeyId.eq(key_id))
            .one(&self.db)
            .await
            .map_err(|e| crate::CoreError::DatabaseError(e.to_string()))?;

        Ok(result.map(|m| m.into()))
    }

    async fn validate_api_key(
        &self,
        key_id: &str,
        key_secret: &str,
    ) -> Result<Option<(Option<Uuid>, ApiKeyRole)>> {
        let key_model = ApiKeyEntity::find()
            .filter(ApiKeyColumn::KeyId.eq(key_id))
            .one(&self.db)
            .await
            .map_err(|e| crate::CoreError::DatabaseError(e.to_string()))?;

        if let Some(model) = key_model {
            if !model.enabled {
                return Ok(None);
            }

            if let Some(expires_at) = model.expires_at {
                if expires_at < chrono::Utc::now().naive_utc() {
                    return Ok(None);
                }
            }

            let expected_hash = hash_secret(key_secret);
            if expected_hash
                .as_bytes()
                .ct_eq(model.key_secret_hash.as_bytes())
                .into()
            {
                let _ = self.update_last_used(model.id).await;
                return Ok(Some((model.workspace_id, model.role.into())));
            }
        }

        Ok(None)
    }

    async fn list_api_keys(
        &self,
        workspace_id: Uuid,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> Result<Vec<ApiKeyInfo>> {
        let mut query = ApiKeyEntity::find().filter(ApiKeyColumn::WorkspaceId.eq(workspace_id));

        if let Some(limit) = limit {
            query = query.limit(limit as u64);
        }

        if let Some(offset) = offset {
            query = query.offset(offset as u64);
        }

        let results = query
            .all(&self.db)
            .await
            .map_err(|e| crate::CoreError::DatabaseError(e.to_string()))?;

        Ok(results.into_iter().map(|m| m.into()).collect())
    }

    async fn delete_api_key(&self, id: Uuid) -> Result<()> {
        let result = ApiKeyEntity::delete_by_id(id)
            .exec(&self.db)
            .await
            .map_err(|e| crate::CoreError::DatabaseError(e.to_string()))?;

        if result.rows_affected == 0 {
            return Err(crate::CoreError::NotFound(format!(
                "API key not found: {}",
                id
            )));
        }

        Ok(())
    }

    async fn revoke_api_key(&self, id: Uuid) -> Result<()> {
        let existing = ApiKeyEntity::find_by_id(id)
            .one(&self.db)
            .await
            .map_err(|e| crate::CoreError::DatabaseError(e.to_string()))?;

        if existing.is_none() {
            return Err(crate::CoreError::NotFound(format!(
                "API key not found: {}",
                id
            )));
        }

        let updated = ApiKeyActiveModel {
            id: Set(existing.unwrap().id),
            enabled: Set(false),
            updated_at: Set(chrono::Utc::now().naive_utc()),
            ..Default::default()
        };

        updated
            .update(&self.db)
            .await
            .map_err(|e| crate::CoreError::DatabaseError(e.to_string()))?;

        Ok(())
    }

    async fn update_last_used(&self, id: Uuid) -> Result<()> {
        let existing = ApiKeyEntity::find_by_id(id)
            .one(&self.db)
            .await
            .map_err(|e| crate::CoreError::DatabaseError(e.to_string()))?;

        if let Some(model) = existing {
            let updated = ApiKeyActiveModel {
                id: Set(model.id),
                last_used_at: Set(Some(chrono::Utc::now().naive_utc())),
                updated_at: Set(chrono::Utc::now().naive_utc()),
                ..Default::default()
            };

            updated
                .update(&self.db)
                .await
                .map_err(|e| crate::CoreError::DatabaseError(e.to_string()))?;
        }

        Ok(())
    }

    async fn get_admin_api_key(&self, _workspace_id: Uuid) -> Result<Option<ApiKeyInfo>> {
        // Admin keys are global (workspace_id is NULL), so we don't filter by workspace_id
        // The workspace_id parameter is kept for backward compatibility but not used
        let result = ApiKeyEntity::find()
            .filter(ApiKeyColumn::Role.eq(super::api_key_entity::ApiKeyRole::Admin.to_string()))
            .filter(ApiKeyColumn::KeyPrefix.eq("niad_"))
            .one(&self.db)
            .await
            .map_err(|e| crate::CoreError::DatabaseError(e.to_string()))?;

        Ok(result.filter(|m| m.enabled).map(|m| m.into()))
    }

    async fn count_api_keys(&self, workspace_id: Uuid) -> Result<u64> {
        let count = ApiKeyEntity::find()
            .filter(ApiKeyColumn::WorkspaceId.eq(workspace_id))
            .count(&self.db)
            .await
            .map_err(|e| crate::CoreError::DatabaseError(e.to_string()))?;

        Ok(count)
    }

    async fn rotate_api_key(
        &self,
        key_id: &str,
        _grace_period_seconds: u64,
    ) -> Result<ApiKeyWithSecret> {
        // 获取现有密钥
        let key_data = self
            .get_api_key_by_id(key_id)
            .await?
            .ok_or_else(|| crate::CoreError::NotFound(format!("API key not found: {}", key_id)))?;

        // 生成新密钥
        let new_secret = generate_secret();
        let new_secret_hash = hash_secret(&new_secret);
        let now = chrono::Utc::now().naive_utc();

        // 更新数据库
        let updated_key = ApiKeyActiveModel {
            id: Set(key_data.id),
            key_secret_hash: Set(new_secret_hash),
            updated_at: Set(now),
            ..Default::default()
        };

        let updated = updated_key
            .update(&self.db)
            .await
            .map_err(|e| crate::CoreError::DatabaseError(e.to_string()))?;

        // 返回新密钥
        Ok(ApiKeyWithSecret {
            key: ApiKeyResponse {
                id: updated.id,
                key_id: updated.key_id,
                key_prefix: updated.key_prefix,
                name: updated.name,
                description: updated.description,
                role: updated.role.into(),
                rate_limit: updated.rate_limit,
                enabled: updated.enabled,
                expires_at: key_data.expires_at,
                created_at: updated.created_at,
            },
            key_secret: new_secret,
        })
    }

    async fn get_keys_older_than(&self, age_threshold_days: i64) -> Result<Vec<ApiKeyInfo>> {
        let threshold = chrono::Utc::now().naive_utc() - chrono::Duration::days(age_threshold_days);

        let keys = ApiKeyEntity::find()
            .filter(ApiKeyColumn::CreatedAt.lt(threshold))
            .filter(ApiKeyColumn::Enabled.eq(true))
            .all(&self.db)
            .await
            .map_err(|e| crate::CoreError::DatabaseError(e.to_string()))?;

        Ok(keys.into_iter().map(|m| m.into()).collect())
    }
}

#[async_trait]
impl SegmentRepository for SeaOrmRepository {
    async fn get_segment(&self, workspace_id: &str, biz_tag: &str) -> Result<Option<SegmentInfo>> {
        let result = SegmentEntity::find()
            .filter(SegmentColumn::WorkspaceId.eq(workspace_id))
            .filter(SegmentColumn::BizTag.eq(biz_tag))
            .one(&self.db)
            .await
            .map_err(|e| crate::CoreError::DatabaseError(e.to_string()))?;

        Ok(result.map(|m| SegmentInfo {
            id: m.id,
            workspace_id: m.workspace_id,
            biz_tag: m.biz_tag,
            current_id: m.current_id,
            max_id: m.max_id,
            step: m.step as u32,
            delta: m.delta as u32,
            created_at: naive_to_utc(Some(m.created_at)),
            updated_at: naive_to_utc(Some(m.updated_at)),
        }))
    }

    async fn allocate_segment(
        &self,
        workspace_id: &str,
        biz_tag: &str,
        step: i32,
    ) -> Result<SegmentInfo> {
        // 获取分布式锁以防止并发分配冲突
        let lock_key = self.segment_lock_key(workspace_id, biz_tag, None);
        let lock_guard = if let Some(ref lock) = self.distributed_lock {
            lock.acquire(&lock_key, 30).await.map_err(|e| {
                crate::CoreError::InternalError(format!(
                    "Failed to acquire distributed lock for segment allocation: {}",
                    e
                ))
            })?
        } else {
            // 如果没有配置分布式锁，使用空守卫
            Box::new(NoopLockGuard)
        };

        let txn = self
            .db
            .begin()
            .await
            .map_err(|e| crate::CoreError::DatabaseError(e.to_string()))?;

        let existing = SegmentEntity::find()
            .filter(SegmentColumn::WorkspaceId.eq(workspace_id))
            .filter(SegmentColumn::BizTag.eq(biz_tag))
            .one(&txn)
            .await
            .map_err(|e| crate::CoreError::DatabaseError(e.to_string()))?;

        let segment = match existing {
            Some(model) => {
                let current_id = model.current_id;
                let max_id = model.max_id;
                let new_max_id = current_id + step as i64;

                let updated = SegmentActiveModel {
                    id: Set(model.id),
                    current_id: Set(new_max_id),
                    updated_at: Set(chrono::Utc::now().naive_utc()),
                    ..Default::default()
                };

                updated
                    .update(&txn)
                    .await
                    .map_err(|e| crate::CoreError::DatabaseError(e.to_string()))?;

                debug!(
                    "Updated segment for {}/{}: current_id={}, max_id={}",
                    workspace_id, biz_tag, new_max_id, max_id
                );

                SegmentInfo {
                    id: model.id,
                    workspace_id: model.workspace_id,
                    biz_tag: model.biz_tag,
                    current_id,
                    max_id: new_max_id,
                    step: model.step as u32,
                    delta: model.delta as u32,
                    created_at: naive_to_utc(Some(model.created_at)),
                    updated_at: Utc::now(),
                }
            }
            None => {
                let start_id = 1i64;
                let max_id = start_id + step as i64;
                let delta = 1;

                let new_segment = SegmentActiveModel {
                    workspace_id: Set(workspace_id.to_string()),
                    biz_tag: Set(biz_tag.to_string()),
                    current_id: Set(max_id),
                    max_id: Set(max_id),
                    step: Set(step),
                    delta: Set(delta),
                    created_at: Set(chrono::Utc::now().naive_utc()),
                    updated_at: Set(chrono::Utc::now().naive_utc()),
                    ..Default::default()
                };

                let inserted = new_segment
                    .insert(&txn)
                    .await
                    .map_err(|e| crate::CoreError::DatabaseError(e.to_string()))?;

                info!(
                    "Created new segment for {}/{}: start_id={}, max_id={}",
                    workspace_id, biz_tag, start_id, max_id
                );

                SegmentInfo {
                    id: inserted.id,
                    workspace_id: inserted.workspace_id,
                    biz_tag: inserted.biz_tag,
                    current_id: start_id,
                    max_id,
                    step: step as u32,
                    delta: delta as u32,
                    created_at: naive_to_utc(Some(inserted.created_at)),
                    updated_at: naive_to_utc(Some(inserted.updated_at)),
                }
            }
        };

        txn.commit()
            .await
            .map_err(|e| crate::CoreError::DatabaseError(e.to_string()))?;

        // 释放分布式锁
        let _ = lock_guard.release().await;

        Ok(segment)
    }

    async fn allocate_segment_with_dc(
        &self,
        workspace_id: &str,
        biz_tag: &str,
        step: i32,
        dc_id: i32,
    ) -> Result<SegmentInfo> {
        // 获取分布式锁以防止并发分配冲突
        let lock_key = self.segment_lock_key(workspace_id, biz_tag, Some(dc_id));
        let lock_guard = if let Some(ref lock) = self.distributed_lock {
            lock.acquire(&lock_key, 30).await.map_err(|e| {
                crate::CoreError::InternalError(format!(
                    "Failed to acquire distributed lock for segment allocation: {}",
                    e
                ))
            })?
        } else {
            // 如果没有配置分布式锁，使用空守卫
            Box::new(NoopLockGuard)
        };

        let txn = self
            .db
            .begin()
            .await
            .map_err(|e| crate::CoreError::DatabaseError(e.to_string()))?;

        let existing = SegmentEntity::find()
            .filter(SegmentColumn::WorkspaceId.eq(workspace_id))
            .filter(SegmentColumn::BizTag.eq(biz_tag))
            .filter(SegmentColumn::DcId.eq(dc_id))
            .one(&txn)
            .await
            .map_err(|e| crate::CoreError::DatabaseError(e.to_string()))?;

        let segment = match existing {
            Some(model) => {
                let current_id = model.current_id;
                let max_id = model.max_id;
                let new_max_id = current_id + step as i64;

                let updated = SegmentActiveModel {
                    id: Set(model.id),
                    current_id: Set(new_max_id),
                    updated_at: Set(chrono::Utc::now().naive_utc()),
                    ..Default::default()
                };

                updated
                    .update(&txn)
                    .await
                    .map_err(|e| crate::CoreError::DatabaseError(e.to_string()))?;

                debug!(
                    "Updated segment for {}/{}/dc{}: current_id={}, max_id={}",
                    workspace_id, biz_tag, dc_id, new_max_id, max_id
                );

                SegmentInfo {
                    id: model.id,
                    workspace_id: model.workspace_id,
                    biz_tag: model.biz_tag,
                    current_id,
                    max_id: new_max_id,
                    step: model.step as u32,
                    delta: model.delta as u32,
                    created_at: naive_to_utc(Some(model.created_at)),
                    updated_at: Utc::now(),
                }
            }
            None => {
                let start_id = (dc_id as i64) * 1000000000000i64 + 1i64;
                let max_id = start_id + step as i64;
                let delta = 1;

                let new_segment = SegmentActiveModel {
                    workspace_id: Set(workspace_id.to_string()),
                    biz_tag: Set(biz_tag.to_string()),
                    current_id: Set(max_id),
                    max_id: Set(max_id),
                    step: Set(step),
                    delta: Set(delta),
                    dc_id: Set(dc_id),
                    created_at: Set(chrono::Utc::now().naive_utc()),
                    updated_at: Set(chrono::Utc::now().naive_utc()),
                    ..Default::default()
                };

                let inserted = new_segment
                    .insert(&txn)
                    .await
                    .map_err(|e| crate::CoreError::DatabaseError(e.to_string()))?;

                info!(
                    "Created new segment for {}/{}/dc{}: start_id={}, max_id={}",
                    workspace_id, biz_tag, dc_id, start_id, max_id
                );

                SegmentInfo {
                    id: inserted.id,
                    workspace_id: inserted.workspace_id,
                    biz_tag: inserted.biz_tag,
                    current_id: start_id,
                    max_id,
                    step: step as u32,
                    delta: delta as u32,
                    created_at: naive_to_utc(Some(inserted.created_at)),
                    updated_at: naive_to_utc(Some(inserted.updated_at)),
                }
            }
        };

        txn.commit()
            .await
            .map_err(|e| crate::CoreError::DatabaseError(e.to_string()))?;

        // 释放分布式锁
        let _ = lock_guard.release().await;

        Ok(segment)
    }

    async fn update_segment(
        &self,
        workspace_id: &str,
        biz_tag: &str,
        current_id: i64,
        _max_id: i64,
    ) -> Result<()> {
        let result = SegmentEntity::update_many()
            .filter(SegmentColumn::WorkspaceId.eq(workspace_id))
            .filter(SegmentColumn::BizTag.eq(biz_tag))
            .set(SegmentActiveModel {
                current_id: Set(current_id),
                updated_at: Set(chrono::Utc::now().naive_utc()),
                ..Default::default()
            })
            .exec(&self.db)
            .await
            .map_err(|e| crate::CoreError::DatabaseError(e.to_string()))?;

        if result.rows_affected == 0 {
            return Err(crate::CoreError::NotFound(format!(
                "Segment not found for {}/{}",
                workspace_id, biz_tag
            )));
        }

        Ok(())
    }

    async fn create_segment(
        &self,
        workspace_id: &str,
        biz_tag: &str,
        start_id: i64,
        max_id: i64,
        step: i32,
        delta: i32,
    ) -> Result<SegmentInfo> {
        let new_segment = SegmentActiveModel {
            workspace_id: Set(workspace_id.to_string()),
            biz_tag: Set(biz_tag.to_string()),
            current_id: Set(start_id),
            max_id: Set(max_id),
            step: Set(step),
            delta: Set(delta),
            created_at: Set(chrono::Utc::now().naive_utc()),
            updated_at: Set(chrono::Utc::now().naive_utc()),
            ..Default::default()
        };

        let inserted = new_segment
            .insert(&self.db)
            .await
            .map_err(|e| crate::CoreError::DatabaseError(e.to_string()))?;

        Ok(SegmentInfo {
            id: inserted.id,
            workspace_id: inserted.workspace_id,
            biz_tag: inserted.biz_tag,
            current_id: inserted.current_id,
            max_id: inserted.max_id,
            step: inserted.step as u32,
            delta: inserted.delta as u32,
            created_at: naive_to_utc(Some(inserted.created_at)),
            updated_at: naive_to_utc(Some(inserted.updated_at)),
        })
    }

    async fn list_segments(&self, workspace_id: &str) -> Result<Vec<SegmentInfo>> {
        let results = SegmentEntity::find()
            .filter(SegmentColumn::WorkspaceId.eq(workspace_id))
            .all(&self.db)
            .await
            .map_err(|e| crate::CoreError::DatabaseError(e.to_string()))?;

        Ok(results
            .into_iter()
            .map(|m| SegmentInfo {
                id: m.id,
                workspace_id: m.workspace_id,
                biz_tag: m.biz_tag,
                current_id: m.current_id,
                max_id: m.max_id,
                step: m.step as u32,
                delta: m.delta as u32,
                created_at: naive_to_utc(Some(m.created_at)),
                updated_at: naive_to_utc(Some(m.updated_at)),
            })
            .collect())
    }

    async fn delete_segment(&self, workspace_id: &str, biz_tag: &str) -> Result<()> {
        let result = SegmentEntity::delete_many()
            .filter(SegmentColumn::WorkspaceId.eq(workspace_id))
            .filter(SegmentColumn::BizTag.eq(biz_tag))
            .exec(&self.db)
            .await
            .map_err(|e| crate::CoreError::DatabaseError(e.to_string()))?;

        if result.rows_affected == 0 {
            return Err(crate::CoreError::NotFound(format!(
                "Segment not found for {}/{}",
                workspace_id, biz_tag
            )));
        }

        Ok(())
    }
}

fn naive_to_utc(naive: Option<NaiveDateTime>) -> DateTime<Utc> {
    naive
        .map(|n| Utc.from_utc_datetime(&n))
        .unwrap_or_else(Utc::now)
}

/// Generate a cryptographically secure random secret
fn generate_secret() -> String {
    const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_-";
    const SECRET_LENGTH: usize = 32;

    let mut rng = rand::thread_rng();
    let secret: String = (0..SECRET_LENGTH)
        .map(|_| {
            let idx = rand::Rng::gen_range(&mut rng, 0..CHARSET.len());
            CHARSET[idx] as char
        })
        .collect();

    secret
}

/// Hash a secret using SHA-256
fn hash_secret(secret: &str) -> String {
    let mut hasher = sha2::Sha256::default();
    hasher.update(secret);
    hex::encode(hasher.finalize())
}

/// No-op lock guard for when distributed lock is not configured
/// Used as a fallback to maintain consistent API without requiring distributed locking
struct NoopLockGuard;

#[async_trait]
impl LockGuard for NoopLockGuard {
    async fn release(&self) -> std::result::Result<(), LockError> {
        Ok(())
    }
}

/// Test prefix logic without database (pure logic tests)
#[cfg(test)]
mod prefix_tests {
    use super::*;

    #[test]
    fn test_admin_role_prefix() {
        let role = ApiKeyRole::Admin;
        let prefix = match role {
            ApiKeyRole::Admin => "niad_",
            ApiKeyRole::User => "nino_",
        };
        assert_eq!(prefix, "niad_", "Admin role should use 'niad_' prefix");
    }

    #[test]
    fn test_user_role_prefix() {
        let role = ApiKeyRole::User;
        let prefix = match role {
            ApiKeyRole::Admin => "niad_",
            ApiKeyRole::User => "nino_",
        };
        assert_eq!(prefix, "nino_", "User role should use 'nino_' prefix");
    }

    #[test]
    fn test_prefix_uuid_format() {
        let uuid = Uuid::new_v4();
        let prefix = "niad_";
        let full_key_id = format!("{}{}", prefix, uuid);

        assert!(full_key_id.starts_with(prefix));
        assert_eq!(full_key_id.len(), prefix.len() + 36); // 36 is standard UUID length
    }

    #[test]
    fn test_secret_length_validation() {
        let short_secret = "too_short";
        assert!(short_secret.len() < 16, "Test secret should be too short");

        let long_secret = "a".repeat(129);
        assert!(long_secret.len() > 128, "Test secret should be too long");

        let valid_secret = "this_is_a_valid_secret_length_16";
        assert!(valid_secret.len() >= 16 && valid_secret.len() <= 128);
    }

    #[test]
    fn test_generate_secret_length() {
        let secret = generate_secret();
        assert_eq!(secret.len(), 32, "Generated secret should be 32 characters");
    }

    #[test]
    fn test_hash_secret_consistency() {
        let secret = "test_secret";
        let hash1 = hash_secret(secret);
        let hash2 = hash_secret(secret);

        assert_eq!(
            hash1, hash2,
            "Hashing same secret should produce same result"
        );
        assert_eq!(hash1.len(), 64, "SHA-256 hash should be 64 hex characters");
    }

    #[test]
    fn test_hash_secret_uniqueness() {
        let secret1 = "secret_one";
        let secret2 = "secret_two";
        let hash1 = hash_secret(secret1);
        let hash2 = hash_secret(secret2);

        assert_ne!(
            hash1, hash2,
            "Different secrets should produce different hashes"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{ConnectOptions, ConnectionTrait, Database, Statement};

    async fn setup_test_db(db: &sea_orm::DatabaseConnection) {
        let backend = db.get_database_backend();

        // Set search_path to include nebula_id schema
        db.execute(Statement::from_string(
            backend,
            r#"SET search_path TO public, nebula_id"#,
        ))
        .await
        .unwrap();

        // Create schema if it doesn't exist
        db.execute(Statement::from_string(
            backend,
            r#"CREATE SCHEMA IF NOT EXISTS nebula_id"#,
        ))
        .await
        .unwrap();

        // Create enums in public schema for SeaORM compatibility
        db.execute(Statement::from_string(
            backend,
            r#"DO $$ BEGIN
                CREATE TYPE public.algorithm_type AS ENUM ('segment', 'snowflake', 'uuid_v7', 'uuid_v4');
            EXCEPTION
                WHEN duplicate_object THEN null;
            END $$"#,
        ))
        .await
        .unwrap();

        db.execute(Statement::from_string(
            backend,
            r#"DO $$ BEGIN
                CREATE TYPE public.id_format AS ENUM ('numeric', 'prefixed', 'uuid');
            EXCEPTION
                WHEN duplicate_object THEN null;
            END $$"#,
        ))
        .await
        .unwrap();

        db.execute(Statement::from_string(
            backend,
            r#"DO $$ BEGIN
                CREATE TYPE public.workspace_status AS ENUM ('active', 'inactive', 'suspended');
            EXCEPTION
                WHEN duplicate_object THEN null;
            END $$"#,
        ))
        .await
        .unwrap();

        db.execute(Statement::from_string(
            backend,
            r#"
            CREATE TABLE IF NOT EXISTS "nebula_id"."workspaces" (
                id UUID PRIMARY KEY,
                name VARCHAR(255) NOT NULL UNIQUE,
                description TEXT,
                status "nebula_id"."workspace_status" NOT NULL DEFAULT 'active',
                max_groups INTEGER NOT NULL DEFAULT 100,
                max_biz_tags INTEGER NOT NULL DEFAULT 1000,
                created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
                updated_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        ))
        .await
        .unwrap();

        db.execute(Statement::from_string(
            backend,
            r#"
            CREATE TABLE IF NOT EXISTS "nebula_id"."groups" (
                id UUID PRIMARY KEY,
                workspace_id UUID NOT NULL REFERENCES "nebula_id"."workspaces"(id) ON DELETE CASCADE,
                name VARCHAR(255) NOT NULL,
                description TEXT,
                max_biz_tags INTEGER NOT NULL DEFAULT 100,
                created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
                updated_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
                UNIQUE(workspace_id, name)
            )
            "#,
        ))
        .await
        .unwrap();

        db.execute(Statement::from_string(
            backend,
            r#"
            CREATE TABLE IF NOT EXISTS "nebula_id"."biz_tags" (
                id UUID PRIMARY KEY,
                workspace_id UUID NOT NULL REFERENCES "nebula_id"."workspaces"(id) ON DELETE CASCADE,
                group_id UUID NOT NULL REFERENCES "nebula_id"."groups"(id) ON DELETE CASCADE,
                name VARCHAR(255) NOT NULL,
                description TEXT,
                algorithm "nebula_id"."algorithm_type" NOT NULL DEFAULT 'segment',
                format "nebula_id"."id_format" NOT NULL DEFAULT 'numeric',
                prefix VARCHAR(50) DEFAULT '',
                base_step INTEGER NOT NULL DEFAULT 1000,
                max_step INTEGER NOT NULL DEFAULT 100000,
                datacenter_ids INTEGER[] DEFAULT ARRAY[0],
                created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
                updated_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
                UNIQUE(workspace_id, group_id, name)
            )
            "#,
        ))
        .await
        .unwrap();

        db.execute(Statement::from_string(
            backend,
            r#"
            CREATE TABLE IF NOT EXISTS "nebula_id"."segments" (
                id BIGINT PRIMARY KEY,
                workspace_id VARCHAR(255) NOT NULL,
                biz_tag VARCHAR(255) NOT NULL,
                current_id BIGINT NOT NULL DEFAULT 1,
                max_id BIGINT NOT NULL,
                step INTEGER NOT NULL DEFAULT 100,
                delta INTEGER NOT NULL DEFAULT 1,
                dc_id INTEGER NOT NULL DEFAULT 0,
                created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
                updated_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        ))
        .await
        .unwrap();

        db.execute(Statement::from_string(
            backend,
            r#"
            CREATE TABLE IF NOT EXISTS "nebula_id"."api_keys" (
                id UUID PRIMARY KEY,
                key_id VARCHAR(36) NOT NULL UNIQUE,
                key_secret_hash VARCHAR(64) NOT NULL,
                key_prefix VARCHAR(8) NOT NULL,
                role VARCHAR(20) NOT NULL DEFAULT 'user',
                workspace_id UUID REFERENCES "nebula_id"."workspaces"(id) ON DELETE CASCADE,
                name VARCHAR(255) NOT NULL,
                description TEXT,
                rate_limit INTEGER NOT NULL DEFAULT 10000,
                enabled BOOLEAN NOT NULL DEFAULT true,
                expires_at TIMESTAMP WITH TIME ZONE DEFAULT (CURRENT_TIMESTAMP + INTERVAL '30 days'),
                last_used_at TIMESTAMP WITH TIME ZONE,
                created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
                updated_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        ))
        .await
        .unwrap();
    }

    #[tokio::test]
    #[ignore]
    async fn test_repository_operations() {
        let db_url =
            std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite::memory:".to_string());
        let db = Database::connect(&db_url).await.unwrap();
        setup_test_db(&db).await;

        let repo = SeaOrmRepository::new(db);

        // Use unique names to avoid conflicts
        let unique_id = uuid::Uuid::new_v4().to_string()[..8].to_string();
        let workspace_name = format!("test_workspace_{}", unique_id);
        let biz_tag = format!("test_tag_{}", unique_id);

        let segment = repo
            .allocate_segment(&workspace_name, &biz_tag, 100)
            .await
            .unwrap();

        assert_eq!(segment.workspace_id, workspace_name);
        assert_eq!(segment.biz_tag, biz_tag);
        assert_eq!(segment.current_id, 1);
        assert_eq!(segment.max_id, 101);
        assert_eq!(segment.step, 100);

        let fetched = repo
            .get_segment(&workspace_name, &biz_tag)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(fetched.id, segment.id);

        let segment2 = repo
            .allocate_segment(&workspace_name, &biz_tag, 100)
            .await
            .unwrap();

        assert_eq!(segment2.current_id, 101);
        assert_eq!(segment2.max_id, 201);

        let list = repo.list_segments(&workspace_name).await.unwrap();

        assert_eq!(list.len(), 1);
    }

    #[tokio::test]
    #[ignore]
    async fn test_cascading_operations() {
        let db_url =
            std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite::memory:".to_string());
        let db = Database::connect(&db_url).await.unwrap();
        setup_test_db(&db).await;

        let repo = SeaOrmRepository::new(db);

        // Use unique names to avoid conflicts
        let unique_id = uuid::Uuid::new_v4().to_string()[..8].to_string();
        let workspace_name = format!("test_workspace_{}", unique_id);

        let workspace = repo
            .create_workspace(&CreateWorkspaceRequest {
                name: workspace_name.clone(),
                description: Some("Test workspace".to_string()),
                max_groups: Some(5),
                max_biz_tags: Some(50),
            })
            .await
            .unwrap();

        assert_eq!(workspace.name, workspace_name);

        let group1 = repo
            .create_group(&CreateGroupRequest {
                workspace_id: workspace.id,
                name: "group1".to_string(),
                description: Some("Test group 1".to_string()),
                max_biz_tags: Some(20),
            })
            .await
            .unwrap();

        let group2 = repo
            .create_group(&CreateGroupRequest {
                workspace_id: workspace.id,
                name: "group2".to_string(),
                description: Some("Test group 2".to_string()),
                max_biz_tags: Some(30),
            })
            .await
            .unwrap();

        let _biz_tag1 = repo
            .create_biz_tag(&CreateBizTagRequest {
                workspace_id: workspace.id,
                group_id: group1.id,
                name: "biz_tag_1".to_string(),
                description: Some("Test biz tag 1".to_string()),
                algorithm: Some(crate::types::id::AlgorithmType::Segment),
                format: Some(crate::types::id::IdFormat::Numeric),
                prefix: None,
                base_step: Some(100),
                max_step: Some(1000),
                datacenter_ids: None,
            })
            .await
            .unwrap();

        let _biz_tag2 = repo
            .create_biz_tag(&CreateBizTagRequest {
                workspace_id: workspace.id,
                group_id: group1.id,
                name: "biz_tag_2".to_string(),
                description: Some("Test biz tag 2".to_string()),
                algorithm: Some(crate::types::id::AlgorithmType::Snowflake),
                format: Some(crate::types::IdFormat::Numeric),
                prefix: Some("prefix_".to_string()),
                base_step: Some(200),
                max_step: Some(2000),
                datacenter_ids: Some(vec![0, 1]),
            })
            .await
            .unwrap();

        let biz_tag3 = repo
            .create_biz_tag(&CreateBizTagRequest {
                workspace_id: workspace.id,
                group_id: group2.id,
                name: "biz_tag_3".to_string(),
                description: Some("Test biz tag 3".to_string()),
                algorithm: None,
                format: None,
                prefix: None,
                base_step: None,
                max_step: None,
                datacenter_ids: None,
            })
            .await
            .unwrap();

        let workspace_with_groups = repo
            .get_workspace_with_groups(workspace.id)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(workspace_with_groups.0.id, workspace.id);
        assert_eq!(workspace_with_groups.1.len(), 2);

        let workspace_with_all = repo
            .get_workspace_with_groups_and_biz_tags(workspace.id)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(workspace_with_all.0.id, workspace.id);
        assert_eq!(workspace_with_all.1.len(), 2);

        let total_biz_tags: usize = workspace_with_all
            .1
            .iter()
            .map(|(_, tags)| tags.len())
            .sum();
        assert_eq!(total_biz_tags, 3);

        let group_with_biz_tags = repo
            .get_group_with_biz_tags(group1.id)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(group_with_biz_tags.0.id, group1.id);
        assert_eq!(group_with_biz_tags.1.len(), 2);

        let biz_tags_in_group1 = repo
            .list_biz_tags_by_workspace_group(workspace.id, group1.id)
            .await
            .unwrap();

        assert_eq!(biz_tags_in_group1.len(), 2);

        let count = repo.count_biz_tags_by_group(group1.id).await.unwrap();
        assert_eq!(count, 2);

        let count2 = repo.count_biz_tags_by_group(group2.id).await.unwrap();
        assert_eq!(count2, 1);

        repo.delete_group_with_biz_tags(group1.id).await.unwrap();

        let remaining_biz_tags = repo
            .list_biz_tags_by_workspace_group(workspace.id, group1.id)
            .await
            .unwrap();

        assert_eq!(remaining_biz_tags.len(), 0);

        let remaining_group = repo.get_group(group1.id).await.unwrap();
        assert!(remaining_group.is_none());

        let biz_tags_in_group2 = repo
            .list_biz_tags_by_workspace_group(workspace.id, group2.id)
            .await
            .unwrap();

        assert_eq!(biz_tags_in_group2.len(), 1);
        assert_eq!(biz_tags_in_group2[0].id, biz_tag3.id);

        repo.delete_biz_tag(biz_tag3.id).await.unwrap();

        let biz_tags_in_group2_after_delete = repo
            .list_biz_tags_by_workspace_group(workspace.id, group2.id)
            .await
            .unwrap();

        assert_eq!(biz_tags_in_group2_after_delete.len(), 0);
    }

    /// Test Admin API key prefix (niad_) is correctly applied
    #[tokio::test]
    #[ignore]
    async fn test_admin_api_key_prefix() {
        let db_url =
            std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite::memory:".to_string());
        let db = Database::connect(&db_url).await.unwrap();
        setup_test_db(&db).await;

        let repo = SeaOrmRepository::new(db);

        let admin_key = repo
            .create_api_key(&CreateApiKeyRequest {
                workspace_id: None,
                name: "Test Admin Key".to_string(),
                description: Some("Admin key for testing".to_string()),
                role: ApiKeyRole::Admin,
                rate_limit: Some(10000),
                expires_at: None,
                key_secret: None,
            })
            .await
            .unwrap();

        // Verify key_id has correct prefix
        assert!(
            admin_key.key.key_id.starts_with("niad_"),
            "Admin key_id should start with 'niad_', got: {}",
            admin_key.key.key_id
        );

        // Verify key_prefix field matches
        assert_eq!(
            admin_key.key.key_prefix, "niad_",
            "Admin key_prefix should be 'niad_'"
        );

        // Verify key_id contains prefix exactly once at start
        assert!(
            admin_key.key.key_id.len() > 5,
            "key_id should be longer than prefix"
        );

        // Verify consistency: key_id should be prefix + uuid
        let key_id_without_prefix = &admin_key.key.key_id[5..];
        let uuid_validation = uuid::Uuid::parse_str(key_id_without_prefix);
        assert!(
            uuid_validation.is_ok(),
            "key_id after prefix should be a valid UUID, got: {}",
            key_id_without_prefix
        );
    }

    /// Test User API key prefix (nino_) is correctly applied
    #[tokio::test]
    #[ignore]
    async fn test_user_api_key_prefix() {
        let db_url =
            std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite::memory:".to_string());
        let db = Database::connect(&db_url).await.unwrap();
        setup_test_db(&db).await;

        let repo = SeaOrmRepository::new(db);

        // Use unique names to avoid conflicts
        let unique_id = uuid::Uuid::new_v4().to_string()[..8].to_string();
        let workspace_name = format!("Test_Workspace_{}", unique_id);

        // First create a workspace for the user key
        let workspace = repo
            .create_workspace(&CreateWorkspaceRequest {
                name: workspace_name,
                description: Some("Workspace for user key testing".to_string()),
                max_groups: Some(5),
                max_biz_tags: Some(50),
            })
            .await
            .unwrap();

        let user_key = repo
            .create_api_key(&CreateApiKeyRequest {
                workspace_id: Some(workspace.id),
                name: "Test User Key".to_string(),
                description: Some("User key for testing".to_string()),
                role: ApiKeyRole::User,
                rate_limit: Some(5000),
                expires_at: None,
                key_secret: None,
            })
            .await
            .unwrap();

        // Verify key_id has correct prefix
        assert!(
            user_key.key.key_id.starts_with("nino_"),
            "User key_id should start with 'nino_', got: {}",
            user_key.key.key_id
        );

        // Verify key_prefix field matches
        assert_eq!(
            user_key.key.key_prefix, "nino_",
            "User key_prefix should be 'nino_'"
        );

        // Verify key_id contains prefix exactly once at start
        assert!(
            user_key.key.key_id.len() > 5,
            "key_id should be longer than prefix"
        );

        // Verify consistency: key_id should be prefix + uuid
        let key_id_without_prefix = &user_key.key.key_id[5..];
        let uuid_validation = uuid::Uuid::parse_str(key_id_without_prefix);
        assert!(
            uuid_validation.is_ok(),
            "key_id after prefix should be a valid UUID, got: {}",
            key_id_without_prefix
        );
    }

    /// Test that API key with provided secret is handled correctly
    #[tokio::test]
    #[ignore]
    async fn test_api_key_with_custom_secret() {
        let db_url =
            std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite::memory:".to_string());
        let db = Database::connect(&db_url).await.unwrap();
        setup_test_db(&db).await;

        let repo = SeaOrmRepository::new(db);

        let custom_secret = "my_custom_secret_for_testing_12345".to_string();

        let admin_key = repo
            .create_api_key(&CreateApiKeyRequest {
                workspace_id: None,
                name: "Admin Key with Custom Secret".to_string(),
                description: Some("Testing custom secret".to_string()),
                role: ApiKeyRole::Admin,
                rate_limit: Some(8000),
                expires_at: None,
                key_secret: Some(custom_secret.clone()),
            })
            .await
            .unwrap();

        // Verify prefix is still correct with custom secret
        assert!(
            admin_key.key.key_id.starts_with("niad_"),
            "Prefix should be applied even with custom secret"
        );
        assert_eq!(
            admin_key.key_secret, custom_secret,
            "Provided secret should be returned as-is"
        );
    }

    /// Test API key prefix and key_id consistency
    #[tokio::test]
    #[ignore]
    async fn test_api_key_prefix_consistency() {
        let db_url =
            std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite::memory:".to_string());
        let db = Database::connect(&db_url).await.unwrap();
        setup_test_db(&db).await;

        let repo = SeaOrmRepository::new(db);

        // Create multiple keys and verify all are consistent
        for i in 0..3 {
            let admin_key = repo
                .create_api_key(&CreateApiKeyRequest {
                    workspace_id: None,
                    name: format!("Admin Key {}", i),
                    description: None,
                    role: ApiKeyRole::Admin,
                    rate_limit: None,
                    expires_at: None,
                    key_secret: None,
                })
                .await
                .unwrap();

            // Each key should have unique UUID portion
            let key_id_without_prefix = &admin_key.key.key_id[5..];

            // Verify it's a valid UUID (and therefore unique)
            let _ = uuid::Uuid::parse_str(key_id_without_prefix)
                .expect("key_id after prefix should be a valid UUID");

            // Verify structure: prefix + uuid format
            assert_eq!(admin_key.key.key_prefix, "niad_");
        }
    }

    /// Test validation rejects invalid key_secret length
    #[tokio::test]
    #[ignore]
    async fn test_api_key_secret_length_validation() {
        let db_url =
            std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite::memory:".to_string());
        let db = Database::connect(&db_url).await.unwrap();
        setup_test_db(&db).await;

        let repo = SeaOrmRepository::new(db);

        // Test too short secret
        let result = repo
            .create_api_key(&CreateApiKeyRequest {
                workspace_id: None,
                name: "Invalid Key".to_string(),
                description: None,
                role: ApiKeyRole::Admin,
                rate_limit: None,
                expires_at: None,
                key_secret: Some("short".to_string()),
            })
            .await;

        assert!(
            result.is_err(),
            "Should reject key_secret shorter than 16 characters"
        );
        if let Err(e) = result {
            assert!(
                e.to_string()
                    .contains("must be between 16 and 128 characters"),
                "Error should mention length requirement"
            );
        }

        // Test too long secret
        let too_long = "a".repeat(129);
        let result = repo
            .create_api_key(&CreateApiKeyRequest {
                workspace_id: None,
                name: "Invalid Key 2".to_string(),
                description: None,
                role: ApiKeyRole::Admin,
                rate_limit: None,
                expires_at: None,
                key_secret: Some(too_long),
            })
            .await;

        assert!(
            result.is_err(),
            "Should reject key_secret longer than 128 characters"
        );
    }

    /// Test get_api_key_by_id works with prefixed key_id
    #[tokio::test]
    #[ignore]
    async fn test_get_api_key_by_id_with_prefix() {
        let db_url =
            std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite::memory:".to_string());
        let db = Database::connect(&db_url).await.unwrap();
        setup_test_db(&db).await;

        let repo = SeaOrmRepository::new(db);

        let admin_key = repo
            .create_api_key(&CreateApiKeyRequest {
                workspace_id: None,
                name: "Test Key".to_string(),
                description: None,
                role: ApiKeyRole::Admin,
                rate_limit: None,
                expires_at: None,
                key_secret: None,
            })
            .await
            .unwrap();

        // Retrieve using the full prefixed key_id
        let retrieved = repo.get_api_key_by_id(&admin_key.key.key_id).await.unwrap();

        assert!(retrieved.is_some(), "Should find key with prefixed key_id");

        let retrieved_key = retrieved.unwrap();
        assert_eq!(retrieved_key.key_id, admin_key.key.key_id);
        assert_eq!(retrieved_key.key_prefix, admin_key.key.key_prefix);
        assert_eq!(retrieved_key.role, admin_key.key.role);
    }
}
