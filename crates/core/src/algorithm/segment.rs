use crate::algorithm::{AlgorithmMetricsSnapshot, GenerateContext, HealthStatus, IdAlgorithm};
use crate::config::{Config, SegmentAlgorithmConfig};
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
        self.dc_states.get(&dc_id).map(|v| v.value().clone())
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

    pub async fn start_health_check(&self, check_interval: Duration) {
        let detector = self.clone();
        tokio::spawn(async move {
            loop {
                sleep(check_interval).await;
                detector.check_recovery().await;
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
    local_dc_id: u8,
    etcd_cluster_health_monitor: Option<Arc<EtcdClusterHealthMonitor>>,
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

        Self {
            config: SegmentAlgorithmConfig::default(),
            buffers: DashMap::new(),
            metrics: Arc::new(AlgorithmMetricsInner::default()),
            segment_loader: Arc::new(DefaultSegmentLoader),
            dc_failure_detector,
            local_dc_id,
            etcd_cluster_health_monitor: None,
        }
    }

    pub fn with_loader(mut self, loader: Arc<dyn SegmentLoader + Send + Sync>) -> Self {
        self.segment_loader = loader;
        self
    }

    pub fn with_dc_failure_detector(mut self, detector: Arc<DcFailureDetector>) -> Self {
        self.dc_failure_detector = detector;
        self
    }

    pub fn with_etcd_cluster_health_monitor(
        mut self,
        monitor: Arc<EtcdClusterHealthMonitor>,
    ) -> Self {
        self.etcd_cluster_health_monitor = Some(monitor);
        self
    }

    pub fn get_dc_failure_detector(&self) -> &Arc<DcFailureDetector> {
        &self.dc_failure_detector
    }

    pub fn get_etcd_cluster_health_monitor(&self) -> Option<&Arc<EtcdClusterHealthMonitor>> {
        self.etcd_cluster_health_monitor.as_ref()
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

    fn should_use_etcd(&self) -> bool {
        if let Some(ref monitor) = self.etcd_cluster_health_monitor {
            let status = monitor.get_status();
            match status {
                crate::coordinator::EtcdClusterStatus::Healthy => true,
                crate::coordinator::EtcdClusterStatus::Degraded => true,
                crate::coordinator::EtcdClusterStatus::Failed => false,
            }
        } else {
            true
        }
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

        self.dc_failure_detector
            .start_health_check(Duration::from_secs(60))
            .await;

        Ok(())
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }
}

struct DefaultSegmentLoader;

#[async_trait]
impl SegmentLoader for DefaultSegmentLoader {
    async fn load_segment(&self, _ctx: &GenerateContext, _worker_id: u8) -> Result<SegmentData> {
        Ok(SegmentData {
            start_id: 1,
            max_id: 1000000,
            step: 1000,
            version: 0,
        })
    }
}

pub struct DatabaseSegmentLoader {
    repository: Arc<dyn SegmentRepository>,
    dc_failure_detector: Arc<DcFailureDetector>,
    local_dc_id: u8,
    etcd_cluster_health_monitor: Option<Arc<EtcdClusterHealthMonitor>>,
}

impl DatabaseSegmentLoader {
    pub fn new(
        repository: Arc<dyn SegmentRepository>,
        dc_failure_detector: Arc<DcFailureDetector>,
        local_dc_id: u8,
    ) -> Self {
        Self {
            repository,
            dc_failure_detector,
            local_dc_id,
            etcd_cluster_health_monitor: None,
        }
    }

    pub fn with_etcd_cluster_health_monitor(
        mut self,
        monitor: Arc<EtcdClusterHealthMonitor>,
    ) -> Self {
        self.etcd_cluster_health_monitor = Some(monitor);
        self
    }
}

#[async_trait]
impl SegmentLoader for DatabaseSegmentLoader {
    async fn load_segment(&self, ctx: &GenerateContext, _worker_id: u8) -> Result<SegmentData> {
        let dc_id = self.dc_failure_detector.select_best_dc(self.local_dc_id);
        let dc_state = self.dc_failure_detector.get_dc_state(dc_id);
        let dc_state_clone = dc_state.clone();

        let segment = if dc_state.is_some() {
            self.repository
                .allocate_segment_with_dc(
                    &ctx.workspace_id,
                    &ctx.biz_tag,
                    self.get_step() as i32,
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
                .allocate_segment(&ctx.workspace_id, &ctx.biz_tag, self.get_step() as i32)
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

impl DatabaseSegmentLoader {
    fn get_step(&self) -> u64 {
        1000
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
}
