#![allow(dead_code)]

use crate::algorithm::traits::{
    AlgorithmMetricsSnapshot, GenerateContext, HealthStatus, IdAlgorithm,
};
use crate::config::Config;
use crate::types::id::Id;
use crate::types::{AlgorithmType, IdBatch, Result};
use async_trait::async_trait;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use uuid::Uuid;

struct UuidMetrics {
    total_generated: AtomicU64,
    total_failed: AtomicU64,
}

impl UuidMetrics {
    fn new() -> Self {
        Self {
            total_generated: AtomicU64::new(0),
            total_failed: AtomicU64::new(0),
        }
    }
}

#[async_trait]
pub trait UuidV7Generator: Send + Sync {
    async fn generate_v7(&self) -> Result<Uuid>;
}

pub struct UuidV7Impl {
    metrics: Arc<UuidMetrics>,
}

impl UuidV7Impl {
    pub fn new() -> Self {
        Self {
            metrics: Arc::new(UuidMetrics::new()),
        }
    }

    pub fn generate() -> Result<Uuid> {
        let uuid = Uuid::now_v7();
        Ok(uuid)
    }
}

impl Default for UuidV7Impl {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl IdAlgorithm for UuidV7Impl {
    async fn generate(&self, _ctx: &GenerateContext) -> Result<Id> {
        let uuid = Uuid::now_v7();
        self.metrics.total_generated.fetch_add(1, Ordering::Relaxed);
        Ok(Id::from_uuid_v7(uuid))
    }

    async fn batch_generate(&self, _ctx: &GenerateContext, size: usize) -> Result<IdBatch> {
        let mut ids = Vec::with_capacity(size);

        for _ in 0..size {
            ids.push(self.generate(_ctx).await?);
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

pub struct UuidV4Impl {
    metrics: Arc<UuidMetrics>,
}

impl UuidV4Impl {
    pub fn new() -> Self {
        Self {
            metrics: Arc::new(UuidMetrics::new()),
        }
    }

    pub fn generate() -> Result<Uuid> {
        let uuid = Uuid::new_v4();
        Ok(uuid)
    }
}

impl Default for UuidV4Impl {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl IdAlgorithm for UuidV4Impl {
    async fn generate(&self, _ctx: &GenerateContext) -> Result<Id> {
        let uuid = Uuid::new_v4();
        self.metrics.total_generated.fetch_add(1, Ordering::Relaxed);
        Ok(Id::from_uuid_v4(uuid))
    }

    async fn batch_generate(&self, _ctx: &GenerateContext, size: usize) -> Result<IdBatch> {
        let mut ids = Vec::with_capacity(size);

        for _ in 0..size {
            ids.push(self.generate(_ctx).await?);
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

    #[tokio::test]
    async fn test_uuid_v7_generation() {
        let generator = UuidV7Impl::new();
        let ctx = GenerateContext::default();
        let id = generator.generate(&ctx).await.unwrap();

        assert_eq!(id.to_uuid_v7().get_version(), Some(uuid::Version::SortRand));
        assert!(!id.to_uuid_v7().is_nil());
    }

    #[test]
    fn test_uuid_v7_format() {
        let uuid = UuidV7Impl::generate().unwrap();
        let uuid_str = uuid.to_string();

        assert_eq!(uuid_str.len(), 36);
        assert_eq!(uuid_str.chars().nth(14).unwrap(), '7');
    }
}
