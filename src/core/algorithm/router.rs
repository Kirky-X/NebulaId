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
}
