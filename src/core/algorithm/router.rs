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

use crate::core::algorithm::{
    AlgorithmBuilder, AlgorithmMetricsSnapshot, DegradationManager, DynAuditLogger,
    GenerateContext, HealthStatus, IdAlgorithm, IdGenerator,
};
use crate::core::config::Config;
#[cfg(feature = "etcd")]
use crate::core::coordinator::EtcdClusterHealthMonitor;
use crate::core::types::{AlgorithmType, CoreError, Id, IdBatch, Result};
use arc_swap::ArcSwap;
use async_trait::async_trait;
use smallvec::SmallVec;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, error, info, warn};

#[async_trait]
impl IdGenerator for AlgorithmRouter {
    async fn generate(&self, workspace: &str, group: &str, biz_tag: &str) -> Result<Id> {
        let ctx = GenerateContext {
            workspace_id: workspace.to_string(),
            group_id: group.to_string(),
            biz_tag: biz_tag.to_string(),
            format: crate::core::types::IdFormat::Numeric,
            prefix: None,
        };
        AlgorithmRouter::generate(self, &ctx).await
    }

    async fn batch_generate(
        &self,
        workspace: &str,
        group: &str,
        biz_tag: &str,
        size: usize,
    ) -> Result<Vec<Id>> {
        let ctx = GenerateContext {
            workspace_id: workspace.to_string(),
            group_id: group.to_string(),
            biz_tag: biz_tag.to_string(),
            format: crate::core::types::IdFormat::Numeric,
            prefix: None,
        };
        let batch = AlgorithmRouter::batch_generate(self, &ctx, size).await?;
        Ok(batch.ids)
    }

    async fn get_algorithm_name(
        &self,
        _workspace: &str,
        _group: &str,
        biz_tag: &str,
    ) -> Result<String> {
        if let Some(alg) = self.current_algorithm.load().get(biz_tag) {
            Ok(format!("{}", *alg))
        } else {
            Ok(format!("{}", self.config.algorithm.get_default_algorithm()))
        }
    }

    async fn health_check(&self) -> HealthStatus {
        let statuses = AlgorithmRouter::health_check(self).await;
        if statuses.is_empty() {
            return HealthStatus::Unhealthy("No algorithms available".to_string());
        }
        if statuses
            .iter()
            .any(|(_, s)| matches!(s, HealthStatus::Unhealthy(_)))
        {
            HealthStatus::Unhealthy("Some algorithms are unhealthy".to_string())
        } else if statuses
            .iter()
            .any(|(_, s)| matches!(s, HealthStatus::Degraded(_)))
        {
            HealthStatus::Degraded("Some algorithms are degraded".to_string())
        } else {
            HealthStatus::Healthy
        }
    }

    async fn get_primary_algorithm(&self) -> String {
        format!("{:?}", self.config.algorithm.get_default_algorithm())
    }

    fn get_degradation_manager(&self) -> &Arc<DegradationManager> {
        &self.degradation_manager
    }

    async fn generate_with_algorithm(
        &self,
        algorithm: AlgorithmType,
        workspace: &str,
        group: &str,
        biz_tag: &str,
    ) -> Result<Id> {
        AlgorithmRouter::generate_with_algorithm(self, algorithm, workspace, group, biz_tag).await
    }

    async fn batch_generate_with_algorithm(
        &self,
        algorithm: AlgorithmType,
        workspace: &str,
        group: &str,
        biz_tag: &str,
        size: usize,
    ) -> Result<Vec<Id>> {
        let batch = AlgorithmRouter::batch_generate_with_algorithm(
            self, algorithm, workspace, group, biz_tag, size,
        )
        .await?;
        Ok(batch.ids)
    }
}

pub struct AlgorithmRouter {
    config: Config,
    algorithms: Arc<ArcSwap<HashMap<AlgorithmType, Arc<dyn IdAlgorithm>>>>,
    fallback_chain: SmallVec<[AlgorithmType; 8]>,
    current_algorithm: Arc<ArcSwap<HashMap<String, AlgorithmType>>>,
    degradation_manager: Arc<DegradationManager>,
    cpu_monitor: Option<Arc<crate::core::algorithm::segment::CpuMonitor>>,
    #[cfg(feature = "etcd")]
    etcd_health_monitor: Option<Arc<EtcdClusterHealthMonitor>>,
    // L12 修复：非 etcd 版本不再持有 `etcd_health_monitor: Option<()>`
    // 占位字段（类型误导）。`with_etcd_health_monitor` builder 方法也仅在
    // etcd feature 下存在；非 etcd 版本调用方（main.rs）根本不会调用它。
}

// L11 修复：删除手动 `unsafe impl Send/Sync`。所有字段（Config /
// Arc<ArcSwap<T>> / SmallVec / Arc<DegradationManager> /
// Option<Arc<CpuMonitor>> / Option<Arc<EtcdClusterHealthMonitor>>）
// 均为 Send + Sync，编译器会自动推导。原 `unsafe impl` 是历史遗留，
// 掩盖了潜在的非线程安全字段，应删除让编译器做严格检查。

impl AlgorithmRouter {
    pub fn new(config: Config, audit_logger: Option<DynAuditLogger>) -> Self {
        let mut fallback_chain: SmallVec<[AlgorithmType; 8]> = SmallVec::new();

        match config.algorithm.get_default_algorithm() {
            AlgorithmType::Segment => {
                fallback_chain.push(AlgorithmType::Snowflake);
                fallback_chain.push(AlgorithmType::UuidV7);
                fallback_chain.push(AlgorithmType::UuidV4);
            }
            AlgorithmType::Snowflake => {
                fallback_chain.push(AlgorithmType::UuidV7);
                fallback_chain.push(AlgorithmType::UuidV4);
            }
            _ => {}
        }

        let primary_algorithm = config.algorithm.get_default_algorithm();
        let degradation_manager = Arc::new(DegradationManager::new(None, audit_logger));

        degradation_manager.set_primary_algorithm(primary_algorithm);
        degradation_manager.set_fallback_chain(fallback_chain.to_vec());

        Self {
            config,
            algorithms: Arc::new(ArcSwap::from_pointee(HashMap::new())),
            fallback_chain,
            current_algorithm: Arc::new(ArcSwap::from_pointee(HashMap::new())),
            degradation_manager,
            cpu_monitor: None,
            #[cfg(feature = "etcd")]
            etcd_health_monitor: None,
        }
    }

    pub fn with_cpu_monitor(
        mut self,
        monitor: Arc<crate::core::algorithm::segment::CpuMonitor>,
    ) -> Self {
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

    pub async fn initialize(&self) -> Result<()> {
        let mut errors = Vec::new();

        for alg_type in [
            AlgorithmType::Segment,
            AlgorithmType::Snowflake,
            AlgorithmType::UuidV7,
            AlgorithmType::UuidV4,
        ] {
            #[allow(unused_mut)]
            let mut builder = AlgorithmBuilder::new(alg_type);
            #[cfg(feature = "etcd")]
            if let Some(ref monitor) = self.etcd_health_monitor {
                builder = builder.with_etcd_health_monitor(monitor.clone());
            }
            if let Some(ref cpu_monitor) = self.cpu_monitor {
                builder = builder.with_cpu_monitor(cpu_monitor.clone());
            }

            match builder.build(&self.config).await {
                Ok(algo) => {
                    // L13 修复：删除 `algo.initialize(&self.config).await` 重复调用。
                    // `AlgorithmBuilder::build` 内部已经调用各算法的 inherent
                    // `initialize(&mut self, ...)` 完成初始化，返回的
                    // `Box<dyn IdAlgorithm>` 已就绪。原代码重复初始化且在
                    // trait object 上调用 `&mut self` 方法（设计气味）。
                    let alg_arc: Arc<dyn IdAlgorithm> = Arc::from(algo);
                    self.algorithms.rcu(|old| {
                        let mut new: HashMap<_, _> = (**old).clone();
                        new.insert(alg_type, alg_arc.clone());
                        Arc::new(new)
                    });
                    self.degradation_manager
                        .register_algorithm(alg_type, alg_arc);
                    info!(
                        alg_type = ?alg_type,
                        "{}",
                        t!("log.core.algorithm.router.algorithm_initialized")
                    );
                }
                Err(_e) => {
                    warn!(
                        alg_type = ?alg_type,
                        "{}",
                        t!(
                            "log.core.algorithm.router.algorithm_build_failed",
                            error = _e
                        )
                    );
                    errors.push((alg_type, _e));
                }
            }
        }

        if self.algorithms.load().is_empty() {
            return Err(CoreError::InternalError(
                "No algorithms available".to_string(),
            ));
        }

        Ok(())
    }

    pub async fn generate(&self, ctx: &GenerateContext) -> Result<Id> {
        let algorithm = self.get_algorithm(ctx).await;
        self.generate_with_algorithm_internal(algorithm, ctx).await
    }

    pub async fn batch_generate(&self, ctx: &GenerateContext, size: usize) -> Result<IdBatch> {
        let algorithm = self.get_algorithm(ctx).await;
        self.batch_generate_with_algorithm_internal(algorithm, ctx, size)
            .await
    }

    async fn get_algorithm(&self, ctx: &GenerateContext) -> AlgorithmType {
        if let Some(alg) = self.current_algorithm.load().get(&ctx.biz_tag) {
            return *alg;
        }
        self.config.algorithm.get_default_algorithm()
    }

    pub async fn set_algorithm(&self, biz_tag: String, algorithm: AlgorithmType) {
        self.current_algorithm.rcu(|old| {
            let mut new: HashMap<_, _> = (**old).clone();
            new.insert(biz_tag.clone(), algorithm);
            Arc::new(new)
        });
    }

    pub async fn generate_with_algorithm(
        &self,
        algorithm: AlgorithmType,
        workspace: &str,
        group: &str,
        biz_tag: &str,
    ) -> Result<Id> {
        debug!(
            algorithm = ?algorithm,
            "{}",
            t!(
                "log.core.algorithm.router.generate_with_algorithm_called",
                workspace = workspace,
                group = group,
                biz_tag = biz_tag
            )
        );
        let ctx = GenerateContext {
            workspace_id: workspace.to_string(),
            group_id: group.to_string(),
            biz_tag: biz_tag.to_string(),
            format: crate::core::types::IdFormat::Numeric,
            prefix: None,
        };
        self.generate_with_algorithm_internal(algorithm, &ctx).await
    }

    pub async fn batch_generate_with_algorithm(
        &self,
        algorithm: AlgorithmType,
        workspace: &str,
        group: &str,
        biz_tag: &str,
        size: usize,
    ) -> Result<IdBatch> {
        let ctx = GenerateContext {
            workspace_id: workspace.to_string(),
            group_id: group.to_string(),
            biz_tag: biz_tag.to_string(),
            format: crate::core::types::IdFormat::Numeric,
            prefix: None,
        };
        self.batch_generate_with_algorithm_internal(algorithm, &ctx, size)
            .await
    }

    async fn generate_with_algorithm_internal(
        &self,
        algorithm: AlgorithmType,
        ctx: &GenerateContext,
    ) -> Result<Id> {
        let effective_algorithm = algorithm;

        debug!(
            algorithm = ?effective_algorithm,
            "{}",
            t!(
                "log.core.algorithm.router.generate_internal_called",
                biz_tag = ctx.biz_tag
            )
        );

        // 一次性加载算法表（无锁），后续查找复用，避免循环中多次加锁
        let algorithms = self.algorithms.load_full();
        let alg_opt = algorithms.get(&effective_algorithm).cloned();

        if let Some(alg) = alg_opt {
            debug!(
                algorithm = ?effective_algorithm,
                "{}",
                t!("log.core.algorithm.router.algorithm_found")
            );
            match alg.generate(ctx).await {
                Ok(id) => {
                    self.degradation_manager
                        .record_generation_result(effective_algorithm, true)
                        .await;
                    debug!(
                        algorithm = ?effective_algorithm,
                        "{}",
                        t!("log.core.algorithm.router.id_generated")
                    );
                    return Ok(id);
                }
                Err(e) => {
                    debug!(
                        error = ?e,
                        "{}",
                        t!(
                            "log.core.algorithm.router.algorithm_failed",
                            algorithm = effective_algorithm
                        )
                    );
                    self.degradation_manager
                        .record_generation_result(effective_algorithm, false)
                        .await;
                    warn!(
                        algorithm = ?effective_algorithm,
                        fallback_chain = ?self.fallback_chain,
                        "{}",
                        t!("log.core.algorithm.router.algorithm_failed_fallback")
                    );
                    for fallback in &self.fallback_chain {
                        if let Some(fallback_alg) = algorithms.get(fallback).cloned() {
                            match fallback_alg.generate(ctx).await {
                                Ok(id) => {
                                    self.degradation_manager
                                        .record_generation_result(*fallback, true)
                                        .await;
                                    info!(
                                        fallback = ?fallback,
                                        "{}",
                                        t!("log.core.algorithm.router.fell_back_to_algorithm")
                                    );
                                    return Ok(id);
                                }
                                Err(_) => {
                                    self.degradation_manager
                                        .record_generation_result(*fallback, false)
                                        .await;
                                    continue;
                                }
                            }
                        }
                    }
                    return Err(e);
                }
            }
        }

        warn!(
            algorithm = ?effective_algorithm,
            fallback_chain = ?self.fallback_chain,
            "{}",
            t!("log.core.algorithm.router.algorithm_not_found")
        );

        for fallback in &self.fallback_chain {
            if let Some(fallback_alg) = algorithms.get(fallback).cloned() {
                match fallback_alg.generate(ctx).await {
                    Ok(id) => {
                        self.degradation_manager
                            .record_generation_result(*fallback, true)
                            .await;
                        return Ok(id);
                    }
                    Err(_) => {
                        self.degradation_manager
                            .record_generation_result(*fallback, false)
                            .await;
                        continue;
                    }
                }
            }
        }

        Err(CoreError::InternalError(
            "All algorithms failed".to_string(),
        ))
    }

    async fn batch_generate_with_algorithm_internal(
        &self,
        algorithm: AlgorithmType,
        ctx: &GenerateContext,
        size: usize,
    ) -> Result<IdBatch> {
        let effective_algorithm = algorithm;

        // 一次性加载算法表（无锁），后续查找复用
        let algorithms = self.algorithms.load_full();
        let alg_opt = algorithms.get(&effective_algorithm).cloned();

        if let Some(alg) = alg_opt {
            match alg.batch_generate(ctx, size).await {
                Ok(batch) => {
                    self.degradation_manager
                        .record_generation_result(effective_algorithm, true)
                        .await;
                    return Ok(batch);
                }
                Err(e) => {
                    debug!(
                        error = ?e,
                        "{}",
                        t!(
                            "log.core.algorithm.router.algorithm_batch_failed",
                            algorithm = effective_algorithm
                        )
                    );
                    self.degradation_manager
                        .record_generation_result(effective_algorithm, false)
                        .await;
                    for fallback in &self.fallback_chain {
                        if let Some(fallback_alg) = algorithms.get(fallback).cloned() {
                            match fallback_alg.batch_generate(ctx, size).await {
                                Ok(batch) => {
                                    self.degradation_manager
                                        .record_generation_result(*fallback, true)
                                        .await;
                                    return Ok(batch);
                                }
                                Err(_) => {
                                    self.degradation_manager
                                        .record_generation_result(*fallback, false)
                                        .await;
                                    continue;
                                }
                            }
                        }
                    }
                    return Err(e);
                }
            }
        }

        for fallback in &self.fallback_chain {
            if let Some(fallback_alg) = algorithms.get(fallback).cloned() {
                match fallback_alg.batch_generate(ctx, size).await {
                    Ok(batch) => {
                        self.degradation_manager
                            .record_generation_result(*fallback, true)
                            .await;
                        return Ok(batch);
                    }
                    Err(_e) => {
                        self.degradation_manager
                            .record_generation_result(*fallback, false)
                            .await;
                        continue;
                    }
                }
            }
        }

        Err(CoreError::InternalError(
            "All algorithms failed".to_string(),
        ))
    }

    pub async fn health_check(&self) -> Vec<(AlgorithmType, HealthStatus)> {
        let algs = self.algorithms.load_full();
        algs.iter().map(|(k, v)| (*k, v.health_check())).collect()
    }

    pub async fn metrics(&self) -> Vec<(AlgorithmType, AlgorithmMetricsSnapshot)> {
        let algs = self.algorithms.load_full();
        algs.iter().map(|(k, v)| (*k, v.metrics())).collect()
    }

    pub fn get_degradation_manager(&self) -> &Arc<DegradationManager> {
        &self.degradation_manager
    }

    pub async fn check_health_and_update_degradation(&self) {
        self.degradation_manager.check_all_health().await;
    }

    pub async fn shutdown(&self) {
        let algs = self.algorithms.load_full();
        for alg in algs.values() {
            if let Err(e) = alg.shutdown().await {
                error!(
                    algorithm = ?alg.algorithm_type(),
                    "{}",
                    t!(
                        "log.core.algorithm.router.shutdown_error",
                        error = e
                    )
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::Config;
    use async_trait::async_trait;

    #[tokio::test]
    async fn test_algorithm_router_initialize() {
        let config = Config::default();
        let router = AlgorithmRouter::new(config, None);

        let result = router.initialize().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_algorithm_router_generate() {
        let config = Config::default();
        let router = AlgorithmRouter::new(config, None);
        router.initialize().await.unwrap();

        let ctx = GenerateContext {
            workspace_id: "test".to_string(),
            group_id: "test".to_string(),
            biz_tag: "test".to_string(),
            format: crate::core::types::IdFormat::Numeric,
            prefix: None,
        };

        let id = router.generate(&ctx).await.unwrap();
        assert!(id.as_u128() > 0);
    }

    #[tokio::test]
    async fn test_algorithm_router_batch_generate() {
        let config = Config::default();
        let router = AlgorithmRouter::new(config, None);
        router
            .set_algorithm("test".to_string(), AlgorithmType::Snowflake)
            .await;
        router.initialize().await.unwrap();

        let ctx = GenerateContext {
            workspace_id: "test".to_string(),
            group_id: "test".to_string(),
            biz_tag: "test".to_string(),
            format: crate::core::types::IdFormat::Numeric,
            prefix: None,
        };

        let mut ids_generated = 0;
        for chunk in (0..100).collect::<Vec<_>>().chunks(10) {
            let batch = router.batch_generate(&ctx, chunk.len()).await.unwrap();
            ids_generated += batch.len();
            tokio::time::sleep(tokio::time::Duration::from_millis(1)).await;
        }
        assert_eq!(ids_generated, 100);
    }

    #[tokio::test]
    async fn test_set_algorithm() {
        let config = Config::default();
        let router = AlgorithmRouter::new(config, None);

        router
            .set_algorithm("order".to_string(), AlgorithmType::Snowflake)
            .await;

        let entry = router.current_algorithm.load().get("order").copied();
        assert!(entry.is_some());
        assert_eq!(entry.unwrap(), AlgorithmType::Snowflake);
    }

    // ============== 测试辅助 Mock 与工具函数 ==============

    /// 健康 Mock：所有方法均成功
    struct MockHealthyAlgorithm {
        alg_type: AlgorithmType,
    }

    #[async_trait]
    impl IdAlgorithm for MockHealthyAlgorithm {
        async fn generate(&self, _ctx: &GenerateContext) -> Result<Id> {
            Ok(Id::from_u128(42))
        }
        async fn batch_generate(&self, _ctx: &GenerateContext, size: usize) -> Result<IdBatch> {
            Ok(IdBatch {
                ids: vec![Id::from_u128(42); size],
                algorithm: self.alg_type,
                biz_tag: String::new(),
                generated_at: chrono::Utc::now(),
            })
        }
        fn health_check(&self) -> HealthStatus {
            HealthStatus::Healthy
        }
        fn metrics(&self) -> AlgorithmMetricsSnapshot {
            AlgorithmMetricsSnapshot::default()
        }
        fn algorithm_type(&self) -> AlgorithmType {
            self.alg_type
        }
        async fn shutdown(&self) -> Result<()> {
            Ok(())
        }
    }

    /// 可配置 Mock：可单独设置 generate/batch/shutdown 失败，以及健康状态
    struct MockConfigurableAlgorithm {
        alg_type: AlgorithmType,
        fail_generate: bool,
        fail_batch: bool,
        fail_shutdown: bool,
        health_kind: MockHealthKind,
    }

    #[derive(Clone, Copy)]
    enum MockHealthKind {
        Healthy,
        Degraded,
        Unhealthy,
    }

    impl MockConfigurableAlgorithm {
        fn new(alg_type: AlgorithmType) -> Self {
            Self {
                alg_type,
                fail_generate: false,
                fail_batch: false,
                fail_shutdown: false,
                health_kind: MockHealthKind::Healthy,
            }
        }
        fn with_generate_failure(mut self) -> Self {
            self.fail_generate = true;
            self
        }
        fn with_batch_failure(mut self) -> Self {
            self.fail_batch = true;
            self
        }
        fn with_shutdown_failure(mut self) -> Self {
            self.fail_shutdown = true;
            self
        }
        fn with_health(mut self, kind: MockHealthKind) -> Self {
            self.health_kind = kind;
            self
        }
    }

    #[async_trait]
    impl IdAlgorithm for MockConfigurableAlgorithm {
        async fn generate(&self, _ctx: &GenerateContext) -> Result<Id> {
            if self.fail_generate {
                Err(CoreError::InternalError(
                    "mock generate failure".to_string(),
                ))
            } else {
                Ok(Id::from_u128(42))
            }
        }
        async fn batch_generate(&self, _ctx: &GenerateContext, size: usize) -> Result<IdBatch> {
            if self.fail_batch {
                Err(CoreError::InternalError("mock batch failure".to_string()))
            } else {
                Ok(IdBatch {
                    ids: vec![Id::from_u128(42); size],
                    algorithm: self.alg_type,
                    biz_tag: String::new(),
                    generated_at: chrono::Utc::now(),
                })
            }
        }
        fn health_check(&self) -> HealthStatus {
            match self.health_kind {
                MockHealthKind::Healthy => HealthStatus::Healthy,
                MockHealthKind::Degraded => HealthStatus::Degraded("mock degraded".to_string()),
                MockHealthKind::Unhealthy => HealthStatus::Unhealthy("mock unhealthy".to_string()),
            }
        }
        fn metrics(&self) -> AlgorithmMetricsSnapshot {
            AlgorithmMetricsSnapshot::default()
        }
        fn algorithm_type(&self) -> AlgorithmType {
            self.alg_type
        }
        async fn shutdown(&self) -> Result<()> {
            if self.fail_shutdown {
                Err(CoreError::InternalError(
                    "mock shutdown failure".to_string(),
                ))
            } else {
                Ok(())
            }
        }
    }

    /// 向 router 中插入 mock 算法
    fn insert_mock(router: &AlgorithmRouter, alg_type: AlgorithmType, mock: Arc<dyn IdAlgorithm>) {
        router.algorithms.rcu(|old| {
            let mut new: HashMap<_, _> = (**old).clone();
            new.insert(alg_type, mock.clone());
            Arc::new(new)
        });
    }

    /// 构造 GenerateContext
    fn make_ctx(biz_tag: &str) -> GenerateContext {
        GenerateContext {
            workspace_id: "ws".to_string(),
            group_id: "g".to_string(),
            biz_tag: biz_tag.to_string(),
            format: crate::core::types::IdFormat::Numeric,
            prefix: None,
        }
    }

    // ============== IdGenerator trait impl 测试 ==============

    #[tokio::test]
    async fn test_id_generator_generate_trait_method_succeeds() {
        let router = AlgorithmRouter::new(Config::default(), None);
        insert_mock(
            &router,
            AlgorithmType::Segment,
            Arc::new(MockHealthyAlgorithm {
                alg_type: AlgorithmType::Segment,
            }),
        );
        let id = IdGenerator::generate(&router, "ws", "g", "bt")
            .await
            .unwrap();
        assert_eq!(id.as_u128(), 42);
    }

    #[tokio::test]
    async fn test_id_generator_generate_trait_method_fallback_when_primary_missing() {
        // 默认算法是 Segment，但 algorithms 中只有 Snowflake — 应回退到 Snowflake
        let router = AlgorithmRouter::new(Config::default(), None);
        insert_mock(
            &router,
            AlgorithmType::Snowflake,
            Arc::new(MockHealthyAlgorithm {
                alg_type: AlgorithmType::Snowflake,
            }),
        );
        let id = IdGenerator::generate(&router, "ws", "g", "bt")
            .await
            .unwrap();
        assert_eq!(id.as_u128(), 42);
    }

    #[tokio::test]
    async fn test_id_generator_batch_generate_trait_method_succeeds() {
        let router = AlgorithmRouter::new(Config::default(), None);
        insert_mock(
            &router,
            AlgorithmType::Segment,
            Arc::new(MockHealthyAlgorithm {
                alg_type: AlgorithmType::Segment,
            }),
        );
        let ids = IdGenerator::batch_generate(&router, "ws", "g", "bt", 5)
            .await
            .unwrap();
        assert_eq!(ids.len(), 5);
        assert!(ids.iter().all(|id| id.as_u128() == 42));
    }

    #[tokio::test]
    async fn test_id_generator_get_algorithm_name_with_biz_tag_override() {
        let router = AlgorithmRouter::new(Config::default(), None);
        router
            .set_algorithm("order".to_string(), AlgorithmType::Snowflake)
            .await;
        let name = IdGenerator::get_algorithm_name(&router, "ws", "g", "order")
            .await
            .unwrap();
        // Display 输出小写
        assert_eq!(name, "snowflake");
    }

    #[tokio::test]
    async fn test_id_generator_get_algorithm_name_without_override_returns_default() {
        let router = AlgorithmRouter::new(Config::default(), None);
        let name = IdGenerator::get_algorithm_name(&router, "ws", "g", "unknown_tag")
            .await
            .unwrap();
        // 默认算法 Segment，Display 输出小写
        assert_eq!(name, "segment");
    }

    #[tokio::test]
    async fn test_id_generator_health_check_empty_algorithms_returns_unhealthy() {
        let router = AlgorithmRouter::new(Config::default(), None);
        let status = IdGenerator::health_check(&router).await;
        match status {
            HealthStatus::Unhealthy(msg) => {
                assert!(msg.contains("No algorithms available"));
            }
            other => panic!("期望 Unhealthy, 实际为 {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_id_generator_health_check_all_healthy_returns_healthy() {
        let router = AlgorithmRouter::new(Config::default(), None);
        insert_mock(
            &router,
            AlgorithmType::Snowflake,
            Arc::new(MockHealthyAlgorithm {
                alg_type: AlgorithmType::Snowflake,
            }),
        );
        let status = IdGenerator::health_check(&router).await;
        assert!(matches!(status, HealthStatus::Healthy));
    }

    #[tokio::test]
    async fn test_id_generator_health_check_with_unhealthy_returns_unhealthy() {
        let router = AlgorithmRouter::new(Config::default(), None);
        insert_mock(
            &router,
            AlgorithmType::Snowflake,
            Arc::new(MockHealthyAlgorithm {
                alg_type: AlgorithmType::Snowflake,
            }),
        );
        insert_mock(
            &router,
            AlgorithmType::UuidV7,
            Arc::new(
                MockConfigurableAlgorithm::new(AlgorithmType::UuidV7)
                    .with_health(MockHealthKind::Unhealthy),
            ),
        );
        let status = IdGenerator::health_check(&router).await;
        match status {
            HealthStatus::Unhealthy(msg) => {
                assert!(msg.contains("Some algorithms are unhealthy"));
            }
            other => panic!("期望 Unhealthy, 实际为 {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_id_generator_health_check_with_only_degraded_returns_degraded() {
        let router = AlgorithmRouter::new(Config::default(), None);
        insert_mock(
            &router,
            AlgorithmType::Snowflake,
            Arc::new(
                MockConfigurableAlgorithm::new(AlgorithmType::Snowflake)
                    .with_health(MockHealthKind::Degraded),
            ),
        );
        let status = IdGenerator::health_check(&router).await;
        match status {
            HealthStatus::Degraded(msg) => {
                assert!(msg.contains("Some algorithms are degraded"));
            }
            other => panic!("期望 Degraded, 实际为 {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_id_generator_get_primary_algorithm_returns_default_debug_format() {
        let router = AlgorithmRouter::new(Config::default(), None);
        let name = IdGenerator::get_primary_algorithm(&router).await;
        // Debug 输出 PascalCase
        assert_eq!(name, "Segment");
    }

    #[tokio::test]
    async fn test_id_generator_get_degradation_manager_returns_arc_reference() {
        let router = AlgorithmRouter::new(Config::default(), None);
        let dm = IdGenerator::get_degradation_manager(&router);
        // 验证返回的是 Arc<DegradationManager>，且 strong_count >= 1
        assert!(Arc::strong_count(dm) >= 1);
    }

    #[tokio::test]
    async fn test_id_generator_generate_with_algorithm_trait_method_succeeds() {
        let router = AlgorithmRouter::new(Config::default(), None);
        insert_mock(
            &router,
            AlgorithmType::Snowflake,
            Arc::new(MockHealthyAlgorithm {
                alg_type: AlgorithmType::Snowflake,
            }),
        );
        let id = IdGenerator::generate_with_algorithm(
            &router,
            AlgorithmType::Snowflake,
            "ws",
            "g",
            "bt",
        )
        .await
        .unwrap();
        assert_eq!(id.as_u128(), 42);
    }

    #[tokio::test]
    async fn test_id_generator_batch_generate_with_algorithm_trait_method_succeeds() {
        let router = AlgorithmRouter::new(Config::default(), None);
        insert_mock(
            &router,
            AlgorithmType::UuidV7,
            Arc::new(MockHealthyAlgorithm {
                alg_type: AlgorithmType::UuidV7,
            }),
        );
        let ids = IdGenerator::batch_generate_with_algorithm(
            &router,
            AlgorithmType::UuidV7,
            "ws",
            "g",
            "bt",
            3,
        )
        .await
        .unwrap();
        assert_eq!(ids.len(), 3);
    }

    // ============== AlgorithmRouter::new 与 builder 测试 ==============

    #[tokio::test]
    async fn test_new_with_segment_default_builds_full_fallback_chain() {
        // 默认 Config: default = "segment"
        let router = AlgorithmRouter::new(Config::default(), None);
        assert_eq!(
            router.fallback_chain.to_vec(),
            vec![
                AlgorithmType::Snowflake,
                AlgorithmType::UuidV7,
                AlgorithmType::UuidV4,
            ]
        );
    }

    #[tokio::test]
    async fn test_new_with_snowflake_default_builds_partial_fallback_chain() {
        let mut config = Config::default();
        config.algorithm.default = "snowflake".to_string();
        let router = AlgorithmRouter::new(config, None);
        assert_eq!(
            router.fallback_chain.to_vec(),
            vec![AlgorithmType::UuidV7, AlgorithmType::UuidV4]
        );
    }

    #[tokio::test]
    async fn test_new_with_uuid_v7_default_builds_empty_fallback_chain() {
        let mut config = Config::default();
        config.algorithm.default = "uuid_v7".to_string();
        let router = AlgorithmRouter::new(config, None);
        assert!(router.fallback_chain.is_empty());
    }

    #[tokio::test]
    async fn test_with_cpu_monitor_builder_sets_monitor() {
        let router = AlgorithmRouter::new(Config::default(), None)
            .with_cpu_monitor(Arc::new(crate::core::algorithm::segment::CpuMonitor::new()));
        assert!(router.cpu_monitor.is_some());
    }

    // ============== generate_with_algorithm (inherent) 测试 ==============

    #[tokio::test]
    async fn test_generate_with_algorithm_inherent_success() {
        let router = AlgorithmRouter::new(Config::default(), None);
        insert_mock(
            &router,
            AlgorithmType::Snowflake,
            Arc::new(MockHealthyAlgorithm {
                alg_type: AlgorithmType::Snowflake,
            }),
        );
        let id = router
            .generate_with_algorithm(AlgorithmType::Snowflake, "ws", "g", "bt")
            .await
            .unwrap();
        assert_eq!(id.as_u128(), 42);
    }

    #[tokio::test]
    async fn test_generate_with_algorithm_inherent_unknown_algorithm_falls_back() {
        // 请求 UuidV4（不在 map），fallback chain = [Snowflake, UuidV7, UuidV4]
        // Snowflake 在 map 中且成功 → 返回 Ok
        let router = AlgorithmRouter::new(Config::default(), None);
        insert_mock(
            &router,
            AlgorithmType::Snowflake,
            Arc::new(MockHealthyAlgorithm {
                alg_type: AlgorithmType::Snowflake,
            }),
        );
        let id = router
            .generate_with_algorithm(AlgorithmType::UuidV4, "ws", "g", "bt")
            .await
            .unwrap();
        assert_eq!(id.as_u128(), 42);
    }

    #[tokio::test]
    async fn test_generate_with_algorithm_inherent_all_fail_returns_error() {
        let router = AlgorithmRouter::new(Config::default(), None);
        // 不插入任何算法 → fallback chain 中也无算法 → 返回 "All algorithms failed"
        let result = router
            .generate_with_algorithm(AlgorithmType::Snowflake, "ws", "g", "bt")
            .await;
        match result {
            Err(CoreError::InternalError(msg)) => {
                assert_eq!(msg, "All algorithms failed");
            }
            other => panic!("期望 InternalError, 实际为 {:?}", other),
        }
    }

    // ============== batch_generate_with_algorithm (inherent) 测试 ==============

    #[tokio::test]
    async fn test_batch_generate_with_algorithm_inherent_success() {
        let router = AlgorithmRouter::new(Config::default(), None);
        insert_mock(
            &router,
            AlgorithmType::Snowflake,
            Arc::new(MockHealthyAlgorithm {
                alg_type: AlgorithmType::Snowflake,
            }),
        );
        let batch = router
            .batch_generate_with_algorithm(AlgorithmType::Snowflake, "ws", "g", "bt", 4)
            .await
            .unwrap();
        assert_eq!(batch.ids.len(), 4);
        assert_eq!(batch.algorithm, AlgorithmType::Snowflake);
    }

    #[tokio::test]
    async fn test_batch_generate_with_algorithm_inherent_unknown_algorithm_falls_back() {
        let router = AlgorithmRouter::new(Config::default(), None);
        insert_mock(
            &router,
            AlgorithmType::Snowflake,
            Arc::new(MockHealthyAlgorithm {
                alg_type: AlgorithmType::Snowflake,
            }),
        );
        let batch = router
            .batch_generate_with_algorithm(AlgorithmType::UuidV4, "ws", "g", "bt", 2)
            .await
            .unwrap();
        assert_eq!(batch.ids.len(), 2);
        // 因 fallback 到 Snowflake，batch.algorithm 应为 Snowflake
        assert_eq!(batch.algorithm, AlgorithmType::Snowflake);
    }

    #[tokio::test]
    async fn test_batch_generate_with_algorithm_inherent_all_fail_returns_error() {
        let router = AlgorithmRouter::new(Config::default(), None);
        let result = router
            .batch_generate_with_algorithm(AlgorithmType::Snowflake, "ws", "g", "bt", 4)
            .await;
        match result {
            Err(CoreError::InternalError(msg)) => {
                assert_eq!(msg, "All algorithms failed");
            }
            other => panic!("期望 InternalError, 实际为 {:?}", other),
        }
    }

    // ============== generate_with_algorithm_internal fallback 路径测试 ==============

    #[tokio::test]
    async fn test_generate_internal_primary_succeeds_no_fallback() {
        // 主算法成功 → 不应触发 fallback
        let router = AlgorithmRouter::new(Config::default(), None);
        insert_mock(
            &router,
            AlgorithmType::Segment,
            Arc::new(MockHealthyAlgorithm {
                alg_type: AlgorithmType::Segment,
            }),
        );
        // 同时插入 fallback 算法但不应被调用（用失败 mock 验证）
        insert_mock(
            &router,
            AlgorithmType::Snowflake,
            Arc::new(
                MockConfigurableAlgorithm::new(AlgorithmType::Snowflake).with_generate_failure(),
            ),
        );
        let ctx = make_ctx("bt");
        let id = router.generate(&ctx).await.unwrap();
        assert_eq!(id.as_u128(), 42);
    }

    #[tokio::test]
    async fn test_generate_internal_primary_fails_first_fallback_succeeds() {
        let router = AlgorithmRouter::new(Config::default(), None);
        insert_mock(
            &router,
            AlgorithmType::Segment,
            Arc::new(
                MockConfigurableAlgorithm::new(AlgorithmType::Segment).with_generate_failure(),
            ),
        );
        insert_mock(
            &router,
            AlgorithmType::Snowflake,
            Arc::new(MockHealthyAlgorithm {
                alg_type: AlgorithmType::Snowflake,
            }),
        );
        let ctx = make_ctx("bt");
        let id = router.generate(&ctx).await.unwrap();
        assert_eq!(id.as_u128(), 42);
    }

    #[tokio::test]
    async fn test_generate_internal_primary_fails_all_fallbacks_fail_returns_original_error() {
        let router = AlgorithmRouter::new(Config::default(), None);
        insert_mock(
            &router,
            AlgorithmType::Segment,
            Arc::new(
                MockConfigurableAlgorithm::new(AlgorithmType::Segment).with_generate_failure(),
            ),
        );
        insert_mock(
            &router,
            AlgorithmType::Snowflake,
            Arc::new(
                MockConfigurableAlgorithm::new(AlgorithmType::Snowflake).with_generate_failure(),
            ),
        );
        insert_mock(
            &router,
            AlgorithmType::UuidV7,
            Arc::new(MockConfigurableAlgorithm::new(AlgorithmType::UuidV7).with_generate_failure()),
        );
        insert_mock(
            &router,
            AlgorithmType::UuidV4,
            Arc::new(MockConfigurableAlgorithm::new(AlgorithmType::UuidV4).with_generate_failure()),
        );
        let ctx = make_ctx("bt");
        let result = router.generate(&ctx).await;
        // 当主算法失败 + 所有 fallback 失败时，返回主算法的原始错误
        match result {
            Err(CoreError::InternalError(msg)) => {
                assert_eq!(msg, "mock generate failure");
            }
            other => panic!("期望 InternalError, 实际为 {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_generate_internal_primary_not_in_map_fallback_succeeds() {
        // Segment 不在 algorithms 中（map 只有 Snowflake），fallback 应成功
        let router = AlgorithmRouter::new(Config::default(), None);
        insert_mock(
            &router,
            AlgorithmType::Snowflake,
            Arc::new(MockHealthyAlgorithm {
                alg_type: AlgorithmType::Snowflake,
            }),
        );
        let ctx = make_ctx("bt");
        let id = router.generate(&ctx).await.unwrap();
        assert_eq!(id.as_u128(), 42);
    }

    #[tokio::test]
    async fn test_generate_internal_primary_not_in_map_all_fallbacks_fail() {
        // Segment 不在 map，fallback chain 中的算法也都不在 map → "All algorithms failed"
        let router = AlgorithmRouter::new(Config::default(), None);
        let ctx = make_ctx("bt");
        let result = router.generate(&ctx).await;
        match result {
            Err(CoreError::InternalError(msg)) => {
                assert_eq!(msg, "All algorithms failed");
            }
            other => panic!("期望 InternalError, 实际为 {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_generate_internal_primary_not_in_map_some_fallbacks_missing() {
        // Segment 不在 map，Snowflake 在 map 但失败，UuidV7 不在 map，UuidV4 成功
        let router = AlgorithmRouter::new(Config::default(), None);
        insert_mock(
            &router,
            AlgorithmType::Snowflake,
            Arc::new(
                MockConfigurableAlgorithm::new(AlgorithmType::Snowflake).with_generate_failure(),
            ),
        );
        insert_mock(
            &router,
            AlgorithmType::UuidV4,
            Arc::new(MockHealthyAlgorithm {
                alg_type: AlgorithmType::UuidV4,
            }),
        );
        let ctx = make_ctx("bt");
        let id = router.generate(&ctx).await.unwrap();
        assert_eq!(id.as_u128(), 42);
    }

    // ============== batch_generate_with_algorithm_internal fallback 路径测试 ==============

    #[tokio::test]
    async fn test_batch_internal_primary_succeeds_no_fallback() {
        let router = AlgorithmRouter::new(Config::default(), None);
        insert_mock(
            &router,
            AlgorithmType::Segment,
            Arc::new(MockHealthyAlgorithm {
                alg_type: AlgorithmType::Segment,
            }),
        );
        let ctx = make_ctx("bt");
        let batch = router.batch_generate(&ctx, 3).await.unwrap();
        assert_eq!(batch.ids.len(), 3);
        assert_eq!(batch.algorithm, AlgorithmType::Segment);
    }

    #[tokio::test]
    async fn test_batch_internal_primary_fails_first_fallback_succeeds() {
        let router = AlgorithmRouter::new(Config::default(), None);
        insert_mock(
            &router,
            AlgorithmType::Segment,
            Arc::new(MockConfigurableAlgorithm::new(AlgorithmType::Segment).with_batch_failure()),
        );
        insert_mock(
            &router,
            AlgorithmType::Snowflake,
            Arc::new(MockHealthyAlgorithm {
                alg_type: AlgorithmType::Snowflake,
            }),
        );
        let ctx = make_ctx("bt");
        let batch = router.batch_generate(&ctx, 5).await.unwrap();
        assert_eq!(batch.ids.len(), 5);
        assert_eq!(batch.algorithm, AlgorithmType::Snowflake);
    }

    #[tokio::test]
    async fn test_batch_internal_primary_fails_all_fallbacks_fail_returns_original_error() {
        let router = AlgorithmRouter::new(Config::default(), None);
        insert_mock(
            &router,
            AlgorithmType::Segment,
            Arc::new(MockConfigurableAlgorithm::new(AlgorithmType::Segment).with_batch_failure()),
        );
        insert_mock(
            &router,
            AlgorithmType::Snowflake,
            Arc::new(MockConfigurableAlgorithm::new(AlgorithmType::Snowflake).with_batch_failure()),
        );
        insert_mock(
            &router,
            AlgorithmType::UuidV7,
            Arc::new(MockConfigurableAlgorithm::new(AlgorithmType::UuidV7).with_batch_failure()),
        );
        insert_mock(
            &router,
            AlgorithmType::UuidV4,
            Arc::new(MockConfigurableAlgorithm::new(AlgorithmType::UuidV4).with_batch_failure()),
        );
        let ctx = make_ctx("bt");
        let result = router.batch_generate(&ctx, 2).await;
        match result {
            Err(CoreError::InternalError(msg)) => {
                assert_eq!(msg, "mock batch failure");
            }
            other => panic!("期望 InternalError, 实际为 {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_batch_internal_primary_not_in_map_fallback_succeeds() {
        let router = AlgorithmRouter::new(Config::default(), None);
        insert_mock(
            &router,
            AlgorithmType::Snowflake,
            Arc::new(MockHealthyAlgorithm {
                alg_type: AlgorithmType::Snowflake,
            }),
        );
        let ctx = make_ctx("bt");
        let batch = router.batch_generate(&ctx, 4).await.unwrap();
        assert_eq!(batch.ids.len(), 4);
        assert_eq!(batch.algorithm, AlgorithmType::Snowflake);
    }

    #[tokio::test]
    async fn test_batch_internal_primary_not_in_map_all_fallbacks_fail() {
        let router = AlgorithmRouter::new(Config::default(), None);
        let ctx = make_ctx("bt");
        let result = router.batch_generate(&ctx, 2).await;
        match result {
            Err(CoreError::InternalError(msg)) => {
                assert_eq!(msg, "All algorithms failed");
            }
            other => panic!("期望 InternalError, 实际为 {:?}", other),
        }
    }

    // ============== health_check (inherent) 测试 ==============

    #[tokio::test]
    async fn test_health_check_inherent_empty_returns_empty_vec() {
        let router = AlgorithmRouter::new(Config::default(), None);
        let statuses = router.health_check().await;
        assert!(statuses.is_empty());
    }

    #[tokio::test]
    async fn test_health_check_inherent_returns_status_for_each_algorithm() {
        let router = AlgorithmRouter::new(Config::default(), None);
        insert_mock(
            &router,
            AlgorithmType::Snowflake,
            Arc::new(MockHealthyAlgorithm {
                alg_type: AlgorithmType::Snowflake,
            }),
        );
        insert_mock(
            &router,
            AlgorithmType::UuidV7,
            Arc::new(
                MockConfigurableAlgorithm::new(AlgorithmType::UuidV7)
                    .with_health(MockHealthKind::Degraded),
            ),
        );
        let statuses = router.health_check().await;
        assert_eq!(statuses.len(), 2);
        let snowflake_status = statuses
            .iter()
            .find(|(t, _)| *t == AlgorithmType::Snowflake)
            .map(|(_, s)| s);
        assert!(matches!(snowflake_status, Some(HealthStatus::Healthy)));
        let uuid_v7_status = statuses
            .iter()
            .find(|(t, _)| *t == AlgorithmType::UuidV7)
            .map(|(_, s)| s);
        assert!(matches!(uuid_v7_status, Some(HealthStatus::Degraded(_))));
    }

    // ============== metrics (inherent) 测试 ==============

    #[tokio::test]
    async fn test_metrics_inherent_empty_returns_empty_vec() {
        let router = AlgorithmRouter::new(Config::default(), None);
        let snapshots = router.metrics().await;
        assert!(snapshots.is_empty());
    }

    #[tokio::test]
    async fn test_metrics_inherent_returns_snapshot_for_each_algorithm() {
        let router = AlgorithmRouter::new(Config::default(), None);
        insert_mock(
            &router,
            AlgorithmType::Snowflake,
            Arc::new(MockHealthyAlgorithm {
                alg_type: AlgorithmType::Snowflake,
            }),
        );
        insert_mock(
            &router,
            AlgorithmType::UuidV7,
            Arc::new(MockHealthyAlgorithm {
                alg_type: AlgorithmType::UuidV7,
            }),
        );
        let snapshots = router.metrics().await;
        assert_eq!(snapshots.len(), 2);
        let algorithm_types: Vec<_> = snapshots.iter().map(|(t, _)| *t).collect();
        assert!(algorithm_types.contains(&AlgorithmType::Snowflake));
        assert!(algorithm_types.contains(&AlgorithmType::UuidV7));
    }

    // ============== get_degradation_manager (inherent) 测试 ==============

    #[tokio::test]
    async fn test_get_degradation_manager_inherent_returns_reference() {
        let router = AlgorithmRouter::new(Config::default(), None);
        let dm_ref = router.get_degradation_manager();
        assert!(Arc::strong_count(dm_ref) >= 1);
    }

    // ============== check_health_and_update_degradation 测试 ==============

    #[tokio::test]
    async fn test_check_health_and_update_degradation_does_not_panic_with_no_algorithms() {
        let router = AlgorithmRouter::new(Config::default(), None);
        // 应当不 panic
        router.check_health_and_update_degradation().await;
    }

    // ============== shutdown 测试 ==============

    #[tokio::test]
    async fn test_shutdown_empty_does_not_panic() {
        let router = AlgorithmRouter::new(Config::default(), None);
        router.shutdown().await;
    }

    #[tokio::test]
    async fn test_shutdown_with_successful_algorithms_does_not_panic() {
        let router = AlgorithmRouter::new(Config::default(), None);
        insert_mock(
            &router,
            AlgorithmType::Snowflake,
            Arc::new(MockHealthyAlgorithm {
                alg_type: AlgorithmType::Snowflake,
            }),
        );
        insert_mock(
            &router,
            AlgorithmType::UuidV7,
            Arc::new(MockHealthyAlgorithm {
                alg_type: AlgorithmType::UuidV7,
            }),
        );
        router.shutdown().await;
    }

    #[tokio::test]
    async fn test_shutdown_with_failing_algorithm_does_not_panic() {
        // 某个算法 shutdown 失败时，应记录日志但不 panic，继续 shutdown 其他算法
        let router = AlgorithmRouter::new(Config::default(), None);
        insert_mock(
            &router,
            AlgorithmType::Snowflake,
            Arc::new(
                MockConfigurableAlgorithm::new(AlgorithmType::Snowflake).with_shutdown_failure(),
            ),
        );
        insert_mock(
            &router,
            AlgorithmType::UuidV7,
            Arc::new(MockHealthyAlgorithm {
                alg_type: AlgorithmType::UuidV7,
            }),
        );
        router.shutdown().await;
        // 不 panic 即通过
    }
}
