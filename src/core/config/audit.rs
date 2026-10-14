// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Audit persistence configuration.
//!
//! 内存容量与文件落盘收敛为独立配置域，与 rate_limit 解耦。

use serde::{Deserialize, Serialize};

/// 审计持久化配置。
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AuditConfig {
    /// 生产环境是否将审计事件持久化到文件。默认 `true`（满足 SOC2/GDPR 审计留痕）；
    /// 非生产构建维持内存环形，不新增文件写入。
    #[serde(default = "default_file_logging_enabled")]
    pub file_logging_enabled: bool,
    /// 审计文件路径（JSON Lines，逐事件一行）。默认 `logs/audit.log`。
    #[serde(default = "default_file_logging_path")]
    pub file_logging_path: String,
    /// 内存环形缓冲容量（条数上限，超出淘汰最旧）。默认 10000。
    #[serde(default = "default_memory_capacity")]
    pub memory_capacity: usize,
}

fn default_file_logging_enabled() -> bool {
    true
}

fn default_file_logging_path() -> String {
    "logs/audit.log".to_string()
}

fn default_memory_capacity() -> usize {
    10_000
}

impl Default for AuditConfig {
    fn default() -> Self {
        Self {
            file_logging_enabled: default_file_logging_enabled(),
            file_logging_path: default_file_logging_path(),
            memory_capacity: default_memory_capacity(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audit_config_defaults() {
        let config = AuditConfig::default();
        assert!(config.file_logging_enabled);
        assert_eq!(config.file_logging_path, "logs/audit.log");
        assert_eq!(config.memory_capacity, 10_000);
    }

    #[test]
    fn test_memory_capacity_is_independent_of_rate_limit() {
        let audit = AuditConfig::default();
        let rate_limit = crate::core::config::RateLimitConfig::default();
        assert_eq!(audit.memory_capacity, 10_000);
        assert_eq!(rate_limit.default_rps, 10_000);
        let audit = AuditConfig {
            memory_capacity: 555,
            ..Default::default()
        };
        assert_eq!(audit.memory_capacity, 555);
        assert_eq!(
            crate::core::config::RateLimitConfig::default().default_rps,
            10_000
        );
    }

    #[test]
    fn test_audit_config_deserializes_with_defaults() {
        let config: AuditConfig = toml::from_str("file_logging_path = \"var/audit.log\"")
            .expect("partial table must deserialize");
        assert!(config.file_logging_enabled);
        assert_eq!(config.file_logging_path, "var/audit.log");
        assert_eq!(config.memory_capacity, 10_000);
    }

    #[test]
    fn test_audit_config_rejects_unknown_fields() {
        let result = toml::from_str::<AuditConfig>("unknown_key = 1");
        assert!(result.is_err(), "未知键必须被 deny_unknown_fields 拒绝");
    }

    #[test]
    fn test_related_credentials_still_redacted_in_debug() {
        let auth = crate::core::config::AuthConfig::default();
        let rendered = format!("{auth:?}");
        assert!(rendered.contains("[REDACTED]"), "Debug 输出必须保留脱敏");
        assert!(
            !rendered.contains("test-secret-value-12345"),
            "Debug 输出不得包含明文 key_secret"
        );
    }
}
