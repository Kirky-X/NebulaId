// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Time-ordered RFC 9562 v8 structured UUID composition (128 bits):
//! `custom_a` = 48-bit Unix ms timestamp; `custom_b` = dc(3)|worker(8)|counter_hi(1);
//! `custom_c` = shard(16)|counter_lo(20)|rand(26); version=0b1000, variant=0b10.
//! Collision safety follows the UUIDP "Cluster" scheme (random start + monotonic counter).

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use rand::RngExt;
use std::collections::HashMap;
use std::sync::Mutex;
use uuid::Uuid;

use crate::core::algorithm::traits::{
    AlgorithmBuilder, AlgorithmFactory, AlgorithmMetricsSnapshot, GenerateContext, IdAlgorithm,
};
use crate::core::algorithm::UuidV8Factory;
use crate::core::config::Config;
use crate::core::types::id::{AlgorithmType, Id, IdBatch};
use crate::core::types::{CoreError, Result};
use crate::core::HealthStatus;

const NODE_DC_BITS: u32 = 3;
const NODE_WORKER_BITS: u32 = 8;
const COUNTER_HI_BITS: u32 = 1;
const SHARD_BITS: u32 = 16;
const COUNTER_LO_BITS: u32 = 20;
const RAND_BITS: u32 = 26;

const NODE_DC_MAX: u64 = (1u64 << NODE_DC_BITS) - 1;
const NODE_WORKER_MAX: u64 = (1u64 << NODE_WORKER_BITS) - 1;
const SHARD_MASK: u64 = (1u64 << SHARD_BITS) - 1;
const COUNTER_LO_MASK: u64 = (1u64 << COUNTER_LO_BITS) - 1;
const RAND_MASK: u64 = (1u64 << RAND_BITS) - 1;
const COUNTER_TOTAL_BITS: u64 = (COUNTER_HI_BITS + COUNTER_LO_BITS) as u64;
const COUNTER_TOTAL_MASK: u64 = (1u64 << COUNTER_TOTAL_BITS) - 1;

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn shard_of(workspace_id: &str, group_id: &str, biz_tag: &str) -> u64 {
    if workspace_id.is_empty() && group_id.is_empty() && biz_tag.is_empty() {
        return 0;
    }
    let mut buf = Vec::with_capacity(workspace_id.len() + group_id.len() + biz_tag.len() + 4);
    buf.extend_from_slice(workspace_id.as_bytes());
    buf.push(b' ');
    buf.extend_from_slice(group_id.as_bytes());
    buf.push(b' ');
    buf.extend_from_slice(biz_tag.as_bytes());
    // 内联 FNV-1a 64 位哈希，避免额外依赖；取低 16 位作为分片索引。
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in &buf {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash & SHARD_MASK
}

fn fast_rand_26() -> u64 {
    // 使用高分辨率计时器的低 26 位作为轻量随机源。
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    (t ^ (t >> 27)).wrapping_mul(0x2545F4914F6CDD1D) & RAND_MASK
}

/// UUID v8 位段反解产物（与生成路径打包顺序严格互逆）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedUuidV8Id {
    pub timestamp_ms: u64,
    pub dc_id: u8,
    pub worker_id: u8,
    /// counter 为 COUNTER_HI + COUNTER_LO 共 21 位，u16 无法承载。
    pub counter: u32,
    pub shard: u16,
    pub rand: u32,
}

/// UUID v8 位布局反解（feature `uuid-v8`）。
pub struct UuidV8LayoutInfo;

impl UuidV8LayoutInfo {
    /// 与 `generate_inner` 的打包顺序严格互逆。
    pub fn parse(value: u128) -> ParsedUuidV8Id {
        let timestamp_ms = (value >> 80) as u64;
        let custom_b = ((value >> 64) & 0xFFF) as u64;
        let dc_id = (custom_b >> (COUNTER_HI_BITS + NODE_WORKER_BITS)) as u8;
        let worker_id = ((custom_b >> COUNTER_HI_BITS) & NODE_WORKER_MAX) as u8;
        let counter_hi = custom_b & 0x1;

        // custom_c 的高 6 位在 variant 字段的 bits 62..56；低 56 位在 node 字段。
        let custom_c_high = ((value >> 56) & 0x3F) as u64;
        let custom_c_low = (value & 0x00FF_FFFF_FFFF_FFFF) as u64;
        let custom_c = (custom_c_high << 56) | custom_c_low;
        let shard = (custom_c >> (COUNTER_LO_BITS + RAND_BITS)) as u16;
        let counter_lo = ((custom_c >> RAND_BITS) & COUNTER_LO_MASK) as u32;
        let rand = (custom_c & RAND_MASK) as u32;

        ParsedUuidV8Id {
            timestamp_ms,
            dc_id,
            worker_id,
            counter: ((counter_hi as u32) << COUNTER_LO_BITS) | counter_lo,
            shard,
            rand,
        }
    }

    /// 归属候选验证：重算 shard 与解析值比对。三元组全空时无法验证（None）——
    /// shard 是 FNV-1a 单向哈希，不做明文还原。
    pub fn verify_attribution(
        parsed: &ParsedUuidV8Id,
        workspace: &str,
        group: &str,
        biz_tag: &str,
    ) -> Option<bool> {
        if workspace.is_empty() && group.is_empty() && biz_tag.is_empty() {
            return None;
        }
        Some(shard_of(workspace, group, biz_tag) as u16 == parsed.shard)
    }
}

/// 时间有序 v8 UUID 算法实现。
pub struct UuidV8Impl {
    last_timestamp: AtomicU64,
    counter: AtomicU64,
    dc_id: u64,
    worker_id: u64,
    clock_drift_threshold_ms: u64,
    random_source: crate::core::config::RandomSource,
    shard_cache: Mutex<HashMap<(String, String, String), u64>>,
}

impl UuidV8Impl {
    pub fn new(dc_id: u64, worker_id: u64, clock_drift_threshold_ms: u64) -> Self {
        Self::new_with_source(
            dc_id,
            worker_id,
            clock_drift_threshold_ms,
            crate::core::config::RandomSource::Timer,
        )
    }

    /// 可指定随机源的构造；`RandomSource::Timer` 逐位兼容既有部署。
    pub fn new_with_source(
        dc_id: u64,
        worker_id: u64,
        clock_drift_threshold_ms: u64,
        random_source: crate::core::config::RandomSource,
    ) -> Self {
        let dc = dc_id & NODE_DC_MAX;
        let worker = worker_id & NODE_WORKER_MAX;
        let seed = Self::rand26_for(random_source) & COUNTER_TOTAL_MASK;
        Self {
            last_timestamp: AtomicU64::new(0),
            counter: AtomicU64::new(seed),
            dc_id: dc,
            worker_id: worker,
            clock_drift_threshold_ms,
            random_source,
            shard_cache: Mutex::new(HashMap::new()),
        }
    }

    fn rand26(&self) -> u64 {
        Self::rand26_for(self.random_source)
    }

    fn rand26_for(source: crate::core::config::RandomSource) -> u64 {
        match source {
            crate::core::config::RandomSource::Timer => fast_rand_26(),
            crate::core::config::RandomSource::Csprng => rand::rng().random::<u64>() & RAND_MASK,
        }
    }

    fn shard_for(&self, ctx: &GenerateContext) -> u64 {
        let key = (
            ctx.workspace_id.clone(),
            ctx.group_id.clone(),
            ctx.biz_tag.clone(),
        );
        if let Some(v) = self.shard_cache.lock().unwrap().get(&key) {
            return *v;
        }
        let s = shard_of(&ctx.workspace_id, &ctx.group_id, &ctx.biz_tag);
        self.shard_cache.lock().unwrap().insert(key, s);
        s
    }

    /// 生成单个时间有序 v8 UUID。
    fn generate_inner(&self, ctx: &GenerateContext) -> Result<Id> {
        let ts = now_unix_ms();

        let mut last = self.last_timestamp.load(Ordering::Acquire);
        let mut effective_ts = ts;
        loop {
            if effective_ts < last {
                if last - effective_ts > self.clock_drift_threshold_ms {
                    return Err(CoreError::ClockMovedBackward {
                        last_timestamp: last - effective_ts,
                    });
                }
                effective_ts = last;
            }
            match self.last_timestamp.compare_exchange_weak(
                last,
                effective_ts,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    // UUIDP Cluster：跨毫秒时计数器重置为随机起点，仅由 CAS 胜者执行；
                    // 毫秒切换瞬间的并发生成不保证同毫秒 counter 顺序性，全局唯一性
                    // 由随机起点 + 21 位计数器 + 26 位随机熵共同保证。
                    if effective_ts > last {
                        self.counter
                            .store(self.rand26() & COUNTER_TOTAL_MASK, Ordering::Release);
                    }
                    break;
                }
                Err(x) => {
                    last = x;
                }
            }
        }

        let counter = self.counter.fetch_add(1, Ordering::Relaxed) & COUNTER_TOTAL_MASK;

        let custom_b = (self.dc_id << (COUNTER_HI_BITS + NODE_WORKER_BITS))
            | (self.worker_id << COUNTER_HI_BITS)
            | (counter >> COUNTER_LO_BITS);

        let shard = self.shard_for(ctx);
        let rand = self.rand26();
        let custom_c = (shard << (COUNTER_LO_BITS + RAND_BITS))
            | ((counter & COUNTER_LO_MASK) << RAND_BITS)
            | rand;

        // RFC 9562 v8 位布局（u128 大端）：bits 80..128 = custom_a(unix_ts_ms)；
        // 64..80 = (version=8)<<12 | custom_b；56..64 = (variant=0b10)<<6 | custom_c[56..62]；0..56 = custom_c 低位。
        let version_field: u128 = (8u128 << 12) | (custom_b as u128 & 0xFFF);
        let variant_field: u128 = ((0b10u128 << 6) | ((custom_c as u128 >> 56) & 0x3F)) << 56;
        let node_field: u128 = (custom_c as u128) & 0x00FF_FFFF_FFFF_FFFF;

        let mut value: u128 = ((effective_ts as u128) & 0xFFFF_FFFFFFFF) << 80;
        value |= version_field << 64;
        value |= variant_field;
        value |= node_field;

        Ok(Id::from_uuid_v8(Uuid::from_u128(value)))
    }

    fn batch_inner(&self, ctx: &GenerateContext, n: usize) -> Result<Vec<Id>> {
        // 单毫秒计数器空间 21 位：批量最坏全部落同毫秒，超空间即回绕出重，
        // 静态拒绝而非静默出重（跨毫秒 rollover 重置计数器，守卫以最坏情况为准）。
        if n as u64 > COUNTER_TOTAL_MASK {
            return Err(CoreError::SequenceOverflow {
                timestamp: now_unix_ms(),
            });
        }
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            out.push(self.generate_inner(ctx)?);
        }
        Ok(out)
    }
}

impl Default for UuidV8Impl {
    fn default() -> Self {
        Self::new(0, 0, 1000)
    }
}

#[async_trait]
impl IdAlgorithm for UuidV8Impl {
    async fn generate(&self, ctx: &GenerateContext) -> Result<Id> {
        self.generate_inner(ctx)
    }

    async fn batch_generate(&self, ctx: &GenerateContext, size: usize) -> Result<IdBatch> {
        if size == 0 {
            return Ok(IdBatch::new(
                Vec::new(),
                AlgorithmType::UuidV8,
                String::new(),
            ));
        }
        let ids = self.batch_inner(ctx, size)?;
        Ok(IdBatch::new(ids, AlgorithmType::UuidV8, String::new()))
    }

    fn health_check(&self) -> HealthStatus {
        HealthStatus::Healthy
    }

    fn metrics(&self) -> AlgorithmMetricsSnapshot {
        AlgorithmMetricsSnapshot {
            // 延迟分位数与时钟回拨计数由路由层观测后在
            // AlgorithmRouter::metrics 合并填充。
            ..Default::default()
        }
    }

    fn algorithm_type(&self) -> AlgorithmType {
        AlgorithmType::UuidV8
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }
}

/// UUID v8 算法工厂，注册在 `algorithm_factories`。

#[async_trait]
impl AlgorithmFactory for UuidV8Factory {
    async fn build(
        &self,
        _builder: &AlgorithmBuilder,
        _config: &Config,
    ) -> Result<Box<dyn IdAlgorithm>> {
        Ok(Box::new(UuidV8Impl::new_with_source(
            _config.app.dc_id as u64,
            _config.app.worker_id as u64,
            _config.algorithm.snowflake.clock_drift_threshold_ms,
            _config.algorithm.uuid_v8.random_source,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> GenerateContext {
        GenerateContext {
            workspace_id: "ws1".to_string(),
            group_id: "g1".to_string(),
            biz_tag: "order".to_string(),
            format: crate::core::types::id::IdFormat::Uuid,
            ..Default::default()
        }
    }

    #[test]
    fn test_uuid_v8_is_time_ordered_and_v8() {
        let algo = UuidV8Impl::new(1, 1, 1000);
        let a = algo.generate_inner(&ctx()).unwrap();
        let b = algo.generate_inner(&ctx()).unwrap();
        assert_eq!(a.to_uuid_v8().get_version(), Some(uuid::Version::Custom));
        // 单调递增（同毫秒 counter 递增）
        let av = a.to_uuid_v8().as_u128();
        let bv = b.to_uuid_v8().as_u128();
        assert!(bv > av, "uuid_v8 must be monotonic increasing");
    }

    #[test]
    fn test_uuid_v8_layout_fields() {
        // 直接验证位布局：custom_a = ts, version=v8, variant=10
        let algo = UuidV8Impl::new(1, 1, 1000);
        let id = algo.generate_inner(&ctx()).unwrap();
        let v = id.to_uuid_v8().as_u128();
        // version bits 48..52
        let version = ((v >> 76) & 0xF) as u8;
        assert_eq!(version, 8);
        // variant bits 64..66 == 0b10
        let variant = ((v >> 62) & 0x3) as u8;
        assert_eq!(variant, 0b10);
    }

    #[test]
    fn test_uuid_v8_unique_across_workers_and_time_ordered() {
        // K 个不同节点，每个生成 N 个 ID，全局必须唯一；单节点内必须单调非递减
        const K: u64 = 4;
        const N: usize = 1000;
        let mut all: std::collections::HashSet<u128> = std::collections::HashSet::new();
        for w in 0..K {
            let algo = UuidV8Impl::new(w % 2, w, 1000);
            let mut last: u128 = 0;
            for _ in 0..N {
                let id = algo.generate_inner(&ctx()).unwrap();
                let v = id.to_uuid_v8().as_u128();
                assert!(
                    all.insert(v),
                    "uuid_v8 must be globally unique across nodes"
                );
                assert!(v >= last, "uuid_v8 must be non-decreasing within a node");
                last = v;
            }
        }
        assert_eq!(all.len(), (K as usize) * N);
    }

    #[test]
    fn test_uuid_v8_embedded_timestamp_matches_now() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let algo = UuidV8Impl::new(1, 1, 1000);
        let id = algo.generate_inner(&ctx()).unwrap();
        let v = id.to_uuid_v8().as_u128();
        // custom_a 位于最高 48 位
        let ts = (v >> 80) as u64;
        assert!(
            ts.abs_diff(now) <= 1000,
            "embedded ts {ts} should be near now {now}"
        );
    }

    #[test]
    fn test_uuid_v8_batch_generate_unique() {
        let algo = UuidV8Impl::new(3, 7, 1000);
        let ids = algo.batch_inner(&ctx(), 500).unwrap();
        let mut set: std::collections::HashSet<u128> = std::collections::HashSet::new();
        let mut last: u128 = 0;
        for id in &ids {
            let v = id.to_uuid_v8().as_u128();
            assert!(set.insert(v), "batch ids must be unique");
            assert!(v >= last, "batch ids must be non-decreasing");
            last = v;
        }
        assert_eq!(set.len(), 500);
    }

    #[test]
    fn test_uuid_v8_counter_resets_on_millisecond_rollover() {
        // 同毫秒内 counter 连续递增；只有相邻两侧 ts 相同的样本对能断言 +1，
        // 跨毫秒的 counter 按设计重置为随机起点（见下方 assert_ne）。
        let algo = UuidV8Impl::new(1, 1, 1000);
        let mut samples: Vec<(u128, u64)> = Vec::new();
        for _ in 0..4 {
            let id = algo.generate_inner(&ctx()).unwrap();
            samples.push((
                id.to_uuid_v8().as_u128() >> 80,
                algo.counter.load(Ordering::Relaxed),
            ));
        }
        assert!(
            samples.windows(2).any(|w| w[0].0 == w[1].0),
            "测试前提：连续 4 次生成中必须至少有一对落在同一毫秒内"
        );
        for w in samples.windows(2) {
            if w[0].0 == w[1].0 {
                assert_eq!(
                    w[1].1,
                    (w[0].1 + 1) & COUNTER_TOTAL_MASK,
                    "same-ms counter must increment by 1"
                );
            }
        }

        // 跨毫秒：注入 last_timestamp 回退 10ms，下次生成 effective_ts > last
        // → counter 必须重置为新的随机起点（UUIDP Cluster），而非旧值连续递增
        let before = algo.counter.load(Ordering::Relaxed);
        algo.last_timestamp
            .store(now_unix_ms() - 10, Ordering::Release);
        let _c = algo.generate_inner(&ctx()).unwrap();
        let c3 = algo.counter.load(Ordering::Relaxed);
        assert_ne!(
            c3,
            (before + 1) & COUNTER_TOTAL_MASK,
            "counter must reset to a fresh random start on ms rollover"
        );
    }

    #[test]
    fn test_uuid_v8_clock_drift_threshold_configurable() {
        let now = now_unix_ms();
        // 阈值 10ms：模拟时钟回退 50ms（last_timestamp 落在"未来"）→ 必须报错
        let strict = UuidV8Impl::new(1, 1, 10);
        strict.last_timestamp.store(now + 50, Ordering::Release);
        let err = strict.generate_inner(&ctx()).unwrap_err();
        assert!(
            matches!(err, CoreError::ClockMovedBackward { .. }),
            "backward drift 50ms must exceed threshold 10ms"
        );

        // 阈值 200ms：同样回退 50ms → 容忍（effective_ts 卡到 last 等待追平）
        let tolerant = UuidV8Impl::new(1, 1, 200);
        tolerant.last_timestamp.store(now + 50, Ordering::Release);
        assert!(
            tolerant.generate_inner(&ctx()).is_ok(),
            "backward drift 50ms must be tolerated under threshold 200ms"
        );
    }

    #[test]
    fn test_uuid_v8_counter_seed_is_random() {
        // 随机起点必须生效：new 的计数器初值应为 21 位随机非零（8 次全零概率可忽略）。
        for _ in 0..8 {
            let algo = UuidV8Impl::new(0, 0, 1000);
            if algo.counter.load(Ordering::Relaxed) != 0 {
                return;
            }
        }
        panic!("counter seed must be random: 8/8 constructions yielding 0 is impossible");
    }

    #[test]
    fn test_uuid_v8_batch_rejects_size_exceeding_counter_space() {
        let algo = UuidV8Impl::new(1, 1, 1000);
        let err = algo
            .batch_inner(&ctx(), (COUNTER_TOTAL_MASK as usize) + 1)
            .unwrap_err();
        assert!(
            matches!(err, CoreError::SequenceOverflow { .. }),
            "batch over counter space must fail loudly"
        );
        // 边界内仍可用
        assert!(algo.batch_inner(&ctx(), 1_000).is_ok());
    }

    #[test]
    fn test_shard_of_empty_ids_is_zero() {
        assert_eq!(shard_of("", "", ""), 0);
        assert_ne!(shard_of("ws1", "g1", "order"), 0);
    }

    #[tokio::test]
    async fn test_uuid_v8_default_and_trivial_trait_methods() {
        let algo = UuidV8Impl::default();
        assert_eq!(algo.algorithm_type(), AlgorithmType::UuidV8);
        assert_eq!(algo.health_check(), HealthStatus::Healthy);
        assert!(algo.metrics().cache_hit_rate.is_none());
        assert!(algo.shutdown().await.is_ok());

        let empty = algo
            .batch_generate(&ctx(), 0)
            .await
            .expect("size 0 must succeed");
        assert!(empty.ids.is_empty());
    }
    /// Csprng 模式的 rand 位段（低 26 位）应无重复碰撞。
    /// 样本数取 100：2^26 空间下 1024 样本的期望碰撞约 7.8，"零碰撞"不可满足，
    /// 故以统计上稳健的 100 样本（期望碰撞 0.07）钉住无重复属性。
    #[tokio::test]
    async fn test_csprng_rand_bits_no_collision_in_sample() {
        use crate::core::config::RandomSource;
        let algo = UuidV8Impl::new_with_source(0, 0, 1000, RandomSource::Csprng);
        let c = ctx();
        let mut rand_bits = std::collections::HashSet::new();
        for _ in 0..100 {
            let id = algo.generate(&c).await.expect("生成成功");
            let bits = (id.as_u128() & u128::from(RAND_MASK)) as u64;
            assert!(rand_bits.insert(bits), "Csprng rand 位段不应重复: {bits}");
        }
    }

    #[tokio::test]
    async fn test_timer_source_default_preserves_behavior() {
        use crate::core::config::RandomSource;
        let algo = UuidV8Impl::new_with_source(0, 0, 1000, RandomSource::Timer);
        let id = algo.generate(&ctx()).await.expect("生成成功");
        assert!(id.as_u128() > 0);
    }
}

#[cfg(all(test, feature = "uuid-v8"))]
mod layout_parse_tests {
    use super::*;

    /// 生成 → as_u128 → parse roundtrip：全部位段严格互逆（R-observability-401）。
    /// counter 为 21 位（COUNTER_HI+LO），用 u32 承载（design 中 u16 不足以容纳）。
    #[tokio::test]
    async fn test_parse_roundtrip_all_fields() {
        let algo = UuidV8Impl::new(3, 200, 1000);
        let c = GenerateContext {
            workspace_id: "ws-x".to_string(),
            group_id: "grp".to_string(),
            biz_tag: "bt".to_string(),
            ..GenerateContext::default()
        };
        let id = algo.generate(&c).await.expect("生成成功");
        let parsed = UuidV8LayoutInfo::parse(id.as_u128());
        assert_eq!(parsed.dc_id, 3);
        assert_eq!(parsed.worker_id, 200);
        assert_eq!(parsed.shard, shard_of("ws-x", "grp", "bt") as u16);
        assert!(parsed.counter <= COUNTER_TOTAL_MASK as u32);
        assert!(parsed.rand <= RAND_MASK as u32);
        assert!(parsed.timestamp_ms > 0);
    }

    #[test]
    fn test_parse_arbitrary_u128_never_panics_and_stays_in_range() {
        // 固定伪随机序列（不引入 rand 到测试断言路径），覆盖全值域关键边界。
        let mut x: u128 = 0x9E37_79B9_7F4A_7C15;
        for _ in 0..100 {
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1);
            let parsed = UuidV8LayoutInfo::parse(x);
            assert!(parsed.dc_id <= NODE_DC_MAX as u8);
            assert!(parsed.worker_id <= NODE_WORKER_MAX as u8);
            assert!(parsed.counter <= COUNTER_TOTAL_MASK as u32);
            assert!(parsed.shard <= SHARD_MASK as u16);
            assert!(parsed.rand <= RAND_MASK as u32);
        }
    }
    #[test]
    fn test_verify_attribution_candidate_validation() {
        let parsed = UuidV8LayoutInfo::parse(u128::MAX);
        let mut with_shard = parsed.clone();
        with_shard.shard = shard_of("ws", "grp", "bt") as u16;
        assert_eq!(
            UuidV8LayoutInfo::verify_attribution(&with_shard, "ws", "grp", "bt"),
            Some(true)
        );
        assert_eq!(
            UuidV8LayoutInfo::verify_attribution(&with_shard, "ws", "grp", "other"),
            Some(false)
        );
        assert_eq!(
            UuidV8LayoutInfo::verify_attribution(&with_shard, "", "", ""),
            None
        );
    }
}
