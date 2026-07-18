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
pub mod auth;
pub mod batch;
pub mod environment;
pub mod error;
pub mod logging;
pub mod monitoring;
pub mod rate_limit;
pub mod redis;
pub mod tls;

// Re-export public types for backward compatibility (downstream uses
// `crate::core::config::AppConfig` etc., which must continue to resolve).
pub use algorithm::{
    AlgorithmConfig, SegmentAlgorithmConfig, SnowflakeAlgorithmConfig, UuidV7Config,
};
pub use app::{AppConfig, DatabaseConfig, DatabaseEngine, EtcdConfig};
pub use app_config::Config;
pub use auth::{ApiKeyEntry, AuthConfig};
pub use batch::BatchGenerateConfig;
pub use environment::{is_production, Environment};
pub use error::{ConfigError, ConfigResult};
pub use logging::{LogFormat, LogLevel, LoggingConfig};
pub use monitoring::MonitoringConfig;
pub use rate_limit::RateLimitConfig;
pub use redis::RedisConfig;
pub use tls::{TlsConfig, TlsVersion};
