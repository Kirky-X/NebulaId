// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! 基于 limiteron 的限流模块：统一限流、并发控制与配额管理。
//! [`RateLimiter`] 用令牌桶算法按 key 限流；[`ConcurrencyLimiter`] 控制最大并发数。

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

/// 限流表分片数（2 的幂）：按 key 哈希分摊读写热点，避免所有 check 串行在一把全局写锁上。
const LIMITER_SHARDS: usize = 16;

/// `last_accessed` 的时钟（Unix 纳秒，u64 可容纳至 2554 年）。
///
/// 原子量替代 `Arc<RwLock<Instant>>`；用 `SystemTime` 因原子量只能存整数，
/// 墙钟回拨最多让空闲桶多存活一个回拨窗口，对分钟级 max_idle 无实际影响。
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
    /// 最后访问时刻（Unix 纳秒原子量），仅用于空闲清理判定。
    last_accessed: Arc<AtomicU64>,
}

impl InternalRateLimiter {
    /// 新建内部限流器：`rate` = 每秒补充数，`capacity` = 桶容量（突发上限）。
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

    /// 检查是否放行并消费一个令牌。
    async fn check(&self) -> Result<RateLimitResult, LimiteronError> {
        self.touch();

        let allowed = self.limiter.allow(1).await?;

        // 消费后经 limiteron 快照读取 limit/remaining/reset_secs；被拒时
        // reset_secs（补满所需秒数）作为 Retry-After，避免过早放行慢补充桶。
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

/// 分片限流表：`LIMITER_SHARDS` 个 `RwLock<HashMap>` 按 key 哈希分布，语义与单表等价仅细化锁粒度。
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

    /// 定位 key 所属分片；`DefaultHasher` 种子固定，仅需进程内分布稳定。
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

/// 应用主限流器：limiteron 令牌桶 + 按 key 跟踪。
#[derive(Clone)]
pub struct RateLimiter {
    limiters: Arc<ShardedLimiters>,
    defaults: Arc<RwLock<(u32, u32)>>,
    cleanup_interval: Arc<RwLock<Duration>>,
}

impl RateLimiter {
    /// 新建限流器：`default_rps` = 默认每秒补充数，`default_burst` = 默认桶容量。
    ///
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

    /// 启动后台清理任务：周期清除空闲超过 `max_idle` 的限流桶。
    ///
    /// 返回 JoinHandle，可用于等待清理任务。
    pub fn start_cleanup(
        &self,
        max_idle: Duration,
        cleanup_interval: Duration,
    ) -> tokio::task::JoinHandle<()> {
        *self.cleanup_interval.write() = cleanup_interval;

        let limiters = self.limiters.clone();
        let max_idle_nanos = max_idle.as_nanos() as u64;

        tokio::spawn(async move {
            let mut interval_timer = interval(cleanup_interval);
            loop {
                interval_timer.tick().await;

                let now_nanos = unix_nanos();
                // 锁内只收集 key 并从分片摘除；桶先移出锁作用域，统一在锁外释放，
                // 避免 drop 成本随待删桶数量放大分片写锁持有时长。
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

    /// 检查指定 key 的限流；`custom_rate`/`custom_burst` 仅在该 key 首次建桶时生效。
    ///
    /// 返回 `RateLimitResult` 表示是否放行及 Retry-After。
    pub async fn check_rate_limit(
        &self,
        key: &str,
        custom_rate: Option<u32>,
        custom_burst: Option<u32>,
    ) -> RateLimitResult {
        let (default_rps, default_burst) = *self.defaults.read();

        let (rate, capacity) = if custom_rate.is_some() || custom_burst.is_some() {
            (
                custom_rate.unwrap_or(default_rps),
                custom_burst.unwrap_or(default_burst),
            )
        } else {
            (default_rps, default_burst)
        };

        // 取或建该 key 的限流桶（写锁粒度 = key 所属分片）
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
                // fail-open：限流器异常时放行，可用性优先
                RateLimitResult {
                    allowed: true,
                    remaining: default_burst as u64,
                    limit: default_burst,
                    retry_after: None,
                }
            }
        }
    }

    /// 查询指定 key 的限流状态。
    pub fn get_usage(&self, key: &str) -> Option<RateLimitStatus> {
        let shard = self.limiters.shard_for(key).read();
        shard.get(key).map(|entry| RateLimitStatus {
            remaining: entry.limiter.tokens(),
            limit: entry.capacity,
            rate: entry.rate,
        })
    }

    /// 当前限流桶总数。
    pub fn bucket_count(&self) -> usize {
        self.limiters
            .for_each_shard(|shard| shard.read().len())
            .into_iter()
            .sum()
    }

    /// 手动清理空闲超过 `max_idle` 的限流桶，返回删除数量。
    pub fn cleanup(&self, max_idle: Duration) -> usize {
        let now_nanos = unix_nanos();
        let max_idle_nanos = max_idle.as_nanos() as u64;

        // 锁内只收集 key 并从分片摘除，桶在所有分片锁外释放（与后台清理同一边界）。
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

        let removed_buckets: Vec<InternalRateLimiter> = per_shard
            .into_iter()
            .flat_map(|(_, buckets)| buckets)
            .collect();
        drop(removed_buckets);
        removed_count
    }

    /// 当前活跃限流器数量。
    pub fn active_limiters_count(&self) -> usize {
        self.limiters
            .for_each_shard(|shard| shard.read().len())
            .into_iter()
            .sum()
    }

    /// 内存使用统计（供监控）。
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

    /// 更新默认限流设置。
    pub fn update_defaults(&self, default_rps: u32, default_burst: u32) {
        let mut defaults = self.defaults.write();
        *defaults = (default_rps, default_burst);
    }

    /// 运行时更新默认限流配置，并丢弃现有桶以便下次请求按新配置重建
    /// （否则既有 key 永远保留旧速率）；重建会重置令牌状态，是一次性宽限窗口。
    pub fn update_config(&self, default_rps: u32, default_burst: u32) {
        self.update_defaults(default_rps, default_burst);

        // 各分片锁内只做一次 O(1) 换表；旧表连同全部桶在所有分片锁外释放，
        // 避免持锁期间逐个 drop 使锁持有时间随桶数量线性放大。
        let old_buckets: Vec<HashMap<String, InternalRateLimiter>> =
            self.limiters.for_each_shard(|shard| {
                let mut shard_guard = shard.write();
                std::mem::take(&mut *shard_guard)
            });

        drop(old_buckets);
    }

    /// 构造内部使用的 limiteron 并发限流器。
    pub fn get_concurrency_limiter(max_concurrent: u64) -> ConcurrencyLimiter {
        ConcurrencyLimiter::new(max_concurrent)
    }
}

/// 限流器内存使用统计（供监控）。
#[derive(Debug, Clone)]
pub struct RateLimiterMemoryStats {
    /// Number of active rate limit buckets
    pub active_limiters: usize,
    /// Default requests per second
    pub default_rps: u32,
    /// Default burst capacity
    pub default_burst: u32,
}

/// limiteron `ConcurrencyLimiter` 的包装。
#[derive(Clone)]
pub struct ConcurrencyLimiter {
    inner: Arc<LimiteronConcurrencyLimiter>,
}

impl ConcurrencyLimiter {
    /// 新建并发限流器：`max_concurrent` = 最大并发操作数。
    pub fn new(max_concurrent: u64) -> Self {
        Self {
            inner: Arc::new(LimiteronConcurrencyLimiter::new(max_concurrent)),
        }
    }

    /// 带超时的并发限流器：等待许可超过 `timeout` 即失败。
    pub fn with_timeout(max_concurrent: u64, timeout: Duration) -> Self {
        Self {
            inner: Arc::new(LimiteronConcurrencyLimiter::with_timeout(
                max_concurrent,
                timeout,
            )),
        }
    }

    /// 获取并发许可；许可在 drop 时归还。
    pub async fn acquire(&self) -> Result<tokio::sync::SemaphorePermit<'_>, LimiteronError> {
        self.inner.acquire(1).await
    }

    /// 最大并发数。
    pub fn max_concurrent(&self) -> u64 {
        self.inner.max_concurrent()
    }
}

// ============================== Unit Tests ==============================

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_rate_limiter_basic() {
        let limiter = RateLimiter::new(10, 5);

        for i in 0..5 {
            let result = limiter.check_rate_limit("test-key", None, None).await;
            assert!(result.allowed, "Request {} should be allowed", i + 1);
        }

        let result = limiter.check_rate_limit("test-key", None, None).await;
        assert!(!result.allowed, "6th request should be rate limited");
        assert_eq!(result.limit, 5);
    }

    #[tokio::test]
    async fn test_rate_limiter_different_keys() {
        let limiter = RateLimiter::new(10, 5);

        for _ in 0..5 {
            let result = limiter.check_rate_limit("key1", None, None).await;
            assert!(result.allowed);
        }

        let result = limiter.check_rate_limit("key1", None, None).await;
        assert!(!result.allowed);

        // key2 是不同 key、不同桶，不受 key1 耗尽影响
        let result = limiter.check_rate_limit("key2", None, None).await;
        assert!(result.allowed);

        assert_eq!(limiter.bucket_count(), 2);
    }

    #[tokio::test]
    async fn test_rate_limiter_custom_limits() {
        let limiter = RateLimiter::new(10, 5);

        // 自定义速率 20 / 容量 10
        for i in 0..10 {
            let result = limiter.check_rate_limit("key1", Some(20), Some(10)).await;
            assert!(
                result.allowed,
                "Request {} should be allowed with custom limits",
                i + 1
            );
        }

        // 第 11 次应被拒
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

        // 每 key 独立桶、容量 100，全部请求应放行
        for task_results in results {
            for result in task_results {
                assert!(result.allowed, "Concurrent request should be allowed");
            }
        }

        assert_eq!(limiter.bucket_count(), num_tasks);
    }

    #[tokio::test]
    async fn test_rate_limiter_cleanup() {
        let limiter = RateLimiter::new(10, 5);

        limiter.check_rate_limit("key1", None, None).await;
        limiter.check_rate_limit("key2", None, None).await;

        assert_eq!(limiter.bucket_count(), 2);

        tokio::time::sleep(Duration::from_millis(10)).await;

        let removed = limiter.cleanup(Duration::from_millis(5));
        assert_eq!(removed, 2);

        assert_eq!(limiter.bucket_count(), 0);
    }

    #[tokio::test]
    async fn test_rate_limiter_memory_stats() {
        let limiter = RateLimiter::new(10, 5);

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

        // 第三个许可会阻塞，用短超时验证
        let timeout_limiter = ConcurrencyLimiter::with_timeout(2, Duration::from_millis(10));
        let _p1 = timeout_limiter.acquire().await.unwrap();
        let _p2 = timeout_limiter.acquire().await.unwrap();

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
        // 消费 1 个令牌后剩 4
        assert_eq!(status.remaining, 4);

        let status = limiter.get_usage("non-existent");
        assert!(status.is_none());
    }

    #[tokio::test]
    async fn test_update_defaults() {
        let limiter = RateLimiter::new(10, 5);

        limiter.update_defaults(20, 10);

        // 新 key 使用新默认值
        let result = limiter.check_rate_limit("new-key", None, None).await;
        assert!(result.allowed);
        assert_eq!(result.limit, 10);
        assert_eq!(result.remaining, 9); // After 1 request, 9 remaining
    }

    #[tokio::test]
    async fn test_token_refill() {
        let limiter = RateLimiter::new(10, 5);

        for _ in 0..5 {
            let result = limiter.check_rate_limit("key", None, None).await;
            assert!(result.allowed);
        }

        let result = limiter.check_rate_limit("key", None, None).await;
        assert!(!result.allowed);

        // 10 RPS 即每 100ms 补 1 个令牌
        tokio::time::sleep(Duration::from_millis(200)).await;

        // 补充后应放行
        let result = limiter.check_rate_limit("key", None, None).await;
        assert!(result.allowed, "Should allow after token refill");
    }

    #[tokio::test]
    async fn test_retry_after_uses_snapshot_reset_secs() {
        // rate=1, capacity=3：耗尽后被拒，Retry-After = 补满 3 个缺口所需 3 秒
        // （快照 reset_secs；旧实现硬编码 1，慢补充大容量桶会过早放行）。
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

    /// 分片表回归：大量 distinct key 并发 check 不 panic，桶计数与清理语义正确。
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
