use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use dashmap::DashMap;
use tokio::sync::mpsc;
use tokio::sync::RwLock;
use tracing::{debug, warn};

use crate::cache::RingBuffer;
use crate::types::{CoreError, IdBatch, Result};

pub trait CacheBackend: Send + Sync {
    fn get(&self, key: &str) -> Pin<Box<dyn Future<Output = Result<Option<Vec<u64>>>> + Send>>;
    fn set(
        &self,
        key: &str,
        values: &[u64],
        ttl_seconds: u64,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send>>;
    fn delete(&self, key: &str) -> Pin<Box<dyn Future<Output = Result<()>> + Send>>;
    fn exists(&self, key: &str) -> Pin<Box<dyn Future<Output = Result<bool>> + Send>>;
}

pub struct MultiLevelCache {
    l1_cache: Arc<DashMap<String, Arc<RingBuffer<u64>>>>,
    l2_buffer: DoubleBufferCache,
    l3_backend: Option<Arc<dyn CacheBackend>>,
    l1_capacity: usize,
    l1_watermark_high: f64,
    l1_watermark_low: f64,
    metrics: CacheMetrics,
}

impl MultiLevelCache {
    pub fn new(
        l1_capacity: usize,
        l1_watermark_high: f64,
        l1_watermark_low: f64,
        l2_capacity: usize,
        l2_buffer_count: usize,
        l3_backend: Option<Arc<dyn CacheBackend>>,
    ) -> Self {
        Self {
            l1_cache: Arc::new(DashMap::new()),
            l2_buffer: DoubleBufferCache::new(l2_capacity, l2_buffer_count),
            l3_backend,
            l1_capacity,
            l1_watermark_high,
            l1_watermark_low,
            metrics: CacheMetrics::new(),
        }
    }

    pub async fn get_ids(&self, key: &str, count: usize) -> Result<IdBatch> {
        self.metrics.total_requests.fetch_add(1, Ordering::Relaxed);

        if let Some(l1_buffer) = self.l1_cache.get(key) {
            let mut ids = Vec::with_capacity(count);
            for _ in 0..count {
                if let Some(id) = l1_buffer.pop() {
                    ids.push(id);
                } else {
                    break;
                }
            }

            if !ids.is_empty() {
                self.metrics.l1_hits.fetch_add(1, Ordering::Relaxed);
                self.metrics
                    .l1_items_fetched
                    .fetch_add(ids.len(), Ordering::Relaxed);
                return Ok(IdBatch::from_u64s(&ids));
            }
        }

        self.metrics.l1_misses.fetch_add(1, Ordering::Relaxed);
        debug!("L1 cache miss for key: {}, trying L2", key);

        self.fetch_from_l2_and_refill(key, count).await
    }

    async fn fetch_from_l2_and_refill(&self, key: &str, count: usize) -> Result<IdBatch> {
        let l2_ids = self.l2_buffer.consume(key, count).await;

        if !l2_ids.is_empty() {
            self.metrics.l2_hits.fetch_add(1, Ordering::Relaxed);
            self.refill_l1(key, &l2_ids).await;

            if l2_ids.len() >= count {
                return Ok(IdBatch::from_u64s(&l2_ids[..count]));
            }
        }

        self.metrics.l2_misses.fetch_add(1, Ordering::Relaxed);
        debug!("L2 cache miss for key: {}, trying L3", key);

        self.fetch_from_l3_and_refill(key, count).await
    }

    async fn fetch_from_l3_and_refill(&self, key: &str, count: usize) -> Result<IdBatch> {
        if let Some(ref backend) = self.l3_backend {
            self.metrics.l3_requests.fetch_add(1, Ordering::Relaxed);

            match backend.get(key).await {
                Ok(Some(ids)) => {
                    self.metrics.l3_hits.fetch_add(1, Ordering::Relaxed);
                    let u64_ids: Vec<u64> = ids.into_iter().map(|id| id as u64).collect();
                    self.refill_l1(key, &u64_ids).await;
                    self.l2_buffer.produce(key, &u64_ids).await;

                    let fetch_count = count.min(u64_ids.len());
                    return Ok(IdBatch::from_u64s(&u64_ids[..fetch_count]));
                }
                Ok(None) => {
                    self.metrics.l3_misses.fetch_add(1, Ordering::Relaxed);
                }
                Err(e) => {
                    self.metrics.l3_errors.fetch_add(1, Ordering::Relaxed);
                    warn!("L3 cache error: {:?}", e);
                    return Err(CoreError::CacheError(format!("L3 cache error: {:?}", e)));
                }
            }
        }

        self.metrics
            .cache_misses_total
            .fetch_add(1, Ordering::Relaxed);
        Err(CoreError::CacheError(
            "All cache levels exhausted".to_string(),
        ))
    }

    async fn refill_l1(&self, key: &str, ids: &[u64]) {
        let buffer = self.l1_cache.entry(key.to_string()).or_insert_with(|| {
            Arc::new(RingBuffer::new(
                self.l1_capacity,
                self.l1_watermark_high,
                self.l1_watermark_low,
            ))
        });

        let pushed = buffer.push_batch(ids);
        self.metrics.l1_refills.fetch_add(1, Ordering::Relaxed);
        self.metrics
            .l1_items_refilled
            .fetch_add(pushed, Ordering::Relaxed);

        debug!("Refilled L1 cache for key: {} with {} items", key, pushed);
    }

    pub async fn put_ids(&self, key: &str, ids: &[u64]) -> Result<()> {
        self.refill_l1(key, ids).await;
        self.l2_buffer.produce(key, ids).await;

        if let Some(ref backend) = self.l3_backend {
            if let Err(e) = backend.set(key, ids, 3600).await {
                warn!("Failed to set L3 cache: {:?}", e);
            }
        }

        Ok(())
    }

    pub async fn invalidate(&self, key: &str) -> Result<()> {
        self.l1_cache.remove(key);
        self.l2_buffer.invalidate(key).await;

        if let Some(ref backend) = self.l3_backend {
            backend.delete(key).await?;
        }

        Ok(())
    }

    pub async fn clear(&self) -> Result<()> {
        self.l1_cache.clear();

        for entry in self.l1_cache.iter() {
            entry.value().clear();
        }

        self.l2_buffer.clear().await;

        if self.l3_backend.is_some() {
            warn!("Clearing L3 cache is not implemented for safety reasons");
        }

        Ok(())
    }

    pub fn metrics(&self) -> CacheMetricsSnapshot {
        CacheMetricsSnapshot {
            total_requests: self.metrics.total_requests.load(Ordering::Relaxed),
            l1_hits: self.metrics.l1_hits.load(Ordering::Relaxed),
            l1_misses: self.metrics.l1_misses.load(Ordering::Relaxed),
            l2_hits: self.metrics.l2_hits.load(Ordering::Relaxed),
            l2_misses: self.metrics.l2_misses.load(Ordering::Relaxed),
            l3_requests: self.metrics.l3_requests.load(Ordering::Relaxed),
            l3_hits: self.metrics.l3_hits.load(Ordering::Relaxed),
            l3_misses: self.metrics.l3_misses.load(Ordering::Relaxed),
            l3_errors: self.metrics.l3_errors.load(Ordering::Relaxed),
            cache_misses_total: self.metrics.cache_misses_total.load(Ordering::Relaxed),
            l1_refills: self.metrics.l1_refills.load(Ordering::Relaxed),
            l2_refills: self.metrics.l2_refills.load(Ordering::Relaxed),
            l1_items_fetched: self.metrics.l1_items_fetched.load(Ordering::Relaxed),
            l1_items_refilled: self.metrics.l1_items_refilled.load(Ordering::Relaxed),
        }
    }
}

#[derive(Debug)]
pub struct DoubleBufferCache {
    buffers: Arc<DashMap<String, DoubleBuffer>>,
    producer_tx: mpsc::Sender<(String, Vec<u64>, mpsc::Sender<()>)>,
    consumer_tx: mpsc::Sender<(String, usize, mpsc::Sender<Vec<u64>>)>,
    _producer_handle: tokio::task::JoinHandle<()>,
    _consumer_handle: tokio::task::JoinHandle<()>,
}

struct DoubleBuffer {
    active: Arc<RwLock<Vec<u64>>>,
    next: Arc<RwLock<Vec<u64>>>,
    write_pos: Arc<AtomicUsize>,
    read_pos: Arc<AtomicUsize>,
}

impl std::fmt::Debug for DoubleBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DoubleBuffer")
            .field("write_pos", &self.write_pos.load(Ordering::Relaxed))
            .field("read_pos", &self.read_pos.load(Ordering::Relaxed))
            .finish()
    }
}

impl DoubleBuffer {
    fn new(capacity: usize) -> Self {
        Self {
            active: Arc::new(RwLock::new(Vec::with_capacity(capacity))),
            next: Arc::new(RwLock::new(Vec::with_capacity(capacity))),
            write_pos: Arc::new(AtomicUsize::new(0)),
            read_pos: Arc::new(AtomicUsize::new(0)),
        }
    }

    async fn produce(&self, ids: &[u64]) {
        let mut next = self.next.write().await;
        let pos = self.write_pos.fetch_add(ids.len(), Ordering::Relaxed);

        if pos == 0 && next.is_empty() {
            next.extend_from_slice(ids);
        } else {
            next.extend_from_slice(ids);
        }
    }

    async fn consume(&self, count: usize) -> Vec<u64> {
        let active_len = {
            let active = self.active.read().await;
            let pos = self.read_pos.load(Ordering::Relaxed);
            if pos < active.len() {
                let available = active.len().saturating_sub(pos);
                let to_fetch = count.min(available);
                let start = pos;
                let end = pos + to_fetch;
                self.read_pos.store(end, Ordering::Relaxed);
                return active[start..end].to_vec();
            }
            active.len()
        };

        if active_len == 0 {
            let mut next = self.next.write().await;
            if !next.is_empty() {
                let mut active = self.active.write().await;
                std::mem::swap(&mut *active, &mut *next);
            }
            self.read_pos.store(0, Ordering::Relaxed);
            self.write_pos.store(0, Ordering::Relaxed);
        }

        {
            let active = self.active.read().await;
            let pos = self.read_pos.load(Ordering::Relaxed);
            let available = active.len().saturating_sub(pos);
            let to_fetch = count.min(available);
            if to_fetch == 0 {
                return Vec::new();
            }
            let start = pos;
            let end = pos + to_fetch;
            self.read_pos.store(end, Ordering::Relaxed);
            return active[start..end].to_vec();
        }
    }

    fn is_empty(&self) -> bool {
        let active_len = futures::executor::block_on(async { self.active.read().await.len() });
        active_len == 0
    }

    async fn clear(&self) {
        self.active.write().await.clear();
        self.next.write().await.clear();
        self.read_pos.store(0, Ordering::Relaxed);
        self.write_pos.store(0, Ordering::Relaxed);
    }
}

impl DoubleBufferCache {
    pub fn new(capacity: usize, _buffer_count: usize) -> Self {
        let (producer_tx, mut producer_rx) =
            mpsc::channel::<(String, Vec<u64>, mpsc::Sender<()>)>(100);
        let (consumer_tx, mut consumer_rx) =
            mpsc::channel::<(String, usize, mpsc::Sender<Vec<u64>>)>(100);

        let buffers = Arc::new(DashMap::new());
        let buffers_for_producer = buffers.clone();
        let buffers_for_consumer = buffers.clone();

        let producer_handle = tokio::spawn(async move {
            while let Some((key, ids, confirm_tx)) = producer_rx.recv().await {
                let buffer = buffers_for_producer
                    .entry(key)
                    .or_insert_with(|| DoubleBuffer::new(capacity));
                buffer.produce(&ids).await;
                let _ = confirm_tx.send(()).await;
            }
        });

        let consumer_handle = tokio::spawn(async move {
            while let Some((key, count, resp_tx)) = consumer_rx.recv().await {
                let buffer = buffers_for_consumer.get(&key);
                if let Some(buffer) = buffer {
                    let ids = buffer.consume(count).await;
                    let _ = resp_tx.send(ids).await;
                } else {
                    let _ = resp_tx.send(Vec::new()).await;
                }
            }
        });

        Self {
            buffers,
            producer_tx,
            consumer_tx,
            _producer_handle: producer_handle,
            _consumer_handle: consumer_handle,
        }
    }

    pub async fn produce(&self, key: &str, ids: &[u64]) {
        let (confirm_tx, mut confirm_rx) = mpsc::channel(1);
        let _ = self
            .producer_tx
            .send((key.to_string(), ids.to_vec(), confirm_tx))
            .await;
        let _ = confirm_rx.recv().await;
    }

    pub async fn consume(&self, key: &str, count: usize) -> Vec<u64> {
        let (resp_tx, mut resp_rx) = mpsc::channel(1);
        let _ = self
            .consumer_tx
            .send((key.to_string(), count, resp_tx))
            .await;

        resp_rx.recv().await.unwrap_or_default()
    }

    pub async fn invalidate(&self, key: &str) {
        if let Some((_, buffer)) = self.buffers.remove(key) {
            buffer.clear().await;
        }
    }

    pub async fn clear(&self) {
        for entry in self.buffers.iter() {
            entry.value().clear().await;
        }
        self.buffers.clear();
    }

    pub fn len(&self) -> usize {
        self.buffers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buffers.is_empty()
    }
}

#[derive(Debug, Default)]
pub struct CacheMetrics {
    total_requests: AtomicUsize,
    l1_hits: AtomicUsize,
    l1_misses: AtomicUsize,
    l2_hits: AtomicUsize,
    l2_misses: AtomicUsize,
    l3_requests: AtomicUsize,
    l3_hits: AtomicUsize,
    l3_misses: AtomicUsize,
    l3_errors: AtomicUsize,
    cache_misses_total: AtomicUsize,
    l1_refills: AtomicUsize,
    l2_refills: AtomicUsize,
    l1_items_fetched: AtomicUsize,
    l1_items_refilled: AtomicUsize,
}

impl CacheMetrics {
    fn new() -> Self {
        Self::default()
    }
}

#[derive(Debug, Clone)]
pub struct CacheMetricsSnapshot {
    pub total_requests: usize,
    pub l1_hits: usize,
    pub l1_misses: usize,
    pub l2_hits: usize,
    pub l2_misses: usize,
    pub l3_requests: usize,
    pub l3_hits: usize,
    pub l3_misses: usize,
    pub l3_errors: usize,
    pub cache_misses_total: usize,
    pub l1_refills: usize,
    pub l2_refills: usize,
    pub l1_items_fetched: usize,
    pub l1_items_refilled: usize,
}

impl CacheMetricsSnapshot {
    pub fn l1_hit_rate(&self) -> f64 {
        let total = self.l1_hits + self.l1_misses;
        if total == 0 {
            0.0
        } else {
            self.l1_hits as f64 / total as f64
        }
    }

    pub fn l2_hit_rate(&self) -> f64 {
        let total = self.l2_hits + self.l2_misses;
        if total == 0 {
            0.0
        } else {
            self.l2_hits as f64 / total as f64
        }
    }

    pub fn l3_hit_rate(&self) -> f64 {
        let total = self.l3_hits + self.l3_misses;
        if total == 0 {
            0.0
        } else {
            self.l3_hits as f64 / total as f64
        }
    }

    pub fn overall_hit_rate(&self) -> f64 {
        let hits = self.l1_hits + self.l2_hits + self.l3_hits;
        let total = self.total_requests;
        if total == 0 {
            0.0
        } else {
            hits as f64 / total as f64
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_multi_level_cache_basic() {
        let cache = MultiLevelCache::new(100, 0.8, 0.2, 200, 2, None);

        let ids: Vec<u64> = (1..=10).collect();
        cache.put_ids("test_key", &ids).await.unwrap();

        let batch = cache.get_ids("test_key", 5).await.unwrap();
        assert_eq!(batch.len(), 5);

        let remaining = cache.get_ids("test_key", 10).await.unwrap();
        assert_eq!(remaining.len(), 5);
    }

    #[tokio::test]
    async fn test_multi_level_cache_miss() {
        let cache = MultiLevelCache::new(100, 0.8, 0.2, 200, 2, None);

        let result = cache.get_ids("non_existent", 10).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_multi_level_cache_invalidate() {
        let cache = MultiLevelCache::new(100, 0.8, 0.2, 200, 2, None);

        let ids: Vec<u64> = (1..=10).collect();
        cache.put_ids("test_key", &ids).await.unwrap();

        cache.invalidate("test_key").await.unwrap();

        let result = cache.get_ids("test_key", 5).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_double_buffer_cache() {
        let cache = DoubleBufferCache::new(100, 2);

        let ids: Vec<u64> = (1..=50).collect();
        cache.produce("test_key", &ids).await;

        let consumed = cache.consume("test_key", 10).await;
        assert_eq!(consumed.len(), 10);
        assert_eq!(consumed, vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);

        let consumed2 = cache.consume("test_key", 10).await;
        assert_eq!(consumed2, vec![11, 12, 13, 14, 15, 16, 17, 18, 19, 20]);
    }
}
