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

#![cfg(test)]

//! gRPC 服务与监控模块端到端测试
//!
//! 覆盖《功能场景穷举分析》以下章节的端到端场景：
//! - 第 3.5 节（gRPC 服务）：Generate/BatchGenerate/Parse/HealthCheck
//! - 第 2.6 节（监控）：QPS 滑动窗口、告警评估、告警通知
//! - 第 3.6 节（热重载）：配置文件变更检测与重载失败处理
//!
//! 测试策略：
//! - gRPC 测试直接调用 NebulaIdService trait 方法（无需启动真实 tonic server）
//! - 监控测试直接调用 DefaultEvaluator / AlertManager 公共 API
//! - 热重载测试使用 tempfile 创建临时配置文件

use std::collections::HashMap;
use std::sync::Arc;

use sdforge::tonic::Request;

use crate::core::algorithm::AlgorithmRouter;
use crate::core::config::Config;
use crate::core::monitoring::core::{
    AlertEvaluator, AlertManager, AlertNotificationSender, AlertRule, AlertSeverity, AlertStatus,
    AlertingConfig, DefaultEvaluator,
};
use crate::core::types::metrics::QpsWindow;
use crate::core::types::GlobalMetrics;
use crate::server::config::management::{ConfigManagementService, ConfigManager};
use crate::server::config::HotReloadConfig;
use crate::server::grpc::GrpcServer;
use crate::server::handlers::mock_generator::MockIdGenerator;
use crate::server::handlers::ApiHandlers;
use crate::server::proto::nebula::id::v1;
use crate::server::proto::nebula::id::v1::nebula_id_service_server::NebulaIdService;
use crate::server::proto::nebula::id::v1::{
    BatchGenerateRequest as GrpcBatchGenerateRequest, GenerateRequest as GrpcGenerateRequest,
    HealthCheckRequest, ParseRequest as GrpcParseRequest,
};

// =============================================================================
// 测试辅助：构造 GrpcServer
// =============================================================================

/// 构造一个连接到 MockIdGenerator + ConfigManager 的 GrpcServer。
/// 复用 grpc.rs 内 create_test_grpc_server 的构造模式。
fn create_test_grpc_server() -> GrpcServer {
    let config = Config::default();
    let hot_config = Arc::new(HotReloadConfig::new(
        config.clone(),
        "config/config.toml".to_string(),
    ));
    let algorithm_router = Arc::new(AlgorithmRouter::new(config, None));
    let config_service: Arc<dyn ConfigManagementService> =
        Arc::new(ConfigManager::new(hot_config, algorithm_router));
    let id_generator: Arc<dyn crate::core::algorithm::IdGenerator> =
        Arc::new(MockIdGenerator::new());
    let handlers = Arc::new(ApiHandlers::new(id_generator, config_service));
    GrpcServer::new(handlers)
}

/// 设置测试环境变量（Config 解析 TOML 时需要 NEBULA_DATABASE_PASSWORD）
fn setup_test_env() {
    std::env::set_var("NEBULA_DATABASE_PASSWORD", "test_password");
}

/// 写入完整的有效 TOML 配置文件（参考 hot_reload.rs 内 write_test_config_file）
fn write_test_config_file(
    path: &std::path::Path,
    app_name: &str,
    http_port: u16,
    default_rps: u32,
    burst_size: u32,
    log_level: &str,
) {
    let content = format!(
        r#"[app]
name = "{app_name}"
host = "127.0.0.1"
http_port = {http_port}
grpc_port = 50051
dc_id = 1
worker_id = 1

[database]
engine = "postgresql"
url = "postgresql://idgen:${{NEBULA_DATABASE_PASSWORD}}@localhost:5432/idgen"
host = "localhost"
port = 5432
username = "idgen"
password = "${{NEBULA_DATABASE_PASSWORD}}"
database = "idgen"
max_connections = 10
min_connections = 1
acquire_timeout_seconds = 5
idle_timeout_seconds = 300

[etcd]
endpoints = ["http://localhost:2379"]
connect_timeout_ms = 5000
watch_timeout_ms = 5000

[auth]
enabled = true
cache_ttl_seconds = 300
api_keys = []

[algorithm]
default = "segment"

[algorithm.segment]
base_step = 1000
min_step = 500
max_step = 100000
switch_threshold = 0.1

[algorithm.snowflake]
datacenter_id_bits = 3
worker_id_bits = 8
sequence_bits = 10
clock_drift_threshold_ms = 1000

[algorithm.uuid_v8]
enabled = true

[monitoring]
metrics_enabled = true
metrics_path = "/metrics"
tracing_enabled = true
otlp_endpoint = ""

[logging]
level = "{log_level}"
format = "json"
include_location = true

[rate_limit]
enabled = true
default_rps = {default_rps}
burst_size = {burst_size}

[tls]
enabled = false
cert_path = ""
key_path = ""
http_enabled = false
grpc_enabled = false
min_tls_version = "tls13"
alpn_protocols = ["h2", "http/1.1"]

[batch_generate]
max_batch_size = 100
"#
    );
    std::fs::write(path, content).unwrap();
}

// =============================================================================
// E2E-GRPC 组：gRPC 服务端到端测试
// =============================================================================

#[tokio::test]
async fn e2e_grpc_generate_returns_valid_id() {
    // Generate 应返回非空 ID，algorithm 为 "segment"
    let server = create_test_grpc_server();
    let req = Request::new(GrpcGenerateRequest {
        namespace: "test-ns".to_string(),
        tag: "test-tag".to_string(),
        metadata: HashMap::new(),
    });
    let resp = server.generate(req).await.expect("generate 应成功");
    let inner = resp.into_inner();
    assert!(!inner.id.is_empty(), "返回的 ID 不应为空");
    assert_eq!(inner.algorithm, "segment");
}

#[tokio::test]
async fn e2e_grpc_batch_generate_validates_count_1_to_100() {
    // count=1（下界）和 count=100（上界）都应成功
    let server = create_test_grpc_server();

    // 下界：count=1
    let req = Request::new(GrpcBatchGenerateRequest {
        namespace: "test-ns".to_string(),
        tag: "test-tag".to_string(),
        count: 1,
        metadata: HashMap::new(),
    });
    let resp = server.batch_generate(req).await.expect("count=1 应成功");
    assert_eq!(resp.into_inner().ids.len(), 1);

    // 上界：count=100
    let req = Request::new(GrpcBatchGenerateRequest {
        namespace: "test-ns".to_string(),
        tag: "test-tag".to_string(),
        count: 100,
        metadata: HashMap::new(),
    });
    let resp = server.batch_generate(req).await.expect("count=100 应成功");
    assert_eq!(resp.into_inner().ids.len(), 100);
}

#[tokio::test]
async fn e2e_grpc_batch_generate_count_zero_returns_invalid_argument() {
    // count=0 应返回 InvalidArgument
    let server = create_test_grpc_server();
    let req = Request::new(GrpcBatchGenerateRequest {
        namespace: "test-ns".to_string(),
        tag: "test-tag".to_string(),
        count: 0,
        metadata: HashMap::new(),
    });
    let err = server
        .batch_generate(req)
        .await
        .expect_err("count=0 应返回错误");
    assert_eq!(err.code(), sdforge::tonic::Code::InvalidArgument);
    assert!(err.message().contains("zero"));
}

#[tokio::test]
async fn e2e_grpc_batch_generate_count_over_100_returns_invalid_argument() {
    // count=101 应返回 InvalidArgument
    let server = create_test_grpc_server();
    let req = Request::new(GrpcBatchGenerateRequest {
        namespace: "test-ns".to_string(),
        tag: "test-tag".to_string(),
        count: 101,
        metadata: HashMap::new(),
    });
    let err = server
        .batch_generate(req)
        .await
        .expect_err("count=101 应返回错误");
    assert_eq!(err.code(), sdforge::tonic::Code::InvalidArgument);
    assert!(err.message().contains("exceeds maximum"));
}

#[tokio::test]
async fn e2e_grpc_parse_returns_metadata() {
    // Parse 数字 ID 应返回包含 timestamp/algorithm 等字段的元数据
    let server = create_test_grpc_server();
    let req = Request::new(GrpcParseRequest {
        id: "12345".to_string(),
    });
    let resp = server.parse(req).await.expect("parse 应成功");
    let inner = resp.into_inner();
    assert_eq!(inner.id, "12345");
    assert!(inner.metadata.contains_key("timestamp"));
    assert!(inner.metadata.contains_key("algorithm"));
    assert!(inner.metadata.contains_key("worker_id"));
    assert!(inner.metadata.contains_key("sequence"));
}

#[tokio::test]
async fn e2e_grpc_health_check_returns_serving() {
    // HealthCheck 应返回 Serving 状态
    let server = create_test_grpc_server();
    let req = Request::new(HealthCheckRequest {
        service: String::new(),
    });
    let resp = server.health_check(req).await.expect("health_check 应成功");
    let inner = resp.into_inner();
    assert_eq!(
        inner.status,
        v1::health_check_response::ServingStatus::Serving as i32
    );
}

// =============================================================================
// E2E-QPS 组：QPS 滑动窗口端到端测试
// =============================================================================

#[test]
fn e2e_qps_window_initial_zero() {
    // 新建的 QpsWindow 初始 QPS 应为 0
    let window = QpsWindow::new(10);
    assert_eq!(window.get_qps(), 0, "初始 QPS 应为 0");
    assert_eq!(window.window_size(), 10);
}

#[test]
fn e2e_qps_window_records_requests() {
    // 记录请求后 QPS 应增加（不为 0）
    let window = QpsWindow::new(10);
    // 记录 10 次请求：current=10, last=0
    // qps = (10*7 + 0*3) / 10 = 7
    for _ in 0..10 {
        window.record();
    }
    let qps = window.get_qps();
    assert_eq!(qps, 7, "记录 10 次后 QPS 应为 7（70% 权重）");
    assert!(qps > 0, "记录请求后 QPS 应大于 0");
}

#[tokio::test]
async fn e2e_qps_window_weighted_average() {
    // 验证 70/30 加权平均：current 秒权重 70%，上一秒权重 30%
    let window = QpsWindow::new(10);

    // 第一秒：记录 10 次请求
    for _ in 0..10 {
        window.record();
    }
    // current=10, last=0 → qps = (10*7 + 0*3) / 10 = 7
    let qps_first = window.get_qps();
    assert_eq!(qps_first, 7, "第一秒：current=10, last=0 → 7");

    // 等待 1 秒让秒切换自然发生
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    // 新的一秒：记录 1 次请求，触发秒切换
    // 秒切换后：last=10（之前的 current），current=1
    // qps = (1*7 + 10*3) / 10 = 37/10 = 3
    window.record();
    let qps_second = window.get_qps();
    assert_eq!(
        qps_second, 3,
        "第二秒：current=1, last=10 → (1*7 + 10*3)/10 = 3，验证 70/30 加权"
    );

    // 验证加权平均公式：(current*7 + last*3) / 10
    // 此时 current=1, last=10，加权平均 = 0.7 + 3.0 = 3.7 → 整数除法 = 3
}

// =============================================================================
// E2E-ALERT 组：告警端到端测试
// =============================================================================

#[test]
fn e2e_alert_evaluator_matching_expression_fires() {
    // 匹配阈值的表达式应触发告警
    let evaluator = DefaultEvaluator;
    let metrics = GlobalMetrics::new();
    // 设置 total_errors > 0 使 id_generation_failed 规则触发
    metrics.increment_errors();
    metrics.increment_errors();

    let rule = AlertRule::new("gen_fail", "id_generation_failed", AlertSeverity::Critical);
    let (firing, value) = evaluator.evaluate(&rule, &metrics);
    assert!(firing, "total_errors > 0 时应触发告警");
    assert_eq!(value.as_deref(), Some("2"), "current_value 应为错误数");
}

#[test]
fn e2e_alert_evaluator_non_matching_expression_does_not_fire() {
    // 不匹配阈值的表达式不应触发告警
    let evaluator = DefaultEvaluator;
    let metrics = GlobalMetrics::new();
    // total_errors = 0，id_generation_failed 不应触发

    let rule = AlertRule::new("gen_fail", "id_generation_failed", AlertSeverity::Critical);
    let (firing, value) = evaluator.evaluate(&rule, &metrics);
    assert!(!firing, "total_errors = 0 时不应触发告警");
    assert_eq!(value.as_deref(), Some("0"));
}

#[tokio::test]
async fn e2e_alert_manager_records_state_transitions() {
    // 验证 AlertManager 状态管理 API：
    // 1. 初始状态：所有规则状态为 Pending，history 为空
    // 2. add_rule 增加状态条目
    // 3. remove_rule 移除状态条目
    // 4. update_config 更新配置后仍可正常查询
    //
    // 注意：完整的 Pending→Firing→Resolved 状态转换需要调用私有的
    // evaluate_rule 方法（在 core.rs 的 #[cfg(test)] mod tests 内
    // 已有 test_evaluate_rule_fires_when_condition_met_and_for_duration_zero
    // 等单元测试覆盖）。e2e 层仅验证公共 API 的状态管理行为。

    let metrics = Arc::new(GlobalMetrics::new());
    let sender = Arc::new(AlertNotificationSender::new(vec![]));

    let config = AlertingConfig {
        enabled: true,
        evaluation_interval_ms: 1000,
        rules: vec![AlertRule::new(
            "rule_a",
            "id_generation_failed",
            AlertSeverity::Warning,
        )],
        channels: vec![],
        global_labels: HashMap::new(),
    };

    let (mut manager, _rx) = AlertManager::new(config, metrics, sender);

    // 1. 初始状态：rule_a 状态为 Pending，history 为空
    let state = manager.get_state("rule_a").expect("rule_a 状态应存在");
    assert_eq!(state.current_status, AlertStatus::Pending);
    assert_eq!(manager.get_alert_count(), 0);
    assert!(manager.get_alerts().is_empty());
    assert!(manager.get_firing_alerts().is_empty());

    // 2. add_rule：增加新规则，状态条目增加
    manager.add_rule(AlertRule::new(
        "rule_b",
        "segment_exhausted",
        AlertSeverity::Critical,
    ));
    assert_eq!(manager.get_all_states().len(), 2);
    let state_b = manager.get_state("rule_b").expect("rule_b 状态应存在");
    assert_eq!(state_b.current_status, AlertStatus::Pending);

    // 3. remove_rule：移除规则，状态条目减少
    manager.remove_rule("rule_a");
    assert_eq!(manager.get_all_states().len(), 1);
    assert!(manager.get_state("rule_a").is_none());

    // 4. update_config：更新配置后仍可正常查询
    let new_config = AlertingConfig {
        enabled: true,
        evaluation_interval_ms: 500,
        rules: vec![AlertRule::new(
            "rule_c",
            "id_generation_failed",
            AlertSeverity::Critical,
        )],
        channels: vec![],
        global_labels: HashMap::new(),
    };
    manager.update_config(new_config);

    // 验证查询 API 正常工作
    assert_eq!(manager.get_alerts().len(), 0);
    assert_eq!(manager.get_firing_alerts().len(), 0);
    assert_eq!(
        manager
            .get_alerts_by_severity(AlertSeverity::Critical)
            .len(),
        0
    );

    manager.shutdown();
}

// =============================================================================
// E2E-RELOAD 组：热重载端到端测试
// =============================================================================

#[tokio::test]
async fn e2e_hot_reload_config_detects_file_change() {
    // 文件变更应被检测：写入初始配置 → 修改文件 → reload_from_file → 验证新配置生效
    setup_test_env();
    let temp_dir = tempfile::TempDir::new().unwrap();
    let config_path = temp_dir.path().join("hot_reload_e2e.toml");

    // 写入初始配置（app.name = "initial"）
    write_test_config_file(&config_path, "initial", 8080, 10000, 100, "info");

    let hot_config = HotReloadConfig::new(
        Config::load_from_file(config_path.to_str().unwrap()).unwrap(),
        config_path.to_str().unwrap().to_string(),
    );

    // 验证初始配置
    assert_eq!(hot_config.get_config().app.name, "initial");

    // 注册回调，验证回调被触发
    let callback_triggered = Arc::new(std::sync::Mutex::new(false));
    let callback_triggered_clone = callback_triggered.clone();
    hot_config.add_reload_callback(move |config| {
        assert_eq!(config.app.name, "updated");
        *callback_triggered_clone.lock().unwrap() = true;
    });

    // 修改文件（app.name = "updated"）
    write_test_config_file(&config_path, "updated", 9090, 5000, 50, "debug");

    // 触发重载
    let result = hot_config.reload_from_file().await;
    assert!(result.is_ok(), "reload_from_file 应返回 Ok");
    assert!(result.unwrap(), "reload_from_file 应返回 true（成功）");

    // 验证配置已更新
    let config = hot_config.get_config();
    assert_eq!(config.app.name, "updated");
    assert_eq!(config.app.http_port, 9090);
    assert_eq!(config.rate_limit.default_rps, 5000);
    assert_eq!(config.rate_limit.burst_size, 50);

    // 验证回调被触发
    assert!(*callback_triggered.lock().unwrap(), "重载回调应被触发");
}

#[tokio::test]
async fn e2e_hot_reload_config_reload_failure_handled() {
    // 重载失败处理：malformed TOML 应返回 Ok(false)，内存配置不变
    setup_test_env();
    let temp_dir = tempfile::TempDir::new().unwrap();
    let config_path = temp_dir.path().join("bad_config_e2e.toml");

    // 写入初始有效配置
    write_test_config_file(&config_path, "before-failure", 8080, 10000, 100, "info");

    let hot_config = HotReloadConfig::new(
        Config::load_from_file(config_path.to_str().unwrap()).unwrap(),
        config_path.to_str().unwrap().to_string(),
    );

    // 验证初始配置
    assert_eq!(hot_config.get_config().app.name, "before-failure");

    // 写入 malformed TOML
    std::fs::write(&config_path, "this is not valid toml = = =\n[[[").unwrap();

    // 尝试重载 — 应返回 Ok(false)，不应 panic
    let result = hot_config.reload_from_file().await;
    assert!(
        result.is_ok(),
        "reload_from_file 不应在 TOML 解析失败时返回 Err"
    );
    assert!(
        !result.unwrap(),
        "reload_from_file 应在 TOML 解析失败时返回 false"
    );

    // 验证内存中的配置未改变（仍是初始有效配置）
    assert_eq!(
        hot_config.get_config().app.name,
        "before-failure",
        "重载失败后内存配置不应改变"
    );
}

// =============================================================================
// wiring T006: gRPC 认证 —— 全 RPC（含双向流）凭证校验
// =============================================================================

mod grpc_auth {
    use super::*;
    use crate::core::database::{
        ApiKeyInfo, ApiKeyRepository, ApiKeyRole, ApiKeyWithSecret, CreateApiKeyRequest,
    };
    use crate::core::types::CoreError;
    use crate::server::middleware::api_key_auth::ApiKeyAuth;
    use sdforge::tonic::{Code, Status};
    use uuid::Uuid;

    /// 固定凭证：grpc-key / grpc-secret → workspace ws-grpc + User 角色
    #[derive(Clone)]
    struct FixedKeyRepo;

    #[async_trait::async_trait]
    impl ApiKeyRepository for FixedKeyRepo {
        async fn create_api_key(
            &self,
            _request: &CreateApiKeyRequest,
        ) -> crate::core::types::Result<ApiKeyWithSecret> {
            Err(crate::core::types::CoreError::NotFound("noop".into()))
        }
        async fn get_api_key_by_id(
            &self,
            _key_id: &str,
        ) -> crate::core::types::Result<Option<ApiKeyInfo>> {
            Ok(None)
        }
        async fn validate_api_key(
            &self,
            key_id: &str,
            _key_secret: &str,
        ) -> crate::core::types::Result<Option<(Option<Uuid>, ApiKeyRole)>> {
            if key_id == "grpc-key" {
                Ok(Some((Some(Uuid::new_v4()), ApiKeyRole::User)))
            } else {
                Ok(None)
            }
        }
        async fn list_api_keys(
            &self,
            _workspace_id: Uuid,
            _limit: Option<u32>,
            _offset: Option<u32>,
        ) -> crate::core::types::Result<Vec<ApiKeyInfo>> {
            Ok(vec![])
        }
        async fn delete_api_key(&self, _id: Uuid) -> crate::core::types::Result<()> {
            Ok(())
        }
        async fn revoke_api_key(&self, _id: Uuid) -> crate::core::types::Result<()> {
            Ok(())
        }
        async fn update_last_used(&self, _id: Uuid) -> crate::core::types::Result<()> {
            Ok(())
        }
        async fn get_admin_api_key(
            &self,
            _workspace_id: Uuid,
        ) -> crate::core::types::Result<Option<ApiKeyInfo>> {
            Ok(None)
        }
        async fn count_api_keys(&self, _workspace_id: Uuid) -> crate::core::types::Result<u64> {
            Ok(0)
        }
        async fn rotate_api_key(
            &self,
            _key_id: &str,
            _grace_period_seconds: u64,
        ) -> crate::core::types::Result<ApiKeyWithSecret> {
            Err(crate::core::types::CoreError::NotFound("noop".into()))
        }
        async fn get_keys_older_than(
            &self,
            _age_threshold_days: i64,
        ) -> crate::core::types::Result<Vec<ApiKeyInfo>> {
            Ok(vec![])
        }
    }

    fn auth_server(auth: ApiKeyAuth) -> GrpcServer {
        let config = Config::default();
        let hot_config = Arc::new(HotReloadConfig::new(
            config.clone(),
            "config/config.toml".to_string(),
        ));
        let algorithm_router = Arc::new(AlgorithmRouter::new(config, None));
        let config_service: Arc<dyn ConfigManagementService> =
            Arc::new(ConfigManager::new(hot_config, algorithm_router));
        let id_generator: Arc<dyn crate::core::algorithm::IdGenerator> =
            Arc::new(MockIdGenerator::new());
        let handlers = Arc::new(ApiHandlers::new(id_generator, config_service));
        GrpcServer::with_auth(handlers, Arc::new(auth))
    }

    fn basic_header(key: &str, secret: &str) -> String {
        use base64::Engine;
        format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(format!("{key}:{secret}"))
        )
    }

    /// `ApiKey key_id:key_secret` 格式头 —— 与 Basic 并列的第二种受支持格式
    fn api_key_header(key: &str, secret: &str) -> String {
        format!("ApiKey {key}:{secret}")
    }

    /// 携带指定 `ApiKey` 格式凭证的 GenerateRequest
    fn api_key_request(key_id: &str, key_secret: &str) -> Request<GrpcGenerateRequest> {
        let mut req = Request::new(GrpcGenerateRequest {
            namespace: "ns".to_string(),
            tag: "tag".to_string(),
            metadata: HashMap::new(),
        });
        req.metadata_mut().insert(
            "authorization",
            api_key_header(key_id, key_secret).parse().unwrap(),
        );
        req
    }

    /// 在真实 tonic 传输上起一个启用认证的 gRPC server，返回监听地址与任务句柄。
    /// 流式入口的 `Streaming<T>` 无法在测试侧凭空构造，只能走真实传输。
    async fn spawn_auth_server() -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
        use sdforge::tonic::transport::Server;
        use v1::nebula_id_service_server::NebulaIdServiceServer;

        let server_impl = auth_server(ApiKeyAuth::new(Arc::new(FixedKeyRepo), true));

        // 占位端口：绑定 :0 取可用端口后释放（测试场景可接受的微小竞态）
        let probe = std::net::TcpListener::bind("127.0.0.1:0").expect("bind probe");
        let addr = probe.local_addr().expect("probe addr");
        drop(probe);

        let jh = tokio::spawn(async move {
            let _ = Server::builder()
                .add_service(NebulaIdServiceServer::new(server_impl))
                .serve(addr)
                .await;
        });
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        (addr, jh)
    }

    #[tokio::test]
    async fn rejects_missing_authorization_metadata() {
        let server = auth_server(ApiKeyAuth::new(Arc::new(FixedKeyRepo), true));
        let req = Request::new(GrpcGenerateRequest {
            namespace: "ns".to_string(),
            tag: "tag".to_string(),
            metadata: HashMap::new(),
        });
        let err: Status = NebulaIdService::generate(&server, req)
            .await
            .expect_err("无凭证必须被拒绝");
        assert_eq!(err.code(), Code::Unauthenticated);
    }

    #[tokio::test]
    async fn rejects_invalid_credentials() {
        let server = auth_server(ApiKeyAuth::new(Arc::new(FixedKeyRepo), true));
        let mut req = Request::new(GrpcGenerateRequest {
            namespace: "ns".to_string(),
            tag: "tag".to_string(),
            metadata: HashMap::new(),
        });
        req.metadata_mut().insert(
            "authorization",
            basic_header("wrong", "creds").parse().unwrap(),
        );
        let err = NebulaIdService::generate(&server, req)
            .await
            .expect_err("无效凭证必须被拒绝");
        assert_eq!(err.code(), Code::Unauthenticated);
    }

    // ===== T023：R-auth-003 失败判因（禁用/过期 vs 凭证无效）=====

    /// 按 key 状态建模的仓储。
    ///
    /// `validate_api_key` 复刻真实仓储 `repository.rs:1018-1026` 的语义：
    /// 禁用或已过期的 key 一律返回 `None`（不区分原因）。正因如此 gRPC 侧
    /// 必须靠 `get_api_key_by_id` 回查 key 行才能判因 —— 本 mock 提供该回查
    /// 数据，使「禁用/过期 → permission_denied」与「凭证无效 →
    /// unauthenticated」两条路径可分别验证。
    struct StatefulKeyRepo;

    /// 构造 key 行（`ApiKeyInfo = ApiKey`，字段见 `api_key_entity.rs`）
    fn key_row(
        key_id: &str,
        enabled: bool,
        expires_at: Option<chrono::NaiveDateTime>,
    ) -> ApiKeyInfo {
        ApiKeyInfo {
            id: Uuid::new_v4(),
            key_id: key_id.to_string(),
            key_prefix: "nino_".to_string(),
            role: ApiKeyRole::User,
            workspace_id: Some(Uuid::new_v4()),
            name: key_id.to_string(),
            description: None,
            rate_limit: 1000,
            enabled,
            expires_at,
            last_used_at: None,
            created_at: chrono::Utc::now().naive_utc(),
        }
    }

    /// 昨天 —— 已过期；明天 —— 未过期
    fn yesterday() -> chrono::NaiveDateTime {
        chrono::Utc::now().naive_utc() - chrono::Duration::days(1)
    }

    fn tomorrow() -> chrono::NaiveDateTime {
        chrono::Utc::now().naive_utc() + chrono::Duration::days(1)
    }

    #[async_trait::async_trait]
    impl ApiKeyRepository for StatefulKeyRepo {
        async fn create_api_key(
            &self,
            _request: &CreateApiKeyRequest,
        ) -> crate::core::types::Result<ApiKeyWithSecret> {
            Err(crate::core::types::CoreError::NotFound("noop".into()))
        }
        async fn get_api_key_by_id(
            &self,
            key_id: &str,
        ) -> crate::core::types::Result<Option<ApiKeyInfo>> {
            match key_id {
                "live-key" => Ok(Some(key_row(key_id, true, None))),
                "disabled-key" => Ok(Some(key_row(key_id, false, None))),
                "expired-key" => Ok(Some(key_row(key_id, true, Some(yesterday())))),
                // 有效期内的 key 但 secret 不符 —— 用于验证「凭证无效」不会被
                // 误判为 permission_denied
                "future-key" => Ok(Some(key_row(key_id, true, Some(tomorrow())))),
                _ => Ok(None),
            }
        }
        async fn validate_api_key(
            &self,
            key_id: &str,
            key_secret: &str,
        ) -> crate::core::types::Result<Option<(Option<Uuid>, ApiKeyRole)>> {
            let row = self.get_api_key_by_id(key_id).await?;
            let Some(row) = row else {
                return Ok(None);
            };
            // 与真实仓储一致：禁用 / 过期都直接 miss，不透露原因
            if !row.enabled {
                return Ok(None);
            }
            if let Some(expires_at) = row.expires_at {
                if expires_at < chrono::Utc::now().naive_utc() {
                    return Ok(None);
                }
            }
            if key_secret != "grpc-secret" {
                return Ok(None);
            }
            Ok(Some((row.workspace_id, row.role)))
        }
        async fn list_api_keys(
            &self,
            _workspace_id: Uuid,
            _limit: Option<u32>,
            _offset: Option<u32>,
        ) -> crate::core::types::Result<Vec<ApiKeyInfo>> {
            Ok(vec![])
        }
        async fn delete_api_key(&self, _id: Uuid) -> crate::core::types::Result<()> {
            Ok(())
        }
        async fn revoke_api_key(&self, _id: Uuid) -> crate::core::types::Result<()> {
            Ok(())
        }
        async fn update_last_used(&self, _id: Uuid) -> crate::core::types::Result<()> {
            Ok(())
        }
        async fn get_admin_api_key(
            &self,
            _workspace_id: Uuid,
        ) -> crate::core::types::Result<Option<ApiKeyInfo>> {
            Ok(None)
        }
        async fn count_api_keys(&self, _workspace_id: Uuid) -> crate::core::types::Result<u64> {
            Ok(0)
        }
        async fn rotate_api_key(
            &self,
            _key_id: &str,
            _grace_period_seconds: u64,
        ) -> crate::core::types::Result<ApiKeyWithSecret> {
            Err(crate::core::types::CoreError::NotFound("noop".into()))
        }
        async fn get_keys_older_than(
            &self,
            _age_threshold_days: i64,
        ) -> crate::core::types::Result<Vec<ApiKeyInfo>> {
            Ok(vec![])
        }
    }

    /// 用 `StatefulKeyRepo` 构造启用认证的 gRPC server
    fn stateful_server() -> GrpcServer {
        auth_server(ApiKeyAuth::new(Arc::new(StatefulKeyRepo), true))
    }

    /// 携带指定 Basic 凭证的 GenerateRequest
    fn generate_request_with(key_id: &str, key_secret: &str) -> Request<GrpcGenerateRequest> {
        let mut req = Request::new(GrpcGenerateRequest {
            namespace: "ns".to_string(),
            tag: "tag".to_string(),
            metadata: HashMap::new(),
        });
        req.metadata_mut().insert(
            "authorization",
            basic_header(key_id, key_secret).parse().unwrap(),
        );
        req
    }

    #[tokio::test]
    async fn disabled_key_returns_permission_denied() {
        // key 行存在但 enabled=false：身份可识别，属授权层拒绝
        let server = stateful_server();
        let err = NebulaIdService::generate(&server, generate_request_with("disabled-key", "x"))
            .await
            .expect_err("禁用 key 必须被拒绝");
        assert_eq!(
            err.code(),
            Code::PermissionDenied,
            "禁用 key 应是 permission_denied 而非 unauthenticated，实际: {:?}",
            err
        );
        assert_eq!(
            err.message(),
            CoreError::ApiKeyDisabled.to_string(),
            "permission_denied 必须携带 ApiKeyDisabled 语义文案"
        );
    }

    #[tokio::test]
    async fn expired_key_returns_permission_denied() {
        // key 行存在且 enabled，但 expires_at 已过
        let server = stateful_server();
        let err = NebulaIdService::generate(&server, generate_request_with("expired-key", "x"))
            .await
            .expect_err("过期 key 必须被拒绝");
        assert_eq!(
            err.code(),
            Code::PermissionDenied,
            "过期 key 应是 permission_denied，实际: {:?}",
            err
        );
        assert_eq!(
            err.message(),
            CoreError::ApiKeyExpired.to_string(),
            "permission_denied 必须携带 ApiKeyExpired 语义文案"
        );
    }

    #[tokio::test]
    async fn unknown_key_returns_unauthenticated() {
        // 判因回查返回 None（key 行不存在）→ 凭证本身无效，不得给 403
        let server = stateful_server();
        let err = NebulaIdService::generate(&server, generate_request_with("ghost-key", "x"))
            .await
            .expect_err("不存在的 key 必须被拒绝");
        assert_eq!(err.code(), Code::Unauthenticated);
        assert_eq!(err.message(), "invalid or unknown api key");
    }

    #[tokio::test]
    async fn wrong_secret_on_existing_valid_key_returns_unauthenticated() {
        // key 行存在、enabled、未过期，但 secret 不符 → 判因必须落回
        // unauthenticated（否则任何 secret 猜错都会变成 403）
        let server = stateful_server();
        let err = NebulaIdService::generate(&server, generate_request_with("live-key", "nope"))
            .await
            .expect_err("secret 不符必须被拒绝");
        assert_eq!(err.code(), Code::Unauthenticated);
        assert_eq!(err.message(), "invalid or unknown api key");
        // 边界：expires_at 有值但未到期 ⇒ 同样只是凭证不符，不得判成过期
        let err = NebulaIdService::generate(&server, generate_request_with("future-key", "nope"))
            .await
            .expect_err("未到期 key 的 secret 不符必须被拒绝");
        assert_eq!(
            err.code(),
            Code::Unauthenticated,
            "expires_at 未到期不应判为 permission_denied，实际: {:?}",
            err
        );
        // 对照组：同一 key 用正确 secret 应通过认证（证明差异只在凭证）
        let ok =
            NebulaIdService::generate(&server, generate_request_with("live-key", "grpc-secret"))
                .await
                .expect("活 key + 正确 secret 应通过认证");
        assert!(!ok.into_inner().id.is_empty());
    }

    #[tokio::test]
    async fn health_check_enforces_auth_and_passes_valid_key() {
        // T023 清理 health_check 死绑定的回归：认证副作用与错误传播都不能丢，
        // 且判因映射在 health_check 入口同样生效
        let server = stateful_server();
        let no_creds = Request::new(HealthCheckRequest {
            service: String::new(),
        });
        let err = NebulaIdService::health_check(&server, no_creds)
            .await
            .expect_err("health_check 无凭证必须被拒绝");
        assert_eq!(err.code(), Code::Unauthenticated);

        let mut disabled = Request::new(HealthCheckRequest {
            service: String::new(),
        });
        disabled.metadata_mut().insert(
            "authorization",
            basic_header("disabled-key", "x").parse().unwrap(),
        );
        let err = NebulaIdService::health_check(&server, disabled)
            .await
            .expect_err("health_check 禁用 key 必须被拒绝");
        assert_eq!(err.code(), Code::PermissionDenied);

        let mut live = Request::new(HealthCheckRequest {
            service: String::new(),
        });
        live.metadata_mut().insert(
            "authorization",
            basic_header("live-key", "grpc-secret").parse().unwrap(),
        );
        let resp = NebulaIdService::health_check(&server, live)
            .await
            .expect("合法凭证下 health_check 应正常返回");
        assert_eq!(
            resp.into_inner().status,
            v1::health_check_response::ServingStatus::Serving as i32
        );
    }

    #[tokio::test]
    async fn accepts_valid_credentials_and_injects_identity() {
        let server = auth_server(ApiKeyAuth::new(Arc::new(FixedKeyRepo), true));
        let mut req = Request::new(GrpcGenerateRequest {
            namespace: "ns".to_string(),
            tag: "tag".to_string(),
            metadata: HashMap::new(),
        });
        req.metadata_mut().insert(
            "authorization",
            basic_header("grpc-key", "grpc-secret").parse().unwrap(),
        );

        let authenticated: sdforge::tonic::Request<GrpcGenerateRequest> =
            server.authenticate(req).await.expect("合法凭证应通过认证");
        let identity = authenticated
            .extensions()
            .get::<Option<Uuid>>()
            .copied()
            .flatten();
        assert!(
            identity.is_some(),
            "workspace_id 必须注入 request extensions"
        );
        let role = authenticated.extensions().get::<ApiKeyRole>().unwrap();
        assert_eq!(*role, ApiKeyRole::User);
    }

    #[tokio::test]
    async fn bypasses_when_auth_disabled() {
        let server = auth_server(ApiKeyAuth::new(Arc::new(FixedKeyRepo), false));
        let req = Request::new(GrpcGenerateRequest {
            namespace: "ns".to_string(),
            tag: "tag".to_string(),
            metadata: HashMap::new(),
        });
        let resp = NebulaIdService::generate(&server, req)
            .await
            .expect("auth.enabled=false 应放行（对齐 HTTP Anonymous 语义）");
        assert_eq!(
            resp.extensions().get::<ApiKeyRole>(),
            None,
            "禁用认证时不注入角色"
        );
    }

    /// 流式 RPC 认证：起真实 tonic server（认证启用），客户端无凭证调用
    /// BatchGenerateStream —— 必须收到 Unauthenticated。
    /// （`Streaming<T>` 无法在测试侧凭空构造，必须走真实传输。）
    #[tokio::test]
    async fn stream_rpc_is_intercepted_over_real_transport() {
        let (addr, jh) = spawn_auth_server().await;

        let mut client =
            v1::nebula_id_service_client::NebulaIdServiceClient::connect(format!("http://{addr}"))
                .await
                .expect("client connect");

        let item = v1::BatchGenerateStreamRequest {
            namespace: "ns".to_string(),
            tag: "tag".to_string(),
            count: 1,
            metadata: HashMap::new(),
        };
        let result = client.batch_generate_stream(tokio_stream::once(item)).await;
        match result {
            Err(status) => assert_eq!(status.code(), Code::Unauthenticated),
            Ok(resp) => {
                let mut stream = resp.into_inner();
                let first = tokio_stream::StreamExt::next(&mut stream).await;
                match first {
                    Some(Err(status)) => {
                        assert_eq!(status.code(), Code::Unauthenticated)
                    }
                    other => panic!("流式 RPC 无凭证不应产出正常项: {other:?}"),
                }
            }
        }

        jh.abort();
    }

    // ===== 第 3 轮：ApiKey 格式 / 禁用放行 / 5 RPC 全覆盖的证据缺口 =====

    #[tokio::test]
    async fn api_key_format_credentials_are_accepted() {
        // gRPC 侧此前只走 Basic：ApiKey 分支一旦断裂只有 HTTP 侧能发现。
        let server = stateful_server();
        let resp = NebulaIdService::generate(&server, api_key_request("live-key", "grpc-secret"))
            .await
            .expect("ApiKey 格式 + 正确 secret 应通过认证");
        assert!(!resp.into_inner().id.is_empty(), "认证通过后应正常返回 ID");
    }

    #[tokio::test]
    async fn api_key_format_wrong_secret_returns_unauthenticated() {
        // 格式正确不代表凭证正确：secret 不符必须仍落 unauthenticated，
        // 不得因为走了另一条解析分支而变成 permission_denied 或直接放行
        let server = stateful_server();
        let err = NebulaIdService::generate(&server, api_key_request("live-key", "nope"))
            .await
            .expect_err("ApiKey 格式 secret 不符必须被拒绝");
        assert_eq!(
            err.code(),
            Code::Unauthenticated,
            "ApiKey 格式 secret 不符应是 unauthenticated，实际: {err:?}"
        );
        assert_eq!(err.message(), "invalid or unknown api key");
    }

    #[tokio::test]
    async fn disabled_auth_passes_with_and_without_credentials() {
        // `auth.enabled=false` 的放行必须与凭证内容无关，否则「禁用认证」
        // 形同虚设（无效凭证反被拒）。同时不得注入真实角色 —— 对齐 HTTP 侧
        // 降级为 Anonymous 的语义。断言只看行为（放行 + 不注入），不断日志。
        let server = auth_server(ApiKeyAuth::new(Arc::new(FixedKeyRepo), false));

        let anonymous = Request::new(GrpcGenerateRequest {
            namespace: "ns".to_string(),
            tag: "tag".to_string(),
            metadata: HashMap::new(),
        });
        let resp = NebulaIdService::generate(&server, anonymous)
            .await
            .expect("禁用认证时无凭证应放行");
        assert!(!resp.into_inner().id.is_empty());

        let passed = server
            .authenticate(generate_request_with("ghost-key", "wrong-secret"))
            .await
            .expect("禁用认证时凭证内容必须被忽略（无效凭证也放行）");
        assert_eq!(
            passed.extensions().get::<ApiKeyRole>(),
            None,
            "禁用认证不得注入角色（HTTP 侧对应 Anonymous）"
        );

        let resp = NebulaIdService::generate(&server, api_key_request("ghost-key", "wrong-secret"))
            .await
            .expect("禁用认证时 ApiKey 格式的无效凭证同样放行");
        assert!(!resp.into_inner().id.is_empty());
    }

    /// R-auth-003 覆盖度：启用认证后 **全部 5 个 RPC 方法**（含双向流）缺
    /// `authorization` metadata 时一律 `Code::Unauthenticated`。走真实传输而非
    /// 直接调 trait —— 流式入口的 `Streaming<T>` 只能由传输层构造，只有这样才能
    /// 把五个方法放进同一张参数化断言表，不给「某个入口漏调 authenticate」留死角。
    #[tokio::test]
    async fn every_rpc_method_rejects_missing_credentials() {
        let (addr, jh) = spawn_auth_server().await;
        let mut client =
            v1::nebula_id_service_client::NebulaIdServiceClient::connect(format!("http://{addr}"))
                .await
                .expect("client connect");

        for method in [
            "generate",
            "batch_generate",
            "batch_generate_stream",
            "parse",
            "health_check",
        ] {
            let status: Status = match method {
                "generate" => client
                    .generate(GrpcGenerateRequest {
                        namespace: "ns".to_string(),
                        tag: "tag".to_string(),
                        metadata: HashMap::new(),
                    })
                    .await
                    .unwrap_err(),
                "batch_generate" => client
                    .batch_generate(GrpcBatchGenerateRequest {
                        namespace: "ns".to_string(),
                        tag: "tag".to_string(),
                        count: 2,
                        metadata: HashMap::new(),
                    })
                    .await
                    .unwrap_err(),
                "parse" => client
                    .parse(GrpcParseRequest {
                        id: "12345".to_string(),
                    })
                    .await
                    .unwrap_err(),
                "health_check" => client
                    .health_check(HealthCheckRequest {
                        service: String::new(),
                    })
                    .await
                    .unwrap_err(),
                // 双向流的拒绝可能发生在调用本身，也可能延后到首个流项
                "batch_generate_stream" => {
                    match client
                        .batch_generate_stream(tokio_stream::once(v1::BatchGenerateStreamRequest {
                            namespace: "ns".to_string(),
                            tag: "tag".to_string(),
                            count: 1,
                            metadata: HashMap::new(),
                        }))
                        .await
                    {
                        Err(status) => status,
                        Ok(resp) => {
                            let mut stream = resp.into_inner();
                            match tokio_stream::StreamExt::next(&mut stream).await {
                                Some(Err(status)) => status,
                                other => panic!("{method} 无凭证不应产出正常项: {other:?}"),
                            }
                        }
                    }
                }
                other => panic!("参数化表外的方法名: {other}"),
            };
            assert_eq!(
                status.code(),
                Code::Unauthenticated,
                "{method} 无凭证必须是 Unauthenticated，实际: {status:?}"
            );
            assert_eq!(
                status.message(),
                "missing authorization metadata",
                "{method} 的拒绝原因必须是缺少 authorization metadata，实际: {status:?}"
            );
        }

        jh.abort();
    }
}
