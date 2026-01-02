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

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "nebula_segments")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub workspace_id: String,
    pub biz_tag: String,
    pub current_id: i64,
    pub max_id: i64,
    pub step: i32,
    pub delta: i32,
    pub dc_id: i32,
    pub created_at: DateTime,
    pub updated_at: DateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Segment {
    pub id: i64,
    pub workspace_id: String,
    pub biz_tag: String,
    pub start_id: i64,
    pub max_id: i64,
    pub step: i32,
    pub delta: i32,
    pub dc_id: i32,
    pub created_at: DateTime,
    pub updated_at: DateTime,
}

impl From<Entity> for Model {
    fn from(_entity: Entity) -> Self {
        unreachable!("Entity cannot be directly converted to Model")
    }
}

impl From<Model> for Segment {
    fn from(model: Model) -> Self {
        Segment {
            id: model.id,
            workspace_id: model.workspace_id,
            biz_tag: model.biz_tag,
            start_id: model.current_id,
            max_id: model.max_id,
            step: model.step,
            delta: model.delta,
            dc_id: model.dc_id,
            created_at: model.created_at,
            updated_at: model.updated_at,
        }
    }
}
