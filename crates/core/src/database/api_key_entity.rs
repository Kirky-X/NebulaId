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

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq, Serialize, Deserialize)]
#[sea_orm(table_name = "api_keys")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: Uuid,
    #[sea_orm(unique)]
    pub key_id: String,
    pub key_secret_hash: String,
    pub key_prefix: String,
    #[sea_orm(rename = "role")]
    pub role: String, // Store as "admin" or "user"
    #[sea_orm(unique)]
    pub workspace_id: Uuid,
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
    pub workspace_id: Uuid,
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
}

impl fmt::Display for ApiKeyRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ApiKeyRole::Admin => write!(f, "admin"),
            ApiKeyRole::User => write!(f, "user"),
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
    pub workspace_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub role: ApiKeyRole,
    pub rate_limit: Option<i32>,
    pub expires_at: Option<DateTime>,
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
    pub key_secret: String, // Only returned on creation
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