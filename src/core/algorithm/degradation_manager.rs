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

//! 降级管理器模块（Degradation Manager）
//!
//! # 当前状态：v0.3.0 完整接入告警管道前的预留 API
//!
//! 本模块包含若干暂时未被生产路径直接调用的 API：
//! - `AlgorithmHealthState::{record_request, can_make_request, get_metrics}`
//! - `AlgorithmMetrics` 结构体
//! - `default_degradation_config` 函数
//!
//! 保留原因：
//!
//! 1. **告警管道集成预留**：v0.3.0 启用告警管道后，`record_request` 和 `get_metrics`
//!    将作为 Prometheus 指标采集入口；`AlgorithmMetrics` 是指标导出的数据结构。
//! 2. **熔断器与降级联动**：`can_make_request` 是熔断器在 HalfOpen 状态下的探针请求
//!    判定入口，待 circuit_breaker 接入告警管道后启用。
//! 3. **测试覆盖**：`default_degradation_config` 被多个单元测试用于快速构造默认配置，
//!    删除会丢失测试便利性。
//! 4. **API 完整性**：降级管理器对外应暴露完整的健康状态、指标查询、配置构造能力。
//!
//! 详见 `specmark/changes/v0.3.0-release/` 中的告警管道设计文档。
#![allow(dead_code)]

use crate::core::algorithm::{audit_trait::DynAuditLogger, HealthStatus, IdAlgorithm};
use crate::core::AlgorithmType;
use arc_swap::ArcSwap;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::time;
use tracing::{debug, info, warn};

// Circuit breaker constants tuned for production stability
const DEFAULT_DEGRADATION_CHECK_INTERVAL_MS: u64 = 5000;
const DEFAULT_RECOVERY_CHECK_INTERVAL_MS: u64 = 30000;
// Higher failure threshold to avoid false positives during temporary network glitches
const DEFAULT_FAILURE_THRESHOLD: u8 = 5;
// Higher recovery threshold to ensure stability before closing circuit
const DEFAULT_RECOVERY_THRESHOLD: u8 = 10;
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

// M13 修复：删除手动 `impl Clone for AlgorithmHealthState`。
// 该 Clone 实现逐个 load/store 原子字段，冗长且易错；且全代码库无任何调用方
// （所有共享都通过 `Arc<AlgorithmHealthState>`，Arc clone 不需要 T: Clone）。
// 如果未来需要 Clone，应改用 `Arc<AtomicXxx>` 字段 + `#[derive(Clone)]`（共享语义）。

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
        warn!(
            alg_type = ?self.alg_type,
            "{}",
            t!("log.core.algorithm.degradation_manager.circuit_breaker_opened")
        );
    }

    pub fn half_open_circuit_breaker(&self) {
        self.circuit_breaker_state
            .store(CIRCUIT_BREAKER_HALF_OPEN, Ordering::SeqCst);
        info!(
            alg_type = ?self.alg_type,
            "{}",
            t!("log.core.algorithm.degradation_manager.circuit_breaker_half_opened")
        );
    }

    pub fn close_circuit_breaker(&self) {
        self.circuit_breaker_state
            .store(CIRCUIT_BREAKER_CLOSED, Ordering::SeqCst);
        *self.circuit_breaker_opened_at.write() = None;
        info!(
            alg_type = ?self.alg_type,
            "{}",
            t!("log.core.algorithm.degradation_manager.circuit_breaker_closed")
        );
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
    algorithms: Arc<ArcSwap<HashMap<AlgorithmType, Arc<dyn IdAlgorithm>>>>,
    health_states: Arc<ArcSwap<HashMap<AlgorithmType, Arc<AlgorithmHealthState>>>>,
    current_state: RwLock<DegradationState>,
    primary_algorithm: RwLock<AlgorithmType>,
    fallback_chain: RwLock<Vec<AlgorithmType>>,
    running: AtomicBool,
    last_check: RwLock<Instant>,
    audit_logger: Option<DynAuditLogger>,
    /// F-02 修复：watch channel 用于优雅关闭后台 task。
    /// `start_background_check` 创建 sender，task 用 `select!` 监听 receiver；
    /// `stop_background_check` 发送 `true` 通知 task 退出。
    shutdown_tx: parking_lot::Mutex<Option<tokio::sync::watch::Sender<bool>>>,
    /// 后台 task 的 JoinHandle，`stop_background_check` 可 await 它确认 task 已退出。
    background_task: parking_lot::Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl DegradationManager {
    pub fn new(config: Option<DegradationConfig>, audit_logger: Option<DynAuditLogger>) -> Self {
        Self {
            config: config.unwrap_or_default(),
            algorithms: Arc::new(ArcSwap::from_pointee(HashMap::new())),
            health_states: Arc::new(ArcSwap::from_pointee(HashMap::new())),
            current_state: RwLock::new(DegradationState::Normal),
            primary_algorithm: RwLock::new(AlgorithmType::Segment),
            fallback_chain: RwLock::new(vec![]),
            running: AtomicBool::new(false),
            last_check: RwLock::new(Instant::now()),
            audit_logger,
            shutdown_tx: parking_lot::Mutex::new(None),
            background_task: parking_lot::Mutex::new(None),
        }
    }

    pub fn register_algorithm(&self, alg_type: AlgorithmType, algorithm: Arc<dyn IdAlgorithm>) {
        self.algorithms.rcu(|old| {
            let mut new: HashMap<_, _> = (**old).clone();
            new.insert(alg_type, algorithm.clone());
            Arc::new(new)
        });
        self.health_states.rcu(|old| {
            let mut new: HashMap<_, _> = (**old).clone();
            new.insert(alg_type, Arc::new(AlgorithmHealthState::new(alg_type)));
            Arc::new(new)
        });
        debug!(
            alg_type = ?alg_type,
            "{}",
            t!("log.core.algorithm.degradation_manager.algorithm_registered")
        );
    }

    pub fn set_primary_algorithm(&self, alg_type: AlgorithmType) {
        *self.primary_algorithm.write() = alg_type;
        info!(
            alg_type = ?alg_type,
            "{}",
            t!("log.core.algorithm.degradation_manager.primary_algorithm_set")
        );
    }

    pub fn set_fallback_chain(&self, chain: Vec<AlgorithmType>) {
        *self.fallback_chain.write() = chain.clone();
        debug!(
            chain = ?chain,
            "{}",
            t!("log.core.algorithm.degradation_manager.fallback_chain_configured")
        );
    }

    pub async fn record_generation_result(&self, alg_type: AlgorithmType, success: bool) {
        let state_opt = { self.health_states.load().get(&alg_type).cloned() };

        if let Some(state) = state_opt {
            if success {
                state.record_success();
                if state.is_degraded.load(Ordering::SeqCst)
                    && state.should_recover(self.config.recovery_threshold)
                {
                    self.attempt_recovery(alg_type, &state).await;
                }
            } else {
                state.record_failure();
                if !state.is_degraded.load(Ordering::SeqCst)
                    && state.should_degrade(self.config.failure_threshold)
                {
                    self.trigger_degradation(alg_type, &state).await;
                }
            }
        }
    }

    async fn trigger_degradation(&self, alg_type: AlgorithmType, state: &AlgorithmHealthState) {
        state.mark_degraded();
        warn!(
            alg_type = ?alg_type,
            "{}",
            t!(
                "log.core.algorithm.degradation_manager.algorithm_degraded",
                failure_count = state.consecutive_failures.load(Ordering::SeqCst)
            )
        );

        let previous_state = format!("{:?}", DegradationState::Normal);
        let new_state = self.determine_effective_algorithm().await;
        *self.current_state.write() = new_state.clone();
        let current_state_str = format!("{:?}", new_state);
        info!(
            new_state = ?new_state,
            "{}",
            t!("log.core.algorithm.degradation_manager.degradation_state_changed")
        );

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
            alg_type = ?alg_type,
            "{}",
            t!(
                "log.core.algorithm.degradation_manager.algorithm_recovered",
                success_count = state.consecutive_successes.load(Ordering::SeqCst)
            )
        );

        let previous_state = format!("{:?}", DegradationState::Degraded(alg_type));
        if alg_type == *self.primary_algorithm.read() {
            let new_state = self.determine_effective_algorithm().await;
            *self.current_state.write() = new_state.clone();
            let current_state_str = format!("{:?}", new_state);
            info!(
                new_state = ?new_state,
                "{}",
                t!("log.core.algorithm.degradation_manager.restored_to_primary")
            );

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

        let health_states = self.health_states.load_full();
        let algorithms = self.algorithms.load_full();

        for (alg_type, health_state) in health_states.iter() {
            if self.config.enable_circuit_breaker {
                let circuit_state = health_state.get_circuit_breaker_state();
                match circuit_state {
                    CircuitBreakerState::Open => {
                        if health_state.is_circuit_open(self.config.circuit_breaker_timeout_ms) {
                            continue;
                        } else {
                            health_state.half_open_circuit_breaker();
                            debug!(
                                alg_type = ?alg_type,
                                "{}",
                                t!("log.core.algorithm.degradation_manager.circuit_breaker_timeout_half_open")
                            );
                        }
                    }
                    CircuitBreakerState::HalfOpen => {
                        let health = if let Some(alg) = algorithms.get(alg_type) {
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
                                        alg_type = ?alg_type,
                                        "{}",
                                        t!(
                                            "log.core.algorithm.degradation_manager.circuit_breaker_closed_after_successes",
                                            successes = successes
                                        )
                                    );
                                } else {
                                    health_state.record_success();
                                }
                            }
                            HealthStatus::Degraded(_) => {
                                health_state.record_failure();
                                health_state.open_circuit_breaker();
                                info!(
                                    alg_type = ?alg_type,
                                    "{}",
                                    t!("log.core.algorithm.degradation_manager.circuit_breaker_reopened")
                                );
                            }
                            HealthStatus::Unhealthy(_) => {
                                health_state.record_failure();
                                health_state.open_circuit_breaker();
                                info!(
                                    alg_type = ?alg_type,
                                    "{}",
                                    t!("log.core.algorithm.degradation_manager.circuit_breaker_opened_unhealthy")
                                );
                            }
                        }
                        continue;
                    }
                    CircuitBreakerState::Closed => {}
                }
            }

            if !health_state.is_degraded.load(Ordering::SeqCst) {
                let health = if let Some(alg) = algorithms.get(alg_type) {
                    alg.health_check()
                } else {
                    HealthStatus::Unhealthy(format!("Algorithm {:?} not found", alg_type))
                };
                match health {
                    HealthStatus::Unhealthy(reason) => {
                        warn!(
                            alg_type = ?alg_type,
                            "{}",
                            t!(
                                "log.core.algorithm.degradation_manager.algorithm_unhealthy",
                                reason = reason
                            )
                        );
                        health_state.record_failure();
                        if self.config.enable_circuit_breaker
                            && health_state.should_degrade(self.config.failure_threshold)
                        {
                            health_state.open_circuit_breaker();
                            self.trigger_degradation(*alg_type, health_state).await;
                            state_changed = true;
                        } else if health_state.should_degrade(self.config.failure_threshold) {
                            self.trigger_degradation(*alg_type, health_state).await;
                            state_changed = true;
                        }
                    }
                    HealthStatus::Degraded(reason) => {
                        debug!(
                            alg_type = ?alg_type,
                            "{}",
                            t!(
                                "log.core.algorithm.degradation_manager.algorithm_health_degraded",
                                reason = reason
                            )
                        );
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

        let health_states = self.health_states.load_full();

        if let Some(state) = health_states.get(&primary) {
            if !state.is_degraded.load(Ordering::SeqCst) {
                return DegradationState::Normal;
            }
        }

        for fallback in chain {
            if let Some(state) = health_states.get(&fallback) {
                if !state.is_degraded.load(Ordering::SeqCst) {
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
                let chain = self.fallback_chain.read().clone();
                let health_states = self.health_states.load_full();
                for alg in chain {
                    if let Some(state) = health_states.get(&alg) {
                        if state.current_state.load(Ordering::SeqCst) {
                            return alg;
                        }
                    }
                }
                *self.primary_algorithm.read()
            }
        }
    }

    pub fn get_algorithm_state(&self, alg_type: AlgorithmType) -> Option<AlgorithmHealthStateInfo> {
        self.health_states
            .load()
            .get(&alg_type)
            .map(|state| AlgorithmHealthStateInfo {
                alg_type: state.alg_type,
                consecutive_failures: state.consecutive_failures.load(Ordering::SeqCst),
                consecutive_successes: state.consecutive_successes.load(Ordering::SeqCst),
                is_degraded: state.is_degraded.load(Ordering::SeqCst),
                is_healthy: state.current_state.load(Ordering::SeqCst),
            })
    }

    pub fn get_all_states(&self) -> Vec<AlgorithmHealthStateInfo> {
        self.health_states
            .load()
            .values()
            .map(|state| AlgorithmHealthStateInfo {
                alg_type: state.alg_type,
                consecutive_failures: state.consecutive_failures.load(Ordering::SeqCst),
                consecutive_successes: state.consecutive_successes.load(Ordering::SeqCst),
                is_degraded: state.is_degraded.load(Ordering::SeqCst),
                is_healthy: state.current_state.load(Ordering::SeqCst),
            })
            .collect()
    }

    pub fn get_current_state(&self) -> DegradationState {
        self.current_state.read().clone()
    }

    pub fn manual_degrade(&self, alg_type: AlgorithmType) {
        // 先尝试在已有 state 上标记（无锁读路径）
        let existing = self.health_states.load().get(&alg_type).cloned();
        if let Some(state) = existing {
            state.mark_degraded();
        } else {
            // 不存在则插入新 state（rcu 整体替换）
            let new_state = Arc::new(AlgorithmHealthState::new(alg_type));
            new_state.mark_degraded();
            self.health_states.rcu(|old| {
                let mut new: HashMap<_, _> = (**old).clone();
                new.insert(alg_type, new_state.clone());
                Arc::new(new)
            });
        }
        info!(
            alg_type = ?alg_type,
            "{}",
            t!("log.core.algorithm.degradation_manager.manual_degradation_triggered")
        );
    }

    pub fn manual_recover(&self, alg_type: AlgorithmType) {
        if let Some(state) = self.health_states.load().get(&alg_type) {
            state.reset();
            info!(
                alg_type = ?alg_type,
                "{}",
                t!("log.core.algorithm.degradation_manager.manual_recovery_triggered")
            );
        }
    }

    pub fn update_config(&mut self, config: DegradationConfig) {
        self.config = config.clone();
        info!(
            "{}",
            t!(
                "log.core.algorithm.degradation_manager.degradation_config_updated",
                enabled = config.enabled
            )
        );
    }

    pub fn start_background_check(self: &Arc<Self>) {
        if self
            .running
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            warn!(
                "{}",
                t!("log.core.algorithm.degradation_manager.background_check_already_running")
            );
            return;
        }

        let check_interval = Duration::from_millis(self.config.check_interval_ms);
        let manager = self.clone();
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::watch::channel(false);

        let handle = tokio::spawn(async move {
            let mut interval = time::interval(check_interval);
            info!(
                interval = ?check_interval,
                "{}",
                t!("log.core.algorithm.degradation_manager.starting_background_check")
            );

            loop {
                tokio::select! {
                    // 优先检查 shutdown 信号
                    _ = shutdown_rx.changed() => {
                        if *shutdown_rx.borrow() {
                            info!(
                                "{}",
                                t!("log.core.algorithm.degradation_manager.background_check_shutdown_received")
                            );
                            break;
                        }
                    }
                    _ = interval.tick() => {
                        if !manager.config.enabled {
                            continue;
                        }
                        manager.check_all_health().await;
                    }
                }
            }
        });

        // 保存 sender 和 handle 以便 stop 时使用（parking_lot::Mutex 安全访问）
        *self.shutdown_tx.lock() = Some(shutdown_tx);
        *self.background_task.lock() = Some(handle);

        info!(
            "{}",
            t!("log.core.algorithm.degradation_manager.background_check_started")
        );
    }

    /// 停止后台健康检查 task。
    ///
    /// 发送 shutdown 信号并等待 task 退出（F-02 修复）。
    /// 与 `system_handlers.rs::start_key_rotation_task` 的模式一致（watch channel）。
    pub async fn stop_background_check(&self) {
        // 取出 sender 发送 shutdown 信号
        let sender_opt = self.shutdown_tx.lock().take();

        if let Some(tx) = sender_opt {
            let _ = tx.send(true);
            debug!(
                "{}",
                t!("log.core.algorithm.degradation_manager.background_check_shutdown_sent")
            );

            // 取出 handle 并等待 task 退出
            let handle_opt = self.background_task.lock().take();
            if let Some(handle) = handle_opt {
                let _ = handle.await;
            }
        }

        self.running.store(false, Ordering::SeqCst);
        info!(
            "{}",
            t!("log.core.algorithm.degradation_manager.background_check_stopped")
        );
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
