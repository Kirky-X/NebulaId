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
