// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! SDK 安全姿态诊断（example CLI）：打印六项检查的三态与建议。
//!
//! 运行：
//! ```bash
//! cargo run --package nebulaid --example sdk_doctor --features sdk
//! ```

use nebulaid::core::Config;
use nebulaid::sdk::{NebulaIdKitBuilder, PostureStatus, SecurityPostureReport};

/// 三态符号：✓ Enabled / ✗ Disabled / ! Undeclared。
fn status_symbol(status: PostureStatus) -> char {
    match status {
        PostureStatus::Enabled => '✓',
        PostureStatus::Disabled => '✗',
        PostureStatus::Undeclared => '!',
    }
}

/// 建议规则：多租户嵌入（Disabled ≥2 且 tenant-context Disabled）优先；
/// 否则有 Disabled 项时输出通用嵌入建议，保证诊断输出恒有可操作下一步。
fn advice(report: &SecurityPostureReport) -> Option<&'static str> {
    let disabled = report
        .checks
        .iter()
        .filter(|c| c.status == PostureStatus::Disabled)
        .count();
    let tenant_disabled = report
        .checks
        .iter()
        .any(|c| c.name == "tenant-context" && c.status == PostureStatus::Disabled);
    if disabled >= 2 && tenant_disabled {
        Some("multi-tenant embedding: enable uuid-v8 or pass tenant identity in your service")
    } else if disabled > 0 {
        Some("SDK embedding: disabled capabilities must be provided by your host service")
    } else {
        None
    }
}

#[tokio::main]
async fn main() -> nebulaid::core::Result<()> {
    // 默认算法须为 snowflake：Config::default() 的 segment 需数据库，未注入会被拒绝。
    let mut config = Config::default();
    config.algorithm.default = "snowflake".to_string();

    let kit = NebulaIdKitBuilder::new(config).build().await?;
    let report = kit.security_posture();

    println!("Nebula ID SDK security posture");
    for check in &report.checks {
        println!(
            "{} {} — {}",
            status_symbol(check.status),
            check.name,
            check.detail
        );
    }
    println!("summary: {}", report.summary());
    if let Some(suggestion) = advice(&report) {
        println!("advice: {suggestion}");
    }

    kit.shutdown().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nebulaid::sdk::PostureCheck;

    fn check(name: &'static str, status: PostureStatus) -> PostureCheck {
        PostureCheck {
            name,
            status,
            detail: "detail".to_string(),
        }
    }

    #[test]
    fn test_status_symbols() {
        assert_eq!(status_symbol(PostureStatus::Enabled), '✓');
        assert_eq!(status_symbol(PostureStatus::Disabled), '✗');
        assert_eq!(status_symbol(PostureStatus::Undeclared), '!');
    }

    #[test]
    fn test_advice_fires_when_two_disabled_and_tenant_disabled() {
        let report = SecurityPostureReport {
            checks: vec![
                check("auth", PostureStatus::Disabled),
                check("tenant-context", PostureStatus::Disabled),
                check("unpredictability", PostureStatus::Undeclared),
            ],
        };
        assert!(advice(&report).is_some());
    }

    #[test]
    fn test_advice_generic_when_tenant_enabled() {
        let report = SecurityPostureReport {
            checks: vec![
                check("auth", PostureStatus::Disabled),
                check("tenant-context", PostureStatus::Enabled),
                check("unpredictability", PostureStatus::Undeclared),
            ],
        };
        assert_eq!(
            advice(&report),
            Some("SDK embedding: disabled capabilities must be provided by your host service")
        );
    }

    #[test]
    fn test_advice_silent_when_all_enabled() {
        let report = SecurityPostureReport {
            checks: vec![
                check("auth", PostureStatus::Enabled),
                check("tenant-context", PostureStatus::Enabled),
                check("unpredictability", PostureStatus::Enabled),
            ],
        };
        assert!(advice(&report).is_none());
    }
}
