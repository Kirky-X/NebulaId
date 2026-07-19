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

//! Phase 9 T043 (HIGH H5) — file-level `#![allow(dead_code)]` retained
//! with explicit justification. This file hosts both the production
//! Snowflake algorithm and the (test-only) `UuidV7Algorithm` /
//! `UuidV4Algorithm` / `UuidMetrics` / `SnowflakeAlgorithmBuilder`
//! implementations. The Uuid variants are exercised by unit tests
//! but not registered in the production `AlgorithmBuilder` registry
//! (production uses the dedicated `uuid_v7.rs` module). They are
//! retained because (a) the tests validate the IdAlgorithm trait
//! contract for UUID-style generators, (b) the Builder pattern is
//! the documented extension point for injecting custom worker-id
//! allocators, and (c) deleting them would drop ~15 tests.
//! Re-evaluate at v0.3.0 once the algorithm-registration story
//! consolidates onto a single registry.

#![allow(dead_code)]

use crate::core::algorithm::{
    AlgorithmMetricsSnapshot, GenerateContext, HealthStatus, IdAlgorithm,
};
use crate::core::config::{Config, SnowflakeAlgorithmConfig};
use crate::core::types::{AlgorithmType, CoreError, Id, IdBatch, Result};
use async_trait::async_trait;
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime};
use tracing::info;

const DEFAULT_START_TIME: u64 = 1704067200000;

/// 缓存 epoch 起点（SystemTime::UNIX_EPOCH + DEFAULT_START_TIME），避免每次 checked_add
fn epoch_start() -> SystemTime {
    static EPOCH_START: OnceLock<SystemTime> = OnceLock::new();
    *EPOCH_START.get_or_init(|| {
        SystemTime::UNIX_EPOCH
            .checked_add(Duration::from_millis(DEFAULT_START_TIME))
            .expect("Invalid timestamp configuration: DEFAULT_START_TIME causes overflow")
    })
}

pub struct SnowflakeAlgorithm {
    config: SnowflakeAlgorithmConfig,
    datacenter_id: u8,
    worker_id: u8,
    sequence: AtomicU64,
    last_timestamp: AtomicU64,
    rotation_count: AtomicU8,
    metrics: Arc<SnowflakeMetrics>,
    clock_drift_ms: AtomicU64,
}

struct SnowflakeMetrics {
    total_generated: AtomicU64,
    total_failed: AtomicU64,
    clock_backwards: AtomicU64,
    sequence_overflows: AtomicU64,
}

impl SnowflakeMetrics {
    fn new() -> Self {
        Self {
            total_generated: AtomicU64::new(0),
            total_failed: AtomicU64::new(0),
            clock_backwards: AtomicU64::new(0),
            sequence_overflows: AtomicU64::new(0),
        }
    }
}

impl SnowflakeAlgorithm {
    pub fn new(datacenter_id: u8, worker_id: u8) -> Self {
        Self {
            config: SnowflakeAlgorithmConfig::default(),
            datacenter_id,
            worker_id,
            sequence: AtomicU64::new(0),
            last_timestamp: AtomicU64::new(0),
            rotation_count: AtomicU8::new(0),
            metrics: Arc::new(SnowflakeMetrics::new()),
            clock_drift_ms: AtomicU64::new(0),
        }
    }

    // L13 修复：`initialize` 从 `impl IdAlgorithm for SnowflakeAlgorithm`
    // 移到 inherent impl。原 trait method `initialize(&mut self, ...)` 让
    // trait 不那么对象安全（`Arc<dyn IdAlgorithm>` 共享后无法调用 `&mut self`）。
    // 现仅在 `AlgorithmBuilder::build` 中通过具体类型调用，初始化完成后
    // 转为 `Box<dyn IdAlgorithm>` 共享。
    pub async fn initialize(&mut self, config: &Config) -> Result<()> {
        self.config = config.algorithm.snowflake.clone();
        self.datacenter_id = config.app.dc_id;
        self.worker_id = config.app.worker_id;

        info!(
            "{}",
            t!(
                "log.core.algorithm.snowflake.initialized",
                datacenter_id = self.datacenter_id,
                worker_id = self.worker_id
            )
        );
        Ok(())
    }

    fn get_timestamp() -> u64 {
        let now = SystemTime::now()
            .duration_since(epoch_start())
            .unwrap_or(Duration::ZERO);

        now.as_millis() as u64
    }

    /// Wait for the next millisecond timestamp.
    ///
    /// L2 修复：原注释声称使用 `std::thread::sleep`，但实际代码用的是
    /// `tokio::time::sleep`（async-friendly）。注释已更新以匹配代码。
    ///
    /// 此函数仅在时钟回拨罕见场景调用，sleep duration 极短（1ms）。
    async fn wait_for_next_ms(&self, last_ts: u64) -> u64 {
        loop {
            let current = Self::get_timestamp();
            if current > last_ts {
                return current;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    }

    async fn generate_id(&self) -> Result<Id> {
        let timestamp = Self::get_timestamp();
        let last_ts = self.last_timestamp.load(Ordering::SeqCst);
        let sequence_mask = self.config.sequence_mask();

        if timestamp < last_ts {
            let drift = last_ts - timestamp;
            self.clock_drift_ms.store(drift, Ordering::Relaxed);
            self.metrics.clock_backwards.fetch_add(1, Ordering::Relaxed);

            tracing::warn!(
                event = "snowflake_clock_backward",
                current_timestamp = timestamp,
                last_timestamp = last_ts,
                drift_ms = drift,
                threshold_ms = self.config.clock_drift_threshold_ms
            );

            if drift > self.config.clock_drift_threshold_ms {
                return Err(CoreError::ClockMovedBackward {
                    last_timestamp: last_ts,
                });
            }

            let wait_ts = self.wait_for_next_ms(last_ts).await;
            return self.generate_id_with_timestamp(wait_ts, sequence_mask);
        }

        if timestamp == last_ts {
            let seq = self.sequence.fetch_add(1, Ordering::SeqCst);

            // 序列号绕回（耗尽）判定：seq > 0 且掩码后归零（说明已绕过一圈回到 0）。
            // seq=0 是合法起始值（首次 fetch_add 返回旧值 0），不得误判为耗尽。
            if seq > 0 && seq & sequence_mask == 0 {
                self.rotation_count.fetch_add(1, Ordering::Relaxed);
                let next_ts = self.wait_for_next_ms(timestamp).await;
                return self.generate_id_with_timestamp(next_ts, sequence_mask);
            }

            let id = self.construct_id(timestamp, seq & sequence_mask);
            self.metrics.total_generated.fetch_add(1, Ordering::Relaxed);
            return Ok(id);
        }

        self.sequence.store(0, Ordering::SeqCst);
        self.last_timestamp.store(timestamp, Ordering::SeqCst);

        // 新毫秒的第一个 ID 用 seq=0，但要通过 fetch_add 推进 sequence 到 1，
        // 否则下次同毫秒调用 fetch_add(1) 会返回 0，导致 ID 重复。
        let seq = self.sequence.fetch_add(1, Ordering::SeqCst);
        let id = self.construct_id(timestamp, seq & sequence_mask);
        self.metrics.total_generated.fetch_add(1, Ordering::Relaxed);
        Ok(id)
    }

    fn generate_id_with_timestamp(&self, timestamp: u64, sequence_mask: u64) -> Result<Id> {
        self.last_timestamp.store(timestamp, Ordering::SeqCst);
        self.sequence.store(0, Ordering::SeqCst);

        let seq = self.sequence.fetch_add(1, Ordering::SeqCst);

        // 序列号溢出判定：seq > 0 且掩码后归零（说明已绕过一圈回到 0）。
        // seq=0 是合法起始值（首次 fetch_add 返回旧值 0），不得误判为溢出。
        // 注：timestamp == self.last_timestamp 比较冗余（前一行刚 store），已移除。
        if seq > 0 && seq & sequence_mask == 0 {
            self.metrics
                .sequence_overflows
                .fetch_add(1, Ordering::Relaxed);

            tracing::warn!(
                event = "snowflake_sequence_overflow",
                timestamp = timestamp,
                sequence = seq,
                mask = sequence_mask
            );

            return Err(CoreError::SequenceOverflow { timestamp });
        }

        let id = self.construct_id(timestamp, seq & sequence_mask);
        self.metrics.total_generated.fetch_add(1, Ordering::Relaxed);
        Ok(id)
    }

    fn construct_id(&self, timestamp: u64, sequence: u64) -> Id {
        let dc_id = self.datacenter_id as u64;
        let worker = self.worker_id as u64;

        let id = (timestamp
            << (self.config.datacenter_id_bits
                + self.config.worker_id_bits
                + self.config.sequence_bits))
            | (dc_id << (self.config.worker_id_bits + self.config.sequence_bits))
            | (worker << self.config.sequence_bits)
            | sequence;

        Id::from_u128(id.into())
    }

    pub fn get_datacenter_id(&self) -> u8 {
        self.datacenter_id
    }

    pub fn get_worker_id(&self) -> u8 {
        self.worker_id
    }

    pub fn get_last_timestamp(&self) -> u64 {
        self.last_timestamp.load(Ordering::Relaxed)
    }

    pub fn get_sequence(&self) -> u64 {
        self.sequence.load(Ordering::Relaxed)
    }
}

#[async_trait]
impl IdAlgorithm for SnowflakeAlgorithm {
    async fn generate(&self, _ctx: &GenerateContext) -> Result<Id> {
        self.generate_id().await
    }

    async fn batch_generate(&self, _ctx: &GenerateContext, size: usize) -> Result<IdBatch> {
        let mut ids = Vec::with_capacity(size);
        let mut retries = 0;
        const MAX_RETRIES: usize = 100;

        while ids.len() < size && retries < MAX_RETRIES {
            match self.generate_id().await {
                Ok(id) => ids.push(id),
                Err(e) => {
                    tracing::debug!(
                        event = "snowflake_retry",
                        retry = retries,
                        error = %e
                    );
                    tokio::time::sleep(tokio::time::Duration::from_millis(1)).await;
                    retries += 1;
                }
            }
        }

        if ids.is_empty() {
            return Err(CoreError::InternalError(
                "Failed to generate IDs after max retries".to_string(),
            ));
        }

        Ok(IdBatch::new(ids, AlgorithmType::Snowflake, String::new()))
    }

    fn health_check(&self) -> HealthStatus {
        let drift = self.clock_drift_ms.load(Ordering::Relaxed);
        if drift > self.config.clock_drift_threshold_ms {
            return HealthStatus::Unhealthy(format!(
                "Clock drift {}ms exceeds threshold {}ms",
                drift, self.config.clock_drift_threshold_ms
            ));
        }

        HealthStatus::Healthy
    }

    fn metrics(&self) -> AlgorithmMetricsSnapshot {
        AlgorithmMetricsSnapshot {
            total_generated: self.metrics.total_generated.load(Ordering::Relaxed),
            total_failed: self.metrics.total_failed.load(Ordering::Relaxed),
            current_qps: 0,
            p50_latency_us: 0,
            p99_latency_us: 0,
            // L15 修复：Snowflake/UUID 算法无缓存概念，返回 None。
            cache_hit_rate: None,
        }
    }

    fn algorithm_type(&self) -> AlgorithmType {
        AlgorithmType::Snowflake
    }

    // L13 修复：`initialize` 已移到 inherent impl（`impl SnowflakeAlgorithm`）。

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }
}

pub struct UuidV7Algorithm {
    metrics: Arc<UuidMetrics>,
}

// L1 修复：重命名 UuidV7Metrics → UuidMetrics，
// 因为 UuidV4Algorithm 也复用此结构（命名误导）。
struct UuidMetrics {
    total_generated: AtomicU64,
    total_failed: AtomicU64,
}

impl Default for UuidMetrics {
    fn default() -> Self {
        Self {
            total_generated: AtomicU64::new(0),
            total_failed: AtomicU64::new(0),
        }
    }
}

impl Default for UuidV7Algorithm {
    fn default() -> Self {
        Self::new()
    }
}

impl UuidV7Algorithm {
    pub fn new() -> Self {
        Self {
            metrics: Arc::new(UuidMetrics::default()),
        }
    }

    pub fn generate_uuid_v7(&self) -> Result<Id> {
        let uuid = uuid::Uuid::now_v7();
        self.metrics.total_generated.fetch_add(1, Ordering::Relaxed);
        Ok(Id::from_uuid_v7(uuid))
    }
}

#[async_trait]
impl IdAlgorithm for UuidV7Algorithm {
    async fn generate(&self, _ctx: &GenerateContext) -> Result<Id> {
        self.generate_uuid_v7()
    }

    async fn batch_generate(&self, _ctx: &GenerateContext, size: usize) -> Result<IdBatch> {
        let mut ids = Vec::with_capacity(size);
        let mut last_error = None;

        for _ in 0..size {
            match self.generate_uuid_v7() {
                Ok(id) => ids.push(id),
                Err(e) => last_error = Some(e),
            }
        }

        if ids.is_empty() {
            return Err(last_error.unwrap_or(CoreError::InternalError("Unknown error".to_string())));
        }

        Ok(IdBatch::new(ids, AlgorithmType::UuidV7, String::new()))
    }

    fn health_check(&self) -> HealthStatus {
        HealthStatus::Healthy
    }

    fn metrics(&self) -> AlgorithmMetricsSnapshot {
        AlgorithmMetricsSnapshot {
            total_generated: self.metrics.total_generated.load(Ordering::Relaxed),
            total_failed: self.metrics.total_failed.load(Ordering::Relaxed),
            current_qps: 0,
            p50_latency_us: 0,
            p99_latency_us: 0,
            // L15 修复：Snowflake/UUID 算法无缓存概念，返回 None。
            cache_hit_rate: None,
        }
    }

    fn algorithm_type(&self) -> AlgorithmType {
        AlgorithmType::UuidV7
    }

    // L13 修复：删除 no-op `initialize`（trait 上已无此方法）。

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }
}

pub struct UuidV4Algorithm {
    metrics: Arc<UuidMetrics>,
}

impl Default for UuidV4Algorithm {
    fn default() -> Self {
        Self::new()
    }
}

impl UuidV4Algorithm {
    pub fn new() -> Self {
        Self {
            metrics: Arc::new(UuidMetrics::default()),
        }
    }

    pub fn generate_uuid_v4(&self) -> Result<Id> {
        let uuid = uuid::Uuid::new_v4();
        self.metrics.total_generated.fetch_add(1, Ordering::Relaxed);
        Ok(Id::from_uuid_v7(uuid))
    }
}

#[async_trait]
impl IdAlgorithm for UuidV4Algorithm {
    async fn generate(&self, _ctx: &GenerateContext) -> Result<Id> {
        self.generate_uuid_v4()
    }

    async fn batch_generate(&self, _ctx: &GenerateContext, size: usize) -> Result<IdBatch> {
        let mut ids = Vec::with_capacity(size);

        for _ in 0..size {
            ids.push(self.generate_uuid_v4()?);
        }

        Ok(IdBatch::new(ids, AlgorithmType::UuidV4, String::new()))
    }

    fn health_check(&self) -> HealthStatus {
        HealthStatus::Healthy
    }

    fn metrics(&self) -> AlgorithmMetricsSnapshot {
        AlgorithmMetricsSnapshot {
            total_generated: self.metrics.total_generated.load(Ordering::Relaxed),
            total_failed: self.metrics.total_failed.load(Ordering::Relaxed),
            current_qps: 0,
            p50_latency_us: 0,
            p99_latency_us: 0,
            // L15 修复：Snowflake/UUID 算法无缓存概念，返回 None。
            cache_hit_rate: None,
        }
    }

    fn algorithm_type(&self) -> AlgorithmType {
        AlgorithmType::UuidV4
    }

    // L13 修复：删除 no-op `initialize`（trait 上已无此方法）。

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }
}

// ============================================================================
// DI Support - Builder Pattern and with_dependencies
// ============================================================================

use confers::interface::{ConfigProvider, ConfigProviderExt};

impl SnowflakeAlgorithm {
    /// Create a new SnowflakeAlgorithm with all dependencies injected.
    ///
    /// This is the primary construction mode for full DI support.
    ///
    /// # Arguments
    ///
    /// * `config` - Configuration provider from confers
    /// * `datacenter_id` - Datacenter ID (0-7)
    /// * `worker_id` - Worker ID (0-255)
    pub fn with_dependencies(
        config: &Arc<dyn ConfigProvider>,
        datacenter_id: u8,
        worker_id: u8,
    ) -> Self {
        let snowflake_config = SnowflakeAlgorithmConfig {
            datacenter_id_bits: config
                .get_int("algorithm.snowflake.datacenter_id_bits")
                .unwrap_or(3) as u8,
            worker_id_bits: config
                .get_int("algorithm.snowflake.worker_id_bits")
                .unwrap_or(8) as u8,
            sequence_bits: config
                .get_int("algorithm.snowflake.sequence_bits")
                .unwrap_or(10) as u8,
            clock_drift_threshold_ms: config
                .get_int("algorithm.snowflake.clock_drift_threshold_ms")
                .unwrap_or(1000) as u64,
        };

        Self {
            config: snowflake_config,
            datacenter_id,
            worker_id,
            sequence: AtomicU64::new(0),
            last_timestamp: AtomicU64::new(0),
            rotation_count: AtomicU8::new(0),
            metrics: Arc::new(SnowflakeMetrics::new()),
            clock_drift_ms: AtomicU64::new(0),
        }
    }

    /// Create a new builder for SnowflakeAlgorithm.
    pub fn builder() -> SnowflakeAlgorithmBuilder {
        SnowflakeAlgorithmBuilder::new()
    }
}

/// Builder for SnowflakeAlgorithm.
#[derive(Default)]
pub struct SnowflakeAlgorithmBuilder {
    config: Option<Arc<dyn ConfigProvider>>,
    datacenter_id: Option<u8>,
    worker_id: Option<u8>,
}

impl SnowflakeAlgorithmBuilder {
    /// Create a new builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the configuration provider.
    pub fn config(mut self, config: Arc<dyn ConfigProvider>) -> Self {
        self.config = Some(config);
        self
    }

    /// Set the datacenter ID.
    pub fn datacenter_id(mut self, id: u8) -> Self {
        self.datacenter_id = Some(id);
        self
    }

    /// Set the worker ID.
    pub fn worker_id(mut self, id: u8) -> Self {
        self.worker_id = Some(id);
        self
    }

    /// Build the SnowflakeAlgorithm.
    pub fn build(self) -> SnowflakeAlgorithm {
        let datacenter_id = self.datacenter_id.unwrap_or(0);
        let worker_id = self.worker_id.unwrap_or(0);

        if let Some(config) = self.config {
            SnowflakeAlgorithm::with_dependencies(&config, datacenter_id, worker_id)
        } else {
            SnowflakeAlgorithm::new(datacenter_id, worker_id)
        }
    }
}

// ============================================================================
// ARCH-HIGH-001 修复：SnowflakeFactory impl 拆分到本文件。
// 原 impl 位于 traits.rs（违反规则 25），现移到具体类型所属文件。
// ============================================================================
#[async_trait]
impl crate::core::algorithm::AlgorithmFactory for crate::core::algorithm::SnowflakeFactory {
    async fn build(
        &self,
        _builder: &crate::core::algorithm::AlgorithmBuilder,
        config: &Config,
    ) -> Result<Box<dyn crate::core::algorithm::IdAlgorithm>> {
        let mut algo = SnowflakeAlgorithm::new(config.app.dc_id, config.app.worker_id);
        algo.initialize(config).await?;
        Ok(Box::new(algo))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use confers::types::{AnnotatedValue, ConfigValue, SourceId};
    use std::collections::HashMap;

    /// 测试用的 ConfigProvider mock，支持预设 int 值。
    struct MockConfigProvider {
        values: HashMap<String, AnnotatedValue>,
    }

    impl MockConfigProvider {
        fn new() -> Self {
            Self {
                values: HashMap::new(),
            }
        }

        fn with_int(mut self, key: impl Into<String>, value: i64) -> Self {
            let key = key.into();
            self.values.insert(
                key.clone(),
                AnnotatedValue::new(ConfigValue::I64(value), SourceId::default(), key),
            );
            self
        }
    }

    impl ConfigProvider for MockConfigProvider {
        fn get_raw(&self, key: &str) -> Option<&AnnotatedValue> {
            self.values.get(key)
        }

        fn keys(&self) -> Vec<String> {
            self.values.keys().cloned().collect()
        }
    }

    #[test]
    fn test_snowflake_config_masks() {
        let config = SnowflakeAlgorithmConfig::default();
        assert_eq!(config.datacenter_id_mask(), 0b111);
        assert_eq!(config.worker_id_mask(), 0b11111111);
        assert_eq!(config.sequence_mask(), 0b1111111111);
        assert_eq!(config.timestamp_bits(), 43);
    }

    #[test]
    fn test_snowflake_construct_id() {
        let algo = SnowflakeAlgorithm::new(1, 1);
        let id = algo.construct_id(1000, 5);
        let value = id.as_u128();

        let timestamp_bits =
            algo.config.datacenter_id_bits + algo.config.worker_id_bits + algo.config.sequence_bits;
        let worker_shift = algo.config.sequence_bits;
        let dc_shift = algo.config.worker_id_bits + algo.config.sequence_bits;

        let expected =
            (1000u128 << timestamp_bits) | (1u128 << dc_shift) | (1u128 << worker_shift) | 5u128;
        assert_eq!(value, expected);
    }

    #[tokio::test]
    async fn test_snowflake_generate() {
        let algo = SnowflakeAlgorithm::new(0, 0);
        let id = algo.generate_id().await.unwrap();
        assert!(id.as_u128() > 0);
    }

    #[tokio::test]
    async fn test_snowflake_uniqueness() {
        let algo = SnowflakeAlgorithm::new(1, 1);
        let mut ids = std::collections::HashSet::new();

        for _ in 0..100 {
            let id = algo.generate_id().await.unwrap();
            assert!(
                ids.insert(id.as_u128()),
                "Duplicate ID generated: {}",
                id.as_u128()
            );
            tokio::time::sleep(tokio::time::Duration::from_millis(1)).await;
        }
    }

    #[tokio::test]
    async fn test_uuid_v7_generate() {
        let algo = UuidV7Algorithm::new();
        let id = algo.generate_uuid_v7().unwrap();
        let uuid = id.to_uuid_v7();

        assert_eq!(uuid.get_version(), Some(uuid::Version::SortRand));
    }

    #[tokio::test]
    async fn test_uuid_v4_generate() {
        let algo = UuidV4Algorithm::new();
        let id = algo.generate_uuid_v4().unwrap();
        let uuid = id.to_uuid_v7();

        assert_eq!(uuid.get_version(), Some(uuid::Version::Random));
    }

    /// R-algorithm-001: generate_id_with_timestamp 在 seq=0（首次调用）时必须成功，
    /// 不得误判为 SequenceOverflow。
    #[test]
    fn test_generate_id_with_timestamp_first_seq_succeeds() {
        let algo = SnowflakeAlgorithm::new(0, 0);
        let sequence_mask = algo.config.sequence_mask();
        let result = algo.generate_id_with_timestamp(1000, sequence_mask);
        assert!(
            result.is_ok(),
            "first call with seq=0 should succeed, got: {:?}",
            result.err()
        );
        let id = result.unwrap();
        assert!(id.as_u128() > 0, "generated ID must be non-zero");
    }

    /// R-algorithm-001: 同一毫秒内连续两次 generate_id 调用都应成功（验证 line 140 bug 修复）。
    #[tokio::test]
    async fn test_generate_id_same_ms_twice_succeeds() {
        let algo = SnowflakeAlgorithm::new(0, 0);
        let id1 = algo.generate_id().await.expect("first call should succeed");
        // 不 sleep，确保同一毫秒内第二次调用
        let id2 = algo
            .generate_id()
            .await
            .expect("second call in same ms should succeed");
        assert_ne!(id1.as_u128(), id2.as_u128(), "IDs must be unique");
    }

    // ========================================================================
    // 时钟回拨路径
    // ========================================================================

    /// R-algorithm-002: 时钟回拨超过阈值时，generate_id 应返回 ClockMovedBackward 错误，
    /// 且 clock_drift_ms 应被记录、health_check 应反映 Unhealthy 状态。
    #[tokio::test]
    async fn test_generate_id_clock_backward_exceeds_threshold_returns_error() {
        let algo = SnowflakeAlgorithm::new(0, 0);
        let current = SnowflakeAlgorithm::get_timestamp();
        let future_ts = current + 2000;
        algo.last_timestamp.store(future_ts, Ordering::SeqCst);

        let result = algo.generate_id().await;
        match result {
            Err(CoreError::ClockMovedBackward { last_timestamp }) => {
                assert_eq!(last_timestamp, future_ts);
            }
            other => panic!("expected ClockMovedBackward, got {:?}", other),
        }

        // 验证 clock_drift_ms 已被记录为 2000
        assert_eq!(algo.clock_drift_ms.load(Ordering::Relaxed), 2000);
        // 验证 health_check 反映不健康状态
        assert!(matches!(algo.health_check(), HealthStatus::Unhealthy(_)));
    }

    /// R-algorithm-003: 时钟回拨未超过阈值时，generate_id 应等待下一毫秒并成功生成 ID，
    /// 且 last_timestamp 应推进到 wait_ts。
    #[tokio::test]
    async fn test_generate_id_clock_backward_within_threshold_waits_and_succeeds() {
        let algo = SnowflakeAlgorithm::new(0, 0);
        let current = SnowflakeAlgorithm::get_timestamp();
        // 设置 last_timestamp 为未来 1ms，drift=1 <= 默认阈值 1000
        algo.last_timestamp.store(current + 1, Ordering::SeqCst);

        let result = algo.generate_id().await;
        assert!(
            result.is_ok(),
            "should succeed after waiting, got: {:?}",
            result.err()
        );
        let id = result.unwrap();
        assert!(id.as_u128() > 0, "generated ID must be non-zero");

        // 验证 last_timestamp 已推进到 wait_ts（> current）
        let last_ts = algo.get_last_timestamp();
        assert!(
            last_ts > current,
            "last_timestamp should advance to wait_ts, got {}",
            last_ts
        );
    }

    /// R-algorithm-001: 同毫秒内序列号耗尽（seq & mask == 0 且 seq > 0）时，
    /// 应触发 rotation_count 自增并等待下一毫秒后生成新 ID。
    #[tokio::test]
    async fn test_generate_id_sequence_wraparound_triggers_rotation() {
        let algo = SnowflakeAlgorithm::new(0, 0);
        let mask = algo.config.sequence_mask();
        let rotation_before = algo.rotation_count.load(Ordering::Relaxed);

        // 重试多次以确保至少一次走绕回路径（依赖时间戳恰好等于 last_timestamp）
        // 每次尝试失败的概率 < 1%（仅在跨毫秒边界时发生），50 次后几乎必然成功
        let mut triggered = false;
        for _ in 0..50 {
            let ts = SnowflakeAlgorithm::get_timestamp();
            algo.last_timestamp.store(ts, Ordering::SeqCst);
            // 设置 sequence 为 mask+1，模拟同毫秒内已生成 mask+1 个 ID 后的状态
            algo.sequence.store(mask + 1, Ordering::SeqCst);

            if let Ok(id) = algo.generate_id().await {
                let rotation_after = algo.rotation_count.load(Ordering::Relaxed);
                if rotation_after > rotation_before {
                    assert!(id.as_u128() > 0, "generated ID must be non-zero");
                    triggered = true;
                    break;
                }
            }
        }

        assert!(
            triggered,
            "wraparound branch should trigger within 50 attempts (rotation_count should increase)"
        );
    }

    // ========================================================================
    // wait_for_next_ms
    // ========================================================================

    /// wait_for_next_ms 应返回比输入 last_ts 更大的时间戳（循环体至少执行一次）。
    #[tokio::test]
    async fn test_wait_for_next_ms_returns_timestamp_greater_than_input() {
        let algo = SnowflakeAlgorithm::new(0, 0);
        let current = SnowflakeAlgorithm::get_timestamp();
        // 输入 current + 5，确保需要等待若干毫秒才能 current > last_ts
        let result = algo.wait_for_next_ms(current + 5).await;
        assert!(
            result > current + 5,
            "wait_for_next_ms should return timestamp > input, got {}",
            result
        );
    }

    // ========================================================================
    // batch_generate
    // ========================================================================

    /// batch_generate 正常路径应返回指定数量的唯一 ID，且 algorithm 字段为 Snowflake。
    #[tokio::test]
    async fn test_batch_generate_normal_path() {
        let algo = SnowflakeAlgorithm::new(1, 1);
        let ctx = GenerateContext::default();
        let batch = algo
            .batch_generate(&ctx, 10)
            .await
            .expect("batch should succeed");
        assert_eq!(batch.ids.len(), 10);
        assert_eq!(batch.algorithm, AlgorithmType::Snowflake);

        let mut seen = std::collections::HashSet::new();
        for id in &batch.ids {
            assert!(
                seen.insert(id.as_u128()),
                "duplicate ID in batch: {}",
                id.as_u128()
            );
        }
    }

    /// batch_generate 在所有 generate_id 调用都失败时，应重试 MAX_RETRIES 次后返回 InternalError。
    #[tokio::test]
    async fn test_batch_generate_retries_exhausted_returns_internal_error() {
        let algo = SnowflakeAlgorithm::new(0, 0);
        let current = SnowflakeAlgorithm::get_timestamp();
        // 设置 last_timestamp 远在未来（drift=10000 > 阈值 1000），所有 generate_id 调用都失败
        algo.last_timestamp
            .store(current + 10_000, Ordering::SeqCst);

        let ctx = GenerateContext::default();
        let result = algo.batch_generate(&ctx, 5).await;
        match result {
            Err(CoreError::InternalError(msg)) => {
                assert!(
                    msg.contains("max retries"),
                    "error message should mention max retries, got: {}",
                    msg
                );
            }
            other => panic!("expected InternalError, got {:?}", other),
        }
    }

    // ========================================================================
    // health_check
    // ========================================================================

    /// health_check 在无时钟漂移时应返回 Healthy。
    #[test]
    fn test_health_check_healthy_when_no_drift() {
        let algo = SnowflakeAlgorithm::new(0, 0);
        assert!(matches!(algo.health_check(), HealthStatus::Healthy));
    }

    /// health_check 在 clock_drift_ms 严格大于阈值时应返回 Unhealthy。
    #[test]
    fn test_health_check_unhealthy_when_drift_exceeds_threshold() {
        let algo = SnowflakeAlgorithm::new(0, 0);
        let threshold = algo.config.clock_drift_threshold_ms;
        algo.clock_drift_ms.store(threshold + 1, Ordering::Relaxed);

        match algo.health_check() {
            HealthStatus::Unhealthy(msg) => {
                assert!(
                    msg.contains("Clock drift"),
                    "message should mention clock drift: {}",
                    msg
                );
            }
            other => panic!("expected Unhealthy, got {:?}", other),
        }
    }

    /// health_check 在 clock_drift_ms 等于阈值时应返回 Healthy（边界：drift > threshold 才 Unhealthy）。
    #[test]
    fn test_health_check_healthy_when_drift_equals_threshold() {
        let algo = SnowflakeAlgorithm::new(0, 0);
        let threshold = algo.config.clock_drift_threshold_ms;
        algo.clock_drift_ms.store(threshold, Ordering::Relaxed);
        assert!(matches!(algo.health_check(), HealthStatus::Healthy));
    }

    // ========================================================================
    // metrics / algorithm_type / initialize / shutdown
    // ========================================================================

    /// metrics 在生成 ID 后应反映正确的 total_generated 计数。
    #[tokio::test]
    async fn test_metrics_snapshot_reflects_generation() {
        let algo = SnowflakeAlgorithm::new(0, 0);
        assert_eq!(algo.metrics().total_generated, 0);

        for _ in 0..3 {
            let _ = algo.generate_id().await.unwrap();
        }

        let snap = algo.metrics();
        assert!(
            snap.total_generated >= 3,
            "total_generated should be >= 3, got {}",
            snap.total_generated
        );
        assert_eq!(snap.current_qps, 0);
        assert_eq!(snap.p50_latency_us, 0);
        assert_eq!(snap.p99_latency_us, 0);
        // L15 修复：Snowflake 无缓存，cache_hit_rate 为 None。
        assert_eq!(snap.cache_hit_rate, None);
    }

    /// algorithm_type 应返回 Snowflake。
    #[test]
    fn test_algorithm_type_returns_snowflake() {
        let algo = SnowflakeAlgorithm::new(0, 0);
        assert_eq!(algo.algorithm_type(), AlgorithmType::Snowflake);
    }

    /// initialize 应从 Config 加载 datacenter_id、worker_id 和 snowflake 配置。
    #[tokio::test]
    async fn test_initialize_updates_config_and_ids() {
        let mut algo = SnowflakeAlgorithm::new(0, 0);
        let mut config = Config::default();
        config.app.dc_id = 5;
        config.app.worker_id = 7;
        config.algorithm.snowflake.datacenter_id_bits = 2;
        config.algorithm.snowflake.worker_id_bits = 4;
        config.algorithm.snowflake.sequence_bits = 8;
        config.algorithm.snowflake.clock_drift_threshold_ms = 500;

        algo.initialize(&config)
            .await
            .expect("initialize should succeed");

        assert_eq!(algo.get_datacenter_id(), 5);
        assert_eq!(algo.get_worker_id(), 7);
        assert_eq!(algo.config.datacenter_id_bits, 2);
        assert_eq!(algo.config.worker_id_bits, 4);
        assert_eq!(algo.config.sequence_bits, 8);
        assert_eq!(algo.config.clock_drift_threshold_ms, 500);
    }

    /// shutdown 应返回 Ok(())。
    #[tokio::test]
    async fn test_shutdown_returns_ok() {
        let algo = SnowflakeAlgorithm::new(0, 0);
        assert!(algo.shutdown().await.is_ok());
    }

    // ========================================================================
    // getters
    // ========================================================================

    /// get_datacenter_id 应返回构造时设置的 datacenter_id。
    #[test]
    fn test_get_datacenter_id_returns_set_value() {
        let algo = SnowflakeAlgorithm::new(5, 7);
        assert_eq!(algo.get_datacenter_id(), 5);
    }

    /// get_worker_id 应返回构造时设置的 worker_id。
    #[test]
    fn test_get_worker_id_returns_set_value() {
        let algo = SnowflakeAlgorithm::new(5, 7);
        assert_eq!(algo.get_worker_id(), 7);
    }

    /// get_last_timestamp 应反映 last_timestamp 原子变量的当前值。
    #[test]
    fn test_get_last_timestamp_reflects_state() {
        let algo = SnowflakeAlgorithm::new(0, 0);
        assert_eq!(algo.get_last_timestamp(), 0);
        algo.last_timestamp.store(12345, Ordering::SeqCst);
        assert_eq!(algo.get_last_timestamp(), 12345);
    }

    /// get_sequence 应反映 sequence 原子变量的当前值。
    #[test]
    fn test_get_sequence_reflects_state() {
        let algo = SnowflakeAlgorithm::new(0, 0);
        assert_eq!(algo.get_sequence(), 0);
        algo.sequence.store(999, Ordering::SeqCst);
        assert_eq!(algo.get_sequence(), 999);
    }

    // ========================================================================
    // UuidV7Algorithm
    // ========================================================================

    /// UuidV7Algorithm::default() 应等价于 new()（行为验证）。
    #[test]
    fn test_uuid_v7_default_equals_new() {
        let a = UuidV7Algorithm::new();
        let b = UuidV7Algorithm::default();
        assert!(a.generate_uuid_v7().is_ok());
        assert!(b.generate_uuid_v7().is_ok());
    }

    /// UuidV7Algorithm::batch_generate 正常路径应返回指定数量的 UUID v7。
    #[tokio::test]
    async fn test_uuid_v7_batch_generate_normal() {
        let algo = UuidV7Algorithm::new();
        let ctx = GenerateContext::default();
        let batch = algo
            .batch_generate(&ctx, 5)
            .await
            .expect("batch should succeed");
        assert_eq!(batch.ids.len(), 5);
        assert_eq!(batch.algorithm, AlgorithmType::UuidV7);
        for id in &batch.ids {
            assert_eq!(id.to_uuid_v7().get_version(), Some(uuid::Version::SortRand));
        }
    }

    /// UuidV7Algorithm::batch_generate 在 size=0 时应返回 InternalError("Unknown error")。
    #[tokio::test]
    async fn test_uuid_v7_batch_generate_empty_size_returns_error() {
        let algo = UuidV7Algorithm::new();
        let ctx = GenerateContext::default();
        match algo.batch_generate(&ctx, 0).await {
            Err(CoreError::InternalError(msg)) => {
                assert!(msg.contains("Unknown error"), "got: {}", msg);
            }
            other => panic!("expected InternalError, got {:?}", other),
        }
    }

    /// UuidV7Algorithm::health_check 应返回 Healthy。
    #[test]
    fn test_uuid_v7_health_check_returns_healthy() {
        assert!(matches!(
            UuidV7Algorithm::new().health_check(),
            HealthStatus::Healthy
        ));
    }

    /// UuidV7Algorithm::metrics 应反映生成计数。
    #[tokio::test]
    async fn test_uuid_v7_metrics_snapshot() {
        let algo = UuidV7Algorithm::new();
        assert_eq!(algo.metrics().total_generated, 0);
        let _ = algo.generate_uuid_v7().unwrap();
        let _ = algo.generate_uuid_v7().unwrap();
        assert_eq!(algo.metrics().total_generated, 2);
    }

    /// UuidV7Algorithm::algorithm_type 应返回 UuidV7。
    #[test]
    fn test_uuid_v7_algorithm_type_returns_uuid_v7() {
        assert_eq!(
            UuidV7Algorithm::new().algorithm_type(),
            AlgorithmType::UuidV7
        );
    }

    /// UuidV7Algorithm::shutdown 应返回 Ok(())。
    #[tokio::test]
    async fn test_uuid_v7_shutdown_returns_ok() {
        assert!(UuidV7Algorithm::new().shutdown().await.is_ok());
    }

    // ========================================================================
    // UuidV4Algorithm
    // ========================================================================

    /// UuidV4Algorithm::default() 应等价于 new()（行为验证）。
    #[test]
    fn test_uuid_v4_default_equals_new() {
        let a = UuidV4Algorithm::new();
        let b = UuidV4Algorithm::default();
        assert!(a.generate_uuid_v4().is_ok());
        assert!(b.generate_uuid_v4().is_ok());
    }

    /// UuidV4Algorithm::batch_generate 正常路径应返回指定数量的 UUID v4。
    #[tokio::test]
    async fn test_uuid_v4_batch_generate_normal() {
        let algo = UuidV4Algorithm::new();
        let ctx = GenerateContext::default();
        let batch = algo
            .batch_generate(&ctx, 5)
            .await
            .expect("batch should succeed");
        assert_eq!(batch.ids.len(), 5);
        assert_eq!(batch.algorithm, AlgorithmType::UuidV4);
        for id in &batch.ids {
            assert_eq!(id.to_uuid_v7().get_version(), Some(uuid::Version::Random));
        }
    }

    /// UuidV4Algorithm::health_check 应返回 Healthy。
    #[test]
    fn test_uuid_v4_health_check_returns_healthy() {
        assert!(matches!(
            UuidV4Algorithm::new().health_check(),
            HealthStatus::Healthy
        ));
    }

    /// UuidV4Algorithm::metrics 应反映生成计数。
    #[tokio::test]
    async fn test_uuid_v4_metrics_snapshot() {
        let algo = UuidV4Algorithm::new();
        assert_eq!(algo.metrics().total_generated, 0);
        let _ = algo.generate_uuid_v4().unwrap();
        let _ = algo.generate_uuid_v4().unwrap();
        assert_eq!(algo.metrics().total_generated, 2);
    }

    /// UuidV4Algorithm::algorithm_type 应返回 UuidV4。
    #[test]
    fn test_uuid_v4_algorithm_type_returns_uuid_v4() {
        assert_eq!(
            UuidV4Algorithm::new().algorithm_type(),
            AlgorithmType::UuidV4
        );
    }

    /// UuidV4Algorithm::shutdown 应返回 Ok(())。
    #[tokio::test]
    async fn test_uuid_v4_shutdown_returns_ok() {
        assert!(UuidV4Algorithm::new().shutdown().await.is_ok());
    }

    // ========================================================================
    // DI: with_dependencies
    // ========================================================================

    /// with_dependencies 应从 ConfigProvider 加载所有 snowflake 配置项。
    #[test]
    fn test_with_dependencies_loads_config_from_provider() {
        let provider = MockConfigProvider::new()
            .with_int("algorithm.snowflake.datacenter_id_bits", 2)
            .with_int("algorithm.snowflake.worker_id_bits", 4)
            .with_int("algorithm.snowflake.sequence_bits", 8)
            .with_int("algorithm.snowflake.clock_drift_threshold_ms", 500);
        let provider_arc: Arc<dyn ConfigProvider> = Arc::new(provider);

        let algo = SnowflakeAlgorithm::with_dependencies(&provider_arc, 3, 5);

        assert_eq!(algo.get_datacenter_id(), 3);
        assert_eq!(algo.get_worker_id(), 5);
        assert_eq!(algo.config.datacenter_id_bits, 2);
        assert_eq!(algo.config.worker_id_bits, 4);
        assert_eq!(algo.config.sequence_bits, 8);
        assert_eq!(algo.config.clock_drift_threshold_ms, 500);
    }

    /// with_dependencies 在 provider 缺少键时应使用默认值（3, 8, 10, 1000）。
    #[test]
    fn test_with_dependencies_uses_defaults_when_provider_empty() {
        let provider_arc: Arc<dyn ConfigProvider> = Arc::new(MockConfigProvider::new());

        let algo = SnowflakeAlgorithm::with_dependencies(&provider_arc, 1, 2);

        assert_eq!(algo.get_datacenter_id(), 1);
        assert_eq!(algo.get_worker_id(), 2);
        assert_eq!(algo.config.datacenter_id_bits, 3);
        assert_eq!(algo.config.worker_id_bits, 8);
        assert_eq!(algo.config.sequence_bits, 10);
        assert_eq!(algo.config.clock_drift_threshold_ms, 1000);
    }

    // ========================================================================
    // Builder
    // ========================================================================

    /// SnowflakeAlgorithmBuilder::new() 应创建空 builder（所有字段为 None）。
    #[test]
    fn test_builder_new_creates_empty_builder() {
        let builder = SnowflakeAlgorithmBuilder::new();
        assert!(builder.config.is_none());
        assert!(builder.datacenter_id.is_none());
        assert!(builder.worker_id.is_none());
    }

    /// SnowflakeAlgorithmBuilder::default() 应等价于 new()。
    #[test]
    fn test_builder_default_equals_new() {
        let a = SnowflakeAlgorithmBuilder::new();
        let b = SnowflakeAlgorithmBuilder::default();
        assert!(a.config.is_none() && b.config.is_none());
        assert!(a.datacenter_id.is_none() && b.datacenter_id.is_none());
        assert!(a.worker_id.is_none() && b.worker_id.is_none());
    }

    /// Builder::build() 在无 config 时应使用 SnowflakeAlgorithm::new 构造（默认配置）。
    #[test]
    fn test_builder_build_without_config_uses_new_constructor() {
        let algo = SnowflakeAlgorithmBuilder::new()
            .datacenter_id(5)
            .worker_id(7)
            .build();

        assert_eq!(algo.get_datacenter_id(), 5);
        assert_eq!(algo.get_worker_id(), 7);
        // 默认 SnowflakeAlgorithmConfig
        assert_eq!(algo.config.datacenter_id_bits, 3);
        assert_eq!(algo.config.worker_id_bits, 8);
        assert_eq!(algo.config.sequence_bits, 10);
        assert_eq!(algo.config.clock_drift_threshold_ms, 1000);
    }

    /// Builder::build() 在有 config 时应使用 with_dependencies 构造（从 provider 加载配置）。
    #[test]
    fn test_builder_build_with_config_uses_with_dependencies() {
        let provider = Arc::new(
            MockConfigProvider::new()
                .with_int("algorithm.snowflake.datacenter_id_bits", 2)
                .with_int("algorithm.snowflake.worker_id_bits", 4)
                .with_int("algorithm.snowflake.sequence_bits", 8)
                .with_int("algorithm.snowflake.clock_drift_threshold_ms", 500),
        );

        let algo = SnowflakeAlgorithmBuilder::new()
            .config(provider)
            .datacenter_id(3)
            .worker_id(5)
            .build();

        assert_eq!(algo.get_datacenter_id(), 3);
        assert_eq!(algo.get_worker_id(), 5);
        assert_eq!(algo.config.datacenter_id_bits, 2);
        assert_eq!(algo.config.worker_id_bits, 4);
        assert_eq!(algo.config.sequence_bits, 8);
        assert_eq!(algo.config.clock_drift_threshold_ms, 500);
    }

    /// Builder::build() 在未设置 dc_id/worker_id 时应默认为 0。
    #[test]
    fn test_builder_build_defaults_to_zero_ids() {
        let algo = SnowflakeAlgorithmBuilder::new().build();
        assert_eq!(algo.get_datacenter_id(), 0);
        assert_eq!(algo.get_worker_id(), 0);
    }

    /// Builder 链式 setter 应正确设置所有字段。
    #[test]
    fn test_builder_chained_setters_work() {
        let provider: Arc<dyn ConfigProvider> = Arc::new(MockConfigProvider::new());
        let builder = SnowflakeAlgorithmBuilder::new()
            .config(provider.clone())
            .datacenter_id(11)
            .worker_id(22);
        assert!(builder.config.is_some());
        assert_eq!(builder.datacenter_id, Some(11));
        assert_eq!(builder.worker_id, Some(22));
        // 验证 provider Arc 引用计数正确（clone 后仍指向同一对象）
        assert_eq!(Arc::strong_count(&provider), 2);
    }

    /// Builder 生成的算法应可正常生成 ID（端到端验证）。
    #[tokio::test]
    async fn test_builder_built_algorithm_can_generate_id() {
        let algo = SnowflakeAlgorithmBuilder::new()
            .datacenter_id(1)
            .worker_id(2)
            .build();

        let id = algo.generate_id().await.expect("generate should succeed");
        assert!(id.as_u128() > 0);
    }

    /// SnowflakeAlgorithm::builder() 应等价于 SnowflakeAlgorithmBuilder::new()。
    #[test]
    fn test_snowflake_builder_method_creates_empty_builder() {
        let builder = SnowflakeAlgorithm::builder();
        assert!(builder.config.is_none());
        assert!(builder.datacenter_id.is_none());
        assert!(builder.worker_id.is_none());
    }

    /// SnowflakeAlgorithm::builder() 构造的算法应可正常工作。
    #[tokio::test]
    async fn test_snowflake_builder_method_builds_working_algorithm() {
        let algo = SnowflakeAlgorithm::builder()
            .datacenter_id(2)
            .worker_id(3)
            .build();
        assert_eq!(algo.get_datacenter_id(), 2);
        assert_eq!(algo.get_worker_id(), 3);

        let ctx = GenerateContext::default();
        let id = algo
            .generate(&ctx)
            .await
            .expect("generate via trait should succeed");
        assert!(id.as_u128() > 0);
    }

    // ========================================================================
    // trait generate 方法覆盖（UuidV7 / UuidV4）
    // ========================================================================

    /// UuidV7Algorithm 通过 IdAlgorithm::generate trait 方法应正常生成 UUID v7。
    #[tokio::test]
    async fn test_uuid_v7_generate_via_trait() {
        let algo = UuidV7Algorithm::new();
        let ctx = GenerateContext::default();
        let id = algo
            .generate(&ctx)
            .await
            .expect("generate via trait should succeed");
        assert_eq!(id.to_uuid_v7().get_version(), Some(uuid::Version::SortRand));
    }

    /// UuidV4Algorithm 通过 IdAlgorithm::generate trait 方法应正常生成 UUID v4。
    #[tokio::test]
    async fn test_uuid_v4_generate_via_trait() {
        let algo = UuidV4Algorithm::new();
        let ctx = GenerateContext::default();
        let id = algo
            .generate(&ctx)
            .await
            .expect("generate via trait should succeed");
        assert_eq!(id.to_uuid_v7().get_version(), Some(uuid::Version::Random));
    }

    /// SnowflakeAlgorithm 通过 IdAlgorithm::generate trait 方法应正常生成 ID。
    #[tokio::test]
    async fn test_snowflake_generate_via_trait() {
        let algo = SnowflakeAlgorithm::new(1, 1);
        let ctx = GenerateContext::default();
        let id = algo
            .generate(&ctx)
            .await
            .expect("generate via trait should succeed");
        assert!(id.as_u128() > 0);
    }
}
