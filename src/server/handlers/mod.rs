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

//! API handlers for Nebula ID.
//!
//! `ApiHandlers` struct + constructors live here; per-domain method impls
//! are split into sub-modules (`id_handlers`, `system_handlers`,
//! `biz_tag_handlers`, `workspace_handlers`, `api_key_handlers`)
//! (rule 25: mod.rs 只放 trait + pub struct + re-export).

use crate::core::database::ApiKeyRepository;
use crate::server::config::management::ConfigManagementService;
use std::sync::Arc;

pub mod api_key_handlers;
pub mod biz_tag_handlers;
pub mod helpers;
pub mod id_handlers;
pub mod mock_generator;
pub mod system_handlers;
pub mod workspace_handlers;

pub use api_key_handlers::KeyRotationHandle;

/// Top-level API handler aggregating ID generator, metrics, config service
/// and optional API key repository.
pub struct ApiHandlers {
    pub(super) id_generator: Arc<dyn crate::core::algorithm::IdGenerator>,
    pub(super) metrics: ApiMetrics,
    pub(super) start_time: std::time::Instant,
    pub(super) config_service: Arc<ConfigManagementService>,
    pub(super) api_key_repo: Option<Arc<dyn ApiKeyRepository>>,
}

#[derive(Default)]
pub struct ApiMetrics {
    pub total_requests: std::sync::atomic::AtomicU64,
    pub successful_generations: std::sync::atomic::AtomicU64,
    pub failed_generations: std::sync::atomic::AtomicU64,
    pub total_ids_generated: std::sync::atomic::AtomicU64,
    pub avg_latency_ms: std::sync::atomic::AtomicU64,
}

impl ApiHandlers {
    pub fn new(
        id_generator: Arc<dyn crate::core::algorithm::IdGenerator>,
        config_service: Arc<ConfigManagementService>,
    ) -> Self {
        Self {
            id_generator,
            metrics: ApiMetrics::default(),
            start_time: std::time::Instant::now(),
            config_service,
            api_key_repo: None,
        }
    }

    pub fn with_api_key_repository(
        id_generator: Arc<dyn crate::core::algorithm::IdGenerator>,
        config_service: Arc<ConfigManagementService>,
        api_key_repo: Arc<dyn ApiKeyRepository>,
    ) -> Self {
        Self {
            id_generator,
            metrics: ApiMetrics::default(),
            start_time: std::time::Instant::now(),
            config_service,
            api_key_repo: Some(api_key_repo),
        }
    }

    pub fn get_config_service(&self) -> Arc<ConfigManagementService> {
        self.config_service.clone()
    }
}
