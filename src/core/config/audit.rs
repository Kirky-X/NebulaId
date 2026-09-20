// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Audit persistence configuration.
//!
//! 审计事件的内存容量与文件持久化此前借用 `rate_limit.default_rps`
//! 作为容量、硬编码在装配处，既无独立语义也无独立开关。本配置把
//! 「容量」「是否落盘」「落盘路径」收敛为独立配置域。

use serde::{Deserialize, Serialize};

/// 审计持久化配置。
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AuditConfig {
    /// 生产环境是否将审计事件持久化到文件。默认 `true`（生产默认开启，
    /// 满足 SOC2/GDPR 审计留痕）。非生产构建维持内存环形，不新增文件写入。
    #[serde(default = "default_file_logging_enabled")]
    pub file_logging_enabled: bool,
    /// 审计文件路径（JSON Lines，逐事件一行）。默认 `logs/audit.log`。
    #[serde(default = "default_file_logging_path")]
    pub file_logging_path: String,
    /// 内存环形缓冲容量（条数上限，超出淘汰最旧）。默认 10000。
    /// 与 `rate_limit` 配置完全解耦：容量语义是「内存中可回查的事件数」，
    /// 与限流速率无关。
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

    /// 三字段默认值：file_logging_enabled=true / path="logs/audit.log" /
    /// memory_capacity=10000。
    #[test]
    fn test_audit_config_defaults() {
        let config = AuditConfig::default();
        assert!(config.file_logging_enabled);
        assert_eq!(config.file_logging_path, "logs/audit.log");
        assert_eq!(config.memory_capacity, 10_000);
    }

    /// 内存容量独立于 rate_limit：AuditConfig 的容量默认值与 RateLimitConfig
    /// 的 default_rps 无任何推导关系（核心解耦点）。
    #[test]
    fn test_memory_capacity_is_independent_of_rate_limit() {
        let audit = AuditConfig::default();
        let rate_limit = crate::core::config::RateLimitConfig::default();
        // 二者恰好在默认值上不同量纲：容量是条数，rps 是速率。断言独立
        // 配置域的存在性而非数值巧合。
        assert_eq!(audit.memory_capacity, 10_000);
        assert_eq!(rate_limit.default_rps, 10_000);
        // 显式覆盖容量不影响（也不读取）rate_limit。
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

    /// TOML 反序列化：缺省字段取默认（与既有配置文件的向后兼容语义一致）。
    #[test]
    fn test_audit_config_deserializes_with_defaults() {
        let config: AuditConfig = toml::from_str("file_logging_path = \"var/audit.log\"")
            .expect("partial table must deserialize");
        assert!(config.file_logging_enabled);
        assert_eq!(config.file_logging_path, "var/audit.log");
        assert_eq!(config.memory_capacity, 10_000);
    }

    /// deny_unknown_fields 风格与既有配置一致：未知键必须拒绝。
    #[test]
    fn test_audit_config_rejects_unknown_fields() {
        let result = toml::from_str::<AuditConfig>("unknown_key = 1");
        assert!(result.is_err(), "未知键必须被 deny_unknown_fields 拒绝");
    }

    /// 脱敏仍生效：审计相关配置域的 Debug 输出不得泄露凭证材料
    /// （auth 配置的 key_secret / api_key_salt 仍为 [REDACTED]）。
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
