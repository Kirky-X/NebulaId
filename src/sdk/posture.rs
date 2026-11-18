// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! SDK 安全姿态盘点（feature `sdk` 门控）。
//!
//! 嵌入式形态（纯算法、零 DB 零网络）没有 server 的认证/限流/审计/传输层，
//! 本模块把「失去哪些安全能力」变成可查询的三态报告，不改变构建或运行行为。

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
}
