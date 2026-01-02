pub mod error;
pub mod id;
pub mod metrics;

pub use error::*;
pub use id::*;
pub use metrics::*;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SegmentInfo {
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
