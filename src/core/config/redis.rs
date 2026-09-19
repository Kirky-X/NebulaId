// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Redis cache configuration.

use serde::{Deserialize, Serialize};

/// Redis configuration for caching
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RedisConfig {
    /// Redis connection URL
    pub url: String,
    /// Connection pool size
    #[serde(default = "default_redis_pool_size")]
    pub pool_size: u32,
    /// Key prefix for cache entries
    #[serde(default = "default_redis_key_prefix")]
    pub key_prefix: String,
    /// Default TTL in seconds
    #[serde(default = "default_redis_ttl_seconds")]
    pub ttl_seconds: u64,
}

/// 手写 `Debug`：`url` 可以写成 `redis://:password@host`，口令不得进 `{:?}` 输出
/// （CWE-532）。脱敏复用 [`crate::core::config::app::redact_optional_url`]，与
/// `DatabaseConfig` 同一份实现。
impl std::fmt::Debug for RedisConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RedisConfig")
            .field(
                "url",
                &crate::core::config::app::redact_optional_url(&self.url),
            )
            .field("pool_size", &self.pool_size)
            .field("key_prefix", &self.key_prefix)
            .field("ttl_seconds", &self.ttl_seconds)
            .finish()
    }
}

fn default_redis_pool_size() -> u32 {
    16
}

fn default_redis_key_prefix() -> String {
    "nebula:id:".to_string()
}

fn default_redis_ttl_seconds() -> u64 {
    600
}

impl Default for RedisConfig {
    fn default() -> Self {
        Self {
            url: std::env::var("REDIS_URL")
                .unwrap_or_else(|_| "redis://localhost:6379".to_string()),
            pool_size: default_redis_pool_size(),
            key_prefix: default_redis_key_prefix(),
            ttl_seconds: default_redis_ttl_seconds(),
        }
    }
}
