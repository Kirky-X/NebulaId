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

//! Server middleware module.
//!
//! This module re-exports middleware components. Concrete implementations live
//! in dedicated submodules.

pub mod api_key_auth;
pub mod locale;
pub mod size_limit;
pub(crate) mod utils;

// Re-export ApiKeyRole for use in router.rs (unified with core::database::ApiKeyRole)
pub use crate::core::database::ApiKeyRole;

// Re-export API key auth components (backward compatibility)
pub use api_key_auth::{admin_required_middleware, auth_middleware_fn, ApiKeyAuth};

// Re-export locale middleware components (Phase 8 T040)
pub use locale::{locale_middleware, Locale};
