// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

use dbnexus::sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};
use std::fmt;

use super::connection::NEBULA_SCHEMA;

#[derive(Clone, PartialEq, DeriveEntityModel, Eq, Serialize, Deserialize)]
#[sea_orm(table_name = "api_keys", schema_name = "nebula_id")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: Uuid,
    #[sea_orm(unique)]
    pub key_id: String,
    pub key_secret_hash: String,
    /// 上一代凭证哈希（与 `key_secret_hash` 同 Argon2 PHC 格式、同 salt）。
    /// 仅宽限期轮换后写入；`grace = 0`（默认）或从未轮换时为 `None`。
    pub prev_secret_hash: Option<String>,
    /// 宽限期绝对到期时刻（UTC）；`None` = 无生效宽限期。
    /// 到期后 `prev_secret_hash` 不再被采信（惰性判定，见 `validate_api_key`）。
    pub rotate_expires_at: Option<DateTime>,
    pub key_prefix: String,
    #[sea_orm(column_name = "role")]
    pub role: String,
    pub workspace_id: Option<Uuid>,
    pub name: String,
    pub description: Option<String>,
    pub rate_limit: i32,
    pub enabled: bool,
    pub expires_at: Option<DateTime>,
    pub last_used_at: Option<DateTime>,
    pub created_at: DateTime,
    pub updated_at: DateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::workspace_entity::Entity",
        from = "Column::WorkspaceId",
        to = "super::workspace_entity::Column::Id"
    )]
    Workspace,
}

impl ActiveModelBehavior for ActiveModel {}

/// 手写 `Debug`：凭证哈希不得出现在 `{:?}` 输出（CWE-532），
/// 否则日志/panic 消息会原样落盘；两个哈希字段固定输出 `[REDACTED]`。
impl std::fmt::Debug for Model {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Model")
            .field("id", &self.id)
            .field("key_id", &self.key_id)
            .field("key_secret_hash", &"[REDACTED]")
            .field("prev_secret_hash", &"[REDACTED]")
            .field("rotate_expires_at", &self.rotate_expires_at)
            .field("key_prefix", &self.key_prefix)
            .field("role", &self.role)
            .field("workspace_id", &self.workspace_id)
            .field("name", &self.name)
            .field("description", &self.description)
            .field("rate_limit", &self.rate_limit)
            .field("enabled", &self.enabled)
            .field("expires_at", &self.expires_at)
            .field("last_used_at", &self.last_used_at)
            .field("created_at", &self.created_at)
            .field("updated_at", &self.updated_at)
            .finish()
    }
}

impl Related<super::workspace_entity::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Workspace.def()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKey {
    pub id: Uuid,
    pub key_id: String,
    pub key_prefix: String,
    pub role: ApiKeyRole,
    pub workspace_id: Option<Uuid>,
    pub name: String,
    pub description: Option<String>,
    pub rate_limit: i32,
    pub enabled: bool,
    pub expires_at: Option<DateTime>,
    pub last_used_at: Option<DateTime>,
    pub created_at: DateTime,
}

pub type ApiKeyInfo = ApiKey;

/// `validate_api_key` 的判定结果：认证通过不再只是二元结论。
/// `used_previous_credential` 区分当代/上一代凭证，决定调用方能否缓存该决策。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AuthenticatedKey {
    pub workspace_id: Option<Uuid>,
    pub role: ApiKeyRole,
    /// `true` 表示只有 `prev_secret_hash`（宽限期内的旧凭证）校验通过。
    /// 该结果有时效性（受 `rotate_expires_at` 约束），不得进入认证决策缓存。
    pub used_previous_credential: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ApiKeyRole {
    Admin,
    User,
    /// （CWE-1188）禁用认证时使用的匿名角色，仅存在于内存（请求 extensions）。
    /// 权限低于 User：只能访问公开端点（health/ready/metrics），其余由
    /// `router.rs::verify_user_role` 拒绝；不会被持久化（`create_api_key` 拒绝）。
    Anonymous,
}

impl fmt::Display for ApiKeyRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ApiKeyRole::Admin => write!(f, "admin"),
            ApiKeyRole::User => write!(f, "user"),
            ApiKeyRole::Anonymous => write!(f, "anonymous"),
        }
    }
}

impl From<String> for ApiKeyRole {
    fn from(s: String) -> Self {
        s.as_str().into()
    }
}

impl From<&str> for ApiKeyRole {
    fn from(s: &str) -> Self {
        match s {
            "admin" => ApiKeyRole::Admin,
            "user" => ApiKeyRole::User,
            // "anonymous" 不应从数据库反序列化：Anonymous 仅运行时存在，
            // 数据库出现该值（误操作/迁移错误/注入）时归一化为 User 并 warn。
            "anonymous" => {
                tracing::warn!(
                    role_value = s,
                    "database contains 'anonymous' role which should not be persisted, \
                     normalizing to User"
                );
                ApiKeyRole::User
            }
            _ => ApiKeyRole::User,
        }
    }
}

impl From<ApiKeyRole> for String {
    fn from(role: ApiKeyRole) -> Self {
        role.to_string()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CreateApiKeyRequest {
    pub workspace_id: Option<Uuid>, // 全局 admin key 时为 NULL
    pub name: String,
    pub description: Option<String>,
    pub role: ApiKeyRole,
    pub rate_limit: Option<i32>,
    pub expires_at: Option<DateTime>,
    pub key_secret: Option<String>, // Some 时使用给定 secret，不再生成
    pub key_id: Option<String>,     // Some 时使用给定 key_id，不再生成
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ApiKeyResponse {
    pub id: Uuid,
    pub key_id: String,
    pub key_prefix: String,
    pub name: String,
    pub description: Option<String>,
    pub role: ApiKeyRole,
    pub rate_limit: i32,
    pub enabled: bool,
    pub expires_at: Option<DateTime>,
    pub created_at: DateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ApiKeyWithSecret {
    pub key: ApiKeyResponse,
    pub key_secret: String,
    /// 本次轮换后的宽限期截止时刻（UTC）；`None` = 未开启宽限期（默认）或新建 key。
    /// 调用方据此知道上一代凭证何时彻底失效。
    pub grace_expires_at: Option<DateTime>,
}

impl From<Model> for ApiKey {
    fn from(model: Model) -> Self {
        ApiKey {
            id: model.id,
            key_id: model.key_id,
            key_prefix: model.key_prefix,
            role: model.role.into(),
            workspace_id: model.workspace_id,
            name: model.name,
            description: model.description,
            rate_limit: model.rate_limit,
            enabled: model.enabled,
            expires_at: model.expires_at,
            last_used_at: model.last_used_at,
            created_at: model.created_at,
        }
    }
}

/// `ApiKey` 比 `ApiKeyResponse` 多 `workspace_id`/`last_used_at` 两列，故需此转换。
impl From<ApiKey> for ApiKeyResponse {
    fn from(key: ApiKey) -> Self {
        ApiKeyResponse {
            id: key.id,
            key_id: key.key_id,
            key_prefix: key.key_prefix,
            name: key.name,
            description: key.description,
            role: key.role,
            rate_limit: key.rate_limit,
            enabled: key.enabled,
            expires_at: key.expires_at,
            created_at: key.created_at,
        }
    }
}

impl From<Model> for ApiKeyResponse {
    fn from(model: Model) -> Self {
        ApiKeyResponse {
            id: model.id,
            key_id: model.key_id,
            key_prefix: model.key_prefix,
            name: model.name,
            description: model.description,
            role: model.role.into(),
            rate_limit: model.rate_limit,
            enabled: model.enabled,
            expires_at: model.expires_at,
            created_at: model.created_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_model() -> Model {
        let now = chrono::Utc::now().naive_utc();
        Model {
            id: uuid::Uuid::new_v4(),
            key_id: "test-key-id".to_string(),
            key_secret_hash: "super-secret-hash".to_string(),
            prev_secret_hash: Some("old-hash".to_string()),
            rotate_expires_at: None,
            key_prefix: "test".to_string(),
            role: "admin".to_string(),
            workspace_id: None,
            name: "test".to_string(),
            description: None,
            rate_limit: 100,
            enabled: true,
            expires_at: None,
            last_used_at: None,
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn test_model_debug_redacts_credential_hashes() {
        let debug = format!("{:?}", sample_model());
        assert!(!debug.contains("super-secret-hash"));
        assert!(!debug.contains("old-hash"));
        assert!(debug.contains("[REDACTED]"));
        assert!(debug.contains("test-key-id"));
    }

    #[test]
    fn test_api_key_role_display_and_from() {
        assert_eq!(ApiKeyRole::Admin.to_string(), "admin");
        assert_eq!(ApiKeyRole::User.to_string(), "user");
        assert_eq!(ApiKeyRole::Anonymous.to_string(), "anonymous");
        assert_eq!(ApiKeyRole::from("admin".to_string()), ApiKeyRole::Admin);
        assert_eq!(ApiKeyRole::from("user"), ApiKeyRole::User);
        assert_eq!(ApiKeyRole::from("anonymous"), ApiKeyRole::User);
        assert_eq!(ApiKeyRole::from("bogus"), ApiKeyRole::User);
    }
}
