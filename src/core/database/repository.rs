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

//! 仓储核心:`SeaOrmRepository` 类型、跨域共享构件（集中式语句超时 helper）
//! 以及各资源域仓储模块（segment / workspace / biz_tag / api_key）的聚合 re-export。
//!
//! T036 起仓储实现按资源域拆分到同级子模块；本模块保留类型定义与构造装配，
//! 并对 trait 做 `pub use` 以维持 `crate::core::database::repository::XxxRepository`
//! 既有路径可用（调用方零改动）。

use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::core::auth::{Argon2KeyHasher, KeyHasher};
pub use crate::core::database::api_key_repository::ApiKeyRepository;
pub use crate::core::database::biz_tag_repository::BizTagRepository;
pub use crate::core::database::segment_repository::SegmentRepository;
pub use crate::core::database::workspace_repository::{GroupRepository, WorkspaceRepository};
use crate::core::types::Result;

/// T028 —— 语句超时默认值（秒）。与 `DatabaseConfig::statement_timeout_secs`
/// 的 serde 默认一致；仓储未经 `with_statement_timeout` 接线配置时按此兜底。
const DEFAULT_STATEMENT_TIMEOUT_SECS: u64 = 5;

/// T028 —— 集中式语句超时 helper：热查询经 `tokio::time::timeout` 包裹，
/// 防止 DB 挂起拖死生成/认证热路径。
///
/// 超时错误映射选择既有 [`crate::core::CoreError::TimeoutError`]（而非
/// `DatabaseError(String)`）：语义精确（调用方可匹配区分"慢"与"坏"），
/// 且避免把超时伪装成 DB 故障误导告警。
pub(crate) async fn with_statement_timeout<T, F>(timeout: Duration, fut: F) -> Result<T>
where
    F: std::future::Future<Output = Result<T>>,
{
    match tokio::time::timeout(timeout, fut).await {
        Ok(result) => result,
        Err(_elapsed) => Err(crate::core::CoreError::TimeoutError),
    }
}

/// 分割（`db` 为连接池廉价克隆、`key_hasher` 为 `Arc<dyn KeyHasher>`、
/// `distributed_lock` 为 `Option<Arc<..>>`）；SDK Kit 化（trait-kit TypeMap
/// 注入，`RepositoryInput` 需 `Clone`）依赖此 trait。
#[derive(Clone)]
pub struct SeaOrmRepository {
    pub(crate) db: dbnexus::sea_orm::DatabaseConnection,
    /// API Key 凭证哈希器：默认 [`Argon2KeyHasher`]（pepper/salt 语义随实现体
    /// 走），可经 [`Self::with_key_hasher`] 注入定制实现。
    pub(crate) key_hasher: Arc<dyn KeyHasher>,
    /// 分布式锁（可选，用于 segment 分配）
    #[cfg(feature = "etcd")]
    pub(crate) distributed_lock:
        Option<std::sync::Arc<dyn crate::core::coordinator::DistributedLock + Send + Sync>>,
    /// 本地分布式锁（无 etcd 时使用）
    #[cfg(not(feature = "etcd"))]
    pub(crate) distributed_lock:
        Option<std::sync::Arc<dyn crate::core::coordinator::DistributedLock + Send + Sync>>,
    /// last_used 写节流表：`key_id -> 上次触发 UPDATE 的时刻`。挂在 `Arc` 上
    /// 是刻意的 —— `SeaOrmRepository` 按 `Clone` 传播（SDK Kit 化克隆连接池），
    /// 各克隆必须共享同一份节流状态，否则节流形同虚设。
    pub(crate) last_used_writes: Arc<Mutex<HashMap<String, Instant>>>,
    /// T028 —— 单语句执行超时。热查询经 `with_statement_timeout` 包裹；
    /// 构造默认 [`DEFAULT_STATEMENT_TIMEOUT_SECS`]，可经
    /// [`Self::with_statement_timeout`] 接线 `DatabaseConfig::statement_timeout_secs`。
    pub(crate) statement_timeout: Duration,
}

impl SeaOrmRepository {
    pub fn new(db: dbnexus::sea_orm::DatabaseConnection, salt: String) -> Self {
        Self {
            db,
            // 构造入口签名（`db`, `salt`）保持不变：默认提供 Argon2id 哈希器
            // fallback，既有装配（main.rs / sdk::kit / router / management）
            // 零改动；定制实现经 [`Self::with_key_hasher`] 注入。
            key_hasher: Arc::new(Argon2KeyHasher::new(salt)),
            distributed_lock: None,
            last_used_writes: Arc::new(Mutex::new(HashMap::new())),
            statement_timeout: Duration::from_secs(DEFAULT_STATEMENT_TIMEOUT_SECS),
        }
    }

    /// 注入自定义 [`KeyHasher`]（默认构造已提供 [`Argon2KeyHasher`] fallback）。
    pub fn with_key_hasher(mut self, key_hasher: Arc<dyn KeyHasher>) -> Self {
        self.key_hasher = key_hasher;
        self
    }

    /// T028 —— 接线语句超时（来自 `DatabaseConfig::statement_timeout_secs`）。
    ///
    /// ```ignore
    /// SeaOrmRepository::new(conn, salt)
    ///     .with_statement_timeout(Duration::from_secs(config.database.statement_timeout_secs))
    /// ```
    pub fn with_statement_timeout(mut self, timeout: Duration) -> Self {
        self.statement_timeout = timeout;
        self
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
    pub fn get_db_connection(&self) -> &dbnexus::sea_orm::DatabaseConnection {
        &self.db
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

    /// 读取注入的分布式锁（兼容/诊断用途）。
    ///
    /// 号段分配自 T015 起改为单语句原子 `UPDATE ... RETURNING`，并发正确性由
    /// 数据库行锁保证，**热路径不再获取该锁**；注入 setter 与本读取器仅为
    /// 兼容既有装配 API（main.rs / sdk::kit）而保留。
    pub fn distributed_lock(
        &self,
    ) -> Option<&std::sync::Arc<dyn crate::core::coordinator::DistributedLock + Send + Sync>> {
        self.distributed_lock.as_ref()
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

    // ============== T028 语句超时 ==============

    /// 慢 future 超时 → 必须映射为可匹配的 `CoreError::TimeoutError`。
    #[tokio::test]
    async fn test_statement_timeout_maps_slow_future_to_timeout_error() {
        let result: Result<()> = with_statement_timeout(Duration::from_millis(20), async {
            tokio::time::sleep(Duration::from_millis(200)).await;
            Ok(())
        })
        .await;
        assert!(
            matches!(result, Err(crate::core::CoreError::TimeoutError)),
            "超时必须映射为 CoreError::TimeoutError，实际 {result:?}"
        );
    }

    /// 快 future 不受超时影响，原样透传结果。
    #[tokio::test]
    async fn test_statement_timeout_passes_through_fast_future() {
        let result: Result<u8> =
            with_statement_timeout(Duration::from_secs(5), async { Ok(7) }).await;
        assert_eq!(result.unwrap(), 7);
    }

    /// 仓储构造默认 5 秒，`with_statement_timeout` builder 可覆盖。
    #[test]
    fn test_seaorm_repository_statement_timeout_default_and_builder() {
        let repo = SeaOrmRepository::new(empty_pg_connection(), "salt".to_string());
        assert_eq!(
            repo.statement_timeout,
            Duration::from_secs(DEFAULT_STATEMENT_TIMEOUT_SECS),
            "构造默认必须与 DatabaseConfig::statement_timeout_secs 的 serde 默认一致"
        );

        let repo = repo.with_statement_timeout(Duration::from_secs(2));
        assert_eq!(repo.statement_timeout, Duration::from_secs(2));
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
        // T015 后锁仅为兼容注入保留（号段热路径不再取锁），getter 可诊断。
        assert!(repo.distributed_lock().is_some());
    }

    #[test]
    fn test_with_lock_returns_repository_with_lock() {
        let lock: Arc<dyn DistributedLock + Send + Sync> = Arc::new(DummyDistributedLock);
        let repo = SeaOrmRepository::new(empty_pg_connection(), "salt".to_string()).with_lock(lock);
        assert!(repo.distributed_lock().is_some());
    }

    // ==================================================================
    // Additional error-path coverage (Phase 2 P0: bring repository.rs to ≥95%)
    //
    // These tests close the remaining gaps in map_err closure coverage
    // for find/exec/insert calls that previously only had success-path
    // tests. Each test forces a DbErr and asserts it propagates as
    // CoreError::DatabaseError.
    // ==================================================================

    // ----- DistributedLock::is_healthy coverage -----

    #[test]
    fn test_dummy_distributed_lock_is_healthy_returns_true() {
        // Cover the `fn is_healthy(&self) -> bool { true }` body in
        // DummyDistributedLock (used as the "happy path" lock in segment
        // allocation tests).
        let lock = DummyDistributedLock;
        assert!(
            lock.is_healthy(),
            "DummyDistributedLock must always report healthy"
        );
    }

    #[test]
    fn test_failing_distributed_lock_is_healthy_returns_false() {
        // Cover the `fn is_healthy(&self) -> bool { false }` body in
        // FailingDistributedLock (used to exercise the InternalError path
        // in allocate_segment / allocate_segment_with_dc).
        let lock = FailingDistributedLock;
        assert!(
            !lock.is_healthy(),
            "FailingDistributedLock must always report unhealthy"
        );
    }
}

/// Integration tests requiring a live database.
///
/// Gated behind the `integration-tests` feature so that default builds
/// (and the coverage report) do not include the ~520 lines of `#[ignore]d`
/// test bodies. Enable with `--features integration-tests` and supply a real
/// `DATABASE_URL` together with `--ignored` to execute them.
#[cfg(all(test, feature = "integration-tests"))]
mod tests {
    use super::*;
    use dbnexus::sea_orm::{ConnectOptions, ConnectionTrait, Database, Statement};

    async fn setup_test_db(db: &dbnexus::sea_orm::DatabaseConnection) {
        // 直接复用生产迁移建表（`nebula-id migrate` 的同一代码路径）。
        // 此前手写 DDL 与实体定义漂移（表名 segments vs nebula_segments、
        // key_id VARCHAR(36) vs VARCHAR(64)、status 枚举列 vs VARCHAR(20)），
        // 在真实 PostgreSQL 上导致全部集成测试失败；以生产迁移为唯一
        // schema 事实源后不再漂移。
        crate::core::database::connection::run_migrations(db)
            .await
            .unwrap();
    }

    /// 测试连接 URL：注入 search_path（与生产 `create_connection` 同一
    /// 规则），否则 sea-orm 枚举 CAST 解析不到 nebula_id 下的类型。
    fn test_db_url() -> String {
        let raw = std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite::memory:".to_string());
        crate::core::database::connection::ensure_pg_search_path(&raw)
    }

    #[tokio::test]
    #[ignore]
    async fn test_repository_operations() {
        let db_url = test_db_url();
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
        let db_url = test_db_url();
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
        let db_url = test_db_url();
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
        let db_url = test_db_url();
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
        let db_url = test_db_url();
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
        let db_url = test_db_url();
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
        let db_url = test_db_url();
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
            "Should reject key_secret shorter than 8 characters"
        );
        if let Err(e) = result {
            assert!(
                e.to_string()
                    .contains("must be between 8 and 128 characters"),
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
        let db_url = test_db_url();
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
