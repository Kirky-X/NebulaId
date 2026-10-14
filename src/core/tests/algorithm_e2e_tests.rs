// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! 核心算法层端到端测试：Builder 生命周期、时钟回拨 fallback、降级/熔断周期、
//! Router+AuditLogger 协同等跨模块路径；与各模块单元测试不重复。

use crate::core::algorithm::degradation_manager::DegradationState;
use crate::core::algorithm::DynAuditLogger;
use crate::core::algorithm::{
    AlgorithmBuilder, AlgorithmRouter, DegradationManager, GenerateContext, HealthStatus,
    IdAlgorithm, IdGenerator,
};
use crate::core::algorithm::{AuditEvent, AuditLogger as CoreAuditLoggerTrait};
use crate::core::config::Config;
use crate::core::types::{AlgorithmType, CoreError, Id, IdFormat};
use async_trait::async_trait;
use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;

// =============================================================================
// 测试辅助：捕获审计事件的内存 AuditLogger
// =============================================================================

/// 内存审计记录器：捕获所有 log 调用，供端到端测试断言事件顺序与内容。
struct CapturingAuditLogger {
    events: Arc<Mutex<Vec<AuditEvent>>>,
    log_count: Arc<AtomicU64>,
}

impl CapturingAuditLogger {
    fn new() -> Self {
        Self {
            events: Arc::new(Mutex::new(Vec::new())),
            log_count: Arc::new(AtomicU64::new(0)),
        }
    }

    fn log_count(&self) -> u64 {
        self.log_count.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl CoreAuditLoggerTrait for CapturingAuditLogger {
    async fn log(&self, event: AuditEvent) {
        self.log_count.fetch_add(1, Ordering::SeqCst);
        self.events.lock().await.push(event);
    }
}

fn make_ctx(biz_tag: &str) -> GenerateContext {
    GenerateContext {
        workspace_id: "ws-e2e".to_string(),
        group_id: "grp-e2e".to_string(),
        biz_tag: biz_tag.to_string(),
        format: IdFormat::Numeric,
        prefix: None,
    }
}

// =============================================================================
// 测试辅助：本地 MockIdAlgorithm
// =============================================================================
//
// degradation_tests.rs 的 MockIdAlgorithm 为模块私有，此处定义本地副本，实现保持一致。

struct MockIdAlgorithm {
    alg_type: AlgorithmType,
    should_fail: Arc<AtomicU64>,
    call_count: Arc<AtomicU64>,
    health_status: Arc<AtomicU64>,
}

impl MockIdAlgorithm {
    fn new(alg_type: AlgorithmType) -> Self {
        Self {
            alg_type,
            should_fail: Arc::new(AtomicU64::new(0)),
            call_count: Arc::new(AtomicU64::new(0)),
            health_status: Arc::new(AtomicU64::new(0)),
        }
    }

    fn set_should_fail(&self, fail: bool) {
        self.should_fail.store(fail as u64, Ordering::Relaxed);
    }
}

#[async_trait]
impl IdAlgorithm for MockIdAlgorithm {
    async fn generate(&self, _ctx: &GenerateContext) -> Result<Id, CoreError> {
        self.call_count.fetch_add(1, Ordering::Relaxed);
        if self.should_fail.load(Ordering::Relaxed) == 1 {
            return Err(CoreError::InternalError(
                "Mock algorithm failure".to_string(),
            ));
        }
        Ok(Id::from_u128(1))
    }

    async fn batch_generate(
        &self,
        _ctx: &GenerateContext,
        _size: usize,
    ) -> Result<crate::core::types::IdBatch, CoreError> {
        self.call_count.fetch_add(1, Ordering::Relaxed);
        if self.should_fail.load(Ordering::Relaxed) == 1 {
            return Err(CoreError::InternalError(
                "Mock algorithm failure".to_string(),
            ));
        }
        Ok(crate::core::types::IdBatch::new(
            vec![1, 2, 3].into_iter().map(Id::from_u128).collect(),
            self.alg_type,
            "test".to_string(),
        ))
    }

    fn health_check(&self) -> HealthStatus {
        match self.health_status.load(Ordering::Relaxed) {
            0 => HealthStatus::Healthy,
            1 => HealthStatus::Degraded("Mock degraded".to_string()),
            _ => HealthStatus::Unhealthy("Mock unhealthy".to_string()),
        }
    }

    fn metrics(&self) -> crate::core::algorithm::traits::AlgorithmMetricsSnapshot {
        crate::core::algorithm::traits::AlgorithmMetricsSnapshot {
            total_generated: self.call_count.load(Ordering::Relaxed),
            ..Default::default()
        }
    }

    fn algorithm_type(&self) -> AlgorithmType {
        self.alg_type
    }

    async fn shutdown(&self) -> Result<(), CoreError> {
        Ok(())
    }
}

// =============================================================================
// Snowflake 端到端
// =============================================================================

/// Snowflake 经 AlgorithmBuilder 构建的完整生命周期
/// （build → generate → batch → metrics → health → shutdown）。
///
/// 与 snowflake.rs 单元测试的区别：走工厂注册表（AlgorithmBuilder + SnowflakeFactory）路径。
#[tokio::test]
#[cfg(feature = "snowflake")]
async fn e2e_snowflake_full_lifecycle_via_builder() {
    // 配置级 state_dir 指向测试专属临时目录，避免向默认 ./data 落守卫文件
    // （残留会导致后续运行假性失败，也会覆盖同目录真实服务的守卫基准）。
    let state_dir =
        std::env::temp_dir().join(format!("nebulaid-e2e-sf-state-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&state_dir);

    let mut config = Config::default();
    config.algorithm.snowflake.state_dir = Some(state_dir.to_string_lossy().into_owned());
    let algorithm: Box<dyn IdAlgorithm> = AlgorithmBuilder::new(AlgorithmType::Snowflake)
        .build(&config)
        .await
        .expect("E2E: AlgorithmBuilder should build Snowflake");

    assert_eq!(algorithm.algorithm_type(), AlgorithmType::Snowflake);

    let ctx = make_ctx("e2e-sf-lifecycle");

    let id1 = algorithm
        .generate(&ctx)
        .await
        .expect("E2E: first generate should succeed");
    assert!(id1.as_u128() > 0, "E2E: generated ID must be non-zero");

    let batch = algorithm
        .batch_generate(&ctx, 50)
        .await
        .expect("E2E: batch_generate should succeed");
    assert_eq!(batch.ids.len(), 50);
    assert_eq!(batch.algorithm, AlgorithmType::Snowflake);

    let mut seen: HashSet<u128> = HashSet::new();
    seen.insert(id1.as_u128());
    for id in &batch.ids {
        assert!(
            seen.insert(id.as_u128()),
            "E2E: duplicate ID detected in batch: {}",
            id.as_u128()
        );
    }
    assert_eq!(seen.len(), 51);

    let metrics = algorithm.metrics();
    assert!(
        metrics.total_generated >= 51,
        "E2E: metrics.total_generated should reflect generations, got {}",
        metrics.total_generated
    );
    assert_eq!(metrics.cache_hit_rate, None, "E2E: Snowflake has no cache");

    assert!(
        matches!(algorithm.health_check(), HealthStatus::Healthy),
        "E2E: Snowflake health_check should be Healthy on normal operation"
    );

    algorithm
        .shutdown()
        .await
        .expect("E2E: shutdown should succeed");

    let _ = std::fs::remove_dir_all(&state_dir);
}

// 时钟回拨 → health_check Unhealthy 场景由 snowflake.rs 单元测试覆盖：
// test_generate_id_clock_backward_exceeds_threshold_returns_error 等。

/// Snowflake 在 size=0 时返回空批次（功能场景穷举分析第 1 节边界）。
#[tokio::test]
#[cfg(feature = "snowflake")]
async fn e2e_snowflake_batch_generate_zero_size_returns_empty_batch() {
    let config = Config::default();
    let algorithm: Box<dyn IdAlgorithm> = AlgorithmBuilder::new(AlgorithmType::Snowflake)
        .build(&config)
        .await
        .expect("E2E: AlgorithmBuilder should build Snowflake");

    let ctx = make_ctx("e2e-sf-zero-size");
    let batch = algorithm
        .batch_generate(&ctx, 0)
        .await
        .expect("E2E: batch_generate(0) should succeed with empty batch");
    assert_eq!(batch.ids.len(), 0);
    assert_eq!(batch.algorithm, AlgorithmType::Snowflake);
}

// =============================================================================
// 跨算法端到端
// =============================================================================

/// 3 种算法均经 AlgorithmBuilder 构建并生成跨算法唯一的 ID。
///
/// 验证工厂注册表（`algorithm_factories()`）中每个 Factory 都能构建并通过 trait 接口生成。
#[tokio::test]
#[cfg(all(feature = "segment", feature = "snowflake", feature = "uuid-v8"))]
async fn e2e_all_algorithm_types_built_via_builder_generate_unique_ids() {
    let config = Config::default();
    let algorithm_types = [
        AlgorithmType::Segment,
        AlgorithmType::Snowflake,
        AlgorithmType::UuidV8,
    ];

    let ctx = make_ctx("e2e-all-algorithms");
    let mut all_ids: HashSet<u128> = HashSet::new();

    for alg_type in algorithm_types {
        let algorithm: Box<dyn IdAlgorithm> = AlgorithmBuilder::new(alg_type)
            .build(&config)
            .await
            .unwrap_or_else(|e| panic!("E2E: build {:?} should succeed, got: {:?}", alg_type, e));

        assert_eq!(algorithm.algorithm_type(), alg_type);

        let id = algorithm.generate(&ctx).await.unwrap_or_else(|e| {
            panic!("E2E: {:?} generate should succeed, got: {:?}", alg_type, e)
        });
        assert!(id.as_u128() > 0, "E2E: {:?} ID must be non-zero", alg_type);

        assert!(
            all_ids.insert(id.as_u128()),
            "E2E: cross-algorithm duplicate ID detected for {:?}: {}",
            alg_type,
            id.as_u128()
        );

        let batch = algorithm
            .batch_generate(&ctx, 5)
            .await
            .unwrap_or_else(|e| panic!("E2E: {:?} batch should succeed, got: {:?}", alg_type, e));
        assert_eq!(batch.ids.len(), 5);
        assert_eq!(batch.algorithm, alg_type);
        for bid in &batch.ids {
            assert!(
                all_ids.insert(bid.as_u128()),
                "E2E: cross-algorithm duplicate ID in batch for {:?}: {}",
                alg_type,
                bid.as_u128()
            );
        }
    }

    assert_eq!(all_ids.len(), 3 + 3 * 5);
}

// =============================================================================
// AlgorithmRouter 端到端
// =============================================================================

/// Router.initialize 应注册本构建特性集内的全部算法（路由前置条件：主算法 + fallback 就绪）。
#[tokio::test]
async fn e2e_router_initialize_registers_all_enabled_algorithms() {
    let config = Config::default();
    let router = AlgorithmRouter::new(config, None);

    router
        .initialize()
        .await
        .expect("E2E: Router.initialize should succeed");

    let health_statuses = router.health_check().await;

    // 期望数按构建特性集计算：initialize 的构建循环与注册表同口径门控
    let expected = {
        let mut n = 0;
        #[cfg(feature = "segment")]
        {
            n += 1;
        }
        #[cfg(feature = "snowflake")]
        {
            n += 1;
        }
        #[cfg(feature = "uuid-v8")]
        {
            n += 1;
        }
        #[cfg(feature = "uuid-v4")]
        {
            n += 1;
        }
        #[cfg(feature = "uuid-v7")]
        {
            n += 1;
        }
        n
    };
    assert_eq!(
        health_statuses.len(),
        expected,
        "E2E: Router should register exactly the algorithms of the enabled feature set"
    );

    let registered: HashSet<AlgorithmType> = health_statuses.iter().map(|(t, _)| *t).collect();
    #[cfg(feature = "segment")]
    assert!(registered.contains(&AlgorithmType::Segment));
    #[cfg(feature = "snowflake")]
    assert!(registered.contains(&AlgorithmType::Snowflake));
    #[cfg(feature = "uuid-v8")]
    assert!(registered.contains(&AlgorithmType::UuidV8));
    #[cfg(feature = "uuid-v4")]
    assert!(
        registered.contains(&AlgorithmType::UuidV4),
        "uuid-v4 feature enabled but UuidV4 missing from router registry"
    );
    #[cfg(feature = "uuid-v7")]
    assert!(
        registered.contains(&AlgorithmType::UuidV7),
        "uuid-v7 feature enabled but UuidV7 missing from router registry"
    );

    for (alg_type, status) in &health_statuses {
        // Segment 无数据库时 health_check 返回 Degraded("No active buffers")，属设计行为；
        // 其他算法不依赖外部状态，应为 Healthy。
        match alg_type {
            AlgorithmType::Segment => {
                assert!(
                    matches!(status, HealthStatus::Degraded(_) | HealthStatus::Healthy),
                    "E2E: Segment should be Degraded (no DB) or Healthy, got {:?}",
                    status
                );
            }
            _ => {
                assert!(
                    matches!(status, HealthStatus::Healthy),
                    "E2E: {:?} should be Healthy (no external deps), got {:?}",
                    alg_type,
                    status
                );
            }
        }
    }
}

/// 回归锚点：v4/v7 经路由 `generate_with_algorithm` 须产出各自布局的 ID
/// （曾因 initialize 漏注册而静默 fallback，表现为「请求 v7 得 v8」）。
#[tokio::test]
#[cfg(all(feature = "uuid-v4", feature = "uuid-v7"))]
async fn e2e_router_generate_v4_v7_yields_correct_layout_no_fallback() {
    let config = Config::default();
    let router = AlgorithmRouter::new(config, None);
    router
        .initialize()
        .await
        .expect("E2E: Router.initialize should succeed");

    let v7 = router
        .generate_with_algorithm(AlgorithmType::UuidV7, "ws", "g", "layout-probe")
        .await
        .expect("v7 via router must succeed");
    let v4 = router
        .generate_with_algorithm(AlgorithmType::UuidV4, "ws", "g", "layout-probe")
        .await
        .expect("v4 via router must succeed");

    let v7_bits = v7.to_uuid_v8().as_u128();
    let v4_bits = v4.to_uuid_v8().as_u128();
    assert_eq!(
        (v7_bits >> 76) & 0xF,
        7,
        "v7 via router must carry version 7"
    );
    assert_eq!(
        (v4_bits >> 76) & 0xF,
        4,
        "v4 via router must carry version 4"
    );
    // 互异：两次路由生成不得相同（fallback 到同一算法的另一表现）
    assert_ne!(v7_bits, v4_bits);
}

/// Router 按 biz_tag 覆盖默认算法：set_algorithm 后该 biz_tag 的生成走 Snowflake。
#[tokio::test]
async fn e2e_router_set_algorithm_per_biz_tag_routes_correctly() {
    let config = Config::default();
    let router = AlgorithmRouter::new(config, None);
    router
        .initialize()
        .await
        .expect("E2E: Router.initialize should succeed");

    router
        .set_algorithm("snowflake-tag".to_string(), AlgorithmType::Snowflake)
        .await;

    let name = IdGenerator::get_algorithm_name(&router, "ws", "g", "snowflake-tag")
        .await
        .expect("E2E: get_algorithm_name should succeed");
    assert_eq!(
        name, "snowflake",
        "E2E: biz_tag override should route to Snowflake"
    );

    let name_default = IdGenerator::get_algorithm_name(&router, "ws", "g", "no-override-tag")
        .await
        .expect("E2E: get_algorithm_name (default) should succeed");
    assert_eq!(
        name_default, "segment",
        "E2E: biz_tag without override should fall back to default Segment"
    );
}

/// Router 在主算法成功时直接返回、不触发 fallback（默认 main=Segment，链=[Snowflake, UuidV8]）。
#[tokio::test]
#[cfg(all(feature = "segment", feature = "snowflake", feature = "uuid-v8"))]
async fn e2e_router_primary_succeeds_no_fallback_invoked() {
    let config = Config::default();
    let router = AlgorithmRouter::new(config, None);
    router
        .initialize()
        .await
        .expect("E2E: Router.initialize should succeed");

    let ctx = make_ctx("e2e-primary-success");
    let id = router
        .generate(&ctx)
        .await
        .expect("E2E: primary Segment should succeed");
    assert!(id.as_u128() > 0);

    let dm = router.get_degradation_manager();
    let segment_state = dm
        .get_algorithm_state(AlgorithmType::Segment)
        .expect("E2E: Segment should be registered in DegradationManager");
    assert!(
        !segment_state.is_degraded,
        "E2E: Segment should not be degraded after success"
    );
    assert!(segment_state.consecutive_successes >= 1);
}

/// Router 接受 AuditLogger trait object 注入且不 panic。
///
/// 设计契约：IdGeneration 审计由 handlers 层负责，Router.generate 不写审计日志。
#[tokio::test]
#[cfg(all(feature = "segment", feature = "snowflake", feature = "uuid-v8"))]
async fn e2e_router_accepts_audit_logger_without_panic_and_does_not_log() {
    let capturing = Arc::new(CapturingAuditLogger::new());
    let audit_logger: DynAuditLogger = capturing.clone();

    let config = Config::default();
    let router = AlgorithmRouter::new(config, Some(audit_logger));
    router
        .initialize()
        .await
        .expect("E2E: Router.initialize should succeed");

    let ctx = make_ctx("e2e-audit-logger");
    let _ = router
        .generate(&ctx)
        .await
        .expect("E2E: generate should succeed");

    let _ = router
        .batch_generate(&ctx, 3)
        .await
        .expect("E2E: batch_generate should succeed");

    // 设计契约：Router 不调用 AuditLogger 记录 IdGeneration 事件（职责在 handlers 层）。
    assert_eq!(
        capturing.log_count(),
        0,
        "E2E: Router should not invoke AuditLogger for IdGeneration events (handlers' responsibility)"
    );
}

// =============================================================================
// DegradationManager 端到端
// =============================================================================

/// DegradationManager 完整降级-恢复周期：主算法失败 → Degraded(fallback) → 恢复 → Normal。
///
/// 使用本地 MockIdAlgorithm；相比 degradation_tests.rs 单元测试，覆盖完整循环与状态转移。
#[tokio::test]
async fn e2e_degradation_manager_full_degrade_recover_cycle() {
    use crate::core::algorithm::degradation_manager::DegradationConfig;

    let config = DegradationConfig {
        enabled: true,
        failure_threshold: 3,
        recovery_threshold: 3,
        auto_recovery: true,
        fallback_chain: vec![AlgorithmType::Snowflake, AlgorithmType::UuidV8],
        ..Default::default()
    };
    let manager = DegradationManager::new(Some(config), None);

    let primary = Arc::new(MockIdAlgorithm::new(AlgorithmType::Segment));
    let fallback = Arc::new(MockIdAlgorithm::new(AlgorithmType::Snowflake));
    manager.register_algorithm(AlgorithmType::Segment, primary.clone());
    manager.register_algorithm(AlgorithmType::Snowflake, fallback.clone());
    manager.set_primary_algorithm(AlgorithmType::Segment);
    manager.set_fallback_chain(vec![AlgorithmType::Snowflake]);

    // 初始 Normal
    assert!(matches!(
        manager.determine_effective_algorithm().await,
        DegradationState::Normal
    ));

    // 阶段 1：主算法 3 次失败 → Degraded(Snowflake)
    primary.set_should_fail(true);
    for _ in 0..3 {
        manager
            .record_generation_result(AlgorithmType::Segment, false)
            .await;
    }
    assert!(matches!(
        manager.determine_effective_algorithm().await,
        DegradationState::Degraded(AlgorithmType::Snowflake)
    ));

    // 阶段 2：主算法 3 次成功 → Normal
    primary.set_should_fail(false);
    for _ in 0..3 {
        manager
            .record_generation_result(AlgorithmType::Segment, true)
            .await;
    }
    assert!(matches!(
        manager.determine_effective_algorithm().await,
        DegradationState::Normal
    ));
}

/// DegradationManager 在所有 fallback 失败时进入 Critical 状态。
#[tokio::test]
async fn e2e_degradation_manager_all_algorithms_fail_enters_critical() {
    use crate::core::algorithm::degradation_manager::DegradationConfig;

    let config = DegradationConfig {
        enabled: true,
        failure_threshold: 3,
        recovery_threshold: 3,
        auto_recovery: true,
        fallback_chain: vec![AlgorithmType::Snowflake],
        ..Default::default()
    };
    let manager = DegradationManager::new(Some(config), None);

    let primary = Arc::new(MockIdAlgorithm::new(AlgorithmType::Segment));
    let fallback = Arc::new(MockIdAlgorithm::new(AlgorithmType::Snowflake));
    manager.register_algorithm(AlgorithmType::Segment, primary.clone());
    manager.register_algorithm(AlgorithmType::Snowflake, fallback.clone());
    manager.set_primary_algorithm(AlgorithmType::Segment);
    manager.set_fallback_chain(vec![AlgorithmType::Snowflake]);

    primary.set_should_fail(true);
    fallback.set_should_fail(true);
    for _ in 0..3 {
        manager
            .record_generation_result(AlgorithmType::Segment, false)
            .await;
    }
    for _ in 0..3 {
        manager
            .record_generation_result(AlgorithmType::Snowflake, false)
            .await;
    }

    assert!(
        matches!(
            manager.determine_effective_algorithm().await,
            DegradationState::Critical
        ),
        "E2E: all algorithms degraded should enter Critical state"
    );
}

// =============================================================================
// 跨模块协同端到端
// =============================================================================

/// Router + AlgorithmBuilder + 真实算法完整集成路径（main.rs 启动后用户请求的调用链）。
///
/// Config 默认 → initialize → 算法注册 → IdGenerator trait 各接口 → health/metrics。
#[tokio::test]
#[cfg(all(feature = "segment", feature = "snowflake", feature = "uuid-v8"))]
async fn e2e_router_full_id_generator_trait_integration() {
    let config = Config::default();
    let router = AlgorithmRouter::new(config, None);
    router
        .initialize()
        .await
        .expect("E2E: Router.initialize should succeed");

    // IdGenerator::generate
    let id = IdGenerator::generate(&router, "ws", "g", "bt")
        .await
        .expect("E2E: IdGenerator::generate should succeed");
    assert!(id.as_u128() > 0);

    let ids = IdGenerator::batch_generate(&router, "ws", "g", "bt", 5)
        .await
        .expect("E2E: IdGenerator::batch_generate should succeed");
    assert_eq!(ids.len(), 5);

    // IdGenerator::generate_with_algorithm (显式指定 Snowflake)
    let id_sf =
        IdGenerator::generate_with_algorithm(&router, AlgorithmType::Snowflake, "ws", "g", "bt-sf")
            .await
            .expect("E2E: IdGenerator::generate_with_algorithm(Snowflake) should succeed");
    assert!(id_sf.as_u128() > 0);

    // IdGenerator::batch_generate_with_algorithm (显式指定 UuidV8)
    let ids_v7 = IdGenerator::batch_generate_with_algorithm(
        &router,
        AlgorithmType::UuidV8,
        "ws",
        "g",
        "bt-v7",
        3,
    )
    .await
    .expect("E2E: IdGenerator::batch_generate_with_algorithm(UuidV8) should succeed");
    assert_eq!(ids_v7.len(), 3);

    // IdGenerator::health_check
    let health = IdGenerator::health_check(&router).await;
    assert!(
        matches!(health, HealthStatus::Healthy),
        "E2E: all algorithms healthy should aggregate to Healthy"
    );

    let primary = IdGenerator::get_primary_algorithm(&router).await;
    assert_eq!(primary, "Segment");

    let dm = IdGenerator::get_degradation_manager(&router);
    assert!(Arc::strong_count(dm) >= 1);
}

/// Router fallback chain 在真实算法均健康时不被触发（正常工况下不得误触发）。
#[tokio::test]
#[cfg(all(feature = "segment", feature = "snowflake", feature = "uuid-v8"))]
async fn e2e_router_fallback_chain_not_triggered_on_healthy_primary() {
    let config = Config::default();
    let router = AlgorithmRouter::new(config, None);
    router
        .initialize()
        .await
        .expect("E2E: Router.initialize should succeed");

    let ctx = make_ctx("e2e-no-fallback");
    for _ in 0..10 {
        let _ = router
            .generate(&ctx)
            .await
            .expect("E2E: generate should succeed without fallback");
    }

    let dm = router.get_degradation_manager();
    let segment_state = dm
        .get_algorithm_state(AlgorithmType::Segment)
        .expect("E2E: Segment should be tracked");
    assert!(!segment_state.is_degraded);
    assert!(segment_state.consecutive_successes >= 10);

    // 未调用的 fallback 算法 consecutive_successes 应为 0
    for alg in [
        AlgorithmType::Snowflake,
        AlgorithmType::UuidV8,
        AlgorithmType::UuidV8,
    ] {
        let state = dm.get_algorithm_state(alg).expect("E2E: alg tracked");
        assert_eq!(state.consecutive_successes, 0);
    }
}
