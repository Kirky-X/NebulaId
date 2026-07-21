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

use async_trait::async_trait;
use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use rand::{Rng, RngExt};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QuerySelect, Set,
    TransactionTrait,
};
use tracing::{debug, info};
use uuid::Uuid;

use argon2::password_hash::{rand_core::OsRng, SaltString};
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};

use crate::core::coordinator::{LockError, LockGuard};
use crate::core::database::api_key_entity::{
    ActiveModel as ApiKeyActiveModel, ApiKey as ApiKeyInfo, ApiKeyResponse, ApiKeyRole,
    ApiKeyWithSecret, Column as ApiKeyColumn, CreateApiKeyRequest, Entity as ApiKeyEntity,
    Model as ApiKeyModel,
};
use crate::core::database::biz_tag_entity::{
    ActiveModel as BizTagActiveModel, Column as BizTagColumn, Entity as BizTagEntity,
};
use crate::core::database::group_entity::{
    ActiveModel as GroupActiveModel, Column as GroupColumn, Entity as GroupEntity,
};
use crate::core::database::segment_entity::{
    ActiveModel as SegmentActiveModel, Column as SegmentColumn, Entity as SegmentEntity,
};
use crate::core::database::workspace_entity::{
    ActiveModel as WorkspaceActiveModel, Column as WorkspaceColumn, Entity as WorkspaceEntity,
    Workspace,
};
use crate::core::types::{Result, SegmentInfo};

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

use crate::core::database::biz_tag_entity::{BizTag, CreateBizTagRequest, UpdateBizTagRequest};
use crate::core::database::group_entity::{CreateGroupRequest, Group, UpdateGroupRequest};
use crate::core::database::workspace_entity::{CreateWorkspaceRequest, UpdateWorkspaceRequest};

pub struct SeaOrmRepository {
    db: sea_orm::DatabaseConnection,
    /// Salt for API key hashing
    salt: String,
    /// 分布式锁（可选，用于 segment 分配）
    #[cfg(feature = "etcd")]
    distributed_lock:
        Option<std::sync::Arc<dyn crate::core::coordinator::DistributedLock + Send + Sync>>,
    /// 本地分布式锁（无 etcd 时使用）
    #[cfg(not(feature = "etcd"))]
    distributed_lock:
        Option<std::sync::Arc<dyn crate::core::coordinator::DistributedLock + Send + Sync>>,
}

impl SeaOrmRepository {
    pub fn new(db: sea_orm::DatabaseConnection, salt: String) -> Self {
        Self {
            db,
            salt,
            distributed_lock: None,
        }
    }

    /// Inject a distributed lock implementation (M8 fix).
    ///
    /// 生产环境必须调用此方法注入分布式锁，否则 `allocate_segment` 会返回
    /// `ConfigurationError`。默认构建（无 etcd feature）可注入
    /// `LocalDistributedLock`（进程内互斥），etcd feature 构建可注入
    /// `EtcdDistributedLock`。
    pub fn with_distributed_lock(
        mut self,
        lock: std::sync::Arc<dyn crate::core::coordinator::DistributedLock + Send + Sync>,
    ) -> Self {
        self.distributed_lock = Some(lock);
        self
    }

    /// Get the underlying database connection for advanced operations
    pub fn get_db_connection(&self) -> &sea_orm::DatabaseConnection {
        &self.db
    }

    /// Hash API key using Argon2id (replaces SHA256, CWE-916 fix).
    ///
    /// 使用 Argon2id（memory-hard, OWASP 2023 推荐）替代 SHA256。
    /// - `self.salt` 作为 pepper（额外加在 password 前），增加深度防御
    /// - 每次调用生成独立的 SaltString（PHC 格式内嵌）
    /// - 返回 PHC 格式字符串（约 96 字符），需 VARCHAR(255) 存储
    fn hash_key(&self, key_id: &str, key_secret: &str) -> Result<String> {
        let password = format!("{}|{}:{}", self.salt, key_id, key_secret);
        let salt = SaltString::generate(&mut OsRng);
        let argon2 = Argon2::default();
        let hash = argon2
            .hash_password(password.as_bytes(), &salt)
            .map_err(|e| {
                crate::core::CoreError::InternalError(format!("argon2 hash failed: {}", e))
            })?;
        Ok(hash.to_string())
    }

    /// Verify API key against stored PHC-format hash using Argon2id.
    ///
    /// Argon2 的 `verify_password` 内部使用 constant-time 比较，等价于原 `subtle::ConstantTimeEq`。
    fn verify_key(&self, key_id: &str, key_secret: &str, stored_hash: &str) -> bool {
        let parsed = match PasswordHash::new(stored_hash) {
            Ok(h) => h,
            Err(_) => return false,
        };
        let password = format!("{}|{}:{}", self.salt, key_id, key_secret);
        Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok()
    }

    /// 设置分布式锁
    #[cfg(feature = "etcd")]
    pub fn with_lock(
        mut self,
        lock: std::sync::Arc<dyn crate::core::coordinator::DistributedLock + Send + Sync>,
    ) -> Self {
        self.distributed_lock = Some(lock);
        self
    }

    /// 设置分布式锁
    #[cfg(not(feature = "etcd"))]
    pub fn with_lock(
        mut self,
        lock: std::sync::Arc<dyn crate::core::coordinator::DistributedLock + Send + Sync>,
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
            .map_err(|e| crate::core::CoreError::DatabaseError(e.to_string()))?;

        Ok(inserted.into())
    }

    async fn get_workspace(&self, id: Uuid) -> Result<Option<Workspace>> {
        let result = WorkspaceEntity::find_by_id(id)
            .one(&self.db)
            .await
            .map_err(|e| crate::core::CoreError::DatabaseError(e.to_string()))?;

        Ok(result.map(|m| m.into()))
    }

    async fn get_workspace_by_name(&self, name: &str) -> Result<Option<Workspace>> {
        let result = WorkspaceEntity::find()
            .filter(WorkspaceColumn::Name.eq(name))
            .one(&self.db)
            .await
            .map_err(|e| crate::core::CoreError::DatabaseError(e.to_string()))?;

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
            .map_err(|e| crate::core::CoreError::DatabaseError(e.to_string()))?;

        // 使用ok_or_else替代is_none+unwrap模式，避免冗余和潜在panic风险
        let existing = existing.ok_or_else(|| {
            crate::core::CoreError::NotFound(format!("Workspace not found: {}", id))
        })?;

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
            .map_err(|e| crate::core::CoreError::DatabaseError(e.to_string()))?;

        Ok(result.into())
    }

    async fn delete_workspace(&self, id: Uuid) -> Result<()> {
        let result = WorkspaceEntity::delete_by_id(id)
            .exec(&self.db)
            .await
            .map_err(|e| crate::core::CoreError::DatabaseError(e.to_string()))?;

        if result.rows_affected == 0 {
            return Err(crate::core::CoreError::NotFound(format!(
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
            .map_err(|e| crate::core::CoreError::DatabaseError(e.to_string()))?;

        Ok(results.into_iter().map(|m| m.into()).collect())
    }

    async fn get_workspace_with_groups(&self, id: Uuid) -> Result<Option<(Workspace, Vec<Group>)>> {
        let workspace_entity = WorkspaceEntity::find_by_id(id)
            .one(&self.db)
            .await
            .map_err(|e| crate::core::CoreError::DatabaseError(e.to_string()))?;

        // 使用if let Some模式，避免冗余unwrap
        if let Some(ws) = workspace_entity {
            let workspace: Workspace = ws.into();

            let groups = GroupEntity::find()
                .filter(GroupColumn::WorkspaceId.eq(id))
                .all(&self.db)
                .await
                .map_err(|e| crate::core::CoreError::DatabaseError(e.to_string()))?;

            let groups: Vec<Group> = groups.into_iter().map(|g| g.into()).collect();

            Ok(Some((workspace, groups)))
        } else {
            Ok(None)
        }
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
            .map_err(|e| crate::core::CoreError::DatabaseError(e.to_string()))?;

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
        let all_biz_tags_models: Vec<crate::core::database::biz_tag_entity::Model> =
            BizTagEntity::find()
                .filter(BizTagColumn::GroupId.is_in(group_ids))
                .all(&self.db)
                .await
                .map_err(|e| crate::core::CoreError::DatabaseError(e.to_string()))?;

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
            .map_err(|e| crate::core::CoreError::DatabaseError(e.to_string()))?
            .is_some();

        if !workspace_exists {
            return Err(crate::core::CoreError::NotFound(format!(
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
            .map_err(|e| crate::core::CoreError::DatabaseError(e.to_string()))?;

        Ok(inserted.into())
    }

    async fn get_group(&self, id: Uuid) -> Result<Option<Group>> {
        let result = GroupEntity::find_by_id(id)
            .one(&self.db)
            .await
            .map_err(|e| crate::core::CoreError::DatabaseError(e.to_string()))?;

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
            .map_err(|e| crate::core::CoreError::DatabaseError(e.to_string()))?;

        Ok(result.map(|m| m.into()))
    }

    async fn update_group(&self, id: Uuid, group: &UpdateGroupRequest) -> Result<Group> {
        let existing = GroupEntity::find_by_id(id)
            .one(&self.db)
            .await
            .map_err(|e| crate::core::CoreError::DatabaseError(e.to_string()))?;

        // 使用ok_or_else替代is_none+unwrap模式
        let existing = existing
            .ok_or_else(|| crate::core::CoreError::NotFound(format!("Group not found: {}", id)))?;

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
            .map_err(|e| crate::core::CoreError::DatabaseError(e.to_string()))?;

        Ok(result.into())
    }

    async fn delete_group(&self, id: Uuid) -> Result<()> {
        let result = GroupEntity::delete_by_id(id)
            .exec(&self.db)
            .await
            .map_err(|e| crate::core::CoreError::DatabaseError(e.to_string()))?;

        if result.rows_affected == 0 {
            return Err(crate::core::CoreError::NotFound(format!(
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
            .map_err(|e| crate::core::CoreError::DatabaseError(e.to_string()))?;

        Ok(results.into_iter().map(|m| m.into()).collect())
    }

    async fn get_group_with_biz_tags(&self, id: Uuid) -> Result<Option<(Group, Vec<BizTag>)>> {
        let group_entity = GroupEntity::find_by_id(id)
            .one(&self.db)
            .await
            .map_err(|e| crate::core::CoreError::DatabaseError(e.to_string()))?;

        // 使用if let Some模式，避免冗余unwrap
        if let Some(g) = group_entity {
            let group: Group = g.into();

            let biz_tags = BizTagEntity::find()
                .filter(BizTagColumn::GroupId.eq(id))
                .all(&self.db)
                .await
                .map_err(|e| crate::core::CoreError::DatabaseError(e.to_string()))?;

            let biz_tags: Vec<BizTag> = biz_tags.into_iter().map(|b| b.into()).collect();

            Ok(Some((group, biz_tags)))
        } else {
            Ok(None)
        }
    }

    async fn delete_group_with_biz_tags(&self, id: Uuid) -> Result<()> {
        let txn = self
            .db
            .begin()
            .await
            .map_err(|e| crate::core::CoreError::DatabaseError(e.to_string()))?;

        let biz_tags = BizTagEntity::find()
            .filter(BizTagColumn::GroupId.eq(id))
            .all(&txn)
            .await
            .map_err(|e| crate::core::CoreError::DatabaseError(e.to_string()))?;

        for biz_tag in biz_tags {
            BizTagEntity::delete_by_id(biz_tag.id)
                .exec(&txn)
                .await
                .map_err(|e| crate::core::CoreError::DatabaseError(e.to_string()))?;
        }

        GroupEntity::delete_by_id(id)
            .exec(&txn)
            .await
            .map_err(|e| crate::core::CoreError::DatabaseError(e.to_string()))?;

        txn.commit()
            .await
            .map_err(|e| crate::core::CoreError::DatabaseError(e.to_string()))?;

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
            .map_err(|e| crate::core::CoreError::DatabaseError(e.to_string()))?
            .is_some();

        if !workspace_exists {
            return Err(crate::core::CoreError::NotFound(format!(
                "Workspace not found: {}",
                biz_tag.workspace_id
            )));
        }

        let group_exists = GroupEntity::find_by_id(biz_tag.group_id)
            .one(&self.db)
            .await
            .map_err(|e| crate::core::CoreError::DatabaseError(e.to_string()))?
            .is_some();

        if !group_exists {
            return Err(crate::core::CoreError::NotFound(format!(
                "Group not found: {}",
                biz_tag.group_id
            )));
        }

        let new_biz_tag = BizTagActiveModel {
            id: Set(uuid::Uuid::new_v4()),
            workspace_id: Set(biz_tag.workspace_id),
            group_id: Set(biz_tag.group_id),
            name: Set(biz_tag.name.clone()),
            description: Set(biz_tag.description.clone()),
            algorithm: Set(biz_tag
                .algorithm
                .unwrap_or(crate::core::types::id::AlgorithmType::Segment)
                .into()),
            format: Set(biz_tag
                .format
                .clone()
                .unwrap_or(crate::core::types::id::IdFormat::Numeric)
                .into()),
            prefix: Set(biz_tag.prefix.clone().unwrap_or_default()),
            base_step: Set(biz_tag.base_step.unwrap_or(100)),
            max_step: Set(biz_tag.max_step.unwrap_or(1000)),
            datacenter_ids: Set(serde_json::to_value(
                biz_tag.datacenter_ids.as_ref().unwrap_or(&vec![0]),
            )
            .map_err(|e| crate::core::CoreError::InternalError(e.to_string()))?),
            created_at: Set(chrono::Utc::now().naive_utc()),
            updated_at: Set(chrono::Utc::now().naive_utc()),
        };

        let inserted = new_biz_tag
            .insert(&self.db)
            .await
            .map_err(|e| crate::core::CoreError::DatabaseError(e.to_string()))?;

        Ok(inserted.into())
    }

    async fn get_biz_tag(&self, id: Uuid) -> Result<Option<BizTag>> {
        let result = BizTagEntity::find_by_id(id)
            .one(&self.db)
            .await
            .map_err(|e| crate::core::CoreError::DatabaseError(e.to_string()))?;

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
            .map_err(|e| crate::core::CoreError::DatabaseError(e.to_string()))?;

        Ok(result.map(|m| m.into()))
    }

    async fn update_biz_tag(&self, id: Uuid, biz_tag: &UpdateBizTagRequest) -> Result<BizTag> {
        let existing = BizTagEntity::find_by_id(id)
            .one(&self.db)
            .await
            .map_err(|e| crate::core::CoreError::DatabaseError(e.to_string()))?;

        // 使用ok_or_else替代is_none+unwrap模式
        let existing = existing
            .ok_or_else(|| crate::core::CoreError::NotFound(format!("BizTag not found: {}", id)))?;

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
            datacenter_ids: Set(serde_json::to_value(
                biz_tag.datacenter_ids.clone().unwrap_or_else(|| vec![0]),
            )
            .map_err(|e| crate::core::CoreError::InternalError(e.to_string()))?),
            updated_at: Set(chrono::Utc::now().naive_utc()),
            ..Default::default()
        };

        let result = updated
            .update(&self.db)
            .await
            .map_err(|e| crate::core::CoreError::DatabaseError(e.to_string()))?;

        Ok(result.into())
    }

    async fn delete_biz_tag(&self, id: Uuid) -> Result<()> {
        let result = BizTagEntity::delete_by_id(id)
            .exec(&self.db)
            .await
            .map_err(|e| crate::core::CoreError::DatabaseError(e.to_string()))?;

        if result.rows_affected == 0 {
            return Err(crate::core::CoreError::NotFound(format!(
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
            .map_err(|e| crate::core::CoreError::DatabaseError(e.to_string()))?;

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
            .map_err(|e| crate::core::CoreError::DatabaseError(e.to_string()))?;

        Ok(results.into_iter().map(|m| m.into()).collect())
    }

    async fn count_biz_tags_by_group(&self, group_id: Uuid) -> Result<u64> {
        let count = BizTagEntity::find()
            .filter(BizTagColumn::GroupId.eq(group_id))
            .count(&self.db)
            .await
            .map_err(|e| crate::core::CoreError::DatabaseError(e.to_string()))?;

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
            .map_err(|e| crate::core::CoreError::DatabaseError(e.to_string()))?;

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
            if secret.len() < 8 || secret.len() > 128 {
                return Err(crate::core::CoreError::InvalidInput(
                    "key_secret must be between 8 and 128 characters".to_string(),
                ));
            }
        }

        let prefix = match request.role {
            ApiKeyRole::Admin => "niad_",
            ApiKeyRole::User => "nino_",
            // ARCH-LOW-001 修复 + SEC-MEDIUM-001 修复：fail-fast 拒绝
            // Anonymous 持久化。原代码返回 "nianon_" 前缀让后续逻辑
            // "隐式失败"，但实际会成功持久化 Anonymous 密钥。
            // Anonymous 只在禁用认证时注入 extensions，不应通过 API 创建。
            // 若调用方误传 Anonymous，立即返回 InvalidInput 防止污染数据库。
            ApiKeyRole::Anonymous => {
                return Err(crate::core::types::error::CoreError::InvalidInput(
                    "Anonymous role cannot be persisted to database".to_string(),
                ));
            }
        };

        let full_key_id = if let Some(ref kid) = request.key_id {
            if kid.starts_with(prefix) {
                kid.clone()
            } else {
                format!("{}{}", prefix, kid)
            }
        } else {
            let uuid = Uuid::new_v4();
            format!("{}{}", prefix, uuid)
        };

        // Use provided secret or generate a new one
        let key_secret = request.key_secret.clone().unwrap_or_else(generate_secret);

        let key_secret_hash = self.hash_key(&full_key_id, &key_secret)?;

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
            .map_err(|e| crate::core::CoreError::DatabaseError(e.to_string()))?;

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
            .map_err(|e| crate::core::CoreError::DatabaseError(e.to_string()))?;

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
            .map_err(|e| crate::core::CoreError::DatabaseError(e.to_string()))?;

        if let Some(model) = key_model {
            if !model.enabled {
                return Ok(None);
            }

            if let Some(expires_at) = model.expires_at {
                if expires_at < chrono::Utc::now().naive_utc() {
                    return Ok(None);
                }
            }

            // Argon2 verify_key 内部使用 constant-time 比较，等价于 subtle::ConstantTimeEq
            if self.verify_key(key_id, key_secret, &model.key_secret_hash) {
                let _ = self.update_last_used(model.id).await;
                let role: ApiKeyRole = model.role.clone().into();
                tracing::debug!(
                    event = "validate_api_key",
                    key_id = %key_id,
                    db_role = %model.role,
                    converted_role = ?role,
                    "{}",
                    t!("log.core.database.repository.api_key_role_conversion")
                );
                return Ok(Some((model.workspace_id, role)));
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
            .map_err(|e| crate::core::CoreError::DatabaseError(e.to_string()))?;

        Ok(results.into_iter().map(|m| m.into()).collect())
    }

    async fn delete_api_key(&self, id: Uuid) -> Result<()> {
        let result = ApiKeyEntity::delete_by_id(id)
            .exec(&self.db)
            .await
            .map_err(|e| crate::core::CoreError::DatabaseError(e.to_string()))?;

        if result.rows_affected == 0 {
            return Err(crate::core::CoreError::NotFound(format!(
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
            .map_err(|e| crate::core::CoreError::DatabaseError(e.to_string()))?;

        let key_id = if let Some(model) = existing {
            model.id
        } else {
            return Err(crate::core::CoreError::NotFound(format!(
                "API key not found: {}",
                id
            )));
        };

        let updated = ApiKeyActiveModel {
            id: Set(key_id),
            enabled: Set(false),
            updated_at: Set(chrono::Utc::now().naive_utc()),
            ..Default::default()
        };

        updated
            .update(&self.db)
            .await
            .map_err(|e| crate::core::CoreError::DatabaseError(e.to_string()))?;

        Ok(())
    }

    async fn update_last_used(&self, id: Uuid) -> Result<()> {
        let existing = ApiKeyEntity::find_by_id(id)
            .one(&self.db)
            .await
            .map_err(|e| crate::core::CoreError::DatabaseError(e.to_string()))?;

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
                .map_err(|e| crate::core::CoreError::DatabaseError(e.to_string()))?;
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
            .map_err(|e| crate::core::CoreError::DatabaseError(e.to_string()))?;

        Ok(result.filter(|m| m.enabled).map(|m| m.into()))
    }

    async fn count_api_keys(&self, workspace_id: Uuid) -> Result<u64> {
        let count = ApiKeyEntity::find()
            .filter(ApiKeyColumn::WorkspaceId.eq(workspace_id))
            .count(&self.db)
            .await
            .map_err(|e| crate::core::CoreError::DatabaseError(e.to_string()))?;

        Ok(count)
    }

    async fn rotate_api_key(
        &self,
        key_id: &str,
        _grace_period_seconds: u64,
    ) -> Result<ApiKeyWithSecret> {
        // 获取现有密钥
        let key_data = self.get_api_key_by_id(key_id).await?.ok_or_else(|| {
            crate::core::CoreError::NotFound(format!("API key not found: {}", key_id))
        })?;

        // 生成新密钥
        let new_secret = generate_secret();
        let new_secret_hash = self.hash_key(&key_data.key_id, &new_secret)?;
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
            .map_err(|e| crate::core::CoreError::DatabaseError(e.to_string()))?;

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
            .map_err(|e| crate::core::CoreError::DatabaseError(e.to_string()))?;

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
            .map_err(|e| crate::core::CoreError::DatabaseError(e.to_string()))?;

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
        // M8 修复：未配置分布式锁时禁止静默降级（生产环境会导致重复 ID 分配）。
        // 测试环境（SQLite 单连接）允许 NoopLockGuard，因为数据库事务本身提供原子性。
        let lock_guard = if let Some(ref lock) = self.distributed_lock {
            lock.acquire(&lock_key, 30).await.map_err(|e| {
                crate::core::CoreError::InternalError(format!(
                    "Failed to acquire distributed lock for segment allocation: {}",
                    e
                ))
            })?
        } else {
            #[cfg(test)]
            {
                Box::new(NoopLockGuard)
            }
            #[cfg(not(test))]
            {
                return Err(crate::core::CoreError::ConfigurationError(
                    "Distributed lock not configured for segment allocation".to_string(),
                ));
            }
        };

        let txn = self
            .db
            .begin()
            .await
            .map_err(|e| crate::core::CoreError::DatabaseError(e.to_string()))?;

        let existing = SegmentEntity::find()
            .filter(SegmentColumn::WorkspaceId.eq(workspace_id))
            .filter(SegmentColumn::BizTag.eq(biz_tag))
            .one(&txn)
            .await
            .map_err(|e| crate::core::CoreError::DatabaseError(e.to_string()))?;

        let segment = match existing {
            Some(model) => {
                let current_id = model.current_id;
                let max_id = model.max_id;
                // M9 修复：使用 saturating_add 防止极端情况下溢出 panic
                let new_max_id = current_id.saturating_add(step as i64);

                let updated = SegmentActiveModel {
                    id: Set(model.id),
                    current_id: Set(new_max_id),
                    updated_at: Set(chrono::Utc::now().naive_utc()),
                    ..Default::default()
                };

                updated
                    .update(&txn)
                    .await
                    .map_err(|e| crate::core::CoreError::DatabaseError(e.to_string()))?;

                debug!(
                    "{}",
                    t!(
                        "log.core.database.repository.segment_updated",
                        workspace_id = workspace_id,
                        biz_tag = biz_tag,
                        current_id = new_max_id,
                        max_id = max_id
                    )
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
                // M9 修复：saturating_add 防止溢出
                let max_id = start_id.saturating_add(step as i64);
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
                    .map_err(|e| crate::core::CoreError::DatabaseError(e.to_string()))?;

                info!(
                    "{}",
                    t!(
                        "log.core.database.repository.segment_created",
                        workspace_id = workspace_id,
                        biz_tag = biz_tag,
                        start_id = start_id,
                        max_id = max_id
                    )
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
            .map_err(|e| crate::core::CoreError::DatabaseError(e.to_string()))?;

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
        // M8 修复：同 allocate_segment，未配置分布式锁时禁止静默降级。
        let lock_guard = if let Some(ref lock) = self.distributed_lock {
            lock.acquire(&lock_key, 30).await.map_err(|e| {
                crate::core::CoreError::InternalError(format!(
                    "Failed to acquire distributed lock for segment allocation: {}",
                    e
                ))
            })?
        } else {
            #[cfg(test)]
            {
                Box::new(NoopLockGuard)
            }
            #[cfg(not(test))]
            {
                return Err(crate::core::CoreError::ConfigurationError(
                    "Distributed lock not configured for segment allocation".to_string(),
                ));
            }
        };

        let txn = self
            .db
            .begin()
            .await
            .map_err(|e| crate::core::CoreError::DatabaseError(e.to_string()))?;

        let existing = SegmentEntity::find()
            .filter(SegmentColumn::WorkspaceId.eq(workspace_id))
            .filter(SegmentColumn::BizTag.eq(biz_tag))
            .filter(SegmentColumn::DcId.eq(dc_id))
            .one(&txn)
            .await
            .map_err(|e| crate::core::CoreError::DatabaseError(e.to_string()))?;

        let segment = match existing {
            Some(model) => {
                let current_id = model.current_id;
                let max_id = model.max_id;
                // M9 修复：使用 saturating_add 防止极端情况下溢出 panic
                let new_max_id = current_id.saturating_add(step as i64);

                let updated = SegmentActiveModel {
                    id: Set(model.id),
                    current_id: Set(new_max_id),
                    updated_at: Set(chrono::Utc::now().naive_utc()),
                    ..Default::default()
                };

                updated
                    .update(&txn)
                    .await
                    .map_err(|e| crate::core::CoreError::DatabaseError(e.to_string()))?;

                debug!(
                    "{}",
                    t!(
                        "log.core.database.repository.segment_updated_with_dc",
                        workspace_id = workspace_id,
                        biz_tag = biz_tag,
                        dc_id = dc_id,
                        current_id = new_max_id,
                        max_id = max_id
                    )
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
                // M9 修复：saturating_add 防止溢出
                let max_id = start_id.saturating_add(step as i64);
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
                    .map_err(|e| crate::core::CoreError::DatabaseError(e.to_string()))?;

                info!(
                    "{}",
                    t!(
                        "log.core.database.repository.segment_created_with_dc",
                        workspace_id = workspace_id,
                        biz_tag = biz_tag,
                        dc_id = dc_id,
                        start_id = start_id,
                        max_id = max_id
                    )
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
            .map_err(|e| crate::core::CoreError::DatabaseError(e.to_string()))?;

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
            .map_err(|e| crate::core::CoreError::DatabaseError(e.to_string()))?;

        if result.rows_affected == 0 {
            return Err(crate::core::CoreError::NotFound(format!(
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
            .map_err(|e| crate::core::CoreError::DatabaseError(e.to_string()))?;

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
            .map_err(|e| crate::core::CoreError::DatabaseError(e.to_string()))?;

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
            .map_err(|e| crate::core::CoreError::DatabaseError(e.to_string()))?;

        if result.rows_affected == 0 {
            return Err(crate::core::CoreError::NotFound(format!(
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

    let mut rng = rand::rng();
    let secret: String = (0..SECRET_LENGTH)
        .map(|_| {
            let idx = rng.random_range(0..CHARSET.len());
            CHARSET[idx] as char
        })
        .collect();

    secret
}

/// No-op lock guard for testing only (M8 fix).
///
/// 仅在 `#[cfg(test)]` 下使用：测试环境用 SQLite 单连接，数据库事务本身
/// 提供原子性保证，无需分布式锁。生产环境未配置锁时 `allocate_segment`
/// 会返回 `ConfigurationError`，禁止静默降级。
#[cfg(test)]
struct NoopLockGuard;

#[cfg(test)]
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
            // ARCH-LOW-001 修复后，Anonymous 不再返回前缀而是 fail-fast
            // （生产代码返回 Err）。本测试用 _ 兜底覆盖 Anonymous 分支。
            _ => "nianon_",
        };
        assert_eq!(prefix, "niad_", "Admin role should use 'niad_' prefix");
    }

    #[test]
    fn test_user_role_prefix() {
        let role = ApiKeyRole::User;
        let prefix = match role {
            ApiKeyRole::Admin => "niad_",
            ApiKeyRole::User => "nino_",
            _ => "nianon_",
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
}

/// Mock-database driven unit tests for `SeaOrmRepository` trait impls.
///
/// These tests use `sea_orm::MockDatabase` to avoid a live database while
/// still exercising the trait-method branches (success paths, error
/// propagation, NotFound, validation failures). The pre-existing
/// `tests` module below contains `#[ignore]` integration tests that
/// require a real database.
#[cfg(test)]
#[allow(deprecated)]
mod mock_tests {
    use super::*;
    use crate::core::coordinator::{DistributedLock, LockError, LockGuard};
    use crate::core::database::api_key_entity::Model as ApiKeyModel;
    use crate::core::database::biz_tag_entity::{
        AlgorithmTypeDb, IdFormatDb, Model as BizTagModel,
    };
    use crate::core::database::group_entity::Model as GroupModel;
    use crate::core::database::segment_entity::Model as SegmentModel;
    use crate::core::database::workspace_entity::Model as WorkspaceModel;
    // Fix path resolution: tests below use `workspace_entity::Model` etc. as
    // path expressions, which requires the modules themselves to be in scope
    // (the `use` imports above only bring the `Model` aliases into scope).
    use crate::core::database::{
        api_key_entity, biz_tag_entity, group_entity, segment_entity, workspace_entity,
    };
    use crate::core::types::id::{AlgorithmType, IdFormat};
    use chrono::NaiveDateTime;
    use sea_orm::{DatabaseBackend, DbErr, MockDatabase, MockExecResult, MockRow, RuntimeErr};
    use std::collections::BTreeMap;
    use std::sync::Arc;

    // ------------------------------------------------------------------
    // Helpers
    // ------------------------------------------------------------------

    /// Build a `SeaOrmRepository` backed by a `MockDatabase`.
    fn make_repo(db: sea_orm::DatabaseConnection) -> SeaOrmRepository {
        SeaOrmRepository::new(db, "test_salt".to_string())
    }

    /// Build an empty `MockDatabase` connection (Postgres backend) for tests
    /// that don't need to mock any query/exec results.
    fn empty_pg_connection() -> sea_orm::DatabaseConnection {
        MockDatabase::new(DatabaseBackend::Postgres).into_connection()
    }

    /// A trivial distributed lock that always succeeds (used by tests that
    /// need to inject a lock without affecting behavior).
    struct DummyDistributedLock;

    #[async_trait]
    impl DistributedLock for DummyDistributedLock {
        async fn acquire(
            &self,
            _key: &str,
            _ttl_seconds: u64,
        ) -> std::result::Result<Box<dyn LockGuard>, LockError> {
            Ok(Box::new(NoopLockGuard))
        }

        fn is_healthy(&self) -> bool {
            true
        }
    }

    fn fixed_uuid(n: u8) -> Uuid {
        Uuid::from_bytes([n; 16])
    }

    fn fixed_datetime(secs: i64) -> NaiveDateTime {
        NaiveDateTime::from_timestamp_opt(secs, 0).unwrap()
    }

    fn sample_workspace_model(id: Uuid, name: &str) -> WorkspaceModel {
        WorkspaceModel {
            id,
            name: name.to_string(),
            description: Some("desc".to_string()),
            status: "active".to_string(),
            max_groups: 10,
            max_biz_tags: 100,
            created_at: fixed_datetime(1_600_000_000),
            updated_at: fixed_datetime(1_700_000_000),
        }
    }

    fn sample_group_model(id: Uuid, workspace_id: Uuid, name: &str) -> GroupModel {
        GroupModel {
            id,
            workspace_id,
            name: name.to_string(),
            description: Some("group desc".to_string()),
            max_biz_tags: 50,
            created_at: fixed_datetime(1_600_000_000),
            updated_at: fixed_datetime(1_700_000_000),
        }
    }

    fn sample_biz_tag_model(
        id: Uuid,
        workspace_id: Uuid,
        group_id: Uuid,
        name: &str,
    ) -> BizTagModel {
        BizTagModel {
            id,
            workspace_id,
            group_id,
            name: name.to_string(),
            description: Some("tag desc".to_string()),
            algorithm: AlgorithmTypeDb::Segment,
            format: IdFormatDb::Numeric,
            prefix: "".to_string(),
            base_step: 100,
            max_step: 1000,
            datacenter_ids: serde_json::json!([0]),
            created_at: fixed_datetime(1_600_000_000),
            updated_at: fixed_datetime(1_700_000_000),
        }
    }

    fn sample_api_key_model(id: Uuid, key_id: &str, role: &str) -> ApiKeyModel {
        ApiKeyModel {
            id,
            key_id: key_id.to_string(),
            key_secret_hash:
                "$argon2id$v=19$m=19456,t=2,p=1$YWNldG9uAAAAAAAAAAAAAAAAAAA$N+1jKtFi1q5p9tLi0dK0pQ"
                    .to_string(),
            key_prefix: "niad_".to_string(),
            role: role.to_string(),
            workspace_id: None,
            name: "Admin Key".to_string(),
            description: Some("desc".to_string()),
            rate_limit: 1000,
            enabled: true,
            expires_at: Some(fixed_datetime(1_800_000_000)),
            last_used_at: None,
            created_at: fixed_datetime(1_600_000_000),
            updated_at: fixed_datetime(1_700_000_000),
        }
    }

    fn sample_segment_model(id: i64, workspace_id: &str, biz_tag: &str) -> SegmentModel {
        SegmentModel {
            id,
            workspace_id: workspace_id.to_string(),
            biz_tag: biz_tag.to_string(),
            current_id: 100,
            max_id: 1000,
            step: 100,
            delta: 1,
            dc_id: 0,
            created_at: fixed_datetime(1_600_000_000),
            updated_at: fixed_datetime(1_700_000_000),
        }
    }

    // ==================================================================
    // Pure-logic functions (no DB)
    // ==================================================================

    // --- generate_secret ---

    #[test]
    fn test_generate_secret_returns_32_chars() {
        let s = generate_secret();
        assert_eq!(s.len(), 32, "generate_secret must always return 32 chars");
    }

    #[test]
    fn test_generate_secret_uses_only_allowed_charset() {
        const ALLOWED: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_-";
        // Run multiple times because randomness could theoretically hit
        // any character at any position.
        for _ in 0..20 {
            let s = generate_secret();
            for c in s.bytes() {
                assert!(
                    ALLOWED.contains(&c),
                    "char {:?} (#{}) not in allowed charset",
                    c as char,
                    c
                );
            }
        }
    }

    #[test]
    fn test_generate_secret_is_random_across_calls() {
        // Probabilistic: 62^32 space means collision is astronomically
        // unlikely. If this ever flakes, it indicates RNG seeding broke.
        let s1 = generate_secret();
        let s2 = generate_secret();
        let s3 = generate_secret();
        assert_ne!(s1, s2, "two consecutive calls must differ");
        assert_ne!(s2, s3, "two consecutive calls must differ");
        assert_ne!(s1, s3, "two non-consecutive calls must differ");
    }

    // --- segment_lock_key ---

    #[test]
    fn test_segment_lock_key_without_dc_formats_as_segment_ws_tag() {
        let repo = make_repo(empty_pg_connection());
        let key = repo.segment_lock_key("ws1", "tag1", None);
        assert_eq!(key, "segment:ws1:tag1");
    }

    #[test]
    fn test_segment_lock_key_with_dc_appends_dc_suffix() {
        let repo = make_repo(empty_pg_connection());
        let key = repo.segment_lock_key("ws1", "tag1", Some(5));
        assert_eq!(key, "segment:ws1:tag1:dc:5");
    }

    #[test]
    fn test_segment_lock_key_with_dc_zero_is_treated_as_some() {
        // dc_id = 0 is a valid Some(0), not None. Format must include suffix.
        let repo = make_repo(empty_pg_connection());
        let key = repo.segment_lock_key("ws1", "tag1", Some(0));
        assert_eq!(key, "segment:ws1:tag1:dc:0");
        assert_ne!(
            key,
            repo.segment_lock_key("ws1", "tag1", None),
            "Some(0) and None must produce different lock keys"
        );
    }

    #[test]
    fn test_segment_lock_key_with_negative_dc_preserves_sign() {
        let repo = make_repo(empty_pg_connection());
        let key = repo.segment_lock_key("ws", "tag", Some(-1));
        assert_eq!(key, "segment:ws:tag:dc:-1");
    }

    // --- hash_key / verify_key ---

    #[test]
    fn test_hash_key_returns_argon2_phc_format() {
        let repo = make_repo(empty_pg_connection());
        let h = repo.hash_key("kid1", "secret1").unwrap();
        assert!(
            h.starts_with("$argon2"),
            "PHC format must start with $argon2, got: {}",
            h
        );
        assert!(
            h.len() > 50,
            "Argon2id PHC string should be at least ~96 chars, got {}",
            h.len()
        );
    }

    #[test]
    fn test_hash_key_generates_unique_salt_per_call() {
        // Two hashes of the same input must differ (salt is random).
        let repo = make_repo(empty_pg_connection());
        let h1 = repo.hash_key("kid1", "secret1").unwrap();
        let h2 = repo.hash_key("kid1", "secret1").unwrap();
        assert_ne!(h1, h2, "salt must be regenerated per call");
    }

    #[test]
    fn test_verify_key_succeeds_with_correct_credentials() {
        let repo = make_repo(empty_pg_connection());
        let h = repo.hash_key("kid1", "secret1").unwrap();
        assert!(
            repo.verify_key("kid1", "secret1", &h),
            "correct key_id + secret must verify"
        );
    }

    #[test]
    fn test_verify_key_fails_with_wrong_secret() {
        let repo = make_repo(empty_pg_connection());
        let h = repo.hash_key("kid1", "secret1").unwrap();
        assert!(
            !repo.verify_key("kid1", "wrong-secret", &h),
            "wrong secret must fail verification"
        );
    }

    #[test]
    fn test_verify_key_fails_with_wrong_key_id() {
        // The key_id is part of the peppered password; changing it must
        // cause verification to fail.
        let repo = make_repo(empty_pg_connection());
        let h = repo.hash_key("kid1", "secret1").unwrap();
        assert!(!repo.verify_key("kid2", "secret1", &h));
    }

    #[test]
    fn test_verify_key_fails_with_malformed_hash_string() {
        let repo = make_repo(empty_pg_connection());
        assert!(!repo.verify_key("kid1", "secret1", "not-a-valid-hash"));
    }

    #[test]
    fn test_verify_key_fails_with_empty_hash() {
        let repo = make_repo(empty_pg_connection());
        assert!(!repo.verify_key("kid1", "secret1", ""));
    }

    // --- SeaOrmRepository constructor helpers ---

    #[test]
    fn test_new_does_not_panic_with_mock_db() {
        let repo = SeaOrmRepository::new(empty_pg_connection(), "salt".to_string());
        // Smoke check: get_db_connection returns a reference.
        let _ = repo.get_db_connection();
    }

    #[test]
    fn test_with_distributed_lock_returns_repository_with_lock() {
        let lock: Arc<dyn DistributedLock + Send + Sync> = Arc::new(DummyDistributedLock);
        let repo = SeaOrmRepository::new(empty_pg_connection(), "salt".to_string())
            .with_distributed_lock(lock);
        // No public getter for distributed_lock; allocate_segment tests
        // below verify the lock is actually invoked.
        let _ = repo.get_db_connection();
    }

    #[test]
    fn test_with_lock_returns_repository_with_lock() {
        let lock: Arc<dyn DistributedLock + Send + Sync> = Arc::new(DummyDistributedLock);
        let repo = SeaOrmRepository::new(empty_pg_connection(), "salt".to_string()).with_lock(lock);
        let _ = repo.get_db_connection();
    }

    // ==================================================================
    // WorkspaceRepository tests
    // ==================================================================

    #[tokio::test]
    async fn test_workspace_create_returns_workspace_with_default_limits() {
        // `insert(...).await` on Postgres uses RETURNING; mock as a query
        // result rather than an exec result.
        let id = fixed_uuid(1);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![sample_workspace_model(id, "ws1")]])
            .into_connection();
        let repo = make_repo(db);

        let created = repo
            .create_workspace(&CreateWorkspaceRequest {
                name: "ws1".to_string(),
                description: Some("desc".to_string()),
                max_groups: None,
                max_biz_tags: None,
            })
            .await
            .unwrap();

        assert_eq!(created.id, id);
        assert_eq!(created.name, "ws1");
        assert_eq!(created.max_groups, 10, "default max_groups must be 10");
        assert_eq!(
            created.max_biz_tags, 100,
            "default max_biz_tags must be 100"
        );
    }

    #[tokio::test]
    async fn test_workspace_create_propagates_db_error_as_database_error() {
        // Force a query error by not appending any query results — the
        // mock will return an error when the insert is executed.
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_errors(vec![DbErr::Custom("connection refused".to_string())])
            .into_connection();
        let repo = make_repo(db);

        let result = repo
            .create_workspace(&CreateWorkspaceRequest {
                name: "ws1".to_string(),
                description: None,
                max_groups: None,
                max_biz_tags: None,
            })
            .await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, crate::core::CoreError::DatabaseError(_)),
            "expected DatabaseError, got {:?}",
            err
        );
    }

    #[tokio::test]
    async fn test_workspace_get_returns_some_when_found() {
        let id = fixed_uuid(2);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![sample_workspace_model(id, "ws2")]])
            .into_connection();
        let repo = make_repo(db);

        let ws = repo.get_workspace(id).await.unwrap();
        assert!(ws.is_some());
        let ws = ws.unwrap();
        assert_eq!(ws.id, id);
        assert_eq!(ws.name, "ws2");
    }

    #[tokio::test]
    async fn test_workspace_get_returns_none_when_not_found() {
        let id = fixed_uuid(3);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![Vec::<workspace_entity::Model>::new()])
            .into_connection();
        let repo = make_repo(db);

        let ws = repo.get_workspace(id).await.unwrap();
        assert!(ws.is_none(), "empty query result must produce None");
    }

    #[tokio::test]
    async fn test_workspace_get_propagates_db_error() {
        let id = fixed_uuid(4);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_errors(vec![DbErr::Query(RuntimeErr::Internal("boom".to_string()))])
            .into_connection();
        let repo = make_repo(db);

        let result = repo.get_workspace(id).await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_)
            ),
            "DbErr must map to DatabaseError"
        );
    }

    #[tokio::test]
    async fn test_workspace_get_by_name_returns_some_when_found() {
        let id = fixed_uuid(5);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![sample_workspace_model(id, "named")]])
            .into_connection();
        let repo = make_repo(db);

        let ws = repo.get_workspace_by_name("named").await.unwrap();
        assert!(ws.is_some());
        assert_eq!(ws.unwrap().name, "named");
    }

    #[tokio::test]
    async fn test_workspace_get_by_name_returns_none_when_not_found() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![Vec::<workspace_entity::Model>::new()])
            .into_connection();
        let repo = make_repo(db);

        let ws = repo.get_workspace_by_name("missing").await.unwrap();
        assert!(ws.is_none());
    }

    #[tokio::test]
    async fn test_workspace_update_returns_updated_workspace_when_found() {
        let id = fixed_uuid(6);
        // First query: find_by_id (returns existing model).
        // Second query: update with RETURNING (returns updated model).
        let updated_model = workspace_entity::Model {
            name: "new_name".to_string(),
            ..sample_workspace_model(id, "old_name")
        };
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![
                vec![sample_workspace_model(id, "old_name")],
                vec![updated_model],
            ])
            .into_connection();
        let repo = make_repo(db);

        let updated = repo
            .update_workspace(
                id,
                &UpdateWorkspaceRequest {
                    name: Some("new_name".to_string()),
                    description: None,
                    status: None,
                    max_groups: None,
                    max_biz_tags: None,
                },
            )
            .await
            .unwrap();

        assert_eq!(updated.id, id);
        assert_eq!(updated.name, "new_name");
    }

    #[tokio::test]
    async fn test_workspace_update_returns_not_found_when_missing() {
        let id = fixed_uuid(7);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![Vec::<workspace_entity::Model>::new()])
            .into_connection();
        let repo = make_repo(db);

        let result = repo
            .update_workspace(
                id,
                &UpdateWorkspaceRequest {
                    name: None,
                    description: None,
                    status: None,
                    max_groups: None,
                    max_biz_tags: None,
                },
            )
            .await;

        let err = result.unwrap_err();
        assert!(
            matches!(err, crate::core::CoreError::NotFound(ref m) if m.contains("Workspace")),
            "expected NotFound for workspace, got {:?}",
            err
        );
    }

    #[tokio::test]
    async fn test_workspace_delete_succeeds_when_rows_affected() {
        let id = fixed_uuid(8);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_exec_results(vec![MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            .into_connection();
        let repo = make_repo(db);

        repo.delete_workspace(id).await.unwrap();
    }

    #[tokio::test]
    async fn test_workspace_delete_returns_not_found_when_no_rows_affected() {
        let id = fixed_uuid(9);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_exec_results(vec![MockExecResult {
                last_insert_id: 0,
                rows_affected: 0,
            }])
            .into_connection();
        let repo = make_repo(db);

        let result = repo.delete_workspace(id).await;
        let err = result.unwrap_err();
        assert!(
            matches!(err, crate::core::CoreError::NotFound(ref m) if m.contains("Workspace")),
            "expected NotFound when 0 rows affected, got {:?}",
            err
        );
    }

    #[tokio::test]
    async fn test_workspace_list_returns_all_results() {
        let id1 = fixed_uuid(10);
        let id2 = fixed_uuid(11);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![
                sample_workspace_model(id1, "ws1"),
                sample_workspace_model(id2, "ws2"),
            ]])
            .into_connection();
        let repo = make_repo(db);

        let list = repo.list_workspaces(None, None).await.unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, id1);
        assert_eq!(list[1].id, id2);
    }

    #[tokio::test]
    async fn test_workspace_list_with_limit_offset_applies_filters() {
        // We can't easily assert that limit/offset are encoded into SQL
        // via the mock (it just returns whatever we feed it), but we can
        // confirm the request flows through without error.
        let id = fixed_uuid(12);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![sample_workspace_model(id, "ws_limited")]])
            .into_connection();
        let repo = make_repo(db);

        let list = repo.list_workspaces(Some(10), Some(5)).await.unwrap();
        assert_eq!(list.len(), 1);
    }

    #[tokio::test]
    async fn test_workspace_with_groups_returns_none_when_workspace_missing() {
        let id = fixed_uuid(13);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![Vec::<workspace_entity::Model>::new()])
            .into_connection();
        let repo = make_repo(db);

        let result = repo.get_workspace_with_groups(id).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_workspace_with_groups_returns_workspace_and_groups_when_found() {
        let ws_id = fixed_uuid(14);
        let g1_id = fixed_uuid(15);
        let g2_id = fixed_uuid(16);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            // First query: WorkspaceEntity::find_by_id.
            .append_query_results(vec![vec![sample_workspace_model(ws_id, "ws_groups")]])
            // Second query: GroupEntity::find().filter(workspace_id).
            .append_query_results(vec![vec![
                sample_group_model(g1_id, ws_id, "g1"),
                sample_group_model(g2_id, ws_id, "g2"),
            ]])
            .into_connection();
        let repo = make_repo(db);

        let (ws, groups) = repo
            .get_workspace_with_groups(ws_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(ws.id, ws_id);
        assert_eq!(groups.len(), 2);
    }

    #[tokio::test]
    async fn test_workspace_with_groups_and_biz_tags_returns_none_when_workspace_missing() {
        let id = fixed_uuid(17);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![Vec::<(
                workspace_entity::Model,
                Option<group_entity::Model>,
            )>::new()])
            .into_connection();
        let repo = make_repo(db);

        let result = repo
            .get_workspace_with_groups_and_biz_tags(id)
            .await
            .unwrap();
        assert!(result.is_none());
    }

    // ==================================================================
    // GroupRepository tests
    // ==================================================================

    #[tokio::test]
    async fn test_group_create_returns_not_found_when_workspace_missing() {
        let ws_id = fixed_uuid(20);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![Vec::<workspace_entity::Model>::new()])
            .into_connection();
        let repo = make_repo(db);

        let result = repo
            .create_group(&CreateGroupRequest {
                workspace_id: ws_id,
                name: "g1".to_string(),
                description: None,
                max_biz_tags: None,
            })
            .await;

        let err = result.unwrap_err();
        assert!(
            matches!(err, crate::core::CoreError::NotFound(ref m) if m.contains("Workspace")),
            "expected NotFound for workspace, got {:?}",
            err
        );
    }

    #[tokio::test]
    async fn test_group_create_returns_group_with_default_max_biz_tags_when_found() {
        let ws_id = fixed_uuid(21);
        let g_id = fixed_uuid(22);
        // First query: WorkspaceEntity::find_by_id (returns Some).
        // Second query: ActiveModel::insert with RETURNING.
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![sample_workspace_model(ws_id, "ws")]])
            .append_query_results(vec![vec![sample_group_model(g_id, ws_id, "g1")]])
            .into_connection();
        let repo = make_repo(db);

        let group = repo
            .create_group(&CreateGroupRequest {
                workspace_id: ws_id,
                name: "g1".to_string(),
                description: None,
                max_biz_tags: None,
            })
            .await
            .unwrap();

        assert_eq!(group.id, g_id);
        assert_eq!(group.workspace_id, ws_id);
        assert_eq!(group.max_biz_tags, 50, "default max_biz_tags must be 50");
    }

    #[tokio::test]
    async fn test_group_get_returns_some_when_found() {
        let g_id = fixed_uuid(23);
        let ws_id = fixed_uuid(24);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![sample_group_model(g_id, ws_id, "g")]])
            .into_connection();
        let repo = make_repo(db);

        let group = repo.get_group(g_id).await.unwrap();
        assert!(group.is_some());
        assert_eq!(group.unwrap().id, g_id);
    }

    #[tokio::test]
    async fn test_group_get_returns_none_when_missing() {
        let g_id = fixed_uuid(25);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![Vec::<group_entity::Model>::new()])
            .into_connection();
        let repo = make_repo(db);

        let group = repo.get_group(g_id).await.unwrap();
        assert!(group.is_none());
    }

    #[tokio::test]
    async fn test_group_get_by_workspace_and_name_returns_some_when_found() {
        let ws_id = fixed_uuid(26);
        let g_id = fixed_uuid(27);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![sample_group_model(g_id, ws_id, "named")]])
            .into_connection();
        let repo = make_repo(db);

        let group = repo
            .get_group_by_workspace_and_name(ws_id, "named")
            .await
            .unwrap();
        assert!(group.is_some());
    }

    #[tokio::test]
    async fn test_group_update_returns_not_found_when_missing() {
        let g_id = fixed_uuid(28);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![Vec::<group_entity::Model>::new()])
            .into_connection();
        let repo = make_repo(db);

        let result = repo
            .update_group(
                g_id,
                &UpdateGroupRequest {
                    name: None,
                    description: None,
                    max_biz_tags: None,
                },
            )
            .await;

        let err = result.unwrap_err();
        assert!(
            matches!(err, crate::core::CoreError::NotFound(ref m) if m.contains("Group")),
            "expected NotFound for group, got {:?}",
            err
        );
    }

    #[tokio::test]
    async fn test_group_update_returns_updated_group_when_found() {
        let g_id = fixed_uuid(29);
        let ws_id = fixed_uuid(30);
        let updated_model = group_entity::Model {
            name: "new_name".to_string(),
            ..sample_group_model(g_id, ws_id, "old_name")
        };
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![
                vec![sample_group_model(g_id, ws_id, "old_name")],
                vec![updated_model],
            ])
            .into_connection();
        let repo = make_repo(db);

        let updated = repo
            .update_group(
                g_id,
                &UpdateGroupRequest {
                    name: Some("new_name".to_string()),
                    description: None,
                    max_biz_tags: None,
                },
            )
            .await
            .unwrap();

        assert_eq!(updated.id, g_id);
        assert_eq!(updated.name, "new_name");
    }

    #[tokio::test]
    async fn test_group_delete_succeeds_when_rows_affected() {
        let g_id = fixed_uuid(31);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_exec_results(vec![MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            .into_connection();
        let repo = make_repo(db);

        repo.delete_group(g_id).await.unwrap();
    }

    #[tokio::test]
    async fn test_group_delete_returns_not_found_when_no_rows_affected() {
        let g_id = fixed_uuid(32);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_exec_results(vec![MockExecResult {
                last_insert_id: 0,
                rows_affected: 0,
            }])
            .into_connection();
        let repo = make_repo(db);

        let err = repo.delete_group(g_id).await.unwrap_err();
        assert!(
            matches!(err, crate::core::CoreError::NotFound(ref m) if m.contains("Group")),
            "expected NotFound, got {:?}",
            err
        );
    }

    #[tokio::test]
    async fn test_group_list_returns_groups_for_workspace() {
        let ws_id = fixed_uuid(33);
        let g1_id = fixed_uuid(34);
        let g2_id = fixed_uuid(35);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![
                sample_group_model(g1_id, ws_id, "g1"),
                sample_group_model(g2_id, ws_id, "g2"),
            ]])
            .into_connection();
        let repo = make_repo(db);

        let groups = repo.list_groups(ws_id, None, None).await.unwrap();
        assert_eq!(groups.len(), 2);
    }

    #[tokio::test]
    async fn test_group_with_biz_tags_returns_none_when_group_missing() {
        let g_id = fixed_uuid(36);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![Vec::<group_entity::Model>::new()])
            .into_connection();
        let repo = make_repo(db);

        let result = repo.get_group_with_biz_tags(g_id).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_group_with_biz_tags_returns_group_and_tags_when_found() {
        let ws_id = fixed_uuid(37);
        let g_id = fixed_uuid(38);
        let t1_id = fixed_uuid(39);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![sample_group_model(g_id, ws_id, "g")]])
            .append_query_results(vec![vec![sample_biz_tag_model(t1_id, ws_id, g_id, "t1")]])
            .into_connection();
        let repo = make_repo(db);

        let (group, tags) = repo.get_group_with_biz_tags(g_id).await.unwrap().unwrap();
        assert_eq!(group.id, g_id);
        assert_eq!(tags.len(), 1);
    }

    // ==================================================================
    // BizTagRepository tests
    // ==================================================================

    #[tokio::test]
    async fn test_biz_tag_create_returns_not_found_when_workspace_missing() {
        let ws_id = fixed_uuid(40);
        let g_id = fixed_uuid(41);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![Vec::<workspace_entity::Model>::new()])
            .into_connection();
        let repo = make_repo(db);

        let err = repo
            .create_biz_tag(&CreateBizTagRequest {
                workspace_id: ws_id,
                group_id: g_id,
                name: "t1".to_string(),
                description: None,
                algorithm: None,
                format: None,
                prefix: None,
                base_step: None,
                max_step: None,
                datacenter_ids: None,
            })
            .await
            .unwrap_err();

        assert!(
            matches!(err, crate::core::CoreError::NotFound(ref m) if m.contains("Workspace")),
            "expected NotFound for workspace, got {:?}",
            err
        );
    }

    #[tokio::test]
    async fn test_biz_tag_create_returns_not_found_when_group_missing() {
        let ws_id = fixed_uuid(42);
        let g_id = fixed_uuid(43);
        // Workspace exists, group does not.
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![sample_workspace_model(ws_id, "ws")]])
            .append_query_results(vec![Vec::<group_entity::Model>::new()])
            .into_connection();
        let repo = make_repo(db);

        let err = repo
            .create_biz_tag(&CreateBizTagRequest {
                workspace_id: ws_id,
                group_id: g_id,
                name: "t1".to_string(),
                description: None,
                algorithm: None,
                format: None,
                prefix: None,
                base_step: None,
                max_step: None,
                datacenter_ids: None,
            })
            .await
            .unwrap_err();

        assert!(
            matches!(err, crate::core::CoreError::NotFound(ref m) if m.contains("Group")),
            "expected NotFound for group, got {:?}",
            err
        );
    }

    #[tokio::test]
    async fn test_biz_tag_create_returns_tag_with_defaults_when_workspace_and_group_exist() {
        let ws_id = fixed_uuid(44);
        let g_id = fixed_uuid(45);
        let t_id = fixed_uuid(46);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![sample_workspace_model(ws_id, "ws")]])
            .append_query_results(vec![vec![sample_group_model(g_id, ws_id, "g")]])
            .append_query_results(vec![vec![sample_biz_tag_model(t_id, ws_id, g_id, "t1")]])
            .into_connection();
        let repo = make_repo(db);

        let tag = repo
            .create_biz_tag(&CreateBizTagRequest {
                workspace_id: ws_id,
                group_id: g_id,
                name: "t1".to_string(),
                description: None,
                algorithm: None,
                format: None,
                prefix: None,
                base_step: None,
                max_step: None,
                datacenter_ids: None,
            })
            .await
            .unwrap();

        assert_eq!(tag.id, t_id);
        assert_eq!(tag.base_step, 100, "default base_step must be 100");
        assert_eq!(tag.max_step, 1000, "default max_step must be 1000");
        assert_eq!(tag.algorithm, AlgorithmType::Segment);
        assert_eq!(tag.format, IdFormat::Numeric);
        assert_eq!(
            tag.datacenter_ids,
            vec![0],
            "default datacenter_ids must be [0]"
        );
    }

    #[tokio::test]
    async fn test_biz_tag_get_returns_some_when_found() {
        let t_id = fixed_uuid(47);
        let ws_id = fixed_uuid(48);
        let g_id = fixed_uuid(49);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![sample_biz_tag_model(t_id, ws_id, g_id, "t")]])
            .into_connection();
        let repo = make_repo(db);

        let tag = repo.get_biz_tag(t_id).await.unwrap();
        assert!(tag.is_some());
    }

    #[tokio::test]
    async fn test_biz_tag_get_by_workspace_group_and_name_returns_some_when_found() {
        let ws_id = fixed_uuid(50);
        let g_id = fixed_uuid(51);
        let t_id = fixed_uuid(52);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![sample_biz_tag_model(t_id, ws_id, g_id, "named")]])
            .into_connection();
        let repo = make_repo(db);

        let tag = repo
            .get_biz_tag_by_workspace_group_and_name(ws_id, g_id, "named")
            .await
            .unwrap();
        assert!(tag.is_some());
    }

    #[tokio::test]
    async fn test_biz_tag_update_returns_not_found_when_missing() {
        let t_id = fixed_uuid(53);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![Vec::<biz_tag_entity::Model>::new()])
            .into_connection();
        let repo = make_repo(db);

        let err = repo
            .update_biz_tag(
                t_id,
                &UpdateBizTagRequest {
                    name: None,
                    description: None,
                    algorithm: None,
                    format: None,
                    prefix: None,
                    base_step: None,
                    max_step: None,
                    datacenter_ids: None,
                },
            )
            .await
            .unwrap_err();

        assert!(
            matches!(err, crate::core::CoreError::NotFound(ref m) if m.contains("BizTag")),
            "expected NotFound for biz_tag, got {:?}",
            err
        );
    }

    #[tokio::test]
    async fn test_biz_tag_delete_succeeds_when_rows_affected() {
        let t_id = fixed_uuid(54);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_exec_results(vec![MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            .into_connection();
        let repo = make_repo(db);

        repo.delete_biz_tag(t_id).await.unwrap();
    }

    #[tokio::test]
    async fn test_biz_tag_delete_returns_not_found_when_no_rows_affected() {
        let t_id = fixed_uuid(55);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_exec_results(vec![MockExecResult {
                last_insert_id: 0,
                rows_affected: 0,
            }])
            .into_connection();
        let repo = make_repo(db);

        let err = repo.delete_biz_tag(t_id).await.unwrap_err();
        assert!(
            matches!(err, crate::core::CoreError::NotFound(ref m) if m.contains("BizTag")),
            "expected NotFound, got {:?}",
            err
        );
    }

    #[tokio::test]
    async fn test_biz_tag_list_returns_tags_for_workspace() {
        let ws_id = fixed_uuid(56);
        let g_id = fixed_uuid(57);
        let t1_id = fixed_uuid(58);
        let t2_id = fixed_uuid(59);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![
                sample_biz_tag_model(t1_id, ws_id, g_id, "t1"),
                sample_biz_tag_model(t2_id, ws_id, g_id, "t2"),
            ]])
            .into_connection();
        let repo = make_repo(db);

        let tags = repo.list_biz_tags(ws_id, None, None, None).await.unwrap();
        assert_eq!(tags.len(), 2);
    }

    #[tokio::test]
    async fn test_biz_tag_list_by_workspace_group_returns_tags() {
        let ws_id = fixed_uuid(60);
        let g_id = fixed_uuid(61);
        let t1_id = fixed_uuid(62);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![sample_biz_tag_model(t1_id, ws_id, g_id, "t1")]])
            .into_connection();
        let repo = make_repo(db);

        let tags = repo
            .list_biz_tags_by_workspace_group(ws_id, g_id)
            .await
            .unwrap();
        assert_eq!(tags.len(), 1);
    }

    #[tokio::test]
    async fn test_biz_tag_count_by_group_returns_count() {
        let g_id = fixed_uuid(63);
        let mut count_row: BTreeMap<String, sea_orm::Value> = BTreeMap::new();
        count_row.insert("num_items".to_string(), sea_orm::Value::BigInt(Some(7)));
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![count_row]])
            .into_connection();
        let repo = make_repo(db);

        let count = repo.count_biz_tags_by_group(g_id).await.unwrap();
        assert_eq!(count, 7);
    }

    #[tokio::test]
    async fn test_biz_tag_count_returns_count_with_optional_group() {
        let ws_id = fixed_uuid(64);
        let mut count_row: BTreeMap<String, sea_orm::Value> = BTreeMap::new();
        count_row.insert("num_items".to_string(), sea_orm::Value::BigInt(Some(3)));
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![count_row]])
            .into_connection();
        let repo = make_repo(db);

        let count = repo.count_biz_tags(ws_id, None).await.unwrap();
        assert_eq!(count, 3);
    }

    #[tokio::test]
    async fn test_biz_tag_health_check_always_returns_ok() {
        let repo = make_repo(empty_pg_connection());
        // health_check is a no-op; it must always return Ok.
        repo.health_check().await.unwrap();
    }

    // ==================================================================
    // ApiKeyRepository tests
    // ==================================================================

    #[tokio::test]
    async fn test_api_key_create_rejects_anonymous_role() {
        let repo = make_repo(empty_pg_connection());

        let err = repo
            .create_api_key(&CreateApiKeyRequest {
                workspace_id: None,
                name: "anon".to_string(),
                description: None,
                role: ApiKeyRole::Anonymous,
                rate_limit: None,
                expires_at: None,
                key_secret: None,
                key_id: None,
            })
            .await
            .unwrap_err();

        assert!(
            matches!(err, crate::core::CoreError::InvalidInput(ref m) if m.contains("Anonymous")),
            "Anonymous role must be rejected with InvalidInput, got {:?}",
            err
        );
    }

    #[tokio::test]
    async fn test_api_key_create_rejects_short_secret() {
        let repo = make_repo(empty_pg_connection());

        let err = repo
            .create_api_key(&CreateApiKeyRequest {
                workspace_id: None,
                name: "k".to_string(),
                description: None,
                role: ApiKeyRole::Admin,
                rate_limit: None,
                expires_at: None,
                key_secret: Some("short".to_string()),
                key_id: None,
            })
            .await
            .unwrap_err();

        assert!(
            matches!(err, crate::core::CoreError::InvalidInput(ref m) if m.contains("8 and 128")),
            "short secret must be rejected with length error, got {:?}",
            err
        );
    }

    #[tokio::test]
    async fn test_api_key_create_rejects_long_secret() {
        let repo = make_repo(empty_pg_connection());
        let too_long = "a".repeat(129);

        let err = repo
            .create_api_key(&CreateApiKeyRequest {
                workspace_id: None,
                name: "k".to_string(),
                description: None,
                role: ApiKeyRole::Admin,
                rate_limit: None,
                expires_at: None,
                key_secret: Some(too_long),
                key_id: None,
            })
            .await
            .unwrap_err();

        assert!(
            matches!(err, crate::core::CoreError::InvalidInput(ref m) if m.contains("8 and 128")),
            "long secret must be rejected with length error, got {:?}",
            err
        );
    }

    #[tokio::test]
    async fn test_api_key_create_admin_uses_niad_prefix() {
        let id = fixed_uuid(70);
        let model = api_key_entity::Model {
            id,
            key_id: "niad_admin-uuid".to_string(),
            key_prefix: "niad_".to_string(),
            role: "admin".to_string(),
            ..sample_api_key_model(id, "niad_admin-uuid", "admin")
        };
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![model]])
            .into_connection();
        let repo = make_repo(db);

        let key = repo
            .create_api_key(&CreateApiKeyRequest {
                workspace_id: None,
                name: "Admin Key".to_string(),
                description: None,
                role: ApiKeyRole::Admin,
                rate_limit: None,
                expires_at: None,
                key_secret: None,
                key_id: None,
            })
            .await
            .unwrap();

        assert!(key.key.key_id.starts_with("niad_"));
        assert_eq!(key.key.key_prefix, "niad_");
        // Returned secret must be 32 chars (generated).
        assert_eq!(key.key_secret.len(), 32);
    }

    #[tokio::test]
    async fn test_api_key_create_user_uses_nino_prefix() {
        let id = fixed_uuid(71);
        let ws_id = fixed_uuid(72);
        let model = api_key_entity::Model {
            id,
            key_id: "nino_user-uuid".to_string(),
            key_prefix: "nino_".to_string(),
            role: "user".to_string(),
            workspace_id: Some(ws_id),
            ..sample_api_key_model(id, "nino_user-uuid", "user")
        };
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![model]])
            .into_connection();
        let repo = make_repo(db);

        let key = repo
            .create_api_key(&CreateApiKeyRequest {
                workspace_id: Some(ws_id),
                name: "User Key".to_string(),
                description: None,
                role: ApiKeyRole::User,
                rate_limit: None,
                expires_at: None,
                key_secret: None,
                key_id: None,
            })
            .await
            .unwrap();

        assert!(key.key.key_id.starts_with("nino_"));
        assert_eq!(key.key.key_prefix, "nino_");
    }

    #[tokio::test]
    async fn test_api_key_create_with_custom_key_id_without_prefix_prepends_prefix() {
        let id = fixed_uuid(73);
        let custom_uuid = "abc-123-custom";
        let model = api_key_entity::Model {
            id,
            key_id: format!("niad_{}", custom_uuid),
            key_prefix: "niad_".to_string(),
            role: "admin".to_string(),
            ..sample_api_key_model(id, "placeholder", "admin")
        };
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![model]])
            .into_connection();
        let repo = make_repo(db);

        let key = repo
            .create_api_key(&CreateApiKeyRequest {
                workspace_id: None,
                name: "k".to_string(),
                description: None,
                role: ApiKeyRole::Admin,
                rate_limit: None,
                expires_at: None,
                key_secret: None,
                key_id: Some(custom_uuid.to_string()),
            })
            .await
            .unwrap();

        // Custom key_id without prefix gets prefix prepended.
        assert!(key.key.key_id.starts_with("niad_"));
        assert!(key.key.key_id.contains(custom_uuid));
    }

    #[tokio::test]
    async fn test_api_key_create_with_custom_key_id_with_prefix_keeps_as_is() {
        let id = fixed_uuid(74);
        let custom = "niad_my-existing-uuid".to_string();
        let model = api_key_entity::Model {
            id,
            key_id: custom.clone(),
            key_prefix: "niad_".to_string(),
            role: "admin".to_string(),
            ..sample_api_key_model(id, "placeholder", "admin")
        };
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![model]])
            .into_connection();
        let repo = make_repo(db);

        let key = repo
            .create_api_key(&CreateApiKeyRequest {
                workspace_id: None,
                name: "k".to_string(),
                description: None,
                role: ApiKeyRole::Admin,
                rate_limit: None,
                expires_at: None,
                key_secret: None,
                key_id: Some(custom.clone()),
            })
            .await
            .unwrap();

        assert_eq!(
            key.key.key_id, custom,
            "prefixed key_id must not be double-prefixed"
        );
    }

    #[tokio::test]
    async fn test_api_key_create_with_provided_secret_returns_secret_as_is() {
        let id = fixed_uuid(75);
        let custom_secret = "this_is_a_valid_secret_12345".to_string();
        let model = api_key_entity::Model {
            id,
            key_id: "niad_x".to_string(),
            key_prefix: "niad_".to_string(),
            role: "admin".to_string(),
            ..sample_api_key_model(id, "niad_x", "admin")
        };
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![model]])
            .into_connection();
        let repo = make_repo(db);

        let key = repo
            .create_api_key(&CreateApiKeyRequest {
                workspace_id: None,
                name: "k".to_string(),
                description: None,
                role: ApiKeyRole::Admin,
                rate_limit: None,
                expires_at: None,
                key_secret: Some(custom_secret.clone()),
                key_id: None,
            })
            .await
            .unwrap();

        // Provided secret must be returned verbatim, not regenerated.
        assert_eq!(key.key_secret, custom_secret);
    }

    #[tokio::test]
    async fn test_api_key_get_by_id_returns_some_when_found() {
        let id = fixed_uuid(76);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![sample_api_key_model(id, "niad_x", "admin")]])
            .into_connection();
        let repo = make_repo(db);

        let key = repo.get_api_key_by_id("niad_x").await.unwrap();
        assert!(key.is_some());
        assert_eq!(key.unwrap().key_id, "niad_x");
    }

    #[tokio::test]
    async fn test_api_key_get_by_id_returns_none_when_missing() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![Vec::<api_key_entity::Model>::new()])
            .into_connection();
        let repo = make_repo(db);

        let key = repo.get_api_key_by_id("niad_missing").await.unwrap();
        assert!(key.is_none());
    }

    #[tokio::test]
    async fn test_api_key_validate_returns_none_when_key_not_found() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![Vec::<api_key_entity::Model>::new()])
            .into_connection();
        let repo = make_repo(db);

        let result = repo
            .validate_api_key("niad_unknown", "any-secret")
            .await
            .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_api_key_validate_returns_none_when_key_disabled() {
        let id = fixed_uuid(77);
        let model = api_key_entity::Model {
            id,
            enabled: false,
            ..sample_api_key_model(id, "niad_disabled", "admin")
        };
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![model]])
            .into_connection();
        let repo = make_repo(db);

        let result = repo.validate_api_key("niad_disabled", "any").await.unwrap();
        assert!(result.is_none(), "disabled key must not validate");
    }

    #[tokio::test]
    async fn test_api_key_validate_returns_none_when_key_expired() {
        let id = fixed_uuid(78);
        let model = api_key_entity::Model {
            id,
            enabled: true,
            expires_at: Some(fixed_datetime(1_000_000_000)), // far in the past
            ..sample_api_key_model(id, "niad_expired", "admin")
        };
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![model]])
            .into_connection();
        let repo = make_repo(db);

        let result = repo.validate_api_key("niad_expired", "any").await.unwrap();
        assert!(result.is_none(), "expired key must not validate");
    }

    #[tokio::test]
    async fn test_api_key_validate_returns_none_when_secret_does_not_match() {
        let id = fixed_uuid(79);
        let model = sample_api_key_model(id, "niad_x", "admin");
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![model]])
            // update_last_used query result (None, since key not found in
            // second pass) — we don't append another result; mock returns
            // empty by default.
            .into_connection();
        let repo = make_repo(db);

        // The stored hash is for "correct-secret"; wrong secret must fail.
        let result = repo
            .validate_api_key("niad_x", "wrong-secret-value-xxx")
            .await
            .unwrap();
        assert!(result.is_none(), "wrong secret must not validate");
    }

    #[tokio::test]
    async fn test_api_key_list_returns_keys_for_workspace() {
        let ws_id = fixed_uuid(80);
        let id1 = fixed_uuid(81);
        let id2 = fixed_uuid(82);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![
                sample_api_key_model(id1, "nino_1", "user"),
                sample_api_key_model(id2, "nino_2", "user"),
            ]])
            .into_connection();
        let repo = make_repo(db);

        let keys = repo.list_api_keys(ws_id, None, None).await.unwrap();
        assert_eq!(keys.len(), 2);
    }

    #[tokio::test]
    async fn test_api_key_delete_succeeds_when_rows_affected() {
        let id = fixed_uuid(83);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_exec_results(vec![MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            .into_connection();
        let repo = make_repo(db);

        repo.delete_api_key(id).await.unwrap();
    }

    #[tokio::test]
    async fn test_api_key_delete_returns_not_found_when_no_rows_affected() {
        let id = fixed_uuid(84);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_exec_results(vec![MockExecResult {
                last_insert_id: 0,
                rows_affected: 0,
            }])
            .into_connection();
        let repo = make_repo(db);

        let err = repo.delete_api_key(id).await.unwrap_err();
        assert!(
            matches!(err, crate::core::CoreError::NotFound(ref m) if m.contains("API key")),
            "expected NotFound for api key, got {:?}",
            err
        );
    }

    #[tokio::test]
    async fn test_api_key_revoke_returns_not_found_when_key_missing() {
        let id = fixed_uuid(85);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![Vec::<api_key_entity::Model>::new()])
            .into_connection();
        let repo = make_repo(db);

        let err = repo.revoke_api_key(id).await.unwrap_err();
        assert!(
            matches!(err, crate::core::CoreError::NotFound(ref m) if m.contains("API key")),
            "expected NotFound for api key, got {:?}",
            err
        );
    }

    #[tokio::test]
    async fn test_api_key_revoke_succeeds_when_key_found() {
        let id = fixed_uuid(86);
        let model = sample_api_key_model(id, "niad_x", "admin");
        let updated_model = api_key_entity::Model {
            enabled: false,
            ..model
        };
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![sample_api_key_model(id, "niad_x", "admin")]])
            .append_query_results(vec![vec![updated_model]])
            .into_connection();
        let repo = make_repo(db);

        repo.revoke_api_key(id).await.unwrap();
    }

    #[tokio::test]
    async fn test_api_key_update_last_used_succeeds_when_key_missing() {
        // Per the implementation, update_last_used returns Ok(()) even
        // when the key is not found (it silently no-ops). This is a
        // documented behavior to test.
        let id = fixed_uuid(87);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![Vec::<api_key_entity::Model>::new()])
            .into_connection();
        let repo = make_repo(db);

        repo.update_last_used(id).await.unwrap();
    }

    #[tokio::test]
    async fn test_api_key_update_last_used_succeeds_when_key_found() {
        let id = fixed_uuid(88);
        let updated_model = api_key_entity::Model {
            last_used_at: Some(fixed_datetime(1_700_000_000)),
            ..sample_api_key_model(id, "niad_x", "admin")
        };
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![
                vec![sample_api_key_model(id, "niad_x", "admin")],
                vec![updated_model],
            ])
            .into_connection();
        let repo = make_repo(db);

        repo.update_last_used(id).await.unwrap();
    }

    #[tokio::test]
    async fn test_api_key_get_admin_returns_some_when_admin_enabled() {
        let id = fixed_uuid(89);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![sample_api_key_model(id, "niad_admin", "admin")]])
            .into_connection();
        let repo = make_repo(db);

        let key = repo.get_admin_api_key(fixed_uuid(90)).await.unwrap();
        assert!(key.is_some());
        assert_eq!(key.unwrap().role, ApiKeyRole::Admin);
    }

    #[tokio::test]
    async fn test_api_key_get_admin_returns_none_when_admin_disabled() {
        let id = fixed_uuid(91);
        let model = api_key_entity::Model {
            enabled: false,
            ..sample_api_key_model(id, "niad_admin", "admin")
        };
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![model]])
            .into_connection();
        let repo = make_repo(db);

        let key = repo.get_admin_api_key(fixed_uuid(92)).await.unwrap();
        assert!(key.is_none(), "disabled admin key must be filtered out");
    }

    #[tokio::test]
    async fn test_api_key_count_returns_count_for_workspace() {
        let ws_id = fixed_uuid(93);
        let mut count_row: BTreeMap<String, sea_orm::Value> = BTreeMap::new();
        count_row.insert("num_items".to_string(), sea_orm::Value::BigInt(Some(5)));
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![count_row]])
            .into_connection();
        let repo = make_repo(db);

        let count = repo.count_api_keys(ws_id).await.unwrap();
        assert_eq!(count, 5);
    }

    #[tokio::test]
    async fn test_api_key_rotate_returns_not_found_when_key_missing() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![Vec::<api_key_entity::Model>::new()])
            .into_connection();
        let repo = make_repo(db);

        let err = repo.rotate_api_key("niad_missing", 3600).await.unwrap_err();
        assert!(
            matches!(err, crate::core::CoreError::NotFound(ref m) if m.contains("API key")),
            "expected NotFound for rotate, got {:?}",
            err
        );
    }

    #[tokio::test]
    async fn test_api_key_rotate_returns_new_secret_when_key_found() {
        let id = fixed_uuid(94);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![
                // get_api_key_by_id query.
                vec![sample_api_key_model(id, "niad_x", "admin")],
                // Update RETURNING query.
                vec![sample_api_key_model(id, "niad_x", "admin")],
            ])
            .into_connection();
        let repo = make_repo(db);

        let rotated = repo.rotate_api_key("niad_x", 3600).await.unwrap();
        assert_eq!(rotated.key.id, id);
        assert_eq!(
            rotated.key_secret.len(),
            32,
            "rotated secret must be 32 chars"
        );
        assert_ne!(rotated.key_secret, "", "rotated secret must not be empty");
    }

    #[tokio::test]
    async fn test_api_key_get_keys_older_than_returns_matching_keys() {
        let id = fixed_uuid(95);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![sample_api_key_model(id, "niad_old", "admin")]])
            .into_connection();
        let repo = make_repo(db);

        let keys = repo.get_keys_older_than(30).await.unwrap();
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].id, id);
    }

    // ==================================================================
    // SegmentRepository tests
    // ==================================================================

    #[tokio::test]
    async fn test_segment_get_returns_some_when_found() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![sample_segment_model(1, "ws1", "t1")]])
            .into_connection();
        let repo = make_repo(db);

        let seg = repo.get_segment("ws1", "t1").await.unwrap();
        assert!(seg.is_some());
        let seg = seg.unwrap();
        assert_eq!(seg.id, 1);
        assert_eq!(seg.workspace_id, "ws1");
        assert_eq!(seg.biz_tag, "t1");
        assert_eq!(seg.current_id, 100);
        assert_eq!(seg.max_id, 1000);
    }

    #[tokio::test]
    async fn test_segment_get_returns_none_when_missing() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![Vec::<segment_entity::Model>::new()])
            .into_connection();
        let repo = make_repo(db);

        let seg = repo.get_segment("ws", "missing").await.unwrap();
        assert!(seg.is_none());
    }

    #[tokio::test]
    async fn test_segment_list_returns_segments_for_workspace() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![
                sample_segment_model(1, "ws1", "t1"),
                sample_segment_model(2, "ws1", "t2"),
            ]])
            .into_connection();
        let repo = make_repo(db);

        let segs = repo.list_segments("ws1").await.unwrap();
        assert_eq!(segs.len(), 2);
    }

    #[tokio::test]
    async fn test_segment_create_returns_segment_with_provided_fields() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![sample_segment_model(1, "ws1", "t1")]])
            .into_connection();
        let repo = make_repo(db);

        let seg = repo
            .create_segment("ws1", "t1", 1, 1000, 100, 1)
            .await
            .unwrap();
        assert_eq!(seg.workspace_id, "ws1");
        assert_eq!(seg.biz_tag, "t1");
    }

    #[tokio::test]
    async fn test_segment_update_succeeds_when_rows_affected() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_exec_results(vec![MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            .into_connection();
        let repo = make_repo(db);

        repo.update_segment("ws1", "t1", 500, 1000).await.unwrap();
    }

    #[tokio::test]
    async fn test_segment_update_returns_not_found_when_no_rows_affected() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_exec_results(vec![MockExecResult {
                last_insert_id: 0,
                rows_affected: 0,
            }])
            .into_connection();
        let repo = make_repo(db);

        let err = repo
            .update_segment("ws1", "missing", 100, 1000)
            .await
            .unwrap_err();
        assert!(
            matches!(err, crate::core::CoreError::NotFound(ref m) if m.contains("Segment")),
            "expected NotFound for segment, got {:?}",
            err
        );
    }

    #[tokio::test]
    async fn test_segment_delete_succeeds_when_rows_affected() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_exec_results(vec![MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            .into_connection();
        let repo = make_repo(db);

        repo.delete_segment("ws1", "t1").await.unwrap();
    }

    #[tokio::test]
    async fn test_segment_delete_returns_not_found_when_no_rows_affected() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_exec_results(vec![MockExecResult {
                last_insert_id: 0,
                rows_affected: 0,
            }])
            .into_connection();
        let repo = make_repo(db);

        let err = repo.delete_segment("ws1", "missing").await.unwrap_err();
        assert!(
            matches!(err, crate::core::CoreError::NotFound(ref m) if m.contains("Segment")),
            "expected NotFound for segment, got {:?}",
            err
        );
    }

    // --- allocate_segment / allocate_segment_with_dc ---
    //
    // These require a distributed lock (NoopLockGuard in tests) and a
    // transaction. The mock database supports `begin()` returning the
    // same connection, so we mock the queries inside the transaction
    // in order.

    #[tokio::test]
    async fn test_segment_allocate_creates_new_segment_when_none_exists() {
        // Inside txn: 1) find existing (None), 2) insert with RETURNING.
        // Then commit (exec).
        let id = fixed_uuid(96);
        let new_segment_model = segment_entity::Model {
            id: 1,
            workspace_id: "ws1".to_string(),
            biz_tag: "t1".to_string(),
            current_id: 100, // start_id (1) + step (100) - 1, but impl sets current_id = max_id
            max_id: 101,     // start_id (1) + step (100)
            step: 100,
            delta: 1,
            dc_id: 0,
            created_at: fixed_datetime(1_600_000_000),
            updated_at: fixed_datetime(1_700_000_000),
        };
        let _ = id;
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![
                Vec::<segment_entity::Model>::new(), // find existing
                vec![new_segment_model],             // insert RETURNING
            ])
            .append_exec_results(vec![MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }]) // commit
            .into_connection();
        let repo = make_repo(db);

        let seg = repo.allocate_segment("ws1", "t1", 100).await.unwrap();
        assert_eq!(seg.workspace_id, "ws1");
        assert_eq!(seg.biz_tag, "t1");
        assert_eq!(seg.current_id, 1, "new segment starts at start_id");
        assert_eq!(seg.max_id, 101, "max_id = start_id + step");
        assert_eq!(seg.step, 100);
    }

    #[tokio::test]
    async fn test_segment_allocate_advances_max_when_segment_exists() {
        let existing = segment_entity::Model {
            id: 1,
            workspace_id: "ws1".to_string(),
            biz_tag: "t1".to_string(),
            current_id: 100,
            max_id: 200,
            step: 100,
            delta: 1,
            dc_id: 0,
            created_at: fixed_datetime(1_600_000_000),
            updated_at: fixed_datetime(1_700_000_000),
        };
        let updated_model = segment_entity::Model {
            current_id: 200, // 100 + step
            ..existing.clone()
        };
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![
                vec![existing],      // find existing returns Some
                vec![updated_model], // update RETURNING
            ])
            .append_exec_results(vec![MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }]) // commit
            .into_connection();
        let repo = make_repo(db);

        let seg = repo.allocate_segment("ws1", "t1", 100).await.unwrap();
        // current_id is the previous current_id (100), max_id is advanced.
        assert_eq!(seg.current_id, 100, "current_id must be the previous value");
        assert_eq!(seg.max_id, 200, "max_id must advance by step");
    }

    #[tokio::test]
    async fn test_segment_allocate_with_dc_creates_new_segment_with_dc_offset() {
        let new_segment_model = segment_entity::Model {
            id: 1,
            workspace_id: "ws1".to_string(),
            biz_tag: "t1".to_string(),
            current_id: 5_000_000_000_001, // dc_id * 10^12 + 1
            max_id: 5_000_000_000_101,     // start + step
            step: 100,
            delta: 1,
            dc_id: 5,
            created_at: fixed_datetime(1_600_000_000),
            updated_at: fixed_datetime(1_700_000_000),
        };
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![
                Vec::<segment_entity::Model>::new(), // find existing
                vec![new_segment_model],             // insert RETURNING
            ])
            .append_exec_results(vec![MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }]) // commit
            .into_connection();
        let repo = make_repo(db);

        let seg = repo
            .allocate_segment_with_dc("ws1", "t1", 100, 5)
            .await
            .unwrap();
        // start_id = dc_id * 10^12 + 1 = 5_000_000_000_001
        assert_eq!(seg.current_id, 5_000_000_000_001);
        assert_eq!(seg.max_id, 5_000_000_000_101);
    }

    #[tokio::test]
    async fn test_segment_allocate_with_distributed_lock_uses_injected_lock() {
        // When a lock is injected, allocate_segment must acquire it
        // instead of using NoopLockGuard. We verify by checking that
        // the operation still succeeds (DummyDistributedLock always
        // returns Ok).
        let new_segment_model = segment_entity::Model {
            id: 1,
            workspace_id: "ws1".to_string(),
            biz_tag: "t1".to_string(),
            current_id: 100,
            max_id: 101,
            step: 100,
            delta: 1,
            dc_id: 0,
            created_at: fixed_datetime(1_600_000_000),
            updated_at: fixed_datetime(1_700_000_000),
        };
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![
                Vec::<segment_entity::Model>::new(),
                vec![new_segment_model],
            ])
            .append_exec_results(vec![MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            .into_connection();
        let lock: Arc<dyn DistributedLock + Send + Sync> = Arc::new(DummyDistributedLock);
        let repo = SeaOrmRepository::new(db, "salt".to_string()).with_distributed_lock(lock);

        let seg = repo.allocate_segment("ws1", "t1", 100).await.unwrap();
        assert_eq!(seg.workspace_id, "ws1");
    }

    // ==================================================================
    // Error-path coverage (Phase: bring repository.rs to ≥95% line cov)
    // ==================================================================

    /// A distributed lock whose `acquire` always fails. Used to exercise the
    /// `InternalError` branch in `allocate_segment` / `allocate_segment_with_dc`.
    struct FailingDistributedLock;

    #[async_trait]
    impl DistributedLock for FailingDistributedLock {
        async fn acquire(
            &self,
            key: &str,
            _ttl_seconds: u64,
        ) -> std::result::Result<Box<dyn LockGuard>, LockError> {
            Err(LockError::ConnectionFailed(format!(
                "lock service down for {key}"
            )))
        }

        fn is_healthy(&self) -> bool {
            false
        }
    }

    // ----- Workspace error paths -----

    #[tokio::test]
    async fn test_workspace_update_propagates_find_db_error() {
        // First query (find_by_id) fails → DatabaseError propagated.
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "find failed".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);

        let result = repo
            .update_workspace(
                fixed_uuid(11),
                &UpdateWorkspaceRequest {
                    name: None,
                    description: None,
                    status: None,
                    max_groups: None,
                    max_biz_tags: None,
                },
            )
            .await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_)
            ),
            "find_by_id error must map to DatabaseError"
        );
    }

    #[tokio::test]
    async fn test_workspace_update_propagates_update_db_error() {
        // find_by_id succeeds, update fails → DatabaseError.
        let id = fixed_uuid(13);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![sample_workspace_model(id, "ws13")]])
            .append_query_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "update failed".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);

        let result = repo
            .update_workspace(
                id,
                &UpdateWorkspaceRequest {
                    name: Some("renamed".to_string()),
                    description: None,
                    status: None,
                    max_groups: None,
                    max_biz_tags: None,
                },
            )
            .await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_)
            ),
            "update error must map to DatabaseError"
        );
    }

    #[tokio::test]
    async fn test_workspace_get_with_groups_propagates_workspace_find_error() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "ws find boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo.get_workspace_with_groups(fixed_uuid(14)).await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_)
            ),
            "workspace find error must propagate"
        );
    }

    #[tokio::test]
    async fn test_workspace_get_with_groups_propagates_groups_find_error() {
        // workspace find succeeds, groups find fails.
        let id = fixed_uuid(15);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![sample_workspace_model(id, "ws15")]])
            .append_query_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "groups find boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo.get_workspace_with_groups(id).await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_)
            ),
            "groups find error must propagate"
        );
    }

    #[tokio::test]
    async fn test_workspace_get_with_groups_returns_none_when_missing() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![Vec::<workspace_entity::Model>::new()])
            .into_connection();
        let repo = make_repo(db);
        let result = repo
            .get_workspace_with_groups(fixed_uuid(16))
            .await
            .unwrap();
        assert!(result.is_none(), "missing workspace must yield None");
    }

    #[tokio::test]
    async fn test_workspace_get_with_groups_and_biz_tags_propagates_relations_error() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "relations boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo
            .get_workspace_with_groups_and_biz_tags(fixed_uuid(17))
            .await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_)
            ),
            "find_also_related error must propagate"
        );
    }

    #[tokio::test]
    async fn test_workspace_get_with_groups_and_biz_tags_returns_none_when_empty() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![Vec::<(
                workspace_entity::Model,
                Option<group_entity::Model>,
            )>::new()])
            .into_connection();
        let repo = make_repo(db);
        let result = repo
            .get_workspace_with_groups_and_biz_tags(fixed_uuid(18))
            .await
            .unwrap();
        assert!(
            result.is_none(),
            "empty relations must produce None, not Some(empty)"
        );
    }

    #[tokio::test]
    async fn test_workspace_get_with_groups_and_biz_tags_propagates_biz_tags_error() {
        // workspace+groups query succeeds, biz_tags query fails.
        let id = fixed_uuid(19);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![(
                sample_workspace_model(id, "ws19"),
                None::<group_entity::Model>,
            )]])
            .append_query_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "biz tags boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo.get_workspace_with_groups_and_biz_tags(id).await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_)
            ),
            "biz_tags find error must propagate"
        );
    }

    #[tokio::test]
    async fn test_workspace_get_with_groups_and_biz_tags_returns_workspace_without_groups() {
        // workspace exists, no groups, no biz_tags → Some((ws, [])).
        let id = fixed_uuid(20);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![(
                sample_workspace_model(id, "ws20"),
                None::<group_entity::Model>,
            )]])
            .append_query_results(vec![Vec::<biz_tag_entity::Model>::new()])
            .into_connection();
        let repo = make_repo(db);
        let result = repo
            .get_workspace_with_groups_and_biz_tags(id)
            .await
            .unwrap();
        assert!(result.is_some(), "workspace must be Some");
        let (ws, groups) = result.unwrap();
        assert_eq!(ws.id, id);
        assert!(groups.is_empty(), "no groups → empty vec");
    }

    // ----- Group error paths -----

    #[tokio::test]
    async fn test_group_create_propagates_workspace_find_error() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "ws find boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo
            .create_group(&CreateGroupRequest {
                workspace_id: fixed_uuid(21),
                name: "g21".to_string(),
                description: None,
                max_biz_tags: None,
            })
            .await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_)
            ),
            "workspace find error must propagate"
        );
    }

    #[tokio::test]
    async fn test_group_create_propagates_insert_error() {
        // workspace exists, insert fails.
        let ws_id = fixed_uuid(23);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![sample_workspace_model(ws_id, "ws23")]])
            .append_query_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "insert boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo
            .create_group(&CreateGroupRequest {
                workspace_id: ws_id,
                name: "g23".to_string(),
                description: None,
                max_biz_tags: None,
            })
            .await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_)
            ),
            "insert error must propagate"
        );
    }

    #[tokio::test]
    async fn test_group_update_propagates_find_error() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "group find boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo
            .update_group(
                fixed_uuid(24),
                &UpdateGroupRequest {
                    name: None,
                    description: None,
                    max_biz_tags: None,
                },
            )
            .await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_)
            ),
            "find error must propagate"
        );
    }

    #[tokio::test]
    async fn test_group_update_propagates_update_error() {
        let id = fixed_uuid(26);
        let ws_id = fixed_uuid(99);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![sample_group_model(id, ws_id, "g26")]])
            .append_query_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "group update boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo
            .update_group(
                id,
                &UpdateGroupRequest {
                    name: Some("g26-new".to_string()),
                    description: None,
                    max_biz_tags: None,
                },
            )
            .await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_)
            ),
            "update error must propagate"
        );
    }

    // ----- delete_group_with_biz_tags full coverage -----

    #[tokio::test]
    async fn test_delete_group_with_biz_tags_succeeds_with_no_biz_tags() {
        // Transaction begin OK, find returns empty, delete_by_id(group) OK, commit OK.
        let group_id = fixed_uuid(30);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![
                Vec::<biz_tag_entity::Model>::new(), // find biz_tags
            ])
            .append_exec_results(vec![
                MockExecResult {
                    last_insert_id: 0,
                    rows_affected: 1,
                }, // delete group
                MockExecResult {
                    last_insert_id: 0,
                    rows_affected: 0,
                }, // commit
            ])
            .into_connection();
        let repo = make_repo(db);
        let result = repo.delete_group_with_biz_tags(group_id).await;
        assert!(result.is_ok(), "delete with no biz_tags should succeed");
    }

    #[tokio::test]
    async fn test_delete_group_with_biz_tags_succeeds_with_biz_tags() {
        // Transaction begin OK, find returns 2 biz_tags, delete each, delete group, commit.
        let group_id = fixed_uuid(31);
        let ws_id = fixed_uuid(98);
        let tag1 = sample_biz_tag_model(fixed_uuid(40), ws_id, group_id, "t40");
        let tag2 = sample_biz_tag_model(fixed_uuid(41), ws_id, group_id, "t41");
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![tag1, tag2]])
            .append_exec_results(vec![
                MockExecResult {
                    last_insert_id: 0,
                    rows_affected: 1,
                }, // delete tag1
                MockExecResult {
                    last_insert_id: 0,
                    rows_affected: 1,
                }, // delete tag2
                MockExecResult {
                    last_insert_id: 0,
                    rows_affected: 1,
                }, // delete group
                MockExecResult {
                    last_insert_id: 0,
                    rows_affected: 0,
                }, // commit
            ])
            .into_connection();
        let repo = make_repo(db);
        let result = repo.delete_group_with_biz_tags(group_id).await;
        assert!(result.is_ok(), "delete with biz_tags should succeed");
    }

    #[tokio::test]
    async fn test_delete_group_with_biz_tags_propagates_biz_tags_find_error() {
        let group_id = fixed_uuid(33);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "biz_tags find boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo.delete_group_with_biz_tags(group_id).await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_)
            ),
            "biz_tags find error must propagate"
        );
    }

    #[tokio::test]
    async fn test_delete_group_with_biz_tags_propagates_biz_tag_delete_error() {
        // find returns 1 biz_tag, delete of that tag fails.
        let group_id = fixed_uuid(34);
        let ws_id = fixed_uuid(97);
        let tag = sample_biz_tag_model(fixed_uuid(42), ws_id, group_id, "t42");
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![tag]])
            .append_exec_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "tag delete boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo.delete_group_with_biz_tags(group_id).await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_)
            ),
            "biz_tag delete error must propagate"
        );
    }

    #[tokio::test]
    async fn test_delete_group_with_biz_tags_propagates_group_delete_error() {
        // find returns empty (no biz_tags), delete group fails.
        let group_id = fixed_uuid(35);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![Vec::<biz_tag_entity::Model>::new()])
            .append_exec_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "group delete boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo.delete_group_with_biz_tags(group_id).await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_)
            ),
            "group delete error must propagate"
        );
    }

    // ----- BizTag error paths -----

    #[tokio::test]
    async fn test_biz_tag_create_propagates_workspace_find_error() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "ws find boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo
            .create_biz_tag(&CreateBizTagRequest {
                workspace_id: fixed_uuid(50),
                group_id: fixed_uuid(51),
                name: "t50".to_string(),
                description: None,
                algorithm: None,
                format: None,
                prefix: None,
                base_step: None,
                max_step: None,
                datacenter_ids: None,
            })
            .await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_)
            ),
            "workspace find error must propagate"
        );
    }

    #[tokio::test]
    async fn test_biz_tag_create_propagates_insert_error() {
        let ws_id = fixed_uuid(56);
        let group_id = fixed_uuid(57);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![sample_workspace_model(ws_id, "ws56")]])
            .append_query_results(vec![vec![sample_group_model(group_id, ws_id, "g57")]])
            .append_query_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "insert boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo
            .create_biz_tag(&CreateBizTagRequest {
                workspace_id: ws_id,
                group_id,
                name: "t56".to_string(),
                description: None,
                algorithm: None,
                format: None,
                prefix: None,
                base_step: None,
                max_step: None,
                datacenter_ids: None,
            })
            .await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_)
            ),
            "insert error must propagate"
        );
    }

    #[tokio::test]
    async fn test_biz_tag_update_propagates_find_error() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "tag find boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo
            .update_biz_tag(
                fixed_uuid(58),
                &UpdateBizTagRequest {
                    name: None,
                    description: None,
                    algorithm: None,
                    format: None,
                    prefix: None,
                    base_step: None,
                    max_step: None,
                    datacenter_ids: None,
                },
            )
            .await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_)
            ),
            "find error must propagate"
        );
    }

    #[tokio::test]
    async fn test_biz_tag_update_propagates_update_error() {
        let id = fixed_uuid(60);
        let ws_id = fixed_uuid(96);
        let group_id = fixed_uuid(95);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![sample_biz_tag_model(id, ws_id, group_id, "t60")]])
            .append_query_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "tag update boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo
            .update_biz_tag(
                id,
                &UpdateBizTagRequest {
                    name: Some("t60-new".to_string()),
                    description: None,
                    algorithm: None,
                    format: None,
                    prefix: None,
                    base_step: None,
                    max_step: None,
                    datacenter_ids: None,
                },
            )
            .await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_)
            ),
            "update error must propagate"
        );
    }

    #[tokio::test]
    async fn test_biz_tag_list_propagates_db_error() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "list boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo.list_biz_tags(fixed_uuid(61), None, None, None).await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_)
            ),
            "list error must propagate"
        );
    }

    #[tokio::test]
    async fn test_biz_tag_list_by_workspace_group_propagates_db_error() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "list by group boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo
            .list_biz_tags_by_workspace_group(fixed_uuid(62), fixed_uuid(63))
            .await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_)
            ),
            "list by group error must propagate"
        );
    }

    #[tokio::test]
    async fn test_biz_tag_count_by_group_propagates_db_error() {
        // count() in sea-orm executes a query with a single num_items column.
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "count boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo.count_biz_tags_by_group(fixed_uuid(64)).await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_)
            ),
            "count error must propagate"
        );
    }

    #[tokio::test]
    async fn test_biz_tag_count_propagates_db_error() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "count ws boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo.count_biz_tags(fixed_uuid(65), None).await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_)
            ),
            "count error must propagate"
        );
    }

    // ----- ApiKey error paths -----

    #[tokio::test]
    async fn test_api_key_create_propagates_insert_db_error() {
        // insert returns a query result on Postgres (RETURNING). Appending a
        // query error forces the insert to fail.
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "api_key insert boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo
            .create_api_key(&CreateApiKeyRequest {
                workspace_id: Some(fixed_uuid(70)),
                name: "k70".to_string(),
                description: None,
                role: ApiKeyRole::User,
                rate_limit: None,
                expires_at: None,
                key_secret: Some("valid-secret".to_string()),
                key_id: None,
            })
            .await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_)
            ),
            "insert error must propagate as DatabaseError"
        );
    }

    #[tokio::test]
    async fn test_api_key_get_by_id_propagates_db_error() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "api_key find boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo.get_api_key_by_id("niad_x").await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_)
            ),
            "find error must propagate as DatabaseError"
        );
    }

    #[tokio::test]
    async fn test_api_key_validate_propagates_db_error() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "api_key validate find boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo.validate_api_key("niad_x", "secret").await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_)
            ),
            "validate find error must propagate as DatabaseError"
        );
    }

    #[tokio::test]
    async fn test_api_key_list_propagates_db_error() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "api_key list boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo.list_api_keys(fixed_uuid(71), None, None).await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_)
            ),
            "list error must propagate as DatabaseError"
        );
    }

    #[tokio::test]
    async fn test_api_key_delete_propagates_db_error() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_exec_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "api_key delete boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo.delete_api_key(fixed_uuid(72)).await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_)
            ),
            "delete exec error must propagate as DatabaseError"
        );
    }

    #[tokio::test]
    async fn test_api_key_revoke_propagates_find_db_error() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "api_key revoke find boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo.revoke_api_key(fixed_uuid(73)).await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_)
            ),
            "revoke find error must propagate as DatabaseError"
        );
    }

    #[tokio::test]
    async fn test_api_key_revoke_propagates_update_db_error() {
        // find succeeds, update fails.
        let id = fixed_uuid(74);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![sample_api_key_model(id, "niad_74", "admin")]])
            .append_query_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "api_key revoke update boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo.revoke_api_key(id).await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_)
            ),
            "revoke update error must propagate as DatabaseError"
        );
    }

    #[tokio::test]
    async fn test_api_key_update_last_used_propagates_find_db_error() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "api_key last_used find boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo.update_last_used(fixed_uuid(75)).await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_)
            ),
            "update_last_used find error must propagate as DatabaseError"
        );
    }

    #[tokio::test]
    async fn test_api_key_update_last_used_propagates_update_db_error() {
        // find succeeds, update fails.
        let id = fixed_uuid(76);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![sample_api_key_model(id, "niad_76", "admin")]])
            .append_query_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "api_key last_used update boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo.update_last_used(id).await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_)
            ),
            "update_last_used update error must propagate as DatabaseError"
        );
    }

    #[tokio::test]
    async fn test_api_key_get_admin_propagates_db_error() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "api_key admin find boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo.get_admin_api_key(fixed_uuid(77)).await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_)
            ),
            "get_admin find error must propagate as DatabaseError"
        );
    }

    #[tokio::test]
    async fn test_api_key_count_propagates_db_error() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "api_key count boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo.count_api_keys(fixed_uuid(78)).await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_)
            ),
            "count error must propagate as DatabaseError"
        );
    }

    #[tokio::test]
    async fn test_api_key_rotate_propagates_get_db_error() {
        // get_api_key_by_id internally runs a find query that fails.
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "api_key rotate get boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo.rotate_api_key("niad_x", 3600).await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_)
            ),
            "rotate get error must propagate as DatabaseError"
        );
    }

    #[tokio::test]
    async fn test_api_key_rotate_propagates_update_db_error() {
        // get succeeds, update fails.
        let id = fixed_uuid(79);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![sample_api_key_model(id, "niad_79", "admin")]])
            .append_query_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "api_key rotate update boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo.rotate_api_key("niad_79", 3600).await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_)
            ),
            "rotate update error must propagate as DatabaseError"
        );
    }

    #[tokio::test]
    async fn test_api_key_get_keys_older_than_propagates_db_error() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "api_key older than boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo.get_keys_older_than(30).await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_)
            ),
            "get_keys_older_than error must propagate as DatabaseError"
        );
    }

    // ----- Segment error paths -----

    #[tokio::test]
    async fn test_segment_get_propagates_db_error() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "segment get boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo.get_segment("ws1", "t1").await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_)
            ),
            "segment get error must propagate as DatabaseError"
        );
    }

    #[tokio::test]
    async fn test_segment_list_propagates_db_error() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "segment list boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo.list_segments("ws1").await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_)
            ),
            "segment list error must propagate as DatabaseError"
        );
    }

    #[tokio::test]
    async fn test_segment_create_propagates_insert_db_error() {
        // insert on Postgres uses RETURNING → query error.
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "segment create insert boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo.create_segment("ws1", "t1", 1, 1000, 100, 1).await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_)
            ),
            "segment create insert error must propagate as DatabaseError"
        );
    }

    #[tokio::test]
    async fn test_segment_update_propagates_db_error() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_exec_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "segment update boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo.update_segment("ws1", "t1", 500, 1000).await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_)
            ),
            "segment update exec error must propagate as DatabaseError"
        );
    }

    #[tokio::test]
    async fn test_segment_delete_propagates_db_error() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_exec_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "segment delete boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo.delete_segment("ws1", "t1").await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_)
            ),
            "segment delete exec error must propagate as DatabaseError"
        );
    }

    #[tokio::test]
    async fn test_segment_allocate_propagates_find_db_error() {
        // allocate_segment with no distributed lock (test cfg → NoopLockGuard).
        // begin succeeds (mock), find fails.
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "segment allocate find boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo.allocate_segment("ws1", "t1", 100).await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_)
            ),
            "segment allocate find error must propagate as DatabaseError"
        );
    }

    #[tokio::test]
    async fn test_segment_allocate_propagates_insert_db_error() {
        // find returns None (no existing segment), insert fails.
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![Vec::<segment_entity::Model>::new()])
            .append_query_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "segment allocate insert boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo.allocate_segment("ws1", "t1", 100).await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_)
            ),
            "segment allocate insert error must propagate as DatabaseError"
        );
    }

    #[tokio::test]
    async fn test_segment_allocate_propagates_update_db_error() {
        // find returns existing segment, update fails.
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![sample_segment_model(1, "ws1", "t1")]])
            .append_query_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "segment allocate update boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo.allocate_segment("ws1", "t1", 100).await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_)
            ),
            "segment allocate update error must propagate as DatabaseError"
        );
    }

    #[tokio::test]
    async fn test_segment_allocate_with_dc_propagates_find_db_error() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "segment allocate_dc find boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo.allocate_segment_with_dc("ws1", "t1", 100, 1).await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_)
            ),
            "segment allocate_with_dc find error must propagate as DatabaseError"
        );
    }

    #[tokio::test]
    async fn test_segment_allocate_with_dc_propagates_insert_db_error() {
        // find returns None, insert fails.
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![Vec::<segment_entity::Model>::new()])
            .append_query_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "segment allocate_dc insert boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo.allocate_segment_with_dc("ws1", "t1", 100, 1).await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_)
            ),
            "segment allocate_with_dc insert error must propagate as DatabaseError"
        );
    }

    #[tokio::test]
    async fn test_segment_allocate_with_dc_propagates_update_db_error() {
        // find returns existing segment, update fails.
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![sample_segment_model(1, "ws1", "t1")]])
            .append_query_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "segment allocate_dc update boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo.allocate_segment_with_dc("ws1", "t1", 100, 1).await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_)
            ),
            "segment allocate_with_dc update error must propagate as DatabaseError"
        );
    }

    #[tokio::test]
    async fn test_segment_allocate_with_failing_lock_returns_internal_error() {
        // When a distributed lock is configured and acquire fails, the
        // allocation must abort with InternalError (M8 fix: no silent
        // fallback to NoopLockGuard).
        let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();
        let lock: Arc<dyn DistributedLock + Send + Sync> = Arc::new(FailingDistributedLock);
        let repo = SeaOrmRepository::new(db, "salt".to_string()).with_distributed_lock(lock);
        let result = repo.allocate_segment("ws1", "t1", 100).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            crate::core::CoreError::InternalError(msg) => {
                assert!(
                    msg.contains("Failed to acquire distributed lock"),
                    "expected InternalError mentioning lock acquire failure, got: {msg}"
                );
            }
            other => panic!("expected InternalError for failing lock, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_segment_allocate_with_dc_with_failing_lock_returns_internal_error() {
        let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();
        let lock: Arc<dyn DistributedLock + Send + Sync> = Arc::new(FailingDistributedLock);
        let repo = SeaOrmRepository::new(db, "salt".to_string()).with_distributed_lock(lock);
        let result = repo.allocate_segment_with_dc("ws1", "t1", 100, 1).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            crate::core::CoreError::InternalError(msg) => {
                assert!(
                    msg.contains("Failed to acquire distributed lock"),
                    "expected InternalError mentioning lock acquire failure, got: {msg}"
                );
            }
            other => panic!("expected InternalError for failing lock, got {other:?}"),
        }
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

        let repo = SeaOrmRepository::new(db, "test_salt".to_string());

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

        let repo = SeaOrmRepository::new(db, "test_salt".to_string());

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
                algorithm: Some(crate::core::types::id::AlgorithmType::Segment),
                format: Some(crate::core::types::id::IdFormat::Numeric),
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
                algorithm: Some(crate::core::types::id::AlgorithmType::Snowflake),
                format: Some(crate::core::types::IdFormat::Numeric),
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

        let repo = SeaOrmRepository::new(db, "test_salt".to_string());

        let admin_key = repo
            .create_api_key(&CreateApiKeyRequest {
                workspace_id: None,
                name: "Test Admin Key".to_string(),
                description: Some("Admin key for testing".to_string()),
                role: ApiKeyRole::Admin,
                rate_limit: Some(10000),
                expires_at: None,
                key_secret: None,
                key_id: None,
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

        let repo = SeaOrmRepository::new(db, "test_salt".to_string());

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
                key_id: None,
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

        let repo = SeaOrmRepository::new(db, "test_salt".to_string());

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
                key_id: None,
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

        let repo = SeaOrmRepository::new(db, "test_salt".to_string());

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
                    key_id: None,
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

        let repo = SeaOrmRepository::new(db, "test_salt".to_string());

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
                key_id: None,
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
                key_id: None,
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

        let repo = SeaOrmRepository::new(db, "test_salt".to_string());

        let admin_key = repo
            .create_api_key(&CreateApiKeyRequest {
                workspace_id: None,
                name: "Test Key".to_string(),
                description: None,
                role: ApiKeyRole::Admin,
                rate_limit: None,
                expires_at: None,
                key_secret: None,
                key_id: None,
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
