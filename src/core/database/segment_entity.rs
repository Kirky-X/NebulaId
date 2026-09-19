// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

use dbnexus::sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "nebula_segments", schema_name = "nebula_id")]
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

impl From<Entity> for Model {
    fn from(_entity: Entity) -> Self {
        unreachable!("Entity cannot be directly converted to Model")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[should_panic(expected = "Entity cannot be directly converted")]
    fn test_entity_to_model_is_unreachable() {
        let _ = Model::from(Entity);
    }
}
