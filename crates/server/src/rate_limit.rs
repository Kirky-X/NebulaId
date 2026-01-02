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

use dashmap::DashMap;
use std::sync::Arc;
use std::time::Instant;

#[derive(Clone)]
pub struct RateLimiter {
    buckets: Arc<DashMap<String, TokenBucket>>,
    default_rate: u32,
    default_burst: u32,
}

#[derive(Clone, Debug)]
struct TokenBucket {
    tokens: f64,
    last_update: Instant,
    rate: u32,
    burst: u32,
}

impl TokenBucket {
    fn new(rate: u32, burst: u32) -> Self {
        Self {
            tokens: burst as f64,
            last_update: Instant::now(),
            rate,
            burst,
        }
    }

    fn try_consume(&mut self) -> bool {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_update).as_secs_f64();
        self.last_update = now;

        self.tokens = (self.tokens + elapsed * self.rate as f64).min(self.burst as f64);

        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    fn remaining(&mut self) -> u64 {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_update).as_secs_f64();

        self.tokens = (self.tokens + elapsed * self.rate as f64).min(self.burst as f64);
        self.tokens as u64
    }

    fn limit(&self) -> u32 {
        self.burst
    }

    fn rate(&self) -> u32 {
        self.rate
    }
}

impl RateLimiter {
    pub fn new(default_rps: u32, default_burst: u32) -> Self {
        Self {
            buckets: Arc::new(DashMap::new()),
            default_rate: default_rps,
            default_burst,
        }
    }

    pub async fn check_rate_limit(
        &self,
        key: &str,
        custom_rate: Option<u32>,
        custom_burst: Option<u32>,
    ) -> RateLimitResult {
        let mut bucket = self.buckets.entry(key.to_string()).or_insert_with(|| {
            TokenBucket::new(
                custom_rate.unwrap_or(self.default_rate),
                custom_burst.unwrap_or(self.default_burst),
            )
        });

        let allowed = bucket.try_consume();
        let remaining = bucket.remaining();
        let limit = bucket.limit();

        RateLimitResult {
            allowed,
            remaining,
            limit,
            retry_after: if allowed { None } else { Some(1) },
        }
    }

    pub fn get_usage(&self, key: &str) -> Option<RateLimitStatus> {
        self.buckets.get_mut(key).map(|mut bucket| {
            let remaining = bucket.remaining();
            RateLimitStatus {
                remaining,
                limit: bucket.limit(),
                rate: bucket.rate(),
            }
        })
    }

    /// 获取当前桶数量
    pub fn bucket_count(&self) -> usize {
        self.buckets.len()
    }

    /// 清理过期的桶（超过指定时间未访问）
    pub fn cleanup(&self, max_idle: std::time::Duration) -> usize {
        let now = Instant::now();
        let mut removed = 0;

        self.buckets.retain(|_, bucket| {
            let is_expired = now.duration_since(bucket.last_update) > max_idle;
            if is_expired {
                removed += 1;
            }
            !is_expired
        });

        removed
    }
}

#[derive(Debug, Clone)]
pub struct RateLimitResult {
    pub allowed: bool,
    pub remaining: u64,
    pub limit: u32,
    pub retry_after: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct RateLimitStatus {
    pub remaining: u64,
    pub limit: u32,
    pub rate: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_token_bucket_basic() {
        let mut bucket = TokenBucket::new(10, 5);

        for _ in 0..5 {
            assert!(bucket.try_consume());
        }

        assert!(!bucket.try_consume());
    }

    #[tokio::test]
    async fn test_token_bucket_refill() {
        let mut bucket = TokenBucket::new(10, 10);

        for _ in 0..10 {
            assert!(bucket.try_consume());
        }

        assert!(!bucket.try_consume());

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        assert!(bucket.try_consume());
    }

    #[tokio::test]
    async fn test_rate_limiter() {
        let limiter = RateLimiter::new(10, 5);

        for _ in 0..5 {
            let result = limiter.check_rate_limit("test-key", None, None).await;
            assert!(result.allowed);
        }

        let result = limiter.check_rate_limit("test-key", None, None).await;
        assert!(!result.allowed);
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

        let result = limiter.check_rate_limit("key2", None, None).await;
        assert!(result.allowed);
    }

    #[tokio::test]
    async fn test_rate_limiter_custom_limits() {
        let limiter = RateLimiter::new(10, 5);

        for _ in 0..10 {
            let result = limiter.check_rate_limit("key1", Some(20), Some(10)).await;
            assert!(result.allowed);
        }

        let result = limiter.check_rate_limit("key1", Some(20), Some(10)).await;
        assert!(!result.allowed);
    }

    #[tokio::test]
    async fn test_rate_limiter_concurrent() {
        use futures_util::future::join_all;
        use std::sync::Arc;
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

        // 所有请求都应该被允许
        for task_results in results {
            for result in task_results {
                assert!(result.allowed, "Concurrent request should be allowed");
            }
        }

        // 验证 bucket 数量
        assert_eq!(limiter.bucket_count(), num_tasks);
    }

    #[tokio::test]
    async fn test_rate_limiter_cleanup() {
        let limiter = RateLimiter::new(10, 5);

        // 添加一些桶
        limiter.check_rate_limit("key1", None, None).await;
        limiter.check_rate_limit("key2", None, None).await;

        assert_eq!(limiter.bucket_count(), 2);

        // 等待过期
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // 清理过期的桶
        let removed = limiter.cleanup(std::time::Duration::from_millis(100));
        assert_eq!(removed, 2);
        assert_eq!(limiter.bucket_count(), 0);
    }
}
