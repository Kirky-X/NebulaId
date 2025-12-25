use crate::algorithm::degradation_manager::DegradationManager;
use crate::algorithm::segment::SegmentAlgorithm;
use crate::algorithm::snowflake::SnowflakeAlgorithm;
use crate::algorithm::uuid_v7::{UuidV4Impl, UuidV7Impl};
use crate::config::Config;
use crate::coordinator::EtcdClusterHealthMonitor;
use crate::types::{AlgorithmType, Id, IdBatch, Result};
use async_trait::async_trait;
use std::sync::Arc;

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HealthStatus {
    Healthy,
    Degraded(String),
    Unhealthy(String),
}

impl Default for HealthStatus {
    fn default() -> Self {
        HealthStatus::Healthy
    }
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
    etcd_health_monitor: Option<Arc<EtcdClusterHealthMonitor>>,
}

impl AlgorithmBuilder {
    pub fn new(algorithm_type: AlgorithmType) -> Self {
        Self {
            algorithm_type,
            etcd_health_monitor: None,
        }
    }

    pub fn with_etcd_health_monitor(mut self, monitor: Arc<EtcdClusterHealthMonitor>) -> Self {
        self.etcd_health_monitor = Some(monitor);
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
                if let Some(ref monitor) = self.etcd_health_monitor {
                    algo = algo.with_etcd_cluster_health_monitor(monitor.clone());
                }
                algo.initialize(config).await?;
                Ok(Box::new(algo))
            }
        }
    }
}
