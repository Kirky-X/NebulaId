// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

#![allow(unused_imports)]

mod api_key_entity;
mod api_key_repository;
mod biz_tag_entity;
mod biz_tag_repository;
mod connection;
mod group_entity;
mod repository;
mod segment_entity;
mod segment_repository;
#[cfg(test)]
pub(crate) mod testing;
pub(crate) mod workspace_entity;
mod workspace_repository;

pub use crate::core::types::id::{AlgorithmType, IdFormat};
pub use api_key_entity::{
    ApiKey, ApiKeyInfo, ApiKeyResponse, ApiKeyRole, ApiKeyWithSecret, AuthenticatedKey,
    CreateApiKeyRequest,
};
pub use biz_tag_entity::{BizTag, CreateBizTagRequest, UpdateBizTagRequest};
pub use connection::create_connection;
pub(crate) use connection::redact_db_url;
pub use connection::run_migrations;
pub use group_entity::{CreateGroupRequest, Group, UpdateGroupRequest};
pub use repository::{
    ApiKeyRepository, BizTagRepository, GroupRepository, SeaOrmRepository, SegmentRepository,
    WorkspaceRepository,
};
pub use workspace_entity::{
    CreateWorkspaceRequest, UpdateWorkspaceRequest, Workspace, WorkspaceStatus,
};
