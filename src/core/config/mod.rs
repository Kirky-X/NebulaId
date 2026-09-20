// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Configuration module for Nebula ID.
//!
//! This module aggregates configuration types split by business domain.
//! Each sub-module owns a specific domain (app/auth/algorithm/logging etc.);
//! mod.rs only re-exports public types and declares sub-modules
//! (rule 25: mod.rs 只放 trait + pub re-export).

// Implementation sub-modules (config management services)
pub(crate) mod dynamic;
pub(crate) mod management;
pub(crate) mod workspace;

// Domain sub-modules (configuration types)
pub mod algorithm;
pub mod app;
pub mod app_config;
pub mod audit;
pub mod auth;
pub mod batch;
pub mod defaults;
pub mod environment;
pub mod error;
pub mod logging;
pub mod monitoring;
pub mod rate_limit;
pub mod redis;
pub mod tls;

// Re-export public types for backward compatibility (downstream uses
// `crate::core::config::NebulaIdConfig` etc., which must continue to resolve).
pub use algorithm::{
    AlgorithmConfig, SegmentAlgorithmConfig, SnowflakeAlgorithmConfig, UuidV8Config,
};
pub use app::{DatabaseConfig, DatabaseEngine, EtcdConfig, NebulaIdConfig};
pub use app_config::resolve_startup_config;
pub use app_config::Config;
pub use app_config::HotReloadSettings;
pub use app_config::StartupConfig;
pub use audit::AuditConfig;
pub use auth::{ApiKeyEntry, AuthConfig};
pub use batch::BatchGenerateConfig;
pub use environment::{is_production, Environment};
pub use error::{ConfigError, ConfigResult};
pub use logging::{LogFormat, LogLevel, LoggingConfig};
pub use monitoring::MonitoringConfig;
pub use rate_limit::RateLimitConfig;
pub use redis::RedisConfig;
pub use tls::{TlsConfig, TlsVersion};
