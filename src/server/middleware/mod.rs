// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Server middleware module: re-exports components defined in submodules.

pub mod api_key_auth;
pub mod locale;
pub mod request_id;
pub mod size_limit;
pub(crate) mod utils;

// ApiKeyRole 统一使用 core::database::ApiKeyRole（router.rs 使用）
pub use crate::core::database::ApiKeyRole;

// API key auth 组件 re-export（向后兼容）
pub use api_key_auth::{admin_required_middleware, auth_middleware_fn, ApiKeyAuth};

// locale 中间件 re-export（向后兼容）
pub use locale::{locale_middleware, Locale};

// request_id 提取/生成/贯穿中间件（re-export）
pub use request_id::{current_request_id, request_id_middleware, RequestId};
