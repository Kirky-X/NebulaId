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

use crate::core::algorithm::traits::{
    AlgorithmMetricsSnapshot, GenerateContext, HealthStatus, IdAlgorithm,
};
// ARCH-HIGH-001 修复：AlgorithmFactory impl 需要 Config 参数（即使不用）。
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

pub struct UuidV7Impl {
    metrics: Arc<UuidMetrics>,
}

impl UuidV7Impl {
    pub fn new() -> Self {
        Self {
            metrics: Arc::new(UuidMetrics::new()),
        }
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
            let uuid = Uuid::now_v7();
            ids.push(Id::from_uuid_v7(uuid));
        }

        // 批量更新 metrics，避免循环内逐次 fetch_add
        let count = ids.len() as u64;
        self.metrics
            .total_generated
            .fetch_add(count, Ordering::Relaxed);

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
            // L15 修复：UUID 算法无缓存概念，返回 None。
            cache_hit_rate: None,
        }
    }

    fn algorithm_type(&self) -> AlgorithmType {
        AlgorithmType::UuidV7
    }

    // L13 修复：删除 no-op `initialize`（trait 上已无此方法）。

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
            let uuid = Uuid::new_v4();
            ids.push(Id::from_uuid_v4(uuid));
        }

        // 批量更新 metrics，避免循环内逐次 fetch_add
        let count = ids.len() as u64;
        self.metrics
            .total_generated
            .fetch_add(count, Ordering::Relaxed);

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
            // L15 修复：UUID 算法无缓存概念，返回 None。
            cache_hit_rate: None,
        }
    }

    fn algorithm_type(&self) -> AlgorithmType {
        AlgorithmType::UuidV4
    }

    // L13 修复：删除 no-op `initialize`（trait 上已无此方法）。

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }
}

// ============================================================================
// ARCH-HIGH-001 修复：UuidV7Factory 和 UuidV4Factory impl 拆分到本文件。
// 原 impl 位于 traits.rs（违反规则 25），现移到具体类型所属文件。
// ============================================================================
#[async_trait]
impl crate::core::algorithm::AlgorithmFactory for crate::core::algorithm::UuidV7Factory {
    async fn build(
        &self,
        _builder: &crate::core::algorithm::AlgorithmBuilder,
        _config: &Config,
    ) -> Result<Box<dyn crate::core::algorithm::IdAlgorithm>> {
        Ok(Box::new(UuidV7Impl::new()))
    }
}

#[async_trait]
impl crate::core::algorithm::AlgorithmFactory for crate::core::algorithm::UuidV4Factory {
    async fn build(
        &self,
        _builder: &crate::core::algorithm::AlgorithmBuilder,
        _config: &Config,
    ) -> Result<Box<dyn crate::core::algorithm::IdAlgorithm>> {
        Ok(Box::new(UuidV4Impl::new()))
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
        let uuid = Uuid::now_v7();
        let uuid_str = uuid.to_string();

        assert_eq!(uuid_str.len(), 36);
        assert_eq!(uuid_str.chars().nth(14).unwrap(), '7');
    }
}
