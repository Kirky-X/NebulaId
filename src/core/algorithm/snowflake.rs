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

//! Snowflake ID generation algorithm.
//!
//! Hosts the production [`SnowflakeAlgorithm`] and its [`IdAlgorithm`]
//! implementation. UUID-style generation lives in the dedicated
//! `uuid_v8.rs` module; the previously test-only UUID generators and
//! DI builder that lived here were removed as dead code.

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

/// Snowflake 位布局元数据（T010：解析知识的唯一权威来源）。
///
/// 此前 server 层 `id_handlers::extract_snowflake_metadata` 手工硬编码
/// 10/8/3 位宽做解码，与配置驱动的生成侧（[`SnowflakeAlgorithmConfig`]）
/// 存在漂移风险；现收敛为本类型，handler 仅调用 [`Self::parse`]。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnowflakeLayoutInfo {
    pub timestamp_bits: u8,
    pub worker_id_bits: u8,
    pub datacenter_id_bits: u8,
    pub sequence_bits: u8,
}

/// 从原始 u128 值解析出的 Snowflake ID 分量。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParsedSnowflakeId {
    /// 自 epoch（DEFAULT_START_TIME）以来的毫秒数
    pub timestamp_ms: u64,
    pub datacenter_id: u8,
    pub worker_id: u16,
    pub sequence: u16,
}

impl SnowflakeLayoutInfo {
    /// 从算法配置推导位布局（与 `construct_id` 的移位顺序互为逆运算）。
    pub fn from_config(cfg: &SnowflakeAlgorithmConfig) -> Self {
        Self {
            timestamp_bits: cfg.timestamp_bits(),
            worker_id_bits: cfg.worker_id_bits,
            datacenter_id_bits: cfg.datacenter_id_bits,
            sequence_bits: cfg.sequence_bits,
        }
    }

    /// 默认位布局（datacenter=3 / worker=8 / sequence=10），供无法获取
    /// 运行时配置的调用方（如 mock generator）回退使用——与历史硬编码
    /// 行为完全一致。
    pub fn standard() -> Self {
        Self {
            timestamp_bits: 64 - 3 - 8 - 10,
            worker_id_bits: 8,
            datacenter_id_bits: 3,
            sequence_bits: 10,
        }
    }

    /// 按本布局解析原始值。字段语义与 [`SnowflakeAlgorithm::construct_id`]
    /// 的组装顺序严格互逆：
    /// `timestamp << (dc+worker+seq) | dc << (worker+seq) | worker << seq | seq`
    pub fn parse(&self, value: u128) -> ParsedSnowflakeId {
        let seq_mask: u128 = (1u128 << self.sequence_bits) - 1;
        let worker_mask: u128 = (1u128 << self.worker_id_bits) - 1;
        let dc_mask: u128 = (1u128 << self.datacenter_id_bits) - 1;

        let worker_shift = self.sequence_bits;
        let dc_shift = self.sequence_bits + self.worker_id_bits;
        let ts_shift = dc_shift + self.datacenter_id_bits;

        ParsedSnowflakeId {
            sequence: (value & seq_mask) as u16,
            worker_id: ((value >> worker_shift) & worker_mask) as u16,
            datacenter_id: ((value >> dc_shift) & dc_mask) as u8,
            timestamp_ms: (value >> ts_shift) as u64,
        }
    }
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
    /// 串行化 `(last_timestamp, sequence)` 状态迁移（修复并发重复 ID 竞态）。
    /// 「新毫秒复位」与「同毫秒递增」必须互斥，否则两线程可同时复位
    /// sequence 并领取相同 seq，产生重复 ID。临界区可能跨越
    /// `wait_for_next_ms` 的 `.await`，故使用可安全跨 await 持有的
    /// `tokio::sync::Mutex`（而非 parking_lot）。
    gen_lock: tokio::sync::Mutex<()>,
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
            gen_lock: tokio::sync::Mutex::new(()),
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
        // 串行化 (last_timestamp, sequence) 状态迁移：若不加锁，两个线程可同时
        // 观察到 timestamp > last_ts，各自复位 sequence 并领取相同 seq，产生重复
        // ID。锁内包含 wait_for_next_ms 的 .await（tokio Mutex 可安全跨 await）。
        let _guard = self.gen_lock.lock().await;

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
}

#[async_trait]
impl IdAlgorithm for SnowflakeAlgorithm {
    async fn generate(&self, _ctx: &GenerateContext) -> Result<Id> {
        self.generate_id().await
    }

    async fn batch_generate(&self, _ctx: &GenerateContext, size: usize) -> Result<IdBatch> {
        // size=0 边界：直接返回空批次，不进入重试循环
        // （否则 `ids.is_empty()` 判定会误报 "Failed to generate IDs"）
        if size == 0 {
            return Ok(IdBatch::new(
                Vec::new(),
                AlgorithmType::Snowflake,
                String::new(),
            ));
        }

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
            // L15 修复：Snowflake/UUID 算法无缓存概念，返回 None。
            cache_hit_rate: None,
            // 延迟分位数与时钟回拨计数由路由层观测后在
            // AlgorithmRouter::metrics() 合并填充（T021）。
            ..Default::default()
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
        let last_ts = algo.last_timestamp.load(Ordering::SeqCst);
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
