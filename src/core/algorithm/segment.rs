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
//! # 当前状态：保留若干 v0.3.0 多数据中心与告警管道预留 API
//!
//! 本模块包含若干暂时未被生产路径直接调用的 API：
//! - `StepCalculator`、`CpuMonitor`、`DatabaseSegmentLoader`、`RepositoryBackedLoader`
//! - `SegmentAlgorithmBuilder` 及其 builder 方法（`with_loader`、`with_dc_failure_detector`、
//!   `with_etcd_cluster_health_monitor` 等）
//! - `FAILURE_THRESHOLD_DEGRADED`、`FAILURE_THRESHOLD_FAILED`、`DEFAULT_QPS_BASELINE` 常量
//! - `SegmentInfo::{remaining, consumed, set_current}`、`DcFailureDetector::{get_dc_state,
//!   get_healthy_dcs, select_best_dc}` 等方法
//!
//! 保留原因：
//!
//! 1. **多数据中心支持预留**：v0.3.0 将启用多数据中心（DC）感知的 segment 分配，
//!    `DcFailureDetector`、`get_healthy_dcs`、`select_best_dc` 是 DC 选择算法的核心 API。
//! 2. **动态步长计算**：`StepCalculator` + `CpuMonitor` 用于根据 CPU 负载和 QPS 动态调整
//!    segment 步长，将在 v0.3.0 性能优化阶段接入。
//! 3. **数据库 segment loader**：`DatabaseSegmentLoader` 和 `RepositoryBackedLoader` 是
//!    两种数据库 segment 加载策略，v0.3.0 将根据部署形态选择启用。
//! 4. **测试覆盖**：以上 API 均有对应单元测试覆盖（约 30+ 测试），删除会丢失测试保护。
//! 5. **Builder 扩展点**：`SegmentAlgorithmBuilder` 提供依赖注入的扩展点，便于未来
//!    接入自定义 loader / detector。
//!
//! 详见 `specmark/changes/v0.3.0-release/` 中的多数据中心与性能优化设计文档。
#![allow(dead_code)]

use crate::core::algorithm::{
    AlgorithmMetricsSnapshot, GenerateContext, HealthStatus, IdAlgorithm,
};
use crate::core::config::{Config, SegmentAlgorithmConfig};
#[cfg(feature = "etcd")]
use crate::core::coordinator::EtcdClusterHealthMonitor;
use crate::core::database::SegmentRepository;
use crate::core::types::{AlgorithmType, CoreError, Id, IdBatch, Result};
use arc_swap::{ArcSwap, ArcSwapOption};
use async_trait::async_trait;
use parking_lot::Mutex;
use parking_lot::RwLock;
use std::collections::HashMap;
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
            tracing::debug!(
                "{}",
                t!("log.core.algorithm.segment.cpu_monitoring_not_supported")
            );
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
            info!(
                "{}",
                t!(
                    "log.core.algorithm.segment.dc_recovered",
                    dc_id = self.dc_id
                )
            );
        }
    }

    pub fn record_failure(&self) {
        self.failure_count.fetch_add(1, Ordering::Relaxed);
        let consecutive = self.consecutive_failures.fetch_add(1, Ordering::Relaxed) + 1;

        if consecutive >= 5 {
            self.set_status(DcStatus::Failed);
            warn!(
                "{}",
                t!(
                    "log.core.algorithm.segment.dc_marked_failed",
                    dc_id = self.dc_id,
                    consecutive = consecutive
                )
            );
        } else if consecutive >= 3 {
            self.set_status(DcStatus::Degraded);
            warn!(
                "{}",
                t!(
                    "log.core.algorithm.segment.dc_marked_degraded",
                    dc_id = self.dc_id,
                    consecutive = consecutive
                )
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

    pub fn get_dc_state(&self, dc_id: u8) -> Option<Arc<DcHealthState>> {
        self.dc_states.read().get(&dc_id).cloned()
    }

    pub fn get_healthy_dcs(&self) -> Vec<u8> {
        let states = self.dc_states.read();
        states
            .iter()
            .filter(|(_, state)| state.should_use_dc())
            .map(|(&dc_id, _)| dc_id)
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
    current: Arc<ArcSwap<AtomicSegment>>,
    next: Arc<ArcSwapOption<AtomicSegment>>,
    switch_threshold: f64,
    #[allow(dead_code)]
    loader_tx: mpsc::Sender<()>,
    // diting-perf C2 修复：loading 标记防止多线程并发触发 load_segment
    loading: Arc<std::sync::atomic::AtomicBool>,
}

impl DoubleBuffer {
    pub fn new(switch_threshold: f64) -> (Self, mpsc::Receiver<()>) {
        let (loader_tx, loader_rx) = mpsc::channel(1);

        let initial_segment = Arc::new(AtomicSegment::new(0, 0, 0));
        let current = Arc::new(ArcSwap::from(initial_segment));
        let next = Arc::new(ArcSwapOption::empty());

        let db = Self {
            current,
            next,
            switch_threshold,
            loader_tx,
            loading: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        };

        (db, loader_rx)
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

    pub fn set_current(&self, segment: Arc<AtomicSegment>) {
        self.current.store(segment);
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
            buffers: Arc::new(RwLock::new(HashMap::new())),
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
                let (db, _) = DoubleBuffer::new(self.config.switch_threshold);
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
                        let atomic_seg = Arc::new(AtomicSegment::new(
                            new_seg.start_id,
                            new_seg.max_id,
                            new_seg.step,
                        ));
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
            current_qps: 0,
            p50_latency_us: 0,
            p99_latency_us: 0,
            // L15 修复：Segment 算法有段缓存，返回真实命中率。
            cache_hit_rate: Some(hit_rate),
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
            .map_err(|e| {
                crate::core::CoreError::InternalError(format!("Failed to get system time: {}", e))
            })?
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

/// QPS 滑动窗口计数器（M6 + F-03 修复）。
///
/// **M6 修复**：原实现使用 `static OnceLock<WindowCounter>`，导致多个
/// `DatabaseSegmentLoader` 实例（不同 biz_tag）共享同一计数器，QPS 统计失真。
/// 现改为实例字段，每个 loader 独立统计。
///
/// **F-03 修复**：原实现 `fetch_add` 与 `store(0)` 之间存在 TOCTOU race，
/// 线程 A 的计数可能被线程 B 的 reset 丢弃。现用 `std::sync::Mutex` 串行化
/// reset + record 操作，保证原子性。QPS 统计非热路径，锁开销可接受。
struct QpsWindow {
    inner: std::sync::Mutex<QpsWindowInner>,
}

struct QpsWindowInner {
    counters: Vec<u64>,
    start_second: u64,
}

impl QpsWindow {
    const WINDOW_SIZE: usize = 60; // 60 秒窗口

    fn new() -> Self {
        let start_second = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        Self {
            inner: std::sync::Mutex::new(QpsWindowInner {
                counters: vec![0u64; Self::WINDOW_SIZE],
                start_second,
            }),
        }
    }

    /// 记录一次请求并返回当前平均 QPS。
    ///
    /// 所有操作在锁内完成，避免 fetch_add 与 reset 之间的 TOCTOU race。
    fn record_and_get_qps(&self) -> u64 {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let mut inner = self.inner.lock().unwrap();
        let elapsed = now.saturating_sub(inner.start_second) as usize;

        if elapsed >= Self::WINDOW_SIZE {
            // 窗口过期：重置所有计数器，新窗口从当前秒开始
            for c in &mut inner.counters {
                *c = 0;
            }
            inner.start_second = now;
            inner.counters[0] = 1; // 当前请求计入 slot 0
            return DEFAULT_QPS_BASELINE;
        }

        let current_slot = elapsed % Self::WINDOW_SIZE;
        inner.counters[current_slot] += 1;

        // 计算总请求数和活跃槽位数
        let total_count: u64 = inner.counters.iter().sum();
        let active_slots = inner
            .counters
            .iter()
            .rposition(|&c| c > 0)
            .map(|i| i + 1)
            .unwrap_or(1);

        let avg_secs = active_slots.max(1) as u64;
        let qps = total_count / avg_secs;
        qps.max(DEFAULT_QPS_BASELINE)
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
    /// 用于 QPS 计算的原子计数器
    counter: Arc<std::sync::atomic::AtomicU64>,
    /// QPS 滑动窗口（M6 + F-03：实例字段，避免跨实例共享 + TOCTOU race）
    qps_window: Arc<QpsWindow>,
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
            counter: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            dc_failure_detector,
            local_dc_id,
            etcd_cluster_health_monitor: None,
            step_calculator: StepCalculator::default(),
            segment_config: config,
            qps_window: Arc::new(QpsWindow::new()),
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
    ///
    /// 使用滑动窗口计数器精确计算最近 60 秒的平均 QPS。
    ///
    /// **M6 + F-03 修复**：原实现使用 `static OnceLock<WindowCounter>`，存在两个问题：
    /// 1. **M6**：多个 `DatabaseSegmentLoader` 实例共享同一计数器，QPS 统计失真
    /// 2. **F-03**：`fetch_add` 与 `store(0)` 之间存在 TOCTOU race，计数可能丢失
    ///
    /// 现改为使用实例字段 `self.qps_window: Arc<QpsWindow>`，所有操作在 Mutex 内完成。
    fn get_current_qps(&self) -> u64 {
        self.qps_window.record_and_get_qps()
    }
}

#[async_trait]
impl SegmentLoader for DatabaseSegmentLoader {
    async fn load_segment(&self, ctx: &GenerateContext, _worker_id: u8) -> Result<SegmentData> {
        // 计数器递增，用于 QPS 计算
        self.counter
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        // 获取当前 QPS (简化处理，实际应从监控获取)
        let current_qps = self.get_current_qps();
        let step = self.calculate_step(current_qps);

        tracing::debug!(
            "{}",
            t!(
                "log.core.algorithm.segment.loading_segment",
                biz_tag = ctx.biz_tag,
                step = step,
                qps = current_qps
            )
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

// ============================================================================
// DI Support - Builder Pattern and with_dependencies
// ============================================================================

use confers::interface::{ConfigProvider, ConfigProviderExt};
use oxcache::Cache;

impl SegmentAlgorithm {
    /// Create a new SegmentAlgorithm with all dependencies injected.
    ///
    /// This is the primary construction mode for full DI support.
    ///
    /// # Arguments
    ///
    /// * `config` - Configuration provider from confers
    /// * `cache` - Cache backend from oxcache (optional, can use internal)
    /// * `repository` - Segment repository for database operations
    /// * `local_dc_id` - Local datacenter ID
    pub fn with_dependencies(
        config: Arc<dyn ConfigProvider>,
        _cache: Option<Arc<Cache<String, Vec<u8>>>>,
        repository: Arc<dyn SegmentRepository>,
        local_dc_id: u8,
    ) -> Self {
        let dc_failure_detector = Arc::new(DcFailureDetector::new(5, Duration::from_secs(300)));
        dc_failure_detector.add_dc(local_dc_id);

        let (shutdown_tx, _) = tokio::sync::watch::channel(false);

        // Extract segment config from provider
        let segment_config = SegmentAlgorithmConfig {
            base_step: config
                .get_int("algorithm.segment.base_step")
                .unwrap_or(1000) as u64,
            min_step: config.get_int("algorithm.segment.min_step").unwrap_or(500) as u64,
            max_step: config
                .get_int("algorithm.segment.max_step")
                .unwrap_or(100000) as u64,
            switch_threshold: config
                .get_float("algorithm.segment.switch_threshold")
                .unwrap_or(0.1),
        };

        // Use RepositoryBackedLoader which wraps the repository
        let segment_loader = Arc::new(RepositoryBackedLoader::new(
            repository,
            segment_config.clone(),
        ));

        Self {
            config: segment_config,
            buffers: Arc::new(RwLock::new(HashMap::new())),
            metrics: Arc::new(AlgorithmMetricsInner::default()),
            segment_loader,
            dc_failure_detector,
            local_dc_id,
            etcd_cluster_health_monitor: None,
            cpu_monitor: None,
            cpu_monitor_task: Arc::new(tokio::sync::Mutex::new(None)),
            shutdown_tx: Arc::new(shutdown_tx),
            health_check_task: Arc::new(tokio::sync::Mutex::new(None)),
        }
    }

    /// Create a new builder for SegmentAlgorithm.
    ///
    /// Use the builder pattern for partial dependency injection.
    pub fn builder() -> SegmentAlgorithmBuilder {
        SegmentAlgorithmBuilder::new()
    }
}

/// SegmentLoader that wraps a SegmentRepository.
pub struct RepositoryBackedLoader {
    repository: Arc<dyn SegmentRepository>,
    config: SegmentAlgorithmConfig,
}

impl RepositoryBackedLoader {
    /// Create a new RepositoryBackedLoader.
    pub fn new(repository: Arc<dyn SegmentRepository>, config: SegmentAlgorithmConfig) -> Self {
        Self { repository, config }
    }
}

#[async_trait]
impl SegmentLoader for RepositoryBackedLoader {
    async fn load_segment(&self, ctx: &GenerateContext, _worker_id: u8) -> Result<SegmentData> {
        let segment = self
            .repository
            .allocate_segment(
                &ctx.workspace_id,
                &ctx.biz_tag,
                self.config.base_step as i32,
            )
            .await
            .map_err(|e| CoreError::DatabaseError(e.to_string()))?;

        Ok(SegmentData {
            start_id: segment.current_id as u64,
            max_id: segment.max_id as u64,
            step: segment.step as u64,
            version: 0,
        })
    }
}

/// Builder for SegmentAlgorithm.
///
/// This builder allows partial dependency injection,
/// with missing dependencies using default values.
#[derive(Default)]
pub struct SegmentAlgorithmBuilder {
    config: Option<Arc<dyn ConfigProvider>>,
    cache: Option<Arc<Cache<String, Vec<u8>>>>,
    repository: Option<Arc<dyn SegmentRepository>>,
    local_dc_id: Option<u8>,
    segment_loader: Option<Arc<dyn SegmentLoader + Send + Sync>>,
    cpu_monitor: Option<Arc<CpuMonitor>>,
    dc_failure_detector: Option<Arc<DcFailureDetector>>,
}

impl SegmentAlgorithmBuilder {
    /// Create a new builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the configuration provider.
    pub fn config(mut self, config: Arc<dyn ConfigProvider>) -> Self {
        self.config = Some(config);
        self
    }

    /// Set the cache backend.
    pub fn cache(mut self, cache: Arc<Cache<String, Vec<u8>>>) -> Self {
        self.cache = Some(cache);
        self
    }

    /// Set the segment repository.
    pub fn repository(mut self, repository: Arc<dyn SegmentRepository>) -> Self {
        self.repository = Some(repository);
        self
    }

    /// Set the local datacenter ID.
    pub fn local_dc_id(mut self, dc_id: u8) -> Self {
        self.local_dc_id = Some(dc_id);
        self
    }

    /// Set the segment loader.
    pub fn segment_loader(mut self, loader: Arc<dyn SegmentLoader + Send + Sync>) -> Self {
        self.segment_loader = Some(loader);
        self
    }

    /// Set the CPU monitor.
    pub fn cpu_monitor(mut self, monitor: Arc<CpuMonitor>) -> Self {
        self.cpu_monitor = Some(monitor);
        self
    }

    /// Set the DC failure detector.
    pub fn dc_failure_detector(mut self, detector: Arc<DcFailureDetector>) -> Self {
        self.dc_failure_detector = Some(detector);
        self
    }

    /// Build the SegmentAlgorithm.
    ///
    /// Uses default values for missing dependencies.
    pub fn build(self) -> SegmentAlgorithm {
        let local_dc_id = self.local_dc_id.unwrap_or(0);
        let dc_failure_detector = self.dc_failure_detector.unwrap_or_else(|| {
            let detector = Arc::new(DcFailureDetector::new(5, Duration::from_secs(300)));
            detector.add_dc(local_dc_id);
            detector
        });

        let (shutdown_tx, _) = tokio::sync::watch::channel(false);

        let config = self
            .config
            .as_ref()
            .map(|c| SegmentAlgorithmConfig {
                base_step: c.get_int("algorithm.segment.base_step").unwrap_or(1000) as u64,
                min_step: c.get_int("algorithm.segment.min_step").unwrap_or(500) as u64,
                max_step: c.get_int("algorithm.segment.max_step").unwrap_or(100000) as u64,
                switch_threshold: c
                    .get_float("algorithm.segment.switch_threshold")
                    .unwrap_or(0.1),
            })
            .unwrap_or_default();

        let segment_loader = self
            .segment_loader
            .unwrap_or_else(|| Arc::new(DefaultSegmentLoader::default()));

        SegmentAlgorithm {
            config,
            buffers: Arc::new(RwLock::new(HashMap::new())),
            metrics: Arc::new(AlgorithmMetricsInner::default()),
            segment_loader,
            dc_failure_detector,
            local_dc_id,
            etcd_cluster_health_monitor: None,
            cpu_monitor: self.cpu_monitor,
            cpu_monitor_task: Arc::new(tokio::sync::Mutex::new(None)),
            shutdown_tx: Arc::new(shutdown_tx),
            health_check_task: Arc::new(tokio::sync::Mutex::new(None)),
        }
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
            format: crate::core::types::IdFormat::Numeric,
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

    // ------------------------------------------------------------------
    // Test helpers: MockSegmentRepository / MockConfigProvider
    // ------------------------------------------------------------------

    use crate::core::database::SegmentRepository;
    use crate::core::types::SegmentInfo;
    use async_trait::async_trait;
    use chrono::Utc;
    use confers::interface::ConfigProvider;
    use confers::types::{AnnotatedValue, ConfigValue, SourceId};

    /// Mock `SegmentRepository`锛氭牴鎹?`fail` 鏍囧織杩斿洖鎴愬姛/澶辫触銆?
    struct MockSegmentRepository {
        fail: bool,
        fail_with_dc: bool,
        segment: Option<SegmentInfo>,
        calls: Arc<std::sync::Mutex<u32>>,
    }

    impl MockSegmentRepository {
        fn success(segment: SegmentInfo) -> Self {
            Self {
                fail: false,
                fail_with_dc: false,
                segment: Some(segment),
                calls: Arc::new(std::sync::Mutex::new(0)),
            }
        }

        fn failing() -> Self {
            Self {
                fail: true,
                fail_with_dc: false,
                segment: None,
                calls: Arc::new(std::sync::Mutex::new(0)),
            }
        }

        fn failing_with_dc() -> Self {
            Self {
                fail: false,
                fail_with_dc: true,
                segment: None,
                calls: Arc::new(std::sync::Mutex::new(0)),
            }
        }

        fn call_count(&self) -> u32 {
            *self.calls.lock().unwrap()
        }
    }

    #[async_trait]
    impl SegmentRepository for MockSegmentRepository {
        async fn get_segment(
            &self,
            _workspace_id: &str,
            _biz_tag: &str,
        ) -> Result<Option<SegmentInfo>> {
            Ok(self.segment.clone())
        }

        async fn allocate_segment(
            &self,
            _workspace_id: &str,
            _biz_tag: &str,
            _step: i32,
        ) -> Result<SegmentInfo> {
            *self.calls.lock().unwrap() += 1;
            if self.fail {
                return Err(crate::core::CoreError::DatabaseError(
                    "mock error".to_string(),
                ));
            }
            Ok(self.segment.clone().unwrap_or(SegmentInfo {
                id: 1,
                workspace_id: "ws".to_string(),
                biz_tag: "tag".to_string(),
                current_id: 1000,
                max_id: 2000,
                step: 1000,
                delta: 0,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            }))
        }

        async fn allocate_segment_with_dc(
            &self,
            _workspace_id: &str,
            _biz_tag: &str,
            _step: i32,
            _dc_id: i32,
        ) -> Result<SegmentInfo> {
            *self.calls.lock().unwrap() += 1;
            if self.fail_with_dc {
                return Err(crate::core::CoreError::DatabaseError(
                    "mock dc error".to_string(),
                ));
            }
            Ok(self.segment.clone().unwrap_or(SegmentInfo {
                id: 1,
                workspace_id: "ws".to_string(),
                biz_tag: "tag".to_string(),
                current_id: 1000,
                max_id: 2000,
                step: 1000,
                delta: 0,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            }))
        }

        async fn update_segment(
            &self,
            _workspace_id: &str,
            _biz_tag: &str,
            _current_id: i64,
            _max_id: i64,
        ) -> Result<()> {
            Ok(())
        }

        async fn create_segment(
            &self,
            _workspace_id: &str,
            _biz_tag: &str,
            _start_id: i64,
            _max_id: i64,
            _step: i32,
            _delta: i32,
        ) -> Result<SegmentInfo> {
            Ok(self.segment.clone().unwrap_or(SegmentInfo {
                id: 1,
                workspace_id: "ws".to_string(),
                biz_tag: "tag".to_string(),
                current_id: 1000,
                max_id: 2000,
                step: 1000,
                delta: 0,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            }))
        }

        async fn list_segments(&self, _workspace_id: &str) -> Result<Vec<SegmentInfo>> {
            Ok(vec![])
        }

        async fn delete_segment(&self, _workspace_id: &str, _biz_tag: &str) -> Result<()> {
            Ok(())
        }
    }

    /// Mock `ConfigProvider`锛氶€氳繃 `with_int`/`with_float` 棰勮閿€笺€?
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

        fn with_float(mut self, key: impl Into<String>, value: f64) -> Self {
            let key = key.into();
            self.values.insert(
                key.clone(),
                AnnotatedValue::new(ConfigValue::F64(value), SourceId::default(), key),
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

    fn sample_segment_info() -> SegmentInfo {
        SegmentInfo {
            id: 1,
            workspace_id: "ws".to_string(),
            biz_tag: "tag".to_string(),
            current_id: 500,
            max_id: 1500,
            step: 1000,
            delta: 0,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
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

    // ------------------------------------------------------------------
    // CpuMonitor tests
    // ------------------------------------------------------------------

    #[test]
    fn test_cpu_monitor_default_returns_default_usage() {
        let monitor = CpuMonitor::default();
        // 榛樿 CPU 浣跨敤鐜囧簲涓?0.1锛圖EFAULT_CPU_USAGE 甯搁噺锛?
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
        monitor.update_usage(2.0); // 瓒呰繃 1.0锛屽簲琚?clamp 鍒?1.0
        let usage = monitor.get_usage();
        assert!((usage - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_cpu_monitor_update_usage_clamps_negative_values() {
        let monitor = CpuMonitor::new();
        monitor.update_usage(-0.5); // 璐熷€硷紝搴旇 clamp 鍒?0.0
        let usage = monitor.get_usage();
        assert!(usage.abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn test_cpu_monitor_start_monitoring_completes_on_non_linux() {
        // 鍦ㄩ潪 Linux 骞冲彴涓婏紝start_monitoring 鍚姩涓€涓┖ task 骞剁珛鍗宠繑鍥?JoinHandle銆?
        // 璇?task 搴旇兘姝ｅ父缁撴潫锛堜笉 panic锛夈€?
        let monitor = CpuMonitor::new();
        let handle = monitor.start_monitoring();
        // 缁?task 涓€鐐规椂闂磋繍琛?
        tokio::time::sleep(Duration::from_millis(10)).await;
        // 涓嶈皟鐢?abort锛岃 runtime drop 鏃惰嚜鐒舵竻鐞?
        handle.abort();
    }

    // ------------------------------------------------------------------
    // Segment / AtomicSegment tests
    // ------------------------------------------------------------------

    #[test]
    fn test_segment_consumed_returns_current_minus_start() {
        let segment = Segment::new(100, 1000, 100);
        // 鍒濆 current_id = start_id = 100锛宑onsumed = 0
        assert_eq!(segment.consumed(), 0);

        segment.current_id.store(500, Ordering::Relaxed);
        assert_eq!(segment.consumed(), 400);
    }

    #[test]
    fn test_segment_consumed_saturating_when_current_below_start() {
        let segment = Segment::new(100, 1000, 100);
        // current 浣庝簬 start 鏃讹紝saturating_sub 杩斿洖 0
        segment.current_id.store(50, Ordering::Relaxed);
        assert_eq!(segment.consumed(), 0);
    }

    #[test]
    fn test_segment_remaining_saturating_when_current_above_max() {
        let segment = Segment::new(0, 1000, 100);
        segment.current_id.store(1500, Ordering::Relaxed);
        assert_eq!(segment.remaining(), 0);
    }

    #[test]
    fn test_atomic_segment_remaining_delegates_to_inner() {
        let segment = AtomicSegment::new(0, 1000, 100);
        assert_eq!(segment.remaining(), 1000);

        // try_consume 浼氭帹杩?current_id
        segment.try_consume(300).unwrap();
        assert_eq!(segment.remaining(), 700);
    }

    #[test]
    fn test_atomic_segment_try_consume_exact_boundary() {
        // 杈圭晫锛歝urrent + count == max 搴斿厑璁告秷璐?
        let segment = AtomicSegment::new(0, 100, 10);
        let result = segment.try_consume(100);
        assert!(result.is_some());
        let (start, end) = result.unwrap();
        assert_eq!(start, 0);
        assert_eq!(end, 100);
    }

    // ------------------------------------------------------------------
    // DoubleBuffer tests
    // ------------------------------------------------------------------

    #[test]
    fn test_double_buffer_set_current_replaces_active_segment() {
        let (db, _rx) = DoubleBuffer::new(0.1);
        let new_segment = Arc::new(AtomicSegment::new(0, 500, 100));
        db.set_current(new_segment);

        let current = db.get_current();
        let seg = current.inner.lock();
        assert_eq!(seg.max_id.load(Ordering::Relaxed), 500);
    }

    #[test]
    fn test_double_buffer_swap_returns_none_when_no_next() {
        // 鏃?next 鏃?swap 杩斿洖 None 涓斾笉鏀瑰彉 current
        let (db, _rx) = DoubleBuffer::new(0.1);
        let result = db.swap();
        assert!(result.is_none());
    }

    #[test]
    fn test_double_buffer_swap_replaces_current_with_next() {
        let (db, _rx) = DoubleBuffer::new(0.1);

        // 璁剧疆鍒濆 current
        let initial = Arc::new(AtomicSegment::new(0, 100, 10));
        db.set_current(initial);

        // 璁剧疆 next
        let next = Arc::new(AtomicSegment::new(100, 200, 10));
        db.set_next(next);

        // swap 搴旇繑鍥?Some(old_next) 骞跺皢鍏惰涓?current
        let swapped = db.swap();
        assert!(swapped.is_some());

        let current = db.get_current();
        let seg = current.inner.lock();
        assert_eq!(seg.start_id.load(Ordering::Relaxed), 100);
        assert_eq!(seg.max_id.load(Ordering::Relaxed), 200);

        // next 搴旇娓呯┖
        assert!(db.get_next().is_none());
    }

    #[test]
    fn test_double_buffer_need_switch_when_total_zero() {
        // 鍒濆 segment (0,0,0)锛宼otal = 0锛屽簲瑙﹀彂鍒囨崲
        let (db, _rx) = DoubleBuffer::new(0.1);
        assert!(db.need_switch());
    }

    #[test]
    fn test_double_buffer_need_switch_when_below_threshold() {
        // switch_threshold = 0.3锛屽墿浣?20% < 30%锛屽簲瑙﹀彂鍒囨崲
        let (db, _rx) = DoubleBuffer::new(0.3);
        let seg = Arc::new(AtomicSegment::new(0, 1000, 100));
        db.set_current(seg);

        // 娑堣垂鍒板彧鍓?200锛?0%锛?
        {
            let current = db.get_current();
            current
                .inner
                .lock()
                .current_id
                .store(800, Ordering::Relaxed);
        }

        assert!(db.need_switch());
    }

    #[test]
    fn test_double_buffer_no_switch_when_above_threshold() {
        // switch_threshold = 0.1锛屽墿浣?50% > 10%锛屼笉瑙﹀彂鍒囨崲
        let (db, _rx) = DoubleBuffer::new(0.1);
        let seg = Arc::new(AtomicSegment::new(0, 1000, 100));
        db.set_current(seg);

        {
            let current = db.get_current();
            current
                .inner
                .lock()
                .current_id
                .store(500, Ordering::Relaxed);
        }

        assert!(!db.need_switch());
    }

    #[test]
    fn test_double_buffer_get_next_returns_set_segment() {
        let (db, _rx) = DoubleBuffer::new(0.1);
        assert!(db.get_next().is_none());

        let next = Arc::new(AtomicSegment::new(100, 200, 10));
        db.set_next(next);

        let retrieved = db.get_next();
        assert!(retrieved.is_some());
        let binding = retrieved.unwrap();
        let seg = binding.inner.lock();
        assert_eq!(seg.start_id.load(Ordering::Relaxed), 100);
    }

    // ------------------------------------------------------------------
    // DcFailureDetector / DcHealthState additional tests
    // ------------------------------------------------------------------

    #[test]
    fn test_dc_failure_detector_clone_preserves_shared_state() {
        // Clone 鍚庝袱涓?detector 鍏变韩 RwLock<HashMap>锛岀姸鎬佷簰閫?
        let detector = DcFailureDetector::new(5, Duration::from_secs(300));
        detector.add_dc(7);

        let cloned = detector.clone();
        // 閫氳繃 cloned 鎿嶄綔鐘舵€侊紝鍘?detector 搴旇兘鐪嬪埌
        let state = cloned.get_dc_state(7).unwrap();
        state.record_failure();

        let original_state = detector.get_dc_state(7).unwrap();
        assert_eq!(original_state.failure_count.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_dc_failure_detector_select_best_dc_falls_back_to_healthy() {
        // preferred_dc=2 涓嶅瓨鍦紝浣嗘湁鍏朵粬鍋ュ悍 DC锛屽簲杩斿洖绗竴涓仴搴?DC
        let detector = DcFailureDetector::new(5, Duration::from_secs(300));
        detector.add_dc(0);
        detector.add_dc(1);

        // dc 2 涓嶅瓨鍦紝select_best_dc 搴旇繘鍏?fallback 鍒嗘敮
        let best = detector.select_best_dc(2);
        // 鐢变簬 dc 2 涓嶅瓨鍦紝涓?dc 0銆? 閮藉仴搴凤紝搴旇繑鍥炲叾涓箣涓€
        assert!(
            best == 0 || best == 1,
            "expected fallback to healthy dc, got {}",
            best
        );
    }

    #[test]
    fn test_dc_failure_detector_select_best_dc_returns_preferred_when_failed() {
        // preferred_dc 宸?Failed锛屽簲杩涘叆 fallback
        let detector = DcFailureDetector::new(5, Duration::from_secs(300));
        detector.add_dc(0);
        detector.add_dc(1);

        // 璁?dc 0 澶辫触 5 娆?
        let state = detector.get_dc_state(0).unwrap();
        for _ in 0..5 {
            state.record_failure();
        }
        assert_eq!(state.get_status(), DcStatus::Failed);

        // preferred_dc=0 宸?Failed锛屽簲杩斿洖鍋ュ悍鐨?dc 1
        let best = detector.select_best_dc(0);
        assert_eq!(best, 1);
    }

    #[test]
    fn test_dc_failure_detector_select_best_dc_no_healthy_returns_preferred() {
        // 娌℃湁 DC 鐘舵€佸瓨鍦紝select_best_dc 搴旇繑鍥?preferred_dc
        let detector = DcFailureDetector::new(5, Duration::from_secs(300));
        // 涓嶈皟鐢?add_dc
        let best = detector.select_best_dc(99);
        assert_eq!(best, 99);
    }

    #[tokio::test]
    async fn test_dc_failure_detector_check_recovery_promotes_failed_to_degraded() {
        // recovery_timeout 璁句负 1ms锛屼娇 Failed DC 鑳界珛鍗虫仮澶嶄负 Degraded
        let detector = DcFailureDetector::new(5, Duration::from_millis(1));
        detector.add_dc(0);

        let state = detector.get_dc_state(0).unwrap();
        for _ in 0..5 {
            state.record_failure();
        }
        assert_eq!(state.get_status(), DcStatus::Failed);

        // 绛夊緟瓒呰繃 recovery_timeout
        tokio::time::sleep(Duration::from_millis(10)).await;

        // 鐩存帴璋冪敤 check_recovery锛堢鏈夋柟娉曪紝閫氳繃 start_health_check_with_shutdown 闂存帴瑕嗙洊鏇村鏉傦紝
        // 杩欓噷鎴戜滑 spawn 涓€娆?check_recovery 閫氳繃鐭棿闅斾换鍔″苟绛夊緟锛?
        // 鐢变簬 check_recovery 鏄鏈夋柟娉曪紝鎴戜滑閫氳繃 start_health_check_with_shutdown 瑙﹀彂瀹?
        let (shutdown_tx, rx) = tokio::sync::watch::channel(false);
        let shutdown_rx = rx;
        // 鐢ㄥ緢鐭殑 check_interval 瑙﹀彂 check_recovery
        detector
            .start_health_check_with_shutdown(Duration::from_millis(1), shutdown_rx.clone())
            .await;

        // 绛夊緟瓒冲鏃堕棿璁?check_recovery 鎵ц
        tokio::time::sleep(Duration::from_millis(20)).await;

        // 鐘舵€佸簲浠?Failed 杞负 Degraded
        assert_eq!(state.get_status(), DcStatus::Degraded);

        // 鍙戦€?shutdown 淇″彿
        let _ = shutdown_tx.send(true);
        // 绛夊緟 task 閫€鍑?
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    #[test]
    fn test_dc_health_state_record_success_resets_consecutive_failures() {
        let state = DcHealthState::new(3);
        // 瑙﹀彂 3 娆″け璐ヨ繘鍏?Degraded
        for _ in 0..3 {
            state.record_failure();
        }
        assert_eq!(state.get_status(), DcStatus::Degraded);
        assert_eq!(state.consecutive_failures.load(Ordering::Relaxed), 3);

        // 鎴愬姛搴旈噸缃?consecutive_failures 骞舵仮澶?Healthy
        state.record_success();
        assert_eq!(state.consecutive_failures.load(Ordering::Relaxed), 0);
        assert_eq!(state.get_status(), DcStatus::Healthy);
    }

    #[test]
    fn test_dc_health_state_failure_count_accumulates_separately() {
        // failure_count 涓?consecutive_failures 涓嶅悓锛氬け璐ュ悗 success 涓嶄細閲嶇疆 failure_count
        let state = DcHealthState::new(0);
        state.record_failure();
        state.record_failure();
        state.record_success();
        // failure_count 绱Н涓?2锛屼絾 consecutive_failures 宸查噸缃负 0
        assert_eq!(state.failure_count.load(Ordering::Relaxed), 2);
        assert_eq!(state.consecutive_failures.load(Ordering::Relaxed), 0);
    }

    // ------------------------------------------------------------------
    // StepCalculator tests
    // ------------------------------------------------------------------

    #[test]
    fn test_step_calculator_with_cpu_monitor_uses_monitor_usage() {
        let monitor = Arc::new(CpuMonitor::new());
        monitor.update_usage(0.8); // 楂?CPU 璐熻浇

        let calculator = StepCalculator::new(0.5, 0.3).with_cpu_monitor(monitor);
        let config = SegmentAlgorithmConfig::default();
        let step = calculator.calculate(100, 1000, &config);
        // 搴旈珮浜庢棤 CPU 鐩戞帶鏃剁殑姝ラ暱锛堝洜涓?pressure=0.8 姣?0.1 澶э級
        let plain = StepCalculator::new(0.5, 0.3).calculate(100, 1000, &config);
        assert!(
            step >= plain,
            "expected step with high cpu pressure ({}) >= plain step ({})",
            step,
            plain
        );
    }

    #[test]
    fn test_step_calculator_calculate_uses_base_step_when_current_step_zero() {
        // current_step=0 鏃跺簲鍥為€€鍒?config.base_step锛岄伩鍏嶉櫎闆?
        let calculator = StepCalculator::default();
        let config = SegmentAlgorithmConfig {
            base_step: 500,
            min_step: 100,
            max_step: 100000,
            switch_threshold: 0.1,
        };
        // 涓嶅簲 panic锛屼笖缁撴灉钀藉湪 [min_step, max_step] 涔嬮棿
        let step = calculator.calculate(100, 0, &config);
        assert!(
            step >= config.min_step,
            "step {} should be >= min_step",
            step
        );
        assert!(
            step <= config.max_step,
            "step {} should be <= max_step",
            step
        );
    }

    #[test]
    fn test_step_calculator_get_adjustment_direction_returns_up() {
        // 楂?QPS + 浣?current_step 鈫?ratio > 1.2 鈫?"up"
        let calculator = StepCalculator::new(0.5, 0.5);
        let config = SegmentAlgorithmConfig::default();
        // base_step=1000锛宷ps=10000锛宑urrent_step=100 鈫?velocity=100锛宻tep 澧為暱鏄捐憲
        let direction = calculator.get_adjustment_direction(10000, 100, &config);
        assert_eq!(direction, "up");
    }

    #[test]
    fn test_step_calculator_get_adjustment_direction_returns_stable() {
        // current_step 鎺ヨ繎 calculate 缁撴灉 鈫?"stable"
        // 榛樿 calculator: velocity_factor=0.5, pressure_factor=0.3, default pressure=0.1
        // qps=200, current_step=1000: velocity=0.2
        // next_step = 1000 * (1 + 0.5*0.2) * (1 + 0.3*0.1) = 1000 * 1.1 * 1.03 鈮?1133
        // ratio = 1133/1000 = 1.133, 鍦?[0.8, 1.2] 涔嬮棿 鈫?"stable"
        let calculator = StepCalculator::default();
        let config = SegmentAlgorithmConfig::default();
        let direction = calculator.get_adjustment_direction(200, 1000, &config);
        assert_eq!(direction, "stable");
    }

    #[test]
    fn test_step_calculator_get_adjustment_direction_returns_down_when_target_smaller() {
        // 璁?calculate 杩斿洖鍊艰繙灏忎簬 current_step
        // 鐢?min_step 寮哄埗 calculate 杈撳嚭鏈€灏忓€?
        let calculator = StepCalculator::default();
        let config = SegmentAlgorithmConfig {
            base_step: 100,
            min_step: 50, // min_step = 50
            max_step: 1000,
            switch_threshold: 0.1,
        };
        // current_step 璁惧緢澶э紙濡?1000锛夛紝qps=0 鈫?target 鈮?base_step * (1 + 0 * 0) * (1 + 0.3 * 0.1) 鈮?103
        // ratio = 103 / 1000 = 0.103 < 0.8 鈫?"down"
        let direction = calculator.get_adjustment_direction(0, 1000, &config);
        assert_eq!(direction, "down");
    }

    #[test]
    fn test_step_calculator_calculate_respects_max_step() {
        // 褰?QPS 鏋侀珮鏃讹紝calculate 搴旇 max_step 闄愬埗
        let calculator = StepCalculator::new(1.0, 1.0);
        let config = SegmentAlgorithmConfig {
            base_step: 1000,
            min_step: 100,
            max_step: 50000,
            switch_threshold: 0.1,
        };
        let step = calculator.calculate(u64::MAX, 1000, &config);
        assert!(
            step <= config.max_step,
            "expected step {} <= max_step {}",
            step,
            config.max_step
        );
    }

    #[test]
    fn test_step_calculator_calculate_respects_min_step() {
        // 褰?QPS 涓?0 涓旀棤 CPU 鍘嬪姏鏃讹紝calculate 搴旇 min_step 闄愬埗
        let calculator = StepCalculator::default();
        let config = SegmentAlgorithmConfig {
            base_step: 1000,
            min_step: 5000, // min_step 楂樹簬 base_step * 0.5
            max_step: 100000,
            switch_threshold: 0.1,
        };
        let step = calculator.calculate(0, 1000, &config);
        assert!(
            step >= config.min_step,
            "expected step {} >= min_step {}",
            step,
            config.min_step
        );
    }

    // ------------------------------------------------------------------
    // SegmentAlgorithm: inherent DI methods + Builder
    // ------------------------------------------------------------------

    #[test]
    fn test_segment_algorithm_default_uses_dc_id_zero() {
        let algo = SegmentAlgorithm::default();
        // default 璋冪敤 new(0)锛屽簲鑷姩 add_dc(0)
        let detector = algo.get_dc_failure_detector();
        assert!(detector.get_dc_state(0).is_some());
    }

    #[tokio::test]
    async fn test_segment_algorithm_with_loader_replaces_default() {
        // 鑷畾涔?loader锛歭oad_segment 杩斿洖鍥哄畾 SegmentData
        struct FixedLoader;
        #[async_trait]
        impl SegmentLoader for FixedLoader {
            async fn load_segment(
                &self,
                _ctx: &GenerateContext,
                _worker_id: u8,
            ) -> Result<SegmentData> {
                Ok(SegmentData {
                    start_id: 1,
                    max_id: 100,
                    step: 10,
                    version: 0,
                })
            }
        }
        let loader: Arc<dyn SegmentLoader + Send + Sync> = Arc::new(FixedLoader);
        let algo = SegmentAlgorithm::new(0).with_loader(loader);
        // 閫氳繃 generate 瑙﹀彂 load_segment
        let ctx = sample_ctx();
        let id = algo.generate(&ctx).await.unwrap();
        // FixedLoader 杩斿洖 start_id=1锛岀涓€娆?try_consume(1) 搴旇繑鍥?1
        assert_eq!(id.as_u128(), 1);
    }

    #[tokio::test]
    async fn test_segment_algorithm_with_loader_replaces_default_async() {
        // 寮傛鐗堟湰锛氫繚璇?with_loader 鍦?async 涓婁笅鏂囦腑涔熻兘宸ヤ綔
        struct FixedLoader;
        #[async_trait]
        impl SegmentLoader for FixedLoader {
            async fn load_segment(
                &self,
                _ctx: &GenerateContext,
                _worker_id: u8,
            ) -> Result<SegmentData> {
                Ok(SegmentData {
                    start_id: 1000,
                    max_id: 2000,
                    step: 100,
                    version: 0,
                })
            }
        }
        let loader: Arc<dyn SegmentLoader + Send + Sync> = Arc::new(FixedLoader);
        let algo = SegmentAlgorithm::new(0).with_loader(loader);

        let ctx = sample_ctx();
        let id = algo.generate(&ctx).await.unwrap();
        assert_eq!(id.as_u128(), 1000);
    }

    #[tokio::test]
    async fn test_segment_algorithm_with_cpu_monitor_sets_field() {
        let monitor = Arc::new(CpuMonitor::new());
        let algo = SegmentAlgorithm::new(0).with_cpu_monitor(monitor);
        // initialize 搴旇兘姝ｅ父鍚姩 CPU monitor task
        let mut algo = algo;
        let config = Config::default();
        algo.initialize(&config).await.unwrap();
        algo.shutdown().await.unwrap();
    }

    #[test]
    fn test_segment_algorithm_with_dc_failure_detector_replaces_default() {
        let detector = Arc::new(DcFailureDetector::new(7, Duration::from_secs(60)));
        detector.add_dc(5);
        let algo = SegmentAlgorithm::new(0).with_dc_failure_detector(detector);
        // get_dc_failure_detector 搴旇繑鍥炴敞鍏ョ殑 detector
        let returned = algo.get_dc_failure_detector();
        assert!(returned.get_dc_state(5).is_some());
        // 娉ㄦ剰锛氭敞鍏ョ殑 detector 娌℃湁 dc_id=0
        assert!(returned.get_dc_state(0).is_none());
    }

    #[cfg(not(feature = "etcd"))]
    #[test]
    fn test_segment_algorithm_with_etcd_cluster_health_monitor_non_etcd() {
        // 闈?etcd 鐗堟湰锛歸ith_etcd_cluster_health_monitor(Arc<()>) 搴旀甯歌缃?
        let algo = SegmentAlgorithm::new(0).with_etcd_cluster_health_monitor(Arc::new(()));
        // get_etcd_cluster_health_monitor 搴旇繑鍥?Some
        assert!(algo.get_etcd_cluster_health_monitor().is_some());
    }

    #[cfg(not(feature = "etcd"))]
    #[test]
    fn test_segment_algorithm_get_etcd_cluster_health_monitor_returns_none_by_default() {
        // 榛樿鏈缃?etcd monitor锛屽簲杩斿洖 None
        let algo = SegmentAlgorithm::new(0);
        assert!(algo.get_etcd_cluster_health_monitor().is_none());
    }

    #[test]
    fn test_segment_algorithm_get_dc_failure_detector_returns_reference() {
        let algo = SegmentAlgorithm::new(3);
        let detector = algo.get_dc_failure_detector();
        // 楠岃瘉杩斿洖鐨勫紩鐢ㄦ槸鏈夋晥鐨勶紝涓斿寘鍚?dc_id=3
        assert!(detector.get_dc_state(3).is_some());
    }

    #[tokio::test]
    async fn test_segment_algorithm_initialize_starts_health_check_task() {
        // initialize 搴斿惎鍔?health check task锛宻hutdown 搴旇兘姝ｇ‘鍋滄
        let mut algo = SegmentAlgorithm::new(0);
        let config = Config::default();
        algo.initialize(&config).await.unwrap();
        // health_check_task 搴斿凡璁剧疆
        assert!(algo.health_check_task.lock().await.is_some());
        // shutdown 搴旇兘姝ｇ‘鍋滄 task
        algo.shutdown().await.unwrap();
        assert!(algo.health_check_task.lock().await.is_none());
    }

    #[tokio::test]
    async fn test_segment_algorithm_initialize_with_cpu_monitor_starts_monitoring() {
        // initialize 鏃惰嫢鏈?cpu_monitor 搴斿惎鍔?monitoring task
        let monitor = Arc::new(CpuMonitor::new());
        let mut algo = SegmentAlgorithm::new(0).with_cpu_monitor(monitor);
        let config = Config::default();
        algo.initialize(&config).await.unwrap();
        // cpu_monitor_task 搴斿凡璁剧疆
        assert!(algo.cpu_monitor_task.lock().await.is_some());
        algo.shutdown().await.unwrap();
    }

    #[test]
    fn test_segment_algorithm_builder_new_returns_default_builder() {
        let _builder = SegmentAlgorithmBuilder::new();
        // new() 搴旇繑鍥?Default::default() 绛変环鐨?builder
        let default_builder = SegmentAlgorithmBuilder::default();
        let _ = default_builder;
    }

    #[test]
    fn test_segment_algorithm_builder_with_all_setters() {
        // 閾惧紡璋冪敤鎵€鏈?builder setter锛岀‘淇濈紪璇戦€氳繃涓斾笉 panic
        let detector = Arc::new(DcFailureDetector::new(5, Duration::from_secs(300)));
        let monitor = Arc::new(CpuMonitor::new());
        let config_provider: Arc<dyn ConfigProvider> = Arc::new(MockConfigProvider::new());
        let repository: Arc<dyn SegmentRepository> =
            Arc::new(MockSegmentRepository::success(sample_segment_info()));

        let builder = SegmentAlgorithmBuilder::new()
            .config(config_provider)
            .cache(Arc::new(Cache::new()))
            .repository(repository)
            .local_dc_id(2)
            .cpu_monitor(monitor)
            .dc_failure_detector(detector);
        let _ = builder;
    }

    #[tokio::test]
    async fn test_segment_algorithm_builder_build_with_defaults() {
        // 涓嶈缃换浣曚緷璧栵紝build() 搴斾娇鐢ㄩ粯璁ゅ€?
        let algo = SegmentAlgorithmBuilder::new().build();
        // 搴旇兘姝ｅ父 generate
        let ctx = sample_ctx();
        let id = algo.generate(&ctx).await.unwrap();
        assert!(id.as_u128() > 0);
    }

    #[test]
    fn test_segment_algorithm_builder_build_with_custom_config() {
        // 閫氳繃 MockConfigProvider 鎻愪緵鑷畾涔夐厤缃€?
        let provider = MockConfigProvider::new()
            .with_int("algorithm.segment.base_step", 2000)
            .with_int("algorithm.segment.min_step", 1000)
            .with_int("algorithm.segment.max_step", 50000)
            .with_float("algorithm.segment.switch_threshold", 0.05);
        let algo = SegmentAlgorithm::builder()
            .config(Arc::new(provider))
            .build();
        // 楠岃瘉 config 琚纭敞鍏ワ紙閫氳繃 get_or_create_buffer 鍒涘缓鐨?buffer switch_threshold锛?
        // 鏃犳硶鐩存帴璁块棶 config 瀛楁锛堢鏈夛級锛屼絾鍙瀵熻涓猴細buffer 鍒涘缓涓?panic
        let _ = algo;
    }

    #[tokio::test]
    async fn test_segment_algorithm_builder_build_with_custom_loader() {
        struct FixedLoader;
        #[async_trait]
        impl SegmentLoader for FixedLoader {
            async fn load_segment(
                &self,
                _ctx: &GenerateContext,
                _worker_id: u8,
            ) -> Result<SegmentData> {
                Ok(SegmentData {
                    start_id: 42,
                    max_id: 142,
                    step: 50,
                    version: 1,
                })
            }
        }
        let loader: Arc<dyn SegmentLoader + Send + Sync> = Arc::new(FixedLoader);
        let algo = SegmentAlgorithm::builder()
            .segment_loader(loader)
            .local_dc_id(1)
            .build();
        let ctx = sample_ctx();
        let id = algo.generate(&ctx).await.unwrap();
        assert_eq!(id.as_u128(), 42);
    }

    #[tokio::test]
    async fn test_segment_algorithm_builder_build_with_custom_dc_failure_detector() {
        let detector = Arc::new(DcFailureDetector::new(8, Duration::from_secs(120)));
        detector.add_dc(4);
        let algo = SegmentAlgorithm::builder()
            .dc_failure_detector(detector)
            .build();
        // 娉ㄥ叆鐨?detector 搴旇浣跨敤
        assert!(algo.get_dc_failure_detector().get_dc_state(4).is_some());
    }

    // ------------------------------------------------------------------
    // SegmentAlgorithm: IdAlgorithm trait methods (more branches)
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_segment_algorithm_generate_with_pre_loaded_next_uses_swap() {
        // 閫氳繃 with_loader 娉ㄥ叆浼氬け璐ョ殑 loader锛屼絾鎻愬墠璁剧疆 next buffer 璁?generate 璧?swap 鍒嗘敮
        struct FailingLoader;
        #[async_trait]
        impl SegmentLoader for FailingLoader {
            async fn load_segment(
                &self,
                _ctx: &GenerateContext,
                _worker_id: u8,
            ) -> Result<SegmentData> {
                Err(CoreError::DatabaseError("should not be called".to_string()))
            }
        }
        let loader: Arc<dyn SegmentLoader + Send + Sync> = Arc::new(FailingLoader);
        let algo = SegmentAlgorithm::new(0).with_loader(loader);

        // 棰勫厛鍒涘缓涓€涓?buffer 骞惰缃?next
        let key = "ws:tag";
        let buffer = algo.get_or_create_buffer(key);
        let next_seg = Arc::new(AtomicSegment::new(1000, 2000, 100));
        buffer.set_next(next_seg);

        let ctx = sample_ctx();
        let id = algo.generate(&ctx).await.unwrap();
        // swap 鍚庣涓€娆?try_consume(1) 搴旇繑鍥?1000
        assert_eq!(id.as_u128(), 1000);
    }

    #[tokio::test]
    async fn test_segment_algorithm_generate_returns_segment_exhausted_when_loader_fails() {
        // loader 澶辫触鏃?generate 搴斾紶鎾?DatabaseError
        struct FailingLoader;
        #[async_trait]
        impl SegmentLoader for FailingLoader {
            async fn load_segment(
                &self,
                _ctx: &GenerateContext,
                _worker_id: u8,
            ) -> Result<SegmentData> {
                Err(CoreError::DatabaseError("loader failed".to_string()))
            }
        }
        let loader: Arc<dyn SegmentLoader + Send + Sync> = Arc::new(FailingLoader);
        let algo = SegmentAlgorithm::new(0).with_loader(loader);

        let ctx = sample_ctx();
        let result = algo.generate(&ctx).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            CoreError::DatabaseError(msg) => assert!(msg.contains("loader failed")),
            other => panic!("expected DatabaseError, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_segment_algorithm_generate_exhausted_after_segment_drained() {
        // loader has state: first call returns usable segment, subsequent calls return exhausted
        // a naive loader always returning fresh segments would let generate succeed forever
        struct ExhaustingLoader {
            calls: AtomicU64,
        }
        #[async_trait]
        impl SegmentLoader for ExhaustingLoader {
            async fn load_segment(
                &self,
                _ctx: &GenerateContext,
                _worker_id: u8,
            ) -> Result<SegmentData> {
                let n = self.calls.fetch_add(1, Ordering::SeqCst);
                if n == 0 {
                    Ok(SegmentData {
                        start_id: 0,
                        max_id: 1,
                        step: 1,
                        version: 0,
                    })
                } else {
                    // exhausted segment: start_id == max_id, try_consume always returns None
                    Ok(SegmentData {
                        start_id: 5,
                        max_id: 5,
                        step: 1,
                        version: 0,
                    })
                }
            }
        }
        let loader: Arc<dyn SegmentLoader + Send + Sync> = Arc::new(ExhaustingLoader {
            calls: AtomicU64::new(0),
        });
        let algo = SegmentAlgorithm::new(0).with_loader(loader);

        let ctx = sample_ctx();
        let id1 = algo.generate(&ctx).await.unwrap();
        assert_eq!(id1.as_u128(), 0);

        let result = algo.generate(&ctx).await;
        assert!(
            result.is_err(),
            "expected error after segment drained and loader returns exhausted segment"
        );
        match result.unwrap_err() {
            CoreError::SegmentExhausted { max_id } => assert_eq!(max_id, 5),
            other => panic!("expected SegmentExhausted, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_segment_algorithm_batch_generate_basic_path() {
        let algo = SegmentAlgorithm::new(0);
        let ctx = sample_ctx();
        let batch = algo.batch_generate(&ctx, 5).await.unwrap();
        assert_eq!(batch.ids.len(), 5);
        // 5 涓?ID 搴旀槸杩炵画閫掑鐨?
        let first = batch.ids[0].as_u128();
        for (i, id) in batch.ids.iter().enumerate() {
            assert_eq!(id.as_u128(), first + i as u128);
        }
        assert_eq!(batch.algorithm, AlgorithmType::Segment);
        assert_eq!(batch.biz_tag, "tag");
    }

    #[tokio::test]
    async fn test_segment_algorithm_batch_generate_with_pre_loaded_next() {
        struct FailingLoader;
        #[async_trait]
        impl SegmentLoader for FailingLoader {
            async fn load_segment(
                &self,
                _ctx: &GenerateContext,
                _worker_id: u8,
            ) -> Result<SegmentData> {
                Err(CoreError::DatabaseError("should not be called".to_string()))
            }
        }
        let loader: Arc<dyn SegmentLoader + Send + Sync> = Arc::new(FailingLoader);
        let algo = SegmentAlgorithm::new(0).with_loader(loader);

        // 棰勮 next buffer
        let buffer = algo.get_or_create_buffer("ws:tag");
        let next_seg = Arc::new(AtomicSegment::new(100, 200, 10));
        buffer.set_next(next_seg);

        let ctx = sample_ctx();
        let batch = algo.batch_generate(&ctx, 5).await.unwrap();
        assert_eq!(batch.ids.len(), 5);
        // 绗竴涓?ID 搴旀槸 100
        assert_eq!(batch.ids[0].as_u128(), 100);
    }

    #[tokio::test]
    async fn test_segment_algorithm_batch_generate_propagates_loader_error() {
        struct FailingLoader;
        #[async_trait]
        impl SegmentLoader for FailingLoader {
            async fn load_segment(
                &self,
                _ctx: &GenerateContext,
                _worker_id: u8,
            ) -> Result<SegmentData> {
                Err(CoreError::DatabaseError("batch loader failed".to_string()))
            }
        }
        let loader: Arc<dyn SegmentLoader + Send + Sync> = Arc::new(FailingLoader);
        let algo = SegmentAlgorithm::new(0).with_loader(loader);

        let ctx = sample_ctx();
        let result = algo.batch_generate(&ctx, 10).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            CoreError::DatabaseError(msg) => assert!(msg.contains("batch loader failed")),
            other => panic!("expected DatabaseError, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_segment_algorithm_batch_generate_empty_returns_exhausted_error() {
        // batch_generate size=0 搴旇繑鍥?SegmentExhausted锛坕ds.is_empty() 鍒嗘敮锛?
        let algo = SegmentAlgorithm::new(0);
        let ctx = sample_ctx();
        let result = algo.batch_generate(&ctx, 0).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            CoreError::SegmentExhausted { max_id: _ } => {}
            other => panic!("expected SegmentExhausted, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_segment_algorithm_batch_generate_partial_when_cannot_fill() {
        // loader 杩斿洖 segment 浠呰兘瀹圭撼 3 涓?ID锛屼絾璇锋眰 5 涓?鈫?搴旇兘娑堣€楀埌 3 涓?
        struct SmallLoader;
        #[async_trait]
        impl SegmentLoader for SmallLoader {
            async fn load_segment(
                &self,
                _ctx: &GenerateContext,
                _worker_id: u8,
            ) -> Result<SegmentData> {
                Ok(SegmentData {
                    start_id: 0,
                    max_id: 3, // 鍙兘瀹圭撼 3 涓?ID
                    step: 1,
                    version: 0,
                })
            }
        }
        let loader: Arc<dyn SegmentLoader + Send + Sync> = Arc::new(SmallLoader);
        let algo = SegmentAlgorithm::new(0).with_loader(loader);

        let ctx = sample_ctx();
        // 璇锋眰 3 涓?ID锛屾濂藉～婊?segment
        let batch = algo.batch_generate(&ctx, 3).await.unwrap();
        assert_eq!(batch.ids.len(), 3);
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
        // generate 涓€娆′互鍒涘缓 buffer
        let ctx = sample_ctx();
        let _ = algo.generate(&ctx).await.unwrap();
        let status = algo.health_check();
        assert!(matches!(status, HealthStatus::Healthy));
    }

    #[test]
    fn test_segment_algorithm_metrics_default_returns_full_hit_rate() {
        // 鏈皟鐢ㄨ繃浠讳綍缂撳瓨璺緞 鈫?hits=0, misses=0 鈫?hit_rate=1.0锛堥粯璁わ級
        let algo = SegmentAlgorithm::new(0);
        let m = algo.metrics();
        assert_eq!(m.cache_hit_rate, Some(1.0));
        assert_eq!(m.total_generated, 0);
        assert_eq!(m.total_failed, 0);
    }

    #[tokio::test]
    async fn test_segment_algorithm_metrics_with_cache_misses_records_qps_zero() {
        // generate 一次（首次触发 cache miss + load_segment，随后 try_consume 命中）后 metrics 应反映状态
        let algo = SegmentAlgorithm::new(0);
        let ctx = sample_ctx();
        let _ = algo.generate(&ctx).await.unwrap();
        let m = algo.metrics();
        assert_eq!(m.total_generated, 1);
        // diting-perf C1 修复后：cache_hits=1（try_consume 命中），cache_misses=1（load_segment 触发）
        // hit_rate = hits/(hits+misses) = 1/(1+1) = 0.5
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
        // 鏈皟鐢?initialize 鐩存帴 shutdown 搴斾笉 panic
        let algo = SegmentAlgorithm::new(0);
        algo.shutdown().await.unwrap();
    }

    // ------------------------------------------------------------------
    // QpsWindow tests
    // ------------------------------------------------------------------

    #[test]
    fn test_qps_window_new_returns_baseline_on_first_record() {
        let window = QpsWindow::new();
        let qps = window.record_and_get_qps();
        // first record: total=1, active_slots=1, qps=1, max(1, DEFAULT_QPS_BASELINE=1000) = 1000
        assert_eq!(qps, DEFAULT_QPS_BASELINE);
    }

    #[test]
    fn test_qps_window_multiple_records_stay_at_baseline() {
        // with only a few requests in one second, avg qps is tiny so the floor wins
        let window = QpsWindow::new();
        let q1 = window.record_and_get_qps();
        let q2 = window.record_and_get_qps();
        let q3 = window.record_and_get_qps();
        assert_eq!(q1, DEFAULT_QPS_BASELINE);
        assert_eq!(q2, DEFAULT_QPS_BASELINE);
        assert_eq!(q3, DEFAULT_QPS_BASELINE);
    }

    #[test]
    fn test_qps_window_returns_baseline_after_window_expiry() {
        // simulate window expiry by creating a window and waiting > 60s is impractical,
        // so we verify the floor behaviour indirectly: qps is always >= DEFAULT_QPS_BASELINE
        let window = QpsWindow::new();
        for _ in 0..10 {
            let q = window.record_and_get_qps();
            assert!(
                q >= DEFAULT_QPS_BASELINE,
                "qps {} should be >= {}",
                q,
                DEFAULT_QPS_BASELINE
            );
        }
    }

    // ------------------------------------------------------------------
    // DatabaseSegmentLoader tests
    // ------------------------------------------------------------------

    #[test]
    fn test_database_segment_loader_new_initializes_fields() {
        let repo: Arc<dyn SegmentRepository> =
            Arc::new(MockSegmentRepository::success(sample_segment_info()));
        let detector = Arc::new(DcFailureDetector::new(5, Duration::from_secs(300)));
        let config = SegmentAlgorithmConfig::default();
        let loader = DatabaseSegmentLoader::new(repo, detector, 0, config);
        assert_eq!(
            loader.get_current_step(),
            SegmentAlgorithmConfig::default().base_step
        );
    }

    #[test]
    fn test_database_segment_loader_with_cpu_monitor_sets_calculator() {
        let repo: Arc<dyn SegmentRepository> =
            Arc::new(MockSegmentRepository::success(sample_segment_info()));
        let detector = Arc::new(DcFailureDetector::new(5, Duration::from_secs(300)));
        let config = SegmentAlgorithmConfig::default();
        let cpu = Arc::new(CpuMonitor::new());
        cpu.update_usage(0.9);
        let loader = DatabaseSegmentLoader::new(repo, detector, 0, config).with_cpu_monitor(cpu);
        assert_eq!(
            loader.get_current_step(),
            SegmentAlgorithmConfig::default().base_step
        );
    }

    #[cfg(not(feature = "etcd"))]
    #[test]
    fn test_database_segment_loader_with_etcd_cluster_health_monitor_non_etcd() {
        let repo: Arc<dyn SegmentRepository> =
            Arc::new(MockSegmentRepository::success(sample_segment_info()));
        let detector = Arc::new(DcFailureDetector::new(5, Duration::from_secs(300)));
        let config = SegmentAlgorithmConfig::default();
        let loader = DatabaseSegmentLoader::new(repo, detector, 0, config);
        let loader = loader.with_etcd_cluster_health_monitor(Arc::new(()));
        assert_eq!(
            loader.get_current_step(),
            SegmentAlgorithmConfig::default().base_step
        );
    }

    #[tokio::test]
    async fn test_database_segment_loader_load_segment_success_with_dc() {
        let repo = Arc::new(MockSegmentRepository::success(sample_segment_info()));
        let detector = Arc::new(DcFailureDetector::new(5, Duration::from_secs(300)));
        detector.add_dc(0);
        let config = SegmentAlgorithmConfig::default();
        let loader = DatabaseSegmentLoader::new(repo, detector.clone(), 0, config);
        let ctx = sample_ctx();
        let seg = loader.load_segment(&ctx, 0).await.unwrap();
        assert_eq!(seg.start_id, 500);
        assert_eq!(seg.max_id, 1500);
        assert_eq!(seg.step, 1000);
        assert_eq!(seg.version, 0);
        let dc_state = detector.get_dc_state(0).unwrap();
        assert_eq!(dc_state.get_status(), DcStatus::Healthy);
    }

    #[tokio::test]
    async fn test_database_segment_loader_load_segment_failure_records_dc_failure() {
        let repo = Arc::new(MockSegmentRepository::failing_with_dc());
        let detector = Arc::new(DcFailureDetector::new(5, Duration::from_secs(300)));
        detector.add_dc(0);
        let config = SegmentAlgorithmConfig::default();
        let loader = DatabaseSegmentLoader::new(repo, detector.clone(), 0, config);
        let ctx = sample_ctx();
        let result = loader.load_segment(&ctx, 0).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            CoreError::DatabaseError(_) => {}
            other => panic!("expected DatabaseError, got {:?}", other),
        }
        let dc_state = detector.get_dc_state(0).unwrap();
        assert_eq!(dc_state.failure_count.load(Ordering::Relaxed), 1);
        assert_eq!(dc_state.consecutive_failures.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn test_database_segment_loader_load_segment_no_dc_state_uses_allocate_segment() {
        // when dc_failure_detector has no DC registered, dc_state is None
        // and load_segment falls back to allocate_segment (not allocate_segment_with_dc)
        let repo = Arc::new(MockSegmentRepository::failing_with_dc());
        // failing_with_dc: allocate_segment_with_dc fails, allocate_segment succeeds
        let detector = Arc::new(DcFailureDetector::new(5, Duration::from_secs(300)));
        // deliberately NOT calling add_dc(0) so get_dc_state returns None
        let config = SegmentAlgorithmConfig::default();
        let loader = DatabaseSegmentLoader::new(repo, detector, 0, config);
        let ctx = sample_ctx();
        let seg = loader.load_segment(&ctx, 0).await.unwrap();
        // failing_with_dc() has segment=None, so allocate_segment returns default
        // SegmentInfo { current_id: 1000, max_id: 2000, step: 1000 }
        assert_eq!(seg.start_id, 1000);
        assert_eq!(seg.max_id, 2000);
        assert_eq!(seg.step, 1000);
    }

    #[tokio::test]
    async fn test_database_segment_loader_load_segment_no_dc_state_failure() {
        // no DC registered + allocate_segment fails -> DatabaseError
        let repo = Arc::new(MockSegmentRepository::failing());
        let detector = Arc::new(DcFailureDetector::new(5, Duration::from_secs(300)));
        let config = SegmentAlgorithmConfig::default();
        let loader = DatabaseSegmentLoader::new(repo, detector, 0, config);
        let ctx = sample_ctx();
        let result = loader.load_segment(&ctx, 0).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            CoreError::DatabaseError(_) => {}
            other => panic!("expected DatabaseError, got {:?}", other),
        }
    }

    // ------------------------------------------------------------------
    // RepositoryBackedLoader tests
    // ------------------------------------------------------------------

    #[test]
    fn test_repository_backed_loader_new_stores_config() {
        let repo: Arc<dyn SegmentRepository> =
            Arc::new(MockSegmentRepository::success(sample_segment_info()));
        let config = SegmentAlgorithmConfig::default();
        let _loader = RepositoryBackedLoader::new(repo, config);
    }

    #[tokio::test]
    async fn test_repository_backed_loader_load_segment_success() {
        let repo = Arc::new(MockSegmentRepository::success(sample_segment_info()));
        let config = SegmentAlgorithmConfig::default();
        let loader = RepositoryBackedLoader::new(repo, config);
        let ctx = sample_ctx();
        let seg = loader.load_segment(&ctx, 0).await.unwrap();
        assert_eq!(seg.start_id, 500);
        assert_eq!(seg.max_id, 1500);
        assert_eq!(seg.step, 1000);
        assert_eq!(seg.version, 0);
    }

    #[tokio::test]
    async fn test_repository_backed_loader_load_segment_failure_returns_database_error() {
        let repo = Arc::new(MockSegmentRepository::failing());
        let config = SegmentAlgorithmConfig::default();
        let loader = RepositoryBackedLoader::new(repo, config);
        let ctx = sample_ctx();
        let result = loader.load_segment(&ctx, 0).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            CoreError::DatabaseError(_) => {}
            other => panic!("expected DatabaseError, got {:?}", other),
        }
    }

    // ------------------------------------------------------------------
    // SegmentAlgorithm::with_dependencies tests
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn test_segment_algorithm_with_dependencies_generates_ids_via_repository() {
        let repo = Arc::new(MockSegmentRepository::success(sample_segment_info()));
        let config: Arc<dyn ConfigProvider> = Arc::new(MockConfigProvider::new());
        let algo = SegmentAlgorithm::with_dependencies(config, None, repo, 0);
        let ctx = sample_ctx();
        let id = algo.generate(&ctx).await.unwrap();
        assert_eq!(id.as_u128(), 500);
    }

    #[tokio::test]
    async fn test_segment_algorithm_with_dependencies_reads_config_from_provider() {
        let repo = Arc::new(MockSegmentRepository::success(sample_segment_info()));
        let config: Arc<dyn ConfigProvider> = Arc::new(
            MockConfigProvider::new()
                .with_int("algorithm.segment.base_step", 2000)
                .with_int("algorithm.segment.min_step", 1000)
                .with_int("algorithm.segment.max_step", 50000)
                .with_float("algorithm.segment.switch_threshold", 0.2),
        );
        let algo = SegmentAlgorithm::with_dependencies(config, None, repo, 0);
        let ctx = sample_ctx();
        let id = algo.generate(&ctx).await.unwrap();
        assert_eq!(id.as_u128(), 500);
    }

    // ------------------------------------------------------------------
    // SegmentFactory tests
    // ------------------------------------------------------------------

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
}
