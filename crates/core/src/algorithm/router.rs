use crate::algorithm::{
    AlgorithmBuilder, AlgorithmMetricsSnapshot, GenerateContext, HealthStatus, IdAlgorithm,
    IdGenerator,
};
use crate::config::Config;
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
        if let Some(alg) = self.current_algorithm.get(biz_tag) {
            Ok(format!("{:?}", alg))
        } else {
            Ok(format!(
                "{:?}",
                self.config.algorithm.get_default_algorithm()
            ))
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
        format!("{:?}", self.config.algorithm.get_default_algorithm())
    }
}

pub struct AlgorithmRouter {
    config: Config,
    algorithms: DashMap<AlgorithmType, Arc<dyn IdAlgorithm>>,
    fallback_chain: Vec<AlgorithmType>,
    current_algorithm: DashMap<String, AlgorithmType>,
}

impl AlgorithmRouter {
    pub fn new(config: Config) -> Self {
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

        Self {
            config,
            algorithms: DashMap::new(),
            fallback_chain,
            current_algorithm: DashMap::new(),
        }
    }

    pub async fn initialize(&mut self) -> Result<()> {
        let mut errors = Vec::new();

        for alg_type in [
            AlgorithmType::Segment,
            AlgorithmType::Snowflake,
            AlgorithmType::UuidV7,
            AlgorithmType::UuidV4,
        ] {
            match AlgorithmBuilder::new(alg_type).build(&self.config).await {
                Ok(mut algo) => {
                    if let Err(e) = algo.initialize(&self.config).await {
                        warn!("Failed to initialize algorithm {:?}: {}", alg_type, e);
                        errors.push((alg_type, e));
                        continue;
                    }
                    self.algorithms.insert(alg_type, Arc::from(algo));
                    info!("Algorithm {:?} initialized successfully", alg_type);
                }
                Err(e) => {
                    warn!("Failed to build algorithm {:?}: {}", alg_type, e);
                    errors.push((alg_type, e));
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
        self.generate_with_algorithm(algorithm, ctx).await
    }

    pub async fn batch_generate(&self, ctx: &GenerateContext, size: usize) -> Result<IdBatch> {
        let algorithm = self.get_algorithm(ctx);
        self.batch_generate_with_algorithm(algorithm, ctx, size)
            .await
    }

    fn get_algorithm(&self, ctx: &GenerateContext) -> AlgorithmType {
        if let Some(alg) = self.current_algorithm.get(&ctx.biz_tag) {
            return *alg;
        }
        self.config.algorithm.get_default_algorithm()
    }

    pub fn set_algorithm(&self, biz_tag: String, algorithm: AlgorithmType) {
        self.current_algorithm.insert(biz_tag, algorithm);
    }

    async fn generate_with_algorithm(
        &self,
        algorithm: AlgorithmType,
        ctx: &GenerateContext,
    ) -> Result<Id> {
        if let Some(alg) = self.algorithms.get(&algorithm) {
            match alg.generate(ctx).await {
                Ok(id) => return Ok(id),
                Err(e) => {
                    debug!("Algorithm {} failed: {:?}", algorithm, e);
                    for fallback in &self.fallback_chain {
                        if let Some(fallback_alg) = self.algorithms.get(fallback) {
                            match fallback_alg.generate(ctx).await {
                                Ok(id) => return Ok(id),
                                Err(_) => continue,
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
                    Ok(id) => return Ok(id),
                    Err(_) => continue,
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
        if let Some(alg) = self.algorithms.get(&algorithm) {
            match alg.batch_generate(ctx, size).await {
                Ok(batch) => return Ok(batch),
                Err(e) => {
                    debug!("Algorithm {} batch failed: {:?}", algorithm, e);
                    for fallback in &self.fallback_chain {
                        if let Some(fallback_alg) = self.algorithms.get(fallback) {
                            match fallback_alg.batch_generate(ctx, size).await {
                                Ok(batch) => return Ok(batch),
                                Err(_) => continue,
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
                    Ok(batch) => return Ok(batch),
                    Err(_) => continue,
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
        let mut router = AlgorithmRouter::new(config);

        let result = router.initialize().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_algorithm_router_generate() {
        let config = Config::default();
        let mut router = AlgorithmRouter::new(config);
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
        let mut router = AlgorithmRouter::new(config);
        router.set_algorithm("test".to_string(), AlgorithmType::Snowflake);
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
        let router = AlgorithmRouter::new(config);

        router.set_algorithm("order".to_string(), AlgorithmType::Snowflake);

        let entry = router.current_algorithm.get("order");
        assert!(entry.is_some());
        assert_eq!(*entry.unwrap(), AlgorithmType::Snowflake);
    }
}
