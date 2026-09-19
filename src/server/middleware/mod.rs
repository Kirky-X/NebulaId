// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Server middleware module.
//!
//! This module re-exports middleware components. Concrete implementations live
//! in dedicated submodules.

pub mod api_key_auth;
pub mod locale;
pub mod request_id;
pub mod size_limit;
pub(crate) mod utils;

// Re-export ApiKeyRole for use in router.rs (unified with core::database::ApiKeyRole)
pub use crate::core::database::ApiKeyRole;

// Re-export API key auth components (backward compatibility)
pub use api_key_auth::{admin_required_middleware, auth_middleware_fn, ApiKeyAuth};

// Re-export locale middleware components (Phase 8 )
pub use locale::{locale_middleware, Locale};

// T022 — request_id 提取/生成/贯穿中间件
pub use request_id::{current_request_id, request_id_middleware, RequestId};
