use crate::algorithm::degradation_manager::{
    DegradationConfig, DegradationManager, DegradationState,
};
use crate::algorithm::traits::{GenerateContext, HealthStatus, IdAlgorithm};
use crate::coordinator::etcd_cluster_health::{EtcdClusterHealthMonitor, EtcdClusterStatus};
use crate::types::{AlgorithmType, Id, Result};
use async_trait::async_trait;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;

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

    fn set_health_status(&self, status: u64) {
        self.health_status.store(status, Ordering::Relaxed);
    }

    #[allow(dead_code)]
    fn get_call_count(&self) -> u64 {
        self.call_count.load(Ordering::Relaxed)
    }
}

#[async_trait]
impl IdAlgorithm for MockIdAlgorithm {
    async fn generate(&self, _ctx: &GenerateContext) -> Result<Id> {
        self.call_count.fetch_add(1, Ordering::Relaxed);
        if self.should_fail.load(Ordering::Relaxed) == 1 {
            return Err(crate::types::CoreError::InternalError(
                "Mock algorithm failure".to_string(),
            ));
        }
        Ok(Id::from_u128(1))
    }

    async fn batch_generate(
        &self,
        _ctx: &GenerateContext,
        _size: usize,
    ) -> Result<crate::types::IdBatch> {
        self.call_count.fetch_add(1, Ordering::Relaxed);
        if self.should_fail.load(Ordering::Relaxed) == 1 {
            return Err(crate::types::CoreError::InternalError(
                "Mock algorithm failure".to_string(),
            ));
        }
        Ok(crate::types::IdBatch::new(
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

    fn metrics(&self) -> crate::algorithm::traits::AlgorithmMetricsSnapshot {
        crate::algorithm::traits::AlgorithmMetricsSnapshot {
            total_generated: self.call_count.load(Ordering::Relaxed),
            total_failed: 0,
            current_qps: 0,
            p50_latency_us: 0,
            p99_latency_us: 0,
            cache_hit_rate: 0.0,
        }
    }

    fn algorithm_type(&self) -> AlgorithmType {
        self.alg_type
    }

    async fn initialize(&mut self, _config: &crate::config::Config) -> Result<()> {
        Ok(())
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn test_etcd_cluster_status_transitions() {
    let config = crate::config::EtcdConfig::default();
    let cache_path = "/tmp/test_etcd_status_transitions.json".to_string();
    let monitor = EtcdClusterHealthMonitor::new(config, cache_path);

    assert_eq!(monitor.get_status(), EtcdClusterStatus::Healthy);
    assert!(!monitor.is_using_cache());

    monitor.record_failure();
    assert_eq!(monitor.get_status(), EtcdClusterStatus::Healthy);

    monitor.record_failure();
    monitor.record_failure();
    assert_eq!(monitor.get_status(), EtcdClusterStatus::Degraded);

    monitor.record_failure();
    monitor.record_failure();
    assert_eq!(monitor.get_status(), EtcdClusterStatus::Failed);
    assert!(monitor.is_using_cache());

    monitor.record_success().await;
    assert_eq!(monitor.get_status(), EtcdClusterStatus::Healthy);
    assert!(!monitor.is_using_cache());
}

#[tokio::test]
async fn test_etcd_cluster_failure_recovery() {
    let config = crate::config::EtcdConfig::default();
    let cache_path = "/tmp/test_etcd_recovery.json".to_string();
    let monitor = EtcdClusterHealthMonitor::new(config, cache_path);

    for _ in 0..5 {
        monitor.record_failure();
    }
    assert_eq!(monitor.get_status(), EtcdClusterStatus::Failed);
    assert!(monitor.is_using_cache());

    monitor.record_success().await;
    assert_eq!(monitor.get_status(), EtcdClusterStatus::Healthy);
    assert!(!monitor.is_using_cache());
}

#[tokio::test]
async fn test_etcd_cluster_cache_fallback() {
    let config = crate::config::EtcdConfig::default();
    let cache_path = "/tmp/test_etcd_cache_fallback.json".to_string();
    let monitor = EtcdClusterHealthMonitor::new(config, cache_path);

    monitor.put_to_cache("test_key".to_string(), "test_value".to_string(), 1);
    monitor.put_to_cache("test_key2".to_string(), "test_value2".to_string(), 2);

    for _ in 0..5 {
        monitor.record_failure();
    }

    assert!(monitor.is_using_cache());

    let entry = monitor.get_from_cache("test_key");
    assert!(entry.is_some());
    assert_eq!(entry.unwrap().value, "test_value");

    let entry2 = monitor.get_from_cache("test_key2");
    assert!(entry2.is_some());
    assert_eq!(entry2.unwrap().value, "test_value2");
}

#[tokio::test]
async fn test_degradation_manager_algorithm_degradation() {
    let config = DegradationConfig {
        enabled: true,
        failure_threshold: 3,
        recovery_threshold: 3,
        auto_recovery: true,
        fallback_chain: vec![AlgorithmType::Snowflake, AlgorithmType::UuidV7],
        ..Default::default()
    };

    let manager = DegradationManager::new(Some(config), None);

    let primary_alg = Arc::new(MockIdAlgorithm::new(AlgorithmType::Segment));
    let fallback_alg = Arc::new(MockIdAlgorithm::new(AlgorithmType::Snowflake));

    manager.register_algorithm(AlgorithmType::Segment, primary_alg.clone());
    manager.register_algorithm(AlgorithmType::Snowflake, fallback_alg.clone());
    manager.set_primary_algorithm(AlgorithmType::Segment);
    manager.set_fallback_chain(vec![AlgorithmType::Snowflake]);

    primary_alg.set_should_fail(true);

    for _ in 0..3 {
        let result = primary_alg.generate(&GenerateContext::default()).await;
        assert!(result.is_err());
        manager
            .record_generation_result(AlgorithmType::Segment, false)
            .await;
    }

    let state = manager.determine_effective_algorithm().await;
    assert!(matches!(state, DegradationState::Degraded(_)));

    primary_alg.set_should_fail(false);

    for _ in 0..3 {
        let result = primary_alg.generate(&GenerateContext::default()).await;
        assert!(result.is_ok());
        manager
            .record_generation_result(AlgorithmType::Segment, true)
            .await;
    }

    let state = manager.determine_effective_algorithm().await;
    assert!(matches!(state, DegradationState::Normal));
}

#[tokio::test]
async fn test_circuit_breaker_state_transitions() {
    let config = DegradationConfig {
        enabled: true,
        failure_threshold: 3,
        recovery_threshold: 2,
        auto_recovery: true,
        enable_circuit_breaker: true,
        circuit_breaker_timeout_ms: 1000,
        half_open_success_threshold: 2,
        fallback_chain: vec![AlgorithmType::Snowflake],
        ..Default::default()
    };

    let manager = DegradationManager::new(Some(config), None);

    let alg = Arc::new(MockIdAlgorithm::new(AlgorithmType::Segment));
    manager.register_algorithm(AlgorithmType::Segment, alg.clone());
    manager.set_primary_algorithm(AlgorithmType::Segment);

    alg.set_should_fail(true);

    for _ in 0..3 {
        manager
            .record_generation_result(AlgorithmType::Segment, false)
            .await;
    }

    manager.check_all_health().await;

    let health_state = manager.get_algorithm_state(AlgorithmType::Segment).unwrap();
    assert!(health_state.is_degraded);

    sleep(Duration::from_millis(1100)).await;

    manager.check_all_health().await;

    let health_state = manager.get_algorithm_state(AlgorithmType::Segment).unwrap();
    assert!(health_state.is_degraded);

    alg.set_should_fail(false);
    alg.set_health_status(0);

    for _ in 0..2 {
        manager
            .record_generation_result(AlgorithmType::Segment, true)
            .await;
    }

    manager.check_all_health().await;

    let health_state = manager.get_algorithm_state(AlgorithmType::Segment).unwrap();
    assert!(!health_state.is_degraded);
}

#[tokio::test]
async fn test_fallback_algorithm_switching() {
    let config = DegradationConfig {
        enabled: true,
        failure_threshold: 3,
        recovery_threshold: 3,
        auto_recovery: true,
        fallback_chain: vec![AlgorithmType::Snowflake, AlgorithmType::UuidV7],
        ..Default::default()
    };

    let manager = DegradationManager::new(Some(config), None);

    let primary_alg = Arc::new(MockIdAlgorithm::new(AlgorithmType::Segment));
    let fallback1 = Arc::new(MockIdAlgorithm::new(AlgorithmType::Snowflake));
    let fallback2 = Arc::new(MockIdAlgorithm::new(AlgorithmType::UuidV7));

    manager.register_algorithm(AlgorithmType::Segment, primary_alg.clone());
    manager.register_algorithm(AlgorithmType::Snowflake, fallback1.clone());
    manager.register_algorithm(AlgorithmType::UuidV7, fallback2.clone());
    manager.set_primary_algorithm(AlgorithmType::Segment);
    manager.set_fallback_chain(vec![AlgorithmType::Snowflake, AlgorithmType::UuidV7]);

    primary_alg.set_should_fail(true);

    for _ in 0..3 {
        manager
            .record_generation_result(AlgorithmType::Segment, false)
            .await;
    }

    let state = manager.determine_effective_algorithm().await;
    assert!(matches!(
        state,
        DegradationState::Degraded(AlgorithmType::Snowflake)
    ));

    fallback1.set_should_fail(true);

    for _ in 0..3 {
        manager
            .record_generation_result(AlgorithmType::Snowflake, false)
            .await;
    }

    let state = manager.determine_effective_algorithm().await;
    assert!(matches!(
        state,
        DegradationState::Degraded(AlgorithmType::UuidV7)
    ));
}

#[tokio::test]
async fn test_degradation_manager_health_monitoring() {
    let config = DegradationConfig {
        enabled: true,
        failure_threshold: 3,
        recovery_threshold: 3,
        auto_recovery: true,
        enable_circuit_breaker: false,
        fallback_chain: vec![],
        ..Default::default()
    };

    let manager = DegradationManager::new(Some(config), None);

    let alg = Arc::new(MockIdAlgorithm::new(AlgorithmType::Segment));
    manager.register_algorithm(AlgorithmType::Segment, alg.clone());
    manager.set_primary_algorithm(AlgorithmType::Segment);

    alg.set_health_status(2);

    for _ in 0..3 {
        manager.check_all_health().await;
    }

    let health_state = manager.get_algorithm_state(AlgorithmType::Segment).unwrap();
    assert!(health_state.is_degraded);

    alg.set_health_status(0);

    for _ in 0..3 {
        manager
            .record_generation_result(AlgorithmType::Segment, true)
            .await;
    }

    let health_state = manager.get_algorithm_state(AlgorithmType::Segment).unwrap();
    assert!(!health_state.is_degraded);
}

#[tokio::test]
async fn test_algorithm_metrics_collection() {
    let config = DegradationConfig::default();
    let manager = DegradationManager::new(Some(config), None);

    let alg = Arc::new(MockIdAlgorithm::new(AlgorithmType::Segment));
    manager.register_algorithm(AlgorithmType::Segment, alg.clone());

    for _ in 0..5 {
        let result = alg.generate(&GenerateContext::default()).await;
        manager
            .record_generation_result(AlgorithmType::Segment, result.is_ok())
            .await;
    }

    let _health_state = manager.get_algorithm_state(AlgorithmType::Segment).unwrap();
    let metrics = manager.get_all_states();
    let segment_metrics = metrics
        .iter()
        .find(|m| m.alg_type == AlgorithmType::Segment)
        .unwrap();

    assert_eq!(segment_metrics.alg_type, AlgorithmType::Segment);
    assert_eq!(segment_metrics.consecutive_successes, 5);
    assert!(!segment_metrics.is_degraded);
}

#[tokio::test]
async fn test_circuit_breaker_timeout() {
    let config = DegradationConfig {
        enabled: true,
        failure_threshold: 3,
        recovery_threshold: 2,
        auto_recovery: true,
        enable_circuit_breaker: true,
        circuit_breaker_timeout_ms: 500,
        half_open_success_threshold: 2,
        fallback_chain: vec![],
        ..Default::default()
    };

    let manager = DegradationManager::new(Some(config), None);

    let alg = Arc::new(MockIdAlgorithm::new(AlgorithmType::Segment));
    manager.register_algorithm(AlgorithmType::Segment, alg.clone());

    alg.set_should_fail(true);

    for _ in 0..3 {
        manager
            .record_generation_result(AlgorithmType::Segment, false)
            .await;
    }

    manager.check_all_health().await;

    let health_state = manager.get_algorithm_state(AlgorithmType::Segment).unwrap();
    assert!(health_state.is_degraded);

    sleep(Duration::from_millis(600)).await;

    manager.check_all_health().await;

    let health_state = manager.get_algorithm_state(AlgorithmType::Segment).unwrap();
    assert!(health_state.is_degraded);

    alg.set_should_fail(false);
    alg.set_health_status(0);

    for _ in 0..2 {
        manager
            .record_generation_result(AlgorithmType::Segment, true)
            .await;
        manager.check_all_health().await;
    }

    let health_state = manager.get_algorithm_state(AlgorithmType::Segment).unwrap();
    assert!(!health_state.is_degraded);
}

#[tokio::test]
async fn test_multiple_algorithm_degradation() {
    let config = DegradationConfig {
        enabled: true,
        failure_threshold: 3,
        recovery_threshold: 3,
        auto_recovery: true,
        fallback_chain: vec![AlgorithmType::Snowflake, AlgorithmType::UuidV7],
        ..Default::default()
    };

    let manager = DegradationManager::new(Some(config), None);

    let alg1 = Arc::new(MockIdAlgorithm::new(AlgorithmType::Segment));
    let alg2 = Arc::new(MockIdAlgorithm::new(AlgorithmType::Snowflake));
    let alg3 = Arc::new(MockIdAlgorithm::new(AlgorithmType::UuidV7));

    manager.register_algorithm(AlgorithmType::Segment, alg1.clone());
    manager.register_algorithm(AlgorithmType::Snowflake, alg2.clone());
    manager.register_algorithm(AlgorithmType::UuidV7, alg3.clone());
    manager.set_primary_algorithm(AlgorithmType::Segment);
    manager.set_fallback_chain(vec![AlgorithmType::Snowflake, AlgorithmType::UuidV7]);

    alg1.set_should_fail(true);

    for _ in 0..3 {
        manager
            .record_generation_result(AlgorithmType::Segment, false)
            .await;
    }

    let state = manager.determine_effective_algorithm().await;
    assert!(matches!(
        state,
        DegradationState::Degraded(AlgorithmType::Snowflake)
    ));

    alg2.set_should_fail(true);

    for _ in 0..3 {
        manager
            .record_generation_result(AlgorithmType::Snowflake, false)
            .await;
    }

    let state = manager.determine_effective_algorithm().await;
    assert!(matches!(
        state,
        DegradationState::Degraded(AlgorithmType::UuidV7)
    ));

    alg1.set_should_fail(false);

    for _ in 0..3 {
        manager
            .record_generation_result(AlgorithmType::Segment, true)
            .await;
    }

    let state = manager.determine_effective_algorithm().await;
    assert!(matches!(state, DegradationState::Normal));
}

#[tokio::test]
async fn test_etcd_cache_persistence_across_instances() {
    let config = crate::config::EtcdConfig::default();
    let cache_path = "/tmp/test_etcd_cache_persistence.json".to_string();

    let monitor1 = EtcdClusterHealthMonitor::new(config.clone(), cache_path.clone());

    monitor1.put_to_cache("key1".to_string(), "value1".to_string(), 1);
    monitor1.put_to_cache("key2".to_string(), "value2".to_string(), 2);

    monitor1.save_local_cache().await.unwrap();

    let monitor2 = EtcdClusterHealthMonitor::new(config, cache_path);
    monitor2.load_local_cache().await.unwrap();

    let entry1 = monitor2.get_from_cache("key1");
    let entry2 = monitor2.get_from_cache("key2");

    assert!(entry1.is_some());
    assert!(entry2.is_some());
    assert_eq!(entry1.unwrap().value, "value1");
    assert_eq!(entry2.unwrap().value, "value2");

    let _ = tokio::fs::remove_file("/tmp/test_etcd_cache_persistence.json").await;
}

#[tokio::test]
async fn test_degradation_with_circuit_breaker_and_fallback() {
    let config = DegradationConfig {
        enabled: true,
        failure_threshold: 3,
        recovery_threshold: 2,
        auto_recovery: true,
        enable_circuit_breaker: true,
        circuit_breaker_timeout_ms: 500,
        half_open_success_threshold: 2,
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

    for _ in 0..3 {
        manager
            .record_generation_result(AlgorithmType::Segment, false)
            .await;
    }

    manager.check_all_health().await;

    let health_state = manager.get_algorithm_state(AlgorithmType::Segment).unwrap();
    assert!(health_state.is_degraded);

    let effective_state = manager.determine_effective_algorithm().await;
    assert!(matches!(
        effective_state,
        DegradationState::Degraded(AlgorithmType::Snowflake)
    ));

    sleep(Duration::from_millis(600)).await;

    manager.check_all_health().await;

    let health_state = manager.get_algorithm_state(AlgorithmType::Segment).unwrap();
    assert!(health_state.is_degraded);

    primary.set_should_fail(false);
    primary.set_health_status(0);

    for _ in 0..2 {
        manager
            .record_generation_result(AlgorithmType::Segment, true)
            .await;
    }

    manager.check_all_health().await;

    let health_state = manager.get_algorithm_state(AlgorithmType::Segment).unwrap();
    assert!(!health_state.is_degraded);

    let effective_state = manager.determine_effective_algorithm().await;
    assert!(matches!(effective_state, DegradationState::Normal));
}
