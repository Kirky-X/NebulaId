use prometheus_client::metrics::{counter::Counter, gauge::Gauge};
use prometheus_client::registry::Registry;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

#[derive(Debug, Default)]
pub struct Metrics {
    pub total_generations: AtomicU64,
    pub successful_generations: AtomicU64,
    pub failed_generations: AtomicU64,
    pub generation_latency_ms: AtomicU64,
    pub current_requests: AtomicU64,
    pub segment_buffer_hits: AtomicU64,
    pub segment_buffer_misses: AtomicU64,
    pub snowflake_sequence_exhaustions: AtomicU64,
    pub clock_backward_rejections: AtomicU64,
}

impl Metrics {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record_generation(&self, success: bool, latency_ms: u64) {
        self.total_generations.fetch_add(1, Ordering::SeqCst);
        if success {
            self.successful_generations.fetch_add(1, Ordering::SeqCst);
        } else {
            self.failed_generations.fetch_add(1, Ordering::SeqCst);
        }
        self.generation_latency_ms
            .fetch_add(latency_ms, Ordering::SeqCst);
    }

    pub fn record_segment_buffer_hit(&self) {
        self.segment_buffer_hits.fetch_add(1, Ordering::SeqCst);
    }

    pub fn record_segment_buffer_miss(&self) {
        self.segment_buffer_misses.fetch_add(1, Ordering::SeqCst);
    }

    pub fn record_sequence_exhaustion(&self) {
        self.snowflake_sequence_exhaustions
            .fetch_add(1, Ordering::SeqCst);
    }

    pub fn record_clock_backward(&self) {
        self.clock_backward_rejections
            .fetch_add(1, Ordering::SeqCst);
    }

    pub fn increment_requests(&self) {
        self.current_requests.fetch_add(1, Ordering::SeqCst);
    }

    pub fn decrement_requests(&self) {
        self.current_requests.fetch_sub(1, Ordering::SeqCst);
    }

    pub fn get_total_generations(&self) -> u64 {
        self.total_generations.load(Ordering::SeqCst)
    }

    pub fn get_successful_generations(&self) -> u64 {
        self.successful_generations.load(Ordering::SeqCst)
    }

    pub fn get_failed_generations(&self) -> u64 {
        self.failed_generations.load(Ordering::SeqCst)
    }

    pub fn get_avg_latency_ms(&self) -> f64 {
        let total = self.total_generations.load(Ordering::SeqCst);
        if total == 0 {
            0.0
        } else {
            let latency_sum = self.generation_latency_ms.load(Ordering::SeqCst);
            latency_sum as f64 / total as f64
        }
    }
}

pub struct MetricsCollector {
    metrics: Metrics,
    start_time: Instant,
}

impl MetricsCollector {
    pub fn new() -> Self {
        Self {
            metrics: Metrics::new(),
            start_time: Instant::now(),
        }
    }

    pub fn metrics(&self) -> &Metrics {
        &self.metrics
    }

    pub fn uptime_seconds(&self) -> u64 {
        self.start_time.elapsed().as_secs()
    }

    pub fn reset(&self) {
        self.metrics.total_generations.store(0, Ordering::SeqCst);
        self.metrics
            .successful_generations
            .store(0, Ordering::SeqCst);
        self.metrics.failed_generations.store(0, Ordering::SeqCst);
        self.metrics
            .generation_latency_ms
            .store(0, Ordering::SeqCst);
        self.metrics.segment_buffer_hits.store(0, Ordering::SeqCst);
        self.metrics
            .segment_buffer_misses
            .store(0, Ordering::SeqCst);
        self.metrics
            .snowflake_sequence_exhaustions
            .store(0, Ordering::SeqCst);
        self.metrics
            .clock_backward_rejections
            .store(0, Ordering::SeqCst);
    }
}

#[derive(Debug)]
pub struct PrometheusMetrics {
    registry: Registry,
    total_generations: Counter,
    successful_generations: Counter,
    failed_generations: Counter,
    generation_latency: prometheus_client::metrics::histogram::Histogram,
    current_requests: Gauge,
    segment_buffer_hits: Counter,
    segment_buffer_misses: Counter,
}

impl PrometheusMetrics {
    pub fn new() -> Self {
        let mut registry = Registry::default();

        let total_generations = Counter::default();
        let successful_generations = Counter::default();
        let failed_generations = Counter::default();
        let generation_latency = prometheus_client::metrics::histogram::Histogram::new(
            vec![1.0, 5.0, 10.0, 50.0, 100.0, 500.0, 1000.0].into_iter(),
        );
        let current_requests = Gauge::default();
        let segment_buffer_hits = Counter::default();
        let segment_buffer_misses = Counter::default();

        registry.register(
            "total_generations",
            "Total number of ID generations",
            total_generations.clone(),
        );
        registry.register(
            "successful_generations",
            "Number of successful ID generations",
            successful_generations.clone(),
        );
        registry.register(
            "failed_generations",
            "Number of failed ID generations",
            failed_generations.clone(),
        );
        registry.register(
            "generation_latency_ms",
            "ID generation latency in milliseconds",
            generation_latency.clone(),
        );
        registry.register(
            "current_requests",
            "Number of current requests being processed",
            current_requests.clone(),
        );
        registry.register(
            "segment_buffer_hits",
            "Number of segment buffer cache hits",
            segment_buffer_hits.clone(),
        );
        registry.register(
            "segment_buffer_misses",
            "Number of segment buffer cache misses",
            segment_buffer_misses.clone(),
        );

        Self {
            registry,
            total_generations,
            successful_generations,
            failed_generations,
            generation_latency,
            current_requests,
            segment_buffer_hits,
            segment_buffer_misses,
        }
    }

    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    pub fn record_generation(&self, success: bool, latency_ms: f64) {
        self.total_generations.inc();
        if success {
            self.successful_generations.inc();
        } else {
            self.failed_generations.inc();
        }
        self.generation_latency.observe(latency_ms / 1000.0);
    }

    pub fn record_segment_buffer_hit(&self) {
        self.segment_buffer_hits.inc();
    }

    pub fn record_segment_buffer_miss(&self) {
        self.segment_buffer_misses.inc();
    }

    pub fn increment_requests(&self) {
        self.current_requests.inc();
    }

    pub fn decrement_requests(&self) {
        self.current_requests.dec();
    }
}

impl Default for PrometheusMetrics {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_recording() {
        let metrics = Metrics::new();

        metrics.record_generation(true, 5);
        metrics.record_generation(true, 10);
        metrics.record_generation(false, 3);

        assert_eq!(metrics.get_total_generations(), 3);
        assert_eq!(metrics.get_successful_generations(), 2);
        assert_eq!(metrics.get_failed_generations(), 1);

        let avg_latency = metrics.get_avg_latency_ms();
        assert!((avg_latency - 6.0).abs() < 0.001);
    }

    #[test]
    fn test_segment_buffer_metrics() {
        let metrics = Metrics::new();

        metrics.record_segment_buffer_hit();
        metrics.record_segment_buffer_hit();
        metrics.record_segment_buffer_miss();

        assert_eq!(metrics.segment_buffer_hits.load(Ordering::SeqCst), 2);
        assert_eq!(metrics.segment_buffer_misses.load(Ordering::SeqCst), 1);
    }
}
