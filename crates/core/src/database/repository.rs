use async_trait::async_trait;
use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QuerySelect, Set, TransactionTrait,
};
use serde_json;
use tracing::{debug, info};
use uuid::Uuid;

use crate::database::biz_tag_entity::{
    ActiveModel as BizTagActiveModel, AlgorithmTypeDb, Column as BizTagColumn,
    Entity as BizTagEntity, IdFormatDb, Model as BizTagModel,
};
use crate::database::group_entity::{
    ActiveModel as GroupActiveModel, Column as GroupColumn, Entity as GroupEntity,
    Model as GroupModel,
};
use crate::database::segment_entity::{
    ActiveModel as SegmentActiveModel, Column as SegmentColumn, Entity as SegmentEntity,
};
use crate::database::workspace_entity::{
    ActiveModel as WorkspaceActiveModel, Column as WorkspaceColumn, Entity as WorkspaceEntity,
    Model as WorkspaceModel, Workspace, WorkspaceStatusDb,
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
            serde_json::to_string(&datacenter_ids_json).unwrap_or_else(|_| "[]".to_string());

        let new_biz_tag = BizTagActiveModel {
            id: Set(uuid::Uuid::new_v4()),
            workspace_id: Set(biz_tag.workspace_id),
            group_id: Set(biz_tag.group_id),
            name: Set(biz_tag.name.clone()),
            description: Set(biz_tag.description.clone()),
            algorithm: Set(biz_tag
                .algorithm
                .clone()
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
            serde_json::to_string(ids).unwrap_or_else(|_| existing.datacenter_ids.clone())
        } else {
            existing.datacenter_ids.clone()
        };

        let updated = BizTagActiveModel {
            id: Set(existing.id),
            name: Set(biz_tag.name.clone().unwrap_or(existing.name)),
            description: Set(biz_tag.description.clone().or(existing.description)),
            algorithm: Set(biz_tag
                .algorithm
                .clone()
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

    #[tokio::test]
    async fn test_repository_operations() {
        let opt = ConnectOptions::new("sqlite::memory:".to_string());
        let db = Database::connect(opt).await.unwrap();

        db.execute(Statement::from_string(
            db.get_database_backend(),
            "CREATE TABLE IF NOT EXISTS nebula_segments (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                workspace_id TEXT NOT NULL,
                biz_tag TEXT NOT NULL,
                current_id INTEGER NOT NULL DEFAULT 1,
                max_id INTEGER NOT NULL,
                step INTEGER NOT NULL DEFAULT 100,
                delta INTEGER NOT NULL DEFAULT 1,
                dc_id INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            )",
        ))
        .await
        .unwrap();

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
}
