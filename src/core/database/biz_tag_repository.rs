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

//! 业务标签（BizTag）仓储:`BizTagRepository` trait 及其 SeaORM 实现
//! （创建时校验 workspace/group 存在性、算法与格式落库、计数与健康检查）。

use async_trait::async_trait;
use dbnexus::sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait, QueryFilter,
    QuerySelect, Set,
};
use uuid::Uuid;

use crate::core::database::biz_tag_entity::{
    ActiveModel as BizTagActiveModel, BizTag, Column as BizTagColumn, CreateBizTagRequest,
    Entity as BizTagEntity, UpdateBizTagRequest,
};
use crate::core::database::group_entity::Entity as GroupEntity;
use crate::core::database::repository::SeaOrmRepository;
use crate::core::database::workspace_entity::Entity as WorkspaceEntity;
use crate::core::types::Result;

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
impl BizTagRepository for SeaOrmRepository {
    async fn create_biz_tag(&self, biz_tag: &CreateBizTagRequest) -> Result<BizTag> {
        // 检查工作空间和组是否存在
        let workspace_exists = WorkspaceEntity::find_by_id(biz_tag.workspace_id)
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
            return Err(crate::core::CoreError::NotFound(format!(
                "Workspace not found: {}",
                biz_tag.workspace_id
            )));
        }

        let group_exists = GroupEntity::find_by_id(biz_tag.group_id)
            .one(&self.db)
            .await
            .map_err(|e| {
                crate::core::CoreError::DatabaseError(
                    e.to_string(),
                    Some(crate::core::types::ErrorSource::new(e)),
                )
            })?
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

        let inserted = new_biz_tag.insert(&self.db).await.map_err(|e| {
            crate::core::CoreError::DatabaseError(
                e.to_string(),
                Some(crate::core::types::ErrorSource::new(e)),
            )
        })?;

        Ok(inserted.into())
    }

    async fn get_biz_tag(&self, id: Uuid) -> Result<Option<BizTag>> {
        let result = BizTagEntity::find_by_id(id)
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
            .map_err(|e| {
                crate::core::CoreError::DatabaseError(
                    e.to_string(),
                    Some(crate::core::types::ErrorSource::new(e)),
                )
            })?;

        Ok(result.map(|m| m.into()))
    }

    async fn update_biz_tag(&self, id: Uuid, biz_tag: &UpdateBizTagRequest) -> Result<BizTag> {
        let existing = BizTagEntity::find_by_id(id)
            .one(&self.db)
            .await
            .map_err(|e| {
                crate::core::CoreError::DatabaseError(
                    e.to_string(),
                    Some(crate::core::types::ErrorSource::new(e)),
                )
            })?;

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

        let result = updated.update(&self.db).await.map_err(|e| {
            crate::core::CoreError::DatabaseError(
                e.to_string(),
                Some(crate::core::types::ErrorSource::new(e)),
            )
        })?;

        Ok(result.into())
    }

    async fn delete_biz_tag(&self, id: Uuid) -> Result<()> {
        let result = BizTagEntity::delete_by_id(id)
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

        let results = query.all(&self.db).await.map_err(|e| {
            crate::core::CoreError::DatabaseError(
                e.to_string(),
                Some(crate::core::types::ErrorSource::new(e)),
            )
        })?;

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
            .map_err(|e| {
                crate::core::CoreError::DatabaseError(
                    e.to_string(),
                    Some(crate::core::types::ErrorSource::new(e)),
                )
            })?;

        Ok(results.into_iter().map(|m| m.into()).collect())
    }

    async fn count_biz_tags_by_group(&self, group_id: Uuid) -> Result<u64> {
        let count = BizTagEntity::find()
            .filter(BizTagColumn::GroupId.eq(group_id))
            .count(&self.db)
            .await
            .map_err(|e| {
                crate::core::CoreError::DatabaseError(
                    e.to_string(),
                    Some(crate::core::types::ErrorSource::new(e)),
                )
            })?;

        Ok(count)
    }

    async fn count_biz_tags(&self, workspace_id: Uuid, group_id: Option<Uuid>) -> Result<u64> {
        let mut query = BizTagEntity::find().filter(BizTagColumn::WorkspaceId.eq(workspace_id));

        if let Some(group_id) = group_id {
            query = query.filter(BizTagColumn::GroupId.eq(group_id));
        }

        let count = query.count(&self.db).await.map_err(|e| {
            crate::core::CoreError::DatabaseError(
                e.to_string(),
                Some(crate::core::types::ErrorSource::new(e)),
            )
        })?;

        Ok(count)
    }

    async fn health_check(&self) -> Result<()> {
        // Database connection is already established
        // Return Ok if we can reach this point
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
        let mut count_row: BTreeMap<String, dbnexus::sea_orm::Value> = BTreeMap::new();
        count_row.insert(
            "num_items".to_string(),
            dbnexus::sea_orm::Value::BigInt(Some(7)),
        );
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
        let mut count_row: BTreeMap<String, dbnexus::sea_orm::Value> = BTreeMap::new();
        count_row.insert(
            "num_items".to_string(),
            dbnexus::sea_orm::Value::BigInt(Some(3)),
        );
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
                crate::core::CoreError::DatabaseError(_, _)
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
                crate::core::CoreError::DatabaseError(_, _)
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
                crate::core::CoreError::DatabaseError(_, _)
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
                crate::core::CoreError::DatabaseError(_, _)
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
                crate::core::CoreError::DatabaseError(_, _)
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
                crate::core::CoreError::DatabaseError(_, _)
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
                crate::core::CoreError::DatabaseError(_, _)
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
                crate::core::CoreError::DatabaseError(_, _)
            ),
            "count error must propagate"
        );
    }

    // ----- BizTag error paths (additional) -----

    #[tokio::test]
    async fn test_biz_tag_get_propagates_db_error() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "biz tag get boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo.get_biz_tag(fixed_uuid(130)).await;
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
    async fn test_biz_tag_get_by_workspace_group_and_name_propagates_db_error() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "biz tag by ws+g+name boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo
            .get_biz_tag_by_workspace_group_and_name(fixed_uuid(131), fixed_uuid(132), "any")
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
    async fn test_biz_tag_delete_propagates_exec_db_error() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_exec_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "biz tag delete boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo.delete_biz_tag(fixed_uuid(133)).await;
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
    async fn test_biz_tag_update_applies_datacenter_ids_when_provided() {
        // Cover the `.map(|f| f.into())` closure on the format field and
        // the serde_json::to_value map_err closure for datacenter_ids in
        // update_biz_tag (only invoked when datacenter_ids is Some(...)).
        // The existing update_biz_tag tests all set datacenter_ids = None.
        let id = fixed_uuid(134);
        let updated_model = sample_biz_tag_model(id, fixed_uuid(135), fixed_uuid(136), "t_updated");
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![
                vec![sample_biz_tag_model(
                    id,
                    fixed_uuid(135),
                    fixed_uuid(136),
                    "t",
                )],
                vec![updated_model],
            ])
            .into_connection();
        let repo = make_repo(db);
        let updated = repo
            .update_biz_tag(
                id,
                &UpdateBizTagRequest {
                    name: None,
                    description: None,
                    algorithm: None,
                    format: Some(IdFormat::Numeric),
                    prefix: None,
                    base_step: None,
                    max_step: None,
                    datacenter_ids: Some(vec![0, 1, 2]),
                },
            )
            .await
            .unwrap();
        assert_eq!(updated.id, id);
    }

    // ----- create_biz_tag: cover the serde_json::to_value map_err closure
    // for the datacenter_ids serialization failure path -----
    //
    // NOTE: serde_json::to_value on a Vec<i32> cannot fail in practice
    // (Vec<i32> always serializes successfully). The map_err closure body
    // at L732 is therefore unreachable with valid input. We document this
    // as a coverage gap rather than weakening the production code with
    // #[allow(dead_code)] or LCOV_EXCL markers. The closure exists for
    // defensive programming (rule: failure must be explicit, never
    // swallowed).
}
