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
pub mod segment_info;

// Re-export HealthStatus from algorithm module for convenience
pub use crate::core::algorithm::HealthStatus;

pub use error::*;
pub use id::*;
pub use metrics::*;
pub use segment_info::SegmentInfo;
