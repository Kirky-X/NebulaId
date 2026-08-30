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

//! Segment 算法模块
//!
//! 提供基于号段（Segment）的 ID 生成实现，包括双缓冲、动态切换与多数据中心
//! 健康探测。生产路径使用 `DefaultSegmentLoader`、`DcFailureDetector` 与 `CpuMonitor`。

use crate::core::algorithm::{
    AlgorithmMetricsSnapshot, GenerateContext, HealthStatus, IdAlgorithm,
};
use crate::core::config::{Config, SegmentAlgorithmConfig};
#[cfg(feature = "etcd")]
use crate::core::coordinator::EtcdClusterHealthMonitor;
use crate::core::types::{AlgorithmType, CoreError, Id, IdBatch, Result};
use arc_swap::{ArcSwap, ArcSwapOption};
use async_trait::async_trait;
use parking_lot::Mutex;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::time::sleep;
use tracing::info;

// Constants for algorithm configuration
const DEFAULT_CPU_USAGE: f64 = 0.1;

/// CPU 使用率监控器
#[derive(Debug)]
pub struct CpuMonitor {
    current_usage: Arc<AtomicU64>,
    last_check: Arc<parking_lot::Mutex<Instant>>,
}

impl Default for CpuMonitor {
    fn default() -> Self {
        Self::new()
    }
}

impl CpuMonitor {
    pub fn new() -> Self {
        Self {
            current_usage: Arc::new(AtomicU64::new(DEFAULT_CPU_USAGE.to_bits())),
            last_check: Arc::new(parking_lot::Mutex::new(Instant::now())),
        }
    }

    /// 获取当前 CPU 使用率（0.0 - 1.0）
    pub fn get_usage(&self) -> f64 {
        f64::from_bits(self.current_usage.load(Ordering::Relaxed))
    }

    /// 更新 CPU 使用率
    pub fn update_usage(&self, usage: f64) {
        let clamped = usage.clamp(0.0, 1.0);
        self.current_usage
            .store(clamped.to_bits(), Ordering::Relaxed);
        *self.last_check.lock() = Instant::now();
    }

    /// 启动 CPU 监控（基于系统指标）
    #[cfg(target_os = "linux")]
    pub fn start_monitoring(&self) -> tokio::task::JoinHandle<()> {
        let usage = self.current_usage.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(5));
            loop {
                interval.tick().await;

                // 读取 /proc/stat 计算 CPU 使用率
                if let Some(cpu_usage) = Self::read_cpu_usage() {
                    usage.store(cpu_usage.to_bits(), Ordering::Relaxed);
                }
            }
        })
    }

    #[cfg(target_os = "linux")]
    fn read_cpu_usage() -> Option<f64> {
        use std::fs;
        let stat = fs::read_to_string("/proc/stat").ok()?;
        let line = stat.lines().next()?;
        let parts: Vec<u64> = line
            .split_whitespace()
            .skip(1)
            .filter_map(|s| s.parse().ok())
            .collect();

        if parts.len() >= 4 {
            let idle = parts[3];
            let total: u64 = parts.iter().sum();
            let usage = 1.0 - (idle as f64 / total as f64);
            Some(usage)
        } else {
            None
        }
    }

    #[cfg(not(target_os = "linux"))]
    pub fn start_monitoring(&self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            tracing::debug!(
                "{}",
                t!("log.core.algorithm.segment.cpu_monitoring_not_supported")
            );
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum DcStatus {
    Healthy,
    Degraded,
    Failed,
}

#[derive(Debug)]
pub struct DcHealthState {
    pub dc_id: u8,
    pub status: AtomicU8,
    pub last_success: Arc<Mutex<Instant>>,
}

impl DcHealthState {
    pub fn new(dc_id: u8) -> Self {
        Self {
            dc_id,
            status: AtomicU8::new(DcStatus::Healthy as u8),
            last_success: Arc::new(Mutex::new(Instant::now())),
        }
    }

    pub fn get_status(&self) -> DcStatus {
        match self.status.load(Ordering::Relaxed) {
            0 => DcStatus::Healthy,
            1 => DcStatus::Degraded,
            _ => DcStatus::Failed,
        }
    }

    pub fn set_status(&self, status: DcStatus) {
        self.status.store(status as u8, Ordering::Relaxed);
    }
}

pub struct DcFailureDetector {
    dc_states: Arc<RwLock<HashMap<u8, Arc<DcHealthState>>>>,
    failure_threshold: u64,
    recovery_timeout: Duration,
}

impl DcFailureDetector {
    pub fn new(failure_threshold: u64, recovery_timeout: Duration) -> Self {
        Self {
            dc_states: Arc::new(RwLock::new(HashMap::new())),
            failure_threshold,
            recovery_timeout,
        }
    }

    pub fn add_dc(&self, dc_id: u8) {
        let mut states = self.dc_states.write();
        states
            .entry(dc_id)
            .or_insert_with(|| Arc::new(DcHealthState::new(dc_id)));
    }

    pub async fn start_health_check_with_shutdown(
        &self,
        check_interval: Duration,
        mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
    ) {
        let detector = self.clone();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = shutdown_rx.changed() => {
                        info!(
                            "{}",
                            t!("log.core.algorithm.segment.health_check_shutdown_signal")
                        );
                        break;
                    }
                    _ = sleep(check_interval) => {
                        detector.check_recovery().await;
                    }
                }
            }
        });
    }

    async fn check_recovery(&self) {
        let now = Instant::now();
        // 直接持读锁迭代，避免 clone 整个 HashMap（仅修改内部 AtomicU8，不影响 HashMap 结构）
        let states = self.dc_states.read();
        for state in states.values() {
            if state.get_status() == DcStatus::Failed {
                let last_success = *state.last_success.lock();
                if now.duration_since(last_success) > self.recovery_timeout {
                    info!(
                        "{}",
                        t!(
                            "log.core.algorithm.segment.attempting_recovery",
                            dc_id = state.dc_id
                        )
                    );
                    state.set_status(DcStatus::Degraded);
                }
            }
        }
    }
}

impl Clone for DcFailureDetector {
    fn clone(&self) -> Self {
        Self {
            dc_states: self.dc_states.clone(),
            failure_threshold: self.failure_threshold,
            recovery_timeout: self.recovery_timeout,
        }
    }
}

pub struct Segment {
    pub start_id: AtomicU64,
    pub max_id: AtomicU64,
    pub current_id: AtomicU64,
}

impl Segment {
    pub fn new(start_id: u64, max_id: u64) -> Self {
        Self {
            start_id: AtomicU64::new(start_id),
            max_id: AtomicU64::new(max_id),
            current_id: AtomicU64::new(start_id),
        }
    }
}

pub struct AtomicSegment {
    pub inner: Mutex<Segment>,
}

impl AtomicSegment {
    pub fn new(start_id: u64, max_id: u64) -> Self {
        Self {
            inner: Mutex::new(Segment::new(start_id, max_id)),
        }
    }

    pub fn try_consume(&self, count: u64) -> Option<(u64, u64)> {
        let segment = self.inner.lock();
        let current = segment.current_id.load(Ordering::Relaxed);
        let max = segment.max_id.load(Ordering::Relaxed);

        if current + count > max {
            return None;
        }

        let start_id = current;
        segment.current_id.store(current + count, Ordering::Relaxed);
        Some((start_id, current + count))
    }
}

pub struct DoubleBuffer {
    current: Arc<ArcSwap<AtomicSegment>>,
    next: Arc<ArcSwapOption<AtomicSegment>>,
    switch_threshold: f64,
    // diting-perf C2 修复：loading 标记防止多线程并发触发 load_segment
    loading: Arc<std::sync::atomic::AtomicBool>,
}

impl DoubleBuffer {
    pub fn new(switch_threshold: f64) -> Self {
        let initial_segment = Arc::new(AtomicSegment::new(0, 0));
        let current = Arc::new(ArcSwap::from(initial_segment));
        let next = Arc::new(ArcSwapOption::empty());

        Self {
            current,
            next,
            switch_threshold,
            loading: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    /// diting-perf C2 修复：CAS 标记 loading=true，返回是否抢占成功。
    /// 成功的线程负责 load_segment；失败的线程应 spin-wait 直到 loading=false。
    pub fn try_start_loading(&self) -> bool {
        self.loading
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    /// diting-perf C2 修复：load_segment 完成（无论成功失败）后必须调用，重置 loading。
    pub fn finish_loading(&self) {
        self.loading.store(false, Ordering::Release);
    }

    /// diting-perf C2 修复：检查是否正在加载。
    pub fn is_loading(&self) -> bool {
        self.loading.load(Ordering::Acquire)
    }

    pub fn set_next(&self, segment: Arc<AtomicSegment>) {
        self.next.store(Some(segment));
    }

    pub fn get_next(&self) -> Option<Arc<AtomicSegment>> {
        self.next.load_full()
    }

    pub fn swap(&self) -> Option<Arc<AtomicSegment>> {
        let new_current = self.next.swap(None);
        if let Some(ref new_current) = new_current {
            self.current.store(new_current.clone());
        }
        new_current
    }

    pub fn need_switch(&self) -> bool {
        let current = self.current.load_full();
        // 合并两次锁为一次，减少锁开销（原实现 remaining() 和 total 各锁一次）
        let segment = current.inner.lock();
        let current_id = segment.current_id.load(Ordering::Relaxed);
        let max_id = segment.max_id.load(Ordering::Relaxed);
        let start_id = segment.start_id.load(Ordering::Relaxed);
        drop(segment);

        let remaining = max_id.saturating_sub(current_id);
        let total = max_id - start_id;

        if total == 0 {
            return true;
        }

        (remaining as f64 / total as f64) < self.switch_threshold
    }

    pub fn get_current(&self) -> Arc<AtomicSegment> {
        self.current.load_full()
    }
}

pub struct SegmentAlgorithm {
    config: SegmentAlgorithmConfig,
    buffers: Arc<RwLock<HashMap<String, Arc<DoubleBuffer>>>>,
    metrics: Arc<AlgorithmMetricsInner>,
    segment_loader: Arc<dyn SegmentLoader + Send + Sync>,
    dc_failure_detector: Arc<DcFailureDetector>,
    // L12 对齐修复：非 etcd 版本不再持有 `etcd_cluster_health_monitor: Option<()>`
    // 占位字段（与 AlgorithmBuilder / AlgorithmRouter 一致）。`with_etcd_cluster_health_monitor`
    // builder 方法仅在 etcd feature 下存在；非 etcd 版本根本不会调用它。
    #[cfg(feature = "etcd")]
    etcd_cluster_health_monitor: Option<Arc<EtcdClusterHealthMonitor>>,
    /// CPU monitor for dynamic step calculation
    cpu_monitor: Option<Arc<CpuMonitor>>,
    /// CPU monitor task handle
    cpu_monitor_task: Arc<tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>>,
    /// Shutdown channel for graceful termination of background tasks
    shutdown_tx: Arc<tokio::sync::watch::Sender<bool>>,
    /// Handle to the health check task
    health_check_task: Arc<tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>>,
}

struct AlgorithmMetricsInner {
    total_generated: AtomicU64,
    total_failed: AtomicU64,
    cache_hits: AtomicU64,
    cache_misses: AtomicU64,
}

impl Default for AlgorithmMetricsInner {
    fn default() -> Self {
        Self {
            total_generated: AtomicU64::new(0),
            total_failed: AtomicU64::new(0),
            cache_hits: AtomicU64::new(0),
            cache_misses: AtomicU64::new(0),
        }
    }
}

#[async_trait]
pub trait SegmentLoader: Send + Sync {
    async fn load_segment(&self, ctx: &GenerateContext, worker_id: u8) -> Result<SegmentData>;
}

#[derive(Debug, Clone)]
pub struct SegmentData {
    pub start_id: u64,
    pub max_id: u64,
}

impl Default for SegmentAlgorithm {
    fn default() -> Self {
        Self::new(0)
    }
}

impl SegmentAlgorithm {
    pub fn new(local_dc_id: u8) -> Self {
        let dc_failure_detector = Arc::new(DcFailureDetector::new(5, Duration::from_secs(300)));
        dc_failure_detector.add_dc(local_dc_id);

        let (shutdown_tx, _) = tokio::sync::watch::channel(false);

        Self {
            config: SegmentAlgorithmConfig::default(),
            buffers: Arc::new(RwLock::new(HashMap::new())),
            metrics: Arc::new(AlgorithmMetricsInner::default()),
            segment_loader: Arc::new(DefaultSegmentLoader::default()),
            dc_failure_detector,
            #[cfg(feature = "etcd")]
            etcd_cluster_health_monitor: None,
            cpu_monitor: None,
            cpu_monitor_task: Arc::new(tokio::sync::Mutex::new(None)),
            shutdown_tx: Arc::new(shutdown_tx),
            health_check_task: Arc::new(tokio::sync::Mutex::new(None)),
        }
    }

    pub fn with_cpu_monitor(mut self, cpu_monitor: Arc<CpuMonitor>) -> Self {
        self.cpu_monitor = Some(cpu_monitor);
        self
    }

    // L13 修复：`initialize` 从 `impl IdAlgorithm for SegmentAlgorithm`
    // 移到 inherent impl。原 trait method `initialize(&mut self, ...)` 让
    // trait 不那么对象安全（`Arc<dyn IdAlgorithm>` 共享后无法调用 `&mut self`）。
    // 现仅在 `AlgorithmBuilder::build` 中通过具体类型调用，初始化完成后
    // 转为 `Box<dyn IdAlgorithm>` 共享。
    pub async fn initialize(&mut self, config: &Config) -> Result<()> {
        self.config = config.algorithm.segment.clone();

        // Start CPU monitoring if available
        if let Some(ref cpu_monitor) = self.cpu_monitor {
            info!(
                "{}",
                t!("log.core.algorithm.segment.starting_cpu_monitoring")
            );
            let monitor_task = cpu_monitor.start_monitoring();
            *self.cpu_monitor_task.lock().await = Some(monitor_task);
        }

        let detector = self.dc_failure_detector.clone();
        let shutdown_rx = self.shutdown_tx.subscribe();
        let task = tokio::spawn(async move {
            detector
                .start_health_check_with_shutdown(Duration::from_secs(60), shutdown_rx)
                .await;
        });

        *self.health_check_task.lock().await = Some(task);

        Ok(())
    }

    #[cfg(feature = "etcd")]
    pub fn with_etcd_cluster_health_monitor(
        mut self,
        monitor: Arc<EtcdClusterHealthMonitor>,
    ) -> Self {
        self.etcd_cluster_health_monitor = Some(monitor);
        self
    }
    // L12 对齐修复：删除非 etcd 版本的 `with_etcd_cluster_health_monitor(Arc<()>)`。
    // 原签名接受 `Arc<()>` 但完全忽略参数，类型误导且调用方可能误以为
    // monitor 被实际使用。非 etcd 版本根本不需要这个 builder 方法。

    // L12 对齐修复：删除非 etcd 版本的 `get_etcd_cluster_health_monitor() -> Option<&()>`。
    // 非 etcd 版本字段不存在，getter 也无意义。

    fn get_or_create_buffer(&self, key: &str) -> Arc<DoubleBuffer> {
        // 快路径：读锁查找已有 buffer（读多写少场景优化，避免每次获取写锁）
        {
            let buffers = self.buffers.read();
            if let Some(buffer) = buffers.get(key) {
                return buffer.clone();
            }
        }
        // 慢路径：写锁创建新 buffer
        let mut buffers = self.buffers.write();
        buffers
            .entry(key.to_string())
            .or_insert_with(|| {
                let db = DoubleBuffer::new(self.config.switch_threshold);
                Arc::new(db)
            })
            .clone()
    }
}

#[async_trait]
impl IdAlgorithm for SegmentAlgorithm {
    async fn generate(&self, ctx: &GenerateContext) -> Result<Id> {
        let key = format!("{}:{}", ctx.workspace_id, ctx.biz_tag);
        let buffer = self.get_or_create_buffer(&key);

        for _ in 0..3 {
            let current = buffer.get_current();

            if let Some((start, _end)) = current.try_consume(1) {
                self.metrics.total_generated.fetch_add(1, Ordering::Relaxed);
                // diting-perf C1 修复：cache_hits 递增，cache_hit_rate 才能正确反映命中率
                self.metrics.cache_hits.fetch_add(1, Ordering::Relaxed);
                return Ok(Id::from_u128(start.into()));
            }

            if buffer.need_switch() {
                let next = buffer.get_next();
                if next.is_some() {
                    buffer.swap();
                } else {
                    // diting-perf C2 修复：CAS 防止多线程同时 load_segment
                    if buffer.try_start_loading() {
                        self.metrics.cache_misses.fetch_add(1, Ordering::Relaxed);
                        let load_result = self.segment_loader.load_segment(ctx, 0).await;
                        buffer.finish_loading(); // 无论成功失败都重置 loading
                        let new_seg = load_result?;
                        let atomic_seg =
                            Arc::new(AtomicSegment::new(new_seg.start_id, new_seg.max_id));
                        buffer.set_next(atomic_seg);
                        buffer.swap();
                    } else {
                        // 另一线程正在加载，spin-wait 直到 loading 释放
                        while buffer.is_loading() {
                            std::hint::spin_loop();
                        }
                        // loading 释放后 next 可能已被设置，直接 swap；若仍未设置则重试循环
                        if buffer.get_next().is_some() {
                            buffer.swap();
                        }
                    }
                }
            }
        }

        self.metrics.total_failed.fetch_add(1, Ordering::Relaxed);
        let current = buffer.get_current();
        let segment = current.inner.lock();
        let max_id = segment.max_id.load(Ordering::Relaxed);
        Err(CoreError::SegmentExhausted { max_id })
    }

    async fn batch_generate(&self, ctx: &GenerateContext, size: usize) -> Result<IdBatch> {
        let mut ids = Vec::with_capacity(size);
        let key = format!("{}:{}", ctx.workspace_id, ctx.biz_tag);
        let buffer = self.get_or_create_buffer(&key);

        while ids.len() < size {
            let current = buffer.get_current();
            let remaining_needed = size - ids.len();

            if let Some((start, end)) = current.try_consume(remaining_needed as u64) {
                let count = (end - start) as usize;
                // ids.reserve(count) 已冗余：Vec::with_capacity(size) 已预分配，
                // 且 count <= remaining_needed = size - ids.len()
                ids.extend((start..end).map(|id| Id::from_u128(id.into())));
                self.metrics
                    .total_generated
                    .fetch_add(count as u64, Ordering::Relaxed);
                break;
            }

            if buffer.need_switch() {
                let next = buffer.get_next();
                if next.is_none() {
                    self.metrics.cache_misses.fetch_add(1, Ordering::Relaxed);
                    let new_seg = self.segment_loader.load_segment(ctx, 0).await?;
                    let atomic_seg = Arc::new(AtomicSegment::new(new_seg.start_id, new_seg.max_id));
                    buffer.set_next(atomic_seg);
                }
                buffer.swap();
            } else {
                break;
            }
        }

        if ids.is_empty() {
            let current = buffer.get_current();
            let segment = current.inner.lock();
            let max_id = segment.max_id.load(Ordering::Relaxed);
            drop(segment);

            self.metrics.total_failed.fetch_add(1, Ordering::Relaxed);
            return Err(CoreError::SegmentExhausted { max_id });
        }

        Ok(IdBatch::new(
            ids,
            AlgorithmType::Segment,
            ctx.biz_tag.clone(),
        ))
    }

    fn health_check(&self) -> HealthStatus {
        if self.buffers.read().is_empty() {
            return HealthStatus::Degraded("No active buffers".to_string());
        }
        HealthStatus::Healthy
    }

    fn metrics(&self) -> AlgorithmMetricsSnapshot {
        let hits = self.metrics.cache_hits.load(Ordering::Relaxed);
        let misses = self.metrics.cache_misses.load(Ordering::Relaxed);
        let total = hits + misses;
        let hit_rate = if total > 0 {
            hits as f64 / total as f64
        } else {
            1.0
        };

        AlgorithmMetricsSnapshot {
            total_generated: self.metrics.total_generated.load(Ordering::Relaxed),
            total_failed: self.metrics.total_failed.load(Ordering::Relaxed),
            // L15 修复：Segment 算法有段缓存，返回真实命中率。
            cache_hit_rate: Some(hit_rate),
            // 延迟分位数与时钟回拨计数由路由层观测后在
            // AlgorithmRouter::metrics() 合并填充（T021）。
            ..Default::default()
        }
    }

    fn algorithm_type(&self) -> AlgorithmType {
        AlgorithmType::Segment
    }

    // L13 修复：`initialize` 已移到 inherent impl（`impl SegmentAlgorithm`）。

    async fn shutdown(&self) -> Result<()> {
        // Signal shutdown and wait for health check task to complete
        let _ = self.shutdown_tx.send(true);
        if let Some(task) = self.health_check_task.lock().await.take() {
            let _ = task.await;
        }
        Ok(())
    }
}

#[derive(Default)]
struct DefaultSegmentLoader {}

#[async_trait]
impl SegmentLoader for DefaultSegmentLoader {
    async fn load_segment(&self, _ctx: &GenerateContext, _worker_id: u8) -> Result<SegmentData> {
        // Generate timestamp-based segment for uniqueness
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| {
                crate::core::CoreError::InternalError(format!("Failed to get system time: {}", e))
            })?
            .as_secs();
        let base_id = timestamp * 10000; // Use timestamp as base for uniqueness

        Ok(SegmentData {
            start_id: base_id,
            max_id: base_id + 1000000,
        })
    }
}

// ============================================================================
// ARCH-HIGH-001 修复：SegmentFactory impl 拆分到本文件。
// 原 impl 位于 traits.rs（违反规则 25），现移到具体类型所属文件。
// 通过 AlgorithmBuilder 的 pub(crate) 访问器获取依赖。
// ============================================================================
#[async_trait]
impl crate::core::algorithm::AlgorithmFactory for crate::core::algorithm::SegmentFactory {
    async fn build(
        &self,
        builder: &crate::core::algorithm::AlgorithmBuilder,
        config: &Config,
    ) -> Result<Box<dyn crate::core::algorithm::IdAlgorithm>> {
        let mut algo = SegmentAlgorithm::new(config.app.dc_id);
        #[cfg(feature = "etcd")]
        if let Some(ref monitor) = builder.etcd_health_monitor() {
            algo = algo.with_etcd_cluster_health_monitor(monitor.clone());
        }
        if let Some(ref cpu_monitor) = builder.cpu_monitor() {
            algo = algo.with_cpu_monitor(cpu_monitor.clone());
        }
        algo.initialize(config).await?;
        Ok(Box::new(algo))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::algorithm::AlgorithmFactory;

    #[test]
    fn test_atomic_segment_try_consume() {
        let segment = Arc::new(AtomicSegment::new(1, 1000));

        let (start, end) = segment.try_consume(10).unwrap();
        assert_eq!(start, 1);
        assert_eq!(end, 11);

        let (start, end) = segment.try_consume(5).unwrap();
        assert_eq!(start, 11);
        assert_eq!(end, 16);

        assert!(segment.try_consume(1000).is_none());
    }

    #[tokio::test]
    async fn test_segment_algorithm_generate() {
        let algo = SegmentAlgorithm::new(0);
        let ctx = GenerateContext {
            workspace_id: "test".to_string(),
            group_id: "test".to_string(),
            biz_tag: "test".to_string(),
            format: crate::core::types::IdFormat::Numeric,
            prefix: None,
        };

        let id = algo.generate(&ctx).await.unwrap();
        assert!(id.as_u128() > 0);
    }

    #[tokio::test]
    async fn test_segment_algorithm_shutdown() {
        let mut algo = SegmentAlgorithm::new(0);

        let config = Config::default();
        let _ = algo.initialize(&config).await;

        algo.shutdown().await.unwrap();
    }

    // CpuMonitor tests
    #[test]
    fn test_cpu_monitor_default_returns_default_usage() {
        let monitor = CpuMonitor::default();
        let usage = monitor.get_usage();
        assert!((usage - 0.1).abs() < f64::EPSILON);
    }

    #[test]
    fn test_cpu_monitor_new_initializes_with_default_usage() {
        let monitor = CpuMonitor::new();
        let usage = monitor.get_usage();
        assert!((usage - 0.1).abs() < f64::EPSILON);
    }

    #[test]
    fn test_cpu_monitor_update_usage_changes_value() {
        let monitor = CpuMonitor::new();
        monitor.update_usage(0.5);
        let usage = monitor.get_usage();
        assert!((usage - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_cpu_monitor_update_usage_clamps_high_values() {
        let monitor = CpuMonitor::new();
        monitor.update_usage(2.0);
        let usage = monitor.get_usage();
        assert!((usage - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_cpu_monitor_update_usage_clamps_negative_values() {
        let monitor = CpuMonitor::new();
        monitor.update_usage(-0.5);
        let usage = monitor.get_usage();
        assert!(usage.abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn test_cpu_monitor_start_monitoring_completes_on_non_linux() {
        let monitor = CpuMonitor::new();
        let handle = monitor.start_monitoring();
        tokio::time::sleep(Duration::from_millis(10)).await;
        handle.abort();
    }

    // DoubleBuffer tests
    #[test]
    fn test_double_buffer_swap_returns_none_when_no_next() {
        let db = DoubleBuffer::new(0.1);
        let result = db.swap();
        assert!(result.is_none());
    }

    #[test]
    fn test_double_buffer_need_switch_when_total_zero() {
        let db = DoubleBuffer::new(0.1);
        assert!(db.need_switch());
    }

    #[test]
    fn test_double_buffer_get_next_returns_set_segment() {
        let db = DoubleBuffer::new(0.1);
        assert!(db.get_next().is_none());

        let next = Arc::new(AtomicSegment::new(100, 200));
        db.set_next(next);

        let retrieved = db.get_next();
        assert!(retrieved.is_some());
        let binding = retrieved.unwrap();
        let seg = binding.inner.lock();
        assert_eq!(seg.start_id.load(Ordering::Relaxed), 100);
    }

    // SegmentAlgorithm: cpu monitor / health check / metrics / trait methods
    #[tokio::test]
    async fn test_segment_algorithm_with_cpu_monitor_sets_field() {
        let monitor = Arc::new(CpuMonitor::new());
        let mut algo = SegmentAlgorithm::new(0).with_cpu_monitor(monitor);
        let config = Config::default();
        algo.initialize(&config).await.unwrap();
        algo.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn test_segment_algorithm_initialize_starts_health_check_task() {
        let mut algo = SegmentAlgorithm::new(0);
        let config = Config::default();
        algo.initialize(&config).await.unwrap();
        assert!(algo.health_check_task.lock().await.is_some());
        algo.shutdown().await.unwrap();
        assert!(algo.health_check_task.lock().await.is_none());
    }

    #[tokio::test]
    async fn test_segment_algorithm_initialize_with_cpu_monitor_starts_monitoring() {
        let monitor = Arc::new(CpuMonitor::new());
        let mut algo = SegmentAlgorithm::new(0).with_cpu_monitor(monitor);
        let config = Config::default();
        algo.initialize(&config).await.unwrap();
        assert!(algo.cpu_monitor_task.lock().await.is_some());
        algo.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn test_segment_algorithm_batch_generate_basic_path() {
        let algo = SegmentAlgorithm::new(0);
        let ctx = sample_ctx();
        let batch = algo.batch_generate(&ctx, 5).await.unwrap();
        assert_eq!(batch.ids.len(), 5);
        let first = batch.ids[0].as_u128();
        for (i, id) in batch.ids.iter().enumerate() {
            assert_eq!(id.as_u128(), first + i as u128);
        }
        assert_eq!(batch.algorithm, AlgorithmType::Segment);
        assert_eq!(batch.biz_tag, "tag");
    }

    #[tokio::test]
    async fn test_segment_algorithm_batch_generate_empty_returns_exhausted_error() {
        let algo = SegmentAlgorithm::new(0);
        let ctx = sample_ctx();
        let result = algo.batch_generate(&ctx, 0).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            CoreError::SegmentExhausted { max_id: _ } => {}
            other => panic!("expected SegmentExhausted, got {:?}", other),
        }
    }

    #[test]
    fn test_segment_algorithm_health_check_returns_degraded_when_no_buffers() {
        let algo = SegmentAlgorithm::new(0);
        let status = algo.health_check();
        match status {
            HealthStatus::Degraded(msg) => assert_eq!(msg, "No active buffers"),
            other => panic!("expected Degraded, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_segment_algorithm_health_check_returns_healthy_when_buffer_exists() {
        let algo = SegmentAlgorithm::new(0);
        let ctx = sample_ctx();
        let _ = algo.generate(&ctx).await.unwrap();
        let status = algo.health_check();
        assert!(matches!(status, HealthStatus::Healthy));
    }

    #[test]
    fn test_segment_algorithm_metrics_default_returns_full_hit_rate() {
        let algo = SegmentAlgorithm::new(0);
        let m = algo.metrics();
        assert_eq!(m.cache_hit_rate, Some(1.0));
        assert_eq!(m.total_generated, 0);
        assert_eq!(m.total_failed, 0);
    }

    #[tokio::test]
    async fn test_segment_algorithm_metrics_with_cache_misses_records_qps_zero() {
        let algo = SegmentAlgorithm::new(0);
        let ctx = sample_ctx();
        let _ = algo.generate(&ctx).await.unwrap();
        let m = algo.metrics();
        assert_eq!(m.total_generated, 1);
        assert_eq!(m.cache_hit_rate, Some(0.5));
        assert_eq!(m.current_qps, 0);
        assert_eq!(m.p50_latency_us, 0);
        assert_eq!(m.p99_latency_us, 0);
    }

    #[test]
    fn test_segment_algorithm_algorithm_type_returns_segment() {
        let algo = SegmentAlgorithm::new(0);
        assert_eq!(algo.algorithm_type(), AlgorithmType::Segment);
    }

    #[tokio::test]
    async fn test_segment_algorithm_shutdown_without_initialize_is_no_op() {
        let algo = SegmentAlgorithm::new(0);
        algo.shutdown().await.unwrap();
    }

    // SegmentFactory tests
    #[tokio::test]
    async fn test_segment_factory_build_creates_working_algorithm() {
        let factory = crate::core::algorithm::SegmentFactory;
        let builder = crate::core::algorithm::AlgorithmBuilder::new(AlgorithmType::Segment);
        let config = Config::default();
        let algo = factory.build(&builder, &config).await.unwrap();
        assert_eq!(algo.algorithm_type(), AlgorithmType::Segment);
    }

    #[tokio::test]
    async fn test_segment_factory_build_with_cpu_monitor() {
        let factory = crate::core::algorithm::SegmentFactory;
        let cpu = Arc::new(CpuMonitor::new());
        let builder = crate::core::algorithm::AlgorithmBuilder::new(AlgorithmType::Segment)
            .with_cpu_monitor(cpu);
        let config = Config::default();
        let algo = factory.build(&builder, &config).await.unwrap();
        assert_eq!(algo.algorithm_type(), AlgorithmType::Segment);
    }

    fn sample_ctx() -> GenerateContext {
        GenerateContext {
            workspace_id: "ws".to_string(),
            group_id: "g".to_string(),
            biz_tag: "tag".to_string(),
            format: crate::core::types::IdFormat::Numeric,
            prefix: None,
        }
    }
}
