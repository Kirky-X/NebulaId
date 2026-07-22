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
use base64::Engine;
use sdforge::axum::{
    body::Body,
    http::{Request, StatusCode},
    middleware::{from_fn, from_fn_with_state},
    routing::get,
    Router,
};
use sdforge::tower::ServiceExt;
use sha2::Digest;
use uuid::Uuid;
use validator::Validate;

use crate::core::database::{
    ApiKeyInfo, ApiKeyRepository, ApiKeyResponse, ApiKeyRole, ApiKeyWithSecret, CreateApiKeyRequest,
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
        })
    }

    async fn get_api_key_by_id(&self, _key_id: &str) -> Result<Option<ApiKeyInfo>> {
        Ok(None)
    }

    async fn validate_api_key(
        &self,
        key_id: &str,
        key_secret: &str,
    ) -> Result<Option<(Option<Uuid>, ApiKeyRole)>> {
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
                return Ok(Some((workspace_id, role.clone())));
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
    let bytes = sdforge::axum::body::to_bytes(body, usize::MAX)
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
// E2E-VAL-005: BatchGenerateRequest size=101 → 验证失败
// ----------------------------------------------------------------------------

#[test]
fn e2e_batch_generate_request_validates_size_101_fails() {
    // size 范围 [1, 100]，101 应超出上限
    let request = BatchGenerateRequest {
        workspace: "ws".to_string(),
        group: "g".to_string(),
        biz_tag: "tag".to_string(),
        size: Some(101),
        algorithm: None,
    };

    let result = Validate::validate(&request);
    assert!(
        result.is_err(),
        "size=101 应超过 max=100 上限，验证必须失败"
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
