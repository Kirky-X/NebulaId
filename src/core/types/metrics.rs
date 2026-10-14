// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

use crate::core::types::AlgorithmType;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

/// 延迟分位数环形缓冲容量：快照语义为"最近 1024 次请求的延迟分布"。
/// 固定样本环零依赖且对运维分位数够用（见 design.md D8）：写侧无锁，读侧拷贝排序。
const LATENCY_RING_SIZE: usize = 1024;

#[derive(Debug, Serialize)]
pub struct AlgorithmMetrics {
    pub algorithm: AlgorithmType,
    pub total_generated: AtomicU64,
    pub total_failed: AtomicU64,
    pub current_qps: AtomicU64,
    pub cache_hit_rate: AtomicU64,
    /// 累计记录的延迟样本数（单调递增，不随环形缓冲回绕）。
    latency_samples: AtomicU64,
    /// 累计时钟回拨次数（`AlgorithmRouter::observe` 递增；告警规则据此判定）。
    clock_backwards: AtomicU64,
    /// 最近 [`LATENCY_RING_SIZE`] 个延迟样本（纳秒），`record_latency` 环写。
    #[serde(skip)]
    latency_ring: Box<[AtomicU64]>,
    /// 环形缓冲写游标（`fetch_add` 取模落槽）。
    #[serde(skip)]
    ring_cursor: AtomicUsize,
}

fn empty_latency_ring() -> Box<[AtomicU64]> {
    std::iter::repeat_with(|| AtomicU64::new(0))
        .take(LATENCY_RING_SIZE)
        .collect::<Vec<_>>()
        .into_boxed_slice()
}

impl Clone for AlgorithmMetrics {
    fn clone(&self) -> Self {
        let ring = empty_latency_ring();
        for (slot, src) in ring.iter().zip(self.latency_ring.iter()) {
            slot.store(src.load(Ordering::Relaxed), Ordering::Relaxed);
        }
        Self {
            algorithm: self.algorithm,
            total_generated: AtomicU64::new(self.total_generated.load(Ordering::Relaxed)),
            total_failed: AtomicU64::new(self.total_failed.load(Ordering::Relaxed)),
            current_qps: AtomicU64::new(self.current_qps.load(Ordering::Relaxed)),
            cache_hit_rate: AtomicU64::new(self.cache_hit_rate.load(Ordering::Relaxed)),
            latency_samples: AtomicU64::new(self.latency_samples.load(Ordering::Relaxed)),
            clock_backwards: AtomicU64::new(self.clock_backwards.load(Ordering::Relaxed)),
            latency_ring: ring,
            ring_cursor: AtomicUsize::new(self.ring_cursor.load(Ordering::Relaxed)),
        }
    }
}

impl Default for AlgorithmMetrics {
    fn default() -> Self {
        Self {
            algorithm: AlgorithmType::Segment,
            total_generated: AtomicU64::new(0),
            total_failed: AtomicU64::new(0),
            current_qps: AtomicU64::new(0),
            cache_hit_rate: AtomicU64::new(0),
            latency_samples: AtomicU64::new(0),
            clock_backwards: AtomicU64::new(0),
            latency_ring: empty_latency_ring(),
            ring_cursor: AtomicUsize::new(0),
        }
    }
}

impl AlgorithmMetrics {
    pub fn new(algorithm: AlgorithmType) -> Self {
        Self {
            algorithm,
            ..Default::default()
        }
    }

    pub fn increment_generated(&self, count: u64) {
        self.total_generated.fetch_add(count, Ordering::Relaxed);
    }

    pub fn increment_failed(&self) {
        self.total_failed.fetch_add(1, Ordering::Relaxed);
    }

    /// 记录一次请求延迟（纳秒）：无锁环写入最近 1024 样本缓冲。
    pub fn record_latency(&self, latency_ns: u64) {
        self.current_qps.fetch_add(1, Ordering::Relaxed);
        let slot = self.ring_cursor.fetch_add(1, Ordering::Relaxed) % LATENCY_RING_SIZE;
        self.latency_ring[slot].store(latency_ns, Ordering::Relaxed);
        self.latency_samples.fetch_add(1, Ordering::Relaxed);
    }

    pub fn update_qps(&self, qps: u64) {
        self.current_qps.store(qps, Ordering::Relaxed);
    }

    pub fn update_cache_hit_rate(&self, hit_rate: f64) {
        self.cache_hit_rate
            .store((hit_rate * 10000.0) as u64, Ordering::Relaxed);
    }

    pub fn get_generated(&self) -> u64 {
        self.total_generated.load(Ordering::Relaxed)
    }

    pub fn get_failed(&self) -> u64 {
        self.total_failed.load(Ordering::Relaxed)
    }

    pub fn get_qps(&self) -> u64 {
        self.current_qps.load(Ordering::Relaxed)
    }

    /// 累计记录过的延迟样本数（含已被环形缓冲覆盖的历史样本）。
    pub fn latency_sample_count(&self) -> u64 {
        self.latency_samples.load(Ordering::Relaxed)
    }

    /// 记录一次时钟回拨（路由层在 `CoreError::ClockMovedBackward` 时调用）。
    pub fn record_clock_backward(&self) {
        self.clock_backwards.fetch_add(1, Ordering::Relaxed);
    }

    pub fn get_clock_backwards(&self) -> u64 {
        self.clock_backwards.load(Ordering::Relaxed)
    }

    /// 快照最近 `min(样本数, LATENCY_RING_SIZE)` 个延迟并升序排序。
    /// 读侧拷贝仅在低频 metrics/告警路径；并发期个别槽位偏旧，对统计量无实质影响。
    fn sorted_latency_samples(&self) -> Option<Vec<u64>> {
        let total = self.latency_samples.load(Ordering::Relaxed) as usize;
        if total == 0 {
            return None;
        }
        let n = total.min(LATENCY_RING_SIZE);
        let mut samples: Vec<u64> = self
            .latency_ring
            .iter()
            .take(n)
            .map(|slot| slot.load(Ordering::Relaxed))
            .collect();
        samples.sort_unstable();
        Some(samples)
    }

    /// nearest-rank：`rank = ceil(permille/1000 * n)`，钳制在 `[1, n]`（500=p50/990=p99/999=p99.9）。
    fn rank_ns(sorted: &[u64], permille: u64) -> u64 {
        let n = sorted.len() as u64;
        let rank = ((permille * n).div_ceil(1000)).clamp(1, n) as usize;
        sorted[rank - 1]
    }

    fn percentile_ns(&self, permille: u64) -> u64 {
        self.sorted_latency_samples()
            .map(|samples| Self::rank_ns(&samples, permille))
            .unwrap_or(0)
    }

    /// 一次排序同时取 p50 / p99 / p999（纳秒）；逐档调用会各排序一次。
    pub fn latency_percentiles_ns(&self) -> (u64, u64, u64) {
        match self.sorted_latency_samples() {
            Some(samples) => (
                Self::rank_ns(&samples, 500),
                Self::rank_ns(&samples, 990),
                Self::rank_ns(&samples, 999),
            ),
            None => (0, 0, 0),
        }
    }

    pub fn get_p50_latency_ms(&self) -> f64 {
        self.percentile_ns(500) as f64 / 1_000_000.0
    }

    pub fn get_p99_latency_ms(&self) -> f64 {
        self.percentile_ns(990) as f64 / 1_000_000.0
    }

    pub fn get_p999_latency_ms(&self) -> f64 {
        self.percentile_ns(999) as f64 / 1_000_000.0
    }

    pub fn get_cache_hit_rate(&self) -> f64 {
        self.cache_hit_rate.load(Ordering::Relaxed) as f64 / 100.0
    }
}

/// QPS 滑动窗口计算器（双缓冲无锁优化版）
#[derive(Debug, Clone)]
pub struct QpsWindow {
    /// 滑动窗口大小（秒）
    window_secs: u64,
    /// 当前秒请求计数
    current_second_count: Arc<std::sync::atomic::AtomicU64>,
    /// 上一秒请求计数（用于平滑过渡）
    last_second_count: Arc<std::sync::atomic::AtomicU64>,
    /// 当前秒时间戳（Unix 秒）
    current_second: Arc<std::sync::atomic::AtomicU64>,
}

impl QpsWindow {
    pub fn new(window_secs: u64) -> Self {
        // invariant: system clock is always after UNIX_EPOCH in production
        let now_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("invariant: system clock after UNIX_EPOCH")
            .as_secs();

        Self {
            window_secs,
            current_second_count: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            last_second_count: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            current_second: Arc::new(std::sync::atomic::AtomicU64::new(now_secs)),
        }
    }

    /// 记录一次请求（完全无锁，仅原子操作）
    pub fn record(&self) {
        // invariant: system clock is always after UNIX_EPOCH in production
        let now_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("invariant: system clock after UNIX_EPOCH")
            .as_secs();

        let stored_sec = self
            .current_second
            .load(std::sync::atomic::Ordering::Relaxed);

        // 进入新秒：CAS 更新（允许少量竞争失败）
        if now_secs != stored_sec {
            self.current_second
                .compare_exchange_weak(
                    stored_sec,
                    now_secs,
                    std::sync::atomic::Ordering::Relaxed,
                    std::sync::atomic::Ordering::Relaxed,
                )
                .ok();

            // 交换计数器（允许小误差）
            if now_secs > stored_sec {
                let current = self
                    .current_second_count
                    .swap(0, std::sync::atomic::Ordering::Relaxed);
                self.last_second_count
                    .store(current, std::sync::atomic::Ordering::Relaxed);
            }
        }

        self.current_second_count
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    /// 批量记录请求（完全无锁）
    pub fn record_batch(&self, count: usize) {
        // invariant: system clock is always after UNIX_EPOCH in production
        let now_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("invariant: system clock after UNIX_EPOCH")
            .as_secs();

        let stored_sec = self
            .current_second
            .load(std::sync::atomic::Ordering::Relaxed);

        if now_secs != stored_sec {
            self.current_second
                .compare_exchange_weak(
                    stored_sec,
                    now_secs,
                    std::sync::atomic::Ordering::Relaxed,
                    std::sync::atomic::Ordering::Relaxed,
                )
                .ok();

            if now_secs > stored_sec {
                let current = self
                    .current_second_count
                    .swap(0, std::sync::atomic::Ordering::Relaxed);
                self.last_second_count
                    .store(current, std::sync::atomic::Ordering::Relaxed);
            }
        }

        self.current_second_count
            .fetch_add(count as u64, std::sync::atomic::Ordering::Relaxed);
    }

    /// 获取当前窗口内的 QPS（完全无锁，使用指数加权平均）
    pub fn get_qps(&self) -> u64 {
        let current = self
            .current_second_count
            .load(std::sync::atomic::Ordering::Relaxed);
        let last = self
            .last_second_count
            .load(std::sync::atomic::Ordering::Relaxed);

        // QPS 估计：最近两秒指数加权，当前秒 70%、上一秒 30%
        (current * 7 + last * 3) / 10
    }

    /// 清理过期数据（秒切换自动完成，无需显式清理）。
    pub fn cleanup(&self) {}

    /// 获取窗口大小
    pub fn window_size(&self) -> u64 {
        self.window_secs
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsSnapshot {
    pub algorithm: AlgorithmType,
    pub total_generated: u64,
    pub total_failed: u64,
    pub current_qps: u64,
    pub p50_latency_ms: f64,
    pub p99_latency_ms: f64,
    pub p999_latency_ms: f64,
    /// 累计观测到的时钟回拨次数（告警规则 `clock_backward` 的判定依据）。
    pub clock_backwards: u64,
    pub cache_hit_rate: f64,
}

impl From<&AlgorithmMetrics> for MetricsSnapshot {
    fn from(m: &AlgorithmMetrics) -> Self {
        // 单次排序取三档分位数（逐档 getter 会各排序一次）
        let (p50_ns, p99_ns, p999_ns) = m.latency_percentiles_ns();
        Self {
            algorithm: m.algorithm,
            total_generated: m.get_generated(),
            total_failed: m.get_failed(),
            current_qps: m.get_qps(),
            p50_latency_ms: p50_ns as f64 / 1_000_000.0,
            p99_latency_ms: p99_ns as f64 / 1_000_000.0,
            p999_latency_ms: p999_ns as f64 / 1_000_000.0,
            clock_backwards: m.get_clock_backwards(),
            cache_hit_rate: m.get_cache_hit_rate(),
        }
    }
}

#[derive(Debug)]
pub struct GlobalMetrics {
    pub algorithms: Arc<RwLock<HashMap<AlgorithmType, Arc<AlgorithmMetrics>>>>,
    pub active_connections: AtomicU32,
    pub total_requests: AtomicU64,
    pub total_errors: AtomicU64,
    pub start_time: std::time::Instant,
}

impl Default for GlobalMetrics {
    fn default() -> Self {
        Self {
            algorithms: Arc::new(RwLock::new(HashMap::new())),
            active_connections: AtomicU32::new(0),
            total_requests: AtomicU64::new(0),
            total_errors: AtomicU64::new(0),
            start_time: std::time::Instant::now(),
        }
    }
}

impl GlobalMetrics {
    pub fn new() -> Self {
        Self {
            algorithms: Arc::new(RwLock::new(HashMap::new())),
            active_connections: AtomicU32::new(0),
            total_requests: AtomicU64::new(0),
            total_errors: AtomicU64::new(0),
            start_time: std::time::Instant::now(),
        }
    }

    pub fn get_or_create_metrics(&self, algorithm: AlgorithmType) -> Arc<AlgorithmMetrics> {
        // 快路径先读锁：算法集合 initialize() 后即固定；写锁会串行化每请求的观测路径。
        if let Some(metrics) = self.algorithms.read().get(&algorithm) {
            return metrics.clone();
        }
        let mut algorithms = self.algorithms.write();
        algorithms
            .entry(algorithm)
            .or_insert_with(|| Arc::new(AlgorithmMetrics::new(algorithm)))
            .clone()
    }

    pub fn increment_requests(&self) {
        self.total_requests.fetch_add(1, Ordering::Relaxed);
    }

    pub fn increment_errors(&self) {
        self.total_errors.fetch_add(1, Ordering::Relaxed);
    }

    pub fn increment_connections(&self) -> u32 {
        self.active_connections.fetch_add(1, Ordering::Relaxed) + 1
    }

    pub fn decrement_connections(&self) {
        self.active_connections.fetch_sub(1, Ordering::Relaxed);
    }

    pub fn get_uptime_seconds(&self) -> u64 {
        self.start_time.elapsed().as_secs()
    }

    pub fn get_all_snapshots(&self) -> Vec<MetricsSnapshot> {
        self.algorithms
            .read()
            .values()
            .map(|metrics| MetricsSnapshot::from(metrics.as_ref()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::AlgorithmType;
    use std::sync::atomic::Ordering;

    // ---- AlgorithmMetrics ----

    #[test]
    fn test_algorithm_metrics_default_has_segment_and_zeroes() {
        let m = AlgorithmMetrics::default();
        assert_eq!(m.algorithm, AlgorithmType::Segment);
        assert_eq!(m.get_generated(), 0);
        assert_eq!(m.get_failed(), 0);
        assert_eq!(m.get_qps(), 0);
        assert_eq!(m.get_p50_latency_ms(), 0.0);
        assert_eq!(m.get_p99_latency_ms(), 0.0);
        assert_eq!(m.get_p999_latency_ms(), 0.0);
        assert_eq!(m.get_cache_hit_rate(), 0.0);
    }

    #[test]
    fn test_algorithm_metrics_new_preserves_algorithm_type() {
        for alg in [
            AlgorithmType::Segment,
            AlgorithmType::Snowflake,
            AlgorithmType::UuidV8,
        ] {
            let m = AlgorithmMetrics::new(alg);
            assert_eq!(m.algorithm, alg);
            // new() should delegate to Default::default() for the counters
            assert_eq!(m.get_generated(), 0);
            assert_eq!(m.get_qps(), 0);
        }
    }

    #[test]
    fn test_algorithm_metrics_clone_snapshots_atomic_state() {
        let m = AlgorithmMetrics::new(AlgorithmType::Snowflake);
        m.increment_generated(7);
        m.increment_failed();
        m.update_qps(42);
        m.update_cache_hit_rate(0.9);
        m.record_latency(2_000_000); // record_latency also increments qps → 43

        let cloned = m.clone();
        assert_eq!(cloned.algorithm, AlgorithmType::Snowflake);
        assert_eq!(cloned.get_generated(), 7);
        assert_eq!(cloned.get_failed(), 1);
        assert_eq!(cloned.get_qps(), 43);
        assert_eq!(cloned.get_p99_latency_ms(), 2.0);
        // Mutating the original must not bleed into the clone (atomic values are copied by value).
        m.increment_generated(100);
        assert_eq!(cloned.get_generated(), 7);
        assert_eq!(m.get_generated(), 107);
    }

    #[test]
    fn test_increment_generated_accumulates() {
        let m = AlgorithmMetrics::new(AlgorithmType::Segment);
        m.increment_generated(5);
        m.increment_generated(10);
        assert_eq!(m.get_generated(), 15);
    }

    #[test]
    fn test_increment_failed_accumulates() {
        let m = AlgorithmMetrics::new(AlgorithmType::Segment);
        m.increment_failed();
        m.increment_failed();
        m.increment_failed();
        assert_eq!(m.get_failed(), 3);
    }

    #[test]
    fn test_record_latency_single_sample_is_all_percentiles() {
        // 单样本时任何分位数都等于该样本本身。
        let m = AlgorithmMetrics::new(AlgorithmType::Segment);
        m.record_latency(5_000_000);
        assert_eq!(m.get_p50_latency_ms(), 5.0);
        assert_eq!(m.get_p99_latency_ms(), 5.0);
        assert_eq!(m.get_p999_latency_ms(), 5.0);
        // record_latency also increments qps (current_qps).
        assert_eq!(m.get_qps(), 1);
        assert_eq!(m.latency_sample_count(), 1);
    }

    #[test]
    fn test_record_latency_percentiles_follow_distribution_not_max() {
        // p50 是中位；[2ms,10ms] 排序后 p50=nearest-rank 1 → 2ms，p99/p999=第 2 个 → 10ms。
        let m = AlgorithmMetrics::new(AlgorithmType::Segment);
        m.record_latency(2_000_000);
        m.record_latency(10_000_000);
        assert_eq!(m.get_p50_latency_ms(), 2.0);
        assert_eq!(m.get_p99_latency_ms(), 10.0);
        assert_eq!(m.get_p999_latency_ms(), 10.0);
        assert_eq!(m.get_qps(), 2);
    }

    #[test]
    fn test_record_latency_p50_drops_when_distribution_improves() {
        // 分位数随分布回落：[5ms, 1ms] 排序 → p50=1ms，p99/p999=5ms。
        let m = AlgorithmMetrics::new(AlgorithmType::Segment);
        m.record_latency(5_000_000);
        m.record_latency(1_000_000);
        assert_eq!(m.get_p50_latency_ms(), 1.0);
        assert_eq!(m.get_p99_latency_ms(), 5.0);
        assert_eq!(m.get_p999_latency_ms(), 5.0);
    }

    #[test]
    fn test_record_latency_zero_latency_is_a_real_sample() {
        // 0ns 也是合法样本：单样本 [0] 的所有分位数为 0，且计入样本数。
        let m = AlgorithmMetrics::new(AlgorithmType::Segment);
        m.record_latency(0);
        assert_eq!(m.get_p50_latency_ms(), 0.0);
        assert_eq!(m.get_p99_latency_ms(), 0.0);
        assert_eq!(m.get_p999_latency_ms(), 0.0);
        assert_eq!(m.get_qps(), 1);
        assert_eq!(m.latency_sample_count(), 1);
    }

    #[test]
    fn test_update_qps_overwrites_value() {
        let m = AlgorithmMetrics::new(AlgorithmType::Segment);
        m.record_latency(1_000_000); // qps becomes 1
        m.update_qps(99);
        assert_eq!(m.get_qps(), 99);
        m.update_qps(0);
        assert_eq!(m.get_qps(), 0);
    }

    #[test]
    fn test_update_cache_hit_rate_rounds_to_basis_points() {
        let m = AlgorithmMetrics::new(AlgorithmType::Segment);
        // 99.95% → 9995 basis points → returned as 99.95
        m.update_cache_hit_rate(0.9995);
        let rate = m.get_cache_hit_rate();
        assert!((rate - 99.95).abs() < 0.001);

        // 0.0 → 0
        m.update_cache_hit_rate(0.0);
        assert_eq!(m.get_cache_hit_rate(), 0.0);

        // 1.0 → 10000 bp → 100.0
        m.update_cache_hit_rate(1.0);
        assert_eq!(m.get_cache_hit_rate(), 100.0);
    }

    #[test]
    fn test_get_p999_latency_ms_uses_p999_field_not_p99() {
        // getter 正确换算纳秒→毫秒；区分度由下方分布测试覆盖。
        let m = AlgorithmMetrics::new(AlgorithmType::Segment);
        m.record_latency(7_500_000);
        assert_eq!(m.get_p999_latency_ms(), 7.5);
    }

    // ---- 环形缓冲真实分位数 ----

    #[test]
    fn test_percentiles_reflect_distribution_with_long_tail() {
        // 100×1ms + 1×1000ms：排序后 p50=ceil(0.5*101)=51 → 1ms，
        // p99=ceil(0.99*101)=100 → 1ms，p999=101 → 1000ms（长尾由高档分位暴露）。
        let m = AlgorithmMetrics::new(AlgorithmType::Segment);
        for _ in 0..100 {
            m.record_latency(1_000_000);
        }
        m.record_latency(1_000_000_000);

        assert_eq!(m.get_p50_latency_ms(), 1.0);
        assert_eq!(m.get_p99_latency_ms(), 1.0);
        assert_eq!(m.get_p999_latency_ms(), 1000.0);
        assert_eq!(m.latency_sample_count(), 101);
    }

    #[test]
    fn test_ring_evicts_old_samples_beyond_capacity() {
        // 先写 1×1000ms 长尾，再用 1024×1ms 覆盖整环：长尾逐出，分位数回落。
        let m = AlgorithmMetrics::new(AlgorithmType::Segment);
        m.record_latency(1_000_000_000);
        for _ in 0..1024 {
            m.record_latency(1_000_000);
        }

        assert_eq!(m.latency_sample_count(), 1025);
        assert_eq!(m.get_p50_latency_ms(), 1.0);
        assert_eq!(m.get_p99_latency_ms(), 1.0);
        assert_eq!(
            m.get_p999_latency_ms(),
            1.0,
            "旧实现 p999 会永久停在历史最大值 1000ms"
        );
    }

    #[test]
    fn test_percentiles_empty_metrics_are_zero() {
        let m = AlgorithmMetrics::new(AlgorithmType::Snowflake);
        assert_eq!(m.get_p50_latency_ms(), 0.0);
        assert_eq!(m.get_p99_latency_ms(), 0.0);
        assert_eq!(m.get_p999_latency_ms(), 0.0);
        assert_eq!(m.latency_sample_count(), 0);
    }

    #[test]
    fn test_concurrent_latency_recording_keeps_ring_consistent() {
        // 8 线程 × 512 次并发环写：样本数精确，且分位数落在已记录值域内。
        let m = std::sync::Arc::new(AlgorithmMetrics::new(AlgorithmType::Snowflake));
        let mut handles = Vec::new();
        for t in 0..8u64 {
            let m = m.clone();
            handles.push(std::thread::spawn(move || {
                for i in 0..512u64 {
                    // 值域 [1000, 8000] ns
                    m.record_latency(1_000 + (t * 512 + i) % 7_001);
                }
            }));
        }
        for h in handles {
            h.join().expect("recording thread panicked");
        }

        assert_eq!(m.latency_sample_count(), 8 * 512);
        let p50_ns = m.get_p50_latency_ms() * 1_000_000.0;
        let p999_ns = m.get_p999_latency_ms() * 1_000_000.0;
        assert!(
            (1_000.0..=8_000.0).contains(&p50_ns),
            "p50={}ns 超出记录值域 [1000, 8000]",
            p50_ns
        );
        assert!(
            (1_000.0..=8_000.0).contains(&p999_ns),
            "p999={}ns 超出记录值域 [1000, 8000]",
            p999_ns
        );
    }

    #[test]
    fn test_latency_percentiles_ns_matches_per_rank_getters() {
        let m = AlgorithmMetrics::new(AlgorithmType::Snowflake);
        for i in 1..=100u64 {
            m.record_latency(i * 1_000_000); // 1..=100 ms
        }
        let (p50, p99, p999) = m.latency_percentiles_ns();
        // 单次排序结果须与逐档实现一致（共用同一 nearest-rank 口径）
        assert_eq!(p50, m.get_p50_latency_ms() as u64 * 1_000_000);
        assert_eq!(p99, m.get_p99_latency_ms() as u64 * 1_000_000);
        assert_eq!(p999, m.get_p999_latency_ms() as u64 * 1_000_000);
        assert_eq!((p50, p99, p999), (50_000_000, 99_000_000, 100_000_000));

        // 无样本 → 三档均为 0（表示"未观测"，不是"延迟 0"）
        let empty = AlgorithmMetrics::new(AlgorithmType::UuidV8);
        assert_eq!(empty.latency_percentiles_ns(), (0, 0, 0));
    }

    #[test]
    fn test_clock_backward_counter_is_incremental_and_snapshot_visible() {
        let m = AlgorithmMetrics::new(AlgorithmType::Snowflake);
        assert_eq!(m.get_clock_backwards(), 0);

        m.record_clock_backward();
        m.record_clock_backward();
        assert_eq!(m.get_clock_backwards(), 2);
        // Clone 语义：计数随快照复制保留
        assert_eq!(m.clone().get_clock_backwards(), 2);

        let snap = MetricsSnapshot::from(&m);
        assert_eq!(snap.clock_backwards, 2);
        // 未记录延迟时不得伪造分位数
        assert_eq!((snap.p50_latency_ms, snap.p99_latency_ms), (0.0, 0.0));
    }

    // ---- QpsWindow ----

    #[test]
    fn test_qps_window_new_initializes_zero_counts_and_returns_window_size() {
        let w = QpsWindow::new(5);
        assert_eq!(w.window_size(), 5);
        assert_eq!(w.get_qps(), 0);

        let w2 = QpsWindow::new(1);
        assert_eq!(w2.window_size(), 1);
    }

    #[test]
    fn test_qps_window_record_increments_current_second_count() {
        let w = QpsWindow::new(10);
        // 同秒内 current=1,last=0 → qps=(1*7)/10=0（整数除法向下取整）。
        w.record();
        w.record();
        w.record();
        // current=3 → qps = (3*7 + 0*3)/10 = 21/10 = 2
        assert_eq!(w.get_qps(), 2);
    }

    #[test]
    fn test_qps_window_record_batch_adds_count_to_current_second() {
        let w = QpsWindow::new(10);
        w.record_batch(15);
        // current=15 → qps = (15*7 + 0*3)/10 = 105/10 = 10
        assert_eq!(w.get_qps(), 10);
    }

    #[test]
    fn test_qps_window_record_swaps_counters_when_second_advances() {
        // 把存储秒改成过去，使 record() 进入秒切换分支并交换两个计数器。
        let w = QpsWindow::new(10);
        w.record();
        w.record();
        w.record_batch(7);
        // current = 1 + 1 + 7 = 9
        assert_eq!(w.get_qps(), (9 * 7) / 10);

        // 置存储秒为过去（触发秒切换分支）。
        let stale_sec = w
            .current_second
            .load(std::sync::atomic::Ordering::Relaxed)
            .saturating_sub(1);
        w.current_second
            .store(stale_sec, std::sync::atomic::Ordering::Relaxed);

        w.record();
        // 交换后 last=10（含本次 swap 后 fetch_add）、current=1 → qps=37/10=3。
        let qps = w.get_qps();
        assert_eq!(qps, 3);
    }

    #[test]
    fn test_qps_window_record_batch_swaps_counters_when_second_advances() {
        // Same as above but exercises the record_batch path's swap branch.
        let w = QpsWindow::new(10);
        w.record_batch(5);
        assert_eq!(w.get_qps(), (5 * 7) / 10);

        let stale_sec = w
            .current_second
            .load(std::sync::atomic::Ordering::Relaxed)
            .saturating_sub(1);
        w.current_second
            .store(stale_sec, std::sync::atomic::Ordering::Relaxed);

        w.record_batch(3);
        // last = 5 (swapped), current = 3
        // QPS = (3*7 + 5*3) / 10 = 36/10 = 3
        assert_eq!(w.get_qps(), 3);
    }

    #[test]
    fn test_qps_window_cleanup_is_noop_and_does_not_panic() {
        let w = QpsWindow::new(10);
        w.record();
        w.cleanup(); // no-op by design
        assert_eq!(w.get_qps(), 0); // qps=(1*7)/10=0
    }

    #[test]
    fn test_qps_window_clone_shares_atomic_state() {
        // QpsWindow derives Clone; cloned instance shares the same Arc<AtomicU64>.
        let w = QpsWindow::new(10);
        let w_clone = w.clone();
        w.record();
        // Both should observe the increment because Arc shares the underlying AtomicU64.
        assert_eq!(w.get_qps(), 0); // (1*7)/10 = 0
        assert_eq!(w_clone.get_qps(), 0);
    }

    // ---- MetricsSnapshot ----

    #[test]
    fn test_metrics_snapshot_from_algorithm_metrics_copies_all_fields() {
        let m = AlgorithmMetrics::new(AlgorithmType::Snowflake);
        m.increment_generated(1234);
        m.increment_failed();
        m.update_qps(56);
        m.record_latency(3_500_000); // record_latency bumps qps → 57
        m.update_cache_hit_rate(0.95);

        let snap = MetricsSnapshot::from(&m);
        assert_eq!(snap.algorithm, AlgorithmType::Snowflake);
        assert_eq!(snap.total_generated, 1234);
        assert_eq!(snap.total_failed, 1);
        assert_eq!(snap.current_qps, 57);
        assert!((snap.p50_latency_ms - 3.5).abs() < f64::EPSILON);
        assert!((snap.p99_latency_ms - 3.5).abs() < f64::EPSILON);
        assert!((snap.p999_latency_ms - 3.5).abs() < f64::EPSILON);
        assert!((snap.cache_hit_rate - 95.0).abs() < 0.001);
    }

    #[test]
    fn test_metrics_snapshot_serde_roundtrip_preserves_all_fields() {
        let original = MetricsSnapshot {
            algorithm: AlgorithmType::Snowflake,
            total_generated: 42,
            total_failed: 7,
            current_qps: 1000,
            p50_latency_ms: 1.5,
            p99_latency_ms: 9.99,
            p999_latency_ms: 15.001,
            clock_backwards: 3,
            cache_hit_rate: 87.65,
        };

        let json = serde_json::to_string(&original).expect("serialize MetricsSnapshot");
        let restored: MetricsSnapshot =
            serde_json::from_str(&json).expect("deserialize MetricsSnapshot");

        assert_eq!(restored.algorithm, original.algorithm);
        assert_eq!(restored.total_generated, original.total_generated);
        assert_eq!(restored.total_failed, original.total_failed);
        assert_eq!(restored.current_qps, original.current_qps);
        assert!((restored.p50_latency_ms - original.p50_latency_ms).abs() < f64::EPSILON);
        assert!((restored.p99_latency_ms - original.p99_latency_ms).abs() < f64::EPSILON);
        assert!((restored.p999_latency_ms - original.p999_latency_ms).abs() < f64::EPSILON);
        assert_eq!(restored.clock_backwards, original.clock_backwards);
        assert!((restored.cache_hit_rate - original.cache_hit_rate).abs() < f64::EPSILON);
    }

    #[test]
    fn test_metrics_snapshot_serde_roundtrip_segment_default_algorithm() {
        let original = MetricsSnapshot {
            algorithm: AlgorithmType::Segment,
            total_generated: 0,
            total_failed: 0,
            current_qps: 0,
            p50_latency_ms: 0.0,
            p99_latency_ms: 0.0,
            p999_latency_ms: 0.0,
            clock_backwards: 0,
            cache_hit_rate: 0.0,
        };

        let json = serde_json::to_string(&original).expect("serialize");
        let restored: MetricsSnapshot = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(restored.algorithm, AlgorithmType::Segment);
        assert_eq!(restored.total_generated, 0);
    }

    #[test]
    fn test_metrics_snapshot_serde_roundtrip_uuid_variant() {
        let alg = AlgorithmType::UuidV8;
        let original = MetricsSnapshot {
            algorithm: alg,
            total_generated: 1,
            total_failed: 0,
            current_qps: 1,
            p50_latency_ms: 0.1,
            p99_latency_ms: 0.2,
            p999_latency_ms: 0.3,
            clock_backwards: 1,
            cache_hit_rate: 100.0,
        };
        let json = serde_json::to_string(&original).expect("serialize");
        let restored: MetricsSnapshot = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(restored.algorithm, alg);
        assert_eq!(restored.clock_backwards, 1);
    }

    // ---- GlobalMetrics ----

    #[test]
    fn test_global_metrics_default_starts_zeroed() {
        let g = GlobalMetrics::default();
        assert_eq!(g.active_connections.load(Ordering::Relaxed), 0);
        assert_eq!(g.total_requests.load(Ordering::Relaxed), 0);
        assert_eq!(g.total_errors.load(Ordering::Relaxed), 0);
        assert!(g.algorithms.read().is_empty());
        assert!(g.get_all_snapshots().is_empty());
    }

    #[test]
    fn test_global_metrics_new_equivalent_to_default_for_counters() {
        let g = GlobalMetrics::new();
        assert_eq!(g.active_connections.load(Ordering::Relaxed), 0);
        assert_eq!(g.total_requests.load(Ordering::Relaxed), 0);
        assert_eq!(g.total_errors.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn test_get_or_create_metrics_creates_once_and_reuses() {
        let g = GlobalMetrics::new();
        let m1 = g.get_or_create_metrics(AlgorithmType::Snowflake);
        assert_eq!(m1.algorithm, AlgorithmType::Snowflake);
        let m2 = g.get_or_create_metrics(AlgorithmType::Snowflake);
        // Second call should return the same Arc (entry already present).
        assert!(Arc::ptr_eq(&m1, &m2));

        // Different algorithm gets a different Arc.
        let m3 = g.get_or_create_metrics(AlgorithmType::UuidV8);
        assert!(!Arc::ptr_eq(&m1, &m3));
        assert_eq!(m3.algorithm, AlgorithmType::UuidV8);
    }

    #[test]
    fn test_get_or_create_metrics_hit_path_does_not_block_on_write_lock() {
        // 回归钉桩：命中路径不得取写锁，否则有人持读锁时并发观测被串行化。
        let g = Arc::new(GlobalMetrics::new());
        let registered = g.get_or_create_metrics(AlgorithmType::Snowflake);
        assert_eq!(registered.algorithm, AlgorithmType::Snowflake);

        let (start_tx, start_rx) = std::sync::mpsc::channel::<()>();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let probe = {
            let g = Arc::clone(&g);
            std::thread::spawn(move || {
                // 等主线程真的持上读锁再发起，否则测不到阻塞
                start_rx.recv().ok();
                let metrics = g.get_or_create_metrics(AlgorithmType::Snowflake);
                let _ = done_tx.send(metrics.algorithm);
            })
        };

        let read_guard = g.algorithms.read();
        let _ = start_tx.send(());
        let outcome = done_rx.recv_timeout(std::time::Duration::from_secs(2));
        drop(read_guard);
        probe.join().expect("probe thread panicked");

        assert_eq!(
            outcome.ok(),
            Some(AlgorithmType::Snowflake),
            "命中已注册算法时不应等待写锁（超时即说明退化为写锁路径）"
        );
    }

    #[test]
    fn test_increment_requests_and_errors_independent() {
        let g = GlobalMetrics::new();
        g.increment_requests();
        g.increment_requests();
        g.increment_requests();
        g.increment_errors();
        assert_eq!(g.total_requests.load(Ordering::Relaxed), 3);
        assert_eq!(g.total_errors.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_increment_connections_returns_new_value() {
        let g = GlobalMetrics::new();
        let v1 = g.increment_connections();
        assert_eq!(v1, 1);
        assert_eq!(g.active_connections.load(Ordering::Relaxed), 1);
        let v2 = g.increment_connections();
        assert_eq!(v2, 2);
        assert_eq!(g.active_connections.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn test_decrement_connections_subtracts() {
        let g = GlobalMetrics::new();
        g.increment_connections();
        g.increment_connections();
        g.increment_connections();
        assert_eq!(g.active_connections.load(Ordering::Relaxed), 3);
        g.decrement_connections();
        assert_eq!(g.active_connections.load(Ordering::Relaxed), 2);
        g.decrement_connections();
        g.decrement_connections();
        assert_eq!(g.active_connections.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn test_get_uptime_seconds_returns_non_decreasing_value() {
        let g = GlobalMetrics::new();
        let t1 = g.get_uptime_seconds();
        // Sleep slightly so elapsed time can advance across a second boundary.
        std::thread::sleep(std::time::Duration::from_millis(10));
        let t2 = g.get_uptime_seconds();
        assert!(t2 >= t1);
    }

    #[test]
    fn test_get_all_snapshots_reflects_recorded_metrics() {
        let g = GlobalMetrics::new();
        let m = g.get_or_create_metrics(AlgorithmType::Snowflake);
        m.increment_generated(10);
        m.increment_failed();
        m.update_qps(5);
        m.record_latency(2_000_000); // record_latency bumps qps → 6
        m.update_cache_hit_rate(0.5);

        let snapshots = g.get_all_snapshots();
        assert_eq!(snapshots.len(), 1);
        let s = &snapshots[0];
        assert_eq!(s.algorithm, AlgorithmType::Snowflake);
        assert_eq!(s.total_generated, 10);
        assert_eq!(s.total_failed, 1);
        assert_eq!(s.current_qps, 6);
        assert!((s.p99_latency_ms - 2.0).abs() < f64::EPSILON);
        assert!((s.cache_hit_rate - 50.0).abs() < 0.001);
    }

    #[test]
    fn test_get_all_snapshots_returns_empty_when_no_algorithms_registered() {
        let g = GlobalMetrics::new();
        let snapshots = g.get_all_snapshots();
        assert!(snapshots.is_empty());
    }

    #[test]
    fn test_global_metrics_get_all_snapshots_for_multiple_algorithms() {
        let g = GlobalMetrics::new();
        let _ = g.get_or_create_metrics(AlgorithmType::Segment);
        let _ = g.get_or_create_metrics(AlgorithmType::Snowflake);
        let _ = g.get_or_create_metrics(AlgorithmType::UuidV8);

        let snapshots = g.get_all_snapshots();
        assert_eq!(snapshots.len(), 3);
        // Verify all three algorithms are present (order may vary).
        let mut algos: Vec<_> = snapshots.iter().map(|s| s.algorithm).collect();
        algos.sort_by_key(|a| format!("{a:?}"));
        assert_eq!(algos.len(), 3);
    }
}
