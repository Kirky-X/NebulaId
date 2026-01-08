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

#![allow(dead_code)]

use crate::algorithm::{AlgorithmMetricsSnapshot, GenerateContext, HealthStatus, IdAlgorithm};
use crate::config::{Config, SegmentAlgorithmConfig};
#[cfg(feature = "etcd")]
use crate::coordinator::EtcdClusterHealthMonitor;
use crate::database::SegmentRepository;
use crate::types::{AlgorithmType, CoreError, Id, IdBatch, Result};
use async_trait::async_trait;
use dashmap::DashMap;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio::time::sleep;
use tracing::{info, warn};

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
            debug!("CPU monitoring not supported on this platform, using default value");
        })
    }
}

const FAILURE_THRESHOLD_DEGRADED: u64 = 3;
const FAILURE_THRESHOLD_FAILED: u64 = 5;
const DEFAULT_QPS_BASELINE: u64 = 1000;

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
    pub failure_count: AtomicU64,
    pub consecutive_failures: AtomicU64,
}

impl DcHealthState {
    pub fn new(dc_id: u8) -> Self {
        Self {
            dc_id,
            status: AtomicU8::new(DcStatus::Healthy as u8),
            last_success: Arc::new(Mutex::new(Instant::now())),
            failure_count: AtomicU64::new(0),
            consecutive_failures: AtomicU64::new(0),
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

    pub fn record_success(&self) {
        *self.last_success.lock() = Instant::now();
        self.consecutive_failures.store(0, Ordering::Relaxed);
        if self.get_status() != DcStatus::Healthy {
            self.set_status(DcStatus::Healthy);
            info!("DC {} recovered to healthy state", self.dc_id);
        }
    }

    pub fn record_failure(&self) {
        self.failure_count.fetch_add(1, Ordering::Relaxed);
        let consecutive = self.consecutive_failures.fetch_add(1, Ordering::Relaxed) + 1;

        if consecutive >= 5 {
            self.set_status(DcStatus::Failed);
            warn!(
                "DC {} marked as failed after {} consecutive failures",
                self.dc_id, consecutive
            );
        } else if consecutive >= 3 {
            self.set_status(DcStatus::Degraded);
            warn!(
                "DC {} marked as degraded after {} consecutive failures",
                self.dc_id, consecutive
            );
        }
    }

    pub fn should_use_dc(&self) -> bool {
        matches!(self.get_status(), DcStatus::Healthy | DcStatus::Degraded)
    }
}

/// 动态步长计算器
/// 基于 QPS 和系统负载自动调整号段步长
///
/// 动态步长计算公式:
/// ```text
/// next_step = base_step * (1 + alpha * velocity) * (1 + beta * pressure)
///
/// 其中:
/// - velocity = current_qps / step
/// - pressure = cpu_usage (0-1)
/// - alpha = 0.5 (速率因子)
/// - beta = 0.3 (压力因子)
///
/// 边界控制:
/// - min_step = base_step * 0.5
/// - max_step = base_step * 100
/// ```
#[derive(Debug, Clone)]
pub struct StepCalculator {
    /// 速率因子 (α)
    velocity_factor: f64,
    /// 压力因子 (β)
    pressure_factor: f64,
    /// CPU 使用率监控器 (可选)
    cpu_monitor: Option<Arc<CpuMonitor>>,
}

impl Default for StepCalculator {
    fn default() -> Self {
        Self {
            velocity_factor: 0.5,
            pressure_factor: 0.3,
            cpu_monitor: None,
        }
    }
}

impl StepCalculator {
    /// 创建步长计算器
    pub fn new(velocity_factor: f64, pressure_factor: f64) -> Self {
        Self {
            velocity_factor,
            pressure_factor,
            cpu_monitor: None,
        }
    }

    /// 设置 CPU 监控器
    pub fn with_cpu_monitor(mut self, cpu_monitor: Arc<CpuMonitor>) -> Self {
        self.cpu_monitor = Some(cpu_monitor);
        self
    }

    /// 获取 CPU 使用率 (优先使用监控器，否则返回默认值)
    fn get_cpu_usage(&self) -> f64 {
        if let Some(ref monitor) = self.cpu_monitor {
            monitor.get_usage()
        } else {
            DEFAULT_CPU_USAGE
        }
    }

    /// 计算动态步长
    ///
    /// # Arguments
    /// * `qps` - 当前 QPS
    /// * `current_step` - 当前步长
    /// * `config` - Segment 算法配置
    ///
    /// # Returns
    /// 计算后的步长值
    pub fn calculate(&self, qps: u64, current_step: u64, config: &SegmentAlgorithmConfig) -> u64 {
        // 避免除零
        let step = if current_step == 0 {
            config.base_step
        } else {
            current_step
        };

        // 计算速率 (velocity = qps / step)
        let velocity = qps as f64 / step as f64;

        // 获取系统压力 (CPU 使用率)
        let pressure = self.get_cpu_usage();

        // 计算步长
        let next_step = config.base_step as f64
            * (1.0 + self.velocity_factor * velocity)
            * (1.0 + self.pressure_factor * pressure);

        // 应用边界控制
        let min_step = (config.base_step as f64 * 0.5).max(config.min_step as f64);
        let max_step = (config.base_step as f64 * 100.0).min(config.max_step as f64);

        next_step.clamp(min_step, max_step).round() as u64
    }

    /// 获取建议的步长调整方向
    ///
    /// # Arguments
    /// * `qps` - 当前 QPS
    /// * `current_step` - 当前步长
    /// * `config` - Segment 算法配置
    ///
    /// # Returns
    /// "up" 表示建议增大步长, "down" 表示建议减小步长, "stable" 表示保持稳定
    pub fn get_adjustment_direction(
        &self,
        qps: u64,
        current_step: u64,
        config: &SegmentAlgorithmConfig,
    ) -> &'static str {
        let target_step = self.calculate(qps, current_step, config);

        let ratio = target_step as f64 / current_step as f64;
        if ratio > 1.2 {
            "up"
        } else if ratio < 0.8 {
            "down"
        } else {
            "stable"
        }
    }
}

pub struct DcFailureDetector {
    dc_states: DashMap<u8, Arc<DcHealthState>>,
    failure_threshold: u64,
    recovery_timeout: Duration,
}

impl DcFailureDetector {
    pub fn new(failure_threshold: u64, recovery_timeout: Duration) -> Self {
        Self {
            dc_states: DashMap::new(),
            failure_threshold,
            recovery_timeout,
        }
    }

    pub fn add_dc(&self, dc_id: u8) {
        self.dc_states
            .entry(dc_id)
            .or_insert_with(|| Arc::new(DcHealthState::new(dc_id)));
    }

    pub fn get_dc_state(&self, dc_id: u8) -> Option<Arc<DcHealthState>> {
        self.dc_states.get(&dc_id).map(|v| Arc::clone(v.value()))
    }

    pub fn get_healthy_dcs(&self) -> Vec<u8> {
        self.dc_states
            .iter()
            .filter(|entry| entry.value().should_use_dc())
            .map(|entry| *entry.key())
            .collect()
    }

    pub fn select_best_dc(&self, preferred_dc: u8) -> u8 {
        let state = self.get_dc_state(preferred_dc);
        if let Some(s) = state {
            if s.should_use_dc() {
                return preferred_dc;
            }
        }

        let healthy_dcs = self.get_healthy_dcs();
        if !healthy_dcs.is_empty() {
            return healthy_dcs[0];
        }

        preferred_dc
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
                        info!("Health check task received shutdown signal");
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
        for entry in self.dc_states.iter() {
            let state = entry.value();
            if state.get_status() == DcStatus::Failed {
                let last_success = *state.last_success.lock();
                if now.duration_since(last_success) > self.recovery_timeout {
                    info!("Attempting recovery for DC {}", state.dc_id);
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
    pub step: AtomicU64,
    pub version: AtomicU8,
}

impl Segment {
    pub fn new(start_id: u64, max_id: u64, step: u64) -> Self {
        Self {
            start_id: AtomicU64::new(start_id),
            max_id: AtomicU64::new(max_id),
            current_id: AtomicU64::new(start_id),
            step: AtomicU64::new(step),
            version: AtomicU8::new(0),
        }
    }

    pub fn remaining(&self) -> u64 {
        let current = self.current_id.load(Ordering::Relaxed);
        let max = self.max_id.load(Ordering::Relaxed);
        max.saturating_sub(current)
    }

    pub fn consumed(&self) -> u64 {
        let start = self.start_id.load(Ordering::Relaxed);
        let current = self.current_id.load(Ordering::Relaxed);
        current.saturating_sub(start)
    }
}

pub struct AtomicSegment {
    pub inner: Mutex<Segment>,
}

impl AtomicSegment {
    pub fn new(start_id: u64, max_id: u64, step: u64) -> Self {
        Self {
            inner: Mutex::new(Segment::new(start_id, max_id, step)),
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

    pub fn remaining(&self) -> u64 {
        self.inner.lock().remaining()
    }
}

pub struct DoubleBuffer {
    current: Arc<Mutex<Arc<AtomicSegment>>>,
    next: Arc<Mutex<Option<Arc<AtomicSegment>>>>,
    switch_threshold: f64,
    #[allow(dead_code)]
    loader_tx: mpsc::Sender<()>,
}

impl DoubleBuffer {
    pub fn new(switch_threshold: f64) -> (Self, mpsc::Receiver<()>) {
        let (loader_tx, loader_rx) = mpsc::channel(1);

        let initial_segment = Arc::new(AtomicSegment::new(0, 0, 0));
        let current = Arc::new(Mutex::new(initial_segment));
        let next = Arc::new(Mutex::new(None));

        let db = Self {
            current,
            next,
            switch_threshold,
            loader_tx,
        };

        (db, loader_rx)
    }

    pub fn set_current(&self, segment: Arc<AtomicSegment>) {
        let mut current_guard = self.current.lock();
        *current_guard = segment;
    }

    pub fn set_next(&self, segment: Arc<AtomicSegment>) {
        let mut next_guard = self.next.lock();
        *next_guard = Some(segment);
    }

    pub fn get_next(&self) -> Option<Arc<AtomicSegment>> {
        let next_guard = self.next.lock();
        next_guard.clone()
    }

    pub fn swap(&self) -> Option<Arc<AtomicSegment>> {
        let mut next_guard = self.next.lock();
        let new_current = next_guard.take();

        if let Some(ref new_current) = new_current {
            let mut current_guard = self.current.lock();
            *current_guard = new_current.clone();
        }

        new_current
    }

    pub fn need_switch(&self) -> bool {
        let current_guard = self.current.lock();
        let remaining = current_guard.remaining();
        let total = {
            let segment = current_guard.inner.lock();
            segment.max_id.load(Ordering::Relaxed) - segment.start_id.load(Ordering::Relaxed)
        };

        drop(current_guard);

        if total == 0 {
            return true;
        }

        (remaining as f64 / total as f64) < self.switch_threshold
    }

    pub fn get_current(&self) -> Arc<AtomicSegment> {
        let current_guard = self.current.lock();
        current_guard.clone()
    }
}

pub struct SegmentAlgorithm {
    config: SegmentAlgorithmConfig,
    buffers: DashMap<String, Arc<DoubleBuffer>>,
    metrics: Arc<AlgorithmMetricsInner>,
    segment_loader: Arc<dyn SegmentLoader + Send + Sync>,
    dc_failure_detector: Arc<DcFailureDetector>,
    #[allow(dead_code)]
    local_dc_id: u8,
    #[cfg(feature = "etcd")]
    etcd_cluster_health_monitor: Option<Arc<EtcdClusterHealthMonitor>>,
    #[cfg(not(feature = "etcd"))]
    etcd_cluster_health_monitor: Option<()>,
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
    pub step: u64,
    pub version: u8,
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
            buffers: DashMap::new(),
            metrics: Arc::new(AlgorithmMetricsInner::default()),
            segment_loader: Arc::new(DefaultSegmentLoader::default()),
            dc_failure_detector,
            local_dc_id,
            etcd_cluster_health_monitor: None,
            cpu_monitor: None,
            cpu_monitor_task: Arc::new(tokio::sync::Mutex::new(None)),
            shutdown_tx: Arc::new(shutdown_tx),
            health_check_task: Arc::new(tokio::sync::Mutex::new(None)),
        }
    }

    pub fn with_loader(mut self, loader: Arc<dyn SegmentLoader + Send + Sync>) -> Self {
        self.segment_loader = loader;
        self
    }

    pub fn with_cpu_monitor(mut self, cpu_monitor: Arc<CpuMonitor>) -> Self {
        self.cpu_monitor = Some(cpu_monitor);
        self
    }

    pub fn with_dc_failure_detector(mut self, detector: Arc<DcFailureDetector>) -> Self {
        self.dc_failure_detector = detector;
        self
    }

    #[cfg(feature = "etcd")]
    pub fn with_etcd_cluster_health_monitor(
        mut self,
        monitor: Arc<EtcdClusterHealthMonitor>,
    ) -> Self {
        self.etcd_cluster_health_monitor = Some(monitor);
        self
    }

    #[cfg(not(feature = "etcd"))]
    pub fn with_etcd_cluster_health_monitor(mut self, _monitor: Arc<()>) -> Self {
        self.etcd_cluster_health_monitor = Some(());
        self
    }

    #[cfg(feature = "etcd")]
    pub fn get_etcd_cluster_health_monitor(&self) -> Option<&Arc<EtcdClusterHealthMonitor>> {
        self.etcd_cluster_health_monitor.as_ref()
    }

    #[cfg(not(feature = "etcd"))]
    pub fn get_etcd_cluster_health_monitor(&self) -> Option<&()> {
        self.etcd_cluster_health_monitor.as_ref()
    }

    pub fn get_dc_failure_detector(&self) -> &Arc<DcFailureDetector> {
        &self.dc_failure_detector
    }

    fn get_or_create_buffer(&self, key: &str) -> Arc<DoubleBuffer> {
        self.buffers
            .entry(key.to_string())
            .or_insert_with(|| {
                let (db, _) = DoubleBuffer::new(self.config.switch_threshold);
                Arc::new(db)
            })
            .value()
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
                return Ok(Id::from_u128(start.into()));
            }

            if buffer.need_switch() {
                let next = buffer.get_next();
                if next.is_some() {
                    buffer.swap();
                } else {
                    self.metrics.cache_misses.fetch_add(1, Ordering::Relaxed);
                    let new_seg = self.segment_loader.load_segment(ctx, 0).await?;
                    let atomic_seg = Arc::new(AtomicSegment::new(
                        new_seg.start_id,
                        new_seg.max_id,
                        new_seg.step,
                    ));
                    buffer.set_next(atomic_seg);
                    buffer.swap();
                }
            }
        }

        self.metrics.total_failed.fetch_add(1, Ordering::Relaxed);
        Err(CoreError::SegmentExhausted { max_id: 0 })
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
                ids.reserve(count);
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
                    let atomic_seg = Arc::new(AtomicSegment::new(
                        new_seg.start_id,
                        new_seg.max_id,
                        new_seg.step,
                    ));
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
        if self.buffers.is_empty() {
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
            current_qps: 0,
            p50_latency_us: 0,
            p99_latency_us: 0,
            cache_hit_rate: hit_rate,
        }
    }

    fn algorithm_type(&self) -> AlgorithmType {
        AlgorithmType::Segment
    }

    async fn initialize(&mut self, config: &Config) -> Result<()> {
        self.config = config.algorithm.segment.clone();

        // Start CPU monitoring if available
        if let Some(ref cpu_monitor) = self.cpu_monitor {
            info!("Starting CPU monitoring task");
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

    async fn shutdown(&self) -> Result<()> {
        // Signal shutdown and wait for health check task to complete
        let _ = self.shutdown_tx.send(true);
        if let Some(task) = self.health_check_task.lock().await.take() {
            let _ = task.await;
        }
        Ok(())
    }
}

struct DefaultSegmentLoader {
    counter: AtomicU64,
}

impl Default for DefaultSegmentLoader {
    fn default() -> Self {
        Self {
            counter: AtomicU64::new(0),
        }
    }
}

#[async_trait]
impl SegmentLoader for DefaultSegmentLoader {
    async fn load_segment(&self, _ctx: &GenerateContext, _worker_id: u8) -> Result<SegmentData> {
        // Generate timestamp-based segment for uniqueness
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| crate::CoreError::InternalError(format!("Failed to get system time: {}", e)))?
            .as_secs();
        let base_id = timestamp * 10000; // Use timestamp as base for uniqueness

        Ok(SegmentData {
            start_id: base_id,
            max_id: base_id + 1000000,
            step: 1000,
            version: 0,
        })
    }
}

pub struct DatabaseSegmentLoader {
    repository: Arc<dyn SegmentRepository>,
    dc_failure_detector: Arc<DcFailureDetector>,
    local_dc_id: u8,
    #[cfg(feature = "etcd")]
    etcd_cluster_health_monitor: Option<Arc<EtcdClusterHealthMonitor>>,
    #[cfg(not(feature = "etcd"))]
    etcd_cluster_health_monitor: Option<()>,
    /// 动态步长计算器
    step_calculator: StepCalculator,
    /// Segment 算法配置
    segment_config: SegmentAlgorithmConfig,
}

impl DatabaseSegmentLoader {
    pub fn new(
        repository: Arc<dyn SegmentRepository>,
        dc_failure_detector: Arc<DcFailureDetector>,
        local_dc_id: u8,
        config: SegmentAlgorithmConfig,
    ) -> Self {
        Self {
            repository,
            dc_failure_detector,
            local_dc_id,
            etcd_cluster_health_monitor: None,
            step_calculator: StepCalculator::default(),
            segment_config: config,
        }
    }

    pub fn with_cpu_monitor(mut self, cpu_monitor: Arc<CpuMonitor>) -> Self {
        self.step_calculator = self.step_calculator.with_cpu_monitor(cpu_monitor);
        self
    }

    #[cfg(feature = "etcd")]
    pub fn with_etcd_cluster_health_monitor(
        mut self,
        monitor: Arc<EtcdClusterHealthMonitor>,
    ) -> Self {
        self.etcd_cluster_health_monitor = Some(monitor);
        self
    }

    #[cfg(not(feature = "etcd"))]
    pub fn with_etcd_cluster_health_monitor(mut self, _monitor: Arc<()>) -> Self {
        self.etcd_cluster_health_monitor = Some(());
        self
    }

    /// 动态计算步长
    ///
    /// 根据当前 QPS 计算合适的步长
    ///
    /// # Arguments
    /// * `qps` - 当前 QPS
    ///
    /// # Returns
    /// 计算后的步长值
    fn calculate_step(&self, qps: u64) -> u64 {
        self.step_calculator
            .calculate(qps, self.segment_config.base_step, &self.segment_config)
    }

    /// 获取当前步长（用于测试）
    pub fn get_current_step(&self) -> u64 {
        self.segment_config.base_step
    }

    /// 获取当前 QPS
    /// TODO: 集成实际监控系统获取真实 QPS 值
    fn get_current_qps(&self) -> u64 {
        // 当前返回基准 QPS，后续应从监控系统获取实际值
        // 默认假设基准 QPS，作为动态调整的基准
        DEFAULT_QPS_BASELINE
    }
}

#[async_trait]
impl SegmentLoader for DatabaseSegmentLoader {
    async fn load_segment(&self, ctx: &GenerateContext, _worker_id: u8) -> Result<SegmentData> {
        // 获取当前 QPS (简化处理，实际应从监控获取)
        let current_qps = self.get_current_qps();
        let step = self.calculate_step(current_qps);

        tracing::debug!(
            "Loading segment for {} with dynamic step: {} (QPS: {})",
            ctx.biz_tag,
            step,
            current_qps
        );
        let dc_id = self.dc_failure_detector.select_best_dc(self.local_dc_id);
        let dc_state = self.dc_failure_detector.get_dc_state(dc_id);
        let dc_state_clone = dc_state.clone();

        let segment = if dc_state.is_some() {
            self.repository
                .allocate_segment_with_dc(
                    &ctx.workspace_id,
                    &ctx.biz_tag,
                    step as i32,
                    dc_id as i32,
                )
                .await
                .map_err(|e| {
                    if let Some(state) = dc_state_clone {
                        state.record_failure();
                    }
                    CoreError::DatabaseError(e.to_string())
                })?
        } else {
            self.repository
                .allocate_segment(&ctx.workspace_id, &ctx.biz_tag, step as i32)
                .await
                .map_err(|e| CoreError::DatabaseError(e.to_string()))?
        };

        if let Some(state) = dc_state {
            state.record_success();
        }

        Ok(SegmentData {
            start_id: segment.current_id as u64,
            max_id: segment.max_id as u64,
            step: segment.step as u64,
            version: 0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_atomic_segment_try_consume() {
        let segment = Arc::new(AtomicSegment::new(1, 1000, 100));

        let (start, end) = segment.try_consume(10).unwrap();
        assert_eq!(start, 1);
        assert_eq!(end, 11);

        let (start, end) = segment.try_consume(5).unwrap();
        assert_eq!(start, 11);
        assert_eq!(end, 16);

        assert!(segment.try_consume(1000).is_none());
    }

    #[test]
    fn test_segment_remaining() {
        let segment = Segment::new(0, 1000, 100);
        assert_eq!(segment.remaining(), 1000);

        segment.current_id.store(500, Ordering::Relaxed);
        assert_eq!(segment.remaining(), 500);
    }

    #[tokio::test]
    async fn test_segment_algorithm_generate() {
        let algo = SegmentAlgorithm::new(0);
        let ctx = GenerateContext {
            workspace_id: "test".to_string(),
            group_id: "test".to_string(),
            biz_tag: "test".to_string(),
            format: crate::types::IdFormat::Numeric,
            prefix: None,
        };

        let id = algo.generate(&ctx).await.unwrap();
        assert!(id.as_u128() > 0);
    }

    #[tokio::test]
    async fn test_dc_failure_detector() {
        let detector = DcFailureDetector::new(5, Duration::from_secs(300));
        detector.add_dc(0);
        detector.add_dc(1);

        // Test selecting best DC
        let best = detector.select_best_dc(0);
        assert_eq!(best, 0);

        // Test get healthy DCs
        let healthy = detector.get_healthy_dcs();
        assert!(healthy.contains(&0));
    }

    #[tokio::test]
    async fn test_dc_health_state() {
        let state = DcHealthState::new(1);

        assert_eq!(state.get_status(), DcStatus::Healthy);
        assert!(state.should_use_dc());

        state.record_failure();
        state.record_failure();
        state.record_failure();
        assert_eq!(state.get_status(), DcStatus::Degraded);
        assert!(state.should_use_dc());

        state.record_failure();
        state.record_failure();
        assert_eq!(state.get_status(), DcStatus::Failed);
        assert!(!state.should_use_dc());

        state.record_success();
        assert_eq!(state.get_status(), DcStatus::Healthy);
    }

    #[test]
    fn test_step_calculator_qps_based() {
        let calculator = StepCalculator::new(0.5, 0.5);
        let config = SegmentAlgorithmConfig::default();
        let step = calculator.calculate(1000, 100, &config);
        assert!(step >= 100);
        assert!(step <= 1000000);
    }

    #[tokio::test]
    async fn test_segment_algorithm_shutdown() {
        let mut algo = SegmentAlgorithm::new(0);

        // Initialize algorithm
        let config = Config::default();
        let _ = algo.initialize(&config).await;

        // Shutdown should complete without hanging
        algo.shutdown().await.unwrap();
    }
}
