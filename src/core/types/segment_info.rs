// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

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
