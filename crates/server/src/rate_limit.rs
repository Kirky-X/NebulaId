use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;
use std::collections::HashMap;

#[derive(Clone)]
pub struct RateLimiter {
    buckets: Arc<Mutex<HashMap<String, TokenBucket>>>,
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
}

impl RateLimiter {
    pub fn new(default_rps: u32, default_burst: u32) -> Self {
        Self {
            buckets: Arc::new(Mutex::new(HashMap::new())),
            default_rate: default_rps,
            default_burst: default_burst,
        }
    }

    pub async fn check_rate_limit(&self, key: &str, custom_rate: Option<u32>, custom_burst: Option<u32>) -> RateLimitResult {
        let mut buckets = self.buckets.lock().await;
        
        let bucket = buckets.entry(key.to_string())
            .or_insert_with(|| TokenBucket::new(
                custom_rate.unwrap_or(self.default_rate),
                custom_burst.unwrap_or(self.default_burst)
            ));

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

    pub async fn get_usage(&self, key: &str) -> Option<RateLimitStatus> {
        let mut buckets = self.buckets.lock().await;
        buckets.get_mut(key).map(|bucket| {
            let remaining = bucket.remaining();
            RateLimitStatus {
                remaining,
                limit: bucket.limit(),
                rate: bucket.rate,
            }
        })
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
}
