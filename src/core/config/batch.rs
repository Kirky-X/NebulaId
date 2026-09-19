// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Batch generation configuration.

use serde::{Deserialize, Serialize};

/// Batch generation configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BatchGenerateConfig {
    /// Maximum batch size for bulk ID generation
    pub max_batch_size: u32,
}

impl Default for BatchGenerateConfig {
    fn default() -> Self {
        Self {
            max_batch_size: 100,
        }
    }
}
