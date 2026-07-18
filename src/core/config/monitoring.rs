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

//! Monitoring configuration.

use serde::{Deserialize, Serialize};

/// Monitoring configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
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
