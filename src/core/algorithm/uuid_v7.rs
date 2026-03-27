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

use crate::core::algorithm::traits::{
    AlgorithmMetricsSnapshot, GenerateContext, HealthStatus, IdAlgorithm,
};
use crate::core::config::Config;
use crate::core::types::id::Id;
use crate::core::types::{AlgorithmType, IdBatch, Result};
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

// ============================================================================
// DI Support - Builder Pattern and with_dependencies
// ============================================================================

use confers::traits::ConfigProvider;

impl UuidV7Impl {
    /// Create a new UuidV7Impl with dependencies injected.
    ///
    /// Note: UuidV7Impl doesn't require external dependencies,
    /// but this method is provided for API consistency.
    pub fn with_dependencies(_config: &Arc<dyn ConfigProvider>) -> Self {
        Self::new()
    }

    /// Create a builder for UuidV7Impl.
    pub fn builder() -> UuidV7Builder {
        UuidV7Builder::new()
    }
}

/// Builder for UuidV7Impl.
#[derive(Default)]
pub struct UuidV7Builder {
    config: Option<Arc<dyn ConfigProvider>>,
}

impl UuidV7Builder {
    /// Create a new builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the configuration provider (optional for UuidV7).
    pub fn config(mut self, config: Arc<dyn ConfigProvider>) -> Self {
        self.config = Some(config);
        self
    }

    /// Build the UuidV7Impl.
    pub fn build(self) -> UuidV7Impl {
        UuidV7Impl::new()
    }
}

impl UuidV4Impl {
    /// Create a new UuidV4Impl with dependencies injected.
    ///
    /// Note: UuidV4Impl doesn't require external dependencies,
    /// but this method is provided for API consistency.
    pub fn with_dependencies(_config: &Arc<dyn ConfigProvider>) -> Self {
        Self::new()
    }

    /// Create a builder for UuidV4Impl.
    pub fn builder() -> UuidV4Builder {
        UuidV4Builder::new()
    }
}

/// Builder for UuidV4Impl.
#[derive(Default)]
pub struct UuidV4Builder {
    config: Option<Arc<dyn ConfigProvider>>,
}

impl UuidV4Builder {
    /// Create a new builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the configuration provider (optional for UuidV4).
    pub fn config(mut self, config: Arc<dyn ConfigProvider>) -> Self {
        self.config = Some(config);
        self
    }

    /// Build the UuidV4Impl.
    pub fn build(self) -> UuidV4Impl {
        UuidV4Impl::new()
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
