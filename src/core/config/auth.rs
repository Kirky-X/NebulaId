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

//! Authentication configuration.

use serde::{Deserialize, Serialize};

/// API key entry for configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
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

/// Authentication configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
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
    /// L16 修复：密钥轮换宽限期（秒）。`> 0` 时旧密钥在宽限期内仍然有效，
    /// 避免轮换瞬间造成请求失败；`0`（默认，T011）表示不设宽限期，轮换后上一代
    /// 凭证立即失效。上限 30 天，超限值由 `ApiHandlers::with_key_rotation_grace_period`
    /// clamp。
    #[serde(default = "default_key_rotation_grace_period_seconds")]
    pub key_rotation_grace_period_seconds: u64,
}

fn default_api_key_salt() -> String {
    // Phase 9 T043 (HIGH H1 / tiangang HIGH-1) — never fall back to a
    // hard-coded salt. The garrison-based auth path
    // already panics in production when `NEBULA_API_KEY_SALT` is unset;
    // this function returns an empty string so the empty-ness check in

    // dev/test the manager generates a random per-process salt, so an
    // empty string here is safe for non-production builds.
    std::env::var("NEBULA_API_KEY_SALT").unwrap_or_default()
}

/// T011（D-A）：默认宽限期 = `0`（关闭）。历史上该值是 7 天，与 L16 之前的
/// 硬编码 `const GRACE_PERIOD_SECONDS: u64 = 7 * 24 * 60 * 60` 一致。
///
/// ARCH-MED-002 修复：把该常量提取为 `pub const`，所有调用方统一引用，
/// 避免未来调整默认值时霰弹手术。原三处重复：
/// - `auth.rs::default_key_rotation_grace_period_seconds()` (本文件)
/// - `handlers/mod.rs::DEFAULT_KEY_ROTATION_GRACE_PERIOD_SECONDS`
/// - `config_adapter.rs::unwrap_or(7 * 24 * 60 * 60)`
///
/// T009（code-hygiene-cleanup）：常量本体已迁入
/// [`crate::core::config::defaults`] 统一注册表，旧路径别名按"直接废弃"
/// 原则移除（无任何消费方后不再保留）。本函数只是注册表常量的 serde 适配器，
/// 改默认值只需动注册表那一行。
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

    /// T011（D-A）：宽限期默认关闭。旧默认 7 天让"轮换"在整整一周内同时接受两代
    /// 凭证，等于把泄露过的 secret 保持有效一周。
    #[test]
    fn test_auth_config_default_grace_period_is_zero() {
        assert_eq!(
            AuthConfig::default().key_rotation_grace_period_seconds,
            0,
            "默认必须为 0（轮换后上一代凭证立即失效），开启需显式配置"
        );
    }

    /// `#[serde(default = ...)]` 与 `Default` 必须同源：真实启动走的是 TOML 反序列化，
    /// 配置文件不写该键时也得拿到同一个"关闭"值。
    #[test]
    fn test_auth_config_missing_grace_period_field_defaults_to_zero() {
        let config: AuthConfig =
            toml::from_str("enabled = true\ncache_ttl_seconds = 300\n").expect("应可解析");
        assert_eq!(config.key_rotation_grace_period_seconds, 0);
    }
}
