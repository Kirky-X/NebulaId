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

//! Cache module tests using oxcache directly.

#[cfg(test)]
mod tests {
    use oxcache::Cache;

    #[tokio::test]
    async fn test_cache_basic_operations() {
        let cache: Cache<String, Vec<u8>> = Cache::builder()
            .build()
            .await
            .expect("Failed to create cache");

        // set - 需要使用 Vec<u8>
        let value1 = b"value1".to_vec();
        cache
            .set(&"key1".to_string(), &value1)
            .await
            .expect("Failed to set");

        // get
        let value = cache.get(&"key1".to_string()).await.expect("Failed to get");
        assert_eq!(value, Some(value1));

        // exists
        assert!(cache
            .exists(&"key1".to_string())
            .await
            .expect("Failed to check"));
        assert!(!cache
            .exists(&"key2".to_string())
            .await
            .expect("Failed to check"));

        // delete
        cache
            .delete(&"key1".to_string())
            .await
            .expect("Failed to delete");
        assert!(!cache
            .exists(&"key1".to_string())
            .await
            .expect("Failed to check"));
    }

    #[tokio::test]
    async fn test_cache_get_or_fallback() {
        let cache: Cache<String, Vec<u8>> = Cache::builder()
            .build()
            .await
            .expect("Failed to create cache");

        let value: Vec<u8> = cache
            .get_or(&"key1".to_string(), || async { Ok(b"fallback".to_vec()) })
            .await
            .expect("Failed to get_or");
        assert_eq!(value, b"fallback");

        // 再次获取应该从缓存
        let value2: Vec<u8> = cache
            .get_or(&"key1".to_string(), || async {
                std::future::pending().await
            })
            .await
            .expect("Failed to get_or");
        assert_eq!(value2, b"fallback");
    }

    #[tokio::test]
    async fn test_cache_clear() {
        let cache: Cache<String, Vec<u8>> = Cache::builder()
            .build()
            .await
            .expect("Failed to create cache");

        let v1 = b"v1".to_vec();
        let v2 = b"v2".to_vec();
        cache.set(&"k1".to_string(), &v1).await.unwrap();
        cache.set(&"k2".to_string(), &v2).await.unwrap();

        assert!(cache.exists(&"k1".to_string()).await.unwrap());

        cache.clear().await.expect("Failed to clear");

        assert!(!cache.exists(&"k1".to_string()).await.unwrap());
    }

    #[tokio::test]
    async fn test_cache_health_check() {
        let cache: Cache<String, Vec<u8>> = Cache::builder()
            .build()
            .await
            .expect("Failed to create cache");

        // oxcache 0.3.8 health_check returns OxCacheResult<()>; reaching here
        // without panic means the cache is healthy.
        cache.health_check().await.expect("Failed to health check");
    }

    #[tokio::test]
    async fn test_cache_with_json_serialization() {
        use serde::{Deserialize, Serialize};

        #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
        struct TestData {
            id: u64,
            name: String,
        }

        let cache: Cache<String, Vec<u8>> = Cache::builder()
            .build()
            .await
            .expect("Failed to create cache");

        let test_data = TestData {
            id: 42,
            name: "test".to_string(),
        };

        // 序列化并存储
        let bytes = serde_json::to_vec(&test_data).expect("Failed to serialize");
        cache
            .set(&"data".to_string(), &bytes)
            .await
            .expect("Failed to set");

        // 读取并反序列化
        let retrieved = cache.get(&"data".to_string()).await.expect("Failed to get");
        let retrieved_data: TestData =
            serde_json::from_slice(&retrieved.expect("None")).expect("Failed to deserialize");

        assert_eq!(retrieved_data, test_data);
    }
}
