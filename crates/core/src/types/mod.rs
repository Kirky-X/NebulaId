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

pub mod error;
pub mod id;
pub mod metrics;

pub use error::*;
pub use id::*;
pub use metrics::*;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SegmentInfo {
    pub id: i64,
    pub workspace_id: String,
    pub biz_tag: String,
    pub current_id: i64,
    pub max_id: i64,
    pub step: u32,
    pub delta: u32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl SegmentInfo {
    #[allow(dead_code)]
    pub fn new(
        workspace_id: String,
        biz_tag: String,
        current_id: i64,
        max_id: i64,
        step: u32,
        delta: u32,
    ) -> Self {
        Self {
            id: 0,
            workspace_id,
            biz_tag,
            current_id,
            max_id,
            step,
            delta,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }
}
