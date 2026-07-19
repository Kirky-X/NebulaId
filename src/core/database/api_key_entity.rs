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

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};
use std::fmt;

use super::connection::NEBULA_SCHEMA;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq, Serialize, Deserialize)]
#[sea_orm(table_name = "api_keys", schema_name = "nebula_id")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: Uuid,
    #[sea_orm(unique)]
    pub key_id: String,
    pub key_secret_hash: String,
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
