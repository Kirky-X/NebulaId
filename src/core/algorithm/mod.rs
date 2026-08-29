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

pub(crate) mod audit_trait;
pub(crate) mod degradation_manager;
pub mod router;
pub(crate) mod segment;
pub(crate) mod snowflake;
pub(crate) mod traits;
pub(crate) mod uuid_v8;

pub use traits::*;

pub use router::AlgorithmRouter;

pub use audit_trait::{AuditEvent, AuditEventType, AuditLogger, AuditResult, DynAuditLogger};

// CircuitBreakerState 由 degradation_manager 唯一定义（T005：删除生产零引用的
// 独立 circuit_breaker.rs 后，熔断器唯一实现为 DegradationManager 内建状态机）。
pub use degradation_manager::{CircuitBreakerState, DegradationManager};

// Re-export CpuMonitor for CPU monitoring
pub use segment::CpuMonitor;

// T010：Snowflake 位布局解析的唯一权威类型
pub use snowflake::{ParsedSnowflakeId, SnowflakeLayoutInfo};
