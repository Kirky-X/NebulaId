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
pub(crate) mod circuit_breaker;
pub(crate) mod degradation_manager;
pub mod router;
pub(crate) mod segment;
pub(crate) mod snowflake;
pub(crate) mod traits;
pub(crate) mod uuid_v7;

pub use traits::*;

pub use router::AlgorithmRouter;

pub use audit_trait::{AuditEvent, AuditEventType, AuditLogger, AuditResult, DynAuditLogger};

pub use circuit_breaker::{CircuitBreaker, CircuitBreakerConfig, CircuitBreakerState};

pub use degradation_manager::DegradationManager;

// Re-export CpuMonitor for CPU monitoring
pub use segment::CpuMonitor;
