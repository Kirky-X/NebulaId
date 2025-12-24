use async_trait::async_trait;
use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set, TransactionTrait};
use tracing::{debug, info};

use crate::database::segment_entity::{ActiveModel, Column, Entity};
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

pub struct SeaOrmSegmentRepository {
    db: sea_orm::DatabaseConnection,
}

impl SeaOrmSegmentRepository {
    pub fn new(db: sea_orm::DatabaseConnection) -> Self {
        Self { db }
    }
}

#[async_trait]
impl SegmentRepository for SeaOrmSegmentRepository {
    async fn get_segment(&self, workspace_id: &str, biz_tag: &str) -> Result<Option<SegmentInfo>> {
        let result = Entity::find()
            .filter(Column::WorkspaceId.eq(workspace_id))
            .filter(Column::BizTag.eq(biz_tag))
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

        let existing = Entity::find()
            .filter(Column::WorkspaceId.eq(workspace_id))
            .filter(Column::BizTag.eq(biz_tag))
            .one(&txn)
            .await
            .map_err(|e| crate::CoreError::DatabaseError(e.to_string()))?;

        let segment = match existing {
            Some(model) => {
                let current_id = model.current_id;
                let max_id = model.max_id;
                let new_max_id = current_id + step as i64;

                let updated = ActiveModel {
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

                let new_segment = ActiveModel {
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

    async fn update_segment(
        &self,
        workspace_id: &str,
        biz_tag: &str,
        current_id: i64,
        _max_id: i64,
    ) -> Result<()> {
        let result = Entity::update_many()
            .filter(Column::WorkspaceId.eq(workspace_id))
            .filter(Column::BizTag.eq(biz_tag))
            .set(ActiveModel {
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
        let new_segment = ActiveModel {
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
        let results = Entity::find()
            .filter(Column::WorkspaceId.eq(workspace_id))
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
        let result = Entity::delete_many()
            .filter(Column::WorkspaceId.eq(workspace_id))
            .filter(Column::BizTag.eq(biz_tag))
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
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            )",
        ))
        .await
        .unwrap();

        let repo = SeaOrmSegmentRepository::new(db);

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
