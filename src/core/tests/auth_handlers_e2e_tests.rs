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

#![cfg(test)]

//! # 认证与请求验证端到端测试（auth handlers e2e tests）
//!
//! 本文件覆盖 `temp/功能场景穷举分析.md` 第 2.1 节（认证）和
//! 第 3.1/3.7 节（认证中间件 + HTTP 处理器验证）的端到端场景。
//!
//! ## 测试分组
//!
//! - **E2E-AUTH 组**（认证中间件 e2e）：Basic / ApiKey 头解析、
//!   缺失头、格式错误、空凭证、错误密钥、禁用认证注入 Anonymous、
//!   失败速率限制（10 次失败后 429、<10 次放行）
//! - **E2E-ADMIN 组**（Admin 权限检查 e2e）：Admin 放行、User 拒绝、
//!   无 ApiKeyRole 扩展 fail-closed
//! - **E2E-VAL 组**（请求验证 e2e）：GenerateRequest / BatchGenerateRequest
//!   字段长度与范围边界
//!
//! ## 与现有单元测试的区别
//!
//! `api_key_auth.rs` 内的 `#[cfg(test)] mod tests` 聚焦「函数孤立行为」
//! （如 `validate_key` 单次调用、`auth_middleware` 单次请求）。本文件
//! 聚焦「真实 HTTP 流量下的端到端行为」：用 `axum::Router` +
//! `tower::ServiceExt::oneshot` 构造完整请求链路，验证中间件 layer 顺序
//! 组合（auth → admin）与请求模型验证的协同。
//!
//! ## 并行安全
//!
//! 所有测试用独立的 `MockApiKeyRepo` 实例和独立的 `ApiKeyAuth` 实例，
//! `auth_failures` 状态在 `Arc<RwLock<HashMap>>` 中按测试隔离，无共享
//! 状态竞争。

use std::sync::Arc;

use async_trait::async_trait;
use axum::{
    body::Body,
    http::{Request, StatusCode},
    middleware::{from_fn, from_fn_with_state},
    routing::get,
    Router,
};
use base64::Engine;
use sdforge::tower::ServiceExt;
use sha2::Digest;
use uuid::Uuid;
use validator::Validate;

use crate::core::database::{
    ApiKeyInfo, ApiKeyRepository, ApiKeyResponse, ApiKeyRole, ApiKeyWithSecret, AuthenticatedKey,
    CreateApiKeyRequest,
};
use crate::core::types::Result;
use crate::server::middleware::api_key_auth::{
    admin_required_middleware, auth_middleware_fn, ApiKeyAuth,
};
use crate::server::models::{BatchGenerateRequest, GenerateRequest};

// ============================================================================
// MockApiKeyRepo —— 参考 api_key_auth.rs 测试中的 mock 实现
// ============================================================================

/// 内存版 `ApiKeyRepository`，用 sha256 哈希存储密钥（与真实仓库的
/// Argon2id 不同，但足够测试认证中间件的逻辑分支）。
#[derive(Clone)]
struct MockApiKeyRepo {
    keys: std::collections::HashMap<String, (String, ApiKeyRole)>,
}

impl MockApiKeyRepo {
    /// 用 sha256 哈希密钥，模拟仓库侧的密钥存储格式。
    fn hash_secret(secret: &str) -> String {
        let mut hasher = sha2::Sha256::default();
        hasher.update(secret);
        hex::encode(hasher.finalize())
    }
}

#[async_trait]
impl ApiKeyRepository for MockApiKeyRepo {
    async fn create_api_key(&self, _request: &CreateApiKeyRequest) -> Result<ApiKeyWithSecret> {
        Ok(ApiKeyWithSecret {
            key: ApiKeyResponse {
                id: Uuid::new_v4(),
                key_id: "mock_key_id".to_string(),
                key_prefix: "nino_".to_string(),
                name: "Mock Key".to_string(),
                description: None,
                role: ApiKeyRole::User,
                rate_limit: 10000,
                enabled: true,
                expires_at: None,
                created_at: chrono::Utc::now().naive_utc(),
            },
            key_secret: "mock_secret".to_string(),
            grace_expires_at: None,
        })
    }

    async fn get_api_key_by_id(&self, _key_id: &str) -> Result<Option<ApiKeyInfo>> {
        Ok(None)
    }

    async fn validate_api_key(
        &self,
        key_id: &str,
        key_secret: &str,
    ) -> Result<Option<AuthenticatedKey>> {
        use subtle::ConstantTimeEq;
        if let Some((expected_secret, role)) = self.keys.get(key_id) {
            let incoming_hash = MockApiKeyRepo::hash_secret(key_secret);
            // 常数时间比较，防止时序侧信道
            if expected_secret
                .as_bytes()
                .ct_eq(incoming_hash.as_bytes())
                .into()
            {
                // Admin 密钥无 workspace_id，User 密钥绑定到 Uuid::nil()
                let workspace_id = if *role == ApiKeyRole::Admin {
                    None
                } else {
                    Some(Uuid::nil())
                };
                return Ok(Some(AuthenticatedKey {
                    workspace_id,
                    role: role.clone(),
                    used_previous_credential: false,
                }));
            }
        }
        Ok(None)
    }

    async fn list_api_keys(
        &self,
        _workspace_id: Uuid,
        _limit: Option<u32>,
        _offset: Option<u32>,
    ) -> Result<Vec<ApiKeyInfo>> {
        Ok(vec![])
    }

    async fn delete_api_key(&self, _id: Uuid) -> Result<()> {
        Ok(())
    }

    async fn revoke_api_key(&self, _id: Uuid) -> Result<()> {
        Ok(())
    }

    async fn update_last_used(&self, _key: Uuid) -> Result<()> {
        Ok(())
    }

    async fn get_admin_api_key(&self, _workspace_id: Uuid) -> Result<Option<ApiKeyInfo>> {
        Ok(None)
    }

    async fn count_api_keys(&self, _workspace_id: Uuid) -> Result<u64> {
        Ok(0)
    }

    /// admin 守卫用：本 mock 只存 key_id→(hash, role)，无行主键也无 enabled，恒查不到行。
    async fn find_api_key_by_row_id(&self, _id: Uuid) -> Result<Option<ApiKeyInfo>> {
        Ok(None)
    }

    /// admin 守卫用：本 mock 不建模 key 行，与 count_api_keys 一致返回 0。
    async fn count_admin_keys(&self) -> Result<u64> {
        Ok(0)
    }

    async fn rotate_api_key(
        &self,
        _key_id: &str,
        _grace_period_seconds: u64,
    ) -> Result<ApiKeyWithSecret> {
        Err(crate::core::types::error::CoreError::InternalError(
            "rotate_api_key not implemented in mock".to_string(),
        ))
    }

    async fn get_keys_older_than(&self, _age_threshold_days: i64) -> Result<Vec<ApiKeyInfo>> {
        Ok(vec![])
    }
}

// ============================================================================
// 辅助函数
// ============================================================================

/// 构造一个预置了 user-key 和 admin-key 的 mock 仓库。
fn make_mock_repo() -> MockApiKeyRepo {
    let mut mock_keys = std::collections::HashMap::new();
    mock_keys.insert(
        "user-key".to_string(),
        (MockApiKeyRepo::hash_secret("user-secret"), ApiKeyRole::User),
    );
    mock_keys.insert(
        "admin-key".to_string(),
        (
            MockApiKeyRepo::hash_secret("admin-secret"),
            ApiKeyRole::Admin,
        ),
    );
    MockApiKeyRepo { keys: mock_keys }
}

/// 构造挂载了 auth middleware 的测试 Router，handler 返回 200 OK。
fn build_auth_router(auth: Arc<ApiKeyAuth>) -> Router {
    Router::new()
        .route("/test", get(|| async { "ok" }))
        .layer(from_fn_with_state(auth, auth_middleware_fn))
}

/// 构造挂载了 auth middleware 且 handler 回显注入的 ApiKeyRole 的 Router，
/// 用于验证 auth middleware 注入的角色扩展。
fn build_role_check_router(auth: Arc<ApiKeyAuth>) -> Router {
    Router::new()
        .route(
            "/test",
            get(|request: Request<Body>| async move {
                if let Some(role) = request.extensions().get::<ApiKeyRole>() {
                    format!("{:?}", role)
                } else {
                    "no-role".to_string()
                }
            }),
        )
        .layer(from_fn_with_state(auth, auth_middleware_fn))
}

/// 构造挂载了 admin_required_middleware 的 Router（无 auth），
/// 用于隔离测试 admin 权限检查。
fn build_admin_router() -> Router {
    Router::new()
        .route("/test", get(|| async { "ok" }))
        .layer(from_fn(admin_required_middleware))
}

/// 构造 Basic 认证头：`Basic base64(key_id:key_secret)`。
fn basic_auth_header(key_id: &str, key_secret: &str) -> String {
    let credentials = format!("{}:{}", key_id, key_secret);
    let encoded = base64::engine::general_purpose::STANDARD.encode(credentials);
    format!("Basic {}", encoded)
}

/// 构造 ApiKey 认证头：`ApiKey key_id:key_secret`。
fn api_key_header(key_id: &str, key_secret: &str) -> String {
    format!("ApiKey {}:{}", key_id, key_secret)
}

/// 构造一个 GET /test 请求，可选携带 Authorization 头。
fn make_request(auth_header: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder().uri("/test").method("GET");
    if let Some(value) = auth_header {
        builder = builder.header("authorization", value);
    }
    builder.body(Body::empty()).unwrap()
}

/// 构造一个携带指定 ApiKeyRole 扩展的 GET /test 请求。
fn make_request_with_role(role: ApiKeyRole) -> Request<Body> {
    Request::builder()
        .uri("/test")
        .method("GET")
        .extension(role)
        .body(Body::empty())
        .unwrap()
}

/// 读取响应 body 为字符串（用 axum 0.8 内置的 `to_bytes`）。
async fn read_body_to_string(body: Body) -> String {
    let bytes = axum::body::to_bytes(body, usize::MAX)
        .await
        .expect("failed to read response body");
    String::from_utf8(bytes.to_vec()).expect("response body is not valid UTF-8")
}

// ============================================================================
// E2E-AUTH 组：认证中间件 e2e
// ============================================================================

// ----------------------------------------------------------------------------
// E2E-AUTH-001: Basic base64(key_id:key_secret) 头 → 200
// ----------------------------------------------------------------------------

#[tokio::test]
async fn e2e_auth_middleware_basic_auth_success() {
    let repo = Arc::new(make_mock_repo()) as Arc<dyn ApiKeyRepository>;
    let auth = Arc::new(ApiKeyAuth::new(repo, true));
    let router = build_auth_router(auth);

    let header = basic_auth_header("user-key", "user-secret");
    let resp = router.oneshot(make_request(Some(&header))).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
}

// ----------------------------------------------------------------------------
// E2E-AUTH-002: ApiKey key_id:key_secret 头 → 200
// ----------------------------------------------------------------------------

#[tokio::test]
async fn e2e_auth_middleware_api_key_auth_success() {
    let repo = Arc::new(make_mock_repo()) as Arc<dyn ApiKeyRepository>;
    let auth = Arc::new(ApiKeyAuth::new(repo, true));
    let router = build_auth_router(auth);

    let header = api_key_header("user-key", "user-secret");
    let resp = router.oneshot(make_request(Some(&header))).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
}

// ----------------------------------------------------------------------------
// E2E-AUTH-003: 无 Authorization 头 → 401
// ----------------------------------------------------------------------------

#[tokio::test]
async fn e2e_auth_middleware_missing_authorization_returns_401() {
    let repo = Arc::new(make_mock_repo()) as Arc<dyn ApiKeyRepository>;
    let auth = Arc::new(ApiKeyAuth::new(repo, true));
    let router = build_auth_router(auth);

    let resp = router.oneshot(make_request(None)).await.unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

// ----------------------------------------------------------------------------
// E2E-AUTH-004: 格式错误的 Basic 头（非法 base64）→ 401
// ----------------------------------------------------------------------------

#[tokio::test]
async fn e2e_auth_middleware_invalid_base64_returns_401() {
    let repo = Arc::new(make_mock_repo()) as Arc<dyn ApiKeyRepository>;
    let auth = Arc::new(ApiKeyAuth::new(repo, true));
    let router = build_auth_router(auth);

    // "Basic !!!" 不是合法的 base64
    let resp = router
        .oneshot(make_request(Some("Basic !!!not-base64!!!")))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

// ----------------------------------------------------------------------------
// E2E-AUTH-005: 空 key_id:key_secret → 401
// ----------------------------------------------------------------------------

#[tokio::test]
async fn e2e_auth_middleware_empty_credentials_returns_401() {
    let repo = Arc::new(make_mock_repo()) as Arc<dyn ApiKeyRepository>;
    let auth = Arc::new(ApiKeyAuth::new(repo, true));
    let router = build_auth_router(auth);

    // base64(":") 编码后是空 key_id 与空 key_secret
    let header = basic_auth_header("", "");
    let resp = router.oneshot(make_request(Some(&header))).await.unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

// ----------------------------------------------------------------------------
// E2E-AUTH-006: 错误密钥 → 401
// ----------------------------------------------------------------------------

#[tokio::test]
async fn e2e_auth_middleware_wrong_secret_returns_401() {
    let repo = Arc::new(make_mock_repo()) as Arc<dyn ApiKeyRepository>;
    let auth = Arc::new(ApiKeyAuth::new(repo, true));
    let router = build_auth_router(auth);

    let header = basic_auth_header("user-key", "wrong-secret");
    let resp = router.oneshot(make_request(Some(&header))).await.unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

// ----------------------------------------------------------------------------
// E2E-AUTH-007: 认证禁用时注入 Anonymous 角色
// ----------------------------------------------------------------------------

#[tokio::test]
async fn e2e_auth_middleware_disabled_injects_anonymous_role() {
    let repo = Arc::new(make_mock_repo()) as Arc<dyn ApiKeyRepository>;
    // enabled = false → 走禁用分支，注入 Anonymous
    let auth = Arc::new(ApiKeyAuth::new(repo, false));
    let router = build_role_check_router(auth);

    let resp = router.oneshot(make_request(None)).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = read_body_to_string(resp.into_body()).await;
    assert_eq!(body, "Anonymous");
}

// ----------------------------------------------------------------------------
// E2E-AUTH-008: 5 分钟内 10 次失败后 → 429
// ----------------------------------------------------------------------------

#[tokio::test]
async fn e2e_auth_failure_rate_blocks_after_10_failures() {
    let repo = Arc::new(make_mock_repo()) as Arc<dyn ApiKeyRepository>;
    let auth = Arc::new(ApiKeyAuth::new(repo, true));
    let router = build_auth_router(auth);

    // 连续发送 10 次错误密钥请求触发失败计数（每次返回 401）
    let bad_header = basic_auth_header("user-key", "wrong");
    for _ in 0..10 {
        let resp = router
            .clone()
            .oneshot(make_request(Some(&bad_header)))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    // 第 11 次请求应被速率限制为 429
    let resp = router
        .oneshot(make_request(Some(&bad_header)))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
}

// ----------------------------------------------------------------------------
// E2E-AUTH-009: 窗口未达阈值时放行（<10 次失败仍允许）
// ----------------------------------------------------------------------------

#[tokio::test]
async fn e2e_auth_failure_rate_allows_after_window_expires() {
    // 由于 `check_auth_failure_rate` 使用 `Instant::now()` 且无法注入
    // mock 时间，这里采用「<10 次失败仍放行」的策略验证窗口边界：
    // 9 次失败后，第 10 次请求仍应返回 401（而非 429），
    // 表明失败窗口未满 10 次时不会误阻断。
    let repo = Arc::new(make_mock_repo()) as Arc<dyn ApiKeyRepository>;
    let auth = Arc::new(ApiKeyAuth::new(repo, true));
    let router = build_auth_router(auth);

    let bad_header = basic_auth_header("user-key", "wrong");
    // 发送 9 次失败请求
    for _ in 0..9 {
        let resp = router
            .clone()
            .oneshot(make_request(Some(&bad_header)))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    // 第 10 次请求：失败计数为 9，未达阈值 10，应仍返回 401（非 429）
    let resp = router
        .oneshot(make_request(Some(&bad_header)))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "9 次失败后第 10 次请求不应被 429 阻断"
    );

    // 补充验证：失败计数 <10 时，有效凭证仍能成功认证
    let repo2 = Arc::new(make_mock_repo()) as Arc<dyn ApiKeyRepository>;
    let auth2 = Arc::new(ApiKeyAuth::new(repo2, true));
    let router2 = build_auth_router(auth2);
    // 预先制造 5 次失败
    for _ in 0..5 {
        let _ = router2
            .clone()
            .oneshot(make_request(Some(&bad_header)))
            .await
            .unwrap();
    }
    // 用有效凭证请求应返回 200
    let good_header = basic_auth_header("user-key", "user-secret");
    let resp = router2
        .oneshot(make_request(Some(&good_header)))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

// ============================================================================
// E2E-ADMIN 组：Admin 权限检查 e2e
// ============================================================================

// ----------------------------------------------------------------------------
// E2E-ADMIN-001: Admin 角色放行 → 200
// ----------------------------------------------------------------------------

#[tokio::test]
async fn e2e_admin_required_allows_admin_role() {
    let router = build_admin_router();

    let resp = router
        .oneshot(make_request_with_role(ApiKeyRole::Admin))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
}

// ----------------------------------------------------------------------------
// E2E-ADMIN-002: User 角色拒绝 → 403
// ----------------------------------------------------------------------------

#[tokio::test]
async fn e2e_admin_required_rejects_user_role() {
    let router = build_admin_router();

    let resp = router
        .oneshot(make_request_with_role(ApiKeyRole::User))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

// ----------------------------------------------------------------------------
// E2E-ADMIN-003: 无 ApiKeyRole 扩展 → 403（fail-closed）
// ----------------------------------------------------------------------------

#[tokio::test]
async fn e2e_admin_required_rejects_missing_extension() {
    let router = build_admin_router();

    // 不注入 ApiKeyRole 扩展
    let resp = router.oneshot(make_request(None)).await.unwrap();

    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

// ============================================================================
// E2E-VAL 组：请求验证 e2e
// ============================================================================

// ----------------------------------------------------------------------------
// E2E-VAL-001: GenerateRequest workspace 65 字符 → 验证失败
// ----------------------------------------------------------------------------

#[test]
fn e2e_generate_request_validates_workspace_length_65_fails() {
    // max = 64，65 字符应超出上限
    let workspace = "a".repeat(65);
    let request = GenerateRequest {
        workspace,
        group: "g".to_string(),
        biz_tag: "tag".to_string(),
        algorithm: None,
    };

    let result = Validate::validate(&request);
    assert!(
        result.is_err(),
        "workspace 长度 65 应超过 max=64 上限，验证必须失败"
    );
}

// ----------------------------------------------------------------------------
// E2E-VAL-002: GenerateRequest workspace 空 → 验证失败
// ----------------------------------------------------------------------------

#[test]
fn e2e_generate_request_validates_workspace_empty_fails() {
    // min = 1，空字符串应低于下限
    let request = GenerateRequest {
        workspace: String::new(),
        group: "g".to_string(),
        biz_tag: "tag".to_string(),
        algorithm: None,
    };

    let result = Validate::validate(&request);
    assert!(
        result.is_err(),
        "workspace 为空应低于 min=1 下限，验证必须失败"
    );
}

// ----------------------------------------------------------------------------
// E2E-VAL-003: GenerateRequest algorithm 21 字符 → 验证失败
// ----------------------------------------------------------------------------

#[test]
fn e2e_generate_request_validates_algorithm_length_21_fails() {
    // algorithm 上限 max=20，21 字符应超出
    let algorithm = "a".repeat(21);
    let request = GenerateRequest {
        workspace: "ws".to_string(),
        group: "g".to_string(),
        biz_tag: "tag".to_string(),
        algorithm: Some(algorithm),
    };

    let result = Validate::validate(&request);
    assert!(
        result.is_err(),
        "algorithm 长度 21 应超过 max=20 上限，验证必须失败"
    );
}

// ----------------------------------------------------------------------------
// E2E-VAL-004: BatchGenerateRequest size=0 → 验证失败
// ----------------------------------------------------------------------------

#[test]
fn e2e_batch_generate_request_validates_size_zero_fails() {
    // size 范围 [1, 100]，0 应低于下限
    let request = BatchGenerateRequest {
        workspace: "ws".to_string(),
        group: "g".to_string(),
        biz_tag: "tag".to_string(),
        size: Some(0),
        algorithm: None,
    };

    let result = Validate::validate(&request);
    assert!(result.is_err(), "size=0 应低于 min=1 下限，验证必须失败");
}

// ----------------------------------------------------------------------------
// E2E-VAL-005: BatchGenerateRequest 上限迁移（T012）
// ----------------------------------------------------------------------------

#[test]
fn e2e_batch_generate_request_validates_size_101_passes_struct_validation() {
    // T012：结构校验仅保留 min=1；max 由 config.batch_generate.max_batch_size
    // （默认 100）在 handler 层运行时校验。此处断言 101 通过结构校验，
    // 超限拒绝行为由 id_handlers/grpc 的边界测试覆盖。
    let request = BatchGenerateRequest {
        workspace: "ws".to_string(),
        group: "g".to_string(),
        biz_tag: "tag".to_string(),
        size: Some(101),
        algorithm: None,
    };

    let result = Validate::validate(&request);
    assert!(
        result.is_ok(),
        "size=101 应通过结构校验（上限已迁移至配置层）"
    );
}

// ----------------------------------------------------------------------------
// E2E-VAL-006: BatchGenerateRequest size=1 和 size=100 → 验证通过
// ----------------------------------------------------------------------------

#[test]
fn e2e_batch_generate_request_validates_size_boundary_1_and_100_pass() {
    // 边界值 size=1（下限）应通过
    let request_min = BatchGenerateRequest {
        workspace: "ws".to_string(),
        group: "g".to_string(),
        biz_tag: "tag".to_string(),
        size: Some(1),
        algorithm: None,
    };
    let result_min = Validate::validate(&request_min);
    assert!(result_min.is_ok(), "size=1 是合法下限边界，验证应通过");

    // 边界值 size=100（上限）应通过
    let request_max = BatchGenerateRequest {
        workspace: "ws".to_string(),
        group: "g".to_string(),
        biz_tag: "tag".to_string(),
        size: Some(100),
        algorithm: None,
    };
    let result_max = Validate::validate(&request_max);
    assert!(result_max.is_ok(), "size=100 是合法上限边界，验证应通过");
}

// ============================================================================
// wiring T007: GET /api/v1/biz-tags 租户隔离（CWE-639 / IDOR）
//
// 背景：handler 曾把 `None` workspace 透传给分页查询，底层回退为 nil UUID
// 过滤 —— 既拿不到本租户数据（功能失效），又会匹配 workspace_id 为 nil 的
// 脏数据行（越权泄露）。本组测试走真实 create_router 全栈（认证中间件 +
// Anonymous 拦截 + 限流 + 审计），断言过滤键只能来自认证身份或显式参数。
// ============================================================================

mod biz_tag_tenant_isolation {
    use super::*;
    use crate::core::algorithm::AlgorithmRouter;
    use crate::core::config::Config;
    use crate::core::database::{BizTag, IdFormat};
    use crate::core::types::AlgorithmType;
    use crate::server::audit::AuditLogger;
    use crate::server::config::management::ConfigManagementService;
    use crate::server::handlers::mock_tests::{MockApiKeyRepository, MockConfigManagementService};
    use crate::server::handlers::ApiHandlers;
    use crate::server::rate_limit::limiter::RateLimiter;
    use crate::server::router::create_router;

    fn workspace_a() -> Uuid {
        Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap()
    }

    fn workspace_b() -> Uuid {
        Uuid::parse_str("22222222-2222-2222-2222-222222222222").unwrap()
    }

    fn make_tag(workspace_id: Uuid, name: &str) -> BizTag {
        BizTag {
            id: Uuid::new_v4(),
            workspace_id,
            group_id: Uuid::new_v4(),
            name: name.to_string(),
            description: None,
            algorithm: AlgorithmType::Segment,
            format: IdFormat::Numeric,
            prefix: "p_".to_string(),
            base_step: 100,
            max_step: 1000,
            datacenter_ids: vec![0],
            created_at: chrono::Utc::now().naive_utc(),
            updated_at: chrono::Utc::now().naive_utc(),
        }
    }

    /// 按 workspace 精确过滤的内存仓储桩（模拟真实 `WHERE workspace_id = ?`）。
    fn make_config_service() -> MockConfigManagementService {
        let all_tags: Vec<BizTag> = vec![
            make_tag(workspace_a(), "tag-a1"),
            make_tag(workspace_a(), "tag-a2"),
            make_tag(workspace_b(), "tag-b1"),
        ];
        let mut svc = MockConfigManagementService::new();
        svc.expect_list_biz_tags()
            .returning(move |workspace_id, _, _, _| {
                Ok(all_tags
                    .iter()
                    .filter(|tag| tag.workspace_id == workspace_id)
                    .cloned()
                    .collect())
            });
        svc.expect_count_biz_tags()
            .returning(move |workspace_id, _| {
                Ok((workspace_id == workspace_a()) as u64 * 2
                    + (workspace_id == workspace_b()) as u64)
            });
        svc
    }

    /// user-a → workspace A 的 User key；admin-k → 无租户绑定的 Admin key。
    fn make_auth() -> Arc<ApiKeyAuth> {
        let mut repo = MockApiKeyRepository::new();
        repo.expect_validate_api_key().returning(move |key_id, _| {
            Ok(match key_id {
                "user-a" => Some(AuthenticatedKey {
                    workspace_id: Some(workspace_a()),
                    role: ApiKeyRole::User,
                    used_previous_credential: false,
                }),
                "admin-k" => Some(AuthenticatedKey {
                    workspace_id: None,
                    role: ApiKeyRole::Admin,
                    used_previous_credential: false,
                }),
                _ => None,
            })
        });
        Arc::new(ApiKeyAuth::new(Arc::new(repo), true))
    }

    async fn build_app() -> Router {
        build_app_with(make_auth(), Arc::new(make_config_service())).await
    }

    /// 用指定 auth / config service 组装真实 `create_router` 全栈
    /// （认证中间件 + Anonymous 拦截 + 限流 + 审计）。
    ///
    /// `pub(super)`：同文件的 Anonymous 端点组需要以「认证禁用」的 auth 复用
    /// 同一套全栈装配，避免复制路由代码。
    pub(super) async fn build_app_with(
        auth: Arc<ApiKeyAuth>,
        config_service: Arc<dyn ConfigManagementService>,
    ) -> Router {
        let config = Config::default();
        let alg_router = Arc::new(AlgorithmRouter::new(config, None));
        let handlers = Arc::new(ApiHandlers::new(alg_router, config_service));
        // 宽松限流：本组测试只关心认证与租户过滤，不应被 429 干扰
        let rate_limiter = Arc::new(RateLimiter::new(10_000, 10_000));
        let audit_logger = Arc::new(AuditLogger::new(10));
        create_router(handlers, auth, rate_limiter, audit_logger).await
    }

    async fn list_biz_tags(uri: &str, key_id: &str) -> (StatusCode, Vec<String>) {
        let app = build_app().await;
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(uri)
                    .header("authorization", basic_auth_header(key_id, "ignored-secret"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = resp.status();
        let body = read_body_to_string(resp.into_body()).await;
        let workspaces: Vec<String> = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|v| v["biz_tags"].as_array().cloned())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item["workspace_id"].as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        (status, workspaces)
    }

    /// User key 不带参数：只能看到本租户 workspace A 的两个 tag。
    #[tokio::test]
    async fn user_key_lists_only_own_workspace() {
        let (status, workspaces) = list_biz_tags("/api/v1/biz-tags", "user-a").await;
        assert_eq!(status, StatusCode::OK, "合法 user key 应返回 200");
        assert_eq!(
            workspaces,
            vec![workspace_a().to_string(); 2],
            "响应必须且只能是 workspace A 的 tag"
        );
    }

    /// User key 伪造 ?workspace_id=B：参数被忽略，结果仍限定在 A。
    #[tokio::test]
    async fn user_key_cannot_override_workspace_via_query() {
        let uri = format!("/api/v1/biz-tags?workspace_id={}", workspace_b());
        let (status, workspaces) = list_biz_tags(&uri, "user-a").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            workspaces,
            vec![workspace_a().to_string(); 2],
            "越权 workspace 参数必须被忽略（IDOR 防护）"
        );
        assert!(
            !workspaces.contains(&workspace_b().to_string()),
            "响应不得含 workspace B 的 tag"
        );
    }

    /// Admin key 显式指定 workspace B：按参数过滤，返回 B 的 tag。
    #[tokio::test]
    async fn admin_key_filters_by_explicit_workspace() {
        let uri = format!("/api/v1/biz-tags?workspace_id={}", workspace_b());
        let (status, workspaces) = list_biz_tags(&uri, "admin-k").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            workspaces,
            vec![workspace_b().to_string()],
            "Admin 按参数过滤时应返回 workspace B 的 tag"
        );
    }

    /// Admin key 不带 workspace 参数：显式 400，不静默回退 nil 全量扫描。
    #[tokio::test]
    async fn admin_key_without_workspace_is_rejected() {
        let (status, workspaces) = list_biz_tags("/api/v1/biz-tags", "admin-k").await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "无 workspace 限定时必须显式报错而非静默查 nil"
        );
        assert!(workspaces.is_empty());
    }
}

// ============================================================================
// wiring T025⑤: biz-tags 端点对 Anonymous 身份一律 401（SEC-CRITICAL-001）
//
// 走真实 `create_router` 全栈逐端点断言 401，锁定「Anonymous 无 biz-tags 业务
// 权限」这一可观测契约。注意：路由层的 `anonymous_block_middleware` 与 handler
// 内的 `verify_user_role` 是两道防线，本组无法区分二者（响应相同），中间件自身
// 行为由 `router.rs` 的单元用例覆盖。
// ============================================================================

mod biz_tag_anonymous_guard {
    use super::biz_tag_tenant_isolation::build_app_with;
    use super::*;
    use crate::server::config::management::ConfigManagementService;
    use crate::server::handlers::mock_tests::{MockApiKeyRepository, MockConfigManagementService};

    /// 认证禁用 → `auth_middleware_fn` 注入 `ApiKeyRole::Anonymous`（LOW-1 语义）。
    fn anonymous_auth() -> Arc<ApiKeyAuth> {
        // ApiKeyRepository mock 无需任何期望：禁用分支在触达仓储前即短路返回。
        let repo = MockApiKeyRepository::new();
        Arc::new(ApiKeyAuth::new(Arc::new(repo), false))
    }

    /// 零期望的 config service：钉住「拒绝发生在服务层之前」。
    ///
    /// 五个 biz-tag handler 自身也在调用服务前拒绝 Anonymous（`verify_user_role`
    /// / list 内的显式分支），与路由层的 `anonymous_block_middleware` 构成两道
    /// 防线；本组断言的是端点的可观测契约。若未来退化为「先查库再鉴权」，
    /// mockall 会因未预期的方法调用直接 panic。
    fn untouched_config_service() -> Arc<dyn ConfigManagementService> {
        Arc::new(MockConfigManagementService::new())
    }

    /// 以 Anonymous 身份发起一次真实 HTTP 请求，返回响应状态码。
    async fn anonymous_request(method: &str, uri: &str, body: Option<&str>) -> StatusCode {
        let app = build_app_with(anonymous_auth(), untouched_config_service()).await;
        let mut builder = Request::builder().uri(uri).method(method);
        if body.is_some() {
            builder = builder.header("content-type", "application/json");
        }
        let resp = app
            .oneshot(
                builder
                    .body(match body {
                        Some(raw) => Body::from(raw.to_string()),
                        None => Body::empty(),
                    })
                    .unwrap(),
            )
            .await
            .unwrap();
        resp.status()
    }

    /// 任意格式合法的 UUID：本组只关心身份，取值本身不重要。
    fn sample_uuid() -> Uuid {
        Uuid::parse_str("33333333-3333-3333-3333-333333333333").unwrap()
    }

    /// Anonymous 即使显式带上 ?workspace_id= 也必须被拒（拒绝基于身份，
    /// 而非参数可解析性）。
    #[tokio::test]
    async fn anonymous_list_biz_tags_is_401() {
        let uri = format!("/api/v1/biz-tags?workspace_id={}", sample_uuid());
        assert_eq!(
            anonymous_request("GET", &uri, None).await,
            StatusCode::UNAUTHORIZED,
            "Anonymous 不得列出 biz-tags"
        );
    }

    #[tokio::test]
    async fn anonymous_create_biz_tag_is_401() {
        let body = r#"{"workspace_id":"11111111-1111-1111-1111-111111111111",
                       "group_id":"22222222-2222-2222-2222-222222222222",
                       "name":"anon-tag","algorithm":"segment","format":"numeric"}"#;
        assert_eq!(
            anonymous_request("POST", "/api/v1/biz-tags", Some(body)).await,
            StatusCode::UNAUTHORIZED,
            "Anonymous 不得创建 biz-tag"
        );
    }

    #[tokio::test]
    async fn anonymous_get_biz_tag_is_401() {
        let uri = format!("/api/v1/biz-tags/{}", sample_uuid());
        assert_eq!(
            anonymous_request("GET", &uri, None).await,
            StatusCode::UNAUTHORIZED,
            "Anonymous 不得读取 biz-tag"
        );
    }

    #[tokio::test]
    async fn anonymous_update_biz_tag_is_401() {
        let uri = format!("/api/v1/biz-tags/{}", sample_uuid());
        assert_eq!(
            anonymous_request("PUT", &uri, Some(r#"{"name":"renamed"}"#)).await,
            StatusCode::UNAUTHORIZED,
            "Anonymous 不得修改 biz-tag"
        );
    }

    /// 破坏性端点：Anonymous 越权删除的后果最重，单独钉桩。
    #[tokio::test]
    async fn anonymous_delete_biz_tag_is_401() {
        let uri = format!("/api/v1/biz-tags/{}", sample_uuid());
        assert_eq!(
            anonymous_request("DELETE", &uri, None).await,
            StatusCode::UNAUTHORIZED,
            "Anonymous 不得删除 biz-tag"
        );
    }
}

// ============================================================================
// wiring T008: garrison cache-memory 认证缓存接线（R-auth-001 / R-auth-002）
// ============================================================================

#[cfg(feature = "garrison-auth")]
mod auth_cache_wiring {
    use super::*;
    use crate::core::database::ApiKey;
    use crate::server::auth::AuthCache;
    use crate::server::handlers::mock_generator::MockIdGenerator;
    use crate::server::handlers::mock_tests::{MockApiKeyRepository, MockConfigManagementService};
    use crate::server::handlers::ApiHandlers;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    /// 共享仓储 mock（T027⑧：替代手写 `CountingRepo` 的 11 个样板方法）。
    ///
    /// 只登记本组测试真正触及的两个方法：
    /// - `validate_api_key`：合法凭证返回 User 身份并累计调用次数；`revoked`
    ///   置位后一律未命中（模拟吊销已落库）。
    /// - `delete_api_key`：置位 `revoked`（`ApiHandlers::revoke_api_key` 的副作用）。
    ///
    /// 其余方法不写期望：mockall 对未登记的调用直接 panic，与旧实现里
    /// `"not implemented in CountingRepo"` 的桩等价，但不必付样板代码。
    ///
    /// `validate_times` **就是缓存语义断言本身**：多一次 = 缓存没命中，
    /// 少一次 = 吊销 / TTL 过期后没回源（mock 释放时校验）。
    /// 返回的计数器供测试在中间点位断言进度（旧实现的 `validate_calls()` 断言）。
    fn mock_repo(
        workspace_id: Uuid,
        validate_times: usize,
    ) -> (MockApiKeyRepository, Arc<AtomicUsize>, Arc<AtomicBool>) {
        let validate_calls = Arc::new(AtomicUsize::new(0));
        let revoked = Arc::new(AtomicBool::new(false));
        let mut repo = MockApiKeyRepository::new();

        let counter = validate_calls.clone();
        let revoked_flag = revoked.clone();
        repo.expect_validate_api_key()
            .times(validate_times)
            .returning(move |key_id, key_secret| {
                counter.fetch_add(1, Ordering::SeqCst);
                if revoked_flag.load(Ordering::SeqCst) {
                    return Ok(None);
                }
                if key_id == "cache-key" && key_secret == "cache-secret" {
                    Ok(Some(AuthenticatedKey {
                        workspace_id: Some(workspace_id),
                        role: ApiKeyRole::User,
                        used_previous_credential: false,
                    }))
                } else {
                    Ok(None)
                }
            });

        let revoked_flag = revoked.clone();
        repo.expect_delete_api_key().returning(move |_| {
            revoked_flag.store(true, Ordering::SeqCst);
            Ok(())
        });

        (repo, validate_calls, revoked)
    }

    /// 登记「校验成功 → 写缓存」路径要读的 key 行（仅 `key_id == "cache-key"`）。
    ///
    /// 次数即"缓存被写入了几次"的断言；其他 key_id 不匹配本期望，
    /// 由调用方按需另登记（未登记的 key_id 会 panic）。
    fn expect_cache_key_row(repo: &mut MockApiKeyRepository, times: usize, workspace_id: Uuid) {
        let info = ApiKey {
            id: Uuid::new_v4(),
            key_id: "cache-key".to_string(),
            key_prefix: "nino_".to_string(),
            role: ApiKeyRole::User,
            workspace_id: Some(workspace_id),
            name: "cache key".to_string(),
            description: None,
            rate_limit: 100,
            enabled: true,
            expires_at: None,
            last_used_at: None,
            created_at: chrono::Utc::now().naive_utc(),
        };
        repo.expect_get_api_key_by_id()
            .withf(|key_id: &str| key_id == "cache-key")
            .times(times)
            .returning(move |_| Ok(Some(info.clone())));
    }

    fn make_cached_auth(
        repo: Arc<MockApiKeyRepository>,
        ttl_seconds: u64,
    ) -> (ApiKeyAuth, Arc<AuthCache>) {
        let cache = Arc::new(AuthCache::new(ttl_seconds));
        let auth = ApiKeyAuth::new(repo, true).with_cache(cache.clone());
        (auth, cache)
    }

    /// R-auth-001 (a)：同一合法凭证第二次校验命中缓存，不再调用仓储。
    #[tokio::test]
    async fn second_validation_hits_cache_without_repository_call() {
        let workspace_id = Uuid::new_v4();
        // 整个测试只允许 1 次回源：第二次必须走缓存
        let (mut repo, validate_calls, _revoked) = mock_repo(workspace_id, 1);
        expect_cache_key_row(&mut repo, 1, workspace_id);
        let (auth, _cache) = make_cached_auth(Arc::new(repo), 300);

        assert!(auth
            .validate_key("cache-key", "cache-secret")
            .await
            .is_some());
        assert_eq!(
            validate_calls.load(Ordering::SeqCst),
            1,
            "首次校验必须回源仓储"
        );

        assert!(auth
            .validate_key("cache-key", "cache-secret")
            .await
            .is_some());
        assert_eq!(
            validate_calls.load(Ordering::SeqCst),
            1,
            "第二次校验必须命中缓存，仓储调用计数不得增加"
        );
    }

    /// 错误凭证永不命中缓存（每次都要回源，防止 key_id 枚举绕过）。
    #[tokio::test]
    async fn wrong_secret_always_falls_back_to_repository() {
        let workspace_id = Uuid::new_v4();
        let (mut repo, validate_calls, _revoked) = mock_repo(workspace_id, 2);
        // 校验未通过 → 不得写缓存 → 不得读 key 行
        repo.expect_get_api_key_by_id().never();
        let (auth, _cache) = make_cached_auth(Arc::new(repo), 300);

        assert!(auth.validate_key("cache-key", "bad-secret").await.is_none());
        assert!(auth.validate_key("cache-key", "bad-secret").await.is_none());
        assert_eq!(
            validate_calls.load(Ordering::SeqCst),
            2,
            "无效凭证不得写入或命中缓存"
        );
    }

    /// 双代凭证仓储 mock（T010）：`cache-secret` 是当代凭证，`previous-secret`
    /// 代表轮换后仍在宽限期内的上一代凭证（`used_previous_credential = true`）。
    ///
    /// `times` 就是断言本身：它限制"允许回源仓储几次"，缓存若意外命中/未命中都
    /// 会在 mock 释放时报违反。
    fn mock_repo_two_generations(
        workspace_id: Uuid,
        times: usize,
    ) -> (MockApiKeyRepository, Arc<AtomicUsize>) {
        let validate_calls = Arc::new(AtomicUsize::new(0));
        let mut repo = MockApiKeyRepository::new();
        let counter = validate_calls.clone();
        repo.expect_validate_api_key()
            .times(times)
            .returning(move |key_id, key_secret| {
                counter.fetch_add(1, Ordering::SeqCst);
                if key_id != "cache-key" {
                    return Ok(None);
                }
                let used_previous = key_secret == "previous-secret";
                if !used_previous && key_secret != "cache-secret" {
                    return Ok(None);
                }
                Ok(Some(AuthenticatedKey {
                    workspace_id: Some(workspace_id),
                    role: ApiKeyRole::User,
                    used_previous_credential: used_previous,
                }))
            });
        (repo, validate_calls)
    }

    /// T010（D-A）：宽限期命中的决策有时效性（受 `rotate_expires_at` 约束），
    /// 缓存 TTL 表达不了这个绝对截止时间，因此这类命中不得写入认证决策缓存 ——
    /// 否则上一代凭证在宽限期结束后仍会被缓存放行到 TTL 到期，等于变相延长窗口。
    #[tokio::test]
    async fn test_grace_credential_hit_is_not_cached() {
        let workspace_id = Uuid::new_v4();
        let (mut repo, validate_calls) = mock_repo_two_generations(workspace_id, 2);
        // 跳过写入 ⇒ 连"读 key 行拿绝对过期时间"都不该发生
        repo.expect_get_api_key_by_id().never();
        let (auth, cache) = make_cached_auth(Arc::new(repo), 300);

        for _ in 0..2 {
            let identity = auth
                .validate_key("cache-key", "previous-secret")
                .await
                .expect("宽限期内的上一代凭证必须通过认证");
            assert!(
                identity.used_previous_credential,
                "上一代凭证命中必须置标记，否则缓存侧无法识别并跳过写入"
            );
        }

        assert_eq!(
            validate_calls.load(Ordering::SeqCst),
            2,
            "上一代凭证的两次校验都必须回源仓储"
        );
        assert!(
            cache.get("cache-key", "previous-secret").await.is_none(),
            "缓存中不得出现上一代凭证的认证决策"
        );
    }

    /// T010 的另一半：跳过写入只针对宽限期命中，当代凭证的缓存路径不得受影响。
    #[tokio::test]
    async fn test_current_credential_hit_is_cached() {
        let workspace_id = Uuid::new_v4();
        // 允许回源 2 次 = 上一代 1 次 + 当代首见 1 次；当代第二次必须命中缓存
        let (mut repo, validate_calls) = mock_repo_two_generations(workspace_id, 2);
        // 次数 1 = 只有当代凭证那次校验读了 key 行
        expect_cache_key_row(&mut repo, 1, workspace_id);
        let (auth, _cache) = make_cached_auth(Arc::new(repo), 300);

        assert!(auth
            .validate_key("cache-key", "previous-secret")
            .await
            .is_some());
        assert!(auth
            .validate_key("cache-key", "cache-secret")
            .await
            .is_some());
        assert_eq!(validate_calls.load(Ordering::SeqCst), 2);

        let identity = auth
            .validate_key("cache-key", "cache-secret")
            .await
            .expect("当代凭证第二次校验仍应通过");
        assert!(
            !identity.used_previous_credential,
            "从缓存恢复的决策必须标记为非宽限期"
        );
        assert_eq!(
            validate_calls.load(Ordering::SeqCst),
            2,
            "当代凭证第二次校验必须命中缓存，不得回源"
        );
    }

    /// R-auth-002：吊销后立即再校验返回 401（经真实中间件，不等待 TTL）。
    #[tokio::test]
    async fn revoked_key_is_rejected_immediately_via_middleware() {
        let workspace_id = Uuid::new_v4();
        // 2 次回源 = 吊销前首次校验 + 吊销后即时回源；
        // 缓存若失效（第二次请求也回源）或吊销后仍命中缓存都会违反 .times(2)
        let (mut repo, validate_calls, revoked) = mock_repo(workspace_id, 2);
        // 真实吊销路径按行 id 查询（查不到）；缓存写入路径按 key_id 查询
        expect_cache_key_row(&mut repo, 1, workspace_id);
        // T003：吊销守卫改用 `find_api_key_by_row_id`（按行主键），不再用
        // `get_api_key_by_id`（按 key_id 字符串）。本用例吊销的是与本组缓存键
        // 无关的随机行 id → 查不到 → 守卫跳过 → 删除照常执行。
        // 刻意不登记 `count_admin_keys`：行不存在时守卫不得进入计数分支。
        repo.expect_find_api_key_by_row_id()
            .times(1)
            .returning(|_| Ok(None));
        let repo = Arc::new(repo);

        let (auth, cache) = make_cached_auth(repo.clone(), 300);

        let config_service: Arc<dyn crate::server::config::management::ConfigManagementService> =
            Arc::new(MockConfigManagementService::new());
        let handlers = Arc::new(
            ApiHandlers::with_api_key_repository(
                Arc::new(MockIdGenerator::new()),
                config_service,
                repo,
            )
            .with_auth_cache(cache),
        );

        let app = build_auth_router(Arc::new(auth));
        let header = basic_auth_header("cache-key", "cache-secret");

        // 吊销前：200，且第二次请求命中缓存
        let ok1 = app
            .clone()
            .oneshot(make_request(Some(&header)))
            .await
            .unwrap();
        assert_eq!(ok1.status(), StatusCode::OK);
        let ok2 = app
            .clone()
            .oneshot(make_request(Some(&header)))
            .await
            .unwrap();
        assert_eq!(ok2.status(), StatusCode::OK);
        assert_eq!(
            validate_calls.load(Ordering::SeqCst),
            1,
            "第二次请求应命中缓存"
        );

        // 吊销（模拟 DELETE /api-keys/{id} 的服务端副作用）
        handlers
            .revoke_api_key(Uuid::new_v4())
            .await
            .expect("吊销应成功");
        assert!(
            revoked.load(Ordering::SeqCst),
            "吊销必须经仓储 delete_api_key 落库"
        );

        // 吊销后立即再请求：401，且必须回源仓储确认
        let denied = app.oneshot(make_request(Some(&header))).await.unwrap();
        assert_eq!(
            denied.status(),
            StatusCode::UNAUTHORIZED,
            "吊销必须即时生效，不得等 TTL 过期"
        );
        assert_eq!(
            validate_calls.load(Ordering::SeqCst),
            2,
            "吊销后校验必须回源仓储"
        );
    }

    /// R-auth-001：TTL 过期后下一次校验回源仓储。
    #[tokio::test]
    async fn ttl_expiry_forces_repository_revalidation() {
        let workspace_id = Uuid::new_v4();
        // 两次回源 + 两次写缓存（过期后重新写回）
        let (mut repo, validate_calls, _revoked) = mock_repo(workspace_id, 2);
        expect_cache_key_row(&mut repo, 2, workspace_id);
        let (auth, _cache) = make_cached_auth(Arc::new(repo), 1);

        assert!(auth
            .validate_key("cache-key", "cache-secret")
            .await
            .is_some());
        assert_eq!(validate_calls.load(Ordering::SeqCst), 1);

        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;

        assert!(auth
            .validate_key("cache-key", "cache-secret")
            .await
            .is_some());
        assert_eq!(
            validate_calls.load(Ordering::SeqCst),
            2,
            "TTL 过期后必须回源"
        );
    }

    /// cache_ttl_seconds=0：缓存禁用，每次校验都回源。
    #[tokio::test]
    async fn zero_ttl_disables_caching() {
        let workspace_id = Uuid::new_v4();
        let (mut repo, validate_calls, _revoked) = mock_repo(workspace_id, 2);
        // 缓存条目不会被命中，但写路径仍会读 key 行（ttl=0 时 put 为空操作）
        expect_cache_key_row(&mut repo, 2, workspace_id);
        let (auth, _cache) = make_cached_auth(Arc::new(repo), 0);

        assert!(auth
            .validate_key("cache-key", "cache-secret")
            .await
            .is_some());
        assert!(auth
            .validate_key("cache-key", "cache-secret")
            .await
            .is_some());
        assert_eq!(validate_calls.load(Ordering::SeqCst), 2);
    }
}

// ============================================================================
// key-rotation-and-config-failfast T005：admin key 吊销守卫 e2e
// ============================================================================

/// 有状态内存仓储：把「key 行」完整建模（行 UUID + key_id + role + enabled +
/// secret 哈希），使 `find_api_key_by_row_id` / `count_admin_keys` 能像真实 SQL
/// 那样回答（含 `role='admin' AND enabled=true` 的双重过滤）。
///
/// 目的：把守卫缺陷钉在 **handler 集成层**。repository 单测只能证明新增的两个
/// 查询方法本身正确，证明不了 handler 真的用对了它们 —— 旧实现正是"方法没问题、
/// 调用点查错列"才让守卫整块跳过的。
mod admin_key_guard_e2e {
    use super::*;
    use crate::core::types::error::CoreError;
    use crate::server::handlers::mock_generator::MockIdGenerator;
    use crate::server::handlers::mock_tests::MockConfigManagementService;
    use crate::server::handlers::ApiHandlers;
    use std::sync::Mutex;

    const ADMIN_KEY_ID: &str = "guard-admin-key";
    const ADMIN_SECRET: &str = "guard-admin-secret";

    #[derive(Clone)]
    struct KeyRow {
        id: Uuid,
        key_id: String,
        secret_hash: String,
        role: ApiKeyRole,
        enabled: bool,
    }

    #[derive(Clone, Default)]
    struct StatefulKeyRepo {
        rows: Arc<Mutex<Vec<KeyRow>>>,
    }

    impl StatefulKeyRepo {
        fn hash_secret(secret: &str) -> String {
            let mut hasher = sha2::Sha256::default();
            hasher.update(secret);
            hex::encode(hasher.finalize())
        }

        /// 预置唯一一行启用中的全局 admin key（workspace_id 为 NULL 的场景）。
        fn with_single_enabled_admin() -> (Self, Uuid) {
            let row = KeyRow {
                id: Uuid::new_v4(),
                key_id: ADMIN_KEY_ID.to_string(),
                secret_hash: Self::hash_secret(ADMIN_SECRET),
                role: ApiKeyRole::Admin,
                enabled: true,
            };
            let row_id = row.id;
            (
                Self {
                    rows: Arc::new(Mutex::new(vec![row])),
                },
                row_id,
            )
        }

        /// 行 → 对外 `ApiKeyInfo`（`get_api_key_by_id` 与
        /// `find_api_key_by_row_id` 共用，避免重复整段 struct 字面量）。
        fn to_info(row: &KeyRow) -> ApiKeyInfo {
            ApiKeyInfo {
                id: row.id,
                key_id: row.key_id.clone(),
                key_prefix: "nino_".to_string(),
                role: row.role.clone(),
                workspace_id: None,
                name: "stateful row".to_string(),
                description: None,
                rate_limit: 10000,
                enabled: row.enabled,
                expires_at: None,
                last_used_at: None,
                created_at: chrono::Utc::now().naive_utc(),
            }
        }

        /// 观察某行的 `enabled`，用于断言"守卫生效时删除根本没执行"。
        fn enabled_of(&self, id: Uuid) -> Option<bool> {
            self.rows
                .lock()
                .unwrap()
                .iter()
                .find(|r| r.id == id)
                .map(|r| r.enabled)
        }
    }

    #[async_trait]
    impl ApiKeyRepository for StatefulKeyRepo {
        async fn create_api_key(&self, _request: &CreateApiKeyRequest) -> Result<ApiKeyWithSecret> {
            Err(CoreError::InternalError(
                "create_api_key not needed by admin guard e2e".to_string(),
            ))
        }

        async fn get_api_key_by_id(&self, key_id: &str) -> Result<Option<ApiKeyInfo>> {
            Ok(self
                .rows
                .lock()
                .unwrap()
                .iter()
                .find(|r| r.key_id == key_id)
                .map(Self::to_info))
        }

        async fn validate_api_key(
            &self,
            key_id: &str,
            key_secret: &str,
        ) -> Result<Option<AuthenticatedKey>> {
            use subtle::ConstantTimeEq;
            let incoming = Self::hash_secret(key_secret);
            let hit = self
                .rows
                .lock()
                .unwrap()
                .iter()
                .find(|r| r.key_id == key_id)
                .filter(|r| r.enabled)
                .filter(|r| r.secret_hash.as_bytes().ct_eq(incoming.as_bytes()).into())
                .map(|r| AuthenticatedKey {
                    workspace_id: None,
                    role: r.role.clone(),
                    used_previous_credential: false,
                });
            Ok(hit)
        }

        async fn list_api_keys(
            &self,
            _workspace_id: Uuid,
            _limit: Option<u32>,
            _offset: Option<u32>,
        ) -> Result<Vec<ApiKeyInfo>> {
            // 刻意恒空：真实库里全局 admin 行的 `workspace_id` 是 NULL，
            // `NULL = nil_uuid` 在 SQL 中不成立 —— 这正是旧守卫失效的根因。
            // 若实现退回 `list_api_keys` 扫描，这里会让它再次看不见 admin 行。
            Ok(vec![])
        }

        /// `ApiHandlers::revoke_api_key` 的落库副作用：置 `enabled = false`。
        async fn delete_api_key(&self, id: Uuid) -> Result<()> {
            if let Some(row) = self.rows.lock().unwrap().iter_mut().find(|r| r.id == id) {
                row.enabled = false;
            }
            Ok(())
        }

        async fn revoke_api_key(&self, id: Uuid) -> Result<()> {
            self.delete_api_key(id).await
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

        /// 按行主键取行 —— 守卫的新查询路径（旧实现错用 `get_api_key_by_id`）。
        async fn find_api_key_by_row_id(&self, id: Uuid) -> Result<Option<ApiKeyInfo>> {
            Ok(self
                .rows
                .lock()
                .unwrap()
                .iter()
                .find(|r| r.id == id)
                .map(Self::to_info))
        }

        /// SQL 侧计数：`role = 'admin' AND enabled = true`，不带 workspace 过滤。
        async fn count_admin_keys(&self) -> Result<u64> {
            let count = self
                .rows
                .lock()
                .unwrap()
                .iter()
                .filter(|r| r.role == ApiKeyRole::Admin && r.enabled)
                .count();
            Ok(count as u64)
        }

        async fn rotate_api_key(
            &self,
            _key_id: &str,
            _grace_period_seconds: u64,
        ) -> Result<ApiKeyWithSecret> {
            Err(CoreError::InternalError(
                "rotate_api_key not needed by admin guard e2e".to_string(),
            ))
        }

        async fn get_keys_older_than(&self, _age_threshold_days: i64) -> Result<Vec<ApiKeyInfo>> {
            Ok(vec![])
        }
    }

    fn build_handlers(repo: Arc<StatefulKeyRepo>) -> ApiHandlers {
        let config_service: Arc<dyn crate::server::config::management::ConfigManagementService> =
            Arc::new(MockConfigManagementService::new());
        ApiHandlers::with_api_key_repository(Arc::new(MockIdGenerator::new()), config_service, repo)
    }

    /// T005：唯一启用中的 admin key 不得被吊销，且吊销被拒后该凭证仍可用。
    ///
    /// 缺陷形态：旧守卫用 `get_api_key_by_id(&id.to_string())` 查行 UUID
    /// （该方法按 `key_id` 过滤）→ 恒 `None` → 整块守卫跳过 → 管理员把自己
    /// 锁在系统外。本用例在 handler 集成层覆盖，不断言 HTTP 状态码。
    #[tokio::test]
    async fn e2e_revoke_only_admin_key_is_rejected_and_key_still_usable() {
        let (repo, admin_row_id) = StatefulKeyRepo::with_single_enabled_admin();
        let repo = Arc::new(repo);
        let handlers = build_handlers(repo.clone());

        let err = handlers.revoke_api_key(admin_row_id).await.unwrap_err();
        assert!(
            matches!(err, CoreError::AuthenticationError(_)),
            "唯一启用中的 admin key 必须被拒绝吊销，实际返回：{err:?}"
        );

        // 守卫拦截 → 删除路径未执行 → 行仍启用（旧实现会把它置为 disabled）
        assert_eq!(
            repo.enabled_of(admin_row_id),
            Some(true),
            "守卫拦截后 key 行的 enabled 必须仍为 true"
        );

        // 该 admin secret 仍可正常认证：把"守卫误放行"这条回归也钉住
        let auth = ApiKeyAuth::new(repo.clone(), true);
        assert!(
            auth.validate_key(ADMIN_KEY_ID, ADMIN_SECRET)
                .await
                .is_some(),
            "守卫拦截后 admin 凭证必须仍可通过校验"
        );
    }
}
