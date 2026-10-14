// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Monitoring module for Nebula ID.

pub mod core;

// 告警 re-export 与 core.rs 项级门控一致：plain test 加 --features alerting，CI 用 --all-features。
#[cfg(feature = "alerting")]
pub use core::{
    Alert, AlertError, AlertManager, AlertNotificationSender, AlertRule, AlertSeverity, AlertState,
    AlertStatus, AlertingConfig, ChannelType, DefaultEvaluator, NotificationChannel,
};
