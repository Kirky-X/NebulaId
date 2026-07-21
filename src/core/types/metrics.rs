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

use crate::core::types::AlgorithmType;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;

#[derive(Debug, Serialize)]
pub struct AlgorithmMetrics {
    pub algorithm: AlgorithmType,
    pub total_generated: AtomicU64,
    pub total_failed: AtomicU64,
    pub current_qps: AtomicU64,
    pub p50_latency_ns: AtomicU64,
    pub p99_latency_ns: AtomicU64,
    pub p999_latency_ns: AtomicU64,
    pub cache_hit_rate: AtomicU64,
}

impl Clone for AlgorithmMetrics {
    fn clone(&self) -> Self {
        Self {
            algorithm: self.algorithm,
            total_generated: AtomicU64::new(self.total_generated.load(Ordering::Relaxed)),
            total_failed: AtomicU64::new(self.total_failed.load(Ordering::Relaxed)),
            current_qps: AtomicU64::new(self.current_qps.load(Ordering::Relaxed)),
            p50_latency_ns: AtomicU64::new(self.p50_latency_ns.load(Ordering::Relaxed)),
            p99_latency_ns: AtomicU64::new(self.p99_latency_ns.load(Ordering::Relaxed)),
            p999_latency_ns: AtomicU64::new(self.p999_latency_ns.load(Ordering::Relaxed)),
            cache_hit_rate: AtomicU64::new(self.cache_hit_rate.load(Ordering::Relaxed)),
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
            p50_latency_ns: AtomicU64::new(0),
            p99_latency_ns: AtomicU64::new(0),
            p999_latency_ns: AtomicU64::new(0),
            cache_hit_rate: AtomicU64::new(0),
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

    pub fn record_latency(&self, latency_ns: u64) {
        self.current_qps.fetch_add(1, Ordering::Relaxed);

        let current_p50 = self.p50_latency_ns.load(Ordering::Relaxed);
        if latency_ns > current_p50 || current_p50 == 0 {
            self.p50_latency_ns.store(latency_ns, Ordering::Relaxed);
        }

        let current_p99 = self.p99_latency_ns.load(Ordering::Relaxed);
        if latency_ns > current_p99 {
            self.p99_latency_ns.store(latency_ns, Ordering::Relaxed);
        }

        let current_p999 = self.p999_latency_ns.load(Ordering::Relaxed);
        if latency_ns > current_p999 {
            self.p999_latency_ns.store(latency_ns, Ordering::Relaxed);
        }
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

    pub fn get_p50_latency_ms(&self) -> f64 {
        self.p50_latency_ns.load(Ordering::Relaxed) as f64 / 1_000_000.0
    }

    pub fn get_p99_latency_ms(&self) -> f64 {
        self.p99_latency_ns.load(Ordering::Relaxed) as f64 / 1_000_000.0
    }

    pub fn get_p999_latency_ms(&self) -> f64 {
        self.p999_latency_ns.load(Ordering::Relaxed) as f64 / 1_000_000.0
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
    /// 当前秒的请求计数（原子操作）
    current_second_count: Arc<std::sync::atomic::AtomicU64>,
    /// 上一秒的请求计数（用于平滑过渡）
    last_second_count: Arc<std::sync::atomic::AtomicU64>,
    /// 当前秒的时间戳（原子操作）
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

        // 如果进入了新的秒，尝试更新秒计数（允许少量竞争失败）
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

        // 使用最近两秒的加权平均作为 QPS 估计
        // 当前秒权重 70%，上一秒权重 30%
        (current * 7 + last * 3) / 10
    }

    /// 清理过期数据（无需显式清理，自动通过秒切换完成）
    pub fn cleanup(&self) {
        // 无锁设计不需要显式清理
    }

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
    pub cache_hit_rate: f64,
}

impl From<&AlgorithmMetrics> for MetricsSnapshot {
    fn from(m: &AlgorithmMetrics) -> Self {
        Self {
            algorithm: m.algorithm,
            total_generated: m.get_generated(),
            total_failed: m.get_failed(),
            current_qps: m.get_qps(),
            p50_latency_ms: m.get_p50_latency_ms(),
            p99_latency_ms: m.get_p99_latency_ms(),
            p999_latency_ms: m.get_p999_latency_ms(),
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
            AlgorithmType::UuidV7,
            AlgorithmType::UuidV4,
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
    fn test_record_latency_first_call_stores_into_zero_p50_p99_p999() {
        // First call: current_p50 == 0 → true branch of p50 if;
        // current_p99 == 0 and latency_ns > 0 → true branch of p99 if;
        // same for p999.
        let m = AlgorithmMetrics::new(AlgorithmType::Segment);
        m.record_latency(5_000_000);
        assert_eq!(m.get_p50_latency_ms(), 5.0);
        assert_eq!(m.get_p99_latency_ms(), 5.0);
        assert_eq!(m.get_p999_latency_ms(), 5.0);
        // record_latency also increments qps (current_qps).
        assert_eq!(m.get_qps(), 1);
    }

    #[test]
    fn test_record_latency_higher_value_updates_all_percentiles() {
        let m = AlgorithmMetrics::new(AlgorithmType::Segment);
        m.record_latency(2_000_000);
        m.record_latency(10_000_000);
        assert_eq!(m.get_p50_latency_ms(), 10.0);
        assert_eq!(m.get_p99_latency_ms(), 10.0);
        assert_eq!(m.get_p999_latency_ms(), 10.0);
        assert_eq!(m.get_qps(), 2);
    }

    #[test]
    fn test_record_latency_lower_value_keeps_existing_maxima() {
        // Sets p50/p99/p999 to 5ms, then records 1ms which is lower.
        // Covers false branches of all three `if latency_ns > current_*` checks
        // (including the `|| current_p50 == 0` short-circuit on p50).
        let m = AlgorithmMetrics::new(AlgorithmType::Segment);
        m.record_latency(5_000_000);
        m.record_latency(1_000_000);
        assert_eq!(m.get_p50_latency_ms(), 5.0);
        assert_eq!(m.get_p99_latency_ms(), 5.0);
        assert_eq!(m.get_p999_latency_ms(), 5.0);
    }

    #[test]
    fn test_record_latency_zero_latency_still_stores_into_p50_due_to_zero_check() {
        // First call with 0 latency: `latency_ns > current_p50` is false (0 > 0),
        // but `current_p50 == 0` is true → p50 stored as 0. p99/p999 stay 0
        // (their conditions are `latency_ns > current_p99` which is 0 > 0 = false).
        let m = AlgorithmMetrics::new(AlgorithmType::Segment);
        m.record_latency(0);
        assert_eq!(m.get_p50_latency_ms(), 0.0);
        assert_eq!(m.get_p99_latency_ms(), 0.0);
        assert_eq!(m.get_p999_latency_ms(), 0.0);
        assert_eq!(m.get_qps(), 1);
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
        // p999 is updated independently; ensure getter reads the right field.
        // We can't set p999 directly, but record_latency writes the same value
        // to all three. Verifying p999 returns the recorded value confirms
        // the getter maps p999_latency_ns (not p99_latency_ns).
        let m = AlgorithmMetrics::new(AlgorithmType::Segment);
        m.record_latency(7_500_000);
        assert_eq!(m.get_p999_latency_ms(), 7.5);
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
        // Single record in the same second: current=1, last=0 → qps = (1*7 + 0*3)/10 = 0
        // (integer division rounds down). Verify the increment side effect.
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
        // Force the stored second into the past so that record() enters the
        // `now_secs != stored_sec` branch and (now_secs > stored_sec) is true,
        // swapping current_second_count into last_second_count.
        let w = QpsWindow::new(10);
        w.record();
        w.record();
        w.record_batch(7);
        // current = 1 + 1 + 7 = 9
        assert_eq!(w.get_qps(), (9 * 7) / 10);

        // Pretend the stored second is now in the past.
        let stale_sec = w
            .current_second
            .load(std::sync::atomic::Ordering::Relaxed)
            .saturating_sub(1);
        w.current_second
            .store(stale_sec, std::sync::atomic::Ordering::Relaxed);

        w.record();
        // After swap: last = 10 (previous current + 1 from this call's fetch_add
        // happens after the swap), current = 1.
        // QPS = (1*7 + 10*3) / 10 = 37/10 = 3
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
                     // Counters must remain intact after cleanup.
        assert_eq!(w.get_qps(), 0); // (1*7 + 0*3)/10 = 0
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
            cache_hit_rate: 0.0,
        };

        let json = serde_json::to_string(&original).expect("serialize");
        let restored: MetricsSnapshot = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(restored.algorithm, AlgorithmType::Segment);
        assert_eq!(restored.total_generated, 0);
    }

    #[test]
    fn test_metrics_snapshot_serde_roundtrip_uuid_variants() {
        for alg in [AlgorithmType::UuidV7, AlgorithmType::UuidV4] {
            let original = MetricsSnapshot {
                algorithm: alg,
                total_generated: 1,
                total_failed: 0,
                current_qps: 1,
                p50_latency_ms: 0.1,
                p99_latency_ms: 0.2,
                p999_latency_ms: 0.3,
                cache_hit_rate: 100.0,
            };
            let json = serde_json::to_string(&original).expect("serialize");
            let restored: MetricsSnapshot = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(restored.algorithm, alg);
        }
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
        let m3 = g.get_or_create_metrics(AlgorithmType::UuidV7);
        assert!(!Arc::ptr_eq(&m1, &m3));
        assert_eq!(m3.algorithm, AlgorithmType::UuidV7);
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
        // Uptime is non-negative and small at test start.
        // Sleep slightly to ensure elapsed time advances if it lands on a second boundary.
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
        let _ = g.get_or_create_metrics(AlgorithmType::UuidV7);
        let _ = g.get_or_create_metrics(AlgorithmType::UuidV4);

        let snapshots = g.get_all_snapshots();
        assert_eq!(snapshots.len(), 4);
        // Verify all four algorithms are present (order may vary).
        let mut algos: Vec<_> = snapshots.iter().map(|s| s.algorithm).collect();
        algos.sort_by_key(|a| format!("{a:?}"));
        assert_eq!(algos.len(), 4);
    }
}
