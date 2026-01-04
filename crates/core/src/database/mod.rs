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

#![allow(unused_imports)]

mod api_key_entity;
mod biz_tag_entity;
mod connection;
mod group_entity;
mod repository;
mod segment_entity;
mod workspace_entity;

pub use crate::types::id::{AlgorithmType, IdFormat};
pub use api_key_entity::{
    ApiKey, ApiKeyInfo, ApiKeyResponse, ApiKeyRole, ApiKeyWithSecret, CreateApiKeyRequest,
};
pub use biz_tag_entity::{BizTag, CreateBizTagRequest, UpdateBizTagRequest};
pub use connection::create_connection;
pub use connection::run_migrations;
pub use group_entity::{CreateGroupRequest, Group, UpdateGroupRequest};
pub use repository::{
    ApiKeyRepository, BizTagRepository, GroupRepository, SeaOrmRepository, SegmentRepository,
    WorkspaceRepository,
};
pub use workspace_entity::{
    CreateWorkspaceRequest, UpdateWorkspaceRequest, Workspace, WorkspaceStatus,
};
