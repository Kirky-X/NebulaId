use crate::types::AlgorithmType;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
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

/// QPS 滑动窗口计算器
#[derive(Debug, Clone)]
pub struct QpsWindow {
    /// 滑动窗口大小（秒）
    window_secs: u64,
    /// 请求时间戳队列
    timestamps: Arc<parking_lot::Mutex<Vec<std::time::Instant>>>,
}

impl QpsWindow {
    pub fn new(window_secs: u64) -> Self {
        Self {
            window_secs,
            timestamps: Arc::new(parking_lot::Mutex::new(Vec::new())),
        }
    }

    /// 记录一次请求
    pub fn record(&self) {
        let mut timestamps = self.timestamps.lock();
        timestamps.push(std::time::Instant::now());
    }

    /// 批量记录请求
    pub fn record_batch(&self, count: usize) {
        let mut timestamps = self.timestamps.lock();
        let now = std::time::Instant::now();
        for _ in 0..count {
            timestamps.push(now);
        }
    }

    /// 获取当前窗口内的 QPS
    pub fn get_qps(&self) -> u64 {
        let mut timestamps = self.timestamps.lock();
        let now = std::time::Instant::now();
        let window_start = now - std::time::Duration::from_secs(self.window_secs);

        // 移除过期的时间戳
        timestamps.retain(|&ts| ts > window_start);

        // 计算窗口内的请求数
        let count = timestamps.len();

        // 估算当前 QPS（基于窗口大小）
        if count > 0 {
            // 返回窗口内的请求数作为 QPS 估计
            count as u64
        } else {
            0
        }
    }

    /// 清理过期数据
    pub fn cleanup(&self) {
        let mut timestamps = self.timestamps.lock();
        let window_start =
            std::time::Instant::now() - std::time::Duration::from_secs(self.window_secs);
        timestamps.retain(|&ts| ts > window_start);
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
    pub algorithms: DashMap<AlgorithmType, AlgorithmMetrics>,
    pub active_connections: AtomicU32,
    pub total_requests: AtomicU64,
    pub total_errors: AtomicU64,
    pub start_time: std::time::Instant,
}

impl Default for GlobalMetrics {
    fn default() -> Self {
        Self {
            algorithms: DashMap::new(),
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
            algorithms: DashMap::new(),
            active_connections: AtomicU32::new(0),
            total_requests: AtomicU64::new(0),
            total_errors: AtomicU64::new(0),
            start_time: std::time::Instant::now(),
        }
    }

    pub fn get_or_create_metrics(
        &self,
        algorithm: AlgorithmType,
    ) -> impl std::ops::Deref<Target = AlgorithmMetrics> + '_ {
        self.algorithms
            .entry(algorithm)
            .or_insert_with(|| AlgorithmMetrics::new(algorithm))
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
            .iter()
            .map(|entry| MetricsSnapshot::from(entry.value()))
            .collect()
    }
}
