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
#[cfg(feature = "uuid-v4")]
pub(crate) mod uuid_v4;
#[cfg(feature = "uuid-v5")]
pub(crate) mod uuid_v5;
#[cfg(feature = "uuid-v7")]
pub(crate) mod uuid_v7;
#[cfg(feature = "uuid-v8")]
pub(crate) mod uuid_v8;
// RFC 9562 v5 确定性派生的公开路径：直调与 SDK 薄包装共用同一实现。
#[cfg(feature = "uuid-v5")]
pub use uuid_v5::{derive_uuid_v5, derive_uuid_v5_batch, derive_uuid_v5_bytes};
// RFC 9562 §5.4 v4 同步发号叶子入口：发电电路径（UuidV4Impl）与同步调用点共用同一实现。
#[cfg(feature = "uuid-v4")]
pub use uuid_v4::mint_uuid_v4;

pub use traits::*;

pub use router::AlgorithmRouter;

pub use audit_trait::{AuditEvent, AuditEventType, AuditLogger, AuditResult, DynAuditLogger};

// CircuitBreakerState 唯一定义于 degradation_manager。
pub use degradation_manager::{CircuitBreakerState, DegradationManager};

#[cfg(feature = "segment")]
pub use segment::CpuMonitor;

// Segment 号段装配面：DbSegmentLoader 供生产注入；SegmentData 是
// `SegmentLoader::load_segment` 的返回类型，公开以支持外部基准/宿主实现装载器。
#[cfg(feature = "segment")]
pub use segment::{DbSegmentLoader, SegmentAlgorithm, SegmentData, SegmentLoader};

// Snowflake 位布局解析的唯一权威类型
#[cfg(feature = "snowflake")]
pub use snowflake::{ParsedSnowflakeId, SnowflakeLayoutInfo};
