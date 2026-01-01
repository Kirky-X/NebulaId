#![allow(dead_code)]

use crate::algorithm::{audit_trait::DynAuditLogger, HealthStatus, IdAlgorithm};
use crate::AlgorithmType;
use dashmap::DashMap;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::time;
use tracing::{debug, info, warn};

const DEFAULT_DEGRADATION_CHECK_INTERVAL_MS: u64 = 5000;
const DEFAULT_RECOVERY_CHECK_INTERVAL_MS: u64 = 30000;
const DEFAULT_FAILURE_THRESHOLD: u8 = 3;
const DEFAULT_RECOVERY_THRESHOLD: u8 = 5;
const DEFAULT_CIRCUIT_BREAKER_TIMEOUT_MS: u64 = 60000;
const DEFAULT_HALF_OPEN_SUCCESS_THRESHOLD: u8 = 2;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum CircuitBreakerState {
    #[default]
    Closed,
    Open,
    HalfOpen,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DegradationState {
    Normal,
    Degraded(AlgorithmType),
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DegradationConfig {
    pub enabled: bool,
    pub check_interval_ms: u64,
    pub failure_threshold: u8,
    pub recovery_check_interval_ms: u64,
    pub recovery_threshold: u8,
    pub auto_recovery: bool,
    pub fallback_chain: Vec<AlgorithmType>,
    pub circuit_breaker_timeout_ms: u64,
    pub half_open_success_threshold: u8,
    pub enable_circuit_breaker: bool,
}

impl Default for DegradationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            check_interval_ms: DEFAULT_DEGRADATION_CHECK_INTERVAL_MS,
            failure_threshold: DEFAULT_FAILURE_THRESHOLD,
            recovery_check_interval_ms: DEFAULT_RECOVERY_CHECK_INTERVAL_MS,
            recovery_threshold: DEFAULT_RECOVERY_THRESHOLD,
            auto_recovery: true,
            fallback_chain: vec![
                AlgorithmType::Segment,
                AlgorithmType::Snowflake,
                AlgorithmType::UuidV7,
            ],
            circuit_breaker_timeout_ms: DEFAULT_CIRCUIT_BREAKER_TIMEOUT_MS,
            half_open_success_threshold: DEFAULT_HALF_OPEN_SUCCESS_THRESHOLD,
            enable_circuit_breaker: true,
        }
    }
}

#[derive(Debug)]
pub struct AlgorithmHealthState {
    pub alg_type: AlgorithmType,
    pub consecutive_failures: AtomicU8,
    pub consecutive_successes: AtomicU8,
    pub last_failure_time: RwLock<Option<Instant>>,
    pub last_success_time: RwLock<Option<Instant>>,
    pub current_state: AtomicBool,
    pub is_degraded: AtomicBool,
    pub circuit_breaker_state: AtomicU8,
    pub circuit_breaker_opened_at: RwLock<Option<Instant>>,
    pub total_requests: AtomicU64,
    pub total_failures: AtomicU64,
    pub total_successes: AtomicU64,
    pub last_request_time: RwLock<Option<Instant>>,
}

const CIRCUIT_BREAKER_CLOSED: u8 = 0;
const CIRCUIT_BREAKER_OPEN: u8 = 1;
const CIRCUIT_BREAKER_HALF_OPEN: u8 = 2;

impl Clone for AlgorithmHealthState {
    fn clone(&self) -> Self {
        Self {
            alg_type: self.alg_type,
            consecutive_failures: AtomicU8::new(self.consecutive_failures.load(Ordering::SeqCst)),
            consecutive_successes: AtomicU8::new(self.consecutive_successes.load(Ordering::SeqCst)),
            last_failure_time: RwLock::new(*self.last_failure_time.read()),
            last_success_time: RwLock::new(*self.last_success_time.read()),
            current_state: AtomicBool::new(self.current_state.load(Ordering::SeqCst)),
            is_degraded: AtomicBool::new(self.is_degraded.load(Ordering::SeqCst)),
            circuit_breaker_state: AtomicU8::new(self.circuit_breaker_state.load(Ordering::SeqCst)),
            circuit_breaker_opened_at: RwLock::new(*self.circuit_breaker_opened_at.read()),
            total_requests: AtomicU64::new(self.total_requests.load(Ordering::SeqCst)),
            total_failures: AtomicU64::new(self.total_failures.load(Ordering::SeqCst)),
            total_successes: AtomicU64::new(self.total_successes.load(Ordering::SeqCst)),
            last_request_time: RwLock::new(*self.last_request_time.read()),
        }
    }
}

impl AlgorithmHealthState {
    pub fn new(alg_type: AlgorithmType) -> Self {
        Self {
            alg_type,
            consecutive_failures: AtomicU8::new(0),
            consecutive_successes: AtomicU8::new(0),
            last_failure_time: RwLock::new(None),
            last_success_time: RwLock::new(None),
            current_state: AtomicBool::new(true),
            is_degraded: AtomicBool::new(false),
            circuit_breaker_state: AtomicU8::new(CIRCUIT_BREAKER_CLOSED),
            circuit_breaker_opened_at: RwLock::new(None),
            total_requests: AtomicU64::new(0),
            total_failures: AtomicU64::new(0),
            total_successes: AtomicU64::new(0),
            last_request_time: RwLock::new(None),
        }
    }

    pub fn record_request(&self, success: bool) {
        self.total_requests.fetch_add(1, Ordering::SeqCst);
        *self.last_request_time.write() = Some(Instant::now());
        if success {
            self.total_successes.fetch_add(1, Ordering::SeqCst);
        } else {
            self.total_failures.fetch_add(1, Ordering::SeqCst);
        }
    }

    pub fn record_failure(&self) {
        self.consecutive_failures.fetch_add(1, Ordering::SeqCst);
        *self.last_failure_time.write() = Some(Instant::now());
        self.consecutive_successes.store(0, Ordering::SeqCst);
    }

    pub fn record_success(&self) {
        self.consecutive_successes.fetch_add(1, Ordering::SeqCst);
        *self.last_success_time.write() = Some(Instant::now());
        self.consecutive_failures.store(0, Ordering::SeqCst);
    }

    pub fn should_degrade(&self, threshold: u8) -> bool {
        self.consecutive_failures.load(Ordering::SeqCst) >= threshold
    }

    pub fn should_recover(&self, threshold: u8) -> bool {
        self.consecutive_successes.load(Ordering::SeqCst) >= threshold
    }

    pub fn mark_degraded(&self) {
        self.is_degraded.store(true, Ordering::SeqCst);
        self.current_state.store(false, Ordering::SeqCst);
    }

    pub fn mark_recovered(&self) {
        self.is_degraded.store(false, Ordering::SeqCst);
        self.current_state.store(true, Ordering::SeqCst);
    }

    pub fn reset(&self) {
        self.consecutive_failures.store(0, Ordering::SeqCst);
        self.consecutive_successes.store(0, Ordering::SeqCst);
        self.is_degraded.store(false, Ordering::SeqCst);
        self.current_state.store(true, Ordering::SeqCst);
        self.circuit_breaker_state
            .store(CIRCUIT_BREAKER_CLOSED, Ordering::SeqCst);
        *self.circuit_breaker_opened_at.write() = None;
    }

    pub fn get_circuit_breaker_state(&self) -> CircuitBreakerState {
        match self.circuit_breaker_state.load(Ordering::SeqCst) {
            CIRCUIT_BREAKER_OPEN => CircuitBreakerState::Open,
            CIRCUIT_BREAKER_HALF_OPEN => CircuitBreakerState::HalfOpen,
            _ => CircuitBreakerState::Closed,
        }
    }

    pub fn open_circuit_breaker(&self) {
        self.circuit_breaker_state
            .store(CIRCUIT_BREAKER_OPEN, Ordering::SeqCst);
        *self.circuit_breaker_opened_at.write() = Some(Instant::now());
        warn!("Circuit breaker opened for {:?}", self.alg_type);
    }

    pub fn half_open_circuit_breaker(&self) {
        self.circuit_breaker_state
            .store(CIRCUIT_BREAKER_HALF_OPEN, Ordering::SeqCst);
        info!("Circuit breaker half-opened for {:?}", self.alg_type);
    }

    pub fn close_circuit_breaker(&self) {
        self.circuit_breaker_state
            .store(CIRCUIT_BREAKER_CLOSED, Ordering::SeqCst);
        *self.circuit_breaker_opened_at.write() = None;
        info!("Circuit breaker closed for {:?}", self.alg_type);
    }

    pub fn can_make_request(&self) -> bool {
        let state = self.get_circuit_breaker_state();
        match state {
            CircuitBreakerState::Closed => true,
            CircuitBreakerState::Open => false,
            CircuitBreakerState::HalfOpen => true,
        }
    }

    pub fn is_circuit_open(&self, timeout_ms: u64) -> bool {
        if self.get_circuit_breaker_state() != CircuitBreakerState::Open {
            return false;
        }
        if let Some(opened_at) = *self.circuit_breaker_opened_at.read() {
            return opened_at.elapsed() < Duration::from_millis(timeout_ms);
        }
        false
    }

    pub fn get_metrics(&self) -> AlgorithmMetrics {
        let total = self.total_requests.load(Ordering::SeqCst);
        let failures = self.total_failures.load(Ordering::SeqCst);
        AlgorithmMetrics {
            alg_type: self.alg_type,
            total_requests: total,
            total_successes: self.total_successes.load(Ordering::SeqCst),
            total_failures: failures,
            success_rate: if total > 0 {
                ((total.saturating_sub(failures)) as f64 / total as f64) * 100.0
            } else {
                0.0
            },
            consecutive_failures: self.consecutive_failures.load(Ordering::SeqCst),
            consecutive_successes: self.consecutive_successes.load(Ordering::SeqCst),
            circuit_breaker_state: self.get_circuit_breaker_state(),
            is_degraded: self.is_degraded.load(Ordering::SeqCst),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlgorithmMetrics {
    pub alg_type: AlgorithmType,
    pub total_requests: u64,
    pub total_successes: u64,
    pub total_failures: u64,
    pub success_rate: f64,
    pub consecutive_failures: u8,
    pub consecutive_successes: u8,
    pub circuit_breaker_state: CircuitBreakerState,
    pub is_degraded: bool,
}

pub struct DegradationManager {
    config: DegradationConfig,
    algorithms: DashMap<AlgorithmType, Arc<dyn IdAlgorithm>>,
    health_states: DashMap<AlgorithmType, Arc<AlgorithmHealthState>>,
    current_state: RwLock<DegradationState>,
    primary_algorithm: RwLock<AlgorithmType>,
    fallback_chain: RwLock<Vec<AlgorithmType>>,
    running: AtomicBool,
    last_check: RwLock<Instant>,
    audit_logger: Option<DynAuditLogger>,
}

impl DegradationManager {
    pub fn new(config: Option<DegradationConfig>, audit_logger: Option<DynAuditLogger>) -> Self {
        Self {
            config: config.unwrap_or_default(),
            algorithms: DashMap::new(),
            health_states: DashMap::new(),
            current_state: RwLock::new(DegradationState::Normal),
            primary_algorithm: RwLock::new(AlgorithmType::Segment),
            fallback_chain: RwLock::new(vec![]),
            running: AtomicBool::new(false),
            last_check: RwLock::new(Instant::now()),
            audit_logger,
        }
    }

    pub fn register_algorithm(&self, alg_type: AlgorithmType, algorithm: Arc<dyn IdAlgorithm>) {
        self.algorithms.insert(alg_type, algorithm);
        self.health_states
            .insert(alg_type, Arc::new(AlgorithmHealthState::new(alg_type)));
        debug!("Registered algorithm {:?} for health monitoring", alg_type);
    }

    pub fn set_primary_algorithm(&self, alg_type: AlgorithmType) {
        *self.primary_algorithm.write() = alg_type;
        info!("Primary algorithm set to {:?}", alg_type);
    }

    pub fn set_fallback_chain(&self, chain: Vec<AlgorithmType>) {
        *self.fallback_chain.write() = chain.clone();
        debug!("Fallback chain configured: {:?}", chain);
    }

    pub async fn record_generation_result(&self, alg_type: AlgorithmType, success: bool) {
        if let Some(state) = self.health_states.get(&alg_type) {
            let health_state = state.value();
            if success {
                health_state.record_success();
                if health_state.is_degraded.load(Ordering::SeqCst)
                    && health_state.should_recover(self.config.recovery_threshold)
                {
                    self.attempt_recovery(alg_type, health_state).await;
                }
            } else {
                health_state.record_failure();
                if !health_state.is_degraded.load(Ordering::SeqCst)
                    && health_state.should_degrade(self.config.failure_threshold)
                {
                    self.trigger_degradation(alg_type, health_state).await;
                }
            }
        }
    }

    async fn trigger_degradation(&self, alg_type: AlgorithmType, state: &AlgorithmHealthState) {
        state.mark_degraded();
        warn!(
            "Algorithm {:?} degraded due to consecutive failures ({})",
            alg_type,
            state.consecutive_failures.load(Ordering::SeqCst)
        );

        let previous_state = format!("{:?}", DegradationState::Normal);
        let new_state = self.determine_effective_algorithm().await;
        *self.current_state.write() = new_state.clone();
        let current_state_str = format!("{:?}", new_state);
        info!("Degradation state changed to: {:?}", new_state);

        if let Some(ref logger) = self.audit_logger {
            let failure_count = state.consecutive_failures.load(Ordering::SeqCst);
            let details = serde_json::json!({
                "failure_count": failure_count,
                "threshold": self.config.failure_threshold,
                "algorithm_type": format!("{:?}", alg_type)
            });
            logger
                .log_degradation_event(
                    None,
                    "algorithm_degradation".to_string(),
                    format!("{:?}", alg_type),
                    previous_state,
                    current_state_str,
                    details,
                )
                .await;
        }
    }

    async fn attempt_recovery(&self, alg_type: AlgorithmType, state: &AlgorithmHealthState) {
        state.mark_recovered();
        info!(
            "Algorithm {:?} recovered after {} consecutive successes",
            alg_type,
            state.consecutive_successes.load(Ordering::SeqCst)
        );

        let previous_state = format!("{:?}", DegradationState::Degraded(alg_type));
        if alg_type == *self.primary_algorithm.read() {
            let new_state = self.determine_effective_algorithm().await;
            *self.current_state.write() = new_state.clone();
            let current_state_str = format!("{:?}", new_state);
            info!("Restored to primary algorithm, state: {:?}", new_state);

            if let Some(ref logger) = self.audit_logger {
                let success_count = state.consecutive_successes.load(Ordering::SeqCst);
                let details = serde_json::json!({
                    "success_count": success_count,
                    "threshold": self.config.recovery_threshold,
                    "algorithm_type": format!("{:?}", alg_type)
                });
                logger
                    .log_degradation_event(
                        None,
                        "algorithm_recovery".to_string(),
                        format!("{:?}", alg_type),
                        previous_state,
                        current_state_str,
                        details,
                    )
                    .await;
            }
        }
    }

    pub async fn check_all_health(&self) {
        let mut state_changed = false;

        for entry in self.health_states.iter() {
            let alg_type = *entry.key();
            let health_state = entry.value();

            if self.config.enable_circuit_breaker {
                let circuit_state = health_state.get_circuit_breaker_state();
                match circuit_state {
                    CircuitBreakerState::Open => {
                        if health_state.is_circuit_open(self.config.circuit_breaker_timeout_ms) {
                            continue;
                        } else {
                            health_state.half_open_circuit_breaker();
                            debug!(
                                "Circuit breaker timeout reached, trying half-open for {:?}",
                                alg_type
                            );
                        }
                    }
                    CircuitBreakerState::HalfOpen => {
                        let health = if let Some(alg) = self.algorithms.get(&alg_type) {
                            alg.health_check()
                        } else {
                            HealthStatus::Unhealthy(format!("Algorithm {:?} not found", alg_type))
                        };

                        match health {
                            HealthStatus::Healthy => {
                                let successes =
                                    health_state.consecutive_successes.load(Ordering::SeqCst);
                                if successes >= self.config.half_open_success_threshold {
                                    health_state.close_circuit_breaker();
                                    if health_state.is_degraded.load(Ordering::SeqCst) {
                                        health_state.mark_recovered();
                                        state_changed = true;
                                    }
                                    info!(
                                        "Circuit breaker closed for {:?} after {} successes",
                                        alg_type, successes
                                    );
                                } else {
                                    health_state.record_success();
                                }
                            }
                            HealthStatus::Degraded(_) => {
                                health_state.record_failure();
                                health_state.open_circuit_breaker();
                                info!("Circuit breaker re-opened for {:?}", alg_type);
                            }
                            HealthStatus::Unhealthy(_) => {
                                health_state.record_failure();
                                health_state.open_circuit_breaker();
                                info!("Circuit breaker opened for unhealthy {:?}", alg_type);
                            }
                        }
                        continue;
                    }
                    CircuitBreakerState::Closed => {}
                }
            }

            if !health_state.is_degraded.load(Ordering::SeqCst) {
                let health = if let Some(alg) = self.algorithms.get(&alg_type) {
                    alg.health_check()
                } else {
                    HealthStatus::Unhealthy(format!("Algorithm {:?} not found", alg_type))
                };
                match health {
                    HealthStatus::Unhealthy(reason) => {
                        warn!("Algorithm {:?} reported unhealthy: {}", alg_type, reason);
                        health_state.record_failure();
                        if self.config.enable_circuit_breaker
                            && health_state.should_degrade(self.config.failure_threshold)
                        {
                            health_state.open_circuit_breaker();
                            self.trigger_degradation(alg_type, health_state).await;
                            state_changed = true;
                        } else if health_state.should_degrade(self.config.failure_threshold) {
                            self.trigger_degradation(alg_type, health_state).await;
                            state_changed = true;
                        }
                    }
                    HealthStatus::Degraded(reason) => {
                        debug!("Algorithm {:?} degraded: {}", alg_type, reason);
                    }
                    HealthStatus::Healthy => {
                        health_state.record_success();
                    }
                }
            }
        }

        if state_changed {
            let new_state = self.determine_effective_algorithm().await;
            *self.current_state.write() = new_state.clone();
        }

        *self.last_check.write() = Instant::now();
    }

    pub async fn determine_effective_algorithm(&self) -> DegradationState {
        let primary = *self.primary_algorithm.read();
        let chain = self.fallback_chain.read().clone();

        if let Some(state) = self.health_states.get(&primary) {
            if !state.value().is_degraded.load(Ordering::SeqCst) {
                return DegradationState::Normal;
            }
        }

        for fallback in chain {
            if let Some(state) = self.health_states.get(&fallback) {
                if !state.value().is_degraded.load(Ordering::SeqCst) {
                    return DegradationState::Degraded(fallback);
                }
            }
        }

        DegradationState::Critical
    }

    pub async fn get_effective_algorithm(&self) -> AlgorithmType {
        let state = self.current_state.read();
        match &*state {
            DegradationState::Normal => *self.primary_algorithm.read(),
            DegradationState::Degraded(alg) => *alg,
            DegradationState::Critical => {
                for alg in self.fallback_chain.read().clone() {
                    if let Some(state) = self.health_states.get(&alg) {
                        if state.value().current_state.load(Ordering::SeqCst) {
                            return alg;
                        }
                    }
                }
                *self.primary_algorithm.read()
            }
        }
    }

    pub fn get_algorithm_state(&self, alg_type: AlgorithmType) -> Option<AlgorithmHealthStateInfo> {
        self.health_states.get(&alg_type).map(|state| {
            let state = state.value();
            AlgorithmHealthStateInfo {
                alg_type: state.alg_type,
                consecutive_failures: state.consecutive_failures.load(Ordering::SeqCst),
                consecutive_successes: state.consecutive_successes.load(Ordering::SeqCst),
                is_degraded: state.is_degraded.load(Ordering::SeqCst),
                is_healthy: state.current_state.load(Ordering::SeqCst),
            }
        })
    }

    pub fn get_all_states(&self) -> Vec<AlgorithmHealthStateInfo> {
        self.health_states
            .iter()
            .map(|e| {
                let state = e.value();
                AlgorithmHealthStateInfo {
                    alg_type: state.alg_type,
                    consecutive_failures: state.consecutive_failures.load(Ordering::SeqCst),
                    consecutive_successes: state.consecutive_successes.load(Ordering::SeqCst),
                    is_degraded: state.is_degraded.load(Ordering::SeqCst),
                    is_healthy: state.current_state.load(Ordering::SeqCst),
                }
            })
            .collect()
    }

    pub fn get_current_state(&self) -> DegradationState {
        self.current_state.read().clone()
    }

    pub fn manual_degrade(&self, alg_type: AlgorithmType) {
        let state = self
            .health_states
            .entry(alg_type)
            .or_insert_with(|| Arc::new(AlgorithmHealthState::new(alg_type)));
        state.value().mark_degraded();
        info!("Manual degradation triggered for {:?}", alg_type);
    }

    pub fn manual_recover(&self, alg_type: AlgorithmType) {
        if let Some(state) = self.health_states.get(&alg_type) {
            state.value().reset();
            info!("Manual recovery triggered for {:?}", alg_type);
        }
    }

    pub fn update_config(&mut self, config: DegradationConfig) {
        self.config = config.clone();
        info!("Degradation config updated: enabled={}", config.enabled);
    }

    pub fn start_background_check(self: &Arc<Self>) {
        if self
            .running
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            warn!("Background health check is already running");
            return;
        }

        let check_interval = Duration::from_millis(self.config.check_interval_ms);
        let manager = self.clone();

        tokio::spawn(async move {
            let mut interval = time::interval(check_interval);
            info!(
                "Starting background health check with interval {:?}",
                check_interval
            );

            loop {
                interval.tick().await;
                if !manager.config.enabled {
                    continue;
                }
                manager.check_all_health().await;
            }
        });

        info!("Background health check started");
    }

    pub fn stop_background_check(&self) {
        self.running.store(false, Ordering::SeqCst);
        info!("Background health check stopped");
    }
}

#[derive(Debug, Clone)]
pub struct AlgorithmHealthStateInfo {
    pub alg_type: AlgorithmType,
    pub consecutive_failures: u8,
    pub consecutive_successes: u8,
    pub is_degraded: bool,
    pub is_healthy: bool,
}

pub fn default_degradation_config() -> DegradationConfig {
    DegradationConfig {
        enabled: true,
        check_interval_ms: DEFAULT_DEGRADATION_CHECK_INTERVAL_MS,
        failure_threshold: DEFAULT_FAILURE_THRESHOLD,
        recovery_check_interval_ms: DEFAULT_RECOVERY_CHECK_INTERVAL_MS,
        recovery_threshold: DEFAULT_RECOVERY_THRESHOLD,
        auto_recovery: true,
        fallback_chain: vec![
            AlgorithmType::Segment,
            AlgorithmType::Snowflake,
            AlgorithmType::UuidV7,
        ],
        circuit_breaker_timeout_ms: DEFAULT_CIRCUIT_BREAKER_TIMEOUT_MS,
        half_open_success_threshold: DEFAULT_HALF_OPEN_SUCCESS_THRESHOLD,
        enable_circuit_breaker: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_health_state_record_failure() {
        let state = AlgorithmHealthState::new(AlgorithmType::Segment);

        assert_eq!(state.consecutive_failures.load(Ordering::SeqCst), 0);

        state.record_failure();
        assert_eq!(state.consecutive_failures.load(Ordering::SeqCst), 1);

        state.record_failure();
        assert_eq!(state.consecutive_failures.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn test_health_state_record_success() {
        let state = AlgorithmHealthState::new(AlgorithmType::Segment);

        assert_eq!(state.consecutive_successes.load(Ordering::SeqCst), 0);

        state.record_success();
        assert_eq!(state.consecutive_successes.load(Ordering::SeqCst), 1);

        state.record_success();
        assert_eq!(state.consecutive_successes.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn test_health_state_should_degrade() {
        let state = AlgorithmHealthState::new(AlgorithmType::Segment);

        assert!(!state.should_degrade(3));

        for _ in 0..2 {
            state.record_failure();
        }
        assert!(!state.should_degrade(3));

        state.record_failure();
        assert!(state.should_degrade(3));
    }

    #[test]
    fn test_health_state_should_recover() {
        let state = AlgorithmHealthState::new(AlgorithmType::Segment);
        state.mark_degraded();

        assert!(!state.should_recover(3));

        for _ in 0..4 {
            state.record_success();
        }
        assert!(state.should_recover(3));
    }

    #[test]
    fn test_degradation_manager_states() {
        let manager = DegradationManager::new(None, None);

        assert_eq!(manager.get_current_state(), DegradationState::Normal);

        manager.manual_degrade(AlgorithmType::Segment);
        let states = manager.get_all_states();
        assert!(states
            .iter()
            .any(|s| s.alg_type == AlgorithmType::Segment && s.is_degraded));

        manager.manual_recover(AlgorithmType::Segment);
        let states = manager.get_all_states();
        assert!(!states
            .iter()
            .any(|s| s.alg_type == AlgorithmType::Segment && s.is_degraded));
    }

    #[test]
    fn test_default_config() {
        let config = DegradationConfig::default();

        assert!(config.enabled);
        assert_eq!(
            config.check_interval_ms,
            DEFAULT_DEGRADATION_CHECK_INTERVAL_MS
        );
        assert_eq!(config.failure_threshold, DEFAULT_FAILURE_THRESHOLD);
        assert!(config.auto_recovery);
    }
}
