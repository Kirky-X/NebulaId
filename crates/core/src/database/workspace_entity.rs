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
#[sea_orm(table_name = "workspaces", schema_name = "nebula_id")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    #[sea_orm(rs_type = "String", db_type = "String(StringLen::N(20))")]
    pub status: String,
    pub max_groups: i32,
    pub max_biz_tags: i32,
    pub created_at: DateTime,
    pub updated_at: DateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::group_entity::Entity")]
    Group,
}

impl ActiveModelBehavior for ActiveModel {}

impl Related<super::group_entity::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Group.def()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workspace {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub status: WorkspaceStatus,
    pub max_groups: i32,
    pub max_biz_tags: i32,
    pub created_at: DateTime,
    pub updated_at: DateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum WorkspaceStatus {
    Active,
    Inactive,
    Suspended,
}

impl WorkspaceStatus {
    pub fn as_str(&self) -> &str {
        match self {
            WorkspaceStatus::Active => "active",
            WorkspaceStatus::Inactive => "inactive",
            WorkspaceStatus::Suspended => "suspended",
        }
    }
}

impl fmt::Display for WorkspaceStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl From<String> for WorkspaceStatus {
    fn from(s: String) -> Self {
        s.as_str().into()
    }
}

impl From<&str> for WorkspaceStatus {
    fn from(s: &str) -> Self {
        match s {
            "active" => WorkspaceStatus::Active,
            "inactive" => WorkspaceStatus::Inactive,
            "suspended" => WorkspaceStatus::Suspended,
            _ => WorkspaceStatus::Inactive,
        }
    }
}

impl From<WorkspaceStatus> for String {
    fn from(status: WorkspaceStatus) -> Self {
        status.to_string()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CreateWorkspaceRequest {
    pub name: String,
    pub description: Option<String>,
    pub max_groups: Option<i32>,
    pub max_biz_tags: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UpdateWorkspaceRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub status: Option<WorkspaceStatus>,
    pub max_groups: Option<i32>,
    pub max_biz_tags: Option<i32>,
}

impl From<Model> for Workspace {
    fn from(model: Model) -> Self {
        Workspace {
            id: model.id,
            name: model.name,
            description: model.description,
            status: model.status.into(),
            max_groups: model.max_groups,
            max_biz_tags: model.max_biz_tags,
            created_at: model.created_at,
            updated_at: model.updated_at,
        }
    }
}
