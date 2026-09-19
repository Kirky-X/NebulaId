// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Monitoring configuration.

use serde::{Deserialize, Serialize};

/// Monitoring configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MonitoringConfig {
    /// Enable Prometheus metrics
    pub metrics_enabled: bool,
    /// Metrics endpoint path
    pub metrics_path: String,
    /// Enable OpenTelemetry tracing
    pub tracing_enabled: bool,
    /// OpenTelemetry collector endpoint
    pub otlp_endpoint: String,
}

impl Default for MonitoringConfig {
    fn default() -> Self {
        Self {
            metrics_enabled: true,
            metrics_path: "/metrics".to_string(),
            tracing_enabled: false,
            otlp_endpoint: "".to_string(),
        }
    }
}
