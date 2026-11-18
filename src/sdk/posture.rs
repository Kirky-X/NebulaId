// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! SDK 安全姿态盘点（feature `sdk` 门控）。
//!
//! 嵌入式形态（纯算法、零 DB 零网络）没有 server 的认证/限流/审计/传输层，
//! 本模块把「失去哪些安全能力」变成可查询的三态报告，不改变构建或运行行为。

use crate::core::Config;

/// 纯函数盘点：输入 Config + 编译期 feature 探测，零 IO。
pub fn assess(config: &Config) -> SecurityPostureReport {
    use PostureStatus::*;

    let checks = vec![
        PostureCheck {
            name: "auth",
            status: if cfg!(feature = "garrison-auth") && config.auth.enabled {
                Enabled
            } else {
                Disabled
            },
            detail: t!("sdk.posture.check.auth.detail"),
        },
        PostureCheck {
            name: "rate-limit",
            status: Disabled,
            detail: t!("sdk.posture.check.rate-limit.detail"),
        },
        PostureCheck {
            name: "audit",
            status: if cfg!(feature = "server") {
                Enabled
            } else {
                Disabled
            },
            detail: t!("sdk.posture.check.audit.detail"),
        },
        PostureCheck {
            name: "tls",
            status: Disabled,
            detail: t!("sdk.posture.check.tls.detail"),
        },
        PostureCheck {
            name: "tenant-context",
            status: if cfg!(feature = "uuid-v8") && config.algorithm.uuid_v8.enabled {
                Enabled
            } else {
                Disabled
            },
            detail: t!("sdk.posture.check.tenant-context.detail"),
        },
        PostureCheck {
            name: "unpredictability",
            status: Undeclared,
            detail: t!("sdk.posture.check.unpredictability.detail"),
        },
    ];

    SecurityPostureReport { checks }
}

/// 安全能力状态。
///
/// `Disabled` 表示能力不在场；`Undeclared` 表示配置位存在但未声明强度，
/// 两者语义不同，不合并为布尔。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostureStatus {
    /// 能力在场且已启用。
    Enabled,
    /// 能力不在场（纯算法嵌入形态的默认状态）。
    Disabled,
    /// 能力相关配置存在，但未声明安全强度。
    Undeclared,
}

/// 单项安全能力盘点结果。
#[derive(Debug, Clone)]
pub struct PostureCheck {
    /// 检查项名称（`auth` / `rate-limit` / `audit` / `tls` / `tenant-context` / `unpredictability`）。
    pub name: &'static str,
    /// 该项状态。
    pub status: PostureStatus,
    /// 人类可读说明。
    pub detail: String,
}

/// 安全姿态报告。
#[derive(Debug, Clone)]
pub struct SecurityPostureReport {
    /// 全部检查项，顺序固定为 design 六项。
    pub checks: Vec<PostureCheck>,
}

impl SecurityPostureReport {
    /// 无 `Disabled` / `Undeclared` 时为真；供调用方做门禁判断。
    pub fn is_hardened(&self) -> bool {
        self.checks
            .iter()
            .all(|c| c.status == PostureStatus::Enabled)
    }

    /// 单行摘要（`N/M security capabilities enabled`），供 build 期日志输出。
    pub fn summary(&self) -> String {
        let enabled = self
            .checks
            .iter()
            .filter(|c| c.status == PostureStatus::Enabled)
            .count();
        format!(
            "{}/{} security capabilities enabled",
            enabled,
            self.checks.len()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::Config;

    fn check(name: &'static str, status: PostureStatus) -> PostureCheck {
        PostureCheck {
            name,
            status,
            detail: String::new(),
        }
    }

    #[test]
    fn test_is_hardened_false_when_undeclared_present() {
        let report = SecurityPostureReport {
            checks: vec![
                check("auth", PostureStatus::Enabled),
                check("unpredictability", PostureStatus::Undeclared),
            ],
        };
        assert!(
            !report.is_hardened(),
            "含 Undeclared 时不得判为 hardened（配置存在但未声明强度）"
        );
    }

    #[test]
    fn test_is_hardened_false_when_disabled_present() {
        let report = SecurityPostureReport {
            checks: vec![
                check("auth", PostureStatus::Enabled),
                check("tls", PostureStatus::Disabled),
            ],
        };
        assert!(!report.is_hardened(), "含 Disabled 时不得判为 hardened");
    }

    #[test]
    fn test_is_hardened_true_when_all_enabled() {
        let report = SecurityPostureReport {
            checks: vec![
                check("auth", PostureStatus::Enabled),
                check("tls", PostureStatus::Enabled),
            ],
        };
        assert!(report.is_hardened(), "全部 Enabled 时应判为 hardened");
    }

    #[test]
    fn test_summary_counts_enabled_over_total() {
        let report = SecurityPostureReport {
            checks: vec![
                check("auth", PostureStatus::Enabled),
                check("tls", PostureStatus::Disabled),
                check("audit", PostureStatus::Undeclared),
            ],
        };
        assert_eq!(report.summary(), "1/3 security capabilities enabled");
    }

    #[test]
    fn test_summary_of_empty_report() {
        let report = SecurityPostureReport { checks: vec![] };
        assert_eq!(report.summary(), "0/0 security capabilities enabled");
    }

    /// 盘点恒产出 design 六项，顺序与名称固定（供 example CLI 逐行输出）。
    #[test]
    fn test_assess_produces_six_named_checks() {
        let report = assess(&Config::default());
        let names: Vec<_> = report.checks.iter().map(|c| c.name).collect();
        assert_eq!(
            names,
            [
                "auth",
                "rate-limit",
                "audit",
                "tls",
                "tenant-context",
                "unpredictability"
            ]
        );
        assert!(
            report.checks.iter().all(|c| !c.detail.is_empty()),
            "每项检查必须带非空 detail 说明"
        );
    }

    /// garrison-auth 关闭时 auth 恒 Disabled；开启与否只看 config.auth.enabled。
    #[test]
    fn test_assess_auth_status_reflects_feature_and_config() {
        let report = assess(&Config::default());
        let auth = report.checks.iter().find(|c| c.name == "auth").unwrap();
        let expected = if cfg!(feature = "garrison-auth") {
            PostureStatus::Enabled
        } else {
            PostureStatus::Disabled
        };
        assert_eq!(auth.status, expected);
    }

    /// uuid-v8 enabled 时 tenant-context Enabled；关闭时 Disabled。
    #[test]
    fn test_assess_tenant_context_two_states() {
        let mut config = Config::default();
        let enabled = assess(&config);
        let tenant = enabled
            .checks
            .iter()
            .find(|c| c.name == "tenant-context")
            .unwrap();
        assert_eq!(
            tenant.status,
            if cfg!(feature = "uuid-v8") {
                PostureStatus::Enabled
            } else {
                PostureStatus::Disabled
            }
        );

        config.algorithm.uuid_v8.enabled = false;
        let disabled = assess(&config);
        let tenant = disabled
            .checks
            .iter()
            .find(|c| c.name == "tenant-context")
            .unwrap();
        assert_eq!(tenant.status, PostureStatus::Disabled);
    }

    /// 速率限制/TLS 在 SDK 形态恒 Disabled（无 HTTP 栈挂载）。
    #[test]
    fn test_assess_rate_limit_and_tls_always_disabled_in_sdk_mode() {
        let mut config = Config::default();
        config.rate_limit.enabled = true;
        config.tls.enabled = true;
        let report = assess(&config);
        for name in ["rate-limit", "tls"] {
            let check = report.checks.iter().find(|c| c.name == name).unwrap();
            assert_eq!(check.status, PostureStatus::Disabled);
            assert!(
                check.detail.contains("server") || check.detail.contains("嵌入"),
                "detail 须注明 server-only/嵌入方责任：{}",
                check.detail
            );
        }
    }

    /// 审计：未注入审计日志器时 Disabled（SDK assess 是纯配置盘点，无 logger 入参）。
    #[test]
    fn test_assess_audit_disabled_without_server_feature() {
        let report = assess(&Config::default());
        let audit = report.checks.iter().find(|c| c.name == "audit").unwrap();
        let expected = if cfg!(feature = "server") {
            PostureStatus::Enabled
        } else {
            PostureStatus::Disabled
        };
        assert_eq!(audit.status, expected);
    }

    /// 不可预测性声明：id-unpredictability-profile 落地前恒 Undeclared。
    #[test]
    fn test_assess_unpredictability_always_undeclared() {
        let report = assess(&Config::default());
        let check = report
            .checks
            .iter()
            .find(|c| c.name == "unpredictability")
            .unwrap();
        assert_eq!(check.status, PostureStatus::Undeclared);
    }
}
