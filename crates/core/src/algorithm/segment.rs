use crate::algorithm::{AlgorithmMetricsSnapshot, GenerateContext, HealthStatus, IdAlgorithm};
use crate::config::{Config, SegmentAlgorithmConfig};
use crate::types::{AlgorithmType, CoreError, Id, IdBatch, Result};
use async_trait::async_trait;
use dashmap::DashMap;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;

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

impl SegmentAlgorithm {
    pub fn new() -> Self {
        Self {
            config: SegmentAlgorithmConfig::default(),
            buffers: DashMap::new(),
            metrics: Arc::new(AlgorithmMetricsInner::default()),
            segment_loader: Arc::new(DefaultSegmentLoader),
        }
    }

    pub fn with_loader(mut self, loader: Arc<dyn SegmentLoader + Send + Sync>) -> Self {
        self.segment_loader = loader;
        self
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
        let mut last_error = None;

        for _ in 0..size {
            match self.generate(ctx).await {
                Ok(id) => ids.push(id),
                Err(e) => last_error = Some(e),
            }
        }

        if ids.is_empty() {
            return Err(last_error.unwrap_or(CoreError::InternalError("Unknown error".to_string())));
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
        let algo = SegmentAlgorithm::new();
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
