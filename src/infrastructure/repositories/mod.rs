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

//! Repository implementations using dbnexus.
//!
//! This module provides repository implementations that use the dbnexus
//! connection pool for database operations.

mod segment_repository;
mod api_key_repository;
mod workspace_repository;

pub use segment_repository::DbNexusSegmentRepository;
pub use api_key_repository::DbNexusApiKeyRepository;
pub use workspace_repository::DbNexusWorkspaceRepository;
