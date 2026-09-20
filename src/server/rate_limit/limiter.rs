// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Rate limiting module powered by limiteron library.
//!
//! This module provides unified rate limiting, concurrency control, and quota management
//! using the limiteron library's robust implementations.
//!
//! # Key Components
//!
//! - [`RateLimiter`]: Main rate limiter using Token Bucket algorithm
//! - [`ConcurrencyLimiter`]: Controls maximum concurrent operations
//!
//! # Features
//!
//! - Token bucket algorithm for smooth rate limiting
//! - Thread-safe and async-native design
//! - Support for per-key rate limits
//! - Built-in concurrency control

use limiteron::error::LimiteronError;
use limiteron::limiters::{
    ConcurrencyLimiter as LimiteronConcurrencyLimiter, Limiter as LimiteronLimiter,
    TokenBucketLimiter,
};
use parking_lot::RwLock;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::time::interval;
use tracing::debug;

/// 限流表分片数（2 的幂）。按 key 哈希分摊读写热点：单把全局
/// `RwLock<HashMap>` 在多核高 QPS 下会让所有 check 在同一写锁上串行
/// （去热点）。依赖选型：`dashmap` 不在 Cargo.toml（本任务禁改
/// 依赖），采用 `parking_lot::RwLock<HashMap>` 分片等价实现。
const LIMITER_SHARDS: usize = 16;

/// `last_accessed` 的时钟精度（Unix 纳秒，u64 可容纳至 2554 年）。原子量
/// 替代 `Arc<RwLock<Instant>>`（每次 touch 一次写锁 → 一次 store，清理路径
/// 读 last_accessed 也不再进锁）。
///
/// 选 `SystemTime` 而非 `Instant`：原子量只能存整数，`Instant` 无稳定
/// 基准可换算；墙钟回拨在极端情况下最多让空闲桶多存活一个回拨窗口，
/// 对分钟级 max_idle 的清理语义无实际影响。纳秒精度保住原 `Instant`
/// 实现的亚毫秒差判定（`cleanup(Duration::ZERO)` 立即清除刚建桶）。
fn unix_nanos() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

/// Rate limit result containing the decision and metadata
#[derive(Debug, Clone)]
pub struct RateLimitResult {
    /// Whether the request is allowed
    pub allowed: bool,
    /// Remaining requests in the current window
    pub remaining: u64,
    /// Maximum requests allowed per window
    pub limit: u32,
    /// Seconds to wait before retrying (if rate limited)
    pub retry_after: Option<u64>,
}

/// Rate limit status for monitoring
#[derive(Debug, Clone)]
pub struct RateLimitStatus {
    /// Remaining requests in the current window
    pub remaining: u64,
    /// Maximum requests allowed per window
    pub limit: u32,
    /// Requests per second
    pub rate: u32,
}

/// Internal rate limiter wrapper using limiteron's TokenBucketLimiter
#[derive(Clone)]
struct InternalRateLimiter {
    limiter: Arc<TokenBucketLimiter>,
    rate: u32,
    capacity: u32,
    /// 最后访问时刻（Unix 纳秒，原子量）。语义与原 `Arc<RwLock<Instant>>`
    /// 一致：仅用于空闲清理判定。
    last_accessed: Arc<AtomicU64>,
}

impl InternalRateLimiter {
    /// Creates a new internal rate limiter
    ///
    /// # Arguments
    /// * `rate` - Requests per second (refill rate)
    /// * `capacity` - Burst capacity (bucket size)
    fn new(rate: u32, capacity: u32) -> Self {
        Self {
            limiter: Arc::new(TokenBucketLimiter::new(capacity as u64, rate as u64)),
            rate,
            capacity,
            last_accessed: Arc::new(AtomicU64::new(unix_nanos())),
        }
    }

    /// Update the last accessed time
    fn touch(&self) {
        self.last_accessed.store(unix_nanos(), Ordering::Relaxed);
    }

    /// Get the last accessed time (Unix 纳秒)
    fn last_accessed_nanos(&self) -> u64 {
        self.last_accessed.load(Ordering::Relaxed)
    }

    /// Check if a request is allowed and consume a token
    async fn check(&self) -> Result<RateLimitResult, LimiteronError> {
        // Update last accessed time
        self.touch();

        let allowed = self.limiter.allow(1).await?;

        // 消费后经 limiteron 标准快照 API 读取限流头数据（limit/remaining/
        // reset_secs），与 allow 共用同一补充逻辑；reset_secs 即补满所需秒数，
        // 被拒时作为 Retry-After（此前硬编码 1 秒，慢补充大容量桶会过早放行）。
        let snapshot = self.limiter.remaining().await?;
        let retry_after = if allowed {
            None
        } else {
            Some(snapshot.reset_secs.max(1))
        };

        Ok(RateLimitResult {
            allowed,
            remaining: snapshot.remaining,
            limit: self.capacity,
            retry_after,
        })
    }
}

/// 分片限流表：`LIMITER_SHARDS` 个 `RwLock<HashMap>` 按 key 哈希分布，
/// 语义与原单把 `RwLock<HashMap<String, InternalRateLimiter>>` 等价
/// （get-or-insert / get / len / 逐出 / 整表换新），仅锁粒度按分片细化。
struct ShardedLimiters {
    shards: Vec<RwLock<HashMap<String, InternalRateLimiter>>>,
}

impl ShardedLimiters {
    fn new() -> Self {
        Self {
            shards: (0..LIMITER_SHARDS)
                .map(|_| RwLock::new(HashMap::new()))
                .collect(),
        }
    }

    /// 定位 key 所属分片。`DefaultHasher` 每次 new() 种子固定（std 实现），
    /// 进程内分布稳定即可，无需跨进程稳定。
    fn shard_for(&self, key: &str) -> &RwLock<HashMap<String, InternalRateLimiter>> {
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        let idx = (hasher.finish() % LIMITER_SHARDS as u64) as usize;
        &self.shards[idx]
    }

    /// 遍历所有分片执行 `f`，收集各分片返回值。
    fn for_each_shard<T>(
        &self,
        f: impl FnMut(&RwLock<HashMap<String, InternalRateLimiter>>) -> T,
    ) -> Vec<T> {
        self.shards.iter().map(f).collect()
    }
}

/// Main rate limiter for the application.
///
/// Uses limiteron's TokenBucketLimiter for smooth, accurate rate limiting
/// with per-key tracking.
#[derive(Clone)]
pub struct RateLimiter {
    limiters: Arc<ShardedLimiters>,
    defaults: Arc<RwLock<(u32, u32)>>,
    cleanup_interval: Arc<RwLock<Duration>>,
}

impl RateLimiter {
    /// Creates a new rate limiter with default settings.
    ///
    /// # Arguments
    /// * `default_rps` - Default requests per second (refill rate)
    /// * `default_burst` - Burst capacity (bucket size)
    ///
    /// # Example
    /// ```ignore
    /// use nebulaid::server::rate_limit::RateLimiter;
    /// let limiter = RateLimiter::new(1000, 100);
    /// ```
    pub fn new(default_rps: u32, default_burst: u32) -> Self {
        Self {
            limiters: Arc::new(ShardedLimiters::new()),
            defaults: Arc::new(RwLock::new((default_rps, default_burst))),
            cleanup_interval: Arc::new(RwLock::new(Duration::from_secs(300))), // 5 minutes default
        }
    }

    /// Start background cleanup task to remove expired rate limit entries.
    ///
    /// This task runs periodically and removes limiters that haven't been used
    /// for longer than the specified idle duration.
    ///
    /// # Arguments
    /// * `max_idle` - Maximum idle duration before a limiter is removed
    /// * `cleanup_interval` - How often to run cleanup
    ///
    /// # Returns
    /// A join handle that can be used to await the cleanup task
    pub fn start_cleanup(
        &self,
        max_idle: Duration,
        cleanup_interval: Duration,
    ) -> tokio::task::JoinHandle<()> {
        // Update cleanup interval
        *self.cleanup_interval.write() = cleanup_interval;

        let limiters = self.limiters.clone();
        let max_idle_nanos = max_idle.as_nanos() as u64;

        tokio::spawn(async move {
            let mut interval_timer = interval(cleanup_interval);
            loop {
                interval_timer.tick().await;

                let now_nanos = unix_nanos();
                // 锁内只做「收集 key + 从分片摘除」：摘下的桶（内含
                // TokenBucketLimiter 状态）先移出各分片锁作用域，统一在锁外
                // 释放，避免 drop 成本随待删桶数量线性放大分片写锁。
                let (removed_count, removed_buckets) = {
                    let per_shard = limiters.for_each_shard(|shard| {
                        let mut shard_guard = shard.write();
                        let keys_to_remove: Vec<String> = shard_guard
                            .iter()
                            .filter(|(_, limiter)| {
                                now_nanos.saturating_sub(limiter.last_accessed_nanos())
                                    > max_idle_nanos
                            })
                            .map(|(key, _)| key.clone())
                            .collect();

                        let buckets = keys_to_remove
                            .into_iter()
                            .filter_map(|key| shard_guard.remove(&key))
                            .collect::<Vec<_>>();
                        (buckets.len(), buckets)
                    });
                    let count: usize = per_shard.iter().map(|(n, _)| *n).sum();
                    let buckets = per_shard
                        .into_iter()
                        .flat_map(|(_, buckets)| buckets)
                        .collect::<Vec<_>>();
                    (count, buckets)
                };

                if removed_count > 0 {
                    debug!(
                        event = "rate_limiter_cleanup",
                        removed_count = removed_count,
                        "{}",
                        t!(
                            "log.server.rate_limit.limiter.cleaned_up_expired_limiters",
                            removed_count = removed_count
                        )
                    );
                }

                drop(removed_buckets);
            }
        })
    }

    /// Check rate limit for a specific key.
    ///
    /// # Arguments
    /// * `key` - The identifier to rate limit (e.g., IP, user ID, API key)
    /// * `custom_rate` - Optional custom rate limit for this key (only used on first request)
    /// * `custom_burst` - Optional custom burst limit for this key (only used on first request)
    ///
    /// # Returns
    /// Returns `RateLimitResult` indicating if the request is allowed.
    pub async fn check_rate_limit(
        &self,
        key: &str,
        custom_rate: Option<u32>,
        custom_burst: Option<u32>,
    ) -> RateLimitResult {
        let (default_rps, default_burst) = *self.defaults.read();

        // Determine the rate and capacity to use
        let (rate, capacity) = if custom_rate.is_some() || custom_burst.is_some() {
            (
                custom_rate.unwrap_or(default_rps),
                custom_burst.unwrap_or(default_burst),
            )
        } else {
            (default_rps, default_burst)
        };

        // Get or create the limiter for this key（写锁粒度=key 所属分片）
        let limiter = {
            let mut shard = self.limiters.shard_for(key).write();
            shard
                .entry(key.to_string())
                .or_insert_with(|| InternalRateLimiter::new(rate, capacity))
                .clone()
        };

        match limiter.check().await {
            Ok(result) => result,
            Err(e) => {
                tracing::warn!(
                    error = ?e,
                    "{}",
                    t!("log.server.rate_limit.limiter.rate_limit_check_error")
                );
                // Fail open - allow request if rate limiter has an error
                RateLimitResult {
                    allowed: true,
                    remaining: default_burst as u64,
                    limit: default_burst,
                    retry_after: None,
                }
            }
        }
    }

    /// Get the current rate limit status for a key.
    pub fn get_usage(&self, key: &str) -> Option<RateLimitStatus> {
        let shard = self.limiters.shard_for(key).read();
        shard.get(key).map(|entry| RateLimitStatus {
            remaining: entry.limiter.tokens(),
            limit: entry.capacity,
            rate: entry.rate,
        })
    }

    /// Get the current number of rate limit buckets.
    pub fn bucket_count(&self) -> usize {
        self.limiters
            .for_each_shard(|shard| shard.read().len())
            .into_iter()
            .sum()
    }

    /// Cleanup expired rate limit entries.
    ///
    /// This method removes limiters that haven't been accessed for longer than
    /// the specified max_idle duration.
    ///
    /// # Arguments
    /// * `max_idle` - Maximum idle duration before a limiter is removed
    ///
    /// # Returns
    /// Number of limiters removed
    pub fn cleanup(&self, max_idle: Duration) -> usize {
        let now_nanos = unix_nanos();
        let max_idle_nanos = max_idle.as_nanos() as u64;

        // 锁内只做「收集 key + 从分片摘除」；摘下的桶带出各分片锁作用域后才
        // 释放，使分片写锁的持有时长与 drop 成本解耦（与后台清理循环同一边界）。
        let per_shard = self.limiters.for_each_shard(|shard| {
            let mut shard_guard = shard.write();
            let keys_to_remove: Vec<String> = shard_guard
                .iter()
                .filter(|(_, limiter)| {
                    now_nanos.saturating_sub(limiter.last_accessed_nanos()) > max_idle_nanos
                })
                .map(|(key, _)| key.clone())
                .collect();

            let buckets = keys_to_remove
                .into_iter()
                .filter_map(|key| shard_guard.remove(&key))
                .collect::<Vec<_>>();
            (buckets.len(), buckets)
        });

        let removed_count: usize = per_shard.iter().map(|(n, _)| *n).sum();

        if removed_count > 0 {
            debug!(
                event = "rate_limiter_cleanup",
                removed_count = removed_count,
                "{}",
                t!(
                    "log.server.rate_limit.limiter.manually_cleaned_up_expired_limiters",
                    removed_count = removed_count
                )
            );
        }

        // 桶在所有分片锁外释放
        let removed_buckets: Vec<InternalRateLimiter> = per_shard
            .into_iter()
            .flat_map(|(_, buckets)| buckets)
            .collect();
        drop(removed_buckets);
        removed_count
    }

    /// Get the current number of active rate limiters.
    pub fn active_limiters_count(&self) -> usize {
        self.limiters
            .for_each_shard(|shard| shard.read().len())
            .into_iter()
            .sum()
    }

    /// Get memory usage statistics for monitoring.
    pub fn memory_stats(&self) -> RateLimiterMemoryStats {
        let active_limiters = self
            .limiters
            .for_each_shard(|shard| shard.read().len())
            .into_iter()
            .sum();
        RateLimiterMemoryStats {
            active_limiters,
            default_rps: self.defaults.read().0,
            default_burst: self.defaults.read().1,
        }
    }

    /// Update the default rate limit settings.
    pub fn update_defaults(&self, default_rps: u32, default_burst: u32) {
        let mut defaults = self.defaults.write();
        *defaults = (default_rps, default_burst);
    }

    /// Update the default rate limit configuration at runtime.
    ///
    /// Unlike [`Self::update_defaults`], this also drops existing buckets so
    /// they are lazily rebuilt with the new configuration on the next
    /// `check_rate_limit` call — otherwise pre-existing keys keep their old
    /// rate/capacity forever and hot updates never take effect on traffic.
    /// Dropping buckets resets their token state (one-time grace window
    /// immediately after an update); documented and accepted trade-off.
    pub fn update_config(&self, default_rps: u32, default_burst: u32) {
        self.update_defaults(default_rps, default_burst);

        // 各分片锁内只做一次 O(1) 换表；旧表连同其全部桶（TokenBucketLimiter
        // 状态、last_accessed 原子量）在所有分片锁外释放。此前用
        // `write().clear()`，逐个 drop 发生在持锁期间，锁持有时间随桶数量
        // 线性放大。
        let old_buckets: Vec<HashMap<String, InternalRateLimiter>> =
            self.limiters.for_each_shard(|shard| {
                let mut shard_guard = shard.write();
                std::mem::take(&mut *shard_guard)
            });

        drop(old_buckets);
    }

    /// Get the limiteron concurrency limiter for internal use.
    pub fn get_concurrency_limiter(max_concurrent: u64) -> ConcurrencyLimiter {
        ConcurrencyLimiter::new(max_concurrent)
    }
}

/// Memory usage statistics for rate limiter monitoring
#[derive(Debug, Clone)]
pub struct RateLimiterMemoryStats {
    /// Number of active rate limit buckets
    pub active_limiters: usize,
    /// Default requests per second
    pub default_rps: u32,
    /// Default burst capacity
    pub default_burst: u32,
}

/// Concurrency limiter wrapper using limiteron's ConcurrencyLimiter.
#[derive(Clone)]
pub struct ConcurrencyLimiter {
    inner: Arc<LimiteronConcurrencyLimiter>,
}

impl ConcurrencyLimiter {
    /// Create a new concurrency limiter.
    ///
    /// # Arguments
    /// * `max_concurrent` - Maximum number of concurrent operations
    pub fn new(max_concurrent: u64) -> Self {
        Self {
            inner: Arc::new(LimiteronConcurrencyLimiter::new(max_concurrent)),
        }
    }

    /// Create a concurrency limiter with a timeout.
    ///
    /// # Arguments
    /// * `max_concurrent` - Maximum number of concurrent operations
    /// * `timeout` - Maximum time to wait for a permit
    pub fn with_timeout(max_concurrent: u64, timeout: Duration) -> Self {
        Self {
            inner: Arc::new(LimiteronConcurrencyLimiter::with_timeout(
                max_concurrent,
                timeout,
            )),
        }
    }

    /// Acquire a permit for concurrent execution.
    ///
    /// Returns a permit that will be released when dropped.
    pub async fn acquire(&self) -> Result<tokio::sync::SemaphorePermit<'_>, LimiteronError> {
        self.inner.acquire(1).await
    }

    // Phase 9 — `try_acquire` removed. The previous
    // implementation called `self.inner.allow(1).await?` (consuming a
    // token) and then **unconditionally** returned `Err(...)`, so the
    // method could never produce a `SemaphorePermit`. No caller in the
    // codebase used it. Per rule 2 (简洁优先) + rule 12 (失败必须显性化),
    // a method whose contract lies about its return type must be
    // deleted, not left as a footgun. If non-blocking permit semantics
    // are needed in the future, `tokio::sync::Semaphore::try_acquire`
    // is the right primitive.

    /// Get maximum concurrent limit.
    pub fn max_concurrent(&self) -> u64 {
        self.inner.max_concurrent()
    }
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_rate_limiter_basic() {
        let limiter = RateLimiter::new(10, 5);

        // First 5 requests should be allowed (burst size)
        for i in 0..5 {
            let result = limiter.check_rate_limit("test-key", None, None).await;
            assert!(result.allowed, "Request {} should be allowed", i + 1);
        }

        // 6th request should be rate limited
        let result = limiter.check_rate_limit("test-key", None, None).await;
        assert!(!result.allowed, "6th request should be rate limited");
        assert_eq!(result.limit, 5);
    }

    #[tokio::test]
    async fn test_rate_limiter_different_keys() {
        let limiter = RateLimiter::new(10, 5);

        // Exhaust key1's limit
        for _ in 0..5 {
            let result = limiter.check_rate_limit("key1", None, None).await;
            assert!(result.allowed);
        }

        // key1 should be rate limited
        let result = limiter.check_rate_limit("key1", None, None).await;
        assert!(!result.allowed);

        // key2 should still be allowed (different key, different bucket)
        let result = limiter.check_rate_limit("key2", None, None).await;
        assert!(result.allowed);

        // Verify bucket count
        assert_eq!(limiter.bucket_count(), 2);
    }

    #[tokio::test]
    async fn test_rate_limiter_custom_limits() {
        let limiter = RateLimiter::new(10, 5);

        // Use custom limits for key1: rate=20, capacity=10
        for i in 0..10 {
            let result = limiter.check_rate_limit("key1", Some(20), Some(10)).await;
            assert!(
                result.allowed,
                "Request {} should be allowed with custom limits",
                i + 1
            );
        }

        // 11th request should be limited
        let result = limiter.check_rate_limit("key1", Some(20), Some(10)).await;
        assert!(!result.allowed, "11th request should be limited");
        assert_eq!(result.limit, 10);
    }

    #[tokio::test]
    async fn test_rate_limiter_concurrent() {
        use futures_util::future::join_all;
        use tokio::task;

        let limiter = Arc::new(RateLimiter::new(100, 100));
        let num_tasks = 10;
        let requests_per_task = 10;

        let handles: Vec<_> = (0..num_tasks)
            .map(|i| {
                let limiter = limiter.clone();
                task::spawn(async move {
                    let mut results = Vec::new();
                    for _j in 0..requests_per_task {
                        let key = format!("key-{}", i);
                        let result = limiter.check_rate_limit(&key, None, None).await;
                        results.push(result);
                    }
                    results
                })
            })
            .collect();

        let results: Vec<Vec<RateLimitResult>> = join_all(handles)
            .await
            .into_iter()
            .filter_map(Result::ok)
            .collect();

        // All requests should be allowed (each key has its own bucket with capacity 100)
        for task_results in results {
            for result in task_results {
                assert!(result.allowed, "Concurrent request should be allowed");
            }
        }

        // Verify bucket count
        assert_eq!(limiter.bucket_count(), num_tasks);
    }

    #[tokio::test]
    async fn test_rate_limiter_cleanup() {
        let limiter = RateLimiter::new(10, 5);

        // Add some buckets
        limiter.check_rate_limit("key1", None, None).await;
        limiter.check_rate_limit("key2", None, None).await;

        assert_eq!(limiter.bucket_count(), 2);

        // Wait for a short time
        tokio::time::sleep(Duration::from_millis(10)).await;

        // Cleanup with very short max_idle should remove entries
        let removed = limiter.cleanup(Duration::from_millis(5));
        assert_eq!(removed, 2);

        // Buckets should be removed
        assert_eq!(limiter.bucket_count(), 0);
    }

    #[tokio::test]
    async fn test_rate_limiter_memory_stats() {
        let limiter = RateLimiter::new(10, 5);

        // Add some buckets
        limiter.check_rate_limit("key1", None, None).await;
        limiter.check_rate_limit("key2", None, None).await;

        let stats = limiter.memory_stats();
        assert_eq!(stats.active_limiters, 2);
        assert_eq!(stats.default_rps, 10);
        assert_eq!(stats.default_burst, 5);
    }

    #[tokio::test]
    async fn test_concurrency_limiter_basic() {
        let limiter = ConcurrencyLimiter::new(2);

        // Should be able to acquire 2 permits
        let permit1 = limiter.acquire().await;
        assert!(permit1.is_ok());

        let permit2 = limiter.acquire().await;
        assert!(permit2.is_ok());

        // Third acquire would block (handled by semaphore)
        // We use a short timeout to test
        let timeout_limiter = ConcurrencyLimiter::with_timeout(2, Duration::from_millis(10));
        let _p1 = timeout_limiter.acquire().await.unwrap();
        let _p2 = timeout_limiter.acquire().await.unwrap();

        // Third would block/timeout
        let result = timeout_limiter.acquire().await;
        assert!(result.is_err() || result.as_ref().is_err());

        assert_eq!(timeout_limiter.max_concurrent(), 2);
    }

    #[tokio::test]
    async fn test_get_usage() {
        let limiter = RateLimiter::new(10, 5);

        limiter.check_rate_limit("key1", None, None).await;

        let status = limiter.get_usage("key1");
        assert!(status.is_some());

        let status = status.unwrap();
        assert_eq!(status.limit, 5);
        assert_eq!(status.rate, 10);
        // After 1 request, should have 4 tokens remaining
        assert_eq!(status.remaining, 4);

        // Non-existent key
        let status = limiter.get_usage("non-existent");
        assert!(status.is_none());
    }

    #[tokio::test]
    async fn test_update_defaults() {
        let limiter = RateLimiter::new(10, 5);

        limiter.update_defaults(20, 10);

        // New keys will use new defaults
        let result = limiter.check_rate_limit("new-key", None, None).await;
        assert!(result.allowed);
        assert_eq!(result.limit, 10);
        assert_eq!(result.remaining, 9); // After 1 request, 9 remaining
    }

    #[tokio::test]
    async fn test_token_refill() {
        // Test that tokens refill over time
        let limiter = RateLimiter::new(10, 5);

        // Exhaust bucket
        for _ in 0..5 {
            let result = limiter.check_rate_limit("key", None, None).await;
            assert!(result.allowed);
        }

        // Should be rate limited now
        let result = limiter.check_rate_limit("key", None, None).await;
        assert!(!result.allowed);

        // Wait for some tokens to refill (10 RPS means 1 token per 100ms)
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Should have some tokens now
        let result = limiter.check_rate_limit("key", None, None).await;
        assert!(result.allowed, "Should allow after token refill");
    }

    #[tokio::test]
    async fn test_retry_after_uses_snapshot_reset_secs() {
        // rate=1, capacity=3：耗尽后被拒，Retry-After = 补满 3 个缺口所需 3 秒
        // （快照 reset_secs；旧实现硬编码 1，对慢补充大容量桶会过早放行）。
        let limiter = RateLimiter::new(1, 3);
        for _ in 0..3 {
            assert!(limiter.check_rate_limit("slow", None, None).await.allowed);
        }
        let denied = limiter.check_rate_limit("slow", None, None).await;
        assert!(!denied.allowed);
        assert_eq!(denied.retry_after, Some(3));
    }

    #[tokio::test]
    async fn test_custom_rate_cleanup_stats_and_config_update() {
        let limiter = RateLimiter::new(10, 5);

        let first = limiter.check_rate_limit("custom", Some(1), Some(1)).await;
        assert!(first.allowed);
        let second = limiter.check_rate_limit("custom", Some(1), Some(1)).await;
        assert!(!second.allowed, "1 rps / 1 burst must deny the second call");

        assert_eq!(limiter.active_limiters_count(), 1);
        assert_eq!(limiter.bucket_count(), 1);
        let stats = limiter.memory_stats();
        assert_eq!(stats.active_limiters, 1);
        assert_eq!(stats.default_rps, 10);
        assert_eq!(stats.default_burst, 5);

        let removed = limiter.cleanup(Duration::ZERO);
        assert_eq!(removed, 1);
        assert_eq!(limiter.active_limiters_count(), 0);

        limiter.update_config(30, 15);
        let stats = limiter.memory_stats();
        assert_eq!(stats.default_rps, 30);
        assert_eq!(stats.default_burst, 15);

        drop(RateLimiter::get_concurrency_limiter(4));
    }

    /// 分片表语义回归：大量 distinct key 并发 check（跨所有分片）
    /// 不 panic，桶计数与 key 总数一致，清理语义逐 key 正确。
    #[tokio::test]
    async fn test_sharded_table_many_concurrent_keys_no_panic() {
        use futures_util::future::join_all;
        use tokio::task;

        let limiter = Arc::new(RateLimiter::new(1000, 1000));
        let num_tasks = 8usize;
        let keys_per_task = 200usize;

        let handles: Vec<_> = (0..num_tasks)
            .map(|i| {
                let limiter = limiter.clone();
                task::spawn(async move {
                    for j in 0..keys_per_task {
                        let key = format!("task-{i}-key-{j}");
                        let result = limiter.check_rate_limit(&key, None, None).await;
                        assert!(result.allowed, "burst capacity must allow first hit");
                    }
                })
            })
            .collect();

        for handle in join_all(handles).await {
            handle.expect("concurrent task must not panic");
        }

        // 分片表 len 之和 == 桶总数
        let total = num_tasks * keys_per_task;
        assert_eq!(limiter.bucket_count(), total);
        assert_eq!(limiter.active_limiters_count(), total);
        assert_eq!(limiter.memory_stats().active_limiters, total);

        // 逐 key get_usage 命中（分片定位正确）
        let probe = "task-0-key-0".to_string();
        let usage = limiter.get_usage(&probe).expect("probe key must exist");
        assert_eq!(usage.limit, 1000);

        // 整表清理语义不变
        let removed = limiter.cleanup(Duration::ZERO);
        assert_eq!(removed, total);
        assert_eq!(limiter.bucket_count(), 0);
    }
}
