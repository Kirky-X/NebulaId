// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

pub(crate) mod audit_trait;
pub(crate) mod degradation_manager;
pub mod router;
#[cfg(feature = "segment")]
pub(crate) mod segment;
#[cfg(feature = "snowflake")]
pub(crate) mod snowflake;
#[cfg(feature = "snowflake")]
pub(crate) mod snowflake_state;
pub(crate) mod traits;
#[cfg(feature = "uuid-v5")]
pub(crate) mod uuid_v5;
#[cfg(feature = "uuid-v8")]
pub(crate) mod uuid_v8;
// RFC 9562 v5 确定性派生的稳定公开路径：直调消费者（嵌入方不需要 Kit 装配）
// 与 SDK 薄包装（IdGenerator::derive_uuid_v5）共用同一实现。
#[cfg(feature = "uuid-v5")]
pub use uuid_v5::{derive_uuid_v5, derive_uuid_v5_batch};

pub use traits::*;

pub use router::AlgorithmRouter;

pub use audit_trait::{AuditEvent, AuditEventType, AuditLogger, AuditResult, DynAuditLogger};

// CircuitBreakerState 由 degradation_manager 唯一定义（删除生产零引用的
// 独立 circuit_breaker.rs 后，熔断器唯一实现为 DegradationManager 内建状态机）。
pub use degradation_manager::{CircuitBreakerState, DegradationManager};

// Re-export CpuMonitor for CPU monitoring
#[cfg(feature = "segment")]
pub use segment::CpuMonitor;

// Segment 号段装配面：`DbSegmentLoader`（真连 DB 号段）与
// `SegmentLoader`/`SegmentAlgorithm` 经此 re-export 后，装配方（main.rs /
// 嵌入式 SDK）可执行 `SegmentAlgorithm::new(dc).with_segment_loader(
// Arc::new(DbSegmentLoader::new(repo)))` 完成生产注入。
#[cfg(feature = "segment")]
pub use segment::{DbSegmentLoader, SegmentAlgorithm, SegmentLoader};

// Snowflake 位布局解析的唯一权威类型
#[cfg(feature = "snowflake")]
pub use snowflake::{ParsedSnowflakeId, SnowflakeLayoutInfo};
