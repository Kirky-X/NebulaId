// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! 服务端配置模块：热加载、配置管理服务、TLS 与 CORS。

pub mod cors;
pub mod hot_reload;
pub mod management;
pub mod tls;

// 兼容既有导入路径的再导出
pub use self::hot_reload::HotReloadConfig;
pub use self::management::{ConfigManagementService, ConfigManager};
pub use self::tls::TlsManager;
