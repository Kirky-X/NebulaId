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

use crate::cache::{CacheBackend, RedisCacheBackend};
use crate::types::CoreError;
use std::sync::Arc;
use tokio::task;

const REDIS_URL: &str = "redis://localhost:6379";
const KEY_PREFIX: &str = "nebula:test:";

async fn get_test_backend() -> RedisCacheBackend {
    RedisCacheBackend::new(
        REDIS_URL,
        KEY_PREFIX.to_string(),
        60, // TTL 60秒
        10, // 连接池大小
    )
    .await
    .expect("Failed to create test backend")
}

#[cfg(test)]
mod integration_tests {
    use super::*;

    #[tokio::test]
    async fn test_redis_integration_get_set() {
        let backend = get_test_backend().await;
        let test_key = "integration_test_get_set";
        let test_values = vec![100u64, 200, 300, 400, 500];

        // SET
        let set_result = backend.set(test_key, &test_values, 60).await;
        assert!(set_result.is_ok(), "SET should succeed");

        // GET
        let get_result = backend.get(test_key).await;
        assert!(get_result.is_ok(), "GET should succeed");
        let retrieved = get_result.unwrap();
        assert!(retrieved.is_some(), "Should find the key");
        assert_eq!(retrieved.unwrap(), test_values, "Values should match");

        // Cleanup
        let _ = backend.delete(test_key).await;
    }

    #[tokio::test]
    async fn test_redis_integration_exists() {
        let backend = get_test_backend().await;
        let test_key = "integration_test_exists";

        // Key should not exist initially
        let exists_before = backend.exists(test_key).await;
        assert!(!exists_before.unwrap(), "Key should not exist");

        // SET
        backend.set(test_key, &[1, 2, 3], 60).await.unwrap();

        // Key should exist now
        let exists_after = backend.exists(test_key).await;
        assert!(exists_after.unwrap(), "Key should exist after SET");

        // Cleanup
        let _ = backend.delete(test_key).await;
    }

    #[tokio::test]
    async fn test_redis_integration_delete() {
        let backend = get_test_backend().await;
        let test_key = "integration_test_delete";

        // SET
        backend.set(test_key, &[1, 2, 3], 60).await.unwrap();
        assert!(backend.exists(test_key).await.unwrap());

        // DELETE
        let delete_result = backend.delete(test_key).await;
        assert!(delete_result.is_ok(), "DELETE should succeed");

        // Verify deletion
        let exists_after = backend.exists(test_key).await;
        assert!(!exists_after.unwrap(), "Key should not exist after DELETE");
    }

    #[tokio::test]
    async fn test_redis_integration_ttl() {
        let backend = get_test_backend().await;
        let test_key = "integration_test_ttl";
        let test_values = vec![1u64, 2, 3];

        // SET with TTL
        backend.set(test_key, &test_values, 60).await.unwrap();

        // Key should exist
        let exists = backend.exists(test_key).await;
        assert!(exists.unwrap(), "Key should exist with TTL");

        // Cleanup
        let _ = backend.delete(test_key).await;
    }

    #[tokio::test]
    async fn test_redis_integration_large_values() {
        let backend = get_test_backend().await;
        let test_key = "integration_test_large";

        // Generate large dataset
        let large_values: Vec<u64> = (1..=1000).collect();

        // SET large values
        let set_result = backend.set(test_key, &large_values, 60).await;
        assert!(set_result.is_ok(), "SET large values should succeed");

        // GET and verify
        let get_result = backend.get(test_key).await;
        assert!(get_result.is_ok(), "GET should succeed");
        let retrieved = get_result.unwrap();
        assert!(retrieved.is_some(), "Should find the key");
        assert_eq!(
            retrieved.unwrap(),
            large_values,
            "Large values should match"
        );

        // Cleanup
        let _ = backend.delete(test_key).await;
    }

    #[tokio::test]
    async fn test_redis_integration_metrics() {
        let backend = get_test_backend().await;
        let test_key = "integration_test_metrics";

        // Initial metrics
        let initial_metrics = backend.metrics();
        assert_eq!(initial_metrics.total_requests, 0);

        // SET
        backend.set(test_key, &[1, 2, 3], 60).await.unwrap();

        // GET (should be a miss initially)
        let _ = backend.get(test_key).await;
        let metrics_after_get = backend.metrics();
        assert!(metrics_after_get.total_requests >= 1);

        // HIT - GET again
        let _ = backend.get(test_key).await;
        let metrics_after_hit = backend.metrics();
        assert!(metrics_after_hit.hits >= 1);

        // Cleanup
        let _ = backend.delete(test_key).await;
    }

    #[tokio::test]
    async fn test_redis_integration_concurrent() {
        let backend = Arc::new(get_test_backend().await);
        let test_keys: Vec<String> = (0..10)
            .map(|i| format!("integration_test_concurrent_{}", i))
            .collect();
        let test_values: Vec<u64> = vec![1, 2, 3, 4, 5];

        // Concurrent SET operations
        let tasks: Vec<_> = test_keys
            .iter()
            .map(|key| {
                let backend = backend.clone();
                let test_values = test_values.clone();
                let key = key.clone();
                task::spawn(async move { backend.set(&key, &test_values, 60).await })
            })
            .collect();

        let results: Vec<Result<(), CoreError>> = futures::future::join_all(tasks)
            .await
            .into_iter()
            .map(|t| t.unwrap())
            .collect();
        assert!(
            results.iter().all(|r: &Result<(), CoreError>| r.is_ok()),
            "All concurrent SETs should succeed"
        );

        // Concurrent GET operations
        let tasks: Vec<_> = test_keys
            .iter()
            .map(|key| {
                let backend = backend.clone();
                let key = key.clone();
                task::spawn(async move { backend.get(&key).await })
            })
            .collect();

        let results: Vec<Result<Option<Vec<u64>>, CoreError>> = futures::future::join_all(tasks)
            .await
            .into_iter()
            .map(|t| t.unwrap())
            .collect();
        assert!(
            results
                .iter()
                .all(|r: &Result<Option<Vec<u64>>, CoreError>| r.is_ok()),
            "All concurrent GETs should succeed"
        );
        assert!(
            results.iter().all(|r| r.as_ref().unwrap().is_some()),
            "All keys should be found"
        );

        // Concurrent DELETE operations
        let tasks: Vec<_> = test_keys
            .iter()
            .map(|key| {
                let backend = backend.clone();
                let key = key.clone();
                task::spawn(async move { backend.delete(&key).await })
            })
            .collect();

        let results: Vec<Result<(), CoreError>> = futures::future::join_all(tasks)
            .await
            .into_iter()
            .map(|t| t.unwrap())
            .collect();
        assert!(
            results.iter().all(|r: &Result<(), CoreError>| r.is_ok()),
            "All concurrent DELETEs should succeed"
        );
    }
}
