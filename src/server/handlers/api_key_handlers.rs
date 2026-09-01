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

//! API Key management handlers + `KeyRotationHandle` (rule 25 split).

use super::helpers::map_db_error;
use crate::core::database::{ApiKeyRole, CreateApiKeyRequest as CoreCreateApiKeyRequest};
use crate::core::{CoreError, Result};
use crate::server::models::{
    ApiKeyListResponse, ApiKeyResponse, ApiKeyWithSecretResponse, CreateApiKeyRequest,
    RevokeApiKeyResponse,
};

/// Handle for managing the key rotation background task.
#[derive(Clone, Debug)]
pub struct KeyRotationHandle {
    pub(super) shutdown_tx: tokio::sync::watch::Sender<bool>,
}

impl KeyRotationHandle {
    /// Signal the key rotation task to stop.
    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(true);
    }
}

impl super::ApiHandlers {
    /// Create a new API Key (admin only).
    pub async fn create_api_key(
        &self,
        workspace_id: Option<uuid::Uuid>,
        req: CreateApiKeyRequest,
    ) -> Result<ApiKeyWithSecretResponse> {
        let repo = self.api_key_repo.as_ref().ok_or_else(|| {
            CoreError::NotFound(
                t!("api.error.handlers.workspace_handlers.api_key_repo_not_configured").to_string(),
            )
        })?;

        let role = match req.role.as_deref() {
            Some("admin") => ApiKeyRole::Admin,
            Some("user") | None => ApiKeyRole::User,
            Some(r) => {
                return Err(CoreError::AuthenticationError(
                    t!("api.error.handlers.api_key_handlers.invalid_role", role = r).to_string(),
                ))
            }
        };

        if role == ApiKeyRole::Admin {
            // Phase 9 T043 (HIGH H8) — reject additional admin keys
            // instead of merely warning. Combined with the C3 SQL CHECK
            // fix (admin key must have NULL workspace_id), this enforces
            // a single global admin key invariant. Previously an
            // attacker with admin credentials could create a second
            // admin key as a persistence backdoor.
            //
            // 缺陷根因（与 revoke 守卫同源）：旧实现用
            // `list_api_keys(Uuid::nil(), Some(1000), Some(0))` + 内存扫描，
            // 而全局 admin key 的 `workspace_id` 是 NULL，`NULL = nil_uuid`
            // 在 SQL 中不成立 → 该查询恒空 → 守卫从未生效。改用
            // `count_admin_keys()`（SQL 侧 `role='admin' AND enabled=true`
            // 计数、不带 workspace 过滤、无 1000 行分页上界）。
            let admin_count = repo.count_admin_keys().await.map_err(map_db_error)?;

            if admin_count > 0 {
                tracing::warn!(
                    event = "admin_key_creation_blocked",
                    workspace_id = ?workspace_id,
                    "{}",
                    t!("log.server.handlers.api_key_handlers.creating_additional_admin_key")
                );
                return Err(CoreError::AuthenticationError(
                    t!("api.error.handlers.api_key_handlers.admin_key_already_exists").to_string(),
                ));
            }
        }

        if role == ApiKeyRole::User {
            let ws_id = workspace_id.ok_or_else(|| {
                CoreError::InvalidInput(t!("api.error.workspace_id_required").to_string())
            })?;

            let existing_keys = repo
                .list_api_keys(ws_id, Some(1000), Some(0))
                .await
                .map_err(map_db_error)?;

            let has_user_key = existing_keys
                .iter()
                .any(|k| k.role == crate::core::database::ApiKeyRole::User);

            if has_user_key {
                return Err(CoreError::AuthenticationError(
                    t!(
                        "api.error.handlers.api_key_handlers.user_key_already_exists",
                        workspace_id = ws_id
                    )
                    .to_string(),
                ));
            }
        }

        let expires_at = match &req.expires_at {
            Some(ts) => Some(
                chrono::DateTime::parse_from_rfc3339(ts)
                    .map_err(|_| {
                        CoreError::InvalidIdFormat("Invalid expires_at format".to_string())
                    })?
                    .with_timezone(&chrono::Utc)
                    .naive_utc(),
            ),
            None => None,
        };

        let core_req = CoreCreateApiKeyRequest {
            workspace_id,
            name: req.name,
            description: req.description,
            role,
            rate_limit: req.rate_limit,
            expires_at,
            key_secret: None,
            key_id: None,
        };

        let key_with_secret = repo.create_api_key(&core_req).await.map_err(map_db_error)?;

        key_with_secret.try_into()
    }

    /// List API Keys for a workspace (admin only).
    pub async fn list_api_keys(
        &self,
        workspace_id: uuid::Uuid,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> Result<ApiKeyListResponse> {
        let repo = self.api_key_repo.as_ref().ok_or_else(|| {
            CoreError::NotFound(
                t!("api.error.handlers.workspace_handlers.api_key_repo_not_configured").to_string(),
            )
        })?;

        let keys = repo
            .list_api_keys(workspace_id, limit, offset)
            .await
            .map_err(map_db_error)?;

        let responses: Vec<ApiKeyResponse> = keys
            .into_iter()
            .map(ApiKeyResponse::try_from)
            .collect::<std::result::Result<Vec<_>, _>>()?;

        let total = repo
            .count_api_keys(workspace_id)
            .await
            .map_err(map_db_error)?;

        Ok(ApiKeyListResponse {
            api_keys: responses,
            total,
        })
    }

    /// 按 `key_id` 失效认证缓存条目（wiring T008）。未启用缓存时为 no-op。
    #[cfg(feature = "garrison-auth")]
    pub(super) async fn invalidate_auth_cache(&self, key_id: &str) {
        if let Some(cache) = self.auth_cache.as_ref() {
            cache.invalidate(key_id).await;
        }
    }

    /// 未启用 `garrison-auth` 时的等价 no-op，保持调用点无需条件编译。
    #[cfg(not(feature = "garrison-auth"))]
    pub(super) async fn invalidate_auth_cache(&self, _key_id: &str) {}

    /// 整体清空认证缓存。用于只能拿到行 `id`、无法定位 `key_id` 的吊销路径 ——
    /// 宁可多失效一些条目（下一次校验回源 DB），也不能留下可继续通过的旧凭证。
    #[cfg(feature = "garrison-auth")]
    async fn clear_auth_cache(&self) {
        if let Some(cache) = self.auth_cache.as_ref() {
            cache.clear().await;
        }
    }

    #[cfg(not(feature = "garrison-auth"))]
    async fn clear_auth_cache(&self) {}

    /// Revoke (delete) an API Key (admin only).
    pub async fn revoke_api_key(&self, id: uuid::Uuid) -> Result<RevokeApiKeyResponse> {
        let repo = self.api_key_repo.as_ref().ok_or_else(|| {
            CoreError::NotFound(
                t!("api.error.handlers.workspace_handlers.api_key_repo_not_configured").to_string(),
            )
        })?;

        // 缺陷根因（两处，同一根因的两个面）：
        // 1) 旧实现用 `get_api_key_by_id(&id.to_string())` 查行 UUID，而该方法按
        //    `key_id` 字符串过滤 → 永远 `None` → 整块守卫被跳过。
        // 2) admin 计数走 `list_api_keys(Uuid::nil(), ..)`，而全局 admin key 的
        //    `workspace_id` 是 NULL，`NULL = nil_uuid` 在 SQL 中不成立 → 该查询恒空。
        // 代之以 `find_api_key_by_row_id`（按主键取行）+ `count_admin_keys`
        // （SQL 侧 `role='admin' AND enabled=true` 计数，不带 workspace 过滤）。
        let key_info = repo
            .find_api_key_by_row_id(id)
            .await
            .map_err(map_db_error)?;

        if let Some(key) = key_info {
            // 只保护"启用中"的 admin：已禁用的行不占"最后一个"名额，
            // `count_admin_keys` 只数 enabled，两侧语义自洽。
            if key.role == crate::core::database::ApiKeyRole::Admin && key.enabled {
                let admin_count = repo.count_admin_keys().await.map_err(map_db_error)?;

                if admin_count <= 1 {
                    // 与 create 守卫（`admin_key_creation_blocked`）同口径留痕：此前这条
                    // 分支只有返回给客户端的 i18n 文案，服务端零日志，运维无法定位是谁在
                    // 尝试吊销最后一个管理凭证。
                    tracing::warn!(
                        event = "admin_key_revoke_blocked",
                        key_id = %id,
                        "refused to revoke the last enabled admin API key"
                    );
                    return Err(CoreError::AuthenticationError(
                        t!("api.error.handlers.api_key_handlers.cannot_revoke_last_admin")
                            .to_string(),
                    ));
                }
            }
        }

        repo.delete_api_key(id).await.map_err(map_db_error)?;
        // wiring T008：吊销后本进程的缓存必须立即失效，不能等 TTL 自然过期。
        // 多节点部署时其他节点最长滞后一个 cache_ttl（口径见 docs/DEPLOYMENT.md 7.1）。
        self.clear_auth_cache().await;

        Ok(RevokeApiKeyResponse {
            success: true,
            message: t!("api.success.handlers.api_key_handlers.revoked", id = id).to_string(),
        })
    }

    /// Rotate an API Key (generate new secret, keep old key active during grace period).
    pub async fn rotate_api_key(&self, key_id: &str) -> Result<ApiKeyWithSecretResponse> {
        if key_id.is_empty() {
            return Err(CoreError::InvalidInput(
                t!("api.error.handlers.api_key_handlers.key_id_empty").to_string(),
            ));
        }

        let repo = self.api_key_repo.as_ref().ok_or_else(|| {
            CoreError::NotFound(
                t!("api.error.handlers.workspace_handlers.api_key_repo_not_configured").to_string(),
            )
        })?;

        // L16 修复：从 `ApiHandlers::key_rotation_grace_period_seconds`
        // 读取，原为硬编码 `const GRACE_PERIOD_SECONDS: u64 = 7 * 24 * 60 * 60`。
        // 默认 0 = 关闭宽限期（T011，见
        // `core::config::defaults::DEFAULT_KEY_ROTATION_GRACE_PERIOD_SECONDS`），
        // 需要"轮换不掉请求"时用 `AuthConfig::key_rotation_grace_period_seconds`
        // + `ApiHandlers::with_key_rotation_grace_period` 显式开启。
        let grace_period_seconds = self.key_rotation_grace_period_seconds;

        let key_with_secret = repo
            .rotate_api_key(key_id, grace_period_seconds)
            .await
            .map_err(map_db_error)?;

        tracing::info!(event = "api_key_rotated", key_id = key_id);
        // wiring T008：清掉该 key_id 名下全部条目（含宽限期内的旧 secret 变体）。
        self.invalidate_auth_cache(key_id).await;

        key_with_secret.try_into()
    }
}

#[cfg(test)]
mod tests {
    use crate::core::database::{ApiKeyInfo, ApiKeyRepository, ApiKeyRole};
    use crate::core::CoreError;
    use crate::server::config::management::ConfigManagementService;
    use crate::server::handlers::mock_generator::MockIdGenerator;
    use crate::server::handlers::mock_tests::{MockApiKeyRepository, MockConfigManagementService};
    use crate::server::handlers::ApiHandlers;
    use crate::server::models::CreateApiKeyRequest;
    use std::sync::Arc;
    use uuid::Uuid;

    fn make_handlers_with_repo(mock_repo: MockApiKeyRepository) -> Arc<ApiHandlers> {
        let mock_gen = Arc::new(MockIdGenerator::new());
        let config_service: Arc<dyn ConfigManagementService> =
            Arc::new(MockConfigManagementService::new());
        let repo: Arc<dyn ApiKeyRepository> = Arc::new(mock_repo);
        Arc::new(ApiHandlers::with_api_key_repository(
            mock_gen,
            config_service,
            repo,
        ))
    }

    fn make_api_key_info(role: ApiKeyRole, enabled: bool) -> ApiKeyInfo {
        ApiKeyInfo {
            id: Uuid::new_v4(),
            key_id: "niad_test-key-id".to_string(),
            key_prefix: "niad_".to_string(),
            role,
            // 全局 admin key 的 workspace_id 是 NULL —— 正是它让
            // `list_api_keys(nil, ..)` 恒空，从而让旧守卫从不生效。
            workspace_id: None,
            name: "test-key".to_string(),
            description: None,
            rate_limit: 10000,
            enabled,
            expires_at: None,
            last_used_at: None,
            created_at: chrono::Utc::now().naive_utc(),
        }
    }

    /// T003：唯一一个启用中的 admin key 不得被吊销 —— 否则管理员把自己锁在系统外。
    ///
    /// 缺陷根因：旧实现用 `get_api_key_by_id(&id.to_string())` 查行 UUID（该方法按
    /// `key_id` 字符串过滤），永远查不到 → 整块守卫被跳过 → 吊销成功。
    #[tokio::test]
    async fn test_revoke_last_enabled_admin_key_is_rejected() {
        let mut mock_repo = MockApiKeyRepository::new();
        mock_repo
            .expect_find_api_key_by_row_id()
            .return_once(move |_| Ok(Some(make_api_key_info(ApiKeyRole::Admin, true))));
        mock_repo.expect_count_admin_keys().return_once(|| Ok(1));
        // 守卫生效时删除路径根本不该被触及。
        mock_repo.expect_delete_api_key().never();

        let handlers = make_handlers_with_repo(mock_repo);
        let result = handlers.revoke_api_key(Uuid::new_v4()).await;

        match result {
            Err(CoreError::AuthenticationError(msg)) => {
                assert!(
                    msg.to_lowercase().contains("admin"),
                    "message must name the admin-key invariant, got: {msg}"
                );
            }
            other => panic!("expected AuthenticationError, got {other:?}"),
        }
    }

    /// T004：已存在 admin key 时，第二个 admin key 必须被拒 —— 即便同库有 1000 条 user key。
    ///
    /// 缺陷根因：旧实现用 `list_api_keys(Uuid::nil(), Some(1000), Some(0))` + 内存扫描，
    /// 而全局 admin key 的 `workspace_id` 是 NULL（`NULL = nil_uuid` 在 SQL 中不成立）
    /// → 查询恒空 → 守卫从未生效；1000 行分页上界是叠加的第二重隐患。
    /// 桩里 `list_api_keys` 故意返回 1000 条 **user** 行：若实现退回到内存扫描，
    /// 这里会因为找不到 admin 而放行（回归会立刻暴露）。
    #[tokio::test]
    async fn test_create_second_admin_key_is_rejected_even_with_thousand_user_keys() {
        let mut mock_repo = MockApiKeyRepository::new();
        mock_repo.expect_count_admin_keys().return_once(|| Ok(1));
        // `returning` 而非 `return_once`：修好后 admin 路径不会调它（期望 0 次），
        // 未修好时调 1 次并返回 1000 条 user 行 —— 两种状态都不该因次数校验而 panic。
        mock_repo.expect_list_api_keys().returning(|_, _, _| {
            let rows: Vec<_> = (0..1000)
                .map(|_| make_api_key_info(ApiKeyRole::User, true))
                .collect();
            Ok(rows)
        });
        mock_repo.expect_create_api_key().never();

        let handlers = make_handlers_with_repo(mock_repo);
        let req = CreateApiKeyRequest {
            workspace_id: None,
            name: "second-admin-key".to_string(),
            description: None,
            role: Some("admin".to_string()),
            rate_limit: None,
            expires_at: None,
        };

        let result = handlers.create_api_key(None, req).await;

        match result {
            Err(CoreError::AuthenticationError(msg)) => {
                assert!(
                    msg.to_lowercase().contains("admin"),
                    "message must name the admin-key invariant, got: {msg}"
                );
            }
            other => panic!("expected AuthenticationError, got {other:?}"),
        }
    }

    /// T003：目标行已禁用（不占"最后一个"名额）、另有启用中的 admin 时，吊销必须放行。
    ///
    /// 这条用例把 `count_admin_keys` 只数 enabled 的语义钉住：若守卫退化成按行计数
    /// （把 disabled 行也算进去），这里会误拒。
    #[tokio::test]
    async fn test_revoke_disabled_admin_key_when_another_enabled_admin_exists_is_allowed() {
        let mut mock_repo = MockApiKeyRepository::new();
        mock_repo
            .expect_find_api_key_by_row_id()
            .return_once(move |_| Ok(Some(make_api_key_info(ApiKeyRole::Admin, false))));
        mock_repo.expect_count_admin_keys().return_once(|| Ok(1));
        mock_repo.expect_delete_api_key().return_once(|_| Ok(()));

        let handlers = make_handlers_with_repo(mock_repo);
        let result = handlers.revoke_api_key(Uuid::new_v4()).await;

        assert!(
            result.is_ok(),
            "disabled admin row must not count as the last admin key, got {result:?}"
        );
    }
}
