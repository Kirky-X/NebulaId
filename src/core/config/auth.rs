// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Authentication configuration.

use serde::{Deserialize, Serialize};

/// API key entry for configuration
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ApiKeyEntry {
    /// Unique key identifier
    pub key_id: String,
    /// Key secret for authentication
    pub key_secret: String,
    /// Associated workspace
    pub workspace: String,
    /// Key role (admin/user)
    pub role: String,
    /// Rate limit (requests per second)
    pub rate_limit: u32,
    /// Key name for identification
    pub name: String,
}

/// 手写 `Debug`：`key_secret` 是明文凭证，不得进 `{:?}` 输出（CWE-532）；其余字段照常输出。
impl std::fmt::Debug for ApiKeyEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApiKeyEntry")
            .field("key_id", &self.key_id)
            .field("key_secret", &"[REDACTED]")
            .field("workspace", &self.workspace)
            .field("role", &self.role)
            .field("rate_limit", &self.rate_limit)
            .field("name", &self.name)
            .finish()
    }
}

/// Authentication configuration
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuthConfig {
    /// Enable/disable authentication
    pub enabled: bool,
    /// Cache TTL (seconds)
    pub cache_ttl_seconds: u64,
    /// List of API keys
    #[serde(default)]
    pub api_keys: Vec<ApiKeyEntry>,
    /// Salt for API key hashing
    #[serde(default = "default_api_key_salt")]
    pub api_key_salt: String,
    /// 密钥轮换宽限期（秒）：`> 0` 时旧密钥仍有效，避免轮换瞬间请求失败；
    /// 默认 `0` 不设宽限期，上限 30 天（超限由 `ApiHandlers::with_key_rotation_grace_period` clamp）。
    #[serde(default = "default_key_rotation_grace_period_seconds")]
    pub key_rotation_grace_period_seconds: u64,
}

/// 手写 `Debug`：`api_key_salt` 是密钥哈希的 pepper，同属不得进日志/panic 输出的
/// 凭证材料（CWE-532）。
impl std::fmt::Debug for AuthConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthConfig")
            .field("enabled", &self.enabled)
            .field("cache_ttl_seconds", &self.cache_ttl_seconds)
            .field("api_keys", &self.api_keys)
            .field("api_key_salt", &"[REDACTED]")
            .field(
                "key_rotation_grace_period_seconds",
                &self.key_rotation_grace_period_seconds,
            )
            .finish()
    }
}

fn default_api_key_salt() -> String {
    // 不回退到硬编码 salt：生产缺 `NEBULA_API_KEY_SALT` 时 garrison 鉴权路径直接 panic；
    // 返回空串，非生产构建由 manager 生成随机 per-process salt。
    std::env::var("NEBULA_API_KEY_SALT").unwrap_or_default()
}

/// 默认宽限期 = `0`（关闭）。常量本体在 [`crate::core::config::defaults`] 注册表，
/// 调整默认值只需改注册表那一行。
fn default_key_rotation_grace_period_seconds() -> u64 {
    crate::core::config::defaults::DEFAULT_KEY_ROTATION_GRACE_PERIOD_SECONDS
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            cache_ttl_seconds: 300,
            api_keys: vec![],
            api_key_salt: default_api_key_salt(),
            key_rotation_grace_period_seconds: default_key_rotation_grace_period_seconds(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 默认必须为 0：轮换后上一代凭证立即失效，开启需显式配置。
    #[test]
    fn test_auth_config_default_grace_period_is_zero() {
        assert_eq!(
            AuthConfig::default().key_rotation_grace_period_seconds,
            0,
            "默认必须为 0（轮换后上一代凭证立即失效），开启需显式配置"
        );
    }

    /// serde 默认与 `Default` 必须同源：启动走 TOML 反序列化，缺键也须得到同一值。
    #[test]
    fn test_auth_config_missing_grace_period_field_defaults_to_zero() {
        let config: AuthConfig =
            toml::from_str("enabled = true\ncache_ttl_seconds = 300\n").expect("应可解析");
        assert_eq!(config.key_rotation_grace_period_seconds, 0);
    }
}
