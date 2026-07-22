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

[algorithm.uuid_v7]
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
