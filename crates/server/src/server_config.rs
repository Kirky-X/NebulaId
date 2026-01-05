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

#![allow(dead_code)]

use crate::config_hot_reload::HotReloadConfig;
use crate::models::{
    AlgorithmConfigInfo, AppConfigInfo, ConfigResponse, DatabaseConfigInfo, LoggingConfigInfo,
    MonitoringConfigInfo, RateLimitConfigInfo, RedisConfigInfo, SegmentConfigInfo,
    SnowflakeConfigInfo, TlsConfigInfo, UpdateConfigResponse, UpdateLoggingRequest,
    UpdateRateLimitRequest, UuidV7ConfigInfo,
};
use nebula_core::config::Config;
use nebula_core::types::Result;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;
use validator::Validate;

pub struct ConfigManagementService {
    admin_service: Arc<AdminConfigService>,
}

impl ConfigManagementService {
    pub fn new(hot_config: Arc<HotReloadConfig>) -> Self {
        Self {
            admin_service: Arc::new(AdminConfigService::new(hot_config)),
        }
    }

    pub fn get_config(&self) -> ConfigResponse {
        self.admin_service.get_config()
    }

    pub async fn update_rate_limit(&self, req: UpdateRateLimitRequest) -> UpdateConfigResponse {
        self.admin_service.update_rate_limit(req).await
    }

    pub async fn update_logging(&self, req: UpdateLoggingRequest) -> UpdateConfigResponse {
        self.admin_service.update_logging(req).await
    }

    pub async fn reload_config(&self) -> UpdateConfigResponse {
        self.admin_service.reload_config().await
    }

    pub async fn get_workspace(&self, _id: Uuid) -> Result<Option<()>> {
        Ok(None)
    }

    #[allow(dead_code)]
    pub async fn get_workspace_by_name(&self, _name: &str) -> Result<Option<()>> {
        Ok(None)
    }

    #[allow(dead_code)]
    pub async fn get_group_by_name(&self, _workspace_id: Uuid, _name: &str) -> Result<Option<()>> {
        Ok(None)
    }

    #[allow(dead_code)]
    pub async fn get_biz_tag(
        &self,
        _workspace_id: Uuid,
        _group_id: Uuid,
        _name: &str,
    ) -> Result<Option<()>> {
        Ok(None)
    }
}

pub struct AdminConfigService {
    hot_config: Arc<HotReloadConfig>,
    rate_limiter: Arc<RwLock<Option<(u32, u32)>>>,
}

impl AdminConfigService {
    pub fn new(hot_config: Arc<HotReloadConfig>) -> Self {
        Self {
            hot_config,
            rate_limiter: Arc::new(RwLock::new(None)),
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
                engine: config.database.engine.to_string(),
                host: Some(config.database.host.clone()),
                port: Some(config.database.port),
                database: Some(config.database.database.clone()),
                max_connections: config.database.max_connections,
                min_connections: config.database.min_connections,
            },
            redis: RedisConfigInfo {
                url: Some(config.redis.url.clone()),
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
                level: config.logging.level.to_string(),
                format: config.logging.format.to_string(),
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
            config.logging.level = nebula_core::config::LogLevel::from(level);
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use nebula_core::config::Config;

    #[tokio::test]
    async fn test_config_to_response() {
        let config = Config::default();
        let response = AdminConfigService::config_to_response(&config);

        assert_eq!(response.app.name, "nebula-id");
        assert_eq!(response.database.engine, "postgresql");
        assert_eq!(response.algorithm.default, "segment");
        assert_eq!(response.rate_limit.default_rps, 10000);
    }

    #[tokio::test]
    async fn test_update_rate_limit_request_validation() {
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
}
