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

use crate::config_hot_reload::HotReloadConfig;
use crate::models::{
    AlgorithmConfigInfo, AppConfigInfo, ConfigResponse, DatabaseConfigInfo, LoggingConfigInfo,
    MonitoringConfigInfo, RateLimitConfigInfo, RedisConfigInfo, SegmentConfigInfo,
    SetAlgorithmRequest, SetAlgorithmResponse, SnowflakeConfigInfo, TlsConfigInfo,
    UpdateConfigResponse, UpdateLoggingRequest, UpdateRateLimitRequest, UuidV7ConfigInfo,
};
use nebula_core::config::Config;
use nebula_core::types::id::AlgorithmType;
use std::sync::Arc;
use tokio::sync::RwLock;
use validator::Validate;

pub struct ConfigManagementService {
    hot_config: Arc<HotReloadConfig>,
    rate_limiter: Arc<RwLock<Option<(u32, u32)>>>,
    algorithm_router: Arc<nebula_core::algorithm::AlgorithmRouter>,
}

impl ConfigManagementService {
    pub fn new(
        hot_config: Arc<HotReloadConfig>,
        algorithm_router: Arc<nebula_core::algorithm::AlgorithmRouter>,
    ) -> Self {
        Self {
            hot_config,
            rate_limiter: Arc::new(RwLock::new(None)),
            algorithm_router,
        }
    }

    pub fn get_config(&self) -> ConfigResponse {
        let config = self.hot_config.get_config();
        Self::config_to_response(&config)
    }

    fn config_to_response(config: &Config) -> ConfigResponse {
        ConfigResponse {
            app: AppConfigInfo {
                name: config.app.name.clone(),
                host: config.app.host.clone(),
                http_port: config.app.http_port,
                grpc_port: config.app.grpc_port,
                dc_id: config.app.dc_id,
                worker_id: config.app.worker_id,
            },
            database: DatabaseConfigInfo {
                engine: config.database.engine.clone(),
                host: config.database.host.clone(),
                port: config.database.port,
                database: config.database.database.clone(),
                max_connections: config.database.max_connections,
                min_connections: config.database.min_connections,
            },
            redis: RedisConfigInfo {
                url: config.redis.url.clone(),
                pool_size: config.redis.pool_size,
                key_prefix: config.redis.key_prefix.clone(),
                ttl_seconds: config.redis.ttl_seconds,
            },
            algorithm: AlgorithmConfigInfo {
                default: config.algorithm.default.clone(),
                segment: SegmentConfigInfo {
                    base_step: config.algorithm.segment.base_step,
                    min_step: config.algorithm.segment.min_step,
                    max_step: config.algorithm.segment.max_step,
                    switch_threshold: config.algorithm.segment.switch_threshold,
                },
                snowflake: SnowflakeConfigInfo {
                    datacenter_id_bits: config.algorithm.snowflake.datacenter_id_bits,
                    worker_id_bits: config.algorithm.snowflake.worker_id_bits,
                    sequence_bits: config.algorithm.snowflake.sequence_bits,
                    clock_drift_threshold_ms: config.algorithm.snowflake.clock_drift_threshold_ms,
                },
                uuid_v7: UuidV7ConfigInfo {
                    enabled: config.algorithm.uuid_v7.enabled,
                },
            },
            monitoring: MonitoringConfigInfo {
                metrics_enabled: config.monitoring.metrics_enabled,
                metrics_path: config.monitoring.metrics_path.clone(),
                tracing_enabled: config.monitoring.tracing_enabled,
            },
            logging: LoggingConfigInfo {
                level: config.logging.level.clone(),
                format: config.logging.format.clone(),
                include_location: config.logging.include_location,
            },
            rate_limit: RateLimitConfigInfo {
                enabled: config.rate_limit.enabled,
                default_rps: config.rate_limit.default_rps,
                burst_size: config.rate_limit.burst_size,
            },
            tls: TlsConfigInfo {
                enabled: config.tls.enabled,
                http_enabled: config.tls.http_enabled,
                grpc_enabled: config.tls.grpc_enabled,
                has_cert: !config.tls.cert_path.is_empty(),
            },
        }
    }

    pub async fn update_rate_limit(&self, req: UpdateRateLimitRequest) -> UpdateConfigResponse {
        if let Err(e) = req.validate() {
            return UpdateConfigResponse {
                success: false,
                message: format!("Validation error: {}", e),
                config: None,
            };
        }

        let mut config = self.hot_config.get_config();

        if let Some(rps) = req.default_rps {
            config.rate_limit.default_rps = rps;
        }
        if let Some(burst) = req.burst_size {
            config.rate_limit.burst_size = burst;
        }

        {
            let mut rate_limiter_guard = self.rate_limiter.write().await;
            *rate_limiter_guard =
                Some((config.rate_limit.default_rps, config.rate_limit.burst_size));
        }

        self.hot_config.update_config(config.clone());

        UpdateConfigResponse {
            success: true,
            message: "Rate limit configuration updated successfully".to_string(),
            config: Some(Self::config_to_response(&config)),
        }
    }

    pub async fn update_logging(&self, req: UpdateLoggingRequest) -> UpdateConfigResponse {
        if let Err(e) = req.validate() {
            return UpdateConfigResponse {
                success: false,
                message: format!("Validation error: {}", e),
                config: None,
            };
        }

        let mut config = self.hot_config.get_config();

        if let Some(level) = req.level {
            let valid_levels = ["debug", "info", "warn", "error"];
            if valid_levels.contains(&level.as_str()) {
                config.logging.level = level;
            } else {
                return UpdateConfigResponse {
                    success: false,
                    message: format!(
                        "Invalid log level. Valid levels: {}",
                        valid_levels.join(", ")
                    ),
                    config: None,
                };
            }
        }

        self.hot_config.update_config(config.clone());

        UpdateConfigResponse {
            success: true,
            message: "Logging configuration updated successfully".to_string(),
            config: Some(Self::config_to_response(&config)),
        }
    }

    pub async fn reload_config(&self) -> UpdateConfigResponse {
        match self.hot_config.reload_from_file().await {
            Ok(_) => UpdateConfigResponse {
                success: true,
                message: "Configuration reloaded from file successfully".to_string(),
                config: Some(Self::config_to_response(&self.hot_config.get_config())),
            },
            Err(e) => UpdateConfigResponse {
                success: false,
                message: format!("Failed to reload configuration: {}", e),
                config: None,
            },
        }
    }

    pub async fn get_rate_limit_override(&self) -> Option<(u32, u32)> {
        let guard = self.rate_limiter.read().await;
        *guard
    }

    pub async fn set_algorithm(&self, req: SetAlgorithmRequest) -> SetAlgorithmResponse {
        if let Err(e) = req.validate() {
            return SetAlgorithmResponse {
                success: false,
                biz_tag: req.biz_tag.clone(),
                algorithm: req.algorithm.clone(),
                message: format!("Validation error: {}", e),
            };
        }

        let algorithm_type = match req.algorithm.to_lowercase().as_str() {
            "segment" => AlgorithmType::Segment,
            "snowflake" => AlgorithmType::Snowflake,
            "uuid_v7" => AlgorithmType::UuidV7,
            _ => {
                return SetAlgorithmResponse {
                    success: false,
                    biz_tag: req.biz_tag.clone(),
                    algorithm: req.algorithm.clone(),
                    message: format!(
                        "Invalid algorithm '{}'. Valid options: segment, snowflake, uuid_v7",
                        req.algorithm
                    ),
                };
            }
        };

        let biz_tag = req.biz_tag.clone();
        let algorithm = req.algorithm.clone();
        self.algorithm_router
            .set_algorithm(biz_tag.clone(), algorithm_type);

        let message = format!(
            "Algorithm for biz_tag '{}' set to '{}' successfully",
            biz_tag, algorithm
        );

        SetAlgorithmResponse {
            success: true,
            biz_tag,
            algorithm,
            message,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nebula_core::config::Config;

    fn create_test_algorithm_router() -> Arc<nebula_core::algorithm::AlgorithmRouter> {
        let config = Config::default();
        Arc::new(nebula_core::algorithm::AlgorithmRouter::new(
            config.clone(),
            None,
        ))
    }

    #[tokio::test]
    async fn test_config_to_response() {
        let config = Config::default();
        let response = ConfigManagementService::config_to_response(&config);

        assert_eq!(response.app.name, "nebula-id");
        assert_eq!(response.database.engine, "postgresql");
        assert_eq!(response.algorithm.default, "segment");
        assert_eq!(response.rate_limit.default_rps, 10000);
    }

    #[tokio::test]
    async fn test_update_rate_limit_request_validation() {
        let hot_config = Arc::new(HotReloadConfig::new(
            Config::default(),
            "config.toml".to_string(),
        ));
        let algorithm_router = create_test_algorithm_router();
        let _service = ConfigManagementService::new(hot_config, algorithm_router);

        let valid_req = UpdateRateLimitRequest {
            default_rps: Some(5000),
            burst_size: Some(50),
        };
        assert!(valid_req.validate().is_ok());

        let invalid_req = UpdateRateLimitRequest {
            default_rps: Some(0),
            burst_size: Some(0),
        };
        assert!(invalid_req.validate().is_err());
    }

    #[tokio::test]
    async fn test_update_logging_request_validation() {
        let hot_config = Arc::new(HotReloadConfig::new(
            Config::default(),
            "config.toml".to_string(),
        ));
        let algorithm_router = create_test_algorithm_router();
        let _service = ConfigManagementService::new(hot_config, algorithm_router);

        let valid_req = UpdateLoggingRequest {
            level: Some("debug".to_string()),
        };
        assert!(valid_req.validate().is_ok());

        let empty_req = UpdateLoggingRequest { level: None };
        assert!(empty_req.validate().is_ok());

        let invalid_req = UpdateLoggingRequest {
            level: Some("invalid_level_that_is_too_long".to_string()),
        };
        assert!(invalid_req.validate().is_err());
    }

    #[tokio::test]
    async fn test_set_algorithm() {
        let hot_config = Arc::new(HotReloadConfig::new(
            Config::default(),
            "config.toml".to_string(),
        ));
        let algorithm_router = create_test_algorithm_router();
        let service = ConfigManagementService::new(hot_config, algorithm_router.clone());

        let req = SetAlgorithmRequest {
            biz_tag: "test-biz".to_string(),
            algorithm: "snowflake".to_string(),
        };

        let response = service.set_algorithm(req).await;
        assert!(response.success);
        assert_eq!(response.biz_tag, "test-biz");
        assert_eq!(response.algorithm, "snowflake");
    }

    #[tokio::test]
    async fn test_set_algorithm_invalid() {
        let hot_config = Arc::new(HotReloadConfig::new(
            Config::default(),
            "config.toml".to_string(),
        ));
        let algorithm_router = create_test_algorithm_router();
        let service = ConfigManagementService::new(hot_config, algorithm_router);

        let req = SetAlgorithmRequest {
            biz_tag: "test-biz".to_string(),
            algorithm: "invalid_algorithm".to_string(),
        };

        let response = service.set_algorithm(req).await;
        assert!(!response.success);
        assert!(response.message.contains("Invalid algorithm"));
    }
}
