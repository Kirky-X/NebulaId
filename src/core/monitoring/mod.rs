// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Monitoring module for Nebula ID.

pub mod core;

// 告警子系统 re-export 随 `alerting` feature 门控(与
// core.rs 的项级门控同 cfg:test 构建恒包含,使 e2e 测试在默认
// feature 下照常编译)。
#[cfg(any(test, feature = "alerting"))]
pub use core::{
    Alert, AlertError, AlertManager, AlertNotificationSender, AlertRule, AlertSeverity, AlertState,
    AlertStatus, AlertingConfig, ChannelType, DefaultEvaluator, NotificationChannel,
};
