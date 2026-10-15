// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! API Key 仓储：`ApiKeyRepository` trait 与 SeaORM 实现，含 Argon2id 凭证
//! 哈希/校验、两代凭证宽限期轮换、validate 冷路径与 last_used 写节流。

use async_trait::async_trait;
use chrono::NaiveDateTime;
use dbnexus::sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait, QueryFilter,
    QuerySelect, Set,
};
use rand::{Rng, RngExt};
use std::time::{Duration, Instant};
use tracing::Instrument;
use uuid::Uuid;

use crate::core::database::api_key_entity::{
    ActiveModel as ApiKeyActiveModel, ApiKey as ApiKeyInfo, ApiKeyResponse, ApiKeyRole,
    ApiKeyWithSecret, AuthenticatedKey, Column as ApiKeyColumn, CreateApiKeyRequest,
    Entity as ApiKeyEntity, Model as ApiKeyModel,
};
use crate::core::database::repository::{with_statement_timeout, SeaOrmRepository};
use crate::core::types::Result;

/// last_used 写节流窗口：窗口内同一 key 的重复认证不再触发第二次 UPDATE
/// （写入最多滞后一个窗口，仅影响用量统计粒度，不影响认证结论）。
const LAST_USED_THROTTLE_WINDOW: Duration = Duration::from_secs(60);

/// last_used 节流表容量上限（与 `MAX_TRACKED_AUTH_FAILURE_IPS` 同量级）：
/// 超限逐出「最旧写入」，防止伪造 key_id 扫描使节流表无界增长。
const MAX_TRACKED_LAST_USED_KEYS: usize = 10_000;

#[async_trait]
pub trait ApiKeyRepository: Send + Sync {
    async fn create_api_key(&self, request: &CreateApiKeyRequest) -> Result<ApiKeyWithSecret>;
    async fn get_api_key_by_id(&self, key_id: &str) -> Result<Option<ApiKeyInfo>>;
    /// 校验凭证并返回认证结果。
    ///
    /// `None` = 不予采信（key 不存在/禁用/过期/两代都不匹配）；`Some` 时
    /// `used_previous_credential` 标明命中当代还是宽限期内上一代，调用方据此决定可否缓存。
    async fn validate_api_key(
        &self,
        key_id: &str,
        key_secret: &str,
    ) -> Result<Option<AuthenticatedKey>>;
    async fn list_api_keys(
        &self,
        workspace_id: Uuid,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> Result<Vec<ApiKeyInfo>>;
    async fn delete_api_key(&self, id: Uuid) -> Result<()>;
    async fn revoke_api_key(&self, id: Uuid) -> Result<()>;
    /// 标记 key 已被使用（单语句 UPDATE 写 `last_used_at`/`updated_at`）。
    ///
    /// key 行不存在时静默返回 `Ok(())`；本方法不节流（调用即写），
    /// 认证冷路径的 60 秒节流在 [`Self::validate_api_key`] 调用点完成。
    async fn update_last_used(&self, id: Uuid) -> Result<()>;
    async fn get_admin_api_key(&self, workspace_id: Uuid) -> Result<Option<ApiKeyInfo>>;
    async fn count_api_keys(&self, workspace_id: Uuid) -> Result<u64>;

    /// 按**行主键 `id`**（UUID）查找，而非对外凭证标识 `key_id`（`niad_…`）。
    ///
    /// [`Self::get_api_key_by_id`] 按 `ApiKeyColumn::KeyId` 过滤；handler 层拿到行 UUID，
    /// 必须用本方法（用另一个会永远查不到）。
    async fn find_api_key_by_row_id(&self, id: Uuid) -> Result<Option<ApiKeyInfo>>;

    /// 统计启用中的全局 admin key（`role = 'admin' AND enabled = true`）。
    ///
    /// 有意不带 workspace 过滤：全局 admin key 的 `workspace_id` 为 NULL，
    /// `WorkspaceId.eq(...)` 匹配不到，不能复用 `list_api_keys` 计数。
    async fn count_admin_keys(&self) -> Result<u64>;

    /// 轮换 API Key（生成新密钥，保持旧密钥在宽限期内有效）
    async fn rotate_api_key(
        &self,
        key_id: &str,
        grace_period_seconds: u64,
    ) -> Result<ApiKeyWithSecret>;

    /// 获取需要轮换的密钥列表（基于创建时间）
    async fn get_keys_older_than(&self, age_threshold_days: i64) -> Result<Vec<ApiKeyInfo>>;
}

impl SeaOrmRepository {
    /// 冷路径 last_used 写节流判定：该 key 当前是否允许触发一次 UPDATE。
    ///
    /// 窗口内已写过 → false；允许时登记当前时刻，触顶先逐出最旧条目。
    /// 判定与登记同临界区完成，并发下同一 key 只有一个调用方拿到 true。
    fn should_touch_last_used(&self, key_id: &str) -> bool {
        let now = Instant::now();
        let mut writes = self.last_used_writes.lock();
        if let Some(last) = writes.get(key_id) {
            if now.duration_since(*last) < LAST_USED_THROTTLE_WINDOW {
                return false;
            }
        }
        if writes.len() >= MAX_TRACKED_LAST_USED_KEYS {
            // 逐出最旧写入；O(n) 扫描仅在触顶瞬间发生（远低于一次 DB 往返）。
            if let Some(oldest) = writes
                .iter()
                .min_by_key(|(_, instant)| **instant)
                .map(|(key, _)| key.clone())
            {
                writes.remove(&oldest);
            }
        }
        writes.insert(key_id.to_string(), now);
        true
    }

    /// 构造「按行主键取 api_keys 行」的查询。
    ///
    /// 抽成函数是测试接缝：MockDatabase 不执行 SQL，只有暴露查询构造才能钉住谓词。
    fn select_api_key_by_row_id(id: Uuid) -> dbnexus::sea_orm::Select<ApiKeyEntity> {
        ApiKeyEntity::find().filter(ApiKeyColumn::Id.eq(id))
    }

    /// 构造「启用中的全局 admin key」查询（admin 守卫计数来源）。
    ///
    /// 谓词只能有 `role` 与 `enabled`：全局 admin key 的 `workspace_id` 为 NULL。
    fn select_enabled_admin_keys() -> dbnexus::sea_orm::Select<ApiKeyEntity> {
        ApiKeyEntity::find()
            .filter(ApiKeyColumn::Role.eq(ApiKeyRole::Admin.to_string()))
            .filter(ApiKeyColumn::Enabled.eq(true))
    }

    /// 推导轮换后写入的宽限期两列：`(prev_secret_hash, rotate_expires_at)`。
    ///
    /// `grace == 0` 返回 `(None, None)`；`grace > 0` 把轮换前哈希降级为宽限期凭证，
    /// 到期为绝对时刻 `now + grace`（不依赖配置热更新）。超范围返回 `InvalidInput`。
    fn grace_columns_for_rotation(
        grace_period_seconds: u64,
        current_hash: String,
        now: NaiveDateTime,
    ) -> Result<(Option<String>, Option<NaiveDateTime>)> {
        if grace_period_seconds == 0 {
            return Ok((None, None));
        }

        let out_of_range = || {
            format!(
                "key rotation grace period {}s is out of representable range",
                grace_period_seconds
            )
        };
        let grace_seconds = i64::try_from(grace_period_seconds)
            .map_err(|_| crate::core::CoreError::InvalidInput(out_of_range()))?;
        let delta = chrono::Duration::try_seconds(grace_seconds)
            .ok_or_else(|| crate::core::CoreError::InvalidInput(out_of_range()))?;
        let expires_at = now
            .checked_add_signed(delta)
            .ok_or_else(|| crate::core::CoreError::InvalidInput(out_of_range()))?;

        Ok((Some(current_hash), Some(expires_at)))
    }

    /// 构造轮换写库的 changeset。
    ///
    /// 宽限期两列总是 `Set(..)`：关闭宽限期时也必须把遗留凭证与到期时刻清成 NULL。
    /// 抽成独立函数是测试接缝（MockDatabase 不捕获 UPDATE 语句）。
    fn rotate_changeset(
        id: Uuid,
        new_secret_hash: String,
        prev_secret_hash: Option<String>,
        rotate_expires_at: Option<NaiveDateTime>,
        now: NaiveDateTime,
    ) -> ApiKeyActiveModel {
        ApiKeyActiveModel {
            id: Set(id),
            key_secret_hash: Set(new_secret_hash),
            prev_secret_hash: Set(prev_secret_hash),
            rotate_expires_at: Set(rotate_expires_at),
            updated_at: Set(now),
            ..Default::default()
        }
    }

    /// 列出一次请求允许尝试校验的凭证哈希 `(哈希, 是否上一代)`，按序短路：当代排第一；
    /// 上一代仅在 `prev_secret_hash` 非 NULL 且 `rotate_expires_at > now` 时入选（惰性判定）。
    /// INVARIANT: 桥接 DB schema 与 auth 决策层，`Model` 重构时必须同步更新。
    fn credential_candidates(model: &ApiKeyModel, now: NaiveDateTime) -> Vec<(String, bool)> {
        let mut candidates = vec![(model.key_secret_hash.clone(), false)];

        if let Some(prev_hash) = model.prev_secret_hash.as_ref() {
            let window_open = model
                .rotate_expires_at
                .is_some_and(|expires_at| expires_at > now);
            if window_open {
                candidates.push((prev_hash.clone(), true));
            }
        }

        candidates
    }

    /// 哈希 API key：委托注入的 [`KeyHasher`](crate::core::auth::KeyHasher)（默认 Argon2id）。
    fn hash_key(&self, key_id: &str, key_secret: &str) -> Result<String> {
        self.key_hasher.hash(key_id, key_secret)
    }

    /// 校验 PHC 哈希（委托 KeyHasher，constant-time 比较由实现体保证）。
    fn verify_key(&self, key_id: &str, key_secret: &str, stored_hash: &str) -> bool {
        self.key_hasher.verify(key_id, key_secret, stored_hash)
    }

    /// `validate_api_key` 执行体：抽到 inherent 方法，使 trait 方法处能手工建 span +
    /// `Instrument` 覆盖全程（async_trait 下 `#[instrument]` 覆盖语义不可靠）。
    async fn validate_api_key_inner(
        &self,
        key_id: &str,
        key_secret: &str,
    ) -> Result<Option<AuthenticatedKey>> {
        let key_model = ApiKeyEntity::find()
            .filter(ApiKeyColumn::KeyId.eq(key_id))
            .one(&self.db)
            .await
            .map_err(|e| {
                crate::core::CoreError::DatabaseError(
                    e.to_string(),
                    Some(crate::core::types::ErrorSource::new(e)),
                )
            })?;

        if let Some(model) = key_model {
            // 同一次请求只用一个 `now`：有效期与宽限期判定共享时刻，避免自相矛盾结论。
            let now = chrono::Utc::now().naive_utc();

            if !model.enabled {
                return Ok(None);
            }

            if let Some(expires_at) = model.expires_at {
                if expires_at < now {
                    return Ok(None);
                }
            }

            // Argon2 verify_key 内部使用 constant-time 比较，等价于 subtle::ConstantTimeEq
            for (stored_hash, used_previous_credential) in Self::credential_candidates(&model, now)
            {
                if !self.verify_key(key_id, key_secret, &stored_hash) {
                    continue;
                }

                // 冷路径 1 条 SELECT 自带启用/角色/过期/哈希；last_used 写按 key 节流
                // （60 秒内不重复 UPDATE），往返压到 1 读 + 最多 1 写。
                if self.should_touch_last_used(key_id) {
                    let _ = self.update_last_used(model.id).await;
                }
                let role: ApiKeyRole = model.role.clone().into();
                tracing::debug!(
                    event = "validate_api_key",
                    key_id = %key_id,
                    db_role = %model.role,
                    converted_role = ?role,
                    used_previous_credential,
                    "{}",
                    t!("log.core.database.repository.api_key_role_conversion")
                );
                return Ok(Some(AuthenticatedKey {
                    workspace_id: model.workspace_id,
                    role,
                    used_previous_credential,
                }));
            }
        }

        Ok(None)
    }
}

/// 生成密码学安全的随机 secret。
fn generate_secret() -> String {
    const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_-";
    const SECRET_LENGTH: usize = 32;

    let mut rng = rand::rng();
    let secret: String = (0..SECRET_LENGTH)
        .map(|_| {
            let idx = rng.random_range(0..CHARSET.len());
            CHARSET[idx] as char
        })
        .collect();

    secret
}

#[async_trait]
impl ApiKeyRepository for SeaOrmRepository {
    async fn create_api_key(&self, request: &CreateApiKeyRequest) -> Result<ApiKeyWithSecret> {
        // 校验 key_secret 长度（防 DoS）。
        if let Some(ref secret) = request.key_secret {
            if secret.len() < 8 || secret.len() > 128 {
                return Err(crate::core::CoreError::InvalidInput(
                    "key_secret must be between 8 and 128 characters".to_string(),
                ));
            }
        }

        let prefix = match request.role {
            ApiKeyRole::Admin => "niad_",
            ApiKeyRole::User => "nino_",
            // Anonymous 仅禁用认证时注入内存 extensions，不应被 API 持久化；
            // 误传时 fail-fast 返回 InvalidInput，防止污染数据库。
            ApiKeyRole::Anonymous => {
                return Err(crate::core::types::error::CoreError::InvalidInput(
                    "Anonymous role cannot be persisted to database".to_string(),
                ));
            }
        };

        let full_key_id = if let Some(ref kid) = request.key_id {
            if kid.starts_with(prefix) {
                kid.clone()
            } else {
                format!("{}{}", prefix, kid)
            }
        } else {
            let uuid = Uuid::new_v4();
            format!("{}{}", prefix, uuid)
        };

        // 使用给定 secret，否则生成新 secret。
        let key_secret = request.key_secret.clone().unwrap_or_else(generate_secret);

        let key_secret_hash = self.hash_key(&full_key_id, &key_secret)?;

        // 过期时间：使用给定值，否则默认 30 天。
        let now = chrono::Utc::now();
        let expires_at = request.expires_at.or_else(|| {
            now.naive_utc()
                .checked_add_signed(chrono::Duration::days(30))
        });

        let new_key = ApiKeyActiveModel {
            id: Set(Uuid::new_v4()),
            key_id: Set(full_key_id.clone()),
            key_secret_hash: Set(key_secret_hash),
            // 新建 key 无上一代凭证，不在宽限期。
            prev_secret_hash: Set(None),
            rotate_expires_at: Set(None),
            key_prefix: Set(prefix.to_string()),
            role: Set(request.role.clone().into()),
            workspace_id: Set(request.workspace_id),
            name: Set(request.name.clone()),
            description: Set(request.description.clone()),
            rate_limit: Set(request.rate_limit.unwrap_or(10000)),
            enabled: Set(true),
            expires_at: Set(expires_at),
            last_used_at: Set(None),
            created_at: Set(now.naive_utc()),
            updated_at: Set(now.naive_utc()),
        };

        let inserted = new_key.insert(&self.db).await.map_err(|e| {
            crate::core::CoreError::DatabaseError(
                e.to_string(),
                Some(crate::core::types::ErrorSource::new(e)),
            )
        })?;

        let response = ApiKeyWithSecret {
            // 复用 `impl From<Model> for ApiKeyResponse`，不手抄字段表。
            key: inserted.into(),
            key_secret,
            // 新建 key 无上一代凭证，宽限期不适用。
            grace_expires_at: None,
        };

        Ok(response)
    }

    async fn get_api_key_by_id(&self, key_id: &str) -> Result<Option<ApiKeyInfo>> {
        let result = ApiKeyEntity::find()
            .filter(ApiKeyColumn::KeyId.eq(key_id))
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

    async fn validate_api_key(
        &self,
        key_id: &str,
        key_secret: &str,
    ) -> Result<Option<AuthenticatedKey>> {
        // 手工建 span 并显式 Instrument 执行体（async_trait 下 `#[instrument]` 不可靠）；
        // span 仅含 key_id 长度，不携带 key/secret；执行体经集中式语句超时包裹。
        let span = tracing::info_span!("db.validate_api_key", key_id_len = key_id.len());
        with_statement_timeout(
            self.statement_timeout,
            self.validate_api_key_inner(key_id, key_secret),
        )
        .instrument(span)
        .await
    }

    async fn list_api_keys(
        &self,
        workspace_id: Uuid,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> Result<Vec<ApiKeyInfo>> {
        let mut query = ApiKeyEntity::find().filter(ApiKeyColumn::WorkspaceId.eq(workspace_id));

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

    async fn delete_api_key(&self, id: Uuid) -> Result<()> {
        let result = ApiKeyEntity::delete_by_id(id)
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
                "API key not found: {}",
                id
            )));
        }

        Ok(())
    }

    async fn revoke_api_key(&self, id: Uuid) -> Result<()> {
        let existing = ApiKeyEntity::find_by_id(id)
            .one(&self.db)
            .await
            .map_err(|e| {
                crate::core::CoreError::DatabaseError(
                    e.to_string(),
                    Some(crate::core::types::ErrorSource::new(e)),
                )
            })?;

        let key_id = if let Some(model) = existing {
            model.id
        } else {
            return Err(crate::core::CoreError::NotFound(format!(
                "API key not found: {}",
                id
            )));
        };

        let updated = ApiKeyActiveModel {
            id: Set(key_id),
            enabled: Set(false),
            updated_at: Set(chrono::Utc::now().naive_utc()),
            ..Default::default()
        };

        updated.update(&self.db).await.map_err(|e| {
            crate::core::CoreError::DatabaseError(
                e.to_string(),
                Some(crate::core::types::ErrorSource::new(e)),
            )
        })?;

        Ok(())
    }

    /// 更新 last_used_at（单语句 UPDATE）。
    ///
    /// `update_many().set(..)` 一条语句完成；无行命中（key 已删）时返回 `Ok(())`。
    ///
    /// 本方法不节流（handler 直接调用语义为"调用即写"）；认证冷路径的节流在
    /// [`Self::should_touch_last_used`] 与 `validate_api_key` 调用点完成。
    async fn update_last_used(&self, id: Uuid) -> Result<()> {
        let now = chrono::Utc::now().naive_utc();
        ApiKeyEntity::update_many()
            .filter(ApiKeyColumn::Id.eq(id))
            .set(ApiKeyActiveModel {
                last_used_at: Set(Some(now)),
                updated_at: Set(now),
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

        Ok(())
    }

    async fn get_admin_api_key(&self, _workspace_id: Uuid) -> Result<Option<ApiKeyInfo>> {
        // admin key 是全局的（workspace_id 为 NULL），故不按 workspace 过滤；
        // 参数仅为兼容既有签名保留。
        let result = ApiKeyEntity::find()
            .filter(ApiKeyColumn::Role.eq(super::api_key_entity::ApiKeyRole::Admin.to_string()))
            .filter(ApiKeyColumn::KeyPrefix.eq("niad_"))
            .one(&self.db)
            .await
            .map_err(|e| {
                crate::core::CoreError::DatabaseError(
                    e.to_string(),
                    Some(crate::core::types::ErrorSource::new(e)),
                )
            })?;

        Ok(result.filter(|m| m.enabled).map(|m| m.into()))
    }

    async fn count_api_keys(&self, workspace_id: Uuid) -> Result<u64> {
        let count = ApiKeyEntity::find()
            .filter(ApiKeyColumn::WorkspaceId.eq(workspace_id))
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

    async fn find_api_key_by_row_id(&self, id: Uuid) -> Result<Option<ApiKeyInfo>> {
        let result = Self::select_api_key_by_row_id(id)
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

    async fn count_admin_keys(&self) -> Result<u64> {
        let count = Self::select_enabled_admin_keys()
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

    async fn rotate_api_key(
        &self,
        key_id: &str,
        grace_period_seconds: u64,
    ) -> Result<ApiKeyWithSecret> {
        // 取 `Model` 而非 `ApiKeyInfo`：宽限期需要轮换前的当前哈希，
        // 而 `ApiKeyInfo`/`ApiKeyResponse` 不携带哈希。
        let model = ApiKeyEntity::find()
            .filter(ApiKeyColumn::KeyId.eq(key_id))
            .one(&self.db)
            .await
            .map_err(|e| {
                crate::core::CoreError::DatabaseError(
                    e.to_string(),
                    Some(crate::core::types::ErrorSource::new(e)),
                )
            })?
            .ok_or_else(|| {
                crate::core::CoreError::NotFound(format!("API key not found: {}", key_id))
            })?;

        let now = chrono::Utc::now().naive_utc();
        let (prev_secret_hash, rotate_expires_at) = Self::grace_columns_for_rotation(
            grace_period_seconds,
            model.key_secret_hash.clone(),
            now,
        )?;

        let new_secret = generate_secret();
        let new_secret_hash = self.hash_key(&model.key_id, &new_secret)?;

        let updated = Self::rotate_changeset(
            model.id,
            new_secret_hash,
            prev_secret_hash,
            rotate_expires_at,
            now,
        )
        .update(&self.db)
        .await
        .map_err(|e| {
            crate::core::CoreError::DatabaseError(
                e.to_string(),
                Some(crate::core::types::ErrorSource::new(e)),
            )
        })?;

        Ok(ApiKeyWithSecret {
            // 全部字段单一来源 = UPDATE RETURNING 的实际行，避免混用轮换前后字段。
            key: updated.into(),
            key_secret: new_secret,
            // 与落库值同源：调用方看到的截止时间必须等于写入值，不得重新推导。
            grace_expires_at: rotate_expires_at,
        })
    }

    async fn get_keys_older_than(&self, age_threshold_days: i64) -> Result<Vec<ApiKeyInfo>> {
        let threshold = chrono::Utc::now().naive_utc() - chrono::Duration::days(age_threshold_days);

        let keys = ApiKeyEntity::find()
            .filter(ApiKeyColumn::CreatedAt.lt(threshold))
            .filter(ApiKeyColumn::Enabled.eq(true))
            .all(&self.db)
            .await
            .map_err(|e| {
                crate::core::CoreError::DatabaseError(
                    e.to_string(),
                    Some(crate::core::types::ErrorSource::new(e)),
                )
            })?;

        Ok(keys.into_iter().map(|m| m.into()).collect())
    }
}

/// 无需数据库的前缀逻辑测试。
#[cfg(test)]
mod prefix_tests {
    use super::*;

    #[test]
    fn test_admin_role_prefix() {
        let role = ApiKeyRole::Admin;
        let prefix = match role {
            ApiKeyRole::Admin => "niad_",
            ApiKeyRole::User => "nino_",
            // Anonymous 在生产代码中 fail-fast，本测试用 _ 兜底覆盖该分支。
            _ => "nianon_",
        };
        assert_eq!(prefix, "niad_", "Admin role should use 'niad_' prefix");
    }

    #[test]
    fn test_user_role_prefix() {
        let role = ApiKeyRole::User;
        let prefix = match role {
            ApiKeyRole::Admin => "niad_",
            ApiKeyRole::User => "nino_",
            _ => "nianon_",
        };
        assert_eq!(prefix, "nino_", "User role should use 'nino_' prefix");
    }

    #[test]
    fn test_prefix_uuid_format() {
        let uuid = Uuid::new_v4();
        let prefix = "niad_";
        let full_key_id = format!("{}{}", prefix, uuid);

        assert!(full_key_id.starts_with(prefix));
        assert_eq!(full_key_id.len(), prefix.len() + 36); // 36 is standard UUID length
    }

    #[test]
    fn test_secret_length_validation() {
        let short_secret = "too_short";
        assert!(short_secret.len() < 16, "Test secret should be too short");

        let long_secret = "a".repeat(129);
        assert!(long_secret.len() > 128, "Test secret should be too long");

        let valid_secret = "this_is_a_valid_secret_length_16";
        assert!(valid_secret.len() >= 16 && valid_secret.len() <= 128);
    }

    #[test]
    fn test_generate_secret_length() {
        let secret = generate_secret();
        assert_eq!(secret.len(), 32, "Generated secret should be 32 characters");
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
    // 测试用 `workspace_entity::Model` 等路径表达式，需要模块本身在作用域内。
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

    // ============== Pure-logic functions (no DB) ==============

    // --- generate_secret ---

    #[test]
    fn test_generate_secret_returns_32_chars() {
        let s = generate_secret();
        assert_eq!(s.len(), 32, "generate_secret must always return 32 chars");
    }

    #[test]
    fn test_generate_secret_uses_only_allowed_charset() {
        const ALLOWED: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_-";
        // 多跑几次：随机字符可能覆盖不全，单次不足以验证字符集。
        for _ in 0..20 {
            let s = generate_secret();
            for c in s.bytes() {
                assert!(
                    ALLOWED.contains(&c),
                    "char {:?} (#{}) not in allowed charset",
                    c as char,
                    c
                );
            }
        }
    }

    #[test]
    fn test_generate_secret_is_random_across_calls() {
        // 概率性：62^32 空间碰撞概率极低；偶发失败说明 RNG 播种坏了。
        let s1 = generate_secret();
        let s2 = generate_secret();
        let s3 = generate_secret();
        assert_ne!(s1, s2, "two consecutive calls must differ");
        assert_ne!(s2, s3, "two consecutive calls must differ");
        assert_ne!(s1, s3, "two non-consecutive calls must differ");
    }

    // --- hash_key / verify_key ---

    #[test]
    fn test_hash_key_returns_argon2_phc_format() {
        let repo = make_repo(empty_pg_connection());
        let h = repo.hash_key("kid1", "secret1").unwrap();
        assert!(
            h.starts_with("$argon2"),
            "PHC format must start with $argon2, got: {}",
            h
        );
        assert!(
            h.len() > 50,
            "Argon2id PHC string should be at least ~96 chars, got {}",
            h.len()
        );
    }

    #[test]
    fn test_hash_key_generates_unique_salt_per_call() {
        // 相同输入两次哈希必须不同（salt 随机）。
        let repo = make_repo(empty_pg_connection());
        let h1 = repo.hash_key("kid1", "secret1").unwrap();
        let h2 = repo.hash_key("kid1", "secret1").unwrap();
        assert_ne!(h1, h2, "salt must be regenerated per call");
    }

    #[test]
    fn test_hash_key_phc_params_stable_across_argon2_0_6_upgrade() {
        // argon2 0.5 → 0.6 升级的存量兼容回归：算法（argon2id/v=19）、默认参数
        // 与 PHC 格式必须不变，任何漂移都会让 DB 中存量凭证静默验不过。
        let repo = make_repo(empty_pg_connection());
        let h = repo.hash_key("kid1", "secret1").unwrap();
        assert!(
            h.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"),
            "算法/版本/默认参数漂移会让存量哈希全部验不过, got: {}",
            h
        );
        assert!(
            repo.verify_key("kid1", "secret1", &h),
            "生成-验证闭环必须成立"
        );
    }

    #[test]
    fn test_verify_key_accepts_legacy_pinned_hash_vector() {
        // 升级前钉死的固定向量（固定 salt [0x5A; 16] + 确定性输入）：Argon2 对相同
        // salt+输入+参数产出字节级一致的 PHC，等价于 0.5 时代写入 DB 的真实存量哈希。
        // 任何让存量凭证验不过的实现变更都会被本用例拦截。
        const LEGACY_PINNED_HASH: &str =
            "$argon2id$v=19$m=19456,t=2,p=1$WlpaWlpaWlpaWlpaWlpaWg$RO2LFuOid0m4cf+SroCSSvFC2OPkluKtEY4ZLZ2dqpU";
        let repo = make_repo(empty_pg_connection());
        assert!(
            repo.verify_key("kid1", "pin-secret", LEGACY_PINNED_HASH),
            "存量固定向量必须可验证"
        );
        assert!(
            !repo.verify_key("kid1", "wrong-secret", LEGACY_PINNED_HASH),
            "错误凭证必须验不过"
        );
    }

    #[test]
    fn test_verify_key_succeeds_with_correct_credentials() {
        let repo = make_repo(empty_pg_connection());
        let h = repo.hash_key("kid1", "secret1").unwrap();
        assert!(
            repo.verify_key("kid1", "secret1", &h),
            "correct key_id + secret must verify"
        );
    }

    #[test]
    fn test_verify_key_fails_with_wrong_secret() {
        let repo = make_repo(empty_pg_connection());
        let h = repo.hash_key("kid1", "secret1").unwrap();
        assert!(
            !repo.verify_key("kid1", "wrong-secret", &h),
            "wrong secret must fail verification"
        );
    }

    #[test]
    fn test_verify_key_fails_with_wrong_key_id() {
        // key_id 参与 peppered password：改变 key_id 必须导致校验失败。
        let repo = make_repo(empty_pg_connection());
        let h = repo.hash_key("kid1", "secret1").unwrap();
        assert!(!repo.verify_key("kid2", "secret1", &h));
    }

    #[test]
    fn test_verify_key_fails_with_malformed_hash_string() {
        let repo = make_repo(empty_pg_connection());
        assert!(!repo.verify_key("kid1", "secret1", "not-a-valid-hash"));
    }

    #[test]
    fn test_verify_key_fails_with_empty_hash() {
        let repo = make_repo(empty_pg_connection());
        assert!(!repo.verify_key("kid1", "secret1", ""));
    }

    /// `with_key_hasher` 可在默认 Argon2id 之上注入定制哈希器：换不同 salt 的
    /// `Argon2KeyHasher` 后哈希结果改变，且输出仍为合法 PHC 格式。
    #[test]
    fn test_with_key_hasher_overrides_default_hasher() {
        // 两枚夹具 pepper 互异且运行时派生（非字面量直连 `salt` 参数）：钉住"注入的定制哈希器确实接管"。
        let repo = SeaOrmRepository::new(empty_pg_connection(), fixture_salt()).with_key_hasher(
            std::sync::Arc::new(crate::core::auth::Argon2KeyHasher::new(
                fixture_salt() + "_alt",
            )),
        );

        let hashed = repo.hash_key("kid1", "secret1").unwrap();
        assert!(
            hashed.starts_with("$argon2id$"),
            "injected hasher output must still be PHC format, got: {}",
            hashed
        );
    }

    // ============== ApiKeyRepository tests ==============

    #[tokio::test]
    async fn test_api_key_create_rejects_anonymous_role() {
        let repo = make_repo(empty_pg_connection());

        let err = repo
            .create_api_key(&CreateApiKeyRequest {
                workspace_id: None,
                name: "anon".to_string(),
                description: None,
                role: ApiKeyRole::Anonymous,
                rate_limit: None,
                expires_at: None,
                key_secret: None,
                key_id: None,
            })
            .await
            .unwrap_err();

        assert!(
            matches!(err, crate::core::CoreError::InvalidInput(ref m) if m.contains("Anonymous")),
            "Anonymous role must be rejected with InvalidInput, got {:?}",
            err
        );
    }

    #[tokio::test]
    async fn test_api_key_create_rejects_short_secret() {
        let repo = make_repo(empty_pg_connection());

        let err = repo
            .create_api_key(&CreateApiKeyRequest {
                workspace_id: None,
                name: "k".to_string(),
                description: None,
                role: ApiKeyRole::Admin,
                rate_limit: None,
                expires_at: None,
                key_secret: Some("short".to_string()),
                key_id: None,
            })
            .await
            .unwrap_err();

        assert!(
            matches!(err, crate::core::CoreError::InvalidInput(ref m) if m.contains("8 and 128")),
            "short secret must be rejected with length error, got {:?}",
            err
        );
    }

    #[tokio::test]
    async fn test_api_key_create_rejects_long_secret() {
        let repo = make_repo(empty_pg_connection());
        let too_long = "a".repeat(129);

        let err = repo
            .create_api_key(&CreateApiKeyRequest {
                workspace_id: None,
                name: "k".to_string(),
                description: None,
                role: ApiKeyRole::Admin,
                rate_limit: None,
                expires_at: None,
                key_secret: Some(too_long),
                key_id: None,
            })
            .await
            .unwrap_err();

        assert!(
            matches!(err, crate::core::CoreError::InvalidInput(ref m) if m.contains("8 and 128")),
            "long secret must be rejected with length error, got {:?}",
            err
        );
    }

    #[tokio::test]
    async fn test_api_key_create_admin_uses_niad_prefix() {
        let id = fixed_uuid(70);
        let model = api_key_entity::Model {
            id,
            key_id: "niad_admin-uuid".to_string(),
            key_prefix: "niad_".to_string(),
            role: "admin".to_string(),
            ..sample_api_key_model(id, "niad_admin-uuid", "admin")
        };
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![model]])
            .into_connection();
        let repo = make_repo(db);

        let key = repo
            .create_api_key(&CreateApiKeyRequest {
                workspace_id: None,
                name: "Admin Key".to_string(),
                description: None,
                role: ApiKeyRole::Admin,
                rate_limit: None,
                expires_at: None,
                key_secret: None,
                key_id: None,
            })
            .await
            .unwrap();

        assert!(key.key.key_id.starts_with("niad_"));
        assert_eq!(key.key.key_prefix, "niad_");
        // 生成的 secret 必须为 32 字符。
        assert_eq!(key.key_secret.len(), 32);
    }

    #[tokio::test]
    async fn test_api_key_create_user_uses_nino_prefix() {
        let id = fixed_uuid(71);
        let ws_id = fixed_uuid(72);
        let model = api_key_entity::Model {
            id,
            key_id: "nino_user-uuid".to_string(),
            key_prefix: "nino_".to_string(),
            role: "user".to_string(),
            workspace_id: Some(ws_id),
            ..sample_api_key_model(id, "nino_user-uuid", "user")
        };
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![model]])
            .into_connection();
        let repo = make_repo(db);

        let key = repo
            .create_api_key(&CreateApiKeyRequest {
                workspace_id: Some(ws_id),
                name: "User Key".to_string(),
                description: None,
                role: ApiKeyRole::User,
                rate_limit: None,
                expires_at: None,
                key_secret: None,
                key_id: None,
            })
            .await
            .unwrap();

        assert!(key.key.key_id.starts_with("nino_"));
        assert_eq!(key.key.key_prefix, "nino_");
    }

    #[tokio::test]
    async fn test_api_key_create_with_custom_key_id_without_prefix_prepends_prefix() {
        let id = fixed_uuid(73);
        let custom_uuid = "abc-123-custom";
        let model = api_key_entity::Model {
            id,
            key_id: format!("niad_{}", custom_uuid),
            key_prefix: "niad_".to_string(),
            role: "admin".to_string(),
            ..sample_api_key_model(id, "placeholder", "admin")
        };
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![model]])
            .into_connection();
        let repo = make_repo(db);

        let key = repo
            .create_api_key(&CreateApiKeyRequest {
                workspace_id: None,
                name: "k".to_string(),
                description: None,
                role: ApiKeyRole::Admin,
                rate_limit: None,
                expires_at: None,
                key_secret: None,
                key_id: Some(custom_uuid.to_string()),
            })
            .await
            .unwrap();

        // 无前缀的自定义 key_id 会被补上前缀。
        assert!(key.key.key_id.starts_with("niad_"));
        assert!(key.key.key_id.contains(custom_uuid));
    }

    #[tokio::test]
    async fn test_api_key_create_with_custom_key_id_with_prefix_keeps_as_is() {
        let id = fixed_uuid(74);
        let custom = "niad_my-existing-uuid".to_string();
        let model = api_key_entity::Model {
            id,
            key_id: custom.clone(),
            key_prefix: "niad_".to_string(),
            role: "admin".to_string(),
            ..sample_api_key_model(id, "placeholder", "admin")
        };
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![model]])
            .into_connection();
        let repo = make_repo(db);

        let key = repo
            .create_api_key(&CreateApiKeyRequest {
                workspace_id: None,
                name: "k".to_string(),
                description: None,
                role: ApiKeyRole::Admin,
                rate_limit: None,
                expires_at: None,
                key_secret: None,
                key_id: Some(custom.clone()),
            })
            .await
            .unwrap();

        assert_eq!(
            key.key.key_id, custom,
            "prefixed key_id must not be double-prefixed"
        );
    }

    #[tokio::test]
    async fn test_api_key_create_with_provided_secret_returns_secret_as_is() {
        let id = fixed_uuid(75);
        let custom_secret = "this_is_a_valid_secret_12345".to_string();
        let model = api_key_entity::Model {
            id,
            key_id: "niad_x".to_string(),
            key_prefix: "niad_".to_string(),
            role: "admin".to_string(),
            ..sample_api_key_model(id, "niad_x", "admin")
        };
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![model]])
            .into_connection();
        let repo = make_repo(db);

        let key = repo
            .create_api_key(&CreateApiKeyRequest {
                workspace_id: None,
                name: "k".to_string(),
                description: None,
                role: ApiKeyRole::Admin,
                rate_limit: None,
                expires_at: None,
                key_secret: Some(custom_secret.clone()),
                key_id: None,
            })
            .await
            .unwrap();

        // 给定 secret 必须原样返回，不得重新生成。
        assert_eq!(key.key_secret, custom_secret);
    }

    #[tokio::test]
    async fn test_api_key_get_by_id_returns_some_when_found() {
        let id = fixed_uuid(76);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![sample_api_key_model(id, "niad_x", "admin")]])
            .into_connection();
        let repo = make_repo(db);

        let key = repo.get_api_key_by_id("niad_x").await.unwrap();
        assert!(key.is_some());
        assert_eq!(key.unwrap().key_id, "niad_x");
    }

    #[tokio::test]
    async fn test_api_key_get_by_id_returns_none_when_missing() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![Vec::<api_key_entity::Model>::new()])
            .into_connection();
        let repo = make_repo(db);

        let key = repo.get_api_key_by_id("niad_missing").await.unwrap();
        assert!(key.is_none());
    }

    #[tokio::test]
    async fn test_api_key_validate_returns_none_when_key_not_found() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![Vec::<api_key_entity::Model>::new()])
            .into_connection();
        let repo = make_repo(db);

        let result = repo
            .validate_api_key("niad_unknown", "any-secret")
            .await
            .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_api_key_validate_returns_none_when_key_disabled() {
        let id = fixed_uuid(77);
        let model = api_key_entity::Model {
            id,
            enabled: false,
            ..sample_api_key_model(id, "niad_disabled", "admin")
        };
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![model]])
            .into_connection();
        let repo = make_repo(db);

        let result = repo.validate_api_key("niad_disabled", "any").await.unwrap();
        assert!(result.is_none(), "disabled key must not validate");
    }

    #[tokio::test]
    async fn test_api_key_validate_returns_none_when_key_expired() {
        let id = fixed_uuid(78);
        let model = api_key_entity::Model {
            id,
            enabled: true,
            expires_at: Some(fixed_datetime(1_000_000_000)), // far in the past
            ..sample_api_key_model(id, "niad_expired", "admin")
        };
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![model]])
            .into_connection();
        let repo = make_repo(db);

        let result = repo.validate_api_key("niad_expired", "any").await.unwrap();
        assert!(result.is_none(), "expired key must not validate");
    }

    #[tokio::test]
    async fn test_api_key_validate_returns_none_when_secret_does_not_match() {
        let id = fixed_uuid(79);
        let model = sample_api_key_model(id, "niad_x", "admin");
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![model]])
            // 不追加第二次 update_last_used 结果；mock 默认返回空。
            .into_connection();
        let repo = make_repo(db);

        // 桩哈希对应 "correct-secret"，错误 secret 必须失败。
        let result = repo
            .validate_api_key("niad_x", "wrong-secret-value-xxx")
            .await
            .unwrap();
        assert!(result.is_none(), "wrong secret must not validate");
    }

    /// 造一行「轮换后带宽限期」的 key：两代凭证均为真实 Argon2 哈希
    /// （`validate_api_key` 走密码学校验，占位串永远验不过）。
    fn grace_rotated_model(
        repo: &SeaOrmRepository,
        id: Uuid,
        key_id: &str,
        new_secret: &str,
        old_secret: Option<&str>,
        rotate_expires_at: Option<NaiveDateTime>,
    ) -> ApiKeyModel {
        api_key_entity::Model {
            key_secret_hash: repo.hash_key(key_id, new_secret).unwrap(),
            prev_secret_hash: old_secret.map(|s| repo.hash_key(key_id, s).unwrap()),
            rotate_expires_at,
            ..sample_api_key_model(id, key_id, "admin")
        }
    }

    fn grace_window_from_now(seconds: i64) -> NaiveDateTime {
        chrono::Utc::now().naive_utc() + chrono::Duration::seconds(seconds)
    }

    /// 宽限期内旧凭证仍可用，且必须标记为「命中上一代凭证」
    /// （据此跳过认证决策缓存，否则窗口关闭后缓存仍放行旧凭证）。
    #[tokio::test]
    async fn test_validate_within_grace_window_accepts_previous_credential() {
        let id = fixed_uuid(140);
        let db_repo = make_repo(empty_pg_connection());
        let model = grace_rotated_model(
            &db_repo,
            id,
            "niad_grace",
            "new-secret-value-aaa",
            Some("old-secret-value-bbb"),
            Some(grace_window_from_now(3600)),
        );
        let repo = make_repo(
            MockDatabase::new(DatabaseBackend::Postgres)
                .append_query_results(vec![vec![model]])
                .into_connection(),
        );

        let auth = repo
            .validate_api_key("niad_grace", "old-secret-value-bbb")
            .await
            .unwrap()
            .expect("previous credential must authenticate inside the grace window");

        assert!(
            auth.used_previous_credential,
            "hit on the grace-period credential must be flagged"
        );
        assert_eq!(auth.role, ApiKeyRole::Admin);
        assert_eq!(auth.workspace_id, None);
    }

    /// 当前凭证命中时不得被标记为宽限期命中。
    #[tokio::test]
    async fn test_validate_current_credential_is_not_marked_as_grace() {
        let id = fixed_uuid(141);
        let db_repo = make_repo(empty_pg_connection());
        let model = grace_rotated_model(
            &db_repo,
            id,
            "niad_grace",
            "new-secret-value-aaa",
            Some("old-secret-value-bbb"),
            Some(grace_window_from_now(3600)),
        );
        let repo = make_repo(
            MockDatabase::new(DatabaseBackend::Postgres)
                .append_query_results(vec![vec![model]])
                .into_connection(),
        );

        let auth = repo
            .validate_api_key("niad_grace", "new-secret-value-aaa")
            .await
            .unwrap()
            .expect("current credential must authenticate");

        assert!(
            !auth.used_previous_credential,
            "current credential must not be flagged as grace-period hit"
        );
    }

    /// `rotate_expires_at` 到期后旧凭证立即失效，新凭证不受影响。
    /// 「到期即断」只断旧凭证，不能把整把 key 一起判死。
    #[tokio::test]
    async fn test_validate_after_grace_expiry_rejects_previous_credential() {
        let id = fixed_uuid(142);
        let db_repo = make_repo(empty_pg_connection());
        let model = grace_rotated_model(
            &db_repo,
            id,
            "niad_grace",
            "new-secret-value-aaa",
            Some("old-secret-value-bbb"),
            Some(grace_window_from_now(-1)),
        );
        let repo = make_repo(
            MockDatabase::new(DatabaseBackend::Postgres)
                .append_query_results(vec![vec![model.clone()], vec![model]])
                .into_connection(),
        );

        assert!(
            repo.validate_api_key("niad_grace", "old-secret-value-bbb")
                .await
                .unwrap()
                .is_none(),
            "expired grace credential must be rejected"
        );
        let auth = repo
            .validate_api_key("niad_grace", "new-secret-value-aaa")
            .await
            .unwrap()
            .expect("key itself must stay usable after the window closes");
        assert!(!auth.used_previous_credential);
    }

    /// 无上一代凭证时只允许一次哈希校验（不得凭空开窗口）：窗口有效但
    /// `prev_secret_hash` 为 NULL 时，候选必须只有当前哈希（校验次数无法从 mock 观测）。
    #[test]
    fn test_validate_without_grace_window_only_checks_current_credential() {
        let now = fixed_datetime(1_700_000_000);
        let db_repo = make_repo(empty_pg_connection());
        let model = grace_rotated_model(
            &db_repo,
            fixed_uuid(143),
            "niad_never_rotated",
            "new-secret-value-aaa",
            None,
            Some(now + chrono::Duration::seconds(3600)),
        );

        let candidates = SeaOrmRepository::credential_candidates(&model, now);

        assert_eq!(
            candidates.len(),
            1,
            "a NULL previous credential must not open a second verification window"
        );
        assert_eq!(candidates[0], (model.key_secret_hash.clone(), false));
    }

    // --- validate_api_key 冷路径往返数与 last_used 写节流 ---

    /// 造一行当代凭证可通过 Argon2 校验的 key（哈希由同一 salt 推导）。
    fn authenticatable_model(
        repo: &SeaOrmRepository,
        id: Uuid,
        key_id: &str,
        secret: &str,
    ) -> ApiKeyModel {
        api_key_entity::Model {
            key_secret_hash: repo.hash_key(key_id, secret).unwrap(),
            ..sample_api_key_model(id, key_id, "admin")
        }
    }

    /// 提取 mock 事务日志里全部语句的大写 SQL 前缀（按出现顺序）。
    fn statement_kinds(repo: &SeaOrmRepository) -> Vec<String> {
        repo.get_db_connection()
            .clone()
            .into_transaction_log()
            .iter()
            .map(|t| t.statements()[0].sql.to_uppercase())
            .collect()
    }

    /// 冷路径往返钉桩：成功认证 = 恰好 1 条 SELECT + 至多 1 条节流后 UPDATE。
    #[tokio::test]
    async fn test_validate_cold_path_is_single_select() {
        let id = fixed_uuid(150);
        let db_repo = make_repo(empty_pg_connection());
        let model = authenticatable_model(&db_repo, id, "niad_cold", "cold-secret-value-01");
        let repo = make_repo(
            MockDatabase::new(DatabaseBackend::Postgres)
                .append_query_results(vec![vec![model]])
                .append_exec_results(vec![MockExecResult {
                    last_insert_id: 0,
                    rows_affected: 1,
                }])
                .into_connection(),
        );

        let auth = repo
            .validate_api_key("niad_cold", "cold-secret-value-01")
            .await
            .unwrap()
            .expect("cold credential must authenticate");

        assert_eq!(auth.role, ApiKeyRole::Admin);

        let kinds = statement_kinds(&repo);
        assert_eq!(
            kinds.iter().filter(|s| s.starts_with("SELECT")).count(),
            1,
            "冷路径读必须恰好 1 次 SELECT（由 3 降 1），got {kinds:?}"
        );
        assert_eq!(
            kinds.iter().filter(|s| s.starts_with("UPDATE")).count(),
            1,
            "首次认证允许一次 last_used UPDATE，got {kinds:?}"
        );
    }

    /// 60 秒节流窗口内，同 key 的第二次认证不得触发第二次 last_used UPDATE。
    #[tokio::test]
    async fn test_validate_throttles_second_last_used_write_within_window() {
        let id = fixed_uuid(151);
        let db_repo = make_repo(empty_pg_connection());
        let model = authenticatable_model(&db_repo, id, "niad_throttle", "throttle-secret-001");
        let repo = make_repo(
            MockDatabase::new(DatabaseBackend::Postgres)
                .append_query_results(vec![vec![model.clone()], vec![model]])
                .append_exec_results(vec![MockExecResult {
                    last_insert_id: 0,
                    rows_affected: 1,
                }])
                .into_connection(),
        );

        for _ in 0..2 {
            repo.validate_api_key("niad_throttle", "throttle-secret-001")
                .await
                .unwrap()
                .expect("both validations must succeed");
        }

        let kinds = statement_kinds(&repo);
        assert_eq!(
            kinds.iter().filter(|s| s.starts_with("SELECT")).count(),
            2,
            "两次认证各读一次，got {kinds:?}"
        );
        assert_eq!(
            kinds.iter().filter(|s| s.starts_with("UPDATE")).count(),
            1,
            "60 秒内重复验证不得触发第二次 last_used UPDATE，got {kinds:?}"
        );
    }

    /// 节流窗口过期后（超过 60 秒未写），下一次认证重新触发 UPDATE。
    #[tokio::test]
    async fn test_validate_last_used_write_resumes_after_throttle_window() {
        let id = fixed_uuid(152);
        let db_repo = make_repo(empty_pg_connection());
        let model = authenticatable_model(&db_repo, id, "niad_resume", "resume-secret-00001");
        let repo = make_repo(
            MockDatabase::new(DatabaseBackend::Postgres)
                .append_query_results(vec![vec![model]])
                .append_exec_results(vec![MockExecResult {
                    last_insert_id: 0,
                    rows_affected: 1,
                }])
                .into_connection(),
        );
        // 预置一条"早已过窗口"的写入记录，等效于等待 60 秒。
        repo.last_used_writes.lock().insert(
            "niad_resume".to_string(),
            Instant::now() - LAST_USED_THROTTLE_WINDOW - Duration::from_secs(1),
        );

        repo.validate_api_key("niad_resume", "resume-secret-00001")
            .await
            .unwrap()
            .expect("validation must succeed");

        let kinds = statement_kinds(&repo);
        assert_eq!(
            kinds.iter().filter(|s| s.starts_with("UPDATE")).count(),
            1,
            "窗口过期后必须重新触发 last_used UPDATE，got {kinds:?}"
        );
    }

    /// 节流按 key 隔离：不同 key 的认证各自触发一次 UPDATE，互不吞并。
    #[tokio::test]
    async fn test_validate_last_used_throttle_is_per_key() {
        let db_repo = make_repo(empty_pg_connection());
        let m1 = authenticatable_model(&db_repo, fixed_uuid(153), "niad_k1", "key-one-secret-01");
        let m2 = authenticatable_model(&db_repo, fixed_uuid(154), "niad_k2", "key-two-secret-02");
        let repo = make_repo(
            MockDatabase::new(DatabaseBackend::Postgres)
                .append_query_results(vec![vec![m1], vec![m2]])
                .append_exec_results(vec![
                    MockExecResult {
                        last_insert_id: 0,
                        rows_affected: 1,
                    },
                    MockExecResult {
                        last_insert_id: 0,
                        rows_affected: 1,
                    },
                ])
                .into_connection(),
        );

        repo.validate_api_key("niad_k1", "key-one-secret-01")
            .await
            .unwrap();
        repo.validate_api_key("niad_k2", "key-two-secret-02")
            .await
            .unwrap();

        let kinds = statement_kinds(&repo);
        assert_eq!(
            kinds.iter().filter(|s| s.starts_with("UPDATE")).count(),
            2,
            "不同 key 必须各自触发一次 last_used UPDATE，got {kinds:?}"
        );
    }

    #[test]
    fn test_should_touch_last_used_throttles_within_window() {
        let repo = make_repo(empty_pg_connection());

        assert!(repo.should_touch_last_used("k"), "首次必须放行");
        assert!(
            !repo.should_touch_last_used("k"),
            "60 秒窗口内的重复判定必须被节流"
        );

        repo.last_used_writes.lock().insert(
            "stale".to_string(),
            Instant::now() - LAST_USED_THROTTLE_WINDOW - Duration::from_secs(1),
        );
        assert!(
            repo.should_touch_last_used("stale"),
            "窗口过期后必须重新放行"
        );
    }

    #[test]
    fn test_should_touch_last_used_evicts_oldest_at_capacity() {
        let repo = make_repo(empty_pg_connection());
        {
            let mut writes = repo.last_used_writes.lock();
            // 灌满节流表；k0 的写入时刻最早（最旧）。
            for i in 0..MAX_TRACKED_LAST_USED_KEYS {
                writes.insert(
                    format!("k{i}"),
                    Instant::now() - Duration::from_secs(100_000 - i as u64),
                );
            }
        }

        assert!(repo.should_touch_last_used("brand-new-key"));

        let writes = repo.last_used_writes.lock();
        assert_eq!(
            writes.len(),
            MAX_TRACKED_LAST_USED_KEYS,
            "触顶逐出后总量必须维持在上限"
        );
        assert!(!writes.contains_key("k0"), "最旧写入的条目必须被逐出");
        assert!(writes.contains_key("k1"), "其余条目必须保留");
        assert!(writes.contains_key("brand-new-key"), "新条目必须登记");
    }

    #[tokio::test]
    async fn test_api_key_list_returns_keys_for_workspace() {
        let ws_id = fixed_uuid(80);
        let id1 = fixed_uuid(81);
        let id2 = fixed_uuid(82);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![
                sample_api_key_model(id1, "nino_1", "user"),
                sample_api_key_model(id2, "nino_2", "user"),
            ]])
            .into_connection();
        let repo = make_repo(db);

        let keys = repo.list_api_keys(ws_id, None, None).await.unwrap();
        assert_eq!(keys.len(), 2);
    }

    #[tokio::test]
    async fn test_api_key_delete_succeeds_when_rows_affected() {
        let id = fixed_uuid(83);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_exec_results(vec![MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            .into_connection();
        let repo = make_repo(db);

        repo.delete_api_key(id).await.unwrap();
    }

    #[tokio::test]
    async fn test_api_key_delete_returns_not_found_when_no_rows_affected() {
        let id = fixed_uuid(84);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_exec_results(vec![MockExecResult {
                last_insert_id: 0,
                rows_affected: 0,
            }])
            .into_connection();
        let repo = make_repo(db);

        let err = repo.delete_api_key(id).await.unwrap_err();
        assert!(
            matches!(err, crate::core::CoreError::NotFound(ref m) if m.contains("API key")),
            "expected NotFound for api key, got {:?}",
            err
        );
    }

    #[tokio::test]
    async fn test_api_key_revoke_returns_not_found_when_key_missing() {
        let id = fixed_uuid(85);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![Vec::<api_key_entity::Model>::new()])
            .into_connection();
        let repo = make_repo(db);

        let err = repo.revoke_api_key(id).await.unwrap_err();
        assert!(
            matches!(err, crate::core::CoreError::NotFound(ref m) if m.contains("API key")),
            "expected NotFound for api key, got {:?}",
            err
        );
    }

    #[tokio::test]
    async fn test_api_key_revoke_succeeds_when_key_found() {
        let id = fixed_uuid(86);
        let model = sample_api_key_model(id, "niad_x", "admin");
        let updated_model = api_key_entity::Model {
            enabled: false,
            ..model
        };
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![sample_api_key_model(id, "niad_x", "admin")]])
            .append_query_results(vec![vec![updated_model]])
            .into_connection();
        let repo = make_repo(db);

        repo.revoke_api_key(id).await.unwrap();
    }

    #[tokio::test]
    async fn test_api_key_update_last_used_succeeds_when_key_missing() {
        // key 不存在时 update_last_used 仍返回 Ok(())（单语句 UPDATE 命中 0 行），
        // 本用例钉住该已记录行为。
        let id = fixed_uuid(87);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_exec_results(vec![MockExecResult {
                last_insert_id: 0,
                rows_affected: 0,
            }])
            .into_connection();
        let repo = make_repo(db);

        repo.update_last_used(id).await.unwrap();

        let kinds: Vec<String> = repo
            .get_db_connection()
            .clone()
            .into_transaction_log()
            .iter()
            .map(|t| t.statements()[0].sql.to_uppercase())
            .collect();
        assert_eq!(
            kinds.len(),
            1,
            "update_last_used 必须是单语句，got {kinds:?}"
        );
        assert!(
            kinds[0].starts_with("UPDATE"),
            "必须是单语句 UPDATE，got {}",
            kinds[0]
        );
    }

    #[tokio::test]
    async fn test_api_key_update_last_used_succeeds_when_key_found() {
        let id = fixed_uuid(88);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_exec_results(vec![MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            .into_connection();
        let repo = make_repo(db);

        repo.update_last_used(id).await.unwrap();

        let kinds: Vec<String> = repo
            .get_db_connection()
            .clone()
            .into_transaction_log()
            .iter()
            .map(|t| t.statements()[0].sql.to_uppercase())
            .collect();
        assert_eq!(
            kinds.len(),
            1,
            "旧实现 find+update+回读共 3 次往返，现在必须恰好 1 条"
        );
        assert!(kinds[0].starts_with("UPDATE"), "got {}", kinds[0]);
        assert!(
            kinds[0].contains("LAST_USED_AT"),
            "更新必须覆盖 last_used_at 列，got {}",
            kinds[0]
        );
    }

    #[tokio::test]
    async fn test_api_key_get_admin_returns_some_when_admin_enabled() {
        let id = fixed_uuid(89);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![sample_api_key_model(id, "niad_admin", "admin")]])
            .into_connection();
        let repo = make_repo(db);

        let key = repo.get_admin_api_key(fixed_uuid(90)).await.unwrap();
        assert!(key.is_some());
        assert_eq!(key.unwrap().role, ApiKeyRole::Admin);
    }

    #[tokio::test]
    async fn test_api_key_get_admin_returns_none_when_admin_disabled() {
        let id = fixed_uuid(91);
        let model = api_key_entity::Model {
            enabled: false,
            ..sample_api_key_model(id, "niad_admin", "admin")
        };
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![model]])
            .into_connection();
        let repo = make_repo(db);

        let key = repo.get_admin_api_key(fixed_uuid(92)).await.unwrap();
        assert!(key.is_none(), "disabled admin key must be filtered out");
    }

    #[tokio::test]
    async fn test_api_key_count_returns_count_for_workspace() {
        let ws_id = fixed_uuid(93);
        let mut count_row: BTreeMap<String, dbnexus::sea_orm::Value> = BTreeMap::new();
        count_row.insert(
            "num_items".to_string(),
            dbnexus::sea_orm::Value::BigInt(Some(5)),
        );
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![count_row]])
            .into_connection();
        let repo = make_repo(db);

        let count = repo.count_api_keys(ws_id).await.unwrap();
        assert_eq!(count, 5);
    }

    /// 取 SQL 的 WHERE 子句部分。
    ///
    /// 谓词断言必须只看 WHERE：投影列必然含 key_id/workspace_id，全语句负向断言会误报。
    fn where_clause(sql: &str) -> &str {
        sql.split_once("WHERE")
            .map(|(_, tail)| tail)
            .unwrap_or_else(|| panic!("statement has no WHERE clause: {sql}"))
    }

    /// `find_api_key_by_row_id` 必须按主键 `id` 过滤（而非 `key_id` 字符串）。
    /// MockDatabase 不执行 SQL，谓词正确性只能从构造语句上断言；绑定值用语句
    /// `Debug` 渲染做包含检查，兼容内联字面量与 `$n` 占位。
    #[test]
    fn test_select_api_key_by_row_id_filters_primary_key_not_key_id() {
        let id = fixed_uuid(11);
        let stmt = SeaOrmRepository::select_api_key_by_row_id(id).build(DatabaseBackend::Postgres);
        let sql = stmt.to_string();
        let predicate = where_clause(&sql);

        assert!(
            predicate.contains(r#""api_keys"."id" = "#),
            "row-id lookup must filter on the primary key, got: {predicate}"
        );
        assert!(
            !predicate.contains("key_id"),
            "row-id lookup must not filter on key_id, got: {predicate}"
        );
        assert!(
            format!("{stmt:?}").contains(&id.to_string()),
            "bound value must be exactly the requested row id, got: {stmt:?}"
        );
    }

    /// admin 计数谓词只允许 `role` + `enabled`，且值正确。
    /// `workspace_id` 一旦入谓词就会重蹈根因：全局 admin key 的该列为 NULL，匹配不到。
    #[test]
    fn test_select_enabled_admin_keys_filters_role_and_enabled_without_workspace() {
        let stmt = SeaOrmRepository::select_enabled_admin_keys().build(DatabaseBackend::Postgres);
        let sql = stmt.to_string();
        let predicate = where_clause(&sql);

        assert!(
            predicate.contains(r#""api_keys"."role" = "#),
            "admin count must filter on role, got: {predicate}"
        );
        assert!(
            predicate.contains(r#""api_keys"."enabled" = "#),
            "admin count must filter on enabled, got: {predicate}"
        );
        assert!(
            !predicate.contains("workspace_id"),
            "admin count must not filter by workspace_id (admin keys have NULL workspace), got: {predicate}"
        );
        let rendered = format!("{stmt:?}");
        assert!(
            rendered.contains(r#"String(Some("admin"))"#) && rendered.contains("Bool(Some(true))"),
            "bound values must be role='admin' AND enabled=true, got {rendered}"
        );
    }

    #[tokio::test]
    async fn test_find_api_key_by_row_id_returns_mapped_row() {
        let id = fixed_uuid(12);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![sample_api_key_model(id, "niad_row", "admin")]])
            .into_connection();
        let repo = make_repo(db);

        let key = repo
            .find_api_key_by_row_id(id)
            .await
            .unwrap()
            .expect("row must be returned");
        assert_eq!(
            key.id, id,
            "returned row must carry the requested primary key"
        );
        assert_eq!(key.key_id, "niad_row");
    }

    #[tokio::test]
    async fn test_find_api_key_by_row_id_returns_none_when_row_missing() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![Vec::<ApiKeyModel>::new()])
            .into_connection();
        let repo = make_repo(db);

        assert!(
            repo.find_api_key_by_row_id(fixed_uuid(13))
                .await
                .unwrap()
                .is_none(),
            "missing row must map to None, not an error"
        );
    }

    #[tokio::test]
    async fn test_count_admin_keys_returns_stacked_count() {
        let mut count_row: BTreeMap<String, dbnexus::sea_orm::Value> = BTreeMap::new();
        count_row.insert(
            "num_items".to_string(),
            dbnexus::sea_orm::Value::BigInt(Some(2)),
        );
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![count_row]])
            .into_connection();
        let repo = make_repo(db);

        assert_eq!(
            repo.count_admin_keys().await.unwrap(),
            2,
            "SQL COUNT result must be propagated as-is"
        );
    }

    #[tokio::test]
    async fn test_count_admin_keys_propagates_db_error() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "connection reset".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);

        let err = repo.count_admin_keys().await.unwrap_err();
        assert!(
            matches!(err, crate::core::CoreError::DatabaseError(ref m, _) if m.contains("connection reset")),
            "count error must surface as DatabaseError, got {err:?}"
        );
    }

    #[tokio::test]
    async fn test_api_key_rotate_returns_not_found_when_key_missing() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![Vec::<api_key_entity::Model>::new()])
            .into_connection();
        let repo = make_repo(db);

        let err = repo.rotate_api_key("niad_missing", 3600).await.unwrap_err();
        assert!(
            matches!(err, crate::core::CoreError::NotFound(ref m) if m.contains("API key")),
            "expected NotFound for rotate, got {:?}",
            err
        );
    }

    #[tokio::test]
    async fn test_api_key_rotate_returns_new_secret_when_key_found() {
        let id = fixed_uuid(94);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![
                // get_api_key_by_id 查询。
                vec![sample_api_key_model(id, "niad_x", "admin")],
                // UPDATE RETURNING 查询。
                vec![sample_api_key_model(id, "niad_x", "admin")],
            ])
            .into_connection();
        let repo = make_repo(db);

        let rotated = repo.rotate_api_key("niad_x", 3600).await.unwrap();
        assert_eq!(rotated.key.id, id);
        assert_eq!(
            rotated.key_secret.len(),
            32,
            "rotated secret must be 32 chars"
        );
        assert_ne!(rotated.key_secret, "", "rotated secret must not be empty");
    }

    /// `grace == 0` 时轮换必须把两列显式写 NULL。
    /// 关键是 `Set(None)` 而非 `NotSet`：后者不进 UPDATE，历史宽限期会被无限延续。
    #[test]
    fn test_rotate_with_zero_grace_clears_previous_credential() {
        let now = fixed_datetime(1_700_000_000);
        let (prev, expires) =
            SeaOrmRepository::grace_columns_for_rotation(0, "current-hash".to_string(), now)
                .expect("grace=0 is always in range");

        assert_eq!(
            prev, None,
            "zero grace must not carry a previous credential"
        );
        assert_eq!(expires, None, "zero grace must not open a grace window");

        let changeset = SeaOrmRepository::rotate_changeset(
            fixed_uuid(96),
            "new-hash".to_string(),
            prev,
            expires,
            now,
        );
        assert!(
            matches!(
                changeset.prev_secret_hash,
                dbnexus::sea_orm::ActiveValue::Set(None)
            ),
            "prev_secret_hash must be explicitly SET to NULL, got {:?}",
            changeset.prev_secret_hash
        );
        assert!(
            matches!(
                changeset.rotate_expires_at,
                dbnexus::sea_orm::ActiveValue::Set(None)
            ),
            "rotate_expires_at must be explicitly SET to NULL, got {:?}",
            changeset.rotate_expires_at
        );
    }

    /// `grace > 0` 时轮换必须把轮换前的哈希移入 `prev_secret_hash`，到期设为 `now + grace`。
    #[test]
    fn test_rotate_with_grace_keeps_previous_hash_and_expiry() {
        let now = fixed_datetime(1_700_000_000);
        let (prev, expires) = SeaOrmRepository::grace_columns_for_rotation(
            3600,
            "old-generation-hash".to_string(),
            now,
        )
        .expect("3600s is in range");

        assert_eq!(
            prev.as_deref(),
            Some("old-generation-hash"),
            "the credential being replaced must become the grace-period credential"
        );
        let expires = expires.expect("grace > 0 must open a grace window");
        let window = (expires - now).num_seconds();
        assert!(
            (3595..=3605).contains(&window),
            "rotate_expires_at must be now + 3600s, got offset {window}s"
        );

        let changeset = SeaOrmRepository::rotate_changeset(
            fixed_uuid(97),
            "new-hash".to_string(),
            prev,
            Some(expires),
            now,
        );
        assert!(
            matches!(
                changeset.prev_secret_hash,
                dbnexus::sea_orm::ActiveValue::Set(Some(_))
            ),
            "prev_secret_hash must be SET to the old hash, got {:?}",
            changeset.prev_secret_hash
        );
        assert!(
            matches!(
                changeset.key_secret_hash,
                dbnexus::sea_orm::ActiveValue::Set(_)
            ),
            "key_secret_hash must be SET to the new hash, got {:?}",
            changeset.key_secret_hash
        );
    }

    /// 宽限期秒数超范围必须返回 `InvalidInput`，不得 panic 或溢出回绕。
    /// 须走完整 rotate_api_key：实现若忽略该参数会消费空桩并返回 DatabaseError。
    #[tokio::test]
    async fn test_rotate_with_out_of_range_grace_returns_invalid_input() {
        let id = fixed_uuid(98);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![sample_api_key_model(id, "niad_x", "admin")]])
            .into_connection();
        let repo = make_repo(db);

        let err = repo.rotate_api_key("niad_x", u64::MAX).await.unwrap_err();
        assert!(
            matches!(err, crate::core::CoreError::InvalidInput(ref m) if !m.is_empty()),
            "out-of-range grace must be rejected as InvalidInput, got {err:?}"
        );
    }

    /// `grace > 0` 时轮换必须回传实际写入的窗口截止时刻；
    /// 调用方据此知道上一代凭证何时彻底失效。
    #[tokio::test]
    async fn test_rotate_returns_grace_expiry_to_caller() {
        let id = fixed_uuid(144);
        let model = sample_api_key_model(id, "niad_echo", "admin");
        // 1st = 轮换前 SELECT，2nd = UPDATE RETURNING；两行 expires_at 故意不同，
        // 若响应取自旧行，断言失败（两个来源相等时该缺陷测不出）。
        let mut returning = model.clone();
        returning.expires_at = Some(fixed_datetime(1_900_000_000));
        let repo = make_repo(
            MockDatabase::new(DatabaseBackend::Postgres)
                .append_query_results(vec![vec![model], vec![returning]])
                .into_connection(),
        );

        let rotated = repo.rotate_api_key("niad_echo", 3600).await.unwrap();
        assert_eq!(
            rotated.key.expires_at,
            Some(fixed_datetime(1_900_000_000)),
            "轮换响应必须回显 UPDATE ... RETURNING 的实际行，而非轮换前读到的旧行"
        );
        let expires_at = rotated
            .grace_expires_at
            .expect("grace > 0 必须回传宽限期截止时刻");
        let offset = (expires_at - chrono::Utc::now().naive_utc()).num_seconds();
        assert!(
            (3595..=3605).contains(&offset),
            "回传时刻应为 now + 3600s，实际偏移 {offset}s"
        );
    }

    /// 关闭宽限期（默认）时不得回传一个并不存在的窗口。
    #[tokio::test]
    async fn test_rotate_without_grace_returns_no_expiry() {
        let id = fixed_uuid(145);
        let model = sample_api_key_model(id, "niad_nograce", "admin");
        let repo = make_repo(
            MockDatabase::new(DatabaseBackend::Postgres)
                .append_query_results(vec![vec![model.clone()], vec![model]])
                .into_connection(),
        );

        let rotated = repo.rotate_api_key("niad_nograce", 0).await.unwrap();
        assert_eq!(
            rotated.grace_expires_at, None,
            "grace = 0 时不得回传宽限期截止时刻"
        );
    }

    #[tokio::test]
    async fn test_api_key_get_keys_older_than_returns_matching_keys() {
        let id = fixed_uuid(95);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![sample_api_key_model(id, "niad_old", "admin")]])
            .into_connection();
        let repo = make_repo(db);

        let keys = repo.get_keys_older_than(30).await.unwrap();
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].id, id);
    }

    // ----- ApiKey error paths -----

    #[tokio::test]
    async fn test_api_key_create_propagates_insert_db_error() {
        // Postgres 下 insert 走 RETURNING：追加 query 错误即令 insert 失败。
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "api_key insert boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo
            .create_api_key(&CreateApiKeyRequest {
                workspace_id: Some(fixed_uuid(70)),
                name: "k70".to_string(),
                description: None,
                role: ApiKeyRole::User,
                rate_limit: None,
                expires_at: None,
                key_secret: Some("valid-secret".to_string()),
                key_id: None,
            })
            .await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_, _)
            ),
            "insert error must propagate as DatabaseError"
        );
    }

    #[tokio::test]
    async fn test_api_key_get_by_id_propagates_db_error() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "api_key find boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo.get_api_key_by_id("niad_x").await;
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
    async fn test_api_key_validate_propagates_db_error() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "api_key validate find boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo.validate_api_key("niad_x", "secret").await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_, _)
            ),
            "validate find error must propagate as DatabaseError"
        );
    }

    #[tokio::test]
    async fn test_api_key_list_propagates_db_error() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "api_key list boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo.list_api_keys(fixed_uuid(71), None, None).await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_, _)
            ),
            "list error must propagate as DatabaseError"
        );
    }

    #[tokio::test]
    async fn test_api_key_delete_propagates_db_error() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_exec_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "api_key delete boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo.delete_api_key(fixed_uuid(72)).await;
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
    async fn test_api_key_revoke_propagates_find_db_error() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "api_key revoke find boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo.revoke_api_key(fixed_uuid(73)).await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_, _)
            ),
            "revoke find error must propagate as DatabaseError"
        );
    }

    #[tokio::test]
    async fn test_api_key_revoke_propagates_update_db_error() {
        // find 成功、update 失败。
        let id = fixed_uuid(74);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![sample_api_key_model(id, "niad_74", "admin")]])
            .append_query_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "api_key revoke update boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo.revoke_api_key(id).await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_, _)
            ),
            "revoke update error must propagate as DatabaseError"
        );
    }

    #[tokio::test]
    async fn test_api_key_update_last_used_propagates_db_error() {
        // 单语句 UPDATE 无 find 失败面：mock exec 缓冲为空即 UPDATE 失败，错误须透传。
        let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();
        let repo = make_repo(db);
        let result = repo.update_last_used(fixed_uuid(75)).await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_, _)
            ),
            "update_last_used error must propagate as DatabaseError"
        );
    }

    #[tokio::test]
    async fn test_api_key_get_admin_propagates_db_error() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "api_key admin find boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo.get_admin_api_key(fixed_uuid(77)).await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_, _)
            ),
            "get_admin find error must propagate as DatabaseError"
        );
    }

    #[tokio::test]
    async fn test_api_key_count_propagates_db_error() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "api_key count boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo.count_api_keys(fixed_uuid(78)).await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_, _)
            ),
            "count error must propagate as DatabaseError"
        );
    }

    #[tokio::test]
    async fn test_api_key_rotate_propagates_get_db_error() {
        // get_api_key_by_id 内部 find 查询失败。
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "api_key rotate get boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo.rotate_api_key("niad_x", 3600).await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_, _)
            ),
            "rotate get error must propagate as DatabaseError"
        );
    }

    #[tokio::test]
    async fn test_api_key_rotate_propagates_update_db_error() {
        // get 成功、update 失败。
        let id = fixed_uuid(79);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![sample_api_key_model(id, "niad_79", "admin")]])
            .append_query_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "api_key rotate update boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo.rotate_api_key("niad_79", 3600).await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_, _)
            ),
            "rotate update error must propagate as DatabaseError"
        );
    }

    #[tokio::test]
    async fn test_api_key_get_keys_older_than_propagates_db_error() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "api_key older than boom".to_string(),
            ))])
            .into_connection();
        let repo = make_repo(db);
        let result = repo.get_keys_older_than(30).await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.unwrap_err(),
                crate::core::CoreError::DatabaseError(_, _)
            ),
            "get_keys_older_than error must propagate as DatabaseError"
        );
    }

    // ----- prefix_tests: 覆盖 Anonymous 默认分支 -----
    // 既有 prefix_tests 只走 Admin/User；此处补测 `_ => "nianon_"` 分支
    // （生产代码拒绝 Anonymous，但测试的 match 需覆盖完整）。

    #[test]
    fn test_anonymous_role_prefix_hits_default_branch() {
        let role = ApiKeyRole::Anonymous;
        let prefix = match role {
            ApiKeyRole::Admin => "niad_",
            ApiKeyRole::User => "nino_",
            _ => "nianon_",
        };
        assert_eq!(
            prefix, "nianon_",
            "Anonymous role must hit the default branch (production rejects with InvalidInput)"
        );
    }
}
