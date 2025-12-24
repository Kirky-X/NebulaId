use crate::algorithm::{AlgorithmMetricsSnapshot, GenerateContext, HealthStatus, IdAlgorithm};
use crate::config::{Config, SnowflakeAlgorithmConfig};
use crate::types::{AlgorithmType, CoreError, Id, IdBatch, Result};
use async_trait::async_trait;
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tracing::info;

const DEFAULT_START_TIME: u64 = 1704067200000;

pub struct SnowflakeAlgorithm {
    config: SnowflakeAlgorithmConfig,
    datacenter_id: u8,
    worker_id: u8,
    sequence: AtomicU64,
    last_timestamp: AtomicU64,
    rotation_count: AtomicU8,
    metrics: Arc<SnowflakeMetrics>,
    clock_drift_ms: AtomicU64,
}

struct SnowflakeMetrics {
    total_generated: AtomicU64,
    total_failed: AtomicU64,
    clock_backwards: AtomicU64,
    sequence_overflows: AtomicU64,
}

impl SnowflakeMetrics {
    fn new() -> Self {
        Self {
            total_generated: AtomicU64::new(0),
            total_failed: AtomicU64::new(0),
            clock_backwards: AtomicU64::new(0),
            sequence_overflows: AtomicU64::new(0),
        }
    }
}

impl SnowflakeAlgorithm {
    pub fn new(datacenter_id: u8, worker_id: u8) -> Self {
        Self {
            config: SnowflakeAlgorithmConfig::default(),
            datacenter_id,
            worker_id,
            sequence: AtomicU64::new(0),
            last_timestamp: AtomicU64::new(0),
            rotation_count: AtomicU8::new(0),
            metrics: Arc::new(SnowflakeMetrics::new()),
            clock_drift_ms: AtomicU64::new(0),
        }
    }

    fn get_timestamp() -> u64 {
        let start = SystemTime::UNIX_EPOCH
            .checked_add(Duration::from_millis(DEFAULT_START_TIME))
            .unwrap();

        let now = SystemTime::now()
            .duration_since(start)
            .unwrap_or(Duration::ZERO);

        now.as_millis() as u64
    }

    fn wait_for_next_ms(&self, last_ts: u64) -> u64 {
        loop {
            let current = Self::get_timestamp();
            if current > last_ts {
                return current;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    pub fn generate_id(&self) -> Result<Id> {
        let timestamp = Self::get_timestamp();
        let last_ts = self.last_timestamp.load(Ordering::SeqCst);
        let sequence_mask = self.config.sequence_mask();

        if timestamp < last_ts {
            let drift = last_ts - timestamp;
            self.clock_drift_ms.store(drift, Ordering::Relaxed);
            self.metrics.clock_backwards.fetch_add(1, Ordering::Relaxed);

            if drift > self.config.clock_drift_threshold_ms {
                return Err(CoreError::ClockMovedBackward {
                    last_timestamp: last_ts,
                });
            }

            let wait_ts = self.wait_for_next_ms(last_ts);
            return self.generate_id_with_timestamp(wait_ts, sequence_mask);
        }

        if timestamp == last_ts {
            let seq = self.sequence.fetch_add(1, Ordering::SeqCst);

            if seq & sequence_mask == 0 {
                self.rotation_count.fetch_add(1, Ordering::SeqCst);
                let next_ts = self.wait_for_next_ms(timestamp);
                return self.generate_id_with_timestamp(next_ts, sequence_mask);
            }

            let id = self.construct_id(timestamp, seq & sequence_mask);
            self.metrics.total_generated.fetch_add(1, Ordering::Relaxed);
            return Ok(id);
        }

        self.sequence.store(0, Ordering::SeqCst);
        self.last_timestamp.store(timestamp, Ordering::SeqCst);

        let id = self.construct_id(timestamp, 0);
        self.metrics.total_generated.fetch_add(1, Ordering::Relaxed);
        Ok(id)
    }

    fn generate_id_with_timestamp(&self, timestamp: u64, sequence_mask: u64) -> Result<Id> {
        self.last_timestamp.store(timestamp, Ordering::SeqCst);
        self.sequence.store(0, Ordering::SeqCst);

        let seq = self.sequence.fetch_add(1, Ordering::SeqCst);

        if seq & sequence_mask == 0 && timestamp == self.last_timestamp.load(Ordering::SeqCst) {
            self.metrics
                .sequence_overflows
                .fetch_add(1, Ordering::Relaxed);
            return Err(CoreError::SequenceOverflow { timestamp });
        }

        let id = self.construct_id(timestamp, seq & sequence_mask);
        self.metrics.total_generated.fetch_add(1, Ordering::Relaxed);
        Ok(id)
    }

    fn construct_id(&self, timestamp: u64, sequence: u64) -> Id {
        let dc_id = self.datacenter_id as u64;
        let worker = self.worker_id as u64;

        let id = (timestamp
            << (self.config.datacenter_id_bits
                + self.config.worker_id_bits
                + self.config.sequence_bits))
            | (dc_id << (self.config.worker_id_bits + self.config.sequence_bits))
            | (worker << self.config.sequence_bits)
            | sequence;

        Id::from_u128(id.into())
    }

    pub fn get_datacenter_id(&self) -> u8 {
        self.datacenter_id
    }

    pub fn get_worker_id(&self) -> u8 {
        self.worker_id
    }

    pub fn get_last_timestamp(&self) -> u64 {
        self.last_timestamp.load(Ordering::Relaxed)
    }

    pub fn get_sequence(&self) -> u64 {
        self.sequence.load(Ordering::Relaxed)
    }
}

#[async_trait]
impl IdAlgorithm for SnowflakeAlgorithm {
    async fn generate(&self, _ctx: &GenerateContext) -> Result<Id> {
        self.generate_id()
    }

    async fn batch_generate(&self, _ctx: &GenerateContext, size: usize) -> Result<IdBatch> {
        let mut ids = Vec::with_capacity(size);
        let mut retries = 0;
        const MAX_RETRIES: usize = 100;

        while ids.len() < size && retries < MAX_RETRIES {
            match self.generate_id() {
                Ok(id) => ids.push(id),
                Err(_) => {
                    tokio::time::sleep(tokio::time::Duration::from_millis(1)).await;
                    retries += 1;
                }
            }
        }

        if ids.is_empty() {
            return Err(CoreError::InternalError("Failed to generate IDs after max retries".to_string()));
        }

        Ok(IdBatch::new(ids, AlgorithmType::Snowflake, String::new()))
    }

    fn health_check(&self) -> HealthStatus {
        let drift = self.clock_drift_ms.load(Ordering::Relaxed);
        if drift > self.config.clock_drift_threshold_ms {
            return HealthStatus::Unhealthy(format!(
                "Clock drift {}ms exceeds threshold {}ms",
                drift, self.config.clock_drift_threshold_ms
            ));
        }

        HealthStatus::Healthy
    }

    fn metrics(&self) -> AlgorithmMetricsSnapshot {
        AlgorithmMetricsSnapshot {
            total_generated: self.metrics.total_generated.load(Ordering::Relaxed),
            total_failed: self.metrics.total_failed.load(Ordering::Relaxed),
            current_qps: 0,
            p50_latency_us: 0,
            p99_latency_us: 0,
            cache_hit_rate: 0.0,
        }
    }

    fn algorithm_type(&self) -> AlgorithmType {
        AlgorithmType::Snowflake
    }

    async fn initialize(&mut self, config: &Config) -> Result<()> {
        self.config = config.algorithm.snowflake.clone();
        self.datacenter_id = config.app.dc_id;
        self.worker_id = config.app.worker_id;

        info!(
            "Snowflake algorithm initialized with datacenter_id={}, worker_id={}",
            self.datacenter_id, self.worker_id
        );
        Ok(())
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }
}

pub struct UuidV7Algorithm {
    metrics: Arc<UuidV7Metrics>,
}

struct UuidV7Metrics {
    total_generated: AtomicU64,
    total_failed: AtomicU64,
}

impl Default for UuidV7Metrics {
    fn default() -> Self {
        Self {
            total_generated: AtomicU64::new(0),
            total_failed: AtomicU64::new(0),
        }
    }
}

impl UuidV7Algorithm {
    pub fn new() -> Self {
        Self {
            metrics: Arc::new(UuidV7Metrics::default()),
        }
    }

    pub fn generate_uuid_v7(&self) -> Result<Id> {
        let uuid = uuid::Uuid::now_v7();
        self.metrics.total_generated.fetch_add(1, Ordering::Relaxed);
        Ok(Id::from_uuid_v7(uuid))
    }
}

#[async_trait]
impl IdAlgorithm for UuidV7Algorithm {
    async fn generate(&self, _ctx: &GenerateContext) -> Result<Id> {
        self.generate_uuid_v7()
    }

    async fn batch_generate(&self, _ctx: &GenerateContext, size: usize) -> Result<IdBatch> {
        let mut ids = Vec::with_capacity(size);
        let mut last_error = None;

        for _ in 0..size {
            match self.generate_uuid_v7() {
                Ok(id) => ids.push(id),
                Err(e) => last_error = Some(e),
            }
        }

        if ids.is_empty() {
            return Err(last_error.unwrap_or(CoreError::InternalError("Unknown error".to_string())));
        }

        Ok(IdBatch::new(ids, AlgorithmType::UuidV7, String::new()))
    }

    fn health_check(&self) -> HealthStatus {
        HealthStatus::Healthy
    }

    fn metrics(&self) -> AlgorithmMetricsSnapshot {
        AlgorithmMetricsSnapshot {
            total_generated: self.metrics.total_generated.load(Ordering::Relaxed),
            total_failed: self.metrics.total_failed.load(Ordering::Relaxed),
            current_qps: 0,
            p50_latency_us: 0,
            p99_latency_us: 0,
            cache_hit_rate: 0.0,
        }
    }

    fn algorithm_type(&self) -> AlgorithmType {
        AlgorithmType::UuidV7
    }

    async fn initialize(&mut self, _config: &Config) -> Result<()> {
        Ok(())
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }
}

pub struct UuidV4Algorithm {
    metrics: Arc<UuidV7Metrics>,
}

impl UuidV4Algorithm {
    pub fn new() -> Self {
        Self {
            metrics: Arc::new(UuidV7Metrics::default()),
        }
    }

    pub fn generate_uuid_v4(&self) -> Result<Id> {
        let uuid = uuid::Uuid::new_v4();
        self.metrics.total_generated.fetch_add(1, Ordering::Relaxed);
        Ok(Id::from_uuid_v7(uuid))
    }
}

#[async_trait]
impl IdAlgorithm for UuidV4Algorithm {
    async fn generate(&self, _ctx: &GenerateContext) -> Result<Id> {
        self.generate_uuid_v4()
    }

    async fn batch_generate(&self, _ctx: &GenerateContext, size: usize) -> Result<IdBatch> {
        let mut ids = Vec::with_capacity(size);

        for _ in 0..size {
            ids.push(self.generate_uuid_v4()?);
        }

        Ok(IdBatch::new(ids, AlgorithmType::UuidV4, String::new()))
    }

    fn health_check(&self) -> HealthStatus {
        HealthStatus::Healthy
    }

    fn metrics(&self) -> AlgorithmMetricsSnapshot {
        AlgorithmMetricsSnapshot {
            total_generated: self.metrics.total_generated.load(Ordering::Relaxed),
            total_failed: self.metrics.total_failed.load(Ordering::Relaxed),
            current_qps: 0,
            p50_latency_us: 0,
            p99_latency_us: 0,
            cache_hit_rate: 0.0,
        }
    }

    fn algorithm_type(&self) -> AlgorithmType {
        AlgorithmType::UuidV4
    }

    async fn initialize(&mut self, _config: &Config) -> Result<()> {
        Ok(())
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_snowflake_config_masks() {
        let config = SnowflakeAlgorithmConfig::default();
        assert_eq!(config.datacenter_id_mask(), 0b111);
        assert_eq!(config.worker_id_mask(), 0b11111111);
        assert_eq!(config.sequence_mask(), 0b1111111111);
        assert_eq!(config.timestamp_bits(), 43);
    }

    #[test]
    fn test_snowflake_construct_id() {
        let algo = SnowflakeAlgorithm::new(1, 1);
        let id = algo.construct_id(1000, 5);
        let value = id.as_u128();

        let timestamp_bits = algo.config.datacenter_id_bits
            + algo.config.worker_id_bits
            + algo.config.sequence_bits;
        let worker_shift = algo.config.sequence_bits;
        let dc_shift = algo.config.worker_id_bits + algo.config.sequence_bits;

        let expected = (1000u128 << timestamp_bits)
            | (1u128 << dc_shift)
            | (1u128 << worker_shift)
            | 5u128;
        assert_eq!(value, expected);
    }

    #[tokio::test]
    async fn test_snowflake_generate() {
        let algo = SnowflakeAlgorithm::new(0, 0);
        let id = algo.generate_id().unwrap();
        assert!(id.as_u128() > 0);
    }

    #[tokio::test]
    async fn test_snowflake_uniqueness() {
        let algo = SnowflakeAlgorithm::new(1, 1);
        let mut ids = std::collections::HashSet::new();

        for _ in 0..100 {
            let id = algo.generate_id().unwrap();
            assert!(
                ids.insert(id.as_u128()),
                "Duplicate ID generated: {}",
                id.as_u128()
            );
            tokio::time::sleep(tokio::time::Duration::from_millis(1)).await;
        }
    }

    #[tokio::test]
    async fn test_uuid_v7_generate() {
        let algo = UuidV7Algorithm::new();
        let id = algo.generate_uuid_v7().unwrap();
        let uuid = id.to_uuid_v7();

        assert_eq!(uuid.get_version(), Some(uuid::Version::SortRand));
    }

    #[tokio::test]
    async fn test_uuid_v4_generate() {
        let algo = UuidV4Algorithm::new();
        let id = algo.generate_uuid_v4().unwrap();
        let uuid = id.to_uuid_v7();

        assert_eq!(uuid.get_version(), Some(uuid::Version::Random));
    }
}
