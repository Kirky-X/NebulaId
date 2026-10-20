// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

use crate::core::config::Config;
use crate::core::types::{AlgorithmType, CoreError, Id, IdBatch, Result};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

pub use crate::core::algorithm::DegradationManager;

// 工厂 impl 拆分到各算法文件，本文件只放接口定义。
#[cfg(feature = "segment")]
use crate::core::algorithm::segment::CpuMonitor;
#[cfg(feature = "segment")]
use crate::core::algorithm::segment::SegmentLoader;
#[cfg(feature = "etcd")]
use crate::core::coordinator::EtcdClusterHealthMonitor;

#[async_trait]
pub trait IdAlgorithm: Send + Sync {
    async fn generate(&self, ctx: &GenerateContext) -> Result<Id>;

    async fn batch_generate(&self, ctx: &GenerateContext, size: usize) -> Result<IdBatch>;

    fn health_check(&self) -> HealthStatus;

    fn metrics(&self) -> AlgorithmMetricsSnapshot;

    fn algorithm_type(&self) -> AlgorithmType;

    async fn shutdown(&self) -> Result<()>;
    // initialize 为 inherent method：`&mut self` 在 `dyn IdAlgorithm` 上调用受限。
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

    /// 生成单个 ID 并报告实际服务算法（`None` = 未指定，按路由配置解析）。
    ///
    /// 响应装配（HTTP/gRPC）必须用实际算法：请求算法失败后由降级链兜底时，
    /// 回显请求值会让调用方误以为 ID 来自所请求的算法。
    async fn generate_reporting_algorithm(
        &self,
        algorithm: Option<AlgorithmType>,
        workspace: &str,
        group: &str,
        biz_tag: &str,
    ) -> Result<(Id, AlgorithmType)>;

    /// 批量生成并返回携带实际算法标记的批次（`IdBatch.algorithm` 为真实服务算法）。
    async fn batch_generate_reporting_algorithm(
        &self,
        algorithm: Option<AlgorithmType>,
        workspace: &str,
        group: &str,
        biz_tag: &str,
        size: usize,
    ) -> Result<IdBatch>;

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

    /// 当前生效的 Snowflake 位布局（解析知识唯一来源）。
    ///
    /// 返回 `None` 表示实现不基于 Snowflake 或无法获知布局；调用方应回退
    /// 到 [`crate::core::algorithm::SnowflakeLayoutInfo::standard`]。
    #[cfg(feature = "snowflake")]
    fn snowflake_layout(&self) -> Option<crate::core::algorithm::SnowflakeLayoutInfo> {
        None
    }
}

#[derive(Debug, Clone)]
pub struct GenerateContext {
    pub workspace_id: String,
    pub group_id: String,
    pub biz_tag: String,
    pub format: crate::core::types::IdFormat,
    pub prefix: Option<String>,
}

impl Default for GenerateContext {
    fn default() -> Self {
        Self {
            workspace_id: String::new(),
            group_id: String::new(),
            biz_tag: String::new(),
            format: crate::core::types::IdFormat::Numeric,
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
    /// 最近请求延迟 p50（微秒）；0 表示尚未经路由层观测，非延迟为 0。
    pub p50_latency_us: u64,
    /// p99（微秒），填充语义同 [`Self::p50_latency_us`]。
    pub p99_latency_us: u64,
    /// p99.9（微秒），填充语义同 [`Self::p50_latency_us`]。
    pub p999_latency_us: u64,
    /// 累计观测到的时钟回拨次数，由路由层合并填充，供 `/metrics` 与告警使用。
    pub clock_backwards: u64,
    /// 缓存命中率；`None` 表示该算法无缓存概念（如 Snowflake/UUID）。
    pub cache_hit_rate: Option<f64>,
}

impl AlgorithmMetricsSnapshot {
    pub fn new() -> Self {
        Self::default()
    }
}

pub struct AlgorithmBuilder {
    algorithm_type: AlgorithmType,
    #[cfg(feature = "segment")]
    cpu_monitor: Option<Arc<CpuMonitor>>,
    #[cfg(feature = "etcd")]
    etcd_health_monitor: Option<Arc<EtcdClusterHealthMonitor>>,
    /// Segment 号段装载器注入口（工厂构造参数透传）。
    ///
    /// `Some` 时以该装载器构建 `SegmentAlgorithm`；`None` 时委托原始
    /// `SegmentFactory`，构建行为不变。
    #[cfg(feature = "segment")]
    segment_loader: Option<Arc<dyn SegmentLoader + Send + Sync>>,
}

impl AlgorithmBuilder {
    pub fn new(algorithm_type: AlgorithmType) -> Self {
        Self {
            algorithm_type,
            #[cfg(feature = "segment")]
            cpu_monitor: None,
            #[cfg(feature = "etcd")]
            etcd_health_monitor: None,
            #[cfg(feature = "segment")]
            segment_loader: None,
        }
    }

    /// 注入 Segment 号段装载器（仅对 `AlgorithmType::Segment` 生效）。
    ///
    /// 生产经 router 传入仓储后由工厂透传；测试可注入内存造段器。
    #[cfg(feature = "segment")]
    pub fn with_segment_loader(mut self, loader: Arc<dyn SegmentLoader + Send + Sync>) -> Self {
        self.segment_loader = Some(loader);
        self
    }

    /// 暴露 `segment_loader` 给工厂 impl（pub(crate)）。
    #[cfg(feature = "segment")]
    pub(crate) fn segment_loader(&self) -> &Option<Arc<dyn SegmentLoader + Send + Sync>> {
        &self.segment_loader
    }

    #[cfg(feature = "segment")]
    pub fn with_cpu_monitor(mut self, monitor: Arc<CpuMonitor>) -> Self {
        self.cpu_monitor = Some(monitor);
        self
    }

    #[cfg(feature = "etcd")]
    pub fn with_etcd_health_monitor(mut self, monitor: Arc<EtcdClusterHealthMonitor>) -> Self {
        self.etcd_health_monitor = Some(monitor);
        self
    }

    /// 暴露 `cpu_monitor` 给工厂 impl（pub(crate)）。
    #[cfg(feature = "segment")]
    pub(crate) fn cpu_monitor(&self) -> &Option<Arc<CpuMonitor>> {
        &self.cpu_monitor
    }

    /// 暴露 `etcd_health_monitor` 给工厂 impl。
    #[cfg(feature = "etcd")]
    pub(crate) fn etcd_health_monitor(&self) -> &Option<Arc<EtcdClusterHealthMonitor>> {
        &self.etcd_health_monitor
    }

    pub async fn build(&self, config: &Config) -> Result<Box<dyn IdAlgorithm>> {
        // 经工厂注册表分发：新增算法只需在 algorithm_factories 注册，无需修改 build。
        let factories = algorithm_factories();
        let factory = factories.get(&self.algorithm_type).ok_or_else(|| {
            CoreError::InternalError(format!(
                "No factory registered for algorithm: {:?}",
                self.algorithm_type
            ))
        })?;
        factory.build(self, config).await
    }
}

// AlgorithmFactory + 工厂注册表：build 查表分发，新增算法只需注册新工厂。

/// 算法工厂 trait：每种算法类型对应一个实现。
///
/// 新增算法：实现本 trait 并在 `algorithm_factories` 注册，无需修改 build。
#[async_trait]
pub trait AlgorithmFactory: Send + Sync {
    async fn build(
        &self,
        builder: &AlgorithmBuilder,
        config: &Config,
    ) -> Result<Box<dyn IdAlgorithm>>;
}

/// 内置算法的工厂 struct。`pub` 让外部测试代码可构造并注入。
#[cfg(feature = "snowflake")]
pub struct SnowflakeFactory;
#[cfg(feature = "uuid-v8")]
pub struct UuidV8Factory;
#[cfg(feature = "uuid-v4")]
pub struct UuidV4Factory;
#[cfg(feature = "uuid-v7")]
pub struct UuidV7Factory;
#[cfg(feature = "segment")]
pub struct SegmentFactory;

/// Segment 工厂的 DbSegmentLoader 装配包装。
///
/// 携带 `with_segment_loader` 注入的装载器时按其构建 `SegmentAlgorithm`；
/// 未注入时委托 [`SegmentFactory::build`]，默认行为不变。
#[cfg(feature = "segment")]
struct DbAwareSegmentFactory;

#[cfg(feature = "segment")]
#[async_trait]
impl AlgorithmFactory for DbAwareSegmentFactory {
    async fn build(
        &self,
        builder: &AlgorithmBuilder,
        config: &Config,
    ) -> Result<Box<dyn IdAlgorithm>> {
        match builder.segment_loader() {
            Some(loader) => {
                // 与 segment.rs 的 SegmentFactory 构建逻辑同构，仅多 segment_loader 注入。
                let mut algo = crate::core::algorithm::SegmentAlgorithm::new(config.app.dc_id);
                #[cfg(feature = "etcd")]
                if let Some(ref monitor) = builder.etcd_health_monitor() {
                    algo = algo.with_etcd_cluster_health_monitor(monitor.clone());
                }
                if let Some(ref cpu_monitor) = builder.cpu_monitor() {
                    algo = algo.with_cpu_monitor(cpu_monitor.clone());
                }
                algo = algo.with_segment_loader(loader.clone());
                algo.initialize(config).await?;
                Ok(Box::new(algo))
            }
            None => SegmentFactory.build(builder, config).await,
        }
    }
}

/// 算法工厂注册表（懒加载，进程级单例）；`pub` 供外部测试验证完整性。
pub fn algorithm_factories() -> &'static HashMap<AlgorithmType, Arc<dyn AlgorithmFactory>> {
    static FACTORIES: OnceLock<HashMap<AlgorithmType, Arc<dyn AlgorithmFactory>>> = OnceLock::new();
    FACTORIES.get_or_init(|| {
        // 零算法组合（三特性全关）无任何 insert，mut 在该组合下未被使用。
        #[allow(unused_mut)]
        let mut m: HashMap<AlgorithmType, Arc<dyn AlgorithmFactory>> = HashMap::new();
        #[cfg(feature = "snowflake")]
        m.insert(AlgorithmType::Snowflake, Arc::new(SnowflakeFactory));
        #[cfg(feature = "uuid-v8")]
        m.insert(AlgorithmType::UuidV8, Arc::new(UuidV8Factory));
        #[cfg(feature = "uuid-v4")]
        m.insert(AlgorithmType::UuidV4, Arc::new(UuidV4Factory));
        #[cfg(feature = "uuid-v7")]
        m.insert(AlgorithmType::UuidV7, Arc::new(UuidV7Factory));
        // Segment 经 DbAwareSegmentFactory 代理，支持工厂路径注入
        // DbSegmentLoader（未注入时委托 SegmentFactory，行为不变）。
        #[cfg(feature = "segment")]
        m.insert(AlgorithmType::Segment, Arc::new(DbAwareSegmentFactory));
        m
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_algorithm_metrics_snapshot_new_is_default() {
        let snapshot = AlgorithmMetricsSnapshot::new();
        assert_eq!(snapshot.total_generated, 0);
        assert_eq!(snapshot.total_failed, 0);
        assert!(snapshot.cache_hit_rate.is_none());
    }

    #[cfg(feature = "etcd")]
    #[test]
    fn test_builder_with_etcd_health_monitor() {
        use crate::core::config::EtcdConfig;
        use crate::core::coordinator::EtcdClusterHealthMonitor;

        let monitor = Arc::new(EtcdClusterHealthMonitor::new(
            EtcdConfig::default(),
            "/tmp/nebulaid-test-etcd-cache.json".to_string(),
        ));
        let builder =
            AlgorithmBuilder::new(AlgorithmType::Segment).with_etcd_health_monitor(monitor);
        let _ = builder;
    }
}
