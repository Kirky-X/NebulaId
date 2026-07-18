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
}

fn default_api_key_salt() -> String {
    std::env::var("NEBULA_API_KEY_SALT").unwrap_or_else(|_| "nebula_default_salt".to_string())
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            cache_ttl_seconds: 300,
            api_keys: vec![],
            api_key_salt: default_api_key_salt(),
        }
    }
}
