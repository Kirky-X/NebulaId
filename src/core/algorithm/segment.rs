// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Segment 算法模块：基于号段的 ID 生成，含双缓冲、动态切换与多数据中心健康探测。
//! 生产路径经 [`DbSegmentLoader`] 从仓储分配号段；`SegmentAlgorithm::new` 默认装配
//! [`UnconfiguredSegmentLoader`]（显性报错），需在装配处注入；`cfg(test)` 下默认注入内存造段器。
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

    /// 至多消费 `count` 个：`left = max - current`，`left == 0` 返回 `None`，
    /// 否则消费 `min(count, left)` 返回区间 `[start, start + taken)`。
    ///
    /// 「至多消费」语义是批量补满的前提：旧实现「不足即整单失败」会让
    /// `batch_generate` 无法部分收割段尾剩余，只能整批放弃（静默短批根源）。
    pub fn try_consume(&self, count: u64) -> Option<(u64, u64)> {
        let segment = self.inner.lock();
        let current = segment.current_id.load(Ordering::Relaxed);
        let max = segment.max_id.load(Ordering::Relaxed);

        let left = max.saturating_sub(current);
        if left == 0 {
            return None;
        }
        let taken = count.min(left);
        segment.current_id.store(current + taken, Ordering::Relaxed);
        Some((current, current + taken))
    }
}

pub struct DoubleBuffer {
    current: Arc<ArcSwap<AtomicSegment>>,
    next: Arc<ArcSwapOption<AtomicSegment>>,
    switch_threshold: f64,
    // loading 标记防止多线程并发触发 load_segment
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

    /// CAS 抢占 loading 标记：成功者负责 load_segment，失败者 spin-wait 至 loading=false。
    pub fn try_start_loading(&self) -> bool {
        self.loading
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    /// load_segment 完成（无论成败）后调用，重置 loading。
    pub fn finish_loading(&self) {
        self.loading.store(false, Ordering::Release);
    }

    /// 是否正在加载。
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

/// loading 标志的 RAII 释放守卫：持有期间 `is_loading == true`，drop 时
/// 复位。保证错误与 panic 路径都不会把加载者卡死。
struct LoadingGuard {
    buffer: Arc<DoubleBuffer>,
}

impl Drop for LoadingGuard {
    fn drop(&mut self) {
        self.buffer.finish_loading();
    }
}

/// 最近一次号段加载结果（loader 调用点写入，health_check 反映）。
///
/// 「最近一次」语义：新结果覆盖旧结果——失败后再次成功即恢复 Healthy。
#[derive(Debug, Clone)]
enum LastLoadResult {
    /// 尚未发生过号段加载
    NotYet,
    /// 最近一次加载成功
    Ok(Instant),
    /// 最近一次加载失败（错误摘要）
    Err(String),
}

pub struct SegmentAlgorithm {
    config: SegmentAlgorithmConfig,
    buffers: Arc<RwLock<HashMap<String, Arc<DoubleBuffer>>>>,
    metrics: Arc<AlgorithmMetricsInner>,
    segment_loader: Arc<dyn SegmentLoader + Send + Sync>,
    dc_failure_detector: Arc<DcFailureDetector>,
    /// 最近一次号段加载结果（loader 调用点写入，health_check 反映）。
    last_load_result: Mutex<LastLoadResult>,
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

    /// 是否已装配可用的号段数据源（供健康面区分「空闲」与「不可用」）。
    /// 默认 `true`：实现了 `load_segment` 的装载器即视为已配置。
    fn is_configured(&self) -> bool {
        true
    }
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

        // 默认号段装载器：生产构建为 UnconfiguredSegmentLoader（显性报错，
        // 杜绝内存伪造号段被无感知带入生产），cfg(test) 下为 TestSegmentLoader
        // 以维持既有 crate 内测试行为。
        let segment_loader: Arc<dyn SegmentLoader + Send + Sync> = {
            #[cfg(test)]
            {
                Arc::new(TestSegmentLoader::default())
            }
            #[cfg(not(test))]
            {
                Arc::new(UnconfiguredSegmentLoader)
            }
        };

        Self {
            config: SegmentAlgorithmConfig::default(),
            buffers: Arc::new(RwLock::new(HashMap::new())),
            metrics: Arc::new(AlgorithmMetricsInner::default()),
            segment_loader,
            dc_failure_detector,
            last_load_result: Mutex::new(LastLoadResult::NotYet),
            #[cfg(feature = "etcd")]
            etcd_cluster_health_monitor: None,
            cpu_monitor: None,
            cpu_monitor_task: Arc::new(tokio::sync::Mutex::new(None)),
            shutdown_tx: Arc::new(shutdown_tx),
            health_check_task: Arc::new(tokio::sync::Mutex::new(None)),
        }
    }

    /// 注入自定义号段装载器（pub 化：装配处注入入口）。
    ///
    /// 生产装配方经 mod.rs re-export 拿到 `SegmentAlgorithm` 与
    /// `DbSegmentLoader` 后，用本方法替换 `new()` 的默认装载器。
    pub fn with_segment_loader(mut self, loader: Arc<dyn SegmentLoader + Send + Sync>) -> Self {
        self.segment_loader = loader;
        self
    }

    pub fn with_cpu_monitor(mut self, cpu_monitor: Arc<CpuMonitor>) -> Self {
        self.cpu_monitor = Some(cpu_monitor);
        self
    }

    // initialize 为 inherent 方法：trait 上的 &mut self 破坏对象安全性，
    // 仅在 AlgorithmBuilder::build 中通过具体类型调用。
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

    /// 记录最近一次号段加载结果（loader 调用点写入）：成功记单调时刻，
    /// 失败记错误摘要。`health_check` 据此判定是否非 Healthy。
    fn record_load_result(&self, result: &Result<SegmentData>) {
        let mut last = self.last_load_result.lock();
        match result {
            Ok(_) => *last = LastLoadResult::Ok(Instant::now()),
            Err(e) => *last = LastLoadResult::Err(e.to_string()),
        }
    }

    /// 取段公共流程（单条/批量统一入口）：确保 next 号段就绪并 swap 到 current。
    /// CAS `try_start_loading` 抢占加载权——成功方负责 `load_segment` 并无论
    /// 成败 `finish_loading`；失败方异步轮询等待释放后重读 next。返回 `Ok`
    /// 表示已尽力推进（调用方继续循环重试消费），`Err` 仅透传 loader 错误。
    async fn ensure_next_segment(
        &self,
        buffer: &Arc<DoubleBuffer>,
        ctx: &GenerateContext,
    ) -> Result<()> {
        if buffer.get_next().is_some() {
            buffer.swap();
            return Ok(());
        }

        if buffer.try_start_loading() {
            self.metrics.cache_misses.fetch_add(1, Ordering::Relaxed);
            // RAII 释放 loading：成功、错误（`?` 回收 guard）乃至 panic 路径
            // 都保证复位，杜绝悬挂。guard 在 set_next 之后 drop——loading
            // 释放时 next 已就绪，等待方醒来即可取到。
            let _guard = LoadingGuard {
                buffer: Arc::clone(buffer),
            };
            let load_result = self.segment_loader.load_segment(ctx, 0).await;
            self.record_load_result(&load_result);
            let new_seg = load_result?;
            buffer.set_next(Arc::new(AtomicSegment::new(
                new_seg.start_id,
                new_seg.max_id,
            )));
            buffer.swap();
            return Ok(());
        }

        while buffer.is_loading() {
            sleep(Duration::from_millis(1)).await;
        }
        if buffer.get_next().is_some() {
            buffer.swap();
        }
        Ok(())
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
                self.metrics.cache_hits.fetch_add(1, Ordering::Relaxed);
                return Ok(Id::from_u128(start.into()));
            }

            if buffer.need_switch() {
                self.ensure_next_segment(&buffer, ctx).await?;
            }
        }

        self.metrics.total_failed.fetch_add(1, Ordering::Relaxed);
        let current = buffer.get_current();
        let segment = current.inner.lock();
        let max_id = segment.max_id.load(Ordering::Relaxed);
        Err(CoreError::SegmentExhausted { max_id })
    }

    async fn batch_generate(&self, ctx: &GenerateContext, size: usize) -> Result<IdBatch> {
        // size=0：空批次直接返回（与雪花批量语义一致，不进循环）。
        if size == 0 {
            return Ok(IdBatch::new(
                Vec::new(),
                AlgorithmType::Segment,
                ctx.biz_tag.clone(),
            ));
        }

        let mut ids = Vec::with_capacity(size);
        let key = format!("{}:{}", ctx.workspace_id, ctx.biz_tag);
        let buffer = self.get_or_create_buffer(&key);

        // 连续无进展轮次：try_consume 仅在段耗尽（left==0）时失败，正常
        // 取段-消费循环每轮都有进展；只有 loader 恒定返回空段这类异常才会
        // 连续无进展，超过上限即显性报错，不静默返回短批。跨段补满属于
        // 有进展轮次，不受此上限约束。
        const MAX_BATCH_STALE_ROUNDS: u32 = 8;
        let mut stale_rounds: u32 = 0;

        // 批量硬不变量：返回 Ok 时 ids.len() == size，不静默返回短批
        // （调用方不检查 ids.len() 会误以为拿满）。
        while ids.len() < size {
            let current = buffer.get_current();
            let remaining_needed = (size - ids.len()) as u64;

            if let Some((start, end)) = current.try_consume(remaining_needed) {
                let count = (end - start) as usize;
                ids.extend((start..end).map(|id| Id::from_u128(id.into())));
                self.metrics
                    .total_generated
                    .fetch_add(count as u64, Ordering::Relaxed);
                stale_rounds = 0;
                continue;
            }

            stale_rounds += 1;
            if stale_rounds > MAX_BATCH_STALE_ROUNDS {
                let segment = current.inner.lock();
                let max_id = segment.max_id.load(Ordering::Relaxed);
                drop(segment);
                self.metrics.total_failed.fetch_add(1, Ordering::Relaxed);
                return Err(CoreError::SegmentExhausted { max_id });
            }

            if buffer.need_switch() {
                self.ensure_next_segment(&buffer, ctx).await?;
            }
        }

        Ok(IdBatch::new(
            ids,
            AlgorithmType::Segment,
            ctx.biz_tag.clone(),
        ))
    }

    /// 健康状况反映两路真实故障信号：1. 未装配号段数据源（装载器未配置，每次
    /// 取段必失败）→ Degraded；2. 最近一次号段加载失败且尚无成功恢复 → Degraded。
    ///
    /// 空闲（尚无活动缓冲）不是故障：号段在首个请求时按需分配，冷启动即报
    /// Degraded 会让 `segment` 特性默认开启的部署（主算法 Snowflake/UUID）
    /// 的 `/health` 永远为 Degraded，误导探针与监控。
    fn health_check(&self) -> HealthStatus {
        if !self.segment_loader.is_configured() {
            // 结构性原因先于偶发失败：未装配数据源时每次取段必失败，
            // 消息直接指向注入点（with_segment_loader / with_segment_repository）。
            return HealthStatus::Degraded(
                "Segment loader not configured (inject DbSegmentLoader via with_segment_loader)"
                    .to_string(),
            );
        }
        match &*self.last_load_result.lock() {
            LastLoadResult::Err(err) => {
                HealthStatus::Degraded(format!("Last segment load failed: {err}"))
            }
            LastLoadResult::Ok(at) => {
                // 成功时刻用于观测：debug 级别输出距上次成功加载的毫秒数。
                tracing::debug!(
                    event = "segment_health_check_ok",
                    ms_since_last_load = at.elapsed().as_millis() as u64
                );
                HealthStatus::Healthy
            }
            LastLoadResult::NotYet => HealthStatus::Healthy,
        }
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
            // Segment 算法有段缓存，返回真实命中率。
            cache_hit_rate: Some(hit_rate),
            // 延迟分位数与时钟回拨计数由路由层观测后在
            // AlgorithmRouter::metrics() 合并填充。
            ..Default::default()
        }
    }

    fn algorithm_type(&self) -> AlgorithmType {
        AlgorithmType::Segment
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

/// 未装配数据源的默认号段装载器（生产构建的 `SegmentAlgorithm::new` 默认值）。
///
/// `load_segment` 恒返回 [`CoreError::ConfigurationError`]：内存伪造号段在多实例
/// 部署下必然重复 ID；须在装配处注入 [`DbSegmentLoader`]（经
/// [`SegmentRepository::allocate_segment`] 落库分配）后才可用。
struct UnconfiguredSegmentLoader;

#[async_trait]
impl SegmentLoader for UnconfiguredSegmentLoader {
    async fn load_segment(&self, _ctx: &GenerateContext, _worker_id: u8) -> Result<SegmentData> {
        Err(CoreError::ConfigurationError(t!(
            "error-detail.segment_loader_not_configured"
        )))
    }

    /// 未装配数据源：健康面显性报 Degraded，避免「空闲即健康」掩盖取段必失败。
    fn is_configured(&self) -> bool {
        false
    }
}

/// 数据库号段装载器：调 [`SegmentRepository::allocate_segment`] 原子分配号段，
/// 并把 [`SegmentInfo`] 映射为 [`SegmentData`]。
///
/// `SegmentInfo.current_id` = 分配区间起始值（含）；`max_id` = 区间上界
/// （AtomicSegment 按 `current + count <= max` 消费，实际发号至 `max - 1`）。
/// `worker_id` 参数不参与分配键（仓储按 workspace_id + biz_tag 定位号段行），
/// 仅为兼容 [`SegmentLoader`] trait 签名而保留。
///
/// 动态步长：`clamp(base × (1 + 2×cpu), min, max)`。CPU 越忙步长越大——单次
/// 取段的 DB 往返成本被更多号段摊薄；结果收敛到 i32 值域（DB step 列类型）。
pub fn dynamic_step(base_step: u64, min_step: u64, max_step: u64, cpu_usage: f64) -> i32 {
    let scaled = base_step as f64 * (1.0 + 2.0 * cpu_usage.clamp(0.0, 1.0));
    let clamped = scaled.clamp(min_step as f64, max_step as f64) as u64;
    clamped.min(i32::MAX as u64) as i32
}

pub struct DbSegmentLoader<R: SegmentRepository> {
    repository: Arc<R>,
    step: i32,
    /// 动态步长策略（Some 时覆盖固定 `step`）。
    dynamic: Option<DynamicStepPolicy>,
}

/// 动态步长策略参数快照（来自 Segment 配置 + CPU 监视器）。
struct DynamicStepPolicy {
    base_step: u64,
    min_step: u64,
    max_step: u64,
    cpu_monitor: Arc<CpuMonitor>,
}

impl<R: SegmentRepository> DbSegmentLoader<R> {
    /// 默认分配步长：与 `SegmentAlgorithmConfig::default().base_step`（1000）对齐。
    pub const DEFAULT_STEP: i32 = 1000;

    /// 以默认步长构建装载器。
    pub fn new(repository: Arc<R>) -> Self {
        Self {
            repository,
            step: Self::DEFAULT_STEP,
            dynamic: None,
        }
    }

    /// 以指定步长构建装载器（步长应落在 biz_tag 的 base_step～max_step 约束内，
    /// 超大步长会放大 DB 崩溃时的号段空洞）。
    pub fn with_step(repository: Arc<R>, step: i32) -> Self {
        Self {
            repository,
            step,
            dynamic: None,
        }
    }

    /// 动态步长装载器：每次取段按 CPU 使用率在 [min_step, max_step] 内计算步长。
    pub fn with_dynamic_step(
        repository: Arc<R>,
        segment_config: SegmentAlgorithmConfig,
        cpu_monitor: Arc<CpuMonitor>,
    ) -> Self {
        Self {
            repository,
            step: Self::DEFAULT_STEP,
            dynamic: Some(DynamicStepPolicy {
                base_step: segment_config.base_step,
                min_step: segment_config.min_step,
                max_step: segment_config.max_step,
                cpu_monitor,
            }),
        }
    }
}

#[async_trait]
impl<R: SegmentRepository + 'static> SegmentLoader for DbSegmentLoader<R> {
    async fn load_segment(&self, ctx: &GenerateContext, _worker_id: u8) -> Result<SegmentData> {
        let step = match &self.dynamic {
            Some(policy) => dynamic_step(
                policy.base_step,
                policy.min_step,
                policy.max_step,
                policy.cpu_monitor.get_usage(),
            ),
            None => self.step,
        };
        let info = self
            .repository
            .allocate_segment(&ctx.workspace_id, &ctx.biz_tag, step)
            .await?;

        // i64 → u64 防御性转换：正常路径两者恒为正，异常脏数据（负数）
        // 收敛为 0 而非按位回绕成超大号段。
        let start_id = u64::try_from(info.current_id).unwrap_or(0);
        let max_id = u64::try_from(info.max_id).unwrap_or(0);

        Ok(SegmentData { start_id, max_id })
    }
}

/// 内存造段器（仅测试构建）：以当前秒级时间戳为基址伪造号段，保证单进程内
/// 多次取段互不重叠。仅供测试显式注入；生产构建由
/// [`UnconfiguredSegmentLoader`] 兜底报错。
#[cfg(test)]
#[derive(Default)]
struct TestSegmentLoader {}

#[cfg(test)]
#[async_trait]
impl SegmentLoader for TestSegmentLoader {
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
// SegmentFactory impl 位于本文件（具体类型所属文件）。
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

        // 至多消费语义：剩余 984 请求 1000 → 部分收割 984 个（旧实现整单 None）。
        let (start, end) = segment.try_consume(1000).unwrap();
        assert_eq!(start, 16);
        assert_eq!(end, 1000);

        // 剩余归零后返回 None。
        assert!(segment.try_consume(1).is_none());
        assert!(segment.try_consume(1000).is_none());
    }

    /// 剩余为 0 时无论请求多少都返回 None（含 count=0 的退化输入）。
    #[test]
    fn test_atomic_segment_try_consume_exhausted_returns_none() {
        let segment = AtomicSegment::new(0, 0);
        assert!(segment.try_consume(1).is_none());
        assert!(segment.try_consume(0).is_none());

        let segment = AtomicSegment::new(5, 5);
        assert!(segment.try_consume(1).is_none());
    }

    /// count=0 在剩余充足时返回空区间（不发号也不报错）。
    #[test]
    fn test_atomic_segment_try_consume_zero_count_returns_empty_range() {
        let segment = AtomicSegment::new(1, 1000);
        let (start, end) = segment.try_consume(0).unwrap();
        assert_eq!((start, end), (1, 1));
    }

    /// 并发消费总量恰等于段容量：16 任务并发 try_consume(37) 至段耗尽，
    /// 收割总数 == 段容量（部分收割不丢号、不重号）。
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn test_atomic_segment_concurrent_consume_no_overlap() {
        let segment = Arc::new(AtomicSegment::new(0, 1000));
        const TASKS: usize = 16;
        let handles: Vec<_> = (0..TASKS)
            .map(|_| {
                let segment = Arc::clone(&segment);
                tokio::spawn(async move {
                    let mut taken: u64 = 0;
                    while let Some((start, end)) = segment.try_consume(37) {
                        assert!(start < end, "部分收割不得返回空区间");
                        taken += end - start;
                    }
                    taken
                })
            })
            .collect();

        let mut total: u64 = 0;
        for h in handles {
            total += h.await.unwrap();
        }
        assert_eq!(total, 1000, "并发收割总量必须恰等于段容量");
    }

    #[tokio::test]
    async fn test_segment_algorithm_generate() {
        // 显式注入内存造段器（依赖造段行为的测试不再隐式依赖默认装配）。
        let algo =
            SegmentAlgorithm::new(0).with_segment_loader(Arc::new(TestSegmentLoader::default()));
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
        let algo =
            SegmentAlgorithm::new(0).with_segment_loader(Arc::new(TestSegmentLoader::default()));
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

    /// size=0 直接返回空批次（与雪花批量语义一致，不进循环不报错）。
    #[tokio::test]
    async fn test_segment_algorithm_batch_generate_empty_returns_empty_batch() {
        let algo =
            SegmentAlgorithm::new(0).with_segment_loader(Arc::new(TestSegmentLoader::default()));
        let ctx = sample_ctx();
        let batch = algo.batch_generate(&ctx, 0).await.unwrap();
        assert!(batch.ids.is_empty());
        assert_eq!(batch.algorithm, AlgorithmType::Segment);
    }

    /// 计数造段器：按调用次序发放互接续的 [n*1000+1, n*1000+1001) 号段，
    /// 记录调用次数（供跨段补满与并发合并断言）。
    struct CountingSegmentLoader {
        calls: std::sync::atomic::AtomicU64,
    }

    impl CountingSegmentLoader {
        fn new() -> Self {
            Self {
                calls: std::sync::atomic::AtomicU64::new(0),
            }
        }

        fn call_count(&self) -> u64 {
            self.calls.load(Ordering::Relaxed)
        }
    }

    #[async_trait]
    impl SegmentLoader for CountingSegmentLoader {
        async fn load_segment(
            &self,
            _ctx: &GenerateContext,
            _worker_id: u8,
        ) -> Result<SegmentData> {
            let n = self.calls.fetch_add(1, Ordering::Relaxed);
            let start = n * 1000 + 1;
            Ok(SegmentData {
                start_id: start,
                max_id: start + 1000,
            })
        }
    }

    /// 批量硬不变量：请求 2500、单段步长 1000 → 跨 ≥3 个号段补满，
    /// ID 连续、无重复、数量精确等于请求量（不静默短批）。
    #[tokio::test]
    async fn test_batch_generate_fills_across_multiple_segments() {
        let loader = Arc::new(CountingSegmentLoader::new());
        let algo = SegmentAlgorithm::new(0).with_segment_loader(loader.clone());
        let ctx = sample_ctx();

        let batch = algo.batch_generate(&ctx, 2500).await.unwrap();

        assert_eq!(batch.ids.len(), 2500, "批量必须精确补满请求量");
        let mut seen = std::collections::HashSet::new();
        for (i, id) in batch.ids.iter().enumerate() {
            let v = id.as_u128();
            assert!(seen.insert(v), "ID 重复：{v}");
            if i > 0 {
                assert_eq!(v, batch.ids[i - 1].as_u128() + 1, "批量内 ID 必须连续");
            }
        }
        assert!(
            loader.call_count() >= 3,
            "2500 个 ID 至少跨 3 个步长 1000 的号段，实际取段 {} 次",
            loader.call_count()
        );
    }

    /// 并发批量取段合并：8 任务对同一 key 各批量 200（总需求 1600、步长
    /// 1000 → 至少 2 段），CAS 合并下 loader 调用数必须严格小于任务数
    /// （旧路径无 CAS 时会各自打 DB、set_next 相互覆盖浪费整段）。
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn test_concurrent_batch_generate_merges_segment_loads() {
        let loader = Arc::new(CountingSegmentLoader::new());
        let algo = Arc::new(SegmentAlgorithm::new(0).with_segment_loader(loader.clone()));
        let ctx = sample_ctx();

        let handles: Vec<_> = (0..8)
            .map(|_| {
                let algo = Arc::clone(&algo);
                let ctx = ctx.clone();
                tokio::spawn(async move { algo.batch_generate(&ctx, 200).await })
            })
            .collect();

        for h in handles {
            let batch = h.await.unwrap().expect("并发批量必须成功");
            assert_eq!(batch.ids.len(), 200, "每个批量必须精确补满");
        }
        let calls = loader.call_count();
        assert!(
            calls < 8,
            "CAS 合并下 loader 调用数必须小于并发任务数，实际 {}",
            calls
        );
    }

    /// loader 恒错时批量把原始错误透传（不吞成 SegmentExhausted，也不短批）。
    #[tokio::test]
    async fn test_batch_generate_loader_error_passthrough() {
        let algo = SegmentAlgorithm::new(0).with_segment_loader(Arc::new(FailingSegmentLoader));
        let ctx = sample_ctx();
        match algo.batch_generate(&ctx, 3).await {
            Err(CoreError::DatabaseError(msg, _)) => {
                assert!(msg.contains("segment db unreachable"));
            }
            other => panic!("expected DatabaseError passthrough, got {:?}", other),
        }
    }

    /// 空闲（无活动缓冲）不是故障：未使用过的算法在冷启动时也应报 Healthy，
    /// 否则默认开启 segment 特性的部署 `/health` 恒 Degraded。
    #[test]
    fn test_segment_algorithm_health_check_is_healthy_when_idle() {
        let algo = SegmentAlgorithm::new(0);
        assert_eq!(algo.health_check(), HealthStatus::Healthy);
    }

    /// 生产默认装载器（未装配数据源）必须报 Degraded：取段必失败不能被空闲掩盖。
    #[test]
    fn test_segment_algorithm_health_check_degraded_when_loader_unconfigured() {
        let algo =
            SegmentAlgorithm::new(0).with_segment_loader(Arc::new(UnconfiguredSegmentLoader));
        match algo.health_check() {
            HealthStatus::Degraded(msg) => {
                assert!(msg.contains("Segment loader not configured"), "got: {msg}")
            }
            other => panic!("expected Degraded, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_segment_algorithm_health_check_returns_healthy_when_buffer_exists() {
        let algo =
            SegmentAlgorithm::new(0).with_segment_loader(Arc::new(TestSegmentLoader::default()));
        let ctx = sample_ctx();
        let _ = algo.generate(&ctx).await.unwrap();
        let status = algo.health_check();
        assert!(matches!(status, HealthStatus::Healthy));
    }

    // ===== health_check 反映最近号段加载结果=====

    /// 恒失败装载器：注入 load 错误以驱动 health_check 降级路径。
    struct FailingSegmentLoader;

    #[async_trait]
    impl SegmentLoader for FailingSegmentLoader {
        async fn load_segment(
            &self,
            _ctx: &GenerateContext,
            _worker_id: u8,
        ) -> Result<SegmentData> {
            Err(CoreError::DatabaseError(
                "segment db unreachable".to_string(),
                None,
            ))
        }
    }

    /// 可切换装载器：按 `fail` 标志注入失败/成功，驱动「失败 → 恢复」转换。
    struct SwitchableSegmentLoader {
        fail: std::sync::atomic::AtomicBool,
    }

    impl SwitchableSegmentLoader {
        fn new(fail: bool) -> Self {
            Self {
                fail: std::sync::atomic::AtomicBool::new(fail),
            }
        }

        fn set_fail(&self, fail: bool) {
            self.fail.store(fail, Ordering::Relaxed);
        }
    }

    #[async_trait]
    impl SegmentLoader for SwitchableSegmentLoader {
        async fn load_segment(
            &self,
            _ctx: &GenerateContext,
            _worker_id: u8,
        ) -> Result<SegmentData> {
            if self.fail.load(Ordering::Relaxed) {
                Err(CoreError::DatabaseError(
                    "transient load failure".to_string(),
                    None,
                ))
            } else {
                Ok(SegmentData {
                    start_id: 1,
                    max_id: 1000,
                })
            }
        }
    }

    /// loader 注入错误后：generate 失败且 health_check 非 Healthy，
    /// 降级消息必须携带「最近一次加载失败」与错误摘要。
    #[tokio::test]
    async fn test_health_check_degrades_after_loader_failure() {
        let algo = SegmentAlgorithm::new(0).with_segment_loader(Arc::new(FailingSegmentLoader));
        let ctx = sample_ctx();

        assert!(algo.generate(&ctx).await.is_err());
        match algo.health_check() {
            HealthStatus::Degraded(msg) => {
                assert!(
                    msg.contains("Last segment load failed")
                        && msg.contains("segment db unreachable"),
                    "degraded message must carry load error, got: {msg}"
                );
            }
            other => panic!("expected Degraded after load failure, got {other:?}"),
        }
    }

    /// 延迟造段器：等待指定毫秒后返回有效号段（模拟 DB 往返延迟）。
    struct SlowSegmentLoader {
        delay_ms: u64,
    }

    #[async_trait]
    impl SegmentLoader for SlowSegmentLoader {
        async fn load_segment(
            &self,
            _ctx: &GenerateContext,
            _worker_id: u8,
        ) -> Result<SegmentData> {
            tokio::time::sleep(Duration::from_millis(self.delay_ms)).await;
            Ok(SegmentData {
                start_id: 1,
                max_id: 10_000,
            })
        }
    }

    /// loading 等待必须异步让出执行权：current_thread runtime（#[tokio::test]
    /// 默认）下并发生成、loader 延迟 50ms——等待方若用同步自旋会占死唯一
    /// 线程，loader 任务永远得不到调度而死锁；改为 sleep 轮询后两个请求
    /// 都必须成功。
    #[tokio::test]
    async fn test_generate_loading_wait_yields_on_current_thread_runtime() {
        let algo = SegmentAlgorithm::new(0)
            .with_segment_loader(Arc::new(SlowSegmentLoader { delay_ms: 50 }));
        let ctx = sample_ctx();
        let (a, b) = tokio::join!(algo.generate(&ctx), algo.generate(&ctx));
        assert!(a.is_ok(), "首个请求必须成功: {:?}", a.err());
        assert!(b.is_ok(), "等待方不得死锁: {:?}", b.err());
    }

    /// loader 恢复成功后：最近一次成功加载覆盖失败记录，health_check 恢复 Healthy。
    #[tokio::test]
    async fn test_health_check_recovers_to_healthy_after_successful_reload() {
        let loader = Arc::new(SwitchableSegmentLoader::new(true));
        let algo = SegmentAlgorithm::new(0).with_segment_loader(loader.clone());
        let ctx = sample_ctx();

        // 失败阶段：generate 报错且 health 非 Healthy。
        assert!(algo.generate(&ctx).await.is_err());
        assert!(matches!(algo.health_check(), HealthStatus::Degraded(_)));

        // 恢复阶段：注入成功 → 成功加载覆盖失败 → Healthy。
        loader.set_fail(false);
        assert!(algo.generate(&ctx).await.is_ok());
        assert!(matches!(algo.health_check(), HealthStatus::Healthy));
    }

    /// batch_generate 的 loader 失败同样计入最近加载结果（batch 调用点记录）。
    #[tokio::test]
    async fn test_batch_generate_loader_failure_recorded_for_health() {
        let algo = SegmentAlgorithm::new(0).with_segment_loader(Arc::new(FailingSegmentLoader));
        let ctx = sample_ctx();

        assert!(algo.batch_generate(&ctx, 3).await.is_err());
        assert!(matches!(algo.health_check(), HealthStatus::Degraded(_)));
    }

    /// 未装配数据源：health_check 在取段前即报 Degraded，消息指向注入点。
    #[tokio::test]
    async fn test_unconfigured_loader_error_counts_as_load_failure_for_health() {
        let algo =
            SegmentAlgorithm::new(0).with_segment_loader(Arc::new(UnconfiguredSegmentLoader));
        let ctx = sample_ctx();

        assert!(algo.generate(&ctx).await.is_err());
        match algo.health_check() {
            HealthStatus::Degraded(msg) => {
                assert!(
                    msg.contains("DbSegmentLoader"),
                    "message must point to DbSegmentLoader injection, got: {msg}"
                );
            }
            other => panic!("expected Degraded, got {other:?}"),
        }
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
        let algo =
            SegmentAlgorithm::new(0).with_segment_loader(Arc::new(TestSegmentLoader::default()));
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

    #[test]
    fn test_dc_health_state_transitions() {
        let state = DcHealthState::new(3);
        assert_eq!(state.dc_id, 3);
        assert_eq!(state.get_status(), DcStatus::Healthy);
        state.set_status(DcStatus::Degraded);
        assert_eq!(state.get_status(), DcStatus::Degraded);
        state.set_status(DcStatus::Failed);
        assert_eq!(state.get_status(), DcStatus::Failed);
    }

    #[tokio::test]
    async fn test_check_recovery_degrades_stale_failed_dc() {
        let detector = DcFailureDetector::new(3, Duration::from_secs(1));
        detector.add_dc(1);
        let failed = DcHealthState::new(2);
        failed.set_status(DcStatus::Failed);
        *failed.last_success.lock() = Instant::now() - Duration::from_secs(3600);
        detector.dc_states.write().insert(2, Arc::new(failed));
        detector.check_recovery().await;
        let states = detector.dc_states.read();
        assert_eq!(
            states.get(&2).expect("dc 2").get_status(),
            DcStatus::Degraded
        );
        assert_eq!(
            states.get(&1).expect("dc 1").get_status(),
            DcStatus::Healthy
        );
    }

    // ===== DbSegmentLoader=====

    use crate::core::types::SegmentInfo;

    /// 受控 mock 仓储：`allocate_segment` 返回预设结果并记录调用参数。
    struct MockSegmentRepository {
        result: Result<SegmentInfo>,
        calls: parking_lot::Mutex<Vec<(String, String, i32)>>,
    }

    impl MockSegmentRepository {
        fn returning(result: Result<SegmentInfo>) -> Self {
            Self {
                result,
                calls: parking_lot::Mutex::new(Vec::new()),
            }
        }

        fn recorded_calls(&self) -> Vec<(String, String, i32)> {
            self.calls.lock().clone()
        }
    }

    fn sample_segment_info(current_id: i64, max_id: i64) -> SegmentInfo {
        SegmentInfo {
            id: 1,
            workspace_id: "ws".to_string(),
            biz_tag: "tag".to_string(),
            current_id,
            max_id,
            step: 1000,
            delta: 1,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[async_trait]
    impl SegmentRepository for MockSegmentRepository {
        async fn get_segment(
            &self,
            _workspace_id: &str,
            _biz_tag: &str,
        ) -> Result<Option<SegmentInfo>> {
            unimplemented!("DbSegmentLoader 单测只走 allocate_segment");
        }

        async fn allocate_segment(
            &self,
            workspace_id: &str,
            biz_tag: &str,
            step: i32,
        ) -> Result<SegmentInfo> {
            self.calls
                .lock()
                .push((workspace_id.to_string(), biz_tag.to_string(), step));
            self.result.clone()
        }

        async fn allocate_segment_with_dc(
            &self,
            _workspace_id: &str,
            _biz_tag: &str,
            _step: i32,
            _dc_id: i32,
        ) -> Result<SegmentInfo> {
            unimplemented!("DbSegmentLoader 单测只走 allocate_segment");
        }

        async fn update_segment(
            &self,
            _workspace_id: &str,
            _biz_tag: &str,
            _dc_id: i32,
            _current_id: i64,
            _max_id: i64,
        ) -> Result<()> {
            unimplemented!("DbSegmentLoader 单测只走 allocate_segment");
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
            unimplemented!("DbSegmentLoader 单测只走 allocate_segment");
        }

        async fn list_segments(&self, _workspace_id: &str) -> Result<Vec<SegmentInfo>> {
            unimplemented!("DbSegmentLoader 单测只走 allocate_segment");
        }

        async fn delete_segment(&self, _workspace_id: &str, _biz_tag: &str) -> Result<()> {
            unimplemented!("DbSegmentLoader 单测只走 allocate_segment");
        }
    }

    /// 动态步长公式：cpu=0 → base；cpu=1 → 3×base；min/max 双边界 clamp；
    /// 越界 cpu 输入（>1 / <0）收敛到 [0,1]。
    #[test]
    fn test_dynamic_step_formula_and_clamps() {
        assert_eq!(dynamic_step(1000, 500, 100_000, 0.0), 1000);
        assert_eq!(dynamic_step(1000, 500, 100_000, 0.1), 1200);
        assert_eq!(dynamic_step(1000, 500, 100_000, 1.0), 3000);
        assert_eq!(dynamic_step(1000, 2000, 100_000, 0.0), 2000, "下界 clamp");
        assert_eq!(dynamic_step(1000, 500, 2500, 1.0), 2500, "上界 clamp");
        assert_eq!(dynamic_step(1000, 500, 100_000, 5.0), 3000, "cpu>1 收敛");
        assert_eq!(dynamic_step(1000, 500, 100_000, -1.0), 1000, "cpu<0 收敛");
    }

    /// 动态装载器：cpu=1.0 时取段步长应为 max_step（CpuMonitor 采样真正
    /// 参与了取段决策）。
    #[tokio::test]
    async fn test_dynamic_loader_computes_step_from_cpu_monitor() {
        let monitor = Arc::new(CpuMonitor::new());
        monitor.update_usage(1.0);
        let cfg = SegmentAlgorithmConfig {
            base_step: 1000,
            min_step: 500,
            max_step: 2500,
            switch_threshold: 0.1,
        };
        let repo = Arc::new(MockSegmentRepository::returning(Ok(sample_segment_info(
            1000, 2000,
        ))));
        let loader = DbSegmentLoader::with_dynamic_step(repo.clone(), cfg, monitor);

        let _ = loader.load_segment(&sample_ctx(), 0).await.unwrap();
        assert_eq!(
            repo.recorded_calls()[0].2,
            2500,
            "cpu=1.0 时取段步长必须是 max_step"
        );
    }

    #[tokio::test]
    async fn test_db_segment_loader_maps_segment_info_to_segment_data() {
        let repo = Arc::new(MockSegmentRepository::returning(Ok(sample_segment_info(
            1000, 2000,
        ))));
        let loader = DbSegmentLoader::new(repo.clone());

        let data = loader.load_segment(&sample_ctx(), 7).await.unwrap();

        // SegmentInfo.current_id = 分配区间起始（含），max_id = 上界，直接映射。
        assert_eq!(data.start_id, 1000);
        assert_eq!(data.max_id, 2000);

        // 必须按 ctx 的 workspace_id + biz_tag、默认步长调 allocate_segment。
        let calls = repo.recorded_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0], ("ws".to_string(), "tag".to_string(), 1000));
    }

    #[tokio::test]
    async fn test_db_segment_loader_uses_injected_step() {
        let repo = Arc::new(MockSegmentRepository::returning(Ok(sample_segment_info(
            1, 501,
        ))));
        let loader = DbSegmentLoader::with_step(repo.clone(), 500);

        let data = loader.load_segment(&sample_ctx(), 0).await.unwrap();
        assert_eq!(data.start_id, 1);
        assert_eq!(data.max_id, 501);
        assert_eq!(repo.recorded_calls()[0].2, 500);
    }

    #[tokio::test]
    async fn test_db_segment_loader_propagates_repository_error() {
        let repo = Arc::new(MockSegmentRepository::returning(Err(
            CoreError::DatabaseError("allocate boom".to_string(), None),
        )));
        let loader = DbSegmentLoader::new(repo);

        let result = loader.load_segment(&sample_ctx(), 0).await;
        match result {
            Err(CoreError::DatabaseError(msg, _)) => {
                assert!(msg.contains("allocate boom"), "error must pass through");
            }
            other => panic!("expected DatabaseError passthrough, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_db_segment_loader_clamps_negative_dirty_values_to_zero() {
        // 防御性转换：脏数据（负数）收敛为 0，而非按位回绕成超大号段。
        let repo = Arc::new(MockSegmentRepository::returning(Ok(sample_segment_info(
            -5, -1,
        ))));
        let loader = DbSegmentLoader::new(repo);

        let data = loader.load_segment(&sample_ctx(), 0).await.unwrap();
        assert_eq!(data.start_id, 0);
        assert_eq!(data.max_id, 0);
    }

    #[tokio::test]
    async fn test_unconfigured_segment_loader_returns_configuration_error() {
        let loader = UnconfiguredSegmentLoader;
        let result = loader.load_segment(&sample_ctx(), 0).await;
        match result {
            Err(CoreError::ConfigurationError(msg)) => {
                assert!(
                    msg.contains("DbSegmentLoader"),
                    "message must point to DbSegmentLoader injection, got: {msg}"
                );
            }
            other => panic!("expected ConfigurationError, got {other:?}"),
        }
    }
}
