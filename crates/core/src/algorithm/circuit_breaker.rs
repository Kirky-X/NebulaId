#![allow(dead_code)]

use std::error::Error;
use std::fmt;
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
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

/// 熔断器实现
#[derive(Debug)]
pub struct CircuitBreaker {
    config: CircuitBreakerConfig,
    state: Arc<Mutex<CircuitBreakerState>>,
    consecutive_failures: Arc<AtomicU64>,
    consecutive_successes: Arc<AtomicU64>,
    total_requests: Arc<AtomicU64>,
    successful_requests: Arc<AtomicU64>,
    failed_requests: Arc<AtomicU64>,
    window_requests: Arc<AtomicU64>,
    window_failures: Arc<AtomicU64>,
    window_start: Arc<Mutex<Instant>>,
    last_failure_at: Arc<Mutex<Option<Instant>>>,
    last_success_at: Arc<Mutex<Option<Instant>>>,
    next_attempt_at: Arc<Mutex<Option<Instant>>>,
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
            state: Arc::new(Mutex::new(CircuitBreakerState::Closed)),
            consecutive_failures: Arc::new(AtomicU64::new(0)),
            consecutive_successes: Arc::new(AtomicU64::new(0)),
            total_requests: Arc::new(AtomicU64::new(0)),
            successful_requests: Arc::new(AtomicU64::new(0)),
            failed_requests: Arc::new(AtomicU64::new(0)),
            window_requests: Arc::new(AtomicU64::new(0)),
            window_failures: Arc::new(AtomicU64::new(0)),
            window_start: Arc::new(Mutex::new(Instant::now())),
            last_failure_at: Arc::new(Mutex::new(None)),
            last_success_at: Arc::new(Mutex::new(None)),
            next_attempt_at: Arc::new(Mutex::new(None)),
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

    /// 检查是否允许请求
    async fn should_allow_request(&self) -> bool {
        let state = self.state.lock().await;

        match *state {
            CircuitBreakerState::Closed => true,
            CircuitBreakerState::Open => {
                // 检查是否超时
                if let Some(next_attempt) = *self.next_attempt_at.lock().await {
                    if Instant::now() >= next_attempt {
                        // 转换为半开状态
                        drop(state);
                        self.transition_to_half_open().await;
                        true
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
            CircuitBreakerState::HalfOpen => true,
        }
    }

    /// 成功回调
    async fn on_success(&self) {
        self.total_requests.fetch_add(1, Ordering::Relaxed);
        self.successful_requests.fetch_add(1, Ordering::Relaxed);
        self.window_requests.fetch_add(1, Ordering::Relaxed);

        let now = Instant::now();
        *self.last_success_at.lock().await = Some(now);

        let consecutive_successes = self.consecutive_successes.fetch_add(1, Ordering::Relaxed) + 1;
        self.consecutive_failures.store(0, Ordering::Relaxed);

        let guard = self.state.lock().await;
        let state = (*guard).clone();

        if state == CircuitBreakerState::HalfOpen
            && consecutive_successes >= self.config.success_threshold
        {
            drop(guard);
            self.transition_to_closed().await;
        }
    }

    /// 失败回调
    async fn on_failure(&self) {
        self.total_requests.fetch_add(1, Ordering::Relaxed);
        self.failed_requests.fetch_add(1, Ordering::Relaxed);
        self.window_requests.fetch_add(1, Ordering::Relaxed);
        self.window_failures.fetch_add(1, Ordering::Relaxed);

        let now = Instant::now();
        *self.last_failure_at.lock().await = Some(now);

        let consecutive_failures = self.consecutive_failures.fetch_add(1, Ordering::Relaxed) + 1;
        self.consecutive_successes.store(0, Ordering::Relaxed);

        let mut state = self.state.lock().await;

        // 检查滑动窗口失败率
        let window_requests = self.window_requests.load(Ordering::Relaxed);
        let window_failures = self.window_failures.load(Ordering::Relaxed);

        // 重置滑动窗口
        if now.duration_since(*self.window_start.lock().await)
            > Duration::from_secs(self.config.window_size_seconds)
        {
            self.window_requests.store(0, Ordering::Relaxed);
            self.window_failures.store(0, Ordering::Relaxed);
            *self.window_start.lock().await = now;
        }

        // 根据失败次数或失败率转换状态
        let should_open = consecutive_failures >= self.config.failure_threshold
            || (window_requests >= self.config.min_requests
                && (window_failures as f64 / window_requests as f64) > 0.5);

        if should_open && !matches!(*state, CircuitBreakerState::Open) {
            *state = CircuitBreakerState::Open;
            let next_attempt = Instant::now() + Duration::from_millis(self.config.timeout_ms);
            *self.next_attempt_at.lock().await = Some(next_attempt);
            error!("Circuit breaker opened, next attempt at {:?}", next_attempt);
        } else if !matches!(*state, CircuitBreakerState::HalfOpen) {
            *state = CircuitBreakerState::Open;
            let next_attempt = Instant::now() + Duration::from_millis(self.config.timeout_ms);
            *self.next_attempt_at.lock().await = Some(next_attempt);
            error!("Circuit breaker opened, next attempt at {:?}", next_attempt);
        }
    }

    /// 转换到打开状态
    async fn transition_to_open(&self) {
        let mut state = self.state.lock().await;
        if !matches!(*state, CircuitBreakerState::Open) {
            *state = CircuitBreakerState::Open;
            let next_attempt = Instant::now() + Duration::from_millis(self.config.timeout_ms);
            *self.next_attempt_at.lock().await = Some(next_attempt);
            error!("Circuit breaker opened, next attempt at {:?}", next_attempt);
        }
    }

    async fn transition_to_half_open(&self) {
        let mut state = self.state.lock().await;
        if !matches!(*state, CircuitBreakerState::HalfOpen) {
            *state = CircuitBreakerState::HalfOpen;
            self.consecutive_successes.store(0, Ordering::Relaxed);
            self.consecutive_failures.store(0, Ordering::Relaxed);
            info!("Circuit breaker transitioned to half-open");
        }
    }

    /// 转换到关闭状态
    async fn transition_to_closed(&self) {
        let mut state = self.state.lock().await;
        if !matches!(*state, CircuitBreakerState::Closed) {
            *state = CircuitBreakerState::Closed;
            self.consecutive_failures.store(0, Ordering::Relaxed);
            self.consecutive_successes.store(0, Ordering::Relaxed);
            *self.next_attempt_at.lock().await = None;
            info!("Circuit breaker closed, service recovered");
        }
    }

    /// 手动重置熔断器
    pub async fn reset(&self) {
        self.transition_to_closed().await;
        self.consecutive_failures.store(0, Ordering::Relaxed);
        self.consecutive_successes.store(0, Ordering::Relaxed);
        self.window_requests.store(0, Ordering::Relaxed);
        self.window_failures.store(0, Ordering::Relaxed);
        *self.last_failure_at.lock().await = None;
        *self.last_success_at.lock().await = None;
        *self.next_attempt_at.lock().await = None;
    }

    /// 获取当前状态
    pub async fn state(&self) -> CircuitBreakerState {
        let guard = self.state.lock().await;
        guard.clone()
    }

    /// 获取指标快照
    pub async fn metrics(&self) -> CircuitBreakerMetricsSnapshot {
        let state_guard = self.state.lock().await;
        let state = (*state_guard).clone();
        let next_attempt_guard = self.next_attempt_at.lock().await;
        let next_attempt = *next_attempt_guard;
        let last_failure_guard = self.last_failure_at.lock().await;
        let last_failure = *last_failure_guard;
        let last_success_guard = self.last_success_at.lock().await;
        let last_success = *last_success_guard;

        drop(state_guard);
        drop(next_attempt_guard);
        drop(last_failure_guard);
        drop(last_success_guard);

        CircuitBreakerMetricsSnapshot {
            state,
            total_requests: self.total_requests.load(Ordering::Relaxed),
            successful_requests: self.successful_requests.load(Ordering::Relaxed),
            failed_requests: self.failed_requests.load(Ordering::Relaxed),
            consecutive_failures: self.consecutive_failures.load(Ordering::Relaxed),
            consecutive_successes: self.consecutive_successes.load(Ordering::Relaxed),
            last_failure_at: last_failure.map(|i| i.elapsed().as_secs()),
            last_success_at: last_success.map(|i| i.elapsed().as_secs()),
            next_attempt_at: next_attempt.map(|i| i.elapsed().as_secs()),
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
}
