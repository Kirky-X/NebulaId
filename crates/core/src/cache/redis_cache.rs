#![allow(dead_code)]

use async_trait::async_trait;
use redis::aio::MultiplexedConnection;
use redis::AsyncCommands;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, error, warn};

use crate::algorithm::circuit_breaker::{
    CircuitBreaker, CircuitBreakerConfig, CircuitBreakerError,
};
use crate::cache::multi_level_cache::CacheBackend;
use crate::types::{CoreError, Result};

/// Redis 连接池条目
struct PooledConnection {
    connection: MultiplexedConnection,
    last_used: std::time::Instant,
}

impl PooledConnection {
    fn new(connection: MultiplexedConnection) -> Self {
        Self {
            connection,
            last_used: std::time::Instant::now(),
        }
    }

    fn is_stale(&self, max_idle: Duration) -> bool {
        self.last_used.elapsed() > max_idle
    }
}

/// Redis 连接池
#[derive(Clone)]
struct ConnectionPool {
    url: String,
    pool: Arc<tokio::sync::Mutex<Vec<PooledConnection>>>,
    max_size: usize,
}

impl ConnectionPool {
    fn new(url: String, max_size: usize) -> Self {
        Self {
            url,
            pool: Arc::new(tokio::sync::Mutex::new(Vec::new())),
            max_size,
        }
    }

    async fn get(&self) -> Result<MultiplexedConnection> {
        let mut pool = self.pool.lock().await;

        // 尝试从池中获取
        if let Some(idx) = pool
            .iter()
            .position(|c| !c.is_stale(Duration::from_secs(300)))
        {
            let pooled = pool.remove(idx);
            return Ok(pooled.connection);
        }

        // 检查池大小
        if pool.len() >= self.max_size {
            return Err(CoreError::CacheError(
                "Connection pool exhausted".to_string(),
            ));
        }

        // 创建新连接
        drop(pool);

        let client = redis::Client::open(self.url.as_str())
            .map_err(|e| CoreError::CacheError(format!("Failed to create Redis client: {}", e)))?;

        client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| CoreError::CacheError(format!("Failed to connect to Redis: {}", e)))
    }

    async fn return_(&self, connection: MultiplexedConnection) {
        let mut pool = self.pool.lock().await;
        if pool.len() < self.max_size {
            pool.push(PooledConnection::new(connection));
        }
    }
}

/// Redis 缓存后端
#[derive(Clone)]
pub struct RedisCacheBackend {
    pool: Arc<ConnectionPool>,
    circuit_breaker: Arc<CircuitBreaker>,
    key_prefix: String,
    ttl_seconds: u64,
    metrics: Arc<RedisCacheMetrics>,
}

#[derive(Debug, Default)]
struct RedisCacheMetrics {
    total_requests: AtomicUsize,
    hits: AtomicUsize,
    misses: AtomicUsize,
    errors: AtomicUsize,
    pool_acquires: AtomicUsize,
    pool_retries: AtomicUsize,
}

impl RedisCacheBackend {
    /// 创建 Redis 缓存后端
    pub async fn new(
        url: &str,
        key_prefix: String,
        ttl_seconds: u64,
        pool_size: usize,
    ) -> Result<Self> {
        let pool = Arc::new(ConnectionPool::new(url.to_string(), pool_size));

        // 验证连接
        let conn = pool.get().await?;
        let pong: String = redis::cmd("PING")
            .query_async(&mut conn.clone())
            .await
            .map_err(|e| CoreError::CacheError(format!("Redis PING failed: {}", e)))?;
        pool.return_(conn).await;

        if pong != "PONG" {
            return Err(CoreError::CacheError(
                "Unexpected PONG response".to_string(),
            ));
        }

        // 创建熔断器
        let circuit_breaker = Arc::new(CircuitBreaker::new(CircuitBreakerConfig {
            failure_threshold: 5,
            success_threshold: 3,
            timeout_ms: 30000,
            window_size_seconds: 60,
            min_requests: 10,
        }));

        debug!(
            "RedisCacheBackend connected to {} with prefix '{}', pool_size={}",
            url, key_prefix, pool_size
        );

        Ok(Self {
            pool,
            circuit_breaker,
            key_prefix,
            ttl_seconds,
            metrics: Arc::new(RedisCacheMetrics::default()),
        })
    }

    /// 从配置创建
    pub async fn from_config(config: &crate::config::RedisConfig) -> Result<Self> {
        let pool_size = config.pool_size as usize;
        Self::new(
            &config.url,
            config.key_prefix.clone(),
            config.ttl_seconds,
            pool_size,
        )
        .await
    }

    /// 清理过期的连接
    pub async fn cleanup(&self, max_idle: Duration) {
        let mut pool = self.pool.pool.lock().await;
        pool.retain(|c| !c.is_stale(max_idle));
    }

    /// 获取缓存指标快照
    pub fn metrics(&self) -> RedisCacheMetricsSnapshot {
        RedisCacheMetricsSnapshot {
            total_requests: self.metrics.total_requests.load(Ordering::Relaxed),
            hits: self.metrics.hits.load(Ordering::Relaxed),
            misses: self.metrics.misses.load(Ordering::Relaxed),
            errors: self.metrics.errors.load(Ordering::Relaxed),
            pool_acquires: self.metrics.pool_acquires.load(Ordering::Relaxed),
            pool_retries: 0, // Not tracked yet
        }
    }
}

#[async_trait]
impl CacheBackend for RedisCacheBackend {
    async fn get(&self, key: &str) -> Result<Option<Vec<u64>>> {
        self.metrics.total_requests.fetch_add(1, Ordering::Relaxed);

        let prefixed = format!("{}{}", self.key_prefix, key);
        let pool = self.pool.clone();
        let metrics = self.metrics.clone();

        let result =
            self.circuit_breaker
                .execute(async move {
                    let mut conn = pool.get().await.map_err(|e| {
                        CircuitBreakerError::new(format!("Pool acquire failed: {}", e))
                    })?;
                    metrics.pool_acquires.fetch_add(1, Ordering::Relaxed);

                    let result: redis::RedisResult<Option<Vec<u8>>> = conn.get(&prefixed).await;
                    pool.return_(conn).await;

                    match result {
                        Ok(Some(data)) => {
                            let data_len = data.len();
                            if data_len >= 4 {
                                if let Some((payload, stored_crc)) = data.split_last_chunk::<4>() {
                                    let computed_crc = crc32fast::hash(payload);
                                    let stored_crc_u32 = u32::from_le_bytes(*stored_crc);

                                    if computed_crc != stored_crc_u32 {
                                        metrics.errors.fetch_add(1, Ordering::Relaxed);
                                        warn!("CRC32 mismatch for key {}, data corrupted", key);
                                        return Err(CircuitBreakerError::new(
                                            "CRC32 verification failed".to_string(),
                                        ));
                                    }

                                    let ids = deserialize_ids(&data[..data_len - 4]);
                                    metrics.hits.fetch_add(1, Ordering::Relaxed);
                                    debug!("Redis cache hit for key: {}", key);
                                    return Ok(Some(ids));
                                }
                            }

                            let ids = deserialize_ids(&data);
                            metrics.hits.fetch_add(1, Ordering::Relaxed);
                            debug!("Redis cache hit for key: {} (no CRC)", key);
                            Ok(Some(ids))
                        }
                        Ok(None) => {
                            metrics.misses.fetch_add(1, Ordering::Relaxed);
                            debug!("Redis cache miss for key: {}", key);
                            Ok(None)
                        }
                        Err(e) => {
                            metrics.errors.fetch_add(1, Ordering::Relaxed);
                            error!("Redis get error for key {}: {}", key, e);
                            Err(CircuitBreakerError::new(format!("Redis get failed: {}", e)))
                        }
                    }
                })
                .await;

        match result {
            Ok(v) => Ok(v),
            Err(e) => Err(CoreError::CacheError(e.message)),
        }
    }

    async fn set(&self, key: &str, values: &[u64], ttl_seconds: u64) -> Result<()> {
        let prefixed_key = format!("{}{}", self.key_prefix, key);
        let data = serialize_ids(values);
        let effective_ttl = if ttl_seconds > 0 {
            ttl_seconds
        } else {
            self.ttl_seconds
        };
        let pool = self.pool.clone();
        let metrics = self.metrics.clone();

        let result =
            self.circuit_breaker
                .execute(async move {
                    let mut conn = pool.get().await.map_err(|e| {
                        CircuitBreakerError::new(format!("Pool acquire failed: {}", e))
                    })?;

                    let result: redis::RedisResult<()> = if effective_ttl > 0 {
                        conn.set_ex(&prefixed_key, data, effective_ttl).await
                    } else {
                        conn.set(&prefixed_key, data).await
                    };

                    pool.return_(conn).await;

                    match result {
                        Ok(()) => {
                            debug!(
                                "Redis cache set for key: {} with {} values",
                                key,
                                values.len()
                            );
                            Ok(())
                        }
                        Err(e) => {
                            metrics.errors.fetch_add(1, Ordering::Relaxed);
                            error!("Redis set error for key {}: {}", key, e);
                            Err(CircuitBreakerError::new(format!("Redis set failed: {}", e)))
                        }
                    }
                })
                .await;

        match result {
            Ok(v) => Ok(v),
            Err(e) => Err(CoreError::CacheError(e.message)),
        }
    }

    async fn delete(&self, key: &str) -> Result<()> {
        let prefixed_key = format!("{}{}", self.key_prefix, key);
        let pool = self.pool.clone();
        let metrics = self.metrics.clone();

        let result =
            self.circuit_breaker
                .execute(async move {
                    let mut conn = pool.get().await.map_err(|e| {
                        CircuitBreakerError::new(format!("Pool acquire failed: {}", e))
                    })?;

                    let result: redis::RedisResult<usize> = conn.del(&prefixed_key).await;
                    pool.return_(conn).await;

                    match result {
                        Ok(_) => {
                            debug!("Redis cache delete for key: {}", key);
                            Ok(())
                        }
                        Err(e) => {
                            metrics.errors.fetch_add(1, Ordering::Relaxed);
                            error!("Redis delete error for key {}: {}", key, e);
                            Err(CircuitBreakerError::new(format!(
                                "Redis delete failed: {}",
                                e
                            )))
                        }
                    }
                })
                .await;

        match result {
            Ok(v) => Ok(v),
            Err(e) => Err(CoreError::CacheError(e.message)),
        }
    }

    async fn exists(&self, key: &str) -> Result<bool> {
        let prefixed_key = format!("{}{}", self.key_prefix, key);
        let pool = self.pool.clone();
        let metrics = self.metrics.clone();

        let result =
            self.circuit_breaker
                .execute(async move {
                    let mut conn = pool.get().await.map_err(|e| {
                        CircuitBreakerError::new(format!("Pool acquire failed: {}", e))
                    })?;

                    let result: redis::RedisResult<bool> = conn.exists(&prefixed_key).await;
                    pool.return_(conn).await;

                    match result {
                        Ok(exists) => Ok(exists),
                        Err(e) => {
                            metrics.errors.fetch_add(1, Ordering::Relaxed);
                            error!("Redis exists error for key {}: {}", key, e);
                            Err(CircuitBreakerError::new(format!(
                                "Redis exists failed: {}",
                                e
                            )))
                        }
                    }
                })
                .await;

        match result {
            Ok(v) => Ok(v),
            Err(e) => Err(CoreError::CacheError(e.message)),
        }
    }
}

/// 序列化 u64 数组为字节向量（带 CRC32 校验和）
fn serialize_ids(ids: &[u64]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(ids.len() * 8 + 4);

    for &id in ids {
        bytes.extend_from_slice(&id.to_le_bytes());
    }

    let checksum = crc32fast::hash(&bytes);
    bytes.extend_from_slice(&checksum.to_le_bytes());

    bytes
}

/// 从字节向量反序列化 u64 数组
fn deserialize_ids(bytes: &[u8]) -> Vec<u64> {
    bytes
        .chunks_exact(8)
        .map(|chunk| {
            let mut arr = [0u8; 8];
            arr.copy_from_slice(chunk);
            u64::from_le_bytes(arr)
        })
        .collect()
}

/// Redis 缓存指标快照
#[derive(Debug, Clone, Default)]
pub struct RedisCacheMetricsSnapshot {
    pub total_requests: usize,
    pub hits: usize,
    pub misses: usize,
    pub errors: usize,
    pub pool_acquires: usize,
    pub pool_retries: usize,
}

impl RedisCacheMetricsSnapshot {
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            self.hits as f64 / total as f64
        }
    }

    pub fn error_rate(&self) -> f64 {
        let total = self.total_requests;
        if total == 0 {
            0.0
        } else {
            self.errors as f64 / total as f64
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_serialize_deserialize() {
        let ids = vec![1u64, 2, 3, 100, 1000];

        let bytes = serialize_ids(&ids);
        assert_eq!(bytes.len(), 44);

        let data_without_crc = &bytes[..bytes.len() - 4];
        let decoded = deserialize_ids(data_without_crc);
        assert_eq!(decoded, ids);
    }

    #[tokio::test]
    async fn test_empty_serialize() {
        let ids: Vec<u64> = vec![];

        let bytes = serialize_ids(&ids);
        assert_eq!(bytes.len(), 4);
    }

    #[tokio::test]
    async fn test_large_id_serialize() {
        let ids = vec![u64::MAX, u64::MIN, u64::MAX / 2];

        let bytes = serialize_ids(&ids);
        let data_without_crc = &bytes[..bytes.len() - 4];
        let decoded = deserialize_ids(data_without_crc);
        assert_eq!(decoded, ids);
    }

    #[tokio::test]
    async fn test_crc32_verification() {
        let ids = vec![1u64, 2, 3];
        let bytes = serialize_ids(&ids);

        let payload = &bytes[..bytes.len() - 4];
        let stored_crc = u32::from_le_bytes(bytes[bytes.len() - 4..].try_into().unwrap());
        let computed_crc = crc32fast::hash(payload);

        assert_eq!(stored_crc, computed_crc);
    }
}
