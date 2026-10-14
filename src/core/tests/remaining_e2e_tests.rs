// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

#![cfg(test)]

//! 剩余功能模块端到端测试：ParseRequest 边界、密钥轮换宽限期 clamp、仓储 CRUD、
//! AlertManager 生命周期；workspace 权限校验因函数私有跳过（router.rs 单元测试已覆盖）。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use sdforge::validator::Validate;
use uuid::Uuid;

use crate::core::database::{
    ApiKey, ApiKeyInfo, ApiKeyRepository, ApiKeyResponse, ApiKeyRole, ApiKeyWithSecret,
    AuthenticatedKey, CreateApiKeyRequest,
};
use crate::core::monitoring::core::{
    AlertManager, AlertNotificationSender, AlertRule, AlertSeverity, AlertingConfig,
    NotificationChannel,
};
use crate::core::types::metrics::GlobalMetrics;
use crate::core::types::{CoreError, Result};
use crate::server::handlers::ApiHandlers;
use crate::server::models::ParseRequest;

// =============================================================================
// 1. ParseRequest 验证
// =============================================================================

/// ParseRequest.workspace 长度边界 1-64：0 → Err，1 → Ok，64 → Ok，65 → Err。
#[test]
fn e2e_parse_request_validates_workspace_length() {
    // 长度 0 → 拒绝（低于 min=1）
    let req = make_parse_request_with_workspace("");
    let errs = req.validate();
    assert!(
        errs.is_err(),
        "workspace 长度 0 必须拒绝（min=1），实际得到 {errs:?}"
    );

    // 长度 1 → 通过（达到 min）
    let req = make_parse_request_with_workspace("a");
    assert!(req.validate().is_ok(), "workspace 长度 1 必须通过（min=1）");

    // 长度 64 → 通过（达到 max）
    let ws_64 = "a".repeat(64);
    let req = make_parse_request_with_workspace(&ws_64);
    assert!(
        req.validate().is_ok(),
        "workspace 长度 64 必须通过（max=64）"
    );

    // 长度 65 → 拒绝（超过 max）
    let ws_65 = "a".repeat(65);
    let req = make_parse_request_with_workspace(&ws_65);
    assert!(
        req.validate().is_err(),
        "workspace 长度 65 必须拒绝（max=64）"
    );
}

/// ParseRequest.algorithm 长度边界 0-32：0/32 → Ok，33 → Err（0 为 serde_default）。
#[test]
fn e2e_parse_request_validates_algorithm_length() {
    // 长度 0 → 通过（min=0，serde_default）
    let req = make_parse_request_with_algorithm("");
    assert!(req.validate().is_ok(), "algorithm 长度 0 必须通过（min=0）");

    // 长度 32 → 通过（达到 max）
    let algo_32 = "a".repeat(32);
    let req = make_parse_request_with_algorithm(&algo_32);
    assert!(
        req.validate().is_ok(),
        "algorithm 长度 32 必须通过（max=32）"
    );

    // 长度 33 → 拒绝（超过 max）
    let algo_33 = "a".repeat(33);
    let req = make_parse_request_with_algorithm(&algo_33);
    assert!(
        req.validate().is_err(),
        "algorithm 长度 33 必须拒绝（max=32）"
    );
}

/// ParseRequest.id 无长度约束：空串与超长（10KB）均通过校验。
#[test]
fn e2e_parse_request_empty_id_accepted() {
    // 空 id → 通过（id 无长度约束）
    let req = make_parse_request_with_id("");
    assert!(
        req.validate().is_ok(),
        "空 id 必须通过校验（id 字段无 #[validate] 约束）"
    );

    // 超长 id（10KB）→ 通过
    let long_id = "x".repeat(10_000);
    let req = make_parse_request_with_id(&long_id);
    assert!(
        req.validate().is_ok(),
        "超长 id 必须通过校验（id 字段无 #[validate] 约束）"
    );
}

/// 构造 ParseRequest，覆盖 workspace 字段为指定值，其余字段为合法默认值。
fn make_parse_request_with_workspace(workspace: &str) -> ParseRequest {
    ParseRequest {
        id: "123".to_string(),
        workspace: workspace.to_string(),
        group: "g".to_string(),
        biz_tag: "tag".to_string(),
        algorithm: String::new(),
    }
}

/// 构造 ParseRequest，覆盖 algorithm 字段为指定值。
fn make_parse_request_with_algorithm(algorithm: &str) -> ParseRequest {
    ParseRequest {
        id: "123".to_string(),
        workspace: "ws".to_string(),
        group: "g".to_string(),
        biz_tag: "tag".to_string(),
        algorithm: algorithm.to_string(),
    }
}

/// 构造 ParseRequest，覆盖 id 字段为指定值。
fn make_parse_request_with_id(id: &str) -> ParseRequest {
    ParseRequest {
        id: id.to_string(),
        workspace: "ws".to_string(),
        group: "g".to_string(),
        biz_tag: "tag".to_string(),
        algorithm: String::new(),
    }
}

// =============================================================================
// 2. 密钥轮换宽限期（ApiHandlers::with_key_rotation_grace_period）
// =============================================================================
// 宽限期字段为 pub(super)，测试经 rotate_api_key → repo 捕获参数间接验证 clamp。
// MIN=0（关闭宽限期，默认合法）、MAX=30 天 = 2_592_000 秒。

/// grace_period 在 [0, 30天] 范围内时，原值传入 repo。
///
/// 验证 builder 不修改合法范围内的值。
#[tokio::test]
async fn e2e_key_rotation_grace_period_valid_range() {
    // 选一个范围内的中间值（1 小时），便于与边界值区分
    const VALID_SECONDS: u64 = 3600;

    let mock_repo = Arc::new(RecordingApiKeyRepo::new());
    let handlers = build_handlers_with_repo_and_grace(mock_repo.clone(), VALID_SECONDS);

    let _ = handlers.rotate_api_key("nino_test_key").await;

    let captured = mock_repo.captured_grace_seconds();
    assert_eq!(
        captured,
        Some(VALID_SECONDS),
        "范围内的 grace_period 必须原值传入 repo（不应 clamp）"
    );
}

/// grace_period = 0 表示"关闭宽限期"，必须原值透传（不得抬高为非零窗口）。
#[tokio::test]
async fn e2e_key_rotation_grace_period_zero_disables_grace() {
    let mock_repo = Arc::new(RecordingApiKeyRepo::new());
    let handlers = build_handlers_with_repo_and_grace(mock_repo.clone(), 0);

    let _ = handlers.rotate_api_key("nino_test_key").await;

    let captured = mock_repo.captured_grace_seconds();
    assert_eq!(
        captured,
        Some(0),
        "grace_period=0 必须原值传给仓储（关闭宽限期），不得被抬高成非零窗口"
    );
}

/// grace_period > 30 天 时 clamp 到 30 天。
#[tokio::test]
async fn e2e_key_rotation_grace_period_above_maximum_clamped() {
    const MAX_SECONDS: u64 = 30 * 24 * 60 * 60; // 2_592_000
                                                // 31 天，超过 max
    let over_max: u64 = 31 * 24 * 60 * 60;

    let mock_repo = Arc::new(RecordingApiKeyRepo::new());
    let handlers = build_handlers_with_repo_and_grace(mock_repo.clone(), over_max);

    let _ = handlers.rotate_api_key("nino_test_key").await;

    let captured = mock_repo.captured_grace_seconds();
    assert_eq!(
        captured,
        Some(MAX_SECONDS),
        "grace_period={} 必须 clamp 到 {}（MAX_GRACE_PERIOD_SECONDS）",
        over_max,
        MAX_SECONDS
    );
}

// -----------------------------------------------------------------------------
// 辅助：构造挂载了 mock repo + 指定 grace_period 的 ApiHandlers
// -----------------------------------------------------------------------------

/// 构造 ApiHandlers：MockIdGenerator 占位 + MinimalConfigService，
/// 注入 RecordingApiKeyRepo 并设置 grace period。
fn build_handlers_with_repo_and_grace(
    repo: Arc<RecordingApiKeyRepo>,
    grace_seconds: u64,
) -> Arc<ApiHandlers> {
    let id_generator: Arc<dyn crate::core::algorithm::IdGenerator> =
        Arc::new(crate::server::handlers::mock_generator::MockIdGenerator::new());
    let config_service: Arc<dyn crate::server::config::management::ConfigManagementService> =
        Arc::new(MinimalConfigService);
    let repo_dyn: Arc<dyn ApiKeyRepository> = repo;
    Arc::new(
        ApiHandlers::with_api_key_repository(id_generator, config_service, repo_dyn)
            .with_key_rotation_grace_period(grace_seconds),
    )
}

// =============================================================================
// 3. workspace 权限校验（verify_user_workspace）—— 跳过
// =============================================================================
//
// 跳过原因：`verify_user_workspace` / `verify_workspace_id_match` 定义在
// `src/server/router.rs`，为模块私有 `async fn`，外部 e2e 测试无法直接调用。
// 匹配 Ok / 不匹配 403 / 禁用认证允许三分支已由 router.rs 内 `#[cfg(test)]`
// 单元测试直接覆盖，故本文件不再重复。

// =============================================================================
// 4. 仓储 CRUD（ApiKeyRepository trait 内存 mock）
// =============================================================================

/// create_api_key 后可通过 get_api_key_by_id 查询到，字段与写入一致。
#[tokio::test]
async fn e2e_repository_crud_create_and_get_api_key() {
    let repo = CrudApiKeyRepo::new();

    let create_req = CreateApiKeyRequest {
        workspace_id: Some(Uuid::new_v4()),
        name: "crud-test-key".to_string(),
        description: Some("e2e crud test".to_string()),
        role: ApiKeyRole::User,
        rate_limit: Some(5000),
        expires_at: None,
        key_secret: Some("test-secret-12345".to_string()),
        key_id: Some("nino_crud_test_key".to_string()),
    };

    let created = repo
        .create_api_key(&create_req)
        .await
        .expect("E2E: create_api_key 必须成功");
    assert_eq!(created.key.key_id, "nino_crud_test_key");
    assert_eq!(created.key.name, "crud-test-key");
    assert_eq!(created.key.rate_limit, 5000);
    assert!(created.key.enabled);
    assert!(!created.key_secret.is_empty());

    let fetched = repo
        .get_api_key_by_id(&created.key.key_id)
        .await
        .expect("E2E: get_api_key_by_id 必须成功")
        .expect("E2E: 查询刚创建的 key 必须返回 Some");
    assert_eq!(fetched.key_id, created.key.key_id);
    assert_eq!(fetched.name, created.key.name);
    assert_eq!(fetched.rate_limit, created.key.rate_limit);
    assert_eq!(fetched.role, ApiKeyRole::User);
}

/// create 多个 key 后 list 按 workspace_id 过滤，limit/offset 行为符合预期。
#[tokio::test]
async fn e2e_repository_crud_list_api_keys() {
    let repo = CrudApiKeyRepo::new();
    let ws_id = Uuid::new_v4();
    let other_ws = Uuid::new_v4();

    // 在 ws_id 下创建 3 个 key
    for i in 0..3 {
        let req = CreateApiKeyRequest {
            workspace_id: Some(ws_id),
            name: format!("list-key-{i}"),
            description: None,
            role: ApiKeyRole::User,
            rate_limit: Some(1000),
            expires_at: None,
            key_secret: Some(format!("secret-{i}-12345")),
            key_id: Some(format!("nino_list_key_{i}")),
        };
        repo.create_api_key(&req)
            .await
            .expect("E2E: create_api_key 必须成功");
    }

    // 在 other_ws 下创建 1 个 key（不应出现在 ws_id 的列表中）
    let req = CreateApiKeyRequest {
        workspace_id: Some(other_ws),
        name: "other-key".to_string(),
        description: None,
        role: ApiKeyRole::User,
        rate_limit: Some(1000),
        expires_at: None,
        key_secret: Some("other-secret-12345".to_string()),
        key_id: Some("nino_other_key".to_string()),
    };
    repo.create_api_key(&req)
        .await
        .expect("E2E: create_api_key 必须成功");

    // list ws_id 下的 key（无 limit/offset）
    let all = repo
        .list_api_keys(ws_id, None, None)
        .await
        .expect("E2E: list_api_keys 必须成功");
    assert_eq!(all.len(), 3, "ws_id 下应有 3 个 key，实际 {}", all.len());
    assert!(
        all.iter().all(|k| k.name.starts_with("list-key-")),
        "返回的 key 必须都属于 ws_id"
    );

    // list with limit
    let limited = repo
        .list_api_keys(ws_id, Some(2), None)
        .await
        .expect("E2E: list_api_keys with limit 必须成功");
    assert_eq!(limited.len(), 2, "limit=2 必须只返回 2 个 key");

    // list with offset
    let offset = repo
        .list_api_keys(ws_id, None, Some(1))
        .await
        .expect("E2E: list_api_keys with offset 必须成功");
    assert_eq!(offset.len(), 2, "offset=1 后应返回剩余 2 个 key");

    // other_ws 下只有 1 个 key
    let other = repo
        .list_api_keys(other_ws, None, None)
        .await
        .expect("E2E: list_api_keys other_ws 必须成功");
    assert_eq!(other.len(), 1, "other_ws 下应有 1 个 key");
}

/// revoke 后 get 返回 None；revoke 不存在的 id 返回 NotFound。
#[tokio::test]
async fn e2e_repository_crud_revoke_api_key() {
    let repo = CrudApiKeyRepo::new();

    let create_req = CreateApiKeyRequest {
        workspace_id: Some(Uuid::new_v4()),
        name: "revoke-test-key".to_string(),
        description: None,
        role: ApiKeyRole::User,
        rate_limit: Some(1000),
        expires_at: None,
        key_secret: Some("revoke-secret-12345".to_string()),
        key_id: Some("nino_revoke_key".to_string()),
    };
    let created = repo
        .create_api_key(&create_req)
        .await
        .expect("E2E: create_api_key 必须成功");

    // revoke 前可查询
    let before = repo
        .get_api_key_by_id(&created.key.key_id)
        .await
        .expect("E2E: get before revoke 必须成功");
    assert!(before.is_some(), "revoke 前必须能查到 key");

    repo.revoke_api_key(created.key.id)
        .await
        .expect("E2E: revoke_api_key 必须成功");

    // revoke 后查询返回 None
    let after = repo
        .get_api_key_by_id(&created.key.key_id)
        .await
        .expect("E2E: get after revoke 必须成功");
    assert!(
        after.is_none(),
        "revoke 后必须查不到 key，实际得到 {after:?}"
    );

    // revoke 不存在的 id → NotFound
    let missing = repo.revoke_api_key(Uuid::new_v4()).await;
    assert!(
        matches!(missing, Err(CoreError::NotFound(_))),
        "revoke 不存在的 id 必须返回 NotFound，实际 {missing:?}"
    );
}

// -----------------------------------------------------------------------------
// 内存版 ApiKeyRepository（CRUD 测试用）
// -----------------------------------------------------------------------------

/// 内存版 `ApiKeyRepository`（`Mutex<HashMap>` 存储），聚焦 CRUD 完整链路；
/// 与 auth_handlers_e2e_tests.rs::MockApiKeyRepo（聚焦 validate_api_key）互补。
struct CrudApiKeyRepo {
    keys: Mutex<HashMap<Uuid, ApiKeyInfo>>,
    /// key_id → id 的索引，便于 `get_api_key_by_id` 查询
    key_id_index: Mutex<HashMap<String, Uuid>>,
}

impl CrudApiKeyRepo {
    fn new() -> Self {
        Self {
            keys: Mutex::new(HashMap::new()),
            key_id_index: Mutex::new(HashMap::new()),
        }
    }
}

#[async_trait]
impl ApiKeyRepository for CrudApiKeyRepo {
    async fn create_api_key(&self, request: &CreateApiKeyRequest) -> Result<ApiKeyWithSecret> {
        // 模拟 repository.rs 的 prefix 逻辑（Admin → niad_，User → nino_）
        let prefix = match request.role {
            ApiKeyRole::Admin => "niad_",
            ApiKeyRole::User => "nino_",
            ApiKeyRole::Anonymous => {
                return Err(CoreError::InvalidInput(
                    "Anonymous role cannot be persisted to database".to_string(),
                ));
            }
        };

        let full_key_id = if let Some(ref kid) = request.key_id {
            if kid.starts_with(prefix) {
                kid.clone()
            } else {
                format!("{prefix}{kid}")
            }
        } else {
            format!("{}{}", prefix, Uuid::new_v4())
        };

        let key_secret = request
            .key_secret
            .clone()
            .unwrap_or_else(|| format!("mock-secret-{}", Uuid::new_v4()));

        let now = chrono::Utc::now().naive_utc();
        let expires_at = request
            .expires_at
            .or_else(|| now.checked_add_signed(chrono::Duration::days(30)));

        let id = Uuid::new_v4();
        let info = ApiKey {
            id,
            key_id: full_key_id.clone(),
            key_prefix: prefix.to_string(),
            role: request.role.clone(),
            workspace_id: request.workspace_id,
            name: request.name.clone(),
            description: request.description.clone(),
            rate_limit: request.rate_limit.unwrap_or(10000),
            enabled: true,
            expires_at,
            last_used_at: None,
            created_at: now,
        };

        self.keys.lock().unwrap().insert(id, info.clone());
        self.key_id_index
            .lock()
            .unwrap()
            .insert(full_key_id.clone(), id);

        Ok(ApiKeyWithSecret {
            key: ApiKeyResponse {
                id: info.id,
                key_id: info.key_id,
                key_prefix: info.key_prefix,
                name: info.name,
                description: info.description,
                role: info.role,
                rate_limit: info.rate_limit,
                enabled: info.enabled,
                expires_at: info.expires_at,
                created_at: info.created_at,
            },
            key_secret,
            grace_expires_at: None,
        })
    }

    async fn get_api_key_by_id(&self, key_id: &str) -> Result<Option<ApiKeyInfo>> {
        let index = self.key_id_index.lock().unwrap();
        if let Some(&id) = index.get(key_id) {
            let keys = self.keys.lock().unwrap();
            Ok(keys.get(&id).cloned())
        } else {
            Ok(None)
        }
    }

    async fn validate_api_key(
        &self,
        _key_id: &str,
        _key_secret: &str,
    ) -> Result<Option<AuthenticatedKey>> {
        // CRUD 测试不关注 validate
        Ok(None)
    }

    async fn list_api_keys(
        &self,
        workspace_id: Uuid,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> Result<Vec<ApiKeyInfo>> {
        let keys = self.keys.lock().unwrap();
        let mut filtered: Vec<ApiKeyInfo> = keys
            .values()
            .filter(|k| k.workspace_id == Some(workspace_id))
            .cloned()
            .collect();

        // 模拟 offset（按 created_at 排序保证可重复）
        filtered.sort_by_key(|k| k.created_at);

        let offset_val = offset.unwrap_or(0) as usize;
        if offset_val >= filtered.len() {
            return Ok(Vec::new());
        }
        let sliced = &filtered[offset_val..];

        let limit_val = limit.map(|l| l as usize).unwrap_or(sliced.len());
        Ok(sliced.iter().take(limit_val).cloned().collect())
    }

    async fn delete_api_key(&self, id: Uuid) -> Result<()> {
        let mut keys = self.keys.lock().unwrap();
        let removed = keys.remove(&id);
        if removed.is_none() {
            return Err(CoreError::NotFound(format!("API key not found: {id}")));
        }
        // 同步移除 key_id 索引
        let mut index = self.key_id_index.lock().unwrap();
        if let Some(key_id) = removed.map(|k| k.key_id) {
            index.remove(&key_id);
        }
        Ok(())
    }

    async fn revoke_api_key(&self, id: Uuid) -> Result<()> {
        // 对齐 SeaOrmRepository::revoke_api_key：真实仓库软删除，本 mock 硬删除以简化断言。
        self.delete_api_key(id).await
    }

    async fn update_last_used(&self, _id: Uuid) -> Result<()> {
        Ok(())
    }

    async fn get_admin_api_key(&self, _workspace_id: Uuid) -> Result<Option<ApiKeyInfo>> {
        Ok(None)
    }

    async fn count_api_keys(&self, workspace_id: Uuid) -> Result<u64> {
        let keys = self.keys.lock().unwrap();
        Ok(keys
            .values()
            .filter(|k| k.workspace_id == Some(workspace_id))
            .count() as u64)
    }

    /// admin 守卫用：按行主键 `id` 回查 key 行（本 mock 的 `keys` 就以 `id` 为键）。
    async fn find_api_key_by_row_id(&self, id: Uuid) -> Result<Option<ApiKeyInfo>> {
        Ok(self.keys.lock().unwrap().get(&id).cloned())
    }

    /// admin 守卫用：统计启用中的全局 admin key（`role == Admin && enabled`）。
    async fn count_admin_keys(&self) -> Result<u64> {
        let keys = self.keys.lock().unwrap();
        Ok(keys
            .values()
            .filter(|k| k.role == ApiKeyRole::Admin && k.enabled)
            .count() as u64)
    }

    async fn rotate_api_key(
        &self,
        _key_id: &str,
        _grace_period_seconds: u64,
    ) -> Result<ApiKeyWithSecret> {
        Err(CoreError::InternalError(
            "rotate_api_key not implemented in CrudApiKeyRepo".to_string(),
        ))
    }

    async fn get_keys_older_than(&self, _age_threshold_days: i64) -> Result<Vec<ApiKeyInfo>> {
        Ok(Vec::new())
    }
}

// -----------------------------------------------------------------------------
// 录制式 ApiKeyRepository（密钥轮换宽限期测试用）
// -----------------------------------------------------------------------------

/// 录制 rotate_api_key 传入的 `grace_period_seconds`；其他方法返回最小默认值。
struct RecordingApiKeyRepo {
    captured_grace: Mutex<Option<u64>>,
}

impl RecordingApiKeyRepo {
    fn new() -> Self {
        Self {
            captured_grace: Mutex::new(None),
        }
    }

    fn captured_grace_seconds(&self) -> Option<u64> {
        *self.captured_grace.lock().unwrap()
    }
}

#[async_trait]
impl ApiKeyRepository for RecordingApiKeyRepo {
    async fn create_api_key(&self, _request: &CreateApiKeyRequest) -> Result<ApiKeyWithSecret> {
        Ok(ApiKeyWithSecret {
            key: ApiKeyResponse {
                id: Uuid::new_v4(),
                key_id: "mock".to_string(),
                key_prefix: "nino_".to_string(),
                name: "mock".to_string(),
                description: None,
                role: ApiKeyRole::User,
                rate_limit: 1000,
                enabled: true,
                expires_at: None,
                created_at: chrono::Utc::now().naive_utc(),
            },
            key_secret: "mock-secret".to_string(),
            grace_expires_at: None,
        })
    }

    async fn get_api_key_by_id(&self, _key_id: &str) -> Result<Option<ApiKeyInfo>> {
        Ok(None)
    }

    async fn validate_api_key(
        &self,
        _key_id: &str,
        _key_secret: &str,
    ) -> Result<Option<AuthenticatedKey>> {
        Ok(None)
    }

    async fn list_api_keys(
        &self,
        _workspace_id: Uuid,
        _limit: Option<u32>,
        _offset: Option<u32>,
    ) -> Result<Vec<ApiKeyInfo>> {
        Ok(Vec::new())
    }

    async fn delete_api_key(&self, _id: Uuid) -> Result<()> {
        Ok(())
    }

    async fn revoke_api_key(&self, _id: Uuid) -> Result<()> {
        Ok(())
    }

    async fn update_last_used(&self, _id: Uuid) -> Result<()> {
        Ok(())
    }

    async fn get_admin_api_key(&self, _workspace_id: Uuid) -> Result<Option<ApiKeyInfo>> {
        Ok(None)
    }

    async fn count_api_keys(&self, _workspace_id: Uuid) -> Result<u64> {
        Ok(0)
    }

    /// admin 守卫用：本 mock 不保存 key 行，与 get_api_key_by_id 一致的空调用桩。
    async fn find_api_key_by_row_id(&self, _id: Uuid) -> Result<Option<ApiKeyInfo>> {
        Ok(None)
    }

    /// admin 守卫用：本 mock 不保存 key 行，与 count_api_keys 一致的空调用桩。
    async fn count_admin_keys(&self) -> Result<u64> {
        Ok(0)
    }

    async fn rotate_api_key(
        &self,
        key_id: &str,
        grace_period_seconds: u64,
    ) -> Result<ApiKeyWithSecret> {
        *self.captured_grace.lock().unwrap() = Some(grace_period_seconds);

        Ok(ApiKeyWithSecret {
            key: ApiKeyResponse {
                id: Uuid::new_v4(),
                key_id: key_id.to_string(),
                key_prefix: "nino_".to_string(),
                name: "rotated".to_string(),
                description: None,
                role: ApiKeyRole::User,
                rate_limit: 1000,
                enabled: true,
                expires_at: None,
                created_at: chrono::Utc::now().naive_utc(),
            },
            key_secret: "rotated-secret".to_string(),
            grace_expires_at: None,
        })
    }

    async fn get_keys_older_than(&self, _age_threshold_days: i64) -> Result<Vec<ApiKeyInfo>> {
        Ok(Vec::new())
    }
}

// -----------------------------------------------------------------------------
// 最小化 ConfigManagementService mock（ApiHandlers 构造用）
// -----------------------------------------------------------------------------

/// `ConfigManagementService` 最小 mock：所有方法返回默认值，仅满足构造签名。
struct MinimalConfigService;

/// 构造 `AlgorithmConfigInfo`（该 struct 未实现 `Default`，需手动填充字段）。
fn make_algorithm_config_info() -> crate::server::models::AlgorithmConfigInfo {
    use crate::server::models::*;
    AlgorithmConfigInfo {
        default: "segment".to_string(),
        segment: SegmentConfigInfo {
            base_step: 1000,
            min_step: 100,
            max_step: 10000,
            switch_threshold: 0.8,
        },
        snowflake: SnowflakeConfigInfo {
            datacenter_id_bits: 5,
            worker_id_bits: 5,
            sequence_bits: 12,
            clock_drift_threshold_ms: 2000,
        },
        uuid_v8: UuidV8ConfigInfo { enabled: true },
    }
}

#[async_trait]
impl crate::server::config::management::ConfigManagementService for MinimalConfigService {
    fn get_config(&self) -> crate::server::models::ConfigResponse {
        use crate::server::models::*;
        ConfigResponse {
            app: NebulaIdConfigInfo {
                name: "mock".to_string(),
                host: "127.0.0.1".to_string(),
                http_port: 8080,
                grpc_port: 9091,
                dc_id: 1,
                worker_id: 1,
            },
            database: DatabaseConfigInfo {
                engine: "sqlite".to_string(),
                host: None,
                port: None,
                database: None,
                max_connections: 10,
                min_connections: 1,
            },
            algorithm: make_algorithm_config_info(),
            monitoring: MonitoringConfigInfo {
                metrics_enabled: true,
                metrics_path: "/metrics".to_string(),
                tracing_enabled: false,
            },
            logging: LoggingConfigInfo {
                level: "info".to_string(),
                format: "json".to_string(),
                include_location: false,
            },
            rate_limit: RateLimitConfigInfo {
                enabled: true,
                default_rps: 1000,
                burst_size: 100,
            },
            tls: TlsConfigInfo {
                enabled: false,
                http_enabled: false,
                grpc_enabled: false,
                has_cert: false,
            },
        }
    }

    fn get_secure_config(&self) -> crate::server::models::SecureConfigResponse {
        use crate::server::models::*;
        SecureConfigResponse {
            app: NebulaIdConfigInfo {
                name: "mock".to_string(),
                host: "127.0.0.1".to_string(),
                http_port: 8080,
                grpc_port: 9091,
                dc_id: 1,
                worker_id: 1,
            },
            algorithm: make_algorithm_config_info(),
            monitoring: MonitoringConfigInfo {
                metrics_enabled: true,
                metrics_path: "/metrics".to_string(),
                tracing_enabled: false,
            },
            logging: LoggingConfigInfo {
                level: "info".to_string(),
                format: "json".to_string(),
                include_location: false,
            },
            rate_limit: RateLimitConfigInfo {
                enabled: true,
                default_rps: 1000,
                burst_size: 100,
            },
        }
    }

    fn get_batch_max_size(&self) -> u32 {
        1000
    }

    async fn update_rate_limit(
        &self,
        _req: crate::server::models::UpdateRateLimitRequest,
    ) -> crate::server::models::UpdateConfigResponse {
        crate::server::models::UpdateConfigResponse {
            success: true,
            message: "mock".to_string(),
            config: None,
        }
    }

    async fn update_logging(
        &self,
        _req: crate::server::models::UpdateLoggingRequest,
    ) -> crate::server::models::UpdateConfigResponse {
        crate::server::models::UpdateConfigResponse {
            success: true,
            message: "mock".to_string(),
            config: None,
        }
    }

    async fn reload_config(&self) -> crate::server::models::UpdateConfigResponse {
        crate::server::models::UpdateConfigResponse {
            success: true,
            message: "mock".to_string(),
            config: None,
        }
    }

    async fn get_rate_limit_override(&self) -> Option<(u32, u32)> {
        None
    }

    async fn set_algorithm(
        &self,
        _req: crate::server::models::SetAlgorithmRequest,
    ) -> crate::server::models::SetAlgorithmResponse {
        crate::server::models::SetAlgorithmResponse {
            success: true,
            biz_tag: "mock".to_string(),
            algorithm: "segment".to_string(),
            message: "mock".to_string(),
        }
    }

    async fn create_biz_tag(
        &self,
        _request: &crate::core::database::CreateBizTagRequest,
    ) -> crate::core::Result<crate::core::database::BizTag> {
        Err(CoreError::InternalError("mock".to_string()))
    }

    async fn get_biz_tag(
        &self,
        _id: Uuid,
    ) -> crate::core::Result<Option<crate::core::database::BizTag>> {
        Ok(None)
    }

    async fn update_biz_tag(
        &self,
        _id: Uuid,
        _request: &crate::core::database::UpdateBizTagRequest,
    ) -> crate::core::Result<crate::core::database::BizTag> {
        Err(CoreError::InternalError("mock".to_string()))
    }

    async fn delete_biz_tag(&self, _id: Uuid) -> crate::core::Result<()> {
        Ok(())
    }

    async fn count_biz_tags(
        &self,
        _workspace_id: Uuid,
        _group_id: Option<Uuid>,
    ) -> crate::core::Result<u64> {
        Ok(0)
    }

    async fn list_biz_tags(
        &self,
        _workspace_id: Uuid,
        _group_id: Option<Uuid>,
        _limit: Option<u32>,
        _offset: Option<u32>,
    ) -> crate::core::Result<Vec<crate::core::database::BizTag>> {
        Ok(Vec::new())
    }

    async fn create_workspace(
        &self,
        _req: crate::server::models::CreateWorkspaceRequest,
    ) -> crate::core::Result<crate::server::models::WorkspaceResponse> {
        Err(CoreError::InternalError("mock".to_string()))
    }

    async fn list_workspaces(
        &self,
    ) -> crate::core::Result<crate::server::models::WorkspaceListResponse> {
        Ok(crate::server::models::WorkspaceListResponse {
            workspaces: Vec::new(),
            total: 0,
        })
    }

    async fn get_workspace(
        &self,
        _name: &str,
    ) -> crate::core::Result<Option<crate::server::models::WorkspaceResponse>> {
        Ok(None)
    }

    async fn create_group(
        &self,
        _req: crate::server::models::CreateGroupRequest,
    ) -> crate::core::Result<crate::server::models::GroupResponse> {
        Err(CoreError::InternalError("mock".to_string()))
    }

    async fn list_groups(
        &self,
        _workspace: &str,
    ) -> crate::core::Result<crate::server::models::GroupListResponse> {
        Ok(crate::server::models::GroupListResponse {
            groups: Vec::new(),
            total: 0,
        })
    }

    async fn get_database_metrics(&self) -> crate::server::models::DatabaseMetrics {
        use crate::server::models::{ConnectionPoolMetrics, DatabaseMetrics, HealthStatus};
        DatabaseMetrics {
            status: HealthStatus::Healthy,
            connection_pool: ConnectionPoolMetrics {
                active_connections: 0,
                idle_connections: 0,
                max_connections: 10,
            },
            last_error: None,
        }
    }

    async fn get_cache_metrics(&self) -> crate::server::models::CacheMetrics {
        use crate::server::models::{CacheMetrics, HealthStatus};
        CacheMetrics {
            status: HealthStatus::Healthy,
            hit_rate: 0.0,
            has_cache: false,
            memory_usage_mb: None,
            key_count: None,
        }
    }

    async fn get_algorithm_metrics(
        &self,
    ) -> Vec<(
        crate::core::types::AlgorithmType,
        crate::core::algorithm::AlgorithmMetricsSnapshot,
    )> {
        Vec::new()
    }
}

// =============================================================================
// 5. 告警通知（AlertManager）
// =============================================================================

/// AlertManager::add_rule / remove_rule：加规则后 get_state 可查，移除后返回 None。
#[tokio::test]
async fn e2e_alert_manager_add_and_remove_rule() {
    let (manager, _rx) = build_alert_manager();

    let rule_name = "e2e_test_rule".to_string();
    let rule = AlertRule::new(
        rule_name.clone(),
        "latency_p99 > 200".to_string(),
        AlertSeverity::Warning,
    );

    // add_rule 后能查到状态
    manager.add_rule(rule);
    let state = manager.get_state(&rule_name);
    assert!(
        state.is_some(),
        "add_rule 后必须能通过 get_state 查到对应状态"
    );

    // remove_rule 后状态被清理
    manager.remove_rule(&rule_name);
    let state_after = manager.get_state(&rule_name);
    assert!(
        state_after.is_none(),
        "remove_rule 后 get_state 必须返回 None"
    );
}

/// update_config 替换整个配置；新 rule 需经 add_rule 才会创建状态（实际行为）。
#[tokio::test]
async fn e2e_alert_manager_update_config() {
    let (mut manager, _rx) = build_alert_manager();

    // 初始 config 包含一条 rule（build_alert_manager 中已添加 initial_rule）
    let initial_state = manager.get_state("initial_rule");
    assert!(
        initial_state.is_some(),
        "初始 config 必须包含 initial_rule 的状态"
    );

    // update_config 替换为新 config（只包含 new_rule）
    let mut new_rules_labels = HashMap::new();
    new_rules_labels.insert("env".to_string(), "test".to_string());
    let new_rule = AlertRule {
        name: "new_rule".to_string(),
        expression: "id_generation_qps > 1000 100".to_string(),
        for_duration: 30,
        severity: AlertSeverity::Critical,
        labels: new_rules_labels,
        annotations: HashMap::new(),
        enabled: true,
        description: "new rule for e2e".to_string(),
    };
    let new_config = AlertingConfig {
        enabled: true,
        evaluation_interval_ms: 500,
        rules: vec![new_rule],
        channels: Vec::new(),
        global_labels: HashMap::new(),
    };
    manager.update_config(new_config);

    // update_config 仅替换 config、不自动为新 rule 建 state（实际行为）；
    // 这里经 add_rule 验证替换后的 new_rule 可创建状态。
    manager.add_rule(AlertRule {
        name: "new_rule".to_string(),
        expression: "id_generation_qps > 1000 100".to_string(),
        for_duration: 30,
        severity: AlertSeverity::Critical,
        labels: HashMap::new(),
        annotations: HashMap::new(),
        enabled: true,
        description: "re-added".to_string(),
    });
    let new_state = manager.get_state("new_rule");
    assert!(
        new_state.is_some(),
        "update_config 后新 rule 必须能通过 add_rule 创建状态"
    );
}

/// AlertManager::get_alerts / get_alert_count：初始为空，clear_alert_history 清空。
#[tokio::test]
async fn e2e_alert_manager_query_alerts() {
    let (manager, _rx) = build_alert_manager();

    let alerts = manager.get_alerts();
    assert!(
        alerts.is_empty(),
        "初始状态告警历史必须为空，实际 {} 条",
        alerts.len()
    );
    assert_eq!(manager.get_alert_count(), 0, "初始状态告警计数必须为 0");

    // get_recent_alerts 也应返回空
    let recent = manager.get_recent_alerts(10);
    assert!(recent.is_empty(), "初始状态 recent_alerts 必须为空");

    // get_alerts_by_severity 各级别都应返回空
    let critical = manager.get_alerts_by_severity(AlertSeverity::Critical);
    let warning = manager.get_alerts_by_severity(AlertSeverity::Warning);
    let info = manager.get_alerts_by_severity(AlertSeverity::Info);
    assert!(critical.is_empty(), "初始状态 Critical 告警必须为空");
    assert!(warning.is_empty(), "初始状态 Warning 告警必须为空");
    assert!(info.is_empty(), "初始状态 Info 告警必须为空");

    // clear_alert_history 是幂等操作（空历史也合法）
    manager.clear_alert_history();
    assert_eq!(manager.get_alert_count(), 0, "clear 后告警计数仍必须为 0");
}

// -----------------------------------------------------------------------------
// 辅助：构造 AlertManager
// -----------------------------------------------------------------------------

/// 构造带 Log channel + 初始 rule 的 AlertManager（receiver 须保留，防 broadcast 通道关闭）。
fn build_alert_manager() -> (
    AlertManager,
    tokio::sync::broadcast::Receiver<crate::core::monitoring::Alert>,
) {
    let metrics = Arc::new(GlobalMetrics::new());

    let channels: Vec<NotificationChannel> = vec![NotificationChannel {
        name: "log".to_string(),
        channel_type: crate::core::monitoring::ChannelType::Log,
        config: HashMap::new(),
        enabled: true,
    }];
    let sender = Arc::new(AlertNotificationSender::new(channels));

    let initial_rule = AlertRule {
        name: "initial_rule".to_string(),
        expression: "latency_p99 > 100".to_string(),
        for_duration: 60,
        severity: AlertSeverity::Warning,
        labels: HashMap::new(),
        annotations: HashMap::new(),
        enabled: true,
        description: "initial rule".to_string(),
    };

    let config = AlertingConfig {
        enabled: true,
        evaluation_interval_ms: 1000,
        rules: vec![initial_rule],
        channels: Vec::new(),
        global_labels: HashMap::new(),
    };

    AlertManager::new(config, metrics, sender)
}
