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
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QuerySelect, Set,
    TransactionTrait,
};
use serde_json::to_string;
use tracing::{debug, info};
use uuid::Uuid;

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
    Workspace, WorkspaceStatusDb,
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
}

use crate::database::biz_tag_entity::{BizTag, CreateBizTagRequest, UpdateBizTagRequest};
use crate::database::group_entity::{CreateGroupRequest, Group, UpdateGroupRequest};
use crate::database::workspace_entity::{CreateWorkspaceRequest, UpdateWorkspaceRequest};

pub struct SeaOrmRepository {
    db: sea_orm::DatabaseConnection,
}

impl SeaOrmRepository {
    pub fn new(db: sea_orm::DatabaseConnection) -> Self {
        Self { db }
    }
}

#[async_trait]
impl WorkspaceRepository for SeaOrmRepository {
    async fn create_workspace(&self, workspace: &CreateWorkspaceRequest) -> Result<Workspace> {
        let new_workspace = WorkspaceActiveModel {
            id: Set(uuid::Uuid::new_v4()),
            name: Set(workspace.name.clone()),
            description: Set(workspace.description.clone()),
            status: Set(WorkspaceStatusDb::Active), // 默认设置为Active
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

        Ok(segment)
    }

    async fn allocate_segment_with_dc(
        &self,
        workspace_id: &str,
        biz_tag: &str,
        step: i32,
        dc_id: i32,
    ) -> Result<SegmentInfo> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{ConnectOptions, ConnectionTrait, Database, Statement};

    async fn setup_test_db(db: &sea_orm::DatabaseConnection) {
        let backend = db.get_database_backend();

        db.execute(Statement::from_string(
            backend,
            r#"
            CREATE TABLE IF NOT EXISTS nebula_workspaces (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                description TEXT,
                status INTEGER NOT NULL DEFAULT 1,
                max_groups INTEGER NOT NULL DEFAULT 10,
                max_biz_tags INTEGER NOT NULL DEFAULT 100,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            )
            "#,
        ))
        .await
        .unwrap();

        db.execute(Statement::from_string(
            backend,
            r#"
            CREATE TABLE IF NOT EXISTS nebula_groups (
                id TEXT PRIMARY KEY,
                workspace_id TEXT NOT NULL,
                name TEXT NOT NULL,
                description TEXT,
                max_biz_tags INTEGER NOT NULL DEFAULT 50,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                FOREIGN KEY (workspace_id) REFERENCES nebula_workspaces(id)
            )
            "#,
        ))
        .await
        .unwrap();

        db.execute(Statement::from_string(
            backend,
            r#"
            CREATE TABLE IF NOT EXISTS nebula_biz_tags (
                id TEXT PRIMARY KEY,
                workspace_id TEXT NOT NULL,
                group_id TEXT NOT NULL,
                name TEXT NOT NULL,
                description TEXT,
                algorithm INTEGER NOT NULL DEFAULT 0,
                format INTEGER NOT NULL DEFAULT 0,
                prefix TEXT,
                base_step INTEGER NOT NULL DEFAULT 100,
                max_step INTEGER NOT NULL DEFAULT 1000,
                datacenter_ids TEXT NOT NULL DEFAULT '[]',
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                FOREIGN KEY (workspace_id) REFERENCES nebula_workspaces(id),
                FOREIGN KEY (group_id) REFERENCES nebula_groups(id)
            )
            "#,
        ))
        .await
        .unwrap();

        db.execute(Statement::from_string(
            backend,
            r#"
            CREATE TABLE IF NOT EXISTS nebula_segments (
                id TEXT PRIMARY KEY,
                workspace_id TEXT NOT NULL,
                biz_tag TEXT NOT NULL,
                current_id INTEGER NOT NULL DEFAULT 1,
                max_id INTEGER NOT NULL,
                step INTEGER NOT NULL DEFAULT 100,
                delta INTEGER NOT NULL DEFAULT 1,
                dc_id INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            )
            "#,
        ))
        .await
        .unwrap();
    }

    #[tokio::test]
    #[ignore]
    async fn test_repository_operations() {
        let opt = ConnectOptions::new("sqlite::memory:".to_string());
        let db = Database::connect(opt).await.unwrap();
        setup_test_db(&db).await;

        let repo = SeaOrmRepository::new(db);

        let segment = repo
            .allocate_segment("test_workspace", "test_tag", 100)
            .await
            .unwrap();

        assert_eq!(segment.workspace_id, "test_workspace");
        assert_eq!(segment.biz_tag, "test_tag");
        assert_eq!(segment.current_id, 1);
        assert_eq!(segment.max_id, 101);
        assert_eq!(segment.step, 100);

        let fetched = repo
            .get_segment("test_workspace", "test_tag")
            .await
            .unwrap()
            .unwrap();

        assert_eq!(fetched.id, segment.id);

        let segment2 = repo
            .allocate_segment("test_workspace", "test_tag", 100)
            .await
            .unwrap();

        assert_eq!(segment2.current_id, 101);
        assert_eq!(segment2.max_id, 201);

        let list = repo.list_segments("test_workspace").await.unwrap();

        assert_eq!(list.len(), 1);
    }

    #[tokio::test]
    #[ignore]
    async fn test_cascading_operations() {
        let opt = ConnectOptions::new("sqlite::memory:".to_string());
        let db = Database::connect(opt).await.unwrap();
        setup_test_db(&db).await;

        let repo = SeaOrmRepository::new(db);

        let workspace = repo
            .create_workspace(&CreateWorkspaceRequest {
                name: "test_workspace".to_string(),
                description: Some("Test workspace".to_string()),
                max_groups: Some(5),
                max_biz_tags: Some(50),
            })
            .await
            .unwrap();

        assert_eq!(workspace.name, "test_workspace");

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
}
