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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GroupResponse {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub max_biz_tags: i32,
    pub created_at: DateTime,
    pub updated_at: DateTime,
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

impl From<Group> for GroupResponse {
    fn from(group: Group) -> Self {
        GroupResponse {
            id: group.id,
            workspace_id: group.workspace_id,
            name: group.name,
            description: group.description,
            max_biz_tags: group.max_biz_tags,
            created_at: group.created_at,
            updated_at: group.updated_at,
        }
    }
}
