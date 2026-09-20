// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! 号段（Segment）仓储:`SegmentRepository` trait 及其 SeaORM 实现，
//! 含 原子 `UPDATE ... RETURNING` 分配路径与 语句超时接线。

use async_trait::async_trait;
use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use dbnexus::sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, Set,
};
use tracing::{debug, info, Instrument};
use uuid::Uuid;

use crate::core::database::repository::{with_statement_timeout, SeaOrmRepository};
use crate::core::database::segment_entity::{
    ActiveModel as SegmentActiveModel, Column as SegmentColumn, Entity as SegmentEntity,
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

impl SeaOrmRepository {
    /// 号段原子分配（单语句 `UPDATE ... RETURNING`）。
    ///
    /// 并发正确性由数据库行锁保证：PostgreSQL 对同一行的并发 UPDATE 天然串行，
    /// 每个调用在单条语句内原子地完成「推进 current_id 并取回旧值作区间起点」，
    /// 各调用拿到的 `[start, max)` 区间互不重叠 —— 热路径**不再获取分布式锁**
    /// （etcd 往返是号段分配的吞吐瓶颈，且单行 UPDATE 本身就是互斥点）。
    ///
    /// 首次分配（无行）：`INSERT ... ON CONFLICT DO NOTHING` 兜底建行后重试一次
    /// UPDATE；并发首分配时输的一方 INSERT 静默跳过、由重试 UPDATE 正常取段
    /// （依赖 `UNIQUE (workspace_id, biz_tag, dc_id)` 约束，见 scripts/init.sql）。
    async fn allocate_segment_in_dc(
        &self,
        workspace_id: &str,
        biz_tag: &str,
        step: i32,
        dc_id: i32,
    ) -> Result<SegmentInfo> {
        // 单语句原子推进：RETURNING 中 `current_id - $1` 是推进前的旧值（区间
        // 起点，含），`current_id` 是推进后的新值（区间上界，不含）。
        let update_sql = r#"UPDATE nebula_id.nebula_segments SET current_id = current_id + $1, updated_at = CURRENT_TIMESTAMP WHERE workspace_id = $2 AND biz_tag = $3 AND dc_id = $4 RETURNING id, current_id - $1 AS start_id, current_id AS max_id, step, delta, created_at, updated_at"#;
        let update_stmt = || {
            dbnexus::sea_orm::Statement::from_sql_and_values(
                self.db.get_database_backend(),
                update_sql,
                [
                    step.into(),
                    workspace_id.into(),
                    biz_tag.into(),
                    dc_id.into(),
                ],
            )
        };

        if let Some(row) = self.db.query_one_raw(update_stmt()).await.map_err(|e| {
            crate::core::CoreError::DatabaseError(
                e.to_string(),
                Some(crate::core::types::ErrorSource::new(e)),
            )
        })? {
            debug!(
                workspace_id,
                biz_tag, dc_id, "segment allocated via atomic UPDATE RETURNING"
            );
            return segment_info_from_returning_row(&row, workspace_id, biz_tag);
        }

        // 无行：兜底建行。首段起点约定 dc_id * 10^12 + 1（dc_id = 0 时即 1），
        // 与历史 INSERT 语义一致；行内 current_id 先落在起点上，由随后的重试
        // UPDATE 推进一个步长、RETURNING 交出 [start, start + step) 区间。
        let start_id = (dc_id as i64) * 1_000_000_000_000i64 + 1i64;
        let insert_sql = r#"INSERT INTO nebula_id.nebula_segments (workspace_id, biz_tag, current_id, max_id, step, delta, dc_id) VALUES ($1, $2, $3, $3 + $4, $4, 1, $5) ON CONFLICT (workspace_id, biz_tag, dc_id) DO NOTHING"#;
        let insert_stmt = dbnexus::sea_orm::Statement::from_sql_and_values(
            self.db.get_database_backend(),
            insert_sql,
            [
                workspace_id.into(),
                biz_tag.into(),
                start_id.into(),
                step.into(),
                dc_id.into(),
            ],
        );
        self.db.execute_raw(insert_stmt).await.map_err(|e| {
            crate::core::CoreError::DatabaseError(
                e.to_string(),
                Some(crate::core::types::ErrorSource::new(e)),
            )
        })?;

        info!(
            workspace_id,
            biz_tag, dc_id, start_id, "segment row lazily created, retrying atomic allocation"
        );

        match self.db.query_one_raw(update_stmt()).await.map_err(|e| {
            crate::core::CoreError::DatabaseError(
                e.to_string(),
                Some(crate::core::types::ErrorSource::new(e)),
            )
        })? {
            Some(row) => segment_info_from_returning_row(&row, workspace_id, biz_tag),
            None => Err(crate::core::CoreError::DatabaseError(
                format!(
                    "segment allocation failed after insert-on-conflict retry for \
                     {workspace_id}/{biz_tag}/dc{dc_id}"
                ),
                None,
            )),
        }
    }
}

/// 把原子分配 `UPDATE ... RETURNING` 的结果行映射为 [`SegmentInfo`]。
///
/// 列契约（见 `allocate_segment_in_dc` 的 SQL）：
/// - `start_id` = 推进前旧 current_id（本次分配区间起点，含）；
/// - `max_id` = 推进后新 current_id（区间上界，不含）；
/// - `id`/`step`/`delta`/`created_at`/`updated_at` 原样回填。
fn segment_info_from_returning_row(
    row: &dbnexus::sea_orm::QueryResult,
    workspace_id: &str,
    biz_tag: &str,
) -> Result<SegmentInfo> {
    let id = row.try_get::<i64>("", "id").map_err(|e| {
        crate::core::CoreError::DatabaseError(
            e.to_string(),
            Some(crate::core::types::ErrorSource::new(e)),
        )
    })?;
    let start_id = row.try_get::<i64>("", "start_id").map_err(|e| {
        crate::core::CoreError::DatabaseError(
            e.to_string(),
            Some(crate::core::types::ErrorSource::new(e)),
        )
    })?;
    let max_id = row.try_get::<i64>("", "max_id").map_err(|e| {
        crate::core::CoreError::DatabaseError(
            e.to_string(),
            Some(crate::core::types::ErrorSource::new(e)),
        )
    })?;
    let step = row.try_get::<i32>("", "step").map_err(|e| {
        crate::core::CoreError::DatabaseError(
            e.to_string(),
            Some(crate::core::types::ErrorSource::new(e)),
        )
    })?;
    let delta = row.try_get::<i32>("", "delta").map_err(|e| {
        crate::core::CoreError::DatabaseError(
            e.to_string(),
            Some(crate::core::types::ErrorSource::new(e)),
        )
    })?;
    let created_at: DateTime<Utc> = naive_to_utc(
        row.try_get::<Option<NaiveDateTime>>("", "created_at")
            .map_err(|e| {
                crate::core::CoreError::DatabaseError(
                    e.to_string(),
                    Some(crate::core::types::ErrorSource::new(e)),
                )
            })?,
    );
    let updated_at: DateTime<Utc> = naive_to_utc(
        row.try_get::<Option<NaiveDateTime>>("", "updated_at")
            .map_err(|e| {
                crate::core::CoreError::DatabaseError(
                    e.to_string(),
                    Some(crate::core::types::ErrorSource::new(e)),
                )
            })?,
    );

    Ok(SegmentInfo {
        id,
        workspace_id: workspace_id.to_string(),
        biz_tag: biz_tag.to_string(),
        current_id: start_id,
        max_id,
        step: step.max(0) as u32,
        delta: delta.max(0) as u32,
        created_at,
        updated_at,
    })
}

fn naive_to_utc(naive: Option<NaiveDateTime>) -> DateTime<Utc> {
    naive
        .map(|n| Utc.from_utc_datetime(&n))
        .unwrap_or_else(Utc::now)
}

#[async_trait]
impl SegmentRepository for SeaOrmRepository {
    async fn get_segment(&self, workspace_id: &str, biz_tag: &str) -> Result<Option<SegmentInfo>> {
        let result = SegmentEntity::find()
            .filter(SegmentColumn::WorkspaceId.eq(workspace_id))
            .filter(SegmentColumn::BizTag.eq(biz_tag))
            .one(&self.db)
            .await
            .map_err(|e| {
                crate::core::CoreError::DatabaseError(
                    e.to_string(),
                    Some(crate::core::types::ErrorSource::new(e)),
                )
            })?;

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
        // 非 dc 变体即 dc_id = 0 的号段（实体列默认值 0），与 dc 变体共用同一
        // 原子分配路径，保证两种调用形态读写同一套行、互不越界。
        // 热路径观测：span 字段仅 workspace/biz_tag/step，无敏感数据。
        // 生成热路径经集中式语句超时包裹，DB 挂起时显性返回
        // TimeoutError（由 Segment 降级链接管），而非无限悬挂。
        let span = tracing::info_span!(
            "db.allocate_segment",
            workspace = workspace_id,
            biz_tag = biz_tag,
            step = step
        );
        with_statement_timeout(
            self.statement_timeout,
            self.allocate_segment_in_dc(workspace_id, biz_tag, step, 0),
        )
        .instrument(span)
        .await
    }

    async fn allocate_segment_with_dc(
        &self,
        workspace_id: &str,
        biz_tag: &str,
        step: i32,
        dc_id: i32,
    ) -> Result<SegmentInfo> {
        // 与 `allocate_segment` 同一超时口径（dc 变体同为生成热路径）。
        with_statement_timeout(
            self.statement_timeout,
            self.allocate_segment_in_dc(workspace_id, biz_tag, step, dc_id),
        )
        .await
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
            .map_err(|e| {
                crate::core::CoreError::DatabaseError(
                    e.to_string(),
                    Some(crate::core::types::ErrorSource::new(e)),
                )
            })?;

        if result.rows_affected == 0 {
            return Err(crate::core::CoreError::NotFound(t!(
                "error.detail.segment_not_found",
                workspace_id = workspace_id,
                biz_tag = biz_tag
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

        let inserted = new_segment.insert(&self.db).await.map_err(|e| {
            crate::core::CoreError::DatabaseError(
                e.to_string(),
                Some(crate::core::types::ErrorSource::new(e)),
            )
        })?;

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
            .map_err(|e| {
                crate::core::CoreError::DatabaseError(
                    e.to_string(),
                    Some(crate::core::types::ErrorSource::new(e)),
                )
            })?;

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
            .map_err(|e| {
                crate::core::CoreError::DatabaseError(
                    e.to_string(),
                    Some(crate::core::types::ErrorSource::new(e)),
                )
            })?;

        if result.rows_affected == 0 {
            return Err(crate::core::CoreError::NotFound(t!(
                "error.detail.segment_not_found",
                workspace_id = workspace_id,
                biz_tag = biz_tag
            )));
        }

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

    // --- allocate_segment / allocate_segment_with_dc（原子分配） ---
    //
    // 热路径已无分布式锁、无显式事务：正常路径是 1 条 `UPDATE ... RETURNING`；
    // 首查无行路径是 `INSERT ... ON CONFLICT DO NOTHING` + 重试一次 UPDATE。
    // MockDatabase 依序弹出 append 的 query（raw UPDATE）/exec（INSERT）结果。

    use dbnexus::sea_orm::sea_query::Value;

    /// 构造 `UPDATE ... RETURNING` 的结果行（列契约见
    /// `segment_info_from_returning_row`）。
    fn returning_row(start_id: i64, max_id: i64) -> MockRow {
        BTreeMap::from([
            ("id".to_string(), Value::BigInt(Some(1))),
            ("start_id".to_string(), Value::BigInt(Some(start_id))),
            ("max_id".to_string(), Value::BigInt(Some(max_id))),
            ("step".to_string(), Value::Int(Some(100))),
            ("delta".to_string(), Value::Int(Some(1))),
            (
                "created_at".to_string(),
                Value::ChronoDateTime(Some(fixed_datetime(1_600_000_000))),
            ),
            (
                "updated_at".to_string(),
                Value::ChronoDateTime(Some(fixed_datetime(1_700_000_000))),
            ),
        ])
        .into_mock_row()
    }

    #[tokio::test]
    async fn test_segment_allocate_normal_path_is_single_update_returning() {
        // 正常路径：行已存在，仅 1 条 UPDATE RETURNING，别无其它语句。
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![returning_row(100, 200)]])
            .into_connection();
        let repo = make_repo(db);

        let seg = repo.allocate_segment("ws1", "t1", 100).await.unwrap();
        assert_eq!(seg.current_id, 100, "start_id = 推进前旧 current_id");
        assert_eq!(seg.max_id, 200, "max_id = 推进后新 current_id");
        assert_eq!(seg.step, 100);
        assert_eq!(seg.workspace_id, "ws1");
        assert_eq!(seg.biz_tag, "t1");

        let log = repo.get_db_connection().clone().into_transaction_log();
        assert_eq!(log.len(), 1, "热路径必须恰好 1 条语句");
        let sql = log[0].statements()[0].sql.to_uppercase();
        assert!(sql.contains("UPDATE"), "got: {sql}");
        assert!(sql.contains("RETURNING"), "got: {sql}");
        assert!(!sql.contains("INSERT"), "got: {sql}");
    }

    #[tokio::test]
    async fn test_segment_allocate_creates_row_on_first_allocation() {
        // 首查无行：UPDATE(0 行) → INSERT ON CONFLICT DO NOTHING → 重试 UPDATE。
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![
                Vec::<MockRow>::new(),       // 首查 UPDATE：无行
                vec![returning_row(1, 101)], // 重试 UPDATE
            ])
            .append_exec_results(vec![MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }]) // INSERT DO NOTHING
            .into_connection();
        let repo = make_repo(db);

        let seg = repo.allocate_segment("ws1", "t1", 100).await.unwrap();
        assert_eq!(seg.current_id, 1, "首段起点为 1");
        assert_eq!(seg.max_id, 101, "max_id = start + step");

        let log = repo.get_db_connection().clone().into_transaction_log();
        assert_eq!(log.len(), 3, "UPDATE + INSERT + UPDATE");
        let kinds: Vec<String> = log
            .iter()
            .map(|t| t.statements()[0].sql.to_uppercase())
            .collect();
        assert!(kinds[0].starts_with("UPDATE"), "got: {}", kinds[0]);
        assert!(
            kinds[1].starts_with("INSERT") && kinds[1].contains("ON CONFLICT"),
            "got: {}",
            kinds[1]
        );
        assert!(kinds[2].starts_with("UPDATE"), "got: {}", kinds[2]);
    }

    #[tokio::test]
    async fn test_segment_allocate_with_dc_normal_path_is_single_update_returning() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![returning_row(
                5_000_000_000_100,
                5_000_000_000_200,
            )]])
            .into_connection();
        let repo = make_repo(db);

        let seg = repo
            .allocate_segment_with_dc("ws1", "t1", 100, 5)
            .await
            .unwrap();
        assert_eq!(seg.current_id, 5_000_000_000_100);
        assert_eq!(seg.max_id, 5_000_000_000_200);

        let log = repo.get_db_connection().clone().into_transaction_log();
        assert_eq!(log.len(), 1, "dc 正常路径同样恰好 1 条语句");
        let sql = log[0].statements()[0].sql.to_uppercase();
        assert!(
            sql.contains("UPDATE") && sql.contains("RETURNING"),
            "got: {sql}"
        );
    }

    #[tokio::test]
    async fn test_segment_allocate_with_dc_creates_row_with_dc_offset() {
        // dc 变体首分配：起点约定 dc_id * 10^12 + 1。
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![
                Vec::<MockRow>::new(),
                vec![returning_row(5_000_000_000_001, 5_000_000_000_101)],
            ])
            .append_exec_results(vec![MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            .into_connection();
        let repo = make_repo(db);

        let seg = repo
            .allocate_segment_with_dc("ws1", "t1", 100, 5)
            .await
            .unwrap();
        assert_eq!(seg.current_id, 5_000_000_000_001);
        assert_eq!(seg.max_id, 5_000_000_000_101);
    }

    #[tokio::test]
    async fn test_segment_allocate_does_not_acquire_distributed_lock() {
        // 热路径不再取分布式锁 —— 注入一把 acquire 必失败的锁，
        // 分配仍应成功（若取锁则会 InternalError）。
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![returning_row(1, 101)]])
            .into_connection();
        let lock: Arc<dyn DistributedLock + Send + Sync> = Arc::new(FailingDistributedLock);
        let repo = SeaOrmRepository::new(db, "salt".to_string()).with_distributed_lock(lock);

        let seg = repo.allocate_segment("ws1", "t1", 100).await.unwrap();
        assert_eq!(seg.current_id, 1);
        assert_eq!(seg.max_id, 101);
    }

    #[tokio::test]
    async fn test_segment_allocate_propagates_update_db_error() {
        // 正常路径 UPDATE 报错 → DatabaseError 透传。
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "segment allocate update boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo.allocate_segment("ws1", "t1", 100).await;
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_, _)
            ),
            "segment allocate update error must propagate as DatabaseError"
        );
    }

    #[tokio::test]
    async fn test_segment_allocate_propagates_insert_db_error() {
        // 首查无行且 INSERT 报错 → DatabaseError 透传。
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![Vec::<MockRow>::new()])
            .append_exec_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "segment allocate insert boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo.allocate_segment("ws1", "t1", 100).await;
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_, _)
            ),
            "segment allocate insert error must propagate as DatabaseError"
        );
    }

    #[tokio::test]
    async fn test_segment_allocate_propagates_retry_update_db_error() {
        // 首查无行、INSERT 成功、重试 UPDATE 报错 → DatabaseError 透传。
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![Vec::<MockRow>::new()])
            .append_exec_results(vec![MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            .append_query_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "segment allocate retry update boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo.allocate_segment("ws1", "t1", 100).await;
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_, _)
            ),
            "segment allocate retry update error must propagate as DatabaseError"
        );
    }

    #[tokio::test]
    async fn test_segment_allocate_fails_loudly_when_retry_update_misses() {
        // INSERT 后重试 UPDATE 仍无行（如行被并发删除）→ 显性报错而非静默。
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![Vec::<MockRow>::new(), Vec::<MockRow>::new()])
            .append_exec_results(vec![MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            .into_connection();
        let repo = make_repo(db);
        let result = repo.allocate_segment("ws1", "t1", 100).await;
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(ref m, _) if m.contains("retry")
            ),
            "retry miss must fail loudly as DatabaseError"
        );
    }

    #[tokio::test]
    async fn test_segment_allocate_with_dc_propagates_update_db_error() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "segment allocate_dc update boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo.allocate_segment_with_dc("ws1", "t1", 100, 1).await;
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_, _)
            ),
            "segment allocate_with_dc update error must propagate as DatabaseError"
        );
    }

    #[tokio::test]
    async fn test_segment_allocate_with_dc_propagates_insert_db_error() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![Vec::<MockRow>::new()])
            .append_exec_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "segment allocate_dc insert boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo.allocate_segment_with_dc("ws1", "t1", 100, 1).await;
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_, _)
            ),
            "segment allocate_with_dc insert error must propagate as DatabaseError"
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
                crate::core::CoreError::DatabaseError(_, _)
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
                crate::core::CoreError::DatabaseError(_, _)
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
                crate::core::CoreError::DatabaseError(_, _)
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
                crate::core::CoreError::DatabaseError(_, _)
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
                crate::core::CoreError::DatabaseError(_, _)
            ),
            "segment delete exec error must propagate as DatabaseError"
        );
    }

    // ----- Segment txn.begin/commit error paths -----
    //
    // The .map_err closures for txn.begin() (L1280, L1423) and txn.commit()
    // (L1381, L1528) in allocate_segment / allocate_segment_with_dc are
    // unreachable with sea-orm's MockDatabase, which always returns Ok
    // for begin() and commit(). SeaORM does not expose an API to inject
    // begin()/commit() failures through the mock. To exercise these paths
    // would require either:
    //   (a) a custom wrapper around DatabaseConnection that intercepts
    //       begin()/commit() and injects errors, or
    //   (b) integration tests against a real database that can be made to
    //       fail (e.g., by revoking permissions mid-transaction).
    // Both are out of scope for this Phase 2 P0 coverage push. The closures
    // remain as defensive error handling per rule 12 (failure must be
    // explicit). Documented as a known coverage gap.
}
