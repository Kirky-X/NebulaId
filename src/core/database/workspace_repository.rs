// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! 工作区与工作区组仓储:`WorkspaceRepository` / `GroupRepository` 两个 trait
//! 及其 SeaORM 实现（含关联 biz_tags 的级联读取与级联删除）。

use async_trait::async_trait;
use dbnexus::sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait, QueryFilter,
    QuerySelect, Set, TransactionTrait,
};
use uuid::Uuid;

use crate::core::database::biz_tag_entity::{
    BizTag, Column as BizTagColumn, Entity as BizTagEntity,
};
use crate::core::database::group_entity::{
    ActiveModel as GroupActiveModel, Column as GroupColumn, CreateGroupRequest,
    Entity as GroupEntity, Group, UpdateGroupRequest,
};
use crate::core::database::repository::SeaOrmRepository;
use crate::core::database::workspace_entity::{
    ActiveModel as WorkspaceActiveModel, Column as WorkspaceColumn, CreateWorkspaceRequest,
    Entity as WorkspaceEntity, UpdateWorkspaceRequest, Workspace,
};
use crate::core::types::Result;

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

        let inserted = new_workspace.insert(&self.db).await.map_err(|e| {
            crate::core::CoreError::DatabaseError(
                e.to_string(),
                Some(crate::core::types::ErrorSource::new(e)),
            )
        })?;

        Ok(inserted.into())
    }

    async fn get_workspace(&self, id: Uuid) -> Result<Option<Workspace>> {
        let result = WorkspaceEntity::find_by_id(id)
            .one(&self.db)
            .await
            .map_err(|e| {
                crate::core::CoreError::DatabaseError(
                    e.to_string(),
                    Some(crate::core::types::ErrorSource::new(e)),
                )
            })?;

        Ok(result.map(|m| m.into()))
    }

    async fn get_workspace_by_name(&self, name: &str) -> Result<Option<Workspace>> {
        let result = WorkspaceEntity::find()
            .filter(WorkspaceColumn::Name.eq(name))
            .one(&self.db)
            .await
            .map_err(|e| {
                crate::core::CoreError::DatabaseError(
                    e.to_string(),
                    Some(crate::core::types::ErrorSource::new(e)),
                )
            })?;

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
            .map_err(|e| {
                crate::core::CoreError::DatabaseError(
                    e.to_string(),
                    Some(crate::core::types::ErrorSource::new(e)),
                )
            })?;

        // 使用ok_or_else替代is_none+unwrap模式，避免冗余和潜在panic风险
        let existing = existing.ok_or_else(|| {
            crate::core::CoreError::NotFound(t!("error.detail.workspace_not_found", id = id))
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

        let result = updated.update(&self.db).await.map_err(|e| {
            crate::core::CoreError::DatabaseError(
                e.to_string(),
                Some(crate::core::types::ErrorSource::new(e)),
            )
        })?;

        Ok(result.into())
    }

    async fn delete_workspace(&self, id: Uuid) -> Result<()> {
        let result = WorkspaceEntity::delete_by_id(id)
            .exec(&self.db)
            .await
            .map_err(|e| {
                crate::core::CoreError::DatabaseError(
                    e.to_string(),
                    Some(crate::core::types::ErrorSource::new(e)),
                )
            })?;

        if result.rows_affected == 0 {
            return Err(crate::core::CoreError::NotFound(t!(
                "error.detail.workspace_not_found",
                id = id
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

        let results = query.all(&self.db).await.map_err(|e| {
            crate::core::CoreError::DatabaseError(
                e.to_string(),
                Some(crate::core::types::ErrorSource::new(e)),
            )
        })?;

        Ok(results.into_iter().map(|m| m.into()).collect())
    }

    async fn get_workspace_with_groups(&self, id: Uuid) -> Result<Option<(Workspace, Vec<Group>)>> {
        let workspace_entity = WorkspaceEntity::find_by_id(id)
            .one(&self.db)
            .await
            .map_err(|e| {
                crate::core::CoreError::DatabaseError(
                    e.to_string(),
                    Some(crate::core::types::ErrorSource::new(e)),
                )
            })?;

        // 使用if let Some模式，避免冗余unwrap
        if let Some(ws) = workspace_entity {
            let workspace: Workspace = ws.into();

            let groups = GroupEntity::find()
                .filter(GroupColumn::WorkspaceId.eq(id))
                .all(&self.db)
                .await
                .map_err(|e| {
                    crate::core::CoreError::DatabaseError(
                        e.to_string(),
                        Some(crate::core::types::ErrorSource::new(e)),
                    )
                })?;

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
            .map_err(|e| {
                crate::core::CoreError::DatabaseError(
                    e.to_string(),
                    Some(crate::core::types::ErrorSource::new(e)),
                )
            })?;

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
                .map_err(|e| {
                    crate::core::CoreError::DatabaseError(
                        e.to_string(),
                        Some(crate::core::types::ErrorSource::new(e)),
                    )
                })?;

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
            .map_err(|e| {
                crate::core::CoreError::DatabaseError(
                    e.to_string(),
                    Some(crate::core::types::ErrorSource::new(e)),
                )
            })?
            .is_some();

        if !workspace_exists {
            return Err(crate::core::CoreError::NotFound(t!(
                "error.detail.workspace_not_found",
                id = group.workspace_id
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

        let inserted = new_group.insert(&self.db).await.map_err(|e| {
            crate::core::CoreError::DatabaseError(
                e.to_string(),
                Some(crate::core::types::ErrorSource::new(e)),
            )
        })?;

        Ok(inserted.into())
    }

    async fn get_group(&self, id: Uuid) -> Result<Option<Group>> {
        let result = GroupEntity::find_by_id(id)
            .one(&self.db)
            .await
            .map_err(|e| {
                crate::core::CoreError::DatabaseError(
                    e.to_string(),
                    Some(crate::core::types::ErrorSource::new(e)),
                )
            })?;

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
            .map_err(|e| {
                crate::core::CoreError::DatabaseError(
                    e.to_string(),
                    Some(crate::core::types::ErrorSource::new(e)),
                )
            })?;

        Ok(result.map(|m| m.into()))
    }

    async fn update_group(&self, id: Uuid, group: &UpdateGroupRequest) -> Result<Group> {
        let existing = GroupEntity::find_by_id(id)
            .one(&self.db)
            .await
            .map_err(|e| {
                crate::core::CoreError::DatabaseError(
                    e.to_string(),
                    Some(crate::core::types::ErrorSource::new(e)),
                )
            })?;

        // 使用ok_or_else替代is_none+unwrap模式
        let existing = existing.ok_or_else(|| {
            crate::core::CoreError::NotFound(t!("error.detail.group_not_found", id = id))
        })?;

        let updated = GroupActiveModel {
            id: Set(existing.id),
            name: Set(group.name.clone().unwrap_or(existing.name)),
            description: Set(group.description.clone().or(existing.description)),
            max_biz_tags: Set(group.max_biz_tags.unwrap_or(existing.max_biz_tags)),
            updated_at: Set(chrono::Utc::now().naive_utc()),
            ..Default::default()
        };

        let result = updated.update(&self.db).await.map_err(|e| {
            crate::core::CoreError::DatabaseError(
                e.to_string(),
                Some(crate::core::types::ErrorSource::new(e)),
            )
        })?;

        Ok(result.into())
    }

    async fn delete_group(&self, id: Uuid) -> Result<()> {
        let result = GroupEntity::delete_by_id(id)
            .exec(&self.db)
            .await
            .map_err(|e| {
                crate::core::CoreError::DatabaseError(
                    e.to_string(),
                    Some(crate::core::types::ErrorSource::new(e)),
                )
            })?;

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

        let results = query.all(&self.db).await.map_err(|e| {
            crate::core::CoreError::DatabaseError(
                e.to_string(),
                Some(crate::core::types::ErrorSource::new(e)),
            )
        })?;

        Ok(results.into_iter().map(|m| m.into()).collect())
    }

    async fn get_group_with_biz_tags(&self, id: Uuid) -> Result<Option<(Group, Vec<BizTag>)>> {
        let group_entity = GroupEntity::find_by_id(id)
            .one(&self.db)
            .await
            .map_err(|e| {
                crate::core::CoreError::DatabaseError(
                    e.to_string(),
                    Some(crate::core::types::ErrorSource::new(e)),
                )
            })?;

        // 使用if let Some模式，避免冗余unwrap
        if let Some(g) = group_entity {
            let group: Group = g.into();

            let biz_tags = BizTagEntity::find()
                .filter(BizTagColumn::GroupId.eq(id))
                .all(&self.db)
                .await
                .map_err(|e| {
                    crate::core::CoreError::DatabaseError(
                        e.to_string(),
                        Some(crate::core::types::ErrorSource::new(e)),
                    )
                })?;

            let biz_tags: Vec<BizTag> = biz_tags.into_iter().map(|b| b.into()).collect();

            Ok(Some((group, biz_tags)))
        } else {
            Ok(None)
        }
    }

    async fn delete_group_with_biz_tags(&self, id: Uuid) -> Result<()> {
        let txn = self.db.begin().await.map_err(|e| {
            crate::core::CoreError::DatabaseError(
                e.to_string(),
                Some(crate::core::types::ErrorSource::new(e)),
            )
        })?;

        let biz_tags = BizTagEntity::find()
            .filter(BizTagColumn::GroupId.eq(id))
            .all(&txn)
            .await
            .map_err(|e| {
                crate::core::CoreError::DatabaseError(
                    e.to_string(),
                    Some(crate::core::types::ErrorSource::new(e)),
                )
            })?;

        for biz_tag in biz_tags {
            BizTagEntity::delete_by_id(biz_tag.id)
                .exec(&txn)
                .await
                .map_err(|e| {
                    crate::core::CoreError::DatabaseError(
                        e.to_string(),
                        Some(crate::core::types::ErrorSource::new(e)),
                    )
                })?;
        }

        GroupEntity::delete_by_id(id)
            .exec(&txn)
            .await
            .map_err(|e| {
                crate::core::CoreError::DatabaseError(
                    e.to_string(),
                    Some(crate::core::types::ErrorSource::new(e)),
                )
            })?;

        txn.commit().await.map_err(|e| {
            crate::core::CoreError::DatabaseError(
                e.to_string(),
                Some(crate::core::types::ErrorSource::new(e)),
            )
        })?;

        Ok(())
    }
}

#[allow(unused_imports)]
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
    use crate::core::database::testing::*;
    use crate::core::database::{
        api_key_entity, biz_tag_entity, group_entity, segment_entity, workspace_entity,
    };
    use crate::core::types::id::{AlgorithmType, IdFormat};
    use chrono::NaiveDateTime;
    use dbnexus::sea_orm::{
        DatabaseBackend, DbErr, IntoMockRow, MockDatabase, MockExecResult, MockRow, QueryTrait,
        RuntimeErr,
    };
    use std::collections::BTreeMap;
    use std::sync::Arc;

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
            matches!(err, crate::core::CoreError::DatabaseError(_, _)),
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
                crate::core::CoreError::DatabaseError(_, _)
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
                crate::core::CoreError::DatabaseError(_, _)
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
                crate::core::CoreError::DatabaseError(_, _)
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
                crate::core::CoreError::DatabaseError(_, _)
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
                crate::core::CoreError::DatabaseError(_, _)
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
                crate::core::CoreError::DatabaseError(_, _)
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
                crate::core::CoreError::DatabaseError(_, _)
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
                crate::core::CoreError::DatabaseError(_, _)
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
                crate::core::CoreError::DatabaseError(_, _)
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
                crate::core::CoreError::DatabaseError(_, _)
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
                crate::core::CoreError::DatabaseError(_, _)
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
                crate::core::CoreError::DatabaseError(_, _)
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
                crate::core::CoreError::DatabaseError(_, _)
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
                crate::core::CoreError::DatabaseError(_, _)
            ),
            "group delete error must propagate"
        );
    }

    // ----- Workspace error paths (additional) -----

    #[tokio::test]
    async fn test_workspace_get_by_name_propagates_db_error() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "ws by name boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo.get_workspace_by_name("any").await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_, _)
            ),
            "find error must propagate as DatabaseError"
        );
    }

    #[tokio::test]
    async fn test_workspace_delete_propagates_exec_db_error() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_exec_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "ws delete boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo.delete_workspace(fixed_uuid(110)).await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_, _)
            ),
            "delete exec error must propagate as DatabaseError"
        );
    }

    #[tokio::test]
    async fn test_workspace_list_propagates_db_error() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "ws list boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo.list_workspaces(None, None).await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_, _)
            ),
            "list find error must propagate as DatabaseError"
        );
    }

    #[tokio::test]
    async fn test_workspace_update_applies_status_when_provided() {
        // Cover the `.map(|s| s.into())` closure on the status field
        // (only invoked when status is Some(...)). The existing
        // update_workspace tests all set status = None, leaving the
        // closure body uncovered.
        let id = fixed_uuid(111);
        let updated_model = workspace_entity::Model {
            status: "active".to_string(),
            ..sample_workspace_model(id, "ws_status")
        };
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![
                vec![sample_workspace_model(id, "ws_status")],
                vec![updated_model],
            ])
            .into_connection();
        let repo = make_repo(db);
        let updated = repo
            .update_workspace(
                id,
                &UpdateWorkspaceRequest {
                    name: None,
                    description: None,
                    status: Some(crate::core::database::workspace_entity::WorkspaceStatus::Active),
                    max_groups: None,
                    max_biz_tags: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(updated.id, id);
    }

    // ----- Group error paths (additional) -----

    #[tokio::test]
    async fn test_group_get_propagates_db_error() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "group get boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo.get_group(fixed_uuid(120)).await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_, _)
            ),
            "find error must propagate as DatabaseError"
        );
    }

    #[tokio::test]
    async fn test_group_get_by_workspace_and_name_propagates_db_error() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "group by ws+name boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo
            .get_group_by_workspace_and_name(fixed_uuid(121), "any")
            .await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_, _)
            ),
            "find error must propagate as DatabaseError"
        );
    }

    #[tokio::test]
    async fn test_group_delete_propagates_exec_db_error() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_exec_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "group delete boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo.delete_group(fixed_uuid(122)).await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_, _)
            ),
            "delete exec error must propagate as DatabaseError"
        );
    }

    #[tokio::test]
    async fn test_group_list_propagates_db_error() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "group list boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo.list_groups(fixed_uuid(123), None, None).await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_, _)
            ),
            "list find error must propagate as DatabaseError"
        );
    }

    #[tokio::test]
    async fn test_group_with_biz_tags_propagates_group_find_error() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "group with tags find boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo.get_group_with_biz_tags(fixed_uuid(124)).await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_, _)
            ),
            "group find error in get_group_with_biz_tags must propagate"
        );
    }

    #[tokio::test]
    async fn test_group_with_biz_tags_propagates_biz_tags_find_error() {
        // group find succeeds, biz_tags find fails.
        let g_id = fixed_uuid(125);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![sample_group_model(g_id, fixed_uuid(126), "g")]])
            .append_query_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "biz tags find boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo.get_group_with_biz_tags(g_id).await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_, _)
            ),
            "biz_tags find error must propagate"
        );
    }
}
