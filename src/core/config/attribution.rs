// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! 归属强制配置：受保护 workspace 的 ID 降级链过滤策略。
//!
//! 配置文件静态清单（NEEDS CLARIFICATION 默认值）：不依赖 DB schema 变更，
//! 热更新受 reload 限制；DB 安全级别字段方案列为后续独立变更。

use serde::{Deserialize, Serialize};

/// 租户归属强制配置。
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AttributionConfig {
    /// 是否强制受保护 workspace 的 ID 携带租户上下文。
    #[serde(default)]
    pub enforce_tenant_context: bool,
    /// 受保护 workspace 精确名单；`enforce_tenant_context` 为 true 时生效。
    #[serde(default)]
    pub protected_workspaces: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_attribution_config_default_disabled_and_empty() {
        let config = AttributionConfig::default();
        assert!(!config.enforce_tenant_context);
        assert!(config.protected_workspaces.is_empty());
    }

    /// 既有配置文件无 `[attribution]` 段时，serde default 须给出同一默认值。
    #[test]
    fn test_attribution_config_missing_fields_use_defaults() {
        let config: AttributionConfig = toml::from_str("").expect("空 TOML 应可解析");
        assert!(!config.enforce_tenant_context);
        assert!(config.protected_workspaces.is_empty());
    }

    #[test]
    fn test_attribution_config_parses_explicit_values() {
        let config: AttributionConfig = toml::from_str(
            "enforce_tenant_context = true\nprotected_workspaces = [\"ws-a\", \"ws-b\"]\n",
        )
        .expect("显式配置应可解析");
        assert!(config.enforce_tenant_context);
        assert_eq!(config.protected_workspaces, ["ws-a", "ws-b"]);
    }

    #[test]
    fn test_attribution_config_rejects_unknown_field() {
        let parsed: std::result::Result<AttributionConfig, _> =
            toml::from_str("unknown_field = true\n");
        assert!(parsed.is_err(), "deny_unknown_fields 必须拒绝未知字段");
    }
}
