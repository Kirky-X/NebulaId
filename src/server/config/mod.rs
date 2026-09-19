// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Server configuration module.

pub mod cors;
pub mod hot_reload;
pub mod management;
pub mod tls;

// Re-exports for backward compatibility
pub use self::hot_reload::HotReloadConfig;
pub use self::management::{ConfigManagementService, ConfigManager};
pub use self::tls::TlsManager;
