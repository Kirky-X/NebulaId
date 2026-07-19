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

//! System / observability handlers: health, readiness, metrics,
//! and the background key-rotation task launcher (rule 25 split).

use crate::server::models::{AlgorithmMetrics, HealthResponse, MetricsResponse, ReadyResponse};
use std::sync::atomic::Ordering;

// KeyRotationHandle lives in `api_key_handlers` (it owns the API key repo
// shutdown channel); we only return it from here.
use super::api_key_handlers::KeyRotationHandle;

impl super::ApiHandlers {
    pub async fn health(&self) -> HealthResponse {
        let health_status = self.id_generator.health_check().await;
        HealthResponse {
            status: if health_status.is_healthy() {
                crate::server::models::HealthStatus::Healthy
            } else {
                crate::server::models::HealthStatus::Degraded
            },
            algorithm: self.id_generator.get_primary_algorithm().await.to_string(),
        }
    }

    pub async fn ready(&self) -> ReadyResponse {
        let db_metrics = self.config_service.get_database_metrics().await;
        let cache_metrics = self.config_service.get_cache_metrics().await;

        let db_healthy = db_metrics.status == crate::server::models::HealthStatus::Healthy;
        let cache_healthy = cache_metrics.status == crate::server::models::HealthStatus::Healthy;

        let ready = db_healthy && cache_healthy;
        ReadyResponse {
            ready,
            database: db_healthy,
            cache: cache_healthy,
            message: if ready {
                "Ready to serve traffic".to_string()
            } else {
                "Not ready: database or cache unavailable".to_string()
            },
        }
    }

    pub async fn metrics(&self) -> MetricsResponse {
        let algorithm_metrics = self.config_service.get_algorithm_metrics().await;
        let algorithms = algorithm_metrics
            .into_iter()
            .map(
                |(alg_type, snapshot): (
                    crate::core::types::AlgorithmType,
                    crate::core::algorithm::AlgorithmMetricsSnapshot,
                )| AlgorithmMetrics {
                    algorithm: alg_type.to_string(),
                    status: crate::server::models::HealthStatus::Healthy,
                    total_generated: snapshot.total_generated,
                    total_failed: snapshot.total_failed,
                    cache_hit_rate: snapshot.cache_hit_rate,
                },
            )
            .collect();

        let database = self.config_service.get_database_metrics().await;
        let cache = self.config_service.get_cache_metrics().await;

        MetricsResponse {
            total_requests: self.metrics.total_requests.load(Ordering::SeqCst),
            successful_generations: self.metrics.successful_generations.load(Ordering::SeqCst),
            failed_generations: self.metrics.failed_generations.load(Ordering::SeqCst),
            total_ids_generated: self.metrics.total_ids_generated.load(Ordering::SeqCst),
            avg_latency_ms: self.metrics.avg_latency_ms.load(Ordering::SeqCst),
            uptime_seconds: std::time::Instant::now()
                .duration_since(self.start_time)
                .as_secs(),
            database,
            cache,
            algorithms,
        }
    }

    /// Start background key rotation task.
    /// Returns a handle that can be used to stop the task.
    pub fn start_key_rotation_task(
        &self,
        check_interval: std::time::Duration,
        max_key_age_days: i64,
    ) -> Option<KeyRotationHandle> {
        let repo = match self.api_key_repo.as_ref() {
            Some(r) => r.clone(),
            None => {
                tracing::warn!(
                    "{}",
                    t!("log.server.handlers.system_handlers.cannot_start_key_rotation")
                );
                return None;
            }
        };

        let (shutdown_tx, mut shutdown_rx) = tokio::sync::watch::channel(false);

        let _handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(check_interval);
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        tracing::debug!(
                            "{}",
                            t!("log.server.handlers.system_handlers.running_key_rotation_check")
                        );

                        match repo.get_keys_older_than(max_key_age_days).await {
                            Ok(old_keys) => {
                                for key in old_keys {
                                    tracing::info!(
                                        event = "auto_rotating_key",
                                        key_id = key.key_id,
                                        age_days = max_key_age_days
                                    );

                                    const GRACE_PERIOD_SECONDS: u64 = 7 * 24 * 60 * 60;
                                    if let Err(e) =
                                        repo.rotate_api_key(&key.key_id, GRACE_PERIOD_SECONDS).await
                                    {
                                        tracing::error!(
                                            event = "key_rotation_failed",
                                            key_id = key.key_id,
                                            error = %e
                                        );
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::error!(event = "key_rotation_check_failed", error = %e);
                            }
                        }
                    }
                    _ = shutdown_rx.changed() => {
                        tracing::info!(
                            "{}",
                            t!("log.server.handlers.system_handlers.key_rotation_shutting_down")
                        );
                        break;
                    }
                }
            }
        });

        tracing::info!(
            event = "key_rotation_task_started",
            check_interval_secs = check_interval.as_secs(),
            max_age_days = max_key_age_days
        );

        Some(KeyRotationHandle { shutdown_tx })
    }
}

#[cfg(test)]
mod tests {
    use crate::server::config::management::{ConfigManagementService, ConfigManager};
    use crate::server::config::HotReloadConfig;
    use crate::server::handlers::mock_generator::MockIdGenerator;
    use std::sync::Arc;

    fn create_test_api_handlers() -> (Arc<super::super::ApiHandlers>, Arc<MockIdGenerator>) {
        let mock_gen = Arc::new(MockIdGenerator::new());
        let config = crate::core::config::Config::default();
        let hot_config = Arc::new(HotReloadConfig::new(
            config,
            "config/config.toml".to_string(),
        ));

        let router = Arc::new(crate::core::algorithm::AlgorithmRouter::new(
            crate::core::config::Config::default(),
            None,
        ));

        let config_service: Arc<dyn ConfigManagementService> =
            Arc::new(ConfigManager::new(hot_config, router));
        let handlers = super::super::ApiHandlers::new(mock_gen.clone(), config_service);
        (Arc::new(handlers), mock_gen)
    }

    #[tokio::test]
    async fn test_handle_metrics() {
        let (handlers, _router) = create_test_api_handlers();
        let response = handlers.metrics().await;
        assert!(response.total_requests == response.total_requests);
    }
}
