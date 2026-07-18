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

//! TLS configuration.

use serde::{Deserialize, Serialize};

/// TLS 版本配置
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum TlsVersion {
    /// TLS 1.2
    Tls12,
    /// TLS 1.3 (推荐)
    #[default]
    Tls13,
}

impl std::fmt::Display for TlsVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TlsVersion::Tls12 => write!(f, "TLSv1.2"),
            TlsVersion::Tls13 => write!(f, "TLSv1.3"),
        }
    }
}

/// TLS configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TlsConfig {
    /// Enable/disable TLS
    pub enabled: bool,
    /// Path to TLS certificate file
    pub cert_path: String,
    /// Path to TLS private key file
    pub key_path: String,
    /// Path to CA certificate file (optional)
    pub ca_path: Option<String>,
    /// Enable TLS for HTTP
    pub http_enabled: bool,
    /// Enable TLS for gRPC
    pub grpc_enabled: bool,
    /// Minimum TLS version (default: TLS 1.3)
    #[serde(default)]
    pub min_tls_version: TlsVersion,
    /// ALPN protocols for HTTP/2 support
    #[serde(default)]
    pub alpn_protocols: Vec<String>,
}

impl Default for TlsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            cert_path: "".to_string(),
            key_path: "".to_string(),
            ca_path: None,
            http_enabled: false,
            grpc_enabled: false,
            min_tls_version: TlsVersion::Tls13,
            alpn_protocols: vec!["h2".to_string(), "http/1.1".to_string()],
        }
    }
}
