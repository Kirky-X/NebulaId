// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

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
use std::time::{Duration, Instant, SystemTime};
use tracing::info;

const DEFAULT_START_TIME: u64 = 1704067200000;

/// clock drift 衰减阈值：距最近一次回拨事件持续无新事件达到该毫秒数后，
/// `clock_drift_ms` 衰减清零、health_check 恢复 Healthy。
const DEFAULT_DRIFT_DECAY_AFTER_MS: u64 = 60_000;

/// 进程级单调锚点：`monotonic_millis()` 返回自首次调用起的毫秒数（恒 >= 1，
/// 0 保留为 last_drift_at_ms 的「尚无事件」哨兵）。用于给 drift 事件打时间戳，
/// 不受系统时钟回拨影响（回拨场景下 `SystemTime` 不可靠）。
fn monotonic_millis() -> u64 {
    static ANCHOR: OnceLock<SystemTime> = OnceLock::new();
    let anchor = ANCHOR.get_or_init(SystemTime::now);
    anchor.elapsed().unwrap_or(Duration::ZERO).as_millis() as u64 + 1
}

/// 缓存 epoch 起点（SystemTime::UNIX_EPOCH + DEFAULT_START_TIME），避免每次 checked_add
fn epoch_start() -> SystemTime {
    static EPOCH_START: OnceLock<SystemTime> = OnceLock::new();
    *EPOCH_START.get_or_init(|| {
        SystemTime::UNIX_EPOCH
            .checked_add(Duration::from_millis(DEFAULT_START_TIME))
            .expect("Invalid timestamp configuration: DEFAULT_START_TIME causes overflow")
    })
}

/// Snowflake 位布局元数据（解析知识的唯一权威来源）。
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
    /// 生成状态单原子：`(last_timestamp_ms << sequence_bits) | sequence`。
    ///
    /// 全部生成路径（单条 / 批量）都通过对该字的 CAS 迁移推进，取代历史
    /// `gen_lock: tokio::sync::Mutex` 的全局串行——旧锁把所有生成串行化，
    /// 且临界区跨越 `wait_for_next_ms` 的 `.await`；CAS 方案下「新毫秒复位」
    /// 与「同毫秒递增」的互斥由状态字本身的原子性保证，生成不再互相阻塞。
    /// worker_id / datacenter_id 实例内恒定，不参与 CAS 状态。
    ///
    /// 序列号域为 `[0, sequence_mask]`，`sequence_mask` 本身保留为耗尽哨兵
    /// 不发号：`seq + take` 必须落在 `[0, mask]` 内，否则原子加会溢出污染
    /// 高位时间戳。故单毫秒可发号容量为 `sequence_mask`（10 位 → 1023 个）。
    state: AtomicU64,
    rotation_count: AtomicU8,
    metrics: Arc<SnowflakeMetrics>,
    clock_drift_ms: AtomicU64,
    /// 最近一次时钟回拨事件的单调时刻（`monotonic_millis()` 毫秒；0 = 尚无事件）。
    /// 用于 drift 衰减判定：持续 `drift_decay_after_ms` 无新事件则清零漂移。
    last_drift_at_ms: AtomicU64,
    /// drift 衰减阈值（毫秒）。默认 60_000；测试可改小以模拟时间推进。
    drift_decay_after_ms: u64,
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
            state: AtomicU64::new(0),
            rotation_count: AtomicU8::new(0),
            metrics: Arc::new(SnowflakeMetrics::new()),
            clock_drift_ms: AtomicU64::new(0),
            last_drift_at_ms: AtomicU64::new(0),
            drift_decay_after_ms: DEFAULT_DRIFT_DECAY_AFTER_MS,
        }
    }

    /// drift 衰减：距最近一次回拨事件超过 `drift_decay_after_ms` 且期间无新
    /// 事件时，清零 `clock_drift_ms`，使 health_check 恢复 Healthy。
    ///
    /// 在每次进入生成路径时检查（成功与回拨失败共用入口）：若本次调用又发生
    /// 回拨，回拨分支会刷新 `last_drift_at_ms` 并重新记录漂移，衰减不会吞掉
    /// 持续回拨的健康告警。
    fn maybe_decay_drift(&self) {
        let drift = self.clock_drift_ms.load(Ordering::Relaxed);
        if drift == 0 {
            return;
        }
        let now = monotonic_millis();
        let last = self.last_drift_at_ms.load(Ordering::Relaxed);
        if now.saturating_sub(last) >= self.drift_decay_after_ms {
            self.clock_drift_ms.store(0, Ordering::Relaxed);
            info!(
                event = "snowflake_clock_drift_decayed",
                previous_drift_ms = drift,
                quiet_ms = now.saturating_sub(last)
            );
        }
    }

    // 修复：`initialize` 从 `impl IdAlgorithm for SnowflakeAlgorithm`
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
    /// 混合等待：先自旋（`spin_loop`）至多约 2ms——毫秒轮转的典型等待就是
    /// 亚毫秒级（同毫秒序列耗尽到下一毫秒边界），1ms 周期的 tokio sleep 有
    /// 定时器粒度过冲（实测把持续吞吐压到理论上限的一半左右，见基准
    /// `snowflake/batch_generate_10000`），自旋可把轮转等待收紧到真实边界；
    /// 预算耗尽仍未越过 `last_ts`（时钟回拨等待场景，可达阈值级毫秒数）才
    /// 退回 1ms 周期 sleep，避免长等待空转烧核。
    async fn wait_for_next_ms(&self, last_ts: u64) -> u64 {
        const SPIN_BUDGET: Duration = Duration::from_millis(2);
        let spin_deadline = Instant::now() + SPIN_BUDGET;
        loop {
            let current = Self::get_timestamp();
            if current > last_ts {
                return current;
            }
            if Instant::now() < spin_deadline {
                std::hint::spin_loop();
            } else {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        }
    }

    /// 打包生成状态：`(timestamp_ms << sequence_bits) | sequence`。
    fn pack_state(&self, timestamp_ms: u64, sequence: u64) -> u64 {
        (timestamp_ms << self.config.sequence_bits) | (sequence & self.config.sequence_mask())
    }

    /// 从状态字解出时间戳分量（高位）。
    fn state_timestamp(&self, state: u64) -> u64 {
        state >> self.config.sequence_bits
    }

    /// 从状态字解出序列号分量（低位）。
    fn state_sequence(&self, state: u64) -> u64 {
        state & self.config.sequence_mask()
    }

    /// 时钟回拨公共处理（单条 / 批量共用）：记录 drift、刷新衰减时钟基准
    /// 计数并输出告警日志。
    fn record_clock_backward(&self, current_timestamp: u64, last_timestamp: u64, drift: u64) {
        self.clock_drift_ms.store(drift, Ordering::Relaxed);
        self.last_drift_at_ms
            .store(monotonic_millis(), Ordering::Relaxed);
        self.metrics.clock_backwards.fetch_add(1, Ordering::Relaxed);

        tracing::warn!(
            event = "snowflake_clock_backward",
            current_timestamp = current_timestamp,
            last_timestamp = last_timestamp,
            drift_ms = drift,
            threshold_ms = self.config.clock_drift_threshold_ms
        );
    }

    /// 无锁生成核心：在 [`Self::state`] 上以 CAS 预留 `count` 个连续序列号，
    /// 返回 `(timestamp_ms, start_seq, count)`；调用方据此构造
    /// `[start_seq, start_seq + count)` 的 ID 区间。单条生成即 `count = 1`。
    ///
    /// 状态迁移（全部为互斥的 CAS 成功路径，保证不重不漏）：
    /// - 新毫秒（`timestamp > last_ts`）：CAS 复位 `seq → take`，从 0 起预留
    ///   `min(count, sequence_mask)` 个；
    /// - 同毫秒：CAS `state → state + take`（`fetch_add(n)` 区间预留的 CAS
    ///   形式——先按已读状态校验剩余容量再做原子加，防止 seq 越界污染高位
    ///   时间戳）；
    /// - 同毫秒剩余不足（`take == 0`）：等待真实时钟越过当前毫秒后 CAS 轮转
    ///   `(ts, seq) → (next_ts, 0)` 并重试；
    /// - 时钟回拨：记录 drift（衰减语义不变）；超阈值返回
    ///   [`CoreError::ClockMovedBackward`]；阈值内等待时钟追平后 CAS 把
    ///   `last_ts` 推进到 `wait_ts`（seq 归零）并重试。
    ///
    /// CAS 失败仅说明他线程已推进状态，循环重读即可，无锁且无饥饿。
    async fn reserve(&self, count: u64) -> Result<(u64, u64, u64)> {
        let seq_mask = self.config.sequence_mask();

        // sequence_bits=0（mask=0）时单毫秒可发号容量为 0，直接报溢出，
        // 避免 CAS 循环在零容量状态下空转。
        if seq_mask == 0 {
            return Err(CoreError::SequenceOverflow {
                timestamp: Self::get_timestamp(),
            });
        }

        // drift 衰减检查：持续 60 秒无新回拨事件后清零漂移，
        // health_check 据此恢复 Healthy。
        self.maybe_decay_drift();

        loop {
            let state = self.state.load(Ordering::Acquire);
            let last_ts = self.state_timestamp(state);
            let seq = self.state_sequence(state);
            let timestamp = Self::get_timestamp();

            if timestamp < last_ts {
                let drift = last_ts - timestamp;
                self.record_clock_backward(timestamp, last_ts, drift);

                if drift > self.config.clock_drift_threshold_ms {
                    return Err(CoreError::ClockMovedBackward {
                        last_timestamp: last_ts,
                    });
                }

                // 阈值内：等时钟追平后 CAS 把 last_ts 推进到 wait_ts（seq 归零）。
                // CAS 失败说明他线程已推进状态，直接重读重试。
                let wait_ts = self.wait_for_next_ms(last_ts).await;
                let _ = self.state.compare_exchange(
                    state,
                    self.pack_state(wait_ts, 0),
                    Ordering::AcqRel,
                    Ordering::Acquire,
                );
                continue;
            }

            if timestamp > last_ts {
                // 新毫秒：从 seq 0 起预留 take 个（take <= mask 保证可编码）。
                let take = count.min(seq_mask);
                let new_state = self.pack_state(timestamp, take);
                if self
                    .state
                    .compare_exchange(state, new_state, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
                {
                    return Ok((timestamp, 0, take));
                }
                continue;
            }

            // 同毫秒：剩余 = mask - seq（seq=mask 保留为耗尽哨兵不入号）。
            let take = count.min(seq_mask - seq);
            if take > 0 {
                if self
                    .state
                    .compare_exchange(state, state + take, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
                {
                    return Ok((timestamp, seq, take));
                }
                continue;
            }

            // 同毫秒序列耗尽：推进到下一毫秒重试（seq 归零）。
            let next_ts = self.wait_for_next_ms(timestamp).await;
            if self
                .state
                .compare_exchange(
                    state,
                    self.pack_state(next_ts, 0),
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok()
            {
                self.rotation_count.fetch_add(1, Ordering::Relaxed);
                self.metrics
                    .sequence_overflows
                    .fetch_add(1, Ordering::Relaxed);
                tracing::debug!(
                    event = "snowflake_sequence_exhausted",
                    timestamp = timestamp,
                    next_timestamp = next_ts
                );
            }
        }
    }

    /// 单条生成：预留 1 个序列号并构造 ID。
    async fn generate_id(&self) -> Result<Id> {
        let (timestamp, sequence, _) = self.reserve(1).await?;
        self.metrics.total_generated.fetch_add(1, Ordering::Relaxed);
        Ok(self.construct_id(timestamp, sequence))
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

        // 区间预留：每次向 reserve 申请「还差的个数」，一次 CAS 预留一整段
        // 连续序列号。同毫秒剩余不足时 reserve 内部推进到下一毫秒重试；
        // 时钟回拨超阈值返回 Err（回拨路径的 drift 记录与 衰减不受影响）。
        while ids.len() < size && retries < MAX_RETRIES {
            match self.reserve((size - ids.len()) as u64).await {
                Ok((timestamp, start_seq, count)) => {
                    for seq in start_seq..start_seq + count {
                        ids.push(self.construct_id(timestamp, seq));
                    }
                    self.metrics
                        .total_generated
                        .fetch_add(count, Ordering::Relaxed);
                }
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

        // 重试耗尽仍未凑满请求量（含一例未成的情况）：
        // 显式报错并附上已生成/请求数量，不再静默返回短批次——
        // 静默短批会让调用方误以为拿到了全部请求的 ID。
        if ids.len() < size {
            return Err(CoreError::InternalError(format!(
                "Failed to generate IDs after max retries (generated {}/requested {})",
                ids.len(),
                size
            )));
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
            // 修复：Snowflake/UUID 算法无缓存概念，返回 None。
            cache_hit_rate: None,
            // 延迟分位数与时钟回拨计数由路由层观测后在
            // AlgorithmRouter::metrics() 合并填充。
            ..Default::default()
        }
    }

    fn algorithm_type(&self) -> AlgorithmType {
        AlgorithmType::Snowflake
    }

    // 修复：`initialize` 已移到 inherent impl（`impl SnowflakeAlgorithm`）。

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }
}

// ============================================================================
// SnowflakeFactory impl 拆分到本文件。
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

    /// 状态字打包/解包互为逆运算（位结构钉子）。
    #[test]
    fn test_state_pack_unpack_roundtrip() {
        let algo = SnowflakeAlgorithm::new(1, 1);
        let (ts, seq) = (0x000F_4240u64, 0x123u64);
        let state = algo.pack_state(ts, seq);
        assert_eq!(algo.state_timestamp(state), ts);
        assert_eq!(algo.state_sequence(state), seq);
    }

    /// CAS 化后首颗 ID 必须以 seq=0 起步且成功
    ///（钉住历史上「seq=0 被误判为 SequenceOverflow」的回归）。
    #[tokio::test]
    async fn test_generate_first_id_starts_at_sequence_zero() {
        let algo = SnowflakeAlgorithm::new(0, 0);
        let id = algo
            .generate_id()
            .await
            .expect("first generate must succeed");
        assert!(id.as_u128() > 0, "generated ID must be non-zero");

        let layout = SnowflakeLayoutInfo::from_config(&algo.config);
        let parsed = layout.parse(id.as_u128());
        assert_eq!(parsed.sequence, 0, "first ID must use sequence 0");
    }

    /// 单线程连续生成：时间戳单调不减；同毫秒内序列严格 +1；跨毫秒后序列归零。
    /// 生成量超过两个单毫秒容量（1023 x 2），强制跨越至少两次毫秒边界。
    #[tokio::test]
    async fn test_snowflake_single_thread_sequence_continuity() {
        let algo = SnowflakeAlgorithm::new(1, 1);
        let layout = SnowflakeLayoutInfo::from_config(&algo.config);
        let mut prev: Option<ParsedSnowflakeId> = None;

        for _ in 0..2500 {
            let id = algo.generate_id().await.unwrap();
            let parsed = layout.parse(id.as_u128());
            if let Some(p) = prev {
                if parsed.timestamp_ms == p.timestamp_ms {
                    assert_eq!(
                        parsed.sequence,
                        p.sequence + 1,
                        "sequences must increment by 1 within the same millisecond"
                    );
                } else {
                    assert!(
                        parsed.timestamp_ms > p.timestamp_ms,
                        "timestamp must be monotonically increasing"
                    );
                    assert_eq!(
                        parsed.sequence, 0,
                        "sequence must reset to 0 on new millisecond"
                    );
                }
            }
            prev = Some(parsed);
        }
    }

    /// 16 任务并发生成：总集合不得出现重复 ID（无锁 CAS 正确性钉子）。
    /// 8000 个 ID 跨越多个毫秒边界，覆盖同毫秒递增、序列耗尽轮转与新毫秒
    /// 复位的并发交织。
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn test_snowflake_concurrent_generation_no_duplicates() {
        let algo = Arc::new(SnowflakeAlgorithm::new(1, 1));
        const TASKS: usize = 16;
        const PER_TASK: usize = 500;

        let handles: Vec<_> = (0..TASKS)
            .map(|_| {
                let algo = Arc::clone(&algo);
                tokio::spawn(async move {
                    let mut ids = Vec::with_capacity(PER_TASK);
                    for _ in 0..PER_TASK {
                        ids.push(algo.generate_id().await.unwrap().as_u128());
                    }
                    ids
                })
            })
            .collect();

        let mut all = std::collections::HashSet::new();
        let mut total = 0usize;
        for handle in handles {
            for id in handle.await.unwrap() {
                assert!(
                    all.insert(id),
                    "concurrent generation produced duplicate ID: {id}"
                );
                total += 1;
            }
        }
        assert_eq!(total, TASKS * PER_TASK);
    }

    /// 同一毫秒内连续两次 generate_id 调用都应成功（验证 line 140 bug 修复）。
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

    /// 时钟回拨超过阈值时，generate_id 应返回 ClockMovedBackward 错误，
    /// 且 clock_drift_ms 应被记录、health_check 应反映 Unhealthy 状态。
    #[tokio::test]
    async fn test_generate_id_clock_backward_exceeds_threshold_returns_error() {
        let algo = SnowflakeAlgorithm::new(0, 0);
        let current = SnowflakeAlgorithm::get_timestamp();
        let future_ts = current + 2000;
        algo.state
            .store(algo.pack_state(future_ts, 0), Ordering::SeqCst);

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

    /// 时钟回拨未超过阈值时，generate_id 应等待下一毫秒并成功生成 ID，
    /// 且 last_timestamp 应推进到 wait_ts。
    #[tokio::test]
    async fn test_generate_id_clock_backward_within_threshold_waits_and_succeeds() {
        let algo = SnowflakeAlgorithm::new(0, 0);
        let current = SnowflakeAlgorithm::get_timestamp();
        // 设置状态字时间戳为未来 1ms，drift=1 <= 默认阈值 1000
        algo.state
            .store(algo.pack_state(current + 1, 0), Ordering::SeqCst);

        let result = algo.generate_id().await;
        assert!(
            result.is_ok(),
            "should succeed after waiting, got: {:?}",
            result.err()
        );
        let id = result.unwrap();
        assert!(id.as_u128() > 0, "generated ID must be non-zero");

        // 验证状态字时间戳已推进到 wait_ts（> current）
        let last_ts = algo.state_timestamp(algo.state.load(Ordering::SeqCst));
        assert!(
            last_ts > current,
            "last_timestamp should advance to wait_ts, got {}",
            last_ts
        );
    }

    /// 同毫秒内序列号耗尽（seq & mask == 0 且 seq > 0）时，
    /// 应触发 rotation_count 自增并等待下一毫秒后生成新 ID。
    #[tokio::test]
    async fn test_generate_id_sequence_wraparound_triggers_rotation() {
        let algo = SnowflakeAlgorithm::new(0, 0);
        let mask = algo.config.sequence_mask();
        let rotation_before = algo.rotation_count.load(Ordering::Relaxed);

        // 重试多次以确保至少一次走绕回路径（依赖时间戳恰好等于状态字时间戳）
        // 每次尝试失败的概率 < 1%（仅在跨毫秒边界时发生），50 次后几乎必然成功
        let mut triggered = false;
        for _ in 0..50 {
            let ts = SnowflakeAlgorithm::get_timestamp();
            // CAS 布局下单毫秒 seq 合法域为 [0, mask]：置 seq=mask 即
            // 「本毫秒序列耗尽（哨兵位）」，下一次生成必须走轮转分支
            // （rotation_count 自增）并推进到下一毫秒发号。
            algo.state
                .store(algo.pack_state(ts, mask), Ordering::SeqCst);

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

    /// batch_generate 在所有 generate_id 调用都失败时，应重试 MAX_RETRIES 次
    /// 后返回 InternalError，且消息显式包含「已生成 0/请求 5」两个数量。
    #[tokio::test]
    async fn test_batch_generate_retries_exhausted_returns_internal_error() {
        let algo = SnowflakeAlgorithm::new(0, 0);
        let current = SnowflakeAlgorithm::get_timestamp();
        // 设置状态字时间戳远在未来（drift=10000 > 阈值 1000），所有 reserve 调用都失败
        algo.state
            .store(algo.pack_state(current + 10_000, 0), Ordering::SeqCst);

        let ctx = GenerateContext::default();
        let result = algo.batch_generate(&ctx, 5).await;
        match result {
            Err(CoreError::InternalError(msg)) => {
                assert!(
                    msg.contains("max retries"),
                    "error message should mention max retries, got: {}",
                    msg
                );
                assert!(
                    msg.contains("generated 0/requested 5"),
                    "error message must contain both generated and requested counts, got: {}",
                    msg
                );
            }
            other => panic!("expected InternalError, got {:?}", other),
        }
    }

    /// 正常路径 batch 数量精确不变：凑满请求量时不得误报短批错误。
    #[tokio::test]
    async fn test_batch_generate_exact_size_is_not_treated_as_short_batch() {
        let algo = SnowflakeAlgorithm::new(0, 0);
        let ctx = GenerateContext::default();
        let batch = algo
            .batch_generate(&ctx, 3)
            .await
            .expect("batch should succeed");
        assert_eq!(batch.ids.len(), 3);
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
    // clock drift 衰减
    // ========================================================================

    /// 回拨事件必须同时记录 last_drift_at_ms（衰减判定的时钟基准）。
    #[tokio::test]
    async fn test_backward_event_records_last_drift_at() {
        let algo = SnowflakeAlgorithm::new(0, 0);
        assert_eq!(algo.last_drift_at_ms.load(Ordering::Relaxed), 0);

        let current = SnowflakeAlgorithm::get_timestamp();
        algo.state
            .store(algo.pack_state(current + 2000, 0), Ordering::SeqCst);
        assert!(algo.generate_id().await.is_err());

        assert!(
            algo.last_drift_at_ms.load(Ordering::Relaxed) > 0,
            "backward event must refresh last_drift_at_ms"
        );
    }

    /// 模拟时间推进（衰减阈值内）：drift 保持、health 维持 Unhealthy；
    /// 超过衰减阈值后：drift 清零且 health 恢复 Healthy。
    #[tokio::test]
    async fn test_drift_decays_after_quiet_period_restores_health() {
        let mut algo = SnowflakeAlgorithm::new(0, 0);
        // 阈值注入：0 = 时间一推进（下一次生成路径检查）即衰减。
        algo.drift_decay_after_ms = 0;

        // 注入一次已记录的回拨事件（真实回拨会这样落盘）。
        algo.clock_drift_ms.store(2000, Ordering::Relaxed);
        algo.last_drift_at_ms
            .store(monotonic_millis(), Ordering::Relaxed);
        assert!(matches!(algo.health_check(), HealthStatus::Unhealthy(_)));

        // 阈值内的保持路径：换一把阈值无穷大的实例，生成不得清零 drift。
        let mut held = SnowflakeAlgorithm::new(0, 0);
        held.drift_decay_after_ms = u64::MAX;
        held.clock_drift_ms.store(2000, Ordering::Relaxed);
        held.last_drift_at_ms
            .store(monotonic_millis(), Ordering::Relaxed);
        let id = held.generate_id().await.unwrap();
        assert!(id.as_u128() > 0);
        assert_eq!(
            held.clock_drift_ms.load(Ordering::Relaxed),
            2000,
            "衰减阈值内 drift 不得被清零"
        );
        assert!(matches!(held.health_check(), HealthStatus::Unhealthy(_)));

        // 超时路径：下一次生成成功后 drift 清零、health 恢复 Healthy。
        let id = algo.generate_id().await.unwrap();
        assert!(id.as_u128() > 0);
        assert_eq!(
            algo.clock_drift_ms.load(Ordering::Relaxed),
            0,
            "超过衰减阈值后 drift 必须清零"
        );
        assert!(matches!(algo.health_check(), HealthStatus::Healthy));
    }

    /// 持续回拨会刷新 last_drift_at_ms：即使衰减阈值已到，新事件后重新计时，
    /// drift 不被清零（衰减不能吞掉正在发生的回拨告警）。
    #[tokio::test]
    async fn test_fresh_backward_event_blocks_decay() {
        let mut algo = SnowflakeAlgorithm::new(0, 0);
        // 阈值 0：若非新事件刷新时间戳，进入生成路径的衰减检查会立即清零。
        algo.drift_decay_after_ms = 0;
        // 预置一次「很久以前」的旧事件。
        algo.clock_drift_ms.store(2000, Ordering::Relaxed);
        algo.last_drift_at_ms
            .store(monotonic_millis().saturating_sub(60_000), Ordering::Relaxed);

        // 本次调用发生真实回拨（状态字时间戳在未来 2000ms）：
        // 回拨分支刷新 last_drift_at_ms 并重新记录 drift。
        let current = SnowflakeAlgorithm::get_timestamp();
        algo.state
            .store(algo.pack_state(current + 2000, 0), Ordering::SeqCst);
        assert!(algo.generate_id().await.is_err());

        assert_eq!(
            algo.clock_drift_ms.load(Ordering::Relaxed),
            2000,
            "新回拨事件后 drift 必须保持记录（重新计时）"
        );
        assert!(matches!(algo.health_check(), HealthStatus::Unhealthy(_)));
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
        // 修复：Snowflake 无缓存，cache_hit_rate 为 None。
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

    #[test]
    fn test_layout_info_from_config_matches_defaults() {
        let layout = SnowflakeLayoutInfo::from_config(&SnowflakeAlgorithmConfig::default());
        // 默认 3 / 8 / 10 位宽，时间戳占剩余位：64 - 3 - 8 - 10 = 43
        assert_eq!(layout.datacenter_id_bits, 3);
        assert_eq!(layout.worker_id_bits, 8);
        assert_eq!(layout.sequence_bits, 10);
        assert_eq!(layout.timestamp_bits, 43);
    }
}
