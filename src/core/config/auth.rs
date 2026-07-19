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
    /// L16 修复：密钥轮换宽限期（秒）。
    /// 旧密钥在宽限期内仍然有效，避免轮换瞬间造成请求失败。
    /// 默认 7 天，与原 `const GRACE_PERIOD_SECONDS: u64 = 7 * 24 * 60 * 60` 保持一致。
    #[serde(default = "default_key_rotation_grace_period_seconds")]
    pub key_rotation_grace_period_seconds: u64,
}

fn default_api_key_salt() -> String {
    // Phase 9 T043 (HIGH H1 / tiangang HIGH-1) — never fall back to a
    // hard-coded salt. AuthManager::from_env() (see `core/auth/manager.rs`)
    // already panics in production when `NEBULA_API_KEY_SALT` is unset;
    // this function returns an empty string so the empty-ness check in
    // `AuthManager` triggers the same panic-on-missing-env path. In
    // dev/test the manager generates a random per-process salt, so an
    // empty string here is safe for non-production builds.
    std::env::var("NEBULA_API_KEY_SALT").unwrap_or_default()
}

/// L16 修复：默认密钥轮换宽限期 = 7 天（与原硬编码值一致）。
///
/// ARCH-MED-002 修复：把该常量提取为 `pub const`，所有调用方统一引用，
/// 避免未来调整默认值时霰弹手术。原三处重复：
/// - `auth.rs::default_key_rotation_grace_period_seconds()` (本文件)
/// - `handlers/mod.rs::DEFAULT_KEY_ROTATION_GRACE_PERIOD_SECONDS`
/// - `config_adapter.rs::unwrap_or(7 * 24 * 60 * 60)`
pub const DEFAULT_KEY_ROTATION_GRACE_PERIOD_SECONDS: u64 = 7 * 24 * 60 * 60;

/// L16 修复：serde 默认值函数（引用上述常量）。
fn default_key_rotation_grace_period_seconds() -> u64 {
    DEFAULT_KEY_ROTATION_GRACE_PERIOD_SECONDS
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
