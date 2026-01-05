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

use crate::types::id::{AlgorithmType, IdFormat};
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::from_str;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq, Serialize, Deserialize)]
#[sea_orm(table_name = "biz_tags", schema_name = "nebula_id")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    #[sea_orm(column_name = "workspace_id")]
    pub workspace_id: Uuid,
    #[sea_orm(column_name = "group_id")]
    pub group_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub algorithm: AlgorithmTypeDb,
    pub format: IdFormatDb,
    pub prefix: String,
    pub base_step: i32,
    pub max_step: i32,
    pub datacenter_ids: String,
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
    #[sea_orm(
        belongs_to = "super::group_entity::Entity",
        from = "Column::GroupId",
        to = "super::group_entity::Column::Id"
    )]
    Group,
}

impl ActiveModelBehavior for ActiveModel {}

impl Related<super::workspace_entity::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Workspace.def()
    }
}

impl Related<super::group_entity::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Group.def()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BizTag {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub group_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub algorithm: AlgorithmType,
    pub format: IdFormat,
    pub prefix: String,
    pub base_step: i32,
    pub max_step: i32,
    pub datacenter_ids: Vec<i32>,
    pub created_at: DateTime,
    pub updated_at: DateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CreateBizTagRequest {
    pub workspace_id: Uuid,
    pub group_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub algorithm: Option<AlgorithmType>,
    pub format: Option<IdFormat>,
    pub prefix: Option<String>,
    pub base_step: Option<i32>,
    pub max_step: Option<i32>,
    pub datacenter_ids: Option<Vec<i32>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UpdateBizTagRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub algorithm: Option<AlgorithmType>,
    pub format: Option<IdFormat>,
    pub prefix: Option<String>,
    pub base_step: Option<i32>,
    pub max_step: Option<i32>,
    pub datacenter_ids: Option<Vec<i32>>,
}

impl From<Model> for BizTag {
    fn from(model: Model) -> Self {
        let datacenter_ids = if model.datacenter_ids.is_empty() {
            vec![]
        } else {
            from_str(&model.datacenter_ids).unwrap_or_else(|_| vec![])
        };

        BizTag {
            id: model.id,
            workspace_id: model.workspace_id,
            group_id: model.group_id,
            name: model.name,
            description: model.description,
            algorithm: model.algorithm.into(),
            format: model.format.into(),
            prefix: model.prefix,
            base_step: model.base_step,
            max_step: model.max_step,
            datacenter_ids,
            created_at: model.created_at,
            updated_at: model.updated_at,
        }
    }
}

#[derive(Debug, Clone, Copy, EnumIter, DeriveActiveEnum, PartialEq, Eq, Serialize, Deserialize)]
#[sea_orm(rs_type = "String", db_type = "Enum", enum_name = "algorithm_type")]
pub enum AlgorithmTypeDb {
    #[sea_orm(string_value = "segment")]
    Segment,
    #[sea_orm(string_value = "snowflake")]
    Snowflake,
    #[sea_orm(string_value = "uuid_v7")]
    UuidV7,
    #[sea_orm(string_value = "uuid_v4")]
    UuidV4,
}

#[derive(Debug, Clone, Copy, EnumIter, DeriveActiveEnum, PartialEq, Eq, Serialize, Deserialize)]
#[sea_orm(rs_type = "String", db_type = "Enum", enum_name = "id_format")]
pub enum IdFormatDb {
    #[sea_orm(string_value = "numeric")]
    Numeric,
    #[sea_orm(string_value = "prefixed")]
    Prefixed,
    #[sea_orm(string_value = "uuid")]
    Uuid,
}

impl From<AlgorithmTypeDb> for AlgorithmType {
    fn from(alg_type: AlgorithmTypeDb) -> Self {
        match alg_type {
            AlgorithmTypeDb::Segment => AlgorithmType::Segment,
            AlgorithmTypeDb::Snowflake => AlgorithmType::Snowflake,
            AlgorithmTypeDb::UuidV7 => AlgorithmType::UuidV7,
            AlgorithmTypeDb::UuidV4 => AlgorithmType::UuidV4,
        }
    }
}

impl From<AlgorithmType> for AlgorithmTypeDb {
    fn from(alg_type: AlgorithmType) -> Self {
        match alg_type {
            AlgorithmType::Segment => AlgorithmTypeDb::Segment,
            AlgorithmType::Snowflake => AlgorithmTypeDb::Snowflake,
            AlgorithmType::UuidV7 => AlgorithmTypeDb::UuidV7,
            AlgorithmType::UuidV4 => AlgorithmTypeDb::UuidV4,
        }
    }
}

impl From<IdFormatDb> for IdFormat {
    fn from(format: IdFormatDb) -> Self {
        match format {
            IdFormatDb::Numeric => IdFormat::Numeric,
            IdFormatDb::Prefixed => IdFormat::Prefixed,
            IdFormatDb::Uuid => IdFormat::Uuid,
        }
    }
}

impl From<IdFormat> for IdFormatDb {
    fn from(format: IdFormat) -> Self {
        match format {
            IdFormat::Numeric => IdFormatDb::Numeric,
            IdFormat::Prefixed => IdFormatDb::Prefixed,
            IdFormat::Uuid => IdFormatDb::Uuid,
        }
    }
}
