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
    /// 上一代凭证的哈希（与 `key_secret_hash` 同 Argon2 PHC 格式、同 salt）。
    /// 仅在"开启宽限期的轮换"后写入；`grace = 0`（默认）或从未轮换时为 `None`。
    pub prev_secret_hash: Option<String>,
    /// 宽限期的绝对到期时刻（UTC）。`None` 表示当前没有生效中的宽限期；
    /// 到期后 `prev_secret_hash` 不再被采信（惰性时间判定，见 `validate_api_key`）。
    pub rotate_expires_at: Option<DateTime>,
    pub key_prefix: String,
    #[sea_orm(column_name = "role")]
    pub role: String,
    pub workspace_id: Option<Uuid>, // UUID for proper foreign key
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

/// 手写 `Debug`：`key_secret_hash` 与 `prev_secret_hash` 是凭证哈希材料，
/// 不得出现在 `{:?}` 输出里（CWE-532）。`derive(Debug)` 下任何日志、panic
/// 消息或断言失败输出都会把它原样落盘，且编译期没有任何告警。
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

/// `validate_api_key` 的判定结果。
///
/// 取代原来的 `(workspace_id, role)` 元组：宽限期生效后，"认证通过"不再是一个
/// 二元结论 —— 命中的是当代凭证还是上一代凭证，决定调用方能否缓存该决策。
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
    /// LOW-1 修复（CWE-1188）：禁用认证时使用的匿名角色。
    /// 该角色仅存在于内存中（请求 extensions），不会被持久化到数据库
    /// （`repository.rs` 的 `create_api_key` 会拒绝 Anonymous）。
    /// 权限低于 User：只能访问公开端点（health/ready/metrics），
    /// 其他端点由 `router.rs::verify_user_role` 拒绝。
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
            // ARCH-LOW-002 修复：`"anonymous"` 不应从数据库反序列化。
            // Anonymous 是仅运行时存在的角色（禁用认证时注入 extensions），
            // 不应被持久化。若数据库出现 'anonymous'（运维误操作/迁移脚本
            // 错误/SQL 注入），归一化为 User 默认值并 log warn，让运维
            // 在日志中看到问题。原实现接受 'anonymous' 反序列化会让 Anonymous
            // 通过 ApiKey 传播到 middleware/router，破坏 LOW-1 契约。
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
    pub workspace_id: Option<Uuid>, // Optional: NULL for global admin keys
    pub name: String,
    pub description: Option<String>,
    pub role: ApiKeyRole,
    pub rate_limit: Option<i32>,
    pub expires_at: Option<DateTime>,
    pub key_secret: Option<String>, // Optional: use provided secret instead of generating
    pub key_id: Option<String>,     // Optional: use provided key_id instead of generating
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
    ///
    /// 调用方据此知道上一代凭证何时彻底失效（T012）。
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

/// `ApiKey`（= `ApiKeyInfo`）比 `ApiKeyResponse` 多 `workspace_id` 与 `last_used_at`
/// 两列，列表接口下发的是后者形状；有了这条转换，wire 层就不必为列表另写一份字段表。
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
