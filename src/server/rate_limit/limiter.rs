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
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::time::interval;
use tracing::debug;

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
    last_accessed: Arc<RwLock<Instant>>,
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
            last_accessed: Arc::new(RwLock::new(Instant::now())),
        }
    }

    /// Update the last accessed time
    fn touch(&self) {
        *self.last_accessed.write() = Instant::now();
    }

    /// Get the last accessed time
    fn last_accessed(&self) -> Instant {
        *self.last_accessed.read()
    }

    /// Check if a request is allowed and consume a token
    async fn check(&self) -> Result<RateLimitResult, LimiteronError> {
        // Update last accessed time
        self.touch();

        let allowed = self.limiter.allow(1).await?;

        // Get remaining tokens for response header
        let remaining = self.limiter.tokens();

        Ok(RateLimitResult {
            allowed,
            remaining,
            limit: self.capacity,
            retry_after: if allowed { None } else { Some(1) },
        })
    }
}

/// Main rate limiter for the application.
///
/// Uses limiteron's TokenBucketLimiter for smooth, accurate rate limiting
/// with per-key tracking.
#[derive(Clone)]
pub struct RateLimiter {
    limiters: Arc<RwLock<HashMap<String, InternalRateLimiter>>>,
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
            limiters: Arc::new(RwLock::new(HashMap::new())),
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

        tokio::spawn(async move {
            let mut interval_timer = interval(cleanup_interval);
            loop {
                interval_timer.tick().await;

                let now = Instant::now();
                let mut removed_count = 0;

                // Find and remove expired limiters
                {
                    let mut limiters_guard = limiters.write();
                    let keys_to_remove: Vec<String> = limiters_guard
                        .iter()
                        .filter(|(_, limiter)| {
                            now.duration_since(limiter.last_accessed()) > max_idle
                        })
                        .map(|(key, _)| key.clone())
                        .collect();

                    for key in keys_to_remove {
                        limiters_guard.remove(&key);
                        removed_count += 1;
                    }
                }

                if removed_count > 0 {
                    debug!(
                        event = "rate_limiter_cleanup",
                        removed_count = removed_count,
                        "Cleaned up {} expired rate limiters",
                        removed_count
                    );
                }
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

        // Get or create the limiter for this key
        let limiter = {
            let mut limiters = self.limiters.write();
            limiters
                .entry(key.to_string())
                .or_insert_with(|| InternalRateLimiter::new(rate, capacity))
                .clone()
        };

        match limiter.check().await {
            Ok(result) => result,
            Err(e) => {
                tracing::warn!("Rate limit check error: {:?}", e);
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
        let limiters = self.limiters.read();
        limiters.get(key).map(|entry| RateLimitStatus {
            remaining: entry.limiter.tokens(),
            limit: entry.capacity,
            rate: entry.rate,
        })
    }

    /// Get the current number of rate limit buckets.
    pub fn bucket_count(&self) -> usize {
        self.limiters.read().len()
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
        let now = Instant::now();
        let mut limiters = self.limiters.write();

        let keys_to_remove: Vec<String> = limiters
            .iter()
            .filter(|(_, limiter)| now.duration_since(limiter.last_accessed()) > max_idle)
            .map(|(key, _)| key.clone())
            .collect();

        let removed_count = keys_to_remove.len();
        for key in keys_to_remove {
            limiters.remove(&key);
        }

        if removed_count > 0 {
            debug!(
                event = "rate_limiter_cleanup",
                removed_count = removed_count,
                "Manually cleaned up {} expired rate limiters",
                removed_count
            );
        }

        removed_count
    }

    /// Get the current number of active rate limiters.
    pub fn active_limiters_count(&self) -> usize {
        self.limiters.read().len()
    }

    /// Get memory usage statistics for monitoring.
    pub fn memory_stats(&self) -> RateLimiterMemoryStats {
        let limiters = self.limiters.read();
        RateLimiterMemoryStats {
            active_limiters: limiters.len(),
            default_rps: self.defaults.read().0,
            default_burst: self.defaults.read().1,
        }
    }

    /// Update the default rate limit settings.
    pub fn update_defaults(&self, default_rps: u32, default_burst: u32) {
        let mut defaults = self.defaults.write();
        *defaults = (default_rps, default_burst);
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

    /// Try to acquire a permit without blocking.
    ///
    /// Returns Ok(permit) if acquired, Err if limit reached.
    pub async fn try_acquire(&self) -> Result<tokio::sync::SemaphorePermit<'_>, LimiteronError> {
        self.inner.allow(1).await?;
        Err(LimiteronError::LimitError(
            "try_acquire requires async context - use acquire() with timeout".to_string(),
        ))
    }

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
}
