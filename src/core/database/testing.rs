// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! 仓储层共享测试夹具（仅 `#[cfg(test)]` 编译）：Mock 连接、样例 Model、固定 UUID/时间 helper 与锁替身。

use async_trait::async_trait;
use chrono::NaiveDateTime;
use dbnexus::sea_orm::{DatabaseBackend, MockDatabase};
use uuid::Uuid;

use crate::core::coordinator::{DistributedLock, LockError, LockGuard};
use crate::core::database::api_key_entity::Model as ApiKeyModel;
use crate::core::database::biz_tag_entity::{AlgorithmTypeDb, IdFormatDb, Model as BizTagModel};
use crate::core::database::group_entity::Model as GroupModel;
use crate::core::database::repository::SeaOrmRepository;
use crate::core::database::segment_entity::Model as SegmentModel;
use crate::core::database::workspace_entity::Model as WorkspaceModel;

// ----- Helpers -----

/// 构造基于 `MockDatabase` 的 `SeaOrmRepository`。
pub(crate) fn make_repo(db: dbnexus::sea_orm::DatabaseConnection) -> SeaOrmRepository {
    SeaOrmRepository::new(db, "test_salt".to_string())
}

/// 构造空的 `MockDatabase` 连接（Postgres backend），供无需 mock 查询结果的用例使用。
pub(crate) fn empty_pg_connection() -> dbnexus::sea_orm::DatabaseConnection {
    MockDatabase::new(DatabaseBackend::Postgres).into_connection()
}

/// 恒成功的琐碎分布式锁，供仅需注入锁的测试使用。
pub(crate) struct DummyDistributedLock;

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

pub(crate) fn fixed_uuid(n: u8) -> Uuid {
    Uuid::from_bytes([n; 16])
}

#[allow(deprecated)] // 该 helper 需继续使用 deprecated API
pub(crate) fn fixed_datetime(secs: i64) -> NaiveDateTime {
    NaiveDateTime::from_timestamp_opt(secs, 0).unwrap()
}

pub(crate) fn sample_workspace_model(id: Uuid, name: &str) -> WorkspaceModel {
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

pub(crate) fn sample_group_model(id: Uuid, workspace_id: Uuid, name: &str) -> GroupModel {
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

pub(crate) fn sample_biz_tag_model(
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

pub(crate) fn sample_api_key_model(id: Uuid, key_id: &str, role: &str) -> ApiKeyModel {
    ApiKeyModel {
        id,
        key_id: key_id.to_string(),
        key_secret_hash:
            "$argon2id$v=19$m=19456,t=2,p=1$YWNldG9uAAAAAAAAAAAAAAAAAAA$N+1jKtFi1q5p9tLi0dK0pQ"
                .to_string(),
        prev_secret_hash: None,
        rotate_expires_at: None,
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

pub(crate) fn sample_segment_model(id: i64, workspace_id: &str, biz_tag: &str) -> SegmentModel {
    SegmentModel {
        id,
        workspace_id: workspace_id.to_string(),
        biz_tag: biz_tag.to_string(),
        current_id: 100,
        max_id: 1000,
        step: 100,
        delta: 1,
        dc_id: 0,
        ceiling: None,
        created_at: fixed_datetime(1_600_000_000),
        updated_at: fixed_datetime(1_700_000_000),
    }
}

/// 仅测试用的 no-op 锁守卫：测试用 SQLite 单连接，事务本身提供原子性。
/// 生产未配置锁时 `allocate_segment` 返回 `ConfigurationError`，禁止静默降级。
#[cfg(test)]
pub(crate) struct NoopLockGuard;

#[cfg(test)]
#[async_trait]
impl LockGuard for NoopLockGuard {
    async fn release(&self) -> std::result::Result<(), LockError> {
        Ok(())
    }
}

/// `acquire` 恒失败的分布式锁，用于覆盖 `allocate_segment` 的 `InternalError` 分支。
pub(crate) struct FailingDistributedLock;

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
