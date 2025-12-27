use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq, Serialize, Deserialize)]
#[sea_orm(table_name = "workspaces")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    #[sea_orm(enum_name = "WorkspaceStatusDb")]
    pub status: WorkspaceStatusDb,
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

#[derive(Debug, Clone, Copy, EnumIter, DeriveActiveEnum, PartialEq, Eq, Serialize, Deserialize)]
#[sea_orm(rs_type = "String", db_type = "String(StringLen::N(20))")]
pub enum WorkspaceStatusDb {
    #[sea_orm(string_value = "active")]
    Active,
    #[sea_orm(string_value = "inactive")]
    Inactive,
    #[sea_orm(string_value = "suspended")]
    Suspended,
}

impl From<WorkspaceStatusDb> for WorkspaceStatus {
    fn from(status: WorkspaceStatusDb) -> Self {
        match status {
            WorkspaceStatusDb::Active => WorkspaceStatus::Active,
            WorkspaceStatusDb::Inactive => WorkspaceStatus::Inactive,
            WorkspaceStatusDb::Suspended => WorkspaceStatus::Suspended,
        }
    }
}

impl From<WorkspaceStatus> for WorkspaceStatusDb {
    fn from(status: WorkspaceStatus) -> Self {
        match status {
            WorkspaceStatus::Active => WorkspaceStatusDb::Active,
            WorkspaceStatus::Inactive => WorkspaceStatusDb::Inactive,
            WorkspaceStatus::Suspended => WorkspaceStatusDb::Suspended,
        }
    }
}
