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

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq, Serialize, Deserialize)]
#[sea_orm(table_name = "groups")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    #[sea_orm(column_name = "workspace_id")]
    pub workspace_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub max_biz_tags: i32,
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
    #[sea_orm(has_many = "super::biz_tag_entity::Entity")]
    BizTag,
}

impl ActiveModelBehavior for ActiveModel {}

impl Related<super::workspace_entity::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Workspace.def()
    }
}

impl Related<super::biz_tag_entity::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::BizTag.def()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Group {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub max_biz_tags: i32,
    pub created_at: DateTime,
    pub updated_at: DateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CreateGroupRequest {
    pub workspace_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub max_biz_tags: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UpdateGroupRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub max_biz_tags: Option<i32>,
}

impl From<Model> for Group {
    fn from(model: Model) -> Self {
        Group {
            id: model.id,
            workspace_id: model.workspace_id,
            name: model.name,
            description: model.description,
            max_biz_tags: model.max_biz_tags,
            created_at: model.created_at,
            updated_at: model.updated_at,
        }
    }
}
