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

//! 认证决策缓存门面（wiring T008）—— 复用 garrison `cache-memory` KV。
//!
//! 目标：消除「每个请求都走 DB + Argon2id」的认证热点。缓存的只是**认证决策
//! 结果**（workspace_id + role + key 自身过期时间），因此必须解决两个正确性
//! 问题：
//!
//! 1. **凭证绑定**：缓存键包含 `sha256(key_secret)`，密钥错误的凭证永远不
//!    会命中缓存（否则只要知道 key_id 就能绕过校验）。
//! 2. **key 生命周期**：缓存条目有效期取 `min(cache_ttl, key 剩余有效期)`，
//!    命中时再次校验 `key_expires_at`，避免「key 已过期但缓存仍在放行」。
//!
//! 吊销（revoke）只有行 `id` 而无 `key_id`，无法精确定位条目，故走 `clear()`
//! 全量失效；轮换（rotate）与 `key_id` 已知的路径走 `invalidate(key_id)`。
//! 缓存是尽力而为的加速层：任何 KV 读写失败都退回 DB 校验并记录日志，绝不影响
//! 认证结论。

use super::memory_dao::MemoryGarrisonDao;
use crate::core::database::ApiKeyRole;
use garrison::dao::GarrisonDao;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

/// 缓存键前缀，隔离于 garrison 自身使用的键空间。
const CACHE_KEY_PREFIX: &str = "nebulaid:auth:apikey:";

/// 缓存的认证决策。刻意不含 `key_secret` 或其哈希 —— 哈希只出现在缓存**键**中。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CachedIdentity {
    /// 认证主体所属 workspace；Admin key 无 workspace 绑定，为 `None`。
    pub workspace_id: Option<Uuid>,
    pub role: ApiKeyRole,
    /// key 自身的绝对过期时间（Unix 秒）；`None` 表示永不过期。
    pub key_expires_at: Option<i64>,
}

impl CachedIdentity {
    /// key 在给定的 Unix 时刻是否已过期。
    fn is_key_expired(&self, now_unix: i64) -> bool {
        self.key_expires_at
            .map(|ts| now_unix >= ts)
            .unwrap_or(false)
    }
}

/// garrison `MemoryGarrisonDao` 之上的认证缓存门面。
pub struct AuthCache {
    dao: MemoryGarrisonDao,
    ttl_seconds: u64,
}

impl AuthCache {
    /// `ttl_seconds` 通常取 `auth.cache_ttl_seconds`；`0` 表示禁用缓存写入。
    pub fn new(ttl_seconds: u64) -> Self {
        Self {
            dao: MemoryGarrisonDao::new(),
            ttl_seconds,
        }
    }

    pub fn ttl_seconds(&self) -> u64 {
        self.ttl_seconds
    }

    fn now_unix() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }

    /// 缓存键：`{prefix}{key_id}:{sha256(key_secret)}`。
    ///
    /// 用哈希而非原文明文 secret，避免密钥出现在键名里（键名会进入长度统计与
    /// 调试输出）；哈希在此处仅作**身份绑定**，不承担口令校验职责。
    fn entry_key(key_id: &str, key_secret: &str) -> String {
        let mut hasher = Sha256::default();
        hasher.update(key_secret.as_bytes());
        format!(
            "{}{}:{}",
            CACHE_KEY_PREFIX,
            key_id,
            hex::encode(hasher.finalize())
        )
    }

    /// 前缀匹配键，用于按 `key_id` 清除其名下所有凭证变体（轮换后旧 secret 等）。
    fn key_id_pattern(key_id: &str) -> String {
        format!("{}{}:*", CACHE_KEY_PREFIX, key_id)
    }

    /// 查缓存；命中但 key 已过期时删除条目并返回 `None`（回源 DB）。
    pub async fn get(&self, key_id: &str, key_secret: &str) -> Option<CachedIdentity> {
        let entry_key = Self::entry_key(key_id, key_secret);
        let raw = match self.dao.get(&entry_key).await {
            Ok(Some(raw)) => raw,
            Ok(None) => return None,
            Err(e) => {
                tracing::warn!(error = %e, key_id = %key_id, "auth cache lookup failed; falling back to repository");
                return None;
            }
        };
        let identity: CachedIdentity = match serde_json::from_str(&raw) {
            Ok(identity) => identity,
            Err(e) => {
                // 脏数据不能当作有效凭证使用。
                tracing::warn!(error = %e, key_id = %key_id, "auth cache entry malformed; dropping");
                let _ = self.dao.delete(&entry_key).await;
                return None;
            }
        };
        if identity.is_key_expired(Self::now_unix()) {
            let _ = self.dao.delete(&entry_key).await;
            return None;
        }
        Some(identity)
    }

    /// 写缓存，TTL = `min(cache_ttl, key 剩余有效期)`；剩余有效期已耗尽则不写。
    pub async fn put(&self, key_id: &str, key_secret: &str, identity: &CachedIdentity) {
        if self.ttl_seconds == 0 {
            return;
        }
        let now = Self::now_unix();
        let ttl = match identity.key_expires_at {
            Some(expires_at) => {
                let remaining = expires_at - now;
                if remaining <= 0 {
                    return;
                }
                self.ttl_seconds.min(remaining as u64)
            }
            None => self.ttl_seconds,
        };
        let value = match serde_json::to_string(identity) {
            Ok(value) => value,
            Err(e) => {
                tracing::warn!(error = %e, key_id = %key_id, "failed to serialize auth cache entry; skipping cache write");
                return;
            }
        };
        if let Err(e) = self
            .dao
            .set(&Self::entry_key(key_id, key_secret), &value, ttl)
            .await
        {
            tracing::warn!(error = %e, key_id = %key_id, "auth cache write failed; requests will hit repository");
        }
    }

    /// 失效指定 `key_id` 名下的全部条目（轮换、禁用、删除 key 时调用）。
    pub async fn invalidate(&self, key_id: &str) {
        let pattern = Self::key_id_pattern(key_id);
        let keys = match self.dao.keys(&pattern).await {
            Ok(keys) => keys,
            Err(e) => {
                // 无法枚举就不敢保留：整体清空，避免残留有效条目。
                tracing::warn!(error = %e, key_id = %key_id, "auth cache invalidation scan failed; clearing whole cache");
                self.clear().await;
                return;
            }
        };
        for key in keys {
            if let Err(e) = self.dao.delete(&key).await {
                tracing::warn!(error = %e, "auth cache entry delete failed");
            }
        }
    }

    /// 清空全部认证缓存（吊销路径：只持有行 `id`，无法定位 `key_id`）。
    pub async fn clear(&self) {
        let pattern = format!("{}*", CACHE_KEY_PREFIX);
        let keys = match self.dao.keys(&pattern).await {
            Ok(keys) => keys,
            Err(e) => {
                tracing::warn!(error = %e, "auth cache scan failed; entries may persist until TTL");
                return;
            }
        };
        let removed = keys.len();
        for key in keys {
            if let Err(e) = self.dao.delete(&key).await {
                tracing::warn!(error = %e, "auth cache entry delete failed");
            }
        }
        tracing::debug!(entries = removed, "auth cache cleared");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(workspace_id: Option<Uuid>, expires_at: Option<i64>) -> CachedIdentity {
        CachedIdentity {
            workspace_id,
            role: ApiKeyRole::User,
            key_expires_at: expires_at,
        }
    }

    #[tokio::test]
    async fn test_hit_after_put_preserves_identity() {
        let cache = AuthCache::new(300);
        let ws = Uuid::new_v4();
        cache.put("k1", "s1", &identity(Some(ws), None)).await;

        assert_eq!(
            cache.get("k1", "s1").await,
            Some(identity(Some(ws), None)),
            "写入后同凭证必须命中并原样返回 workspace/role"
        );
    }

    #[tokio::test]
    async fn test_wrong_secret_never_hits() {
        let cache = AuthCache::new(300);
        cache.put("k1", "s1", &identity(None, None)).await;

        assert!(
            cache.get("k1", "wrong-secret").await.is_none(),
            "仅知道 key_id 不得命中他人凭证的缓存"
        );
    }

    #[tokio::test]
    async fn test_expired_key_entry_is_not_served() {
        let cache = AuthCache::new(300);
        // key 已在过去过期（理论上不会写入，这里直接构造条目验证读取侧防线）
        let key = AuthCache::entry_key("k1", "s1");
        let value = serde_json::to_string(&identity(None, Some(1))).unwrap();
        cache.dao.set(&key, &value, 300).await.unwrap();

        assert!(
            cache.get("k1", "s1").await.is_none(),
            "过期 key 的缓存条目必须视为未命中"
        );
        assert!(
            cache.dao.get(&key).await.unwrap().is_none(),
            "过期条目应被顺手清理"
        );
    }

    #[tokio::test]
    async fn test_invalidate_removes_all_entries_of_key_id() {
        let cache = AuthCache::new(300);
        cache.put("k1", "s1", &identity(None, None)).await;
        // 同一 key_id 的旧 secret 变体（轮换宽限期场景）
        cache.put("k1", "s0", &identity(None, None)).await;
        cache.put("k2", "s1", &identity(None, None)).await;

        cache.invalidate("k1").await;

        assert!(cache.get("k1", "s1").await.is_none());
        assert!(cache.get("k1", "s0").await.is_none());
        assert!(
            cache.get("k2", "s1").await.is_some(),
            "invalidate 不得影响其他 key_id"
        );
    }

    #[tokio::test]
    async fn test_clear_removes_everything() {
        let cache = AuthCache::new(300);
        cache.put("k1", "s1", &identity(None, None)).await;
        cache.put("k2", "s2", &identity(None, None)).await;

        cache.clear().await;

        assert!(cache.get("k1", "s1").await.is_none());
        assert!(cache.get("k2", "s2").await.is_none());
    }

    #[tokio::test]
    async fn test_ttl_zero_disables_caching() {
        let cache = AuthCache::new(0);
        cache.put("k1", "s1", &identity(None, None)).await;
        assert!(
            cache.get("k1", "s1").await.is_none(),
            "cache_ttl_seconds=0 时不得写入缓存"
        );
    }

    #[tokio::test]
    async fn test_malformed_entry_is_dropped_not_served() {
        let cache = AuthCache::new(300);
        let key = AuthCache::entry_key("k1", "s1");
        cache.dao.set(&key, "not-json", 300).await.unwrap();

        assert!(cache.get("k1", "s1").await.is_none());
    }

    #[test]
    fn test_cached_identity_serialization_contains_no_secret_material() {
        // R-auth-001：缓存值仅含授权决策三要素。字段集合用穷举断言钉死，
        // 未来给 CachedIdentity 加字段时此测试强制评审是否引入敏感数据。
        let value = serde_json::to_value(&identity(Some(Uuid::new_v4()), Some(42))).unwrap();
        let obj = value
            .as_object()
            .expect("cached identity serializes to object");
        let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(keys, vec!["key_expires_at", "role", "workspace_id"]);
    }
}
