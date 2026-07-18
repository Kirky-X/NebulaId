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

//! Batch generation configuration.

use serde::{Deserialize, Serialize};

/// Batch generation configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
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
