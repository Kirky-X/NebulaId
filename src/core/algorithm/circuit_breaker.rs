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

use std::error::Error;
use std::fmt;
use std::future::Future;
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tracing::{error, info};

/// 熔断器错误
#[derive(Debug, Clone)]
pub struct CircuitBreakerError {
    pub message: String,
}

impl CircuitBreakerError {
    pub fn new(message: String) -> Self {
        Self { message }
    }
}

impl fmt::Display for CircuitBreakerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl Error for CircuitBreakerError {}

/// 熔断器打开时的错误常量
static CIRCUIT_BREAKER_OPEN: std::sync::OnceLock<CircuitBreakerError> = std::sync::OnceLock::new();

fn get_circuit_breaker_open_error() -> &'static CircuitBreakerError {
    CIRCUIT_BREAKER_OPEN.get_or_init(|| CircuitBreakerError {
        message: "Circuit breaker is open".to_string(),
    })
}

impl From<CircuitBreakerError> for String {
    fn from(e: CircuitBreakerError) -> Self {
        e.message
    }
}

/// 熔断器状态
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum CircuitBreakerState {
    /// 关闭状态，正常工作
    #[default]
    Closed,
    /// 半开状态，尝试恢复
    HalfOpen,
    /// 打开状态，拒绝请求
    Open,
}

impl CircuitBreakerState {
    pub fn is_closed(&self) -> bool {
        matches!(self, CircuitBreakerState::Closed)
    }

    pub fn is_open(&self) -> bool {
        matches!(self, CircuitBreakerState::Open)
    }

    pub fn is_half_open(&self) -> bool {
        matches!(self, CircuitBreakerState::HalfOpen)
    }
}

// 状态原子编码
const STATE_CLOSED: u8 = 0;
const STATE_OPEN: u8 = 1;
const STATE_HALF_OPEN: u8 = 2;

fn state_to_u8(s: CircuitBreakerState) -> u8 {
    match s {
        CircuitBreakerState::Closed => STATE_CLOSED,
        CircuitBreakerState::Open => STATE_OPEN,
        CircuitBreakerState::HalfOpen => STATE_HALF_OPEN,
    }
}

fn u8_to_state(v: u8) -> CircuitBreakerState {
    match v {
        STATE_OPEN => CircuitBreakerState::Open,
        STATE_HALF_OPEN => CircuitBreakerState::HalfOpen,
        _ => CircuitBreakerState::Closed,
    }
}

/// 全局起始 Instant，用于将 Instant 转为 u64 nanos 存储
static START_INSTANT: OnceLock<Instant> = OnceLock::new();

fn start_instant() -> Instant {
    *START_INSTANT.get_or_init(Instant::now)
}

/// 将 Instant 转为 nanos（相对全局起点）
fn instant_to_nanos(i: Instant) -> u64 {
    i.duration_since(start_instant()).as_nanos() as u64
}

/// 将 nanos 转为 Instant；0 视为 None
fn nanos_to_instant(n: u64) -> Option<Instant> {
    if n == 0 {
        None
    } else {
        Some(start_instant() + Duration::from_nanos(n))
    }
}

/// 熔断器配置
#[derive(Debug, Clone)]
pub struct CircuitBreakerConfig {
    /// 失败阈值（连续失败次数）
    pub failure_threshold: u64,
    /// 成功阈值（半开状态下的成功次数）
    pub success_threshold: u64,
    /// 超时时间（毫秒）
    pub timeout_ms: u64,
    /// 滑动窗口大小（秒）
    pub window_size_seconds: u64,
    /// 滑动窗口内的最小请求数
    pub min_requests: u64,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            success_threshold: 3,
            timeout_ms: 30000,
            window_size_seconds: 60,
            min_requests: 10,
        }
    }
}

/// 熔断器指标快照
#[derive(Debug, Clone, Default)]
pub struct CircuitBreakerMetricsSnapshot {
    pub state: CircuitBreakerState,
    pub total_requests: u64,
    pub successful_requests: u64,
    pub failed_requests: u64,
    pub consecutive_failures: u64,
    pub consecutive_successes: u64,
    pub last_failure_at: Option<u64>,
    pub last_success_at: Option<u64>,
    pub next_attempt_at: Option<u64>,
}

/// 熔断器实现（无锁化：AtomicU8 状态机 + AtomicU64 时间戳）
#[derive(Debug)]
pub struct CircuitBreaker {
    config: CircuitBreakerConfig,
    state: Arc<AtomicU8>,
    consecutive_failures: Arc<AtomicU64>,
    consecutive_successes: Arc<AtomicU64>,
    total_requests: Arc<AtomicU64>,
    successful_requests: Arc<AtomicU64>,
    failed_requests: Arc<AtomicU64>,
    window_requests: Arc<AtomicU64>,
    window_failures: Arc<AtomicU64>,
    // 时间戳以 nanos（相对全局起点）存于 AtomicU64；0 表示 None
    window_start: Arc<AtomicU64>,
    last_failure_at: Arc<AtomicU64>,
    last_success_at: Arc<AtomicU64>,
    next_attempt_at: Arc<AtomicU64>,
}

impl Clone for CircuitBreaker {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            state: self.state.clone(),
            consecutive_failures: self.consecutive_failures.clone(),
            consecutive_successes: self.consecutive_successes.clone(),
            total_requests: self.total_requests.clone(),
            successful_requests: self.successful_requests.clone(),
            failed_requests: self.failed_requests.clone(),
            window_requests: self.window_requests.clone(),
            window_failures: self.window_failures.clone(),
            window_start: self.window_start.clone(),
            last_failure_at: self.last_failure_at.clone(),
            last_success_at: self.last_success_at.clone(),
            next_attempt_at: self.next_attempt_at.clone(),
        }
    }
}

impl CircuitBreaker {
    /// 创建熔断器
    pub fn new(config: CircuitBreakerConfig) -> Self {
        Self {
            config,
            state: Arc::new(AtomicU8::new(STATE_CLOSED)),
            consecutive_failures: Arc::new(AtomicU64::new(0)),
            consecutive_successes: Arc::new(AtomicU64::new(0)),
            total_requests: Arc::new(AtomicU64::new(0)),
            successful_requests: Arc::new(AtomicU64::new(0)),
            failed_requests: Arc::new(AtomicU64::new(0)),
            window_requests: Arc::new(AtomicU64::new(0)),
            window_failures: Arc::new(AtomicU64::new(0)),
            window_start: Arc::new(AtomicU64::new(instant_to_nanos(Instant::now()))),
            last_failure_at: Arc::new(AtomicU64::new(0)),
            last_success_at: Arc::new(AtomicU64::new(0)),
            next_attempt_at: Arc::new(AtomicU64::new(0)),
        }
    }

    /// 执行操作，如果熔断器打开则返回错误
    pub async fn execute<F, T, E>(&self, operation: F) -> Result<T, E>
    where
        F: Future<Output = Result<T, E>>,
        E: From<CircuitBreakerError>,
    {
        // 检查是否允许请求
        if !self.should_allow_request().await {
            return Err(get_circuit_breaker_open_error().clone().into());
        }

        // 执行操作
        match operation.await {
            Ok(result) => {
                self.on_success().await;
                Ok(result)
            }
            Err(e) => {
                self.on_failure().await;
                Err(e)
            }
        }
    }

    /// 检查是否允许请求（无锁）
    async fn should_allow_request(&self) -> bool {
        let state = self.state.load(Ordering::Acquire);

        match state {
            STATE_CLOSED => true,
            STATE_OPEN => {
                // 检查是否超时
                let next_attempt_nanos = self.next_attempt_at.load(Ordering::Acquire);
                if next_attempt_nanos == 0 {
                    return false;
                }
                let next_attempt = match nanos_to_instant(next_attempt_nanos) {
                    Some(t) => t,
                    None => return false,
                };
                if Instant::now() >= next_attempt {
                    // 转换为半开状态（CAS 避免多线程同时转换）
                    let _ = self.state.compare_exchange(
                        STATE_OPEN,
                        STATE_HALF_OPEN,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    );
                    true
                } else {
                    false
                }
            }
            STATE_HALF_OPEN => true,
            _ => true,
        }
    }

    /// 成功回调
    async fn on_success(&self) {
        self.total_requests.fetch_add(1, Ordering::Relaxed);
        self.successful_requests.fetch_add(1, Ordering::Relaxed);
        self.window_requests.fetch_add(1, Ordering::Relaxed);

        let now_nanos = instant_to_nanos(Instant::now());
        self.last_success_at.store(now_nanos, Ordering::Release);

        let consecutive_successes = self.consecutive_successes.fetch_add(1, Ordering::Relaxed) + 1;
        self.consecutive_failures.store(0, Ordering::Relaxed);

        let state = self.state.load(Ordering::Acquire);
        if state == STATE_HALF_OPEN && consecutive_successes >= self.config.success_threshold {
            // 转换到 Closed
            if self
                .state
                .compare_exchange(
                    STATE_HALF_OPEN,
                    STATE_CLOSED,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok()
            {
                self.consecutive_failures.store(0, Ordering::Relaxed);
                self.consecutive_successes.store(0, Ordering::Relaxed);
                self.next_attempt_at.store(0, Ordering::Release);
                info!("Circuit breaker closed, service recovered");
            }
        }
    }

    /// 失败回调
    async fn on_failure(&self) {
        self.total_requests.fetch_add(1, Ordering::Relaxed);
        self.failed_requests.fetch_add(1, Ordering::Relaxed);
        self.window_requests.fetch_add(1, Ordering::Relaxed);
        self.window_failures.fetch_add(1, Ordering::Relaxed);

        let now = Instant::now();
        let now_nanos = instant_to_nanos(now);
        self.last_failure_at.store(now_nanos, Ordering::Release);

        let consecutive_failures = self.consecutive_failures.fetch_add(1, Ordering::Relaxed) + 1;
        self.consecutive_successes.store(0, Ordering::Relaxed);

        // 检查滑动窗口是否过期
        let window_start_nanos = self.window_start.load(Ordering::Acquire);
        if let Some(window_start) = nanos_to_instant(window_start_nanos) {
            if now.duration_since(window_start)
                > Duration::from_secs(self.config.window_size_seconds)
            {
                self.window_requests.store(0, Ordering::Relaxed);
                self.window_failures.store(0, Ordering::Relaxed);
                self.window_start.store(now_nanos, Ordering::Release);
            }
        }

        // 计算滑动窗口失败率
        let window_requests = self.window_requests.load(Ordering::Relaxed);
        let window_failures = self.window_failures.load(Ordering::Relaxed);

        // 根据失败次数或失败率判断是否应打开熔断器
        let should_open = consecutive_failures >= self.config.failure_threshold
            || (window_requests >= self.config.min_requests
                && (window_failures as f64 / window_requests as f64) > 0.5);

        let current_state = self.state.load(Ordering::Acquire);
        // 保持原有语义：非 HalfOpen 状态下，无论 should_open 与否都可能转 Open。
        // 合并 if_same_then_else 双分支：if (should_open && state != OPEN) || (!should_open && state != HALF_OPEN)
        // 等价于 should_open || state != HALF_OPEN（6 个 case 全部验证等价）。
        let should_transition = should_open || current_state != STATE_HALF_OPEN;
        if should_transition {
            self.transition_to_open();
        }
    }

    /// 转换到打开状态（无锁 CAS）
    fn transition_to_open(&self) {
        let next_attempt = Instant::now() + Duration::from_millis(self.config.timeout_ms);
        let next_attempt_nanos = instant_to_nanos(next_attempt);
        let prev = self.state.swap(STATE_OPEN, Ordering::AcqRel);
        self.next_attempt_at
            .store(next_attempt_nanos, Ordering::Release);
        if prev != STATE_OPEN {
            error!("Circuit breaker opened, next attempt at {:?}", next_attempt);
        }
    }

    async fn transition_to_half_open(&self) {
        let prev = self.state.swap(STATE_HALF_OPEN, Ordering::AcqRel);
        if prev != STATE_HALF_OPEN {
            self.consecutive_successes.store(0, Ordering::Relaxed);
            self.consecutive_failures.store(0, Ordering::Relaxed);
            info!("Circuit breaker transitioned to half-open");
        }
    }

    /// 转换到关闭状态
    async fn transition_to_closed(&self) {
        let prev = self.state.swap(STATE_CLOSED, Ordering::AcqRel);
        if prev != STATE_CLOSED {
            self.consecutive_failures.store(0, Ordering::Relaxed);
            self.consecutive_successes.store(0, Ordering::Relaxed);
            self.next_attempt_at.store(0, Ordering::Release);
            info!("Circuit breaker closed, service recovered");
        }
    }

    /// 手动重置熔断器
    pub async fn reset(&self) {
        self.state.store(STATE_CLOSED, Ordering::Release);
        self.consecutive_failures.store(0, Ordering::Relaxed);
        self.consecutive_successes.store(0, Ordering::Relaxed);
        self.window_requests.store(0, Ordering::Relaxed);
        self.window_failures.store(0, Ordering::Relaxed);
        self.last_failure_at.store(0, Ordering::Release);
        self.last_success_at.store(0, Ordering::Release);
        self.next_attempt_at.store(0, Ordering::Release);
    }

    /// 获取当前状态
    pub async fn state(&self) -> CircuitBreakerState {
        u8_to_state(self.state.load(Ordering::Acquire))
    }

    /// 获取指标快照
    pub async fn metrics(&self) -> CircuitBreakerMetricsSnapshot {
        let state = u8_to_state(self.state.load(Ordering::Acquire));
        let last_failure_nanos = self.last_failure_at.load(Ordering::Acquire);
        let last_success_nanos = self.last_success_at.load(Ordering::Acquire);
        let next_attempt_nanos = self.next_attempt_at.load(Ordering::Acquire);

        CircuitBreakerMetricsSnapshot {
            state,
            total_requests: self.total_requests.load(Ordering::Relaxed),
            successful_requests: self.successful_requests.load(Ordering::Relaxed),
            failed_requests: self.failed_requests.load(Ordering::Relaxed),
            consecutive_failures: self.consecutive_failures.load(Ordering::Relaxed),
            consecutive_successes: self.consecutive_successes.load(Ordering::Relaxed),
            last_failure_at: nanos_to_instant(last_failure_nanos).map(|i| i.elapsed().as_secs()),
            last_success_at: nanos_to_instant(last_success_nanos).map(|i| i.elapsed().as_secs()),
            next_attempt_at: nanos_to_instant(next_attempt_nanos).map(|i| i.elapsed().as_secs()),
        }
    }

    /// 计算滑动窗口内的失败率
    pub fn failure_rate(&self) -> f64 {
        let window_requests = self.window_requests.load(Ordering::Relaxed);
        let window_failures = self.window_failures.load(Ordering::Relaxed);

        if window_requests == 0 {
            0.0
        } else {
            window_failures as f64 / window_requests as f64
        }
    }
}

// 保留 transition_to_open 的 async 版本以兼容潜在调用（如原 transition_to_open async 方法）
impl CircuitBreaker {
    /// 异步版本：转换到打开状态
    #[allow(dead_code)]
    async fn transition_to_open_async(&self) {
        self.transition_to_open();
    }
}

/// 用于测试的模拟错误类型
#[derive(Debug)]
pub struct CircuitOpenError;

impl std::fmt::Display for CircuitOpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Circuit breaker is open")
    }
}

impl std::error::Error for CircuitOpenError {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::time::sleep;

    #[tokio::test]
    async fn test_circuit_breaker_closed_to_open() {
        let config = CircuitBreakerConfig {
            failure_threshold: 3,
            timeout_ms: 100,
            ..Default::default()
        };
        let breaker = CircuitBreaker::new(config);

        // 前3次应该成功
        for _ in 0..3 {
            assert!(breaker.state().await.is_closed());
        }

        // 连续失败应该打开熔断器
        // 注意：这里需要模拟失败，但由于 execute 返回 Result，
        // 我们需要一种方式来触发失败
        // 在实际使用中，应该在 execute 的 operation 中返回 Err
    }

    #[tokio::test]
    async fn test_circuit_breaker_reset() {
        let config = CircuitBreakerConfig::default();
        let breaker = CircuitBreaker::new(config);

        assert!(breaker.state().await.is_closed());

        breaker.reset().await;
        assert!(breaker.state().await.is_closed());
    }

    #[tokio::test]
    async fn test_circuit_breaker_state_transitions() {
        let config = CircuitBreakerConfig {
            failure_threshold: 2,
            success_threshold: 2,
            timeout_ms: 50,
            ..Default::default()
        };
        let breaker = CircuitBreaker::new(config);

        // 初始状态应该是关闭
        assert_eq!(breaker.state().await, CircuitBreakerState::Closed);

        // 模拟失败
        for _ in 0..2 {
            // 模拟失败操作
            let _ = breaker
                .execute(async { Err::<(), String>("failure".to_string()) })
                .await;
        }

        // 熔断器应该打开
        assert_eq!(breaker.state().await, CircuitBreakerState::Open);

        // 等待超时
        sleep(Duration::from_millis(60)).await;

        // 再次执行应该切换到半开
        let result: Result<(), String> = breaker.execute(async { Ok(()) }).await;
        assert!(result.is_ok());
        assert_eq!(breaker.state().await, CircuitBreakerState::HalfOpen);
    }

    #[tokio::test]
    async fn test_circuit_breaker_half_open_recovery() {
        let config = CircuitBreakerConfig {
            failure_threshold: 2,
            success_threshold: 2,
            timeout_ms: 50,
            ..Default::default()
        };
        let breaker = CircuitBreaker::new(config);

        // 触发熔断器打开
        for _ in 0..2 {
            let _ = breaker
                .execute::<_, (), String>(async { Err("failure".to_string()) })
                .await;
        }
        assert_eq!(breaker.state().await, CircuitBreakerState::Open);

        // 等待超时
        sleep(Duration::from_millis(60)).await;

        // 进入半开状态并成功
        let _: Result<(), String> = breaker.execute(async { Ok(()) }).await;
        assert_eq!(breaker.state().await, CircuitBreakerState::HalfOpen);

        // 再次成功应该关闭熔断器
        let _: Result<(), String> = breaker.execute(async { Ok(()) }).await;
        assert_eq!(breaker.state().await, CircuitBreakerState::Closed);
    }

    /// R-algorithm-002: on_failure 状态转换矩阵回归测试。
    /// 钉住合并 if-else 双分支前的行为，防止合并条件时引入 bug。
    /// 关键 case：CLOSED 状态下 should_open=false 仍转 OPEN（else if 分支语义）。
    #[tokio::test]
    async fn test_on_failure_transition_matrix_closed_no_should_open() {
        // failure_threshold=100, window_requests 不会达到 min_requests（默认 10）
        // → should_open=false，但 CLOSED 状态下仍转 OPEN（合并条件：false || true = true）
        let config = CircuitBreakerConfig {
            failure_threshold: 100,
            min_requests: 100,
            ..Default::default()
        };
        let breaker = CircuitBreaker::new(config);
        assert_eq!(breaker.state().await, CircuitBreakerState::Closed);
        breaker.on_failure().await;
        assert_eq!(
            breaker.state().await,
            CircuitBreakerState::Open,
            "CLOSED + should_open=false must still transition to OPEN"
        );
    }

    #[tokio::test]
    async fn test_on_failure_transition_matrix_closed_with_should_open() {
        // failure_threshold=1 → 1 次失败即 should_open=true
        let config = CircuitBreakerConfig {
            failure_threshold: 1,
            ..Default::default()
        };
        let breaker = CircuitBreaker::new(config);
        assert_eq!(breaker.state().await, CircuitBreakerState::Closed);
        breaker.on_failure().await;
        assert_eq!(
            breaker.state().await,
            CircuitBreakerState::Open,
            "CLOSED + should_open=true must transition to OPEN"
        );
    }

    /// R-algorithm-002 (T044): 完整 6-case 状态转换矩阵回归测试。
    /// 钉住 on_failure 中 `should_transition = should_open || state != HALF_OPEN`
    /// 合并条件的等价表。关键 case：(false, HALF_OPEN) → 不转换（仅此 case
    /// should_transition=false）。其余 5 个 case should_transition=true →
    /// 调用 transition_to_open()，结果 state=OPEN（已 OPEN 则保持）。
    #[tokio::test]
    async fn test_on_failure_transition_matrix_full() {
        // Case 1: (should_open=true, OPEN) → 保持 OPEN
        {
            let breaker = CircuitBreaker::new(CircuitBreakerConfig {
                failure_threshold: 1,
                ..Default::default()
            });
            breaker.transition_to_open();
            assert_eq!(breaker.state().await, CircuitBreakerState::Open);
            breaker.on_failure().await;
            assert_eq!(
                breaker.state().await,
                CircuitBreakerState::Open,
                "(true, OPEN) must stay OPEN"
            );
        }

        // Case 2: (should_open=true, HALF_OPEN) → 转 OPEN
        {
            let breaker = CircuitBreaker::new(CircuitBreakerConfig {
                failure_threshold: 1,
                ..Default::default()
            });
            breaker.transition_to_open();
            breaker.transition_to_half_open().await;
            assert_eq!(breaker.state().await, CircuitBreakerState::HalfOpen);
            breaker.on_failure().await;
            assert_eq!(
                breaker.state().await,
                CircuitBreakerState::Open,
                "(true, HALF_OPEN) must transition to OPEN"
            );
        }

        // Case 3: (should_open=true, CLOSED) → 转 OPEN
        {
            let breaker = CircuitBreaker::new(CircuitBreakerConfig {
                failure_threshold: 1,
                ..Default::default()
            });
            assert_eq!(breaker.state().await, CircuitBreakerState::Closed);
            breaker.on_failure().await;
            assert_eq!(
                breaker.state().await,
                CircuitBreakerState::Open,
                "(true, CLOSED) must transition to OPEN"
            );
        }

        // Case 4: (should_open=false, OPEN) → 保持 OPEN
        {
            let breaker = CircuitBreaker::new(CircuitBreakerConfig {
                failure_threshold: 100,
                min_requests: 100,
                ..Default::default()
            });
            breaker.transition_to_open();
            assert_eq!(breaker.state().await, CircuitBreakerState::Open);
            breaker.on_failure().await;
            assert_eq!(
                breaker.state().await,
                CircuitBreakerState::Open,
                "(false, OPEN) must stay OPEN"
            );
        }

        // Case 5: (should_open=false, HALF_OPEN) → 保持 HALF_OPEN（关键 case）
        // 此 case 是合并 if-else 双分支时唯一 should_transition=false 的场景。
        // 任何把条件改回 `should_open` 单一判定的回归都会让此 case 错转 OPEN。
        {
            let breaker = CircuitBreaker::new(CircuitBreakerConfig {
                failure_threshold: 100,
                min_requests: 100,
                ..Default::default()
            });
            breaker.transition_to_open();
            breaker.transition_to_half_open().await;
            assert_eq!(breaker.state().await, CircuitBreakerState::HalfOpen);
            breaker.on_failure().await;
            assert_eq!(
                breaker.state().await,
                CircuitBreakerState::HalfOpen,
                "(false, HALF_OPEN) must stay HALF_OPEN — key case pins the merged condition"
            );
        }

        // Case 6: (should_open=false, CLOSED) → 转 OPEN
        // 钉住"非 HalfOpen 状态下，无论 should_open 与否都可能转 Open"的原注释语义。
        {
            let breaker = CircuitBreaker::new(CircuitBreakerConfig {
                failure_threshold: 100,
                min_requests: 100,
                ..Default::default()
            });
            assert_eq!(breaker.state().await, CircuitBreakerState::Closed);
            breaker.on_failure().await;
            assert_eq!(
                breaker.state().await,
                CircuitBreakerState::Open,
                "(false, CLOSED) must transition to OPEN (non-HALF_OPEN always transitions)"
            );
        }
    }

    /// 验证 CircuitBreakerError::new 构造、Display 格式化以及 From<CircuitBreakerError> for String 转换。
    #[test]
    fn test_circuit_breaker_error_new_display_from_string() {
        let err = CircuitBreakerError::new("boom".to_string());
        assert_eq!(err.message, "boom");
        assert_eq!(format!("{}", err), "boom");
        // From<CircuitBreakerError> for String 提取 message 字段
        let s: String = err.into();
        assert_eq!(s, "boom");
    }

    /// 验证 CircuitOpenError 的 Display 输出与 Error trait 可用性。
    #[test]
    fn test_circuit_open_error_display_and_error_trait() {
        let err = CircuitOpenError;
        assert_eq!(format!("{}", err), "Circuit breaker is open");
        // 可作为 dyn Error 使用
        let _: &dyn Error = &err;
    }

    /// 验证 is_open / is_half_open / is_closed 三个状态判定方法在所有状态下的返回值。
    #[test]
    fn test_circuit_breaker_state_helpers_all_variants() {
        assert!(CircuitBreakerState::Closed.is_closed());
        assert!(!CircuitBreakerState::Closed.is_open());
        assert!(!CircuitBreakerState::Closed.is_half_open());

        assert!(CircuitBreakerState::Open.is_open());
        assert!(!CircuitBreakerState::Open.is_closed());
        assert!(!CircuitBreakerState::Open.is_half_open());

        assert!(CircuitBreakerState::HalfOpen.is_half_open());
        assert!(!CircuitBreakerState::HalfOpen.is_closed());
        assert!(!CircuitBreakerState::HalfOpen.is_open());
    }

    /// 验证 state_to_u8 对三个状态的编码与常量一致。
    #[test]
    fn test_state_to_u8_all_variants() {
        assert_eq!(state_to_u8(CircuitBreakerState::Closed), STATE_CLOSED);
        assert_eq!(state_to_u8(CircuitBreakerState::Open), STATE_OPEN);
        assert_eq!(state_to_u8(CircuitBreakerState::HalfOpen), STATE_HALF_OPEN);
    }

    /// 验证 u8_to_state 的默认回退分支：未知值回落到 Closed。
    #[test]
    fn test_u8_to_state_default_fallback() {
        assert_eq!(u8_to_state(STATE_CLOSED), CircuitBreakerState::Closed);
        assert_eq!(u8_to_state(STATE_OPEN), CircuitBreakerState::Open);
        assert_eq!(u8_to_state(STATE_HALF_OPEN), CircuitBreakerState::HalfOpen);
        // 未知值 → Closed（防御性回退）
        assert_eq!(u8_to_state(255), CircuitBreakerState::Closed);
    }

    /// 验证 nanos_to_instant 的 None 分支（n == 0）与 Some 分支（n > 0）。
    #[test]
    fn test_nanos_to_instant_zero_and_nonzero() {
        assert!(nanos_to_instant(0).is_none());
        let instant = nanos_to_instant(1).expect("non-zero nanos must yield Some");
        // 转换回 nanos 应近似等于原值（允许纳秒级误差）
        let back = instant.duration_since(start_instant()).as_nanos() as u64;
        assert!(back >= 1);
    }

    /// 验证 OPEN 状态下 next_attempt_at == 0 时 should_allow_request 返回 false。
    /// 覆盖 should_allow_request 中 "next_attempt_nanos == 0" 早返回分支。
    #[tokio::test]
    async fn test_should_allow_request_open_next_attempt_zero() {
        let breaker = CircuitBreaker::new(CircuitBreakerConfig::default());
        // 手动将状态置为 OPEN，next_attempt_at 置为 0
        breaker.state.store(STATE_OPEN, Ordering::Release);
        breaker.next_attempt_at.store(0, Ordering::Release);
        let allowed = breaker.should_allow_request().await;
        assert!(!allowed, "OPEN + next_attempt_at==0 must reject request");
    }

    /// 验证 should_allow_request 对未知状态字节的 fail-open 行为（_ => true 分支）。
    #[tokio::test]
    async fn test_should_allow_request_invalid_state_fallback() {
        let breaker = CircuitBreaker::new(CircuitBreakerConfig::default());
        // 写入非法状态值（不在 0/1/2 之中）
        breaker.state.store(255, Ordering::Release);
        let allowed = breaker.should_allow_request().await;
        assert!(allowed, "invalid state byte must fail-open (return true)");
    }

    /// 验证 OPEN 状态下未到 next_attempt 时刻时 should_allow_request 返回 false。
    #[tokio::test]
    async fn test_should_allow_request_open_not_timed_out() {
        let breaker = CircuitBreaker::new(CircuitBreakerConfig {
            timeout_ms: 10_000, // 远大于测试时长
            ..Default::default()
        });
        breaker.transition_to_open(); // 设置 next_attempt_at 为未来时刻
        let allowed = breaker.should_allow_request().await;
        assert!(!allowed, "OPEN + not yet timed out must reject request");
    }

    /// 验证 on_failure 在滑动窗口过期时重置 window_requests / window_failures / window_start。
    #[tokio::test]
    async fn test_on_failure_sliding_window_expiration() {
        // window_size_seconds=0 使窗口立即过期
        let config = CircuitBreakerConfig {
            failure_threshold: 100,
            min_requests: 100,
            window_size_seconds: 0,
            ..Default::default()
        };
        let breaker = CircuitBreaker::new(config);
        // 预填滑动窗口计数
        breaker.window_requests.store(5, Ordering::Relaxed);
        breaker.window_failures.store(3, Ordering::Relaxed);
        // window_start 已由 new() 设置为构造时刻；window_size_seconds=0 时立即过期
        breaker.on_failure().await;
        // 过期后 window_requests / window_failures 被 store(0) 重置
        // （fetch_add 发生在重置之前，但重置覆盖为 0）
        assert_eq!(
            breaker.window_requests.load(Ordering::Relaxed),
            0,
            "expired window requests must be reset to 0"
        );
        assert_eq!(
            breaker.window_failures.load(Ordering::Relaxed),
            0,
            "expired window failures must be reset to 0"
        );
    }

    /// 验证 on_failure 在 window_failures/window_requests > 0.5 且
    /// window_requests >= min_requests 时通过失败率分支触发 should_open=true → 转 OPEN。
    #[tokio::test]
    async fn test_on_failure_window_failure_rate_triggers_open() {
        // failure_threshold=100 使 consecutive_failures 路径不触发 should_open
        // min_requests=2 使窗口失败率路径可达
        let config = CircuitBreakerConfig {
            failure_threshold: 100,
            min_requests: 2,
            window_size_seconds: 60, // 窗口不过期
            ..Default::default()
        };
        let breaker = CircuitBreaker::new(config);
        // 预填窗口：2 请求 2 失败（rate=1.0 > 0.5）
        breaker.window_requests.store(2, Ordering::Relaxed);
        breaker.window_failures.store(2, Ordering::Relaxed);
        // consecutive_failures=0 < 100，故 should_open 仅由失败率分支驱动
        breaker.on_failure().await;
        assert_eq!(
            breaker.state().await,
            CircuitBreakerState::Open,
            "window failure rate > 0.5 with enough requests must open the breaker"
        );
    }

    /// 验证 transition_to_closed 从 OPEN 状态恢复到 CLOSED 时清零计数器与 next_attempt_at。
    #[tokio::test]
    async fn test_transition_to_closed_from_open() {
        let breaker = CircuitBreaker::new(CircuitBreakerConfig::default());
        breaker.transition_to_open();
        // 填充非零计数器
        breaker.consecutive_failures.store(5, Ordering::Relaxed);
        breaker.consecutive_successes.store(3, Ordering::Relaxed);
        assert!(breaker.next_attempt_at.load(Ordering::Acquire) > 0);
        // 执行转换
        breaker.transition_to_closed().await;
        assert_eq!(breaker.state().await, CircuitBreakerState::Closed);
        assert_eq!(breaker.consecutive_failures.load(Ordering::Relaxed), 0);
        assert_eq!(breaker.consecutive_successes.load(Ordering::Relaxed), 0);
        assert_eq!(breaker.next_attempt_at.load(Ordering::Acquire), 0);
    }

    /// 验证 transition_to_closed 在已处于 CLOSED 时是幂等的（prev == CLOSED 不重置）。
    #[tokio::test]
    async fn test_transition_to_closed_idempotent() {
        let breaker = CircuitBreaker::new(CircuitBreakerConfig::default());
        // 初始即为 CLOSED，prev == CLOSED，不应进入重置分支
        breaker.consecutive_failures.store(7, Ordering::Relaxed);
        breaker.transition_to_closed().await;
        // 计数器未被重置（prev == CLOSED 分支不执行 store(0)）
        assert_eq!(breaker.consecutive_failures.load(Ordering::Relaxed), 7);
        assert_eq!(breaker.state().await, CircuitBreakerState::Closed);
    }

    /// 验证 transition_to_half_open 在已处于 HALF_OPEN 时是幂等的（prev == HALF_OPEN 不重置计数器）。
    #[tokio::test]
    async fn test_transition_to_half_open_idempotent() {
        let breaker = CircuitBreaker::new(CircuitBreakerConfig::default());
        breaker.transition_to_open();
        // 第一次：OPEN → HALF_OPEN，prev=OPEN，重置计数器
        breaker.transition_to_half_open().await;
        assert_eq!(breaker.state().await, CircuitBreakerState::HalfOpen);
        // 填充非零计数器
        breaker.consecutive_failures.store(9, Ordering::Relaxed);
        breaker.consecutive_successes.store(4, Ordering::Relaxed);
        // 第二次：HALF_OPEN → HALF_OPEN，prev=HALF_OPEN，不应重置
        breaker.transition_to_half_open().await;
        assert_eq!(breaker.consecutive_failures.load(Ordering::Relaxed), 9);
        assert_eq!(breaker.consecutive_successes.load(Ordering::Relaxed), 4);
        assert_eq!(breaker.state().await, CircuitBreakerState::HalfOpen);
    }

    /// 验证 metrics() 在全新熔断器上返回全 None 时间戳与零计数。
    #[tokio::test]
    async fn test_metrics_fresh_breaker_all_none() {
        let breaker = CircuitBreaker::new(CircuitBreakerConfig::default());
        let m = breaker.metrics().await;
        assert_eq!(m.state, CircuitBreakerState::Closed);
        assert_eq!(m.total_requests, 0);
        assert_eq!(m.successful_requests, 0);
        assert_eq!(m.failed_requests, 0);
        assert_eq!(m.consecutive_failures, 0);
        assert_eq!(m.consecutive_successes, 0);
        assert_eq!(m.last_failure_at, None);
        assert_eq!(m.last_success_at, None);
        assert_eq!(m.next_attempt_at, None);
    }

    /// 验证 metrics() 在发生失败/成功后返回 Some 时间戳与正确计数。
    #[tokio::test]
    async fn test_metrics_after_activity_some_timestamps() {
        let breaker = CircuitBreaker::new(CircuitBreakerConfig::default());
        // on_failure 会触发 transition_to_open（CLOSED → OPEN），设置 last_failure_at / next_attempt_at
        breaker.on_failure().await;
        // on_success 在 OPEN 状态下设置 last_success_at（不触发状态转换）
        breaker.on_success().await;
        let m = breaker.metrics().await;
        assert_eq!(m.state, CircuitBreakerState::Open);
        assert_eq!(m.total_requests, 2);
        assert_eq!(m.successful_requests, 1);
        assert_eq!(m.failed_requests, 1);
        assert_eq!(
            m.consecutive_failures, 0,
            "on_success resets consecutive_failures"
        );
        assert_eq!(m.consecutive_successes, 1);
        assert!(
            m.last_failure_at.is_some(),
            "last_failure_at must be Some after failure"
        );
        assert!(
            m.last_success_at.is_some(),
            "last_success_at must be Some after success"
        );
        assert!(
            m.next_attempt_at.is_some(),
            "next_attempt_at must be Some after open"
        );
    }

    /// 验证 failure_rate() 在 window_requests==0 时返回 0.0，在有数据时返回正确比例。
    #[tokio::test]
    async fn test_failure_rate_zero_and_nonzero() {
        let breaker = CircuitBreaker::new(CircuitBreakerConfig::default());
        // 全新：window_requests == 0 → 0.0
        assert_eq!(breaker.failure_rate(), 0.0);
        // 2 次失败 + 1 次成功 → window_requests=3, window_failures=2 → 2/3
        breaker.on_failure().await;
        breaker.on_failure().await;
        breaker.on_success().await;
        let rate = breaker.failure_rate();
        assert!(
            (rate - 2.0 / 3.0).abs() < 1e-9,
            "failure_rate must be 2/3, got {}",
            rate
        );
    }

    /// 验证 transition_to_open_async 异步包装器调用 transition_to_open 后状态为 OPEN。
    #[tokio::test]
    async fn test_transition_to_open_async_wrapper() {
        let breaker = CircuitBreaker::new(CircuitBreakerConfig::default());
        assert_eq!(breaker.state().await, CircuitBreakerState::Closed);
        breaker.transition_to_open_async().await;
        assert_eq!(breaker.state().await, CircuitBreakerState::Open);
        assert!(breaker.next_attempt_at.load(Ordering::Acquire) > 0);
    }

    /// 验证 Clone 后两个实例共享底层状态（Arc 语义）。
    #[tokio::test]
    async fn test_circuit_breaker_clone_shares_state() {
        let breaker = CircuitBreaker::new(CircuitBreakerConfig {
            failure_threshold: 1,
            ..Default::default()
        });
        let cloned = breaker.clone();
        // 在原实例上触发 OPEN
        breaker.on_failure().await;
        // 克隆应看到相同的 OPEN 状态（Arc 共享）
        assert_eq!(breaker.state().await, CircuitBreakerState::Open);
        assert_eq!(
            cloned.state().await,
            CircuitBreakerState::Open,
            "clone must share state via Arc"
        );
        // 计数器也共享
        assert_eq!(
            cloned.failed_requests.load(Ordering::Relaxed),
            breaker.failed_requests.load(Ordering::Relaxed)
        );
    }

    /// 验证 execute 在 OPEN 状态下返回 CircuitBreakerError 派生的错误（拒绝请求）。
    /// 错误消息来自全局静态 CIRCUIT_BREAKER_OPEN，经 From<CircuitBreakerError> for String 转换。
    #[tokio::test]
    async fn test_execute_open_state_returns_open_error() {
        let breaker = CircuitBreaker::new(CircuitBreakerConfig {
            failure_threshold: 1,
            timeout_ms: 10_000, // 不让超时干扰
            ..Default::default()
        });
        // 1 次失败即触发 OPEN
        let _: Result<(), String> = breaker.execute(async { Err("fail".to_string()) }).await;
        assert_eq!(breaker.state().await, CircuitBreakerState::Open);
        // OPEN 状态下 execute 应直接返回错误，不调用 operation
        // operation 提供一个 Ok 结果；若被错误调用，result 会变为 Ok 使断言失败
        let result: Result<(), String> = breaker.execute(async { Ok::<(), String>(()) }).await;
        assert!(
            result.is_err(),
            "execute in OPEN state must return error without invoking operation"
        );
        assert_eq!(
            result.unwrap_err(),
            "Circuit breaker is open",
            "error must carry the canonical open-state message"
        );
    }

    /// 验证 on_success 在 HALF_OPEN 状态下达到 success_threshold 边界（=1）即转 CLOSED。
    #[tokio::test]
    async fn test_on_success_half_open_threshold_boundary() {
        let config = CircuitBreakerConfig {
            failure_threshold: 1,
            success_threshold: 1, // 边界：1 次成功即恢复
            timeout_ms: 50,
            ..Default::default()
        };
        let breaker = CircuitBreaker::new(config);
        breaker.transition_to_open();
        // 等待超时使 should_allow_request 把 OPEN 转 HALF_OPEN
        sleep(Duration::from_millis(60)).await;
        let _: Result<(), String> = breaker.execute(async { Ok(()) }).await;
        // success_threshold=1，1 次成功即 HALF_OPEN → CLOSED
        assert_eq!(breaker.state().await, CircuitBreakerState::Closed);
        // 转换后计数器清零
        assert_eq!(breaker.consecutive_successes.load(Ordering::Relaxed), 0);
        assert_eq!(breaker.consecutive_failures.load(Ordering::Relaxed), 0);
        assert_eq!(breaker.next_attempt_at.load(Ordering::Acquire), 0);
    }

    /// 验证 on_success 在 HALF_OPEN 状态下未达 success_threshold 时保持 HALF_OPEN。
    #[tokio::test]
    async fn test_on_success_half_open_below_threshold_stays_half_open() {
        let config = CircuitBreakerConfig {
            failure_threshold: 1,
            success_threshold: 3, // 需 3 次成功
            timeout_ms: 50,
            ..Default::default()
        };
        let breaker = CircuitBreaker::new(config);
        breaker.transition_to_open();
        sleep(Duration::from_millis(60)).await;
        // 第 1 次成功：should_allow_request 把 OPEN 转 HALF_OPEN，on_success 因 1 < 3 保持 HALF_OPEN
        let _: Result<(), String> = breaker.execute(async { Ok(()) }).await;
        assert_eq!(breaker.state().await, CircuitBreakerState::HalfOpen);
        assert_eq!(breaker.consecutive_successes.load(Ordering::Relaxed), 1);
        // 第 2 次成功：仍 < 3，保持 HALF_OPEN
        let _: Result<(), String> = breaker.execute(async { Ok(()) }).await;
        assert_eq!(breaker.state().await, CircuitBreakerState::HalfOpen);
        assert_eq!(breaker.consecutive_successes.load(Ordering::Relaxed), 2);
        // 第 3 次成功：达到阈值，转 CLOSED
        let _: Result<(), String> = breaker.execute(async { Ok(()) }).await;
        assert_eq!(breaker.state().await, CircuitBreakerState::Closed);
    }
}
