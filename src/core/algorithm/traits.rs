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

use crate::core::config::Config;
use crate::core::types::{AlgorithmType, CoreError, Id, IdBatch, Result};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

// Forward declaration - actual import is in parent module
pub use crate::core::algorithm::DegradationManager;

// L14 修复后，CpuMonitor / EtcdClusterHealthMonitor 由 AlgorithmBuilder
// 持有并通过 `pub(crate)` 访问器暴露给工厂 impl。工厂 impl 拆分到
// 各算法文件（snowflake.rs / uuid_v7.rs / segment.rs），按规则 25
// 「mod.rs/traits.rs 只放接口定义」要求。
use crate::core::algorithm::segment::CpuMonitor;
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
    // L13 修复：从 trait 中移除 `async fn initialize(&mut self, config: &Config)`。
    // 原因：
    // 1. `&mut self` 在 `dyn IdAlgorithm` trait object 上调用受限——
    //    `Arc<dyn IdAlgorithm>` 共享后无法调用 `&mut self` 方法，必须
    //    在 `Arc::from(algo)` 之前调用。这是设计气味，让 trait "不那么
    //    对象安全"。
    // 2. `AlgorithmBuilder::build` 内部已经调用各算法的 inherent
    //    `initialize(&mut self, ...)` 方法完成初始化，返回的
    //    `Box<dyn IdAlgorithm>` 已经初始化好。router.rs 不应再次调用。
    // 3. 各算法实现的 `initialize` 改为 inherent method（impl SnowflakeAlgorithm
    //    / SegmentAlgorithm / ...），仅在 `build` 时通过具体类型调用。
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
    /// 暂未实现 latency histogram；所有算法当前返回 0。
    /// 0 表示"未实现"，不是"延迟为 0 微秒"。
    pub p50_latency_us: u64,
    /// 暂未实现 latency histogram；所有算法当前返回 0。
    /// 0 表示"未实现"，不是"延迟为 0 微秒"。
    pub p99_latency_us: u64,
    /// L15 修复：`None` 表示该算法无缓存概念（如 Snowflake/UUID），
    /// `Some(rate)` 表示真实缓存命中率（如 Segment）。
    /// 原为 `f64`，UUID/Snowflake 返回 `0.0` 会被
    /// `ConfigManager::get_cache_metrics` 纳入平均值计算，导致整体
    /// 缓存命中率被低估（误把"无缓存"当成"命中率 0%"）。
    pub cache_hit_rate: Option<f64>,
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
    // L12 修复：非 etcd 版本不再持有 `etcd_health_monitor: Option<()>` 占位字段。
    // 原 `Option<()>` 既无意义也误导调用方。`with_etcd_health_monitor` builder
    // 方法仅在 etcd feature 下存在；非 etcd 版本调用方不会触碰该字段。
}

impl AlgorithmBuilder {
    pub fn new(algorithm_type: AlgorithmType) -> Self {
        Self {
            algorithm_type,
            cpu_monitor: None,
            #[cfg(feature = "etcd")]
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
    // L12 修复：删除非 etcd 版本的 `with_etcd_health_monitor(Arc<()>)`。
    // 原签名接受 `Arc<()>` 但完全忽略参数，类型误导且调用方可能误以为
    // monitor 被实际使用。非 etcd 版本根本不需要这个 builder 方法。

    /// ARCH-HIGH-001 修复：暴露 `cpu_monitor` 给工厂 impl（pub(crate)）。
    /// 工厂 impl 拆分到各算法文件后，无法直接访问 AlgorithmBuilder
    /// 私有字段，必须通过访问器。
    pub(crate) fn cpu_monitor(&self) -> &Option<Arc<CpuMonitor>> {
        &self.cpu_monitor
    }

    /// ARCH-HIGH-001 修复：暴露 `etcd_health_monitor` 给工厂 impl。
    #[cfg(feature = "etcd")]
    pub(crate) fn etcd_health_monitor(&self) -> &Option<Arc<EtcdClusterHealthMonitor>> {
        &self.etcd_health_monitor
    }

    pub async fn build(&self, config: &Config) -> Result<Box<dyn IdAlgorithm>> {
        // L14 修复：通过工厂注册表分发，不再按算法类型 match。
        // 新增算法只需实现 `AlgorithmFactory` 并在 `algorithm_factories()`
        // 中注册，无需修改 `build` 方法（开闭原则）。
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

// ============================================================================
// L14 修复：AlgorithmFactory + 注册表
// ============================================================================
//
// 原实现：`AlgorithmBuilder::build` 用 `match self.algorithm_type { ... }`
// 按算法类型分支构建。新增算法必须修改 `build` 方法，违反开闭原则。
//
// 现实现：每种算法对应一个 `AlgorithmFactory` 实现，注册在静态
// `algorithm_factories()` 表中。`build` 方法查表分发，新增算法只需
// 添加新工厂 + 注册，不修改 `build`。
//
// ARCH-HIGH-001 修复：工厂 impl 已拆到各算法文件（snowflake.rs /
// uuid_v7.rs / uuid_v4.rs / segment.rs）。本文件只保留 trait 定义、
// 工厂 struct 声明、注册表函数，符合规则 25「traits.rs 只放接口」。
//
// ARCH-MED-001 修复：trait 和工厂 struct 改 `pub`，外部 crate / 测试
// 可注入 mock 工厂替换真实算法，实现完全的开闭原则。

/// 算法工厂 trait：每种算法类型对应一个实现。
///
/// 新增算法时：
/// 1. 在算法文件中实现 `AlgorithmFactory`
/// 2. 在 `algorithm_factories()` 注册表中插入 `(AlgorithmType::NewAlgo, Arc::new(NewAlgoFactory))`
///
/// 无需修改 `AlgorithmBuilder::build` 方法。
#[async_trait]
pub trait AlgorithmFactory: Send + Sync {
    async fn build(
        &self,
        builder: &AlgorithmBuilder,
        config: &Config,
    ) -> Result<Box<dyn IdAlgorithm>>;
}

/// 4 个内置算法的工厂 struct。`pub` 让外部测试代码可构造并注入。
pub struct SnowflakeFactory;
pub struct UuidV7Factory;
pub struct UuidV4Factory;
pub struct SegmentFactory;

/// 算法工厂注册表（懒加载，进程级单例）。
///
/// ARCH-MED-001 修复：函数 `pub`，外部测试可读取注册表验证完整性。
pub fn algorithm_factories() -> &'static HashMap<AlgorithmType, Arc<dyn AlgorithmFactory>> {
    static FACTORIES: OnceLock<HashMap<AlgorithmType, Arc<dyn AlgorithmFactory>>> = OnceLock::new();
    FACTORIES.get_or_init(|| {
        let mut m: HashMap<AlgorithmType, Arc<dyn AlgorithmFactory>> = HashMap::new();
        m.insert(AlgorithmType::Snowflake, Arc::new(SnowflakeFactory));
        m.insert(AlgorithmType::UuidV7, Arc::new(UuidV7Factory));
        m.insert(AlgorithmType::UuidV4, Arc::new(UuidV4Factory));
        m.insert(AlgorithmType::Segment, Arc::new(SegmentFactory));
        m
    })
}
