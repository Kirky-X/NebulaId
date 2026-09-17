// Copyright © 2026 Kirky.X
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Monitoring module for Nebula ID.

pub mod core;

// T039 —— 告警子系统 re-export 随 `alerting` feature 门控(与
// core.rs 的项级门控同 cfg:test 构建恒包含,使 e2e 测试在默认
// feature 下照常编译)。
#[cfg(any(test, feature = "alerting"))]
pub use core::{
    Alert, AlertError, AlertManager, AlertNotificationSender, AlertRule, AlertSeverity, AlertState,
    AlertStatus, AlertingConfig, ChannelType, DefaultEvaluator, NotificationChannel,
};
