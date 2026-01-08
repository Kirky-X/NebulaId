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

use crate::algorithm::segment::{CpuMonitor, SegmentAlgorithm};
use crate::algorithm::snowflake::SnowflakeAlgorithm;
use crate::algorithm::uuid_v7::{UuidV4Impl, UuidV7Impl};
use crate::config::Config;
#[cfg(feature = "etcd")]
use crate::coordinator::EtcdClusterHealthMonitor;
use crate::types::{AlgorithmType, Id, IdBatch, Result};
use async_trait::async_trait;
use std::sync::Arc;

// Forward declaration - actual import is in parent module
pub use crate::algorithm::DegradationManager;

#[async_trait]
pub trait IdAlgorithm: Send + Sync {
    async fn generate(&self, ctx: &GenerateContext) -> Result<Id>;

    async fn batch_generate(&self, ctx: &GenerateContext, size: usize) -> Result<IdBatch>;

    fn health_check(&self) -> HealthStatus;

    fn metrics(&self) -> AlgorithmMetricsSnapshot;

    fn algorithm_type(&self) -> AlgorithmType;

    async fn initialize(&mut self, config: &Config) -> Result<()>;

    async fn shutdown(&self) -> Result<()>;
}

#[async_trait]
pub trait IdGenerator: Send + Sync {
    async fn generate(&self, workspace: &str, group: &str, biz_tag: &str) -> Result<Id>;

    async fn batch_generate(
        &self,
        workspace: &str,
        group: &str,
        biz_tag: &str,
        size: usize,
    ) -> Result<Vec<Id>>;

    async fn generate_with_algorithm(
        &self,
        algorithm: AlgorithmType,
        workspace: &str,
        group: &str,
        biz_tag: &str,
    ) -> Result<Id>;

    async fn batch_generate_with_algorithm(
        &self,
        algorithm: AlgorithmType,
        workspace: &str,
        group: &str,
        biz_tag: &str,
        size: usize,
    ) -> Result<Vec<Id>>;

    async fn get_algorithm_name(
        &self,
        workspace: &str,
        group: &str,
        biz_tag: &str,
    ) -> Result<String>;

    async fn health_check(&self) -> HealthStatus;

    async fn get_primary_algorithm(&self) -> String;

    fn get_degradation_manager(&self) -> &Arc<DegradationManager>;
}

#[derive(Debug, Clone)]
pub struct GenerateContext {
    pub workspace_id: String,
    pub group_id: String,
    pub biz_tag: String,
    pub format: crate::types::IdFormat,
    pub prefix: Option<String>,
}

impl Default for GenerateContext {
    fn default() -> Self {
        Self {
            workspace_id: String::new(),
            group_id: String::new(),
            biz_tag: String::new(),
            format: crate::types::IdFormat::Numeric,
            prefix: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum HealthStatus {
    #[default]
    Healthy,
    Degraded(String),
    Unhealthy(String),
}

impl HealthStatus {
    pub fn is_healthy(&self) -> bool {
        matches!(self, HealthStatus::Healthy)
    }
}

#[derive(Debug, Clone, Default)]
pub struct AlgorithmMetricsSnapshot {
    pub total_generated: u64,
    pub total_failed: u64,
    pub current_qps: u64,
    pub p50_latency_us: u64,
    pub p99_latency_us: u64,
    pub cache_hit_rate: f64,
}

impl AlgorithmMetricsSnapshot {
    pub fn new() -> Self {
        Self::default()
    }
}

pub struct AlgorithmBuilder {
    algorithm_type: AlgorithmType,
    cpu_monitor: Option<Arc<CpuMonitor>>,
    #[cfg(feature = "etcd")]
    etcd_health_monitor: Option<Arc<EtcdClusterHealthMonitor>>,
    #[cfg(not(feature = "etcd"))]
    etcd_health_monitor: Option<()>,
}

impl AlgorithmBuilder {
    pub fn new(algorithm_type: AlgorithmType) -> Self {
        Self {
            algorithm_type,
            cpu_monitor: None,
            etcd_health_monitor: None,
        }
    }

    pub fn with_cpu_monitor(mut self, monitor: Arc<CpuMonitor>) -> Self {
        self.cpu_monitor = Some(monitor);
        self
    }

    #[cfg(feature = "etcd")]
    pub fn with_etcd_health_monitor(mut self, monitor: Arc<EtcdClusterHealthMonitor>) -> Self {
        self.etcd_health_monitor = Some(monitor);
        self
    }

    #[cfg(not(feature = "etcd"))]
    pub fn with_etcd_health_monitor(mut self, _monitor: Arc<()>) -> Self {
        self.etcd_health_monitor = Some(());
        self
    }

    pub async fn build(&self, config: &Config) -> Result<Box<dyn IdAlgorithm>> {
        match self.algorithm_type {
            AlgorithmType::Snowflake => {
                let mut algo = SnowflakeAlgorithm::new(config.app.dc_id, config.app.worker_id);
                algo.initialize(config).await?;
                Ok(Box::new(algo))
            }
            AlgorithmType::UuidV7 => Ok(Box::new(UuidV7Impl::new())),
            AlgorithmType::UuidV4 => Ok(Box::new(UuidV4Impl::new())),
            AlgorithmType::Segment => {
                let mut algo = SegmentAlgorithm::new(config.app.dc_id);
                #[cfg(feature = "etcd")]
                if let Some(ref monitor) = self.etcd_health_monitor {
                    algo = algo.with_etcd_cluster_health_monitor(monitor.clone());
                }
                if let Some(ref cpu_monitor) = self.cpu_monitor {
                    algo = algo.with_cpu_monitor(cpu_monitor.clone());
                }
                algo.initialize(config).await?;
                Ok(Box::new(algo))
            }
        }
    }
}
