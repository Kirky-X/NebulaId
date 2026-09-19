// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

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
#[serde(deny_unknown_fields)]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tls_version_display() {
        assert_eq!(TlsVersion::Tls12.to_string(), "TLSv1.2");
        assert_eq!(TlsVersion::Tls13.to_string(), "TLSv1.3");
    }

    #[test]
    fn test_tls_config_default_uses_tls13() {
        let config = TlsConfig::default();
        assert_eq!(config.min_tls_version, TlsVersion::Tls13);
        assert!(!config.enabled);
    }
}
