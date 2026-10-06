// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Monitoring module for Nebula ID.

pub mod core;

// 告警子系统 re-export 随纯 `alerting` feature 门控(与
// core.rs 的项级门控一致;本地 plain test 需 --features alerting,
// CI 经 --all-features 覆盖)。
#[cfg(feature = "alerting")]
pub use core::{
    Alert, AlertError, AlertManager, AlertNotificationSender, AlertRule, AlertSeverity, AlertState,
    AlertStatus, AlertingConfig, ChannelType, DefaultEvaluator, NotificationChannel,
};
