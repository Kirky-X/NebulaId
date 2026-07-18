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

//! Algorithm configuration (segment / snowflake / uuid_v7).

use crate::core::types::AlgorithmType;
use serde::{Deserialize, Serialize};

/// Segment algorithm configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SegmentAlgorithmConfig {
    /// Base step size for ID allocation
    pub base_step: u64,
    /// Minimum step size
    pub min_step: u64,
    /// Maximum step size
    pub max_step: u64,
    /// Threshold for dynamic step adjustment
    pub switch_threshold: f64,
}

impl Default for SegmentAlgorithmConfig {
    fn default() -> Self {
        Self {
            base_step: 1000,
            min_step: 500,
            max_step: 100000,
            switch_threshold: 0.1,
        }
    }
}

/// Snowflake algorithm configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SnowflakeAlgorithmConfig {
    /// Number of bits for datacenter ID
    pub datacenter_id_bits: u8,
    /// Number of bits for worker ID
    pub worker_id_bits: u8,
    /// Number of bits for sequence number
    pub sequence_bits: u8,
    /// Clock drift threshold (milliseconds)
    pub clock_drift_threshold_ms: u64,
}

impl SnowflakeAlgorithmConfig {
    pub fn datacenter_id_mask(&self) -> u64 {
        (1 << self.datacenter_id_bits) - 1
    }

    pub fn worker_id_mask(&self) -> u64 {
        (1 << self.worker_id_bits) - 1
    }

    pub fn sequence_mask(&self) -> u64 {
        (1 << self.sequence_bits) - 1
    }

    pub fn timestamp_bits(&self) -> u8 {
        64 - self.datacenter_id_bits - self.worker_id_bits - self.sequence_bits
    }
}

impl Default for SnowflakeAlgorithmConfig {
    fn default() -> Self {
        Self {
            datacenter_id_bits: 3,
            worker_id_bits: 8,
            sequence_bits: 10,
            clock_drift_threshold_ms: 1000,
        }
    }
}

/// UUID v7 configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UuidV7Config {
    /// Enable/disable UUID v7 generation
    pub enabled: bool,
}

impl Default for UuidV7Config {
    fn default() -> Self {
        Self { enabled: true }
    }
}

/// Algorithm configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AlgorithmConfig {
    /// Default algorithm type
    pub default: String,
    /// Segment algorithm settings
    pub segment: SegmentAlgorithmConfig,
    /// Snowflake algorithm settings
    pub snowflake: SnowflakeAlgorithmConfig,
    /// UUID v7 settings
    pub uuid_v7: UuidV7Config,
}

impl Default for AlgorithmConfig {
    fn default() -> Self {
        Self {
            default: "segment".to_string(),
            segment: SegmentAlgorithmConfig::default(),
            snowflake: SnowflakeAlgorithmConfig::default(),
            uuid_v7: UuidV7Config::default(),
        }
    }
}

impl AlgorithmConfig {
    pub fn get_default_algorithm(&self) -> AlgorithmType {
        self.default.parse().unwrap_or(AlgorithmType::Segment)
    }
}
