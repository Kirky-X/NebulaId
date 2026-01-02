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

use crate::algorithm::{
    AlgorithmBuilder, AlgorithmMetricsSnapshot, DegradationManager, DynAuditLogger,
    GenerateContext, HealthStatus, IdAlgorithm, IdGenerator,
};
use crate::config::Config;
#[cfg(feature = "etcd")]
use crate::coordinator::EtcdClusterHealthMonitor;
use crate::database::SegmentRepository;
use crate::types::{AlgorithmType, CoreError, Id, IdBatch, Result};
use async_trait::async_trait;
use dashmap::DashMap;
use std::iter::Iterator;
use std::sync::Arc;
use tracing::{debug, error, info, warn};

#[async_trait]
impl IdGenerator for AlgorithmRouter {
    async fn generate(&self, workspace: &str, group: &str, biz_tag: &str) -> Result<Id> {
        let ctx = GenerateContext {
            workspace_id: workspace.to_string(),
            group_id: group.to_string(),
            biz_tag: biz_tag.to_string(),
            format: crate::types::IdFormat::Numeric,
            prefix: None,
        };
        self.generate(&ctx).await
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
            format: crate::types::IdFormat::Numeric,
            prefix: None,
        };
        let batch = self.batch_generate(&ctx, size).await?;
        Ok(batch.ids)
    }

    async fn get_algorithm_name(
        &self,
        _workspace: &str,
        _group: &str,
        biz_tag: &str,
    ) -> Result<String> {
        if let Some(alg_ref) = self.current_algorithm.get(biz_tag) {
            let alg_type = alg_ref.value().clone();
            match alg_type {
                AlgorithmType::Segment => Ok("segment".to_string()),
                AlgorithmType::Snowflake => Ok("snowflake".to_string()),
                AlgorithmType::UuidV7 => Ok("uuid_v7".to_string()),
                AlgorithmType::UuidV4 => Ok("uuid_v4".to_string()),
            }
        } else {
            match self.config.algorithm.get_default_algorithm() {
                AlgorithmType::Segment => Ok("segment".to_string()),
                AlgorithmType::Snowflake => Ok("snowflake".to_string()),
                AlgorithmType::UuidV7 => Ok("uuid_v7".to_string()),
                AlgorithmType::UuidV4 => Ok("uuid_v4".to_string()),
            }
        }
    }

    async fn health_check(&self) -> HealthStatus {
        let statuses = self.health_check();
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
        format!("{}", self.config.algorithm.get_default_algorithm())
    }

    fn get_degradation_manager(&self) -> &Arc<DegradationManager> {
        &self.degradation_manager
    }

    fn set_algorithm(&self, biz_tag: &str, algorithm: AlgorithmType) {
        self.current_algorithm
            .insert(biz_tag.to_string(), algorithm);
    }
}

pub struct AlgorithmRouter {
    config: Config,
    algorithms: DashMap<AlgorithmType, Arc<dyn IdAlgorithm>>,
    fallback_chain: Vec<AlgorithmType>,
    current_algorithm: DashMap<String, AlgorithmType>,
    degradation_manager: Arc<DegradationManager>,
    #[cfg(feature = "etcd")]
    etcd_health_monitor: Option<Arc<EtcdClusterHealthMonitor>>,
    #[cfg(not(feature = "etcd"))]
    etcd_health_monitor: Option<()>,
    segment_repository: Option<Arc<dyn SegmentRepository>>,
}

impl AlgorithmRouter {
    pub fn new(
        config: Config,
        audit_logger: Option<DynAuditLogger>,
        segment_repository: Option<Arc<dyn SegmentRepository>>,
    ) -> Self {
        let mut fallback_chain = Vec::new();

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
        degradation_manager.set_fallback_chain(fallback_chain.clone());

        Self {
            config,
            algorithms: DashMap::new(),
            fallback_chain,
            current_algorithm: DashMap::new(),
            degradation_manager,
            etcd_health_monitor: None,
            segment_repository,
        }
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

    pub async fn initialize(&self) -> Result<()> {
        let mut errors = Vec::new();

        for alg_type in [
            AlgorithmType::Segment,
            AlgorithmType::Snowflake,
            AlgorithmType::UuidV7,
            AlgorithmType::UuidV4,
        ] {
            let mut builder = AlgorithmBuilder::new(alg_type);
            #[cfg(feature = "etcd")]
            if let Some(ref monitor) = self.etcd_health_monitor {
                builder = builder.with_etcd_health_monitor(monitor.clone());
            }
            if let Some(ref repository) = self.segment_repository {
                builder = builder.with_segment_repository(repository.clone());
            }

            match builder.build(&self.config).await {
                Ok(algo) => {
                    let alg_arc: Arc<dyn IdAlgorithm> = Arc::from(algo);
                    self.algorithms.insert(alg_type, alg_arc.clone());
                    self.degradation_manager
                        .register_algorithm(alg_type, alg_arc);
                    info!("Algorithm {:?} initialized successfully", alg_type);
                }
                Err(_e) => {
                    warn!("Failed to build algorithm {:?}: {}", alg_type, _e);
                    errors.push((alg_type, _e));
                }
            }
        }

        if self.algorithms.is_empty() {
            return Err(CoreError::InternalError(
                "No algorithms available".to_string(),
            ));
        }

        Ok(())
    }

    pub async fn generate(&self, ctx: &GenerateContext) -> Result<Id> {
        let algorithm = self.get_algorithm(ctx);
        tracing::info!(
            "Generating ID for biz_tag='{}' using algorithm='{}'",
            ctx.biz_tag,
            algorithm
        );
        self.generate_with_algorithm(algorithm, ctx).await
    }

    pub async fn batch_generate(&self, ctx: &GenerateContext, size: usize) -> Result<IdBatch> {
        let algorithm = self.get_algorithm(ctx);
        self.batch_generate_with_algorithm(algorithm, ctx, size)
            .await
    }

    fn get_algorithm(&self, ctx: &GenerateContext) -> AlgorithmType {
        if let Some(alg) = self.current_algorithm.get(&ctx.biz_tag) {
            return alg.value().clone();
        }
        self.config.algorithm.get_default_algorithm()
    }

    pub fn set_algorithm(&self, biz_tag: &str, algorithm: AlgorithmType) {
        tracing::info!(
            "Setting algorithm for biz_tag='{}' to '{}'",
            biz_tag,
            algorithm
        );
        self.current_algorithm
            .insert(biz_tag.to_string(), algorithm);
    }

    async fn generate_with_algorithm(
        &self,
        algorithm: AlgorithmType,
        ctx: &GenerateContext,
    ) -> Result<Id> {
        tracing::info!(
            "Attempting to generate ID with algorithm='{}' for biz_tag='{}'",
            algorithm,
            ctx.biz_tag
        );
        let effective_algorithm = algorithm;

        if let Some(alg) = self.algorithms.get(&effective_algorithm) {
            tracing::debug!(
                "Found algorithm implementation for '{}'",
                effective_algorithm
            );
            match alg.generate(ctx).await {
                Ok(id) => {
                    tracing::info!(
                        "Successfully generated ID using '{}': {}",
                        effective_algorithm,
                        id
                    );
                    self.degradation_manager
                        .record_generation_result(effective_algorithm, true)
                        .await;
                    return Ok(id);
                }
                Err(e) => {
                    debug!("Algorithm {} failed: {:?}", effective_algorithm, e);
                    self.degradation_manager
                        .record_generation_result(effective_algorithm, false)
                        .await;
                    for fallback in &self.fallback_chain {
                        if let Some(fallback_alg) = self.algorithms.get(fallback) {
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
                    return Err(e);
                }
            }
        }

        for fallback in &self.fallback_chain {
            if let Some(fallback_alg) = self.algorithms.get(fallback) {
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

    async fn batch_generate_with_algorithm(
        &self,
        algorithm: AlgorithmType,
        ctx: &GenerateContext,
        size: usize,
    ) -> Result<IdBatch> {
        let effective_algorithm = algorithm;

        if let Some(alg) = self.algorithms.get(&effective_algorithm) {
            match alg.batch_generate(ctx, size).await {
                Ok(batch) => {
                    self.degradation_manager
                        .record_generation_result(effective_algorithm, true)
                        .await;
                    return Ok(batch);
                }
                Err(e) => {
                    debug!("Algorithm {} batch failed: {:?}", effective_algorithm, e);
                    self.degradation_manager
                        .record_generation_result(effective_algorithm, false)
                        .await;
                    for fallback in &self.fallback_chain {
                        if let Some(fallback_alg) = self.algorithms.get(fallback) {
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
            if let Some(fallback_alg) = self.algorithms.get(fallback) {
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

    pub fn health_check(&self) -> Vec<(AlgorithmType, HealthStatus)> {
        self.algorithms
            .iter()
            .map(|entry| (*entry.key(), entry.health_check()))
            .collect()
    }

    pub fn metrics(&self) -> Vec<(AlgorithmType, AlgorithmMetricsSnapshot)> {
        self.algorithms
            .iter()
            .map(|entry| (*entry.key(), entry.metrics()))
            .collect()
    }

    pub fn get_degradation_manager(&self) -> &Arc<DegradationManager> {
        &self.degradation_manager
    }

    pub async fn check_health_and_update_degradation(&self) {
        self.degradation_manager.check_all_health().await;
    }

    pub async fn shutdown(&self) {
        for entry in self.algorithms.iter() {
            if let Err(e) = entry.shutdown().await {
                error!(
                    "Error shutting down algorithm {:?}: {}",
                    entry.algorithm_type(),
                    e
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[tokio::test]
    async fn test_algorithm_router_initialize() {
        let config = Config::default();
        let router = AlgorithmRouter::new(config, None, None);

        let result = router.initialize().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_algorithm_router_generate() {
        let config = Config::default();
        let router = AlgorithmRouter::new(config, None, None);
        router.initialize().await.unwrap();

        let ctx = GenerateContext {
            workspace_id: "test".to_string(),
            group_id: "test".to_string(),
            biz_tag: "test".to_string(),
            format: crate::types::IdFormat::Numeric,
            prefix: None,
        };

        let id = router.generate(&ctx).await.unwrap();
        assert!(id.as_u128() > 0);
    }

    #[tokio::test]
    async fn test_algorithm_router_batch_generate() {
        let config = Config::default();
        let router = AlgorithmRouter::new(config, None, None);
        router.set_algorithm("test", AlgorithmType::Snowflake);
        router.initialize().await.unwrap();

        let ctx = GenerateContext {
            workspace_id: "test".to_string(),
            group_id: "test".to_string(),
            biz_tag: "test".to_string(),
            format: crate::types::IdFormat::Numeric,
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

    #[test]
    fn test_set_algorithm() {
        let config = Config::default();
        let router = AlgorithmRouter::new(config, None, None);

        router.set_algorithm("order", AlgorithmType::Snowflake);

        let entry = router.current_algorithm.get("order");
        assert!(entry.is_some());
        assert_eq!(*entry.unwrap(), AlgorithmType::Snowflake);
    }
}
