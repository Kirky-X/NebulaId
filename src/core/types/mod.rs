// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

pub mod error;
pub mod id;
pub mod metrics;
pub mod segment_info;

// Re-export HealthStatus from algorithm module for convenience
pub use crate::core::algorithm::HealthStatus;

pub use error::*;
pub use id::*;
pub use metrics::*;
pub use segment_info::SegmentInfo;
