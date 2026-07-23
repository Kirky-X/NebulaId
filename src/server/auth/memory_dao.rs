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

//! 进程内 `GarrisonDao` 实现。
//!
//! 用 `parking_lot::RwLock<HashMap<String, Entry>>` 存储，每个 entry 附带
//! `Option<Instant>` 过期时间。所有读写操作在同一个 `RwLock` 临界区内完成，
//! 保证 `get_and_delete` / `incr` / `decr` / `compare_and_update_if_greater`
//! 等原子操作的进程内原子性。
//!
//! 适用场景：单实例部署、集成测试、开发环境。多实例部署需使用 Redis 后端
//! （garrison `cache-redis` feature）。

use async_trait::async_trait;
use garrison::dao::GarrisonDao;
use garrison::error::{GarrisonError, GarrisonResult};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// 单条存储记录。
#[derive(Clone)]
struct Entry {
    value: String,
    /// 过期绝对时间；`None` 表示永久驻留（TTL = 0）。
    expire_at: Option<Instant>,
}

impl Entry {
    fn is_expired(&self) -> bool {
        self.expire_at.map(|t| Instant::now() >= t).unwrap_or(false)
    }

    fn remaining_ttl(&self) -> Option<Duration> {
        self.expire_at
            .and_then(|t| t.checked_duration_since(Instant::now()))
    }
}

/// 进程内 `GarrisonDao` 实现。
///
/// 用 `parking_lot::RwLock<HashMap>` 存储 KV，支持 per-entry TTL、glob 扫描、
/// 原子 get_and_delete / incr / decr / CAS。所有原子操作在写锁临界区内完成。
pub struct MemoryGarrisonDao {
    inner: RwLock<HashMap<String, Entry>>,
}

impl MemoryGarrisonDao {
    /// 创建空的 DAO 实例。
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(HashMap::new()),
        }
    }

    /// 当前存储的 key 数量（含已过期但未惰性清理的 entry）。
    pub fn len(&self) -> usize {
        self.inner.read().len()
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.inner.read().is_empty()
    }
}

impl Default for MemoryGarrisonDao {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl GarrisonDao for MemoryGarrisonDao {
    async fn get(&self, key: &str) -> GarrisonResult<Option<String>> {
        let map = self.inner.read();
        match map.get(key) {
            Some(entry) if !entry.is_expired() => Ok(Some(entry.value.clone())),
            Some(_) => Ok(None),
            None => Ok(None),
        }
    }

    async fn set(&self, key: &str, value: &str, ttl_seconds: u64) -> GarrisonResult<()> {
        let expire_at = if ttl_seconds == 0 {
            None
        } else {
            Some(Instant::now() + Duration::from_secs(ttl_seconds))
        };
        let mut map = self.inner.write();
        map.insert(
            key.to_string(),
            Entry {
                value: value.to_string(),
                expire_at,
            },
        );
        Ok(())
    }

    async fn update(&self, key: &str, value: &str) -> GarrisonResult<()> {
        let mut map = self.inner.write();
        match map.get_mut(key) {
            Some(entry) if !entry.is_expired() => {
                entry.value = value.to_string();
                Ok(())
            }
            Some(_) => Err(GarrisonError::Dao(format!("dao-key-expired::{}", key))),
            None => Err(GarrisonError::Dao(format!("dao-key-not-found::{}", key))),
        }
    }

    async fn expire(&self, key: &str, seconds: u64) -> GarrisonResult<()> {
        let mut map = self.inner.write();
        match map.get_mut(key) {
            Some(entry) if !entry.is_expired() => {
                entry.expire_at = if seconds == 0 {
                    None
                } else {
                    Some(Instant::now() + Duration::from_secs(seconds))
                };
                Ok(())
            }
            Some(_) => Err(GarrisonError::Dao(format!("dao-key-expired::{}", key))),
            None => Err(GarrisonError::Dao(format!("dao-key-not-found::{}", key))),
        }
    }

    async fn delete(&self, key: &str) -> GarrisonResult<()> {
        let mut map = self.inner.write();
        map.remove(key);
        Ok(())
    }

    async fn set_permanent(&self, key: &str, value: &str) -> GarrisonResult<()> {
        let mut map = self.inner.write();
        map.insert(
            key.to_string(),
            Entry {
                value: value.to_string(),
                expire_at: None,
            },
        );
        Ok(())
    }

    async fn get_timeout(&self, key: &str) -> GarrisonResult<Option<Duration>> {
        let map = self.inner.read();
        match map.get(key) {
            Some(entry) if !entry.is_expired() => Ok(entry.remaining_ttl()),
            Some(_) => Ok(None),
            None => Ok(None),
        }
    }

    async fn get_with_ttl(&self, key: &str) -> GarrisonResult<Option<(String, Option<Duration>)>> {
        let map = self.inner.read();
        match map.get(key) {
            Some(entry) if !entry.is_expired() => {
                Ok(Some((entry.value.clone(), entry.remaining_ttl())))
            }
            Some(_) => Ok(None),
            None => Ok(None),
        }
    }

    async fn keys(&self, pattern: &str) -> GarrisonResult<Vec<String>> {
        if pattern.len() > 256 {
            return Err(GarrisonError::InvalidParam(format!(
                "dao-keys-pattern-too-long::{}",
                pattern.len()
            )));
        }
        let map = self.inner.read();
        let mut result = Vec::new();
        let now = Instant::now();
        for (key, entry) in map.iter() {
            if entry.expire_at.map(|t| now >= t).unwrap_or(false) {
                continue;
            }
            if glob_match(pattern, key) {
                result.push(key.clone());
            }
        }
        Ok(result)
    }

    async fn rename(&self, old_key: &str, new_key: &str) -> GarrisonResult<()> {
        let mut map = self.inner.write();
        let entry = map
            .get(old_key)
            .filter(|e| !e.is_expired())
            .cloned()
            .ok_or_else(|| GarrisonError::InvalidParam(format!("dao-key-missing::{}", old_key)))?;
        let value = entry.value.clone();
        let expire_at = entry.expire_at;
        map.insert(new_key.to_string(), Entry { value, expire_at });
        map.remove(old_key);
        Ok(())
    }

    async fn get_and_delete(&self, key: &str) -> GarrisonResult<Option<String>> {
        let mut map = self.inner.write();
        match map.remove(key) {
            Some(entry) if !entry.is_expired() => Ok(Some(entry.value)),
            Some(_) => Ok(None),
            None => Ok(None),
        }
    }

    async fn incr(&self, key: &str, ttl_seconds: u64) -> GarrisonResult<u64> {
        let mut map = self.inner.write();
        match map.get_mut(key) {
            Some(entry) if !entry.is_expired() => {
                let cur: u64 = entry
                    .value
                    .parse()
                    .map_err(|_| GarrisonError::Dao(format!("dao-incr-parse-u64::{}", key)))?;
                let new_val = cur
                    .checked_add(1)
                    .ok_or_else(|| GarrisonError::Dao(format!("dao-incr-overflow::{}", key)))?;
                entry.value = new_val.to_string();
                Ok(new_val)
            }
            Some(_) => {
                map.insert(
                    key.to_string(),
                    Entry {
                        value: "1".to_string(),
                        expire_at: if ttl_seconds == 0 {
                            None
                        } else {
                            Some(Instant::now() + Duration::from_secs(ttl_seconds))
                        },
                    },
                );
                Ok(1)
            }
            None => {
                map.insert(
                    key.to_string(),
                    Entry {
                        value: "1".to_string(),
                        expire_at: if ttl_seconds == 0 {
                            None
                        } else {
                            Some(Instant::now() + Duration::from_secs(ttl_seconds))
                        },
                    },
                );
                Ok(1)
            }
        }
    }

    async fn decr(&self, key: &str) -> GarrisonResult<u64> {
        let mut map = self.inner.write();
        match map.get_mut(key) {
            Some(entry) if !entry.is_expired() => {
                let cur: u64 = entry
                    .value
                    .parse()
                    .map_err(|_| GarrisonError::Dao(format!("dao-decr-parse-u64::{}", key)))?;
                if cur == 0 {
                    Ok(0)
                } else {
                    let new_val = cur - 1;
                    if new_val == 0 {
                        map.remove(key);
                    } else {
                        entry.value = new_val.to_string();
                    }
                    Ok(new_val)
                }
            }
            _ => Ok(0),
        }
    }

    async fn compare_and_update_if_greater(
        &self,
        key: &str,
        new_value: u64,
        ttl_seconds: u64,
    ) -> GarrisonResult<bool> {
        let mut map = self.inner.write();
        let current: u64 = match map.get(key) {
            Some(entry) if !entry.is_expired() => entry.value.parse().map_err(|_| {
                GarrisonError::Dao(format!("dao-cas-parse-u64::{}::{}", key, entry.value))
            })?,
            _ => 0,
        };
        if new_value > current {
            map.insert(
                key.to_string(),
                Entry {
                    value: new_value.to_string(),
                    expire_at: if ttl_seconds == 0 {
                        None
                    } else {
                        Some(Instant::now() + Duration::from_secs(ttl_seconds))
                    },
                },
            );
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

/// 简化版 glob 匹配，支持 `*`（任意字符序列）与 `?`（单字符）。
///
/// 与 garrison 内部 `dao::tests::glob_match` 行为一致（`pub(crate)` 不可外部复用）。
fn glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    glob_match_inner(&p, 0, &t, 0)
}

fn glob_match_inner(p: &[char], mut pi: usize, t: &[char], mut ti: usize) -> bool {
    while pi < p.len() {
        match p[pi] {
            '*' => {
                let rest = &p[pi + 1..];
                for skip in 0..=t.len().saturating_sub(ti) {
                    if glob_match_inner(rest, 0, t, ti + skip) {
                        return true;
                    }
                }
                return false;
            }
            '?' => {
                if ti >= t.len() {
                    return false;
                }
                pi += 1;
                ti += 1;
            }
            c => {
                if ti >= t.len() || t[ti] != c {
                    return false;
                }
                pi += 1;
                ti += 1;
            }
        }
    }
    ti == t.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_set_get_basic() {
        let dao = MemoryGarrisonDao::new();
        dao.set("k1", "v1", 0).await.unwrap();
        assert_eq!(dao.get("k1").await.unwrap(), Some("v1".to_string()));
    }

    #[tokio::test]
    async fn test_set_with_ttl_expires() {
        let dao = MemoryGarrisonDao::new();
        dao.set("k1", "v1", 1).await.unwrap();
        assert_eq!(dao.get("k1").await.unwrap(), Some("v1".to_string()));
        tokio::time::sleep(Duration::from_millis(1100)).await;
        assert_eq!(dao.get("k1").await.unwrap(), None);
    }

    #[tokio::test]
    async fn test_get_timeout_permanent() {
        let dao = MemoryGarrisonDao::new();
        dao.set_permanent("k1", "v1").await.unwrap();
        assert_eq!(dao.get_timeout("k1").await.unwrap(), None);
    }

    #[tokio::test]
    async fn test_get_timeout_with_ttl() {
        let dao = MemoryGarrisonDao::new();
        dao.set("k1", "v1", 100).await.unwrap();
        let ttl = dao.get_timeout("k1").await.unwrap().unwrap();
        assert!(ttl <= Duration::from_secs(100));
        assert!(ttl > Duration::from_secs(98));
    }

    #[tokio::test]
    async fn test_update_preserves_ttl() {
        let dao = MemoryGarrisonDao::new();
        dao.set("k1", "v1", 100).await.unwrap();
        let ttl_before = dao.get_timeout("k1").await.unwrap().unwrap();
        tokio::time::sleep(Duration::from_millis(10)).await;
        dao.update("k1", "v2").await.unwrap();
        assert_eq!(dao.get("k1").await.unwrap(), Some("v2".to_string()));
        let ttl_after = dao.get_timeout("k1").await.unwrap().unwrap();
        assert!(ttl_after <= ttl_before);
    }

    #[tokio::test]
    async fn test_update_missing_key_errors() {
        let dao = MemoryGarrisonDao::new();
        let err = dao.update("missing", "v").await.unwrap_err();
        assert!(matches!(err, GarrisonError::Dao(_)));
    }

    #[tokio::test]
    async fn test_expire_resets_ttl() {
        let dao = MemoryGarrisonDao::new();
        dao.set("k1", "v1", 100).await.unwrap();
        dao.expire("k1", 50).await.unwrap();
        let ttl = dao.get_timeout("k1").await.unwrap().unwrap();
        assert!(ttl <= Duration::from_secs(50));
    }

    #[tokio::test]
    async fn test_delete() {
        let dao = MemoryGarrisonDao::new();
        dao.set("k1", "v1", 0).await.unwrap();
        dao.delete("k1").await.unwrap();
        assert_eq!(dao.get("k1").await.unwrap(), None);
    }

    #[tokio::test]
    async fn test_keys_glob_pattern() {
        let dao = MemoryGarrisonDao::new();
        dao.set("garrison:apikey:default:abc", "v1", 0)
            .await
            .unwrap();
        dao.set("garrison:apikey:default:def", "v2", 0)
            .await
            .unwrap();
        dao.set("garrison:apikey:idx:abc", "v3", 0).await.unwrap();
        dao.set("other:key", "v4", 0).await.unwrap();
        let mut keys = dao.keys("garrison:apikey:default:*").await.unwrap();
        keys.sort();
        assert_eq!(keys.len(), 2);
        assert_eq!(keys[0], "garrison:apikey:default:abc");
        assert_eq!(keys[1], "garrison:apikey:default:def");
    }

    #[tokio::test]
    async fn test_keys_question_mark() {
        let dao = MemoryGarrisonDao::new();
        dao.set("k1", "v", 0).await.unwrap();
        dao.set("k2", "v", 0).await.unwrap();
        dao.set("k12", "v", 0).await.unwrap();
        let mut keys = dao.keys("k?").await.unwrap();
        keys.sort();
        assert_eq!(keys, vec!["k1", "k2"]);
    }

    #[tokio::test]
    async fn test_get_and_delete_atomic() {
        let dao = MemoryGarrisonDao::new();
        dao.set("k1", "v1", 0).await.unwrap();
        assert_eq!(
            dao.get_and_delete("k1").await.unwrap(),
            Some("v1".to_string())
        );
        assert_eq!(dao.get("k1").await.unwrap(), None);
    }

    #[tokio::test]
    async fn test_incr_new_key() {
        let dao = MemoryGarrisonDao::new();
        assert_eq!(dao.incr("counter", 100).await.unwrap(), 1);
        assert_eq!(dao.incr("counter", 100).await.unwrap(), 2);
        assert_eq!(dao.incr("counter", 100).await.unwrap(), 3);
    }

    #[tokio::test]
    async fn test_decr_deletes_at_zero() {
        let dao = MemoryGarrisonDao::new();
        dao.set("counter", "2", 100).await.unwrap();
        assert_eq!(dao.decr("counter").await.unwrap(), 1);
        assert_eq!(dao.decr("counter").await.unwrap(), 0);
        assert_eq!(dao.get("counter").await.unwrap(), None);
    }

    #[tokio::test]
    async fn test_decr_nonexistent_returns_zero() {
        let dao = MemoryGarrisonDao::new();
        assert_eq!(dao.decr("missing").await.unwrap(), 0);
    }

    #[tokio::test]
    async fn test_compare_and_update_if_greater() {
        let dao = MemoryGarrisonDao::new();
        assert!(dao
            .compare_and_update_if_greater("nc", 5, 100)
            .await
            .unwrap());
        assert!(!dao
            .compare_and_update_if_greater("nc", 3, 100)
            .await
            .unwrap());
        assert!(dao
            .compare_and_update_if_greater("nc", 6, 100)
            .await
            .unwrap());
        assert_eq!(dao.get("nc").await.unwrap(), Some("6".to_string()));
    }

    #[tokio::test]
    async fn test_rename_preserves_ttl() {
        let dao = MemoryGarrisonDao::new();
        dao.set("old", "v1", 100).await.unwrap();
        dao.rename("old", "new").await.unwrap();
        assert_eq!(dao.get("old").await.unwrap(), None);
        assert_eq!(dao.get("new").await.unwrap(), Some("v1".to_string()));
        let ttl = dao.get_timeout("new").await.unwrap().unwrap();
        assert!(ttl <= Duration::from_secs(100));
    }

    #[tokio::test]
    async fn test_glob_match_lifecycle() {
        assert!(glob_match("hello", "hello"));
        assert!(glob_match("hel*", "hello"));
        assert!(glob_match("h?llo", "hello"));
        assert!(glob_match("*", "anything"));
        assert!(glob_match(
            "garrison:apikey:*:abc",
            "garrison:apikey:default:abc"
        ));
        assert!(!glob_match(
            "garrison:apikey:*:abc",
            "garrison:apikey:default:def"
        ));
        assert!(!glob_match("hello", "hell"));
    }
}
