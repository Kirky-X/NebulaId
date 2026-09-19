// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! 认证决策缓存门面 —— 进程内类型化缓存，带 TTL 抖动与回源 single-flight。
//!
//! 存储是**类型化**的进程内条目表（`Arc<CachedIdentity>` 直接驻留）。早期版本
//! 曾借用 garrison `GarrisonDaoOxcache`（oxcache L1 内存后端），但其 `GarrisonDao`
//! 接口是 `String` 值 KV，每个条目都要 `serde_json` 序列化/反序列化一个往返；
//! 认证决策只是三元组小结构，类型化直存后命中路径零序列化开销。随之而来的
//! 两点退化已在门面内补齐：
//!
//! * garrison 的 glob 扫描（`keys(pattern)`）被**精确前缀删除**取代 —— 不存在
//!   通配语义，`key_id` 中的 `*`/`?` 天然按字面量处理（回归钉桩见
//!   `test_invalidate_does_not_treat_glob_chars_in_key_id_as_pattern`，该测试
//!   正是因 garrison glob 丢条目而失败的基线项）；
//! * garrison 的 Moka 容量淘汰被**条目数硬上限**（[`MAX_AUTH_CACHE_ENTRIES`]，
//!   与 `api_key_auth` 失败表同量级）取代：插入触顶时先清扫已过期条目，仍满
//!   则逐出最早过期者。
//!
//! 目标：消除「每个请求都走 DB + Argon2id」的认证热点。缓存的只是**认证决策
//! 结果**（workspace_id + role + key 自身过期时间），因此必须解决两个正确性
//! 问题：
//!
//! 1. **凭证绑定**：缓存键包含 `sha256(key_secret)`，密钥错误的凭证永远不
//!    会命中缓存（否则只要知道 key_id 就能绕过校验）。
//! 2. **key 生命周期**：缓存条目有效期取 `min(cache_ttl, key 剩余有效期)`，
//!    且对 `cache_ttl` 施加每条目独立的 ±10% 抖动（防同批 key 同时过期集体
//!    回源的雪崩），命中时再次校验 `key_expires_at`，避免「key 已过期但缓存
//!    仍在放行」。
//!
//! 此外提供 [`AuthCache::get_or_load`]：miss 回源的 single-flight —— 并发未
//! 命中同一凭证时只有一个调用方真正回源（DB + Argon2id），其余等待者共享
//! 同一结果（含回源失败 `None`），完成后清理在途表项。
//!
//! 吊销（revoke）只有行 `id` 而无 `key_id`，无法精确定位条目，故走 `clear()`
//! 全量失效；轮换（rotate）与 `key_id` 已知的路径走 `invalidate(key_id)`。
//! **失效语义只对本进程即时生效**：缓存是进程内的，多节点部署时其他节点最长
//! 滞后一个 `cache_ttl` 才会停止放行旧决策（运维口径见 docs/DEPLOYMENT.md）。
//! 同理，命中缓存的请求不回写 DB，`last_used_at` 最多滞后一个 `cache_ttl`。
//! 缓存是尽力而为的加速层：读写失败绝不影响认证结论。

use crate::core::database::ApiKeyRole;
use parking_lot::Mutex;
use rand::RngExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::OnceCell;
use uuid::Uuid;

/// 缓存键前缀，隔离于其他进程内键空间。
const CACHE_KEY_PREFIX: &str = "nebulaid:auth:apikey:";

/// 缓存条目数硬上限。防伪造 key_id 洪泛让条目表无界增长；与 `api_key_auth`
/// 失败表（`MAX_TRACKED_AUTH_FAILURE_IPS`）同一量级。
const MAX_AUTH_CACHE_ENTRIES: usize = 10_000;

/// single-flight 在途表容量上限。正常情况下表项随回源完成即清理，只有
/// 超大规模**并发** distinct-key miss 才可能触顶；触顶时先清理已完成待
/// 回收的表项，仍满则整表清空（单飞暂时退化为各回源，正确性不受影响）。
const MAX_INFLIGHT_LOOKUPS: usize = 10_000;

/// TTL 抖动幅度：±10%。
const TTL_JITTER_RATIO: f64 = 0.1;

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

/// 单个缓存条目。TTL 已在写入时完成 ±10% 抖动并折算成绝对时刻，
/// 读取侧只需一次 `Instant` 比较。
#[derive(Debug)]
struct CacheEntry {
    identity: Arc<CachedIdentity>,
    expires_at: Instant,
}

/// single-flight 在途表：缓存键 -> 共享初始化单元。并发 miss 同 key 只有一个
/// 调用方执行回源，其余等待者复用同一 `OnceCell`（含 `None` 结果）。
type InflightCell = Arc<OnceCell<Option<Arc<CachedIdentity>>>>;
type InflightMap = Arc<Mutex<HashMap<String, InflightCell>>>;

/// 类型化条目表之上的认证缓存门面。
pub struct AuthCache {
    /// 条目表：缓存键 -> 条目。`Arc<Mutex<..>>` 与仓储节流表同理 ——
    /// `AuthCache` 一律以 `Arc<AuthCache>` 共享，包一层为将来克隆语义留余地。
    entries: Arc<Mutex<HashMap<String, CacheEntry>>>,
    /// 回源 single-flight 在途表。完成后表项即清理。
    inflight: InflightMap,
    ttl_seconds: u64,
}

impl AuthCache {
    /// `ttl_seconds` 通常取 `auth.cache_ttl_seconds`；`0` 表示禁用缓存写入。
    ///
    /// 保留 `async` 形态：调用方（main.rs 装配、api_key_auth 测试）已按
    /// 异步构造使用，门面存储不再依赖异步初始化的后端。
    pub async fn new(ttl_seconds: u64) -> Self {
        Self {
            entries: Arc::new(Mutex::new(HashMap::new())),
            inflight: Arc::new(Mutex::new(HashMap::new())),
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
    /// 用哈希而非原文明文 secret，避免密钥出现在键名里（键名会进入调试输出）；
    /// 哈希在此处仅作**身份绑定**，不承担口令校验职责。
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

    /// 按 `key_id` 定位条目时的精确前缀（轮换后同一 key_id 可能有多个 secret 变体）。
    fn key_id_prefix(key_id: &str) -> String {
        format!("{}{}:", CACHE_KEY_PREFIX, key_id)
    }

    /// 缓存 TTL 的 ±10% 抖动（每条目写入时独立抽取）。
    ///
    /// 同批发放的 key 若 TTL 一致，会同时过期并在同一瞬间集体回源（缓存
    /// 雪崩）；抖动把过期时刻摊开到 `[(1-j)T, (1+j)T]`。
    fn jittered_ttl(ttl_seconds: u64) -> u64 {
        if ttl_seconds == 0 {
            return 0;
        }
        let factor = rand::rng().random_range(1.0 - TTL_JITTER_RATIO..1.0 + TTL_JITTER_RATIO);
        ((ttl_seconds as f64 * factor).round() as u64).max(1)
    }

    /// 读取条目；已过 TTL 的条目视为未命中并顺手清理。命中返回决策的克隆
    /// （三元组小结构，远廉价于旧实现的 serde_json 反序列化）。
    fn lookup(&self, entry_key: &str) -> Option<CachedIdentity> {
        let mut entries = self.entries.lock();
        if entries
            .get(entry_key)
            .is_some_and(|entry| entry.expires_at <= Instant::now())
        {
            entries.remove(entry_key);
            return None;
        }
        entries.get(entry_key).map(|e| (*e.identity).clone())
    }

    /// 写入条目，TTL = `min(±10% 抖动后的 cache_ttl, key 剩余有效期)`；
    /// 剩余有效期已耗尽或缓存禁用则不写。触顶时先清扫过期条目，仍满则
    /// 逐出最早过期者（最旧）。
    fn store(&self, entry_key: &str, identity: Arc<CachedIdentity>) {
        if self.ttl_seconds == 0 {
            return;
        }
        let now_unix = Self::now_unix();
        let ttl = match identity.key_expires_at {
            Some(expires_at) => {
                let remaining = expires_at - now_unix;
                if remaining <= 0 {
                    return;
                }
                // 抖动只作用于缓存 TTL 自身，再被 key 剩余寿命钳制 ——
                // 抖动不得延长 key 的绝对有效期。
                Self::jittered_ttl(self.ttl_seconds).min(remaining as u64)
            }
            None => Self::jittered_ttl(self.ttl_seconds),
        };
        let entry = CacheEntry {
            identity,
            expires_at: Instant::now() + Duration::from_secs(ttl.max(1)),
        };
        let mut entries = self.entries.lock();
        if entries.len() >= MAX_AUTH_CACHE_ENTRIES {
            let now = Instant::now();
            entries.retain(|_, e| e.expires_at > now);
            if entries.len() >= MAX_AUTH_CACHE_ENTRIES {
                if let Some(oldest) = entries
                    .iter()
                    .min_by_key(|(_, e)| e.expires_at)
                    .map(|(k, _)| k.clone())
                {
                    entries.remove(&oldest);
                }
            }
        }
        entries.insert(entry_key.to_string(), entry);
    }

    /// 查缓存；命中但 key 已过期时删除条目并返回 `None`（回源 DB）。
    pub async fn get(&self, key_id: &str, key_secret: &str) -> Option<CachedIdentity> {
        let identity = self.lookup(&Self::entry_key(key_id, key_secret))?;
        if identity.is_key_expired(Self::now_unix()) {
            self.entries
                .lock()
                .remove(&Self::entry_key(key_id, key_secret));
            return None;
        }
        Some(identity)
    }

    /// 写缓存。TTL 见 [`Self::store`]。
    pub async fn put(&self, key_id: &str, key_secret: &str, identity: &CachedIdentity) {
        self.store(
            &Self::entry_key(key_id, key_secret),
            Arc::new(identity.clone()),
        );
    }

    /// miss 回源 single-flight：未命中时回源，**并发**未命中同一凭证只执行
    /// 一次 `load`，其余等待者共享同一结果（含 `None`）；完成后清理在途表项。
    ///
    /// - 缓存命中：直接返回，`load` 不被调用；
    /// - 回源成功（`Some`）：写入条目（带抖动 TTL），全部等待者收到同一决策；
    /// - 回源失败（`None`）：不缓存（认证失败不得被钉住），等待者同样收到
    ///   `None`，后续调用重新回源。
    ///
    /// 在途表容量上限 [`MAX_INFLIGHT_LOOKUPS`]：触顶先清理已完成表项，仍满
    /// 则整表清空 —— 单飞暂时退化为各自回源，正确性不受影响。
    pub async fn get_or_load<F, Fut>(
        &self,
        key_id: &str,
        key_secret: &str,
        load: F,
    ) -> Option<CachedIdentity>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Option<CachedIdentity>>,
    {
        let entry_key = Self::entry_key(key_id, key_secret);
        if let Some(hit) = self.lookup(&entry_key) {
            return Some(hit);
        }

        // 登记/共享在途单元：同 key 的并发 miss 拿到同一个 OnceCell。
        // 临界区内无 await（parking_lot 短临界区 + 立即释放）。
        let cell = {
            let mut inflight = self.inflight.lock();
            if inflight.len() >= MAX_INFLIGHT_LOOKUPS {
                inflight.retain(|_, cell| cell.get().is_some());
                if inflight.len() >= MAX_INFLIGHT_LOOKUPS {
                    inflight.clear();
                }
            }
            inflight
                .entry(entry_key.clone())
                .or_insert_with(|| Arc::new(OnceCell::new()))
                .clone()
        };

        // 只有首个到达者真正执行 load；其余等待同一结果。
        let loaded: Option<Arc<CachedIdentity>> = cell
            .get_or_init(|| async { load().await.map(Arc::new) })
            .await
            .clone();

        // 完成即清理：仅当表内仍是本 cell（避免误删后来者新建的单元）。
        {
            let mut inflight = self.inflight.lock();
            if inflight
                .get(&entry_key)
                .is_some_and(|current| Arc::ptr_eq(current, &cell))
            {
                inflight.remove(&entry_key);
            }
        }

        match loaded {
            Some(identity) => {
                self.store(&entry_key, Arc::clone(&identity));
                Some((*identity).clone())
            }
            None => None,
        }
    }

    /// 失效指定 `key_id` 名下的全部条目（轮换、禁用、删除 key 时调用）。
    ///
    /// 类型化表上的失效就是**精确前缀删除**：不存在 glob 通配语义，`key_id`
    /// 源自客户端提交的凭证，含 `*`/`?` 时按字面量匹配，不可能越界删除他人
    /// 条目（garrison glob 扫描时代的回归钉桩见对应测试）。
    pub async fn invalidate(&self, key_id: &str) {
        let prefix = Self::key_id_prefix(key_id);
        self.entries
            .lock()
            .retain(|key, _| !key.starts_with(&prefix));
    }

    /// 清空全部认证缓存（吊销路径：只持有行 `id`，无法定位 `key_id`）。
    pub async fn clear(&self) {
        let removed = {
            let mut entries = self.entries.lock();
            let removed = entries.len();
            entries.clear();
            removed
        };
        tracing::debug!(entries = removed, "auth cache cleared");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn identity(workspace_id: Option<Uuid>, expires_at: Option<i64>) -> CachedIdentity {
        CachedIdentity {
            workspace_id,
            role: ApiKeyRole::User,
            key_expires_at: expires_at,
        }
    }

    fn stored_entry(cache: &AuthCache, key_id: &str, key_secret: &str) -> Option<CacheEntry> {
        cache
            .entries
            .lock()
            .get(&AuthCache::entry_key(key_id, key_secret))
            .map(|e| CacheEntry {
                identity: Arc::clone(&e.identity),
                expires_at: e.expires_at,
            })
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_hit_after_put_preserves_identity() {
        let cache = AuthCache::new(300).await;
        let ws = Uuid::new_v4();
        cache.put("k1", "s1", &identity(Some(ws), None)).await;

        assert_eq!(
            cache.get("k1", "s1").await,
            Some(identity(Some(ws), None)),
            "写入后同凭证必须命中并原样返回 workspace/role"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_typed_entry_is_shared_without_reserialization() {
        // 钉住「值以 Arc 直存」的改造点：put 之后表内条目的 identity 必须是
        // Arc 共享（同 put 无第二次克隆产生的新指针语义无从断言，这里断言
        // 连续两次 get 拿到**相等**决策，且表内确实存在类型化条目 —— 旧
        // garrison String KV 形态下不存在这一层）。
        let cache = AuthCache::new(300).await;
        cache.put("k1", "s1", &identity(None, None)).await;
        assert!(
            stored_entry(&cache, "k1", "s1").is_some(),
            "条目应类型化直存"
        );
        assert_eq!(cache.get("k1", "s1").await, cache.get("k1", "s1").await);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_wrong_secret_never_hits() {
        let cache = AuthCache::new(300).await;
        cache.put("k1", "s1", &identity(None, None)).await;

        assert!(
            cache.get("k1", "wrong-secret").await.is_none(),
            "仅知道 key_id 不得命中他人凭证的缓存"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_expired_key_entry_is_not_served() {
        let cache = AuthCache::new(300).await;
        // key 已在过去过期（理论上不会写入，这里直接构造条目验证读取侧防线）
        let key = AuthCache::entry_key("k1", "s1");
        cache.entries.lock().insert(
            key.clone(),
            CacheEntry {
                identity: Arc::new(identity(None, Some(1))),
                expires_at: Instant::now() + Duration::from_secs(300),
            },
        );

        assert!(
            cache.get("k1", "s1").await.is_none(),
            "过期 key 的缓存条目必须视为未命中"
        );
        assert!(
            cache.entries.lock().get(&key).is_none(),
            "过期条目应被顺手清理"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_invalidate_does_not_treat_glob_chars_in_key_id_as_pattern() {
        // 回归钉桩：key_id 源自客户端凭证串，含 `*`/`?` 时不得被当作 glob 通配，
        // 否则一次 invalidate 就能清掉他人全部缓存条目（放大为拒绝服务）。
        // garrison glob 扫描时代该测试为基线失败项（glob 丢含 `*` 的键）；
        // 类型化表改精确前缀删除后通配语义不复存在。
        let cache = AuthCache::new(300).await;
        cache.put("*", "s-star", &identity(None, None)).await;
        cache.put("k2", "s2", &identity(None, None)).await;
        cache.put("k3", "s3", &identity(None, None)).await;

        cache.invalidate("*").await;

        assert!(
            cache.get("*", "s-star").await.is_none(),
            "自己的条目应被删除"
        );
        assert!(
            cache.get("k2", "s2").await.is_some() && cache.get("k3", "s3").await.is_some(),
            "glob 元字符不得越界命中其他 key_id"
        );

        cache.invalidate("k?").await;
        assert!(
            cache.get("k2", "s2").await.is_some(),
            "`?` 同样必须按字面量处理"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_invalidate_removes_all_entries_of_key_id() {
        let cache = AuthCache::new(300).await;
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

    #[tokio::test(flavor = "multi_thread")]
    async fn test_clear_removes_everything() {
        let cache = AuthCache::new(300).await;
        cache.put("k1", "s1", &identity(None, None)).await;
        cache.put("k2", "s2", &identity(None, None)).await;

        cache.clear().await;

        assert!(cache.get("k1", "s1").await.is_none());
        assert!(cache.get("k2", "s2").await.is_none());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_ttl_zero_disables_caching() {
        let cache = AuthCache::new(0).await;
        cache.put("k1", "s1", &identity(None, None)).await;
        assert!(
            cache.get("k1", "s1").await.is_none(),
            "cache_ttl_seconds=0 时不得写入缓存"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_stale_entry_is_dropped_not_served() {
        // 旧 garrison String KV 时代存在「脏 JSON 条目不得下发」防线
        // （test_malformed_entry_is_dropped_not_served）；类型化直存后该失败
        // 类消失，读取侧仅剩的腐坏形态是过期时刻被外部改写 —— 直接构造
        // 过期条目，断言不下发且被清理。
        let cache = AuthCache::new(300).await;
        let key = AuthCache::entry_key("k1", "s1");
        cache.entries.lock().insert(
            key.clone(),
            CacheEntry {
                identity: Arc::new(identity(None, None)),
                expires_at: Instant::now() - Duration::from_secs(1),
            },
        );

        assert!(cache.get("k1", "s1").await.is_none(), "过期条目不得下发");
        assert!(cache.entries.lock().get(&key).is_none(), "过期条目应被清理");
    }

    #[test]
    fn test_cached_identity_serialization_contains_no_secret_material() {
        // 缓存值仅含授权决策三要素。字段集合用穷举断言钉死，
        // 未来给 CachedIdentity 加字段时此测试强制评审是否引入敏感数据。
        let value = serde_json::to_value(identity(Some(Uuid::new_v4()), Some(42))).unwrap();
        let obj = value
            .as_object()
            .expect("cached identity serializes to object");
        let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(keys, vec!["key_expires_at", "role", "workspace_id"]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_ttl_seconds_accessor() {
        assert_eq!(AuthCache::new(300).await.ttl_seconds(), 300);
        assert_eq!(AuthCache::new(0).await.ttl_seconds(), 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_put_with_already_expired_identity_writes_nothing() {
        let cache = AuthCache::new(300).await;
        cache.put("k1", "s1", &identity(None, Some(1))).await;
        assert!(
            cache.entries.lock().is_empty(),
            "已过期 key 的决策不得写入缓存"
        );
    }

    #[test]
    fn test_ttl_jitter_stays_within_ten_percent() {
        // ±10% 抖动的取值域钉桩：300s 的抖动结果必须全部落在 [270, 330]，
        // 且多次抽样产生不止一个值（抖动确实生效，而非常数退化）。
        let samples: std::collections::HashSet<u64> =
            (0..200).map(|_| AuthCache::jittered_ttl(300)).collect();
        assert!(
            samples.iter().all(|t| (270..=330).contains(t)),
            "抖动结果必须落在 ±10% 区间内，实际 {samples:?}"
        );
        assert!(
            samples.len() > 1,
            "抖动必须产生差异化的 TTL，实际 {samples:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_jitter_spreads_expiry_across_same_batch_entries() {
        // 同批写入的条目过期时刻不得全等（防同批 key 同时过期的雪崩）。
        // 24 个条目取自 [270, 330]s 连续区间，全等概率可忽略。
        let cache = AuthCache::new(300).await;
        for i in 0..24 {
            cache
                .put(&format!("k{i}"), "s", &identity(None, None))
                .await;
        }
        let entries = cache.entries.lock();
        assert_eq!(entries.len(), 24);
        let expiries: Vec<Instant> = entries.values().map(|e| e.expires_at).collect();
        let now = Instant::now();
        for expiry in &expiries {
            let delta = expiry.duration_since(now).as_secs_f64();
            assert!(
                (260.0..=340.0).contains(&delta),
                "过期时刻必须落在抖动区间附近，实际 {delta}s"
            );
        }
        let distinct = expiries.iter().collect::<std::collections::HashSet<_>>();
        assert!(
            distinct.len() > 1,
            "同批条目的过期时刻不得全部相等（抖动未生效）"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_jitter_never_outlives_key_expiry() {
        // 抖动只作用于缓存 TTL，不得延长 key 的绝对有效期：
        // key 剩余 5s 时，条目过期时刻不得超过 5s（尽管抖动上限是 330s）。
        let cache = AuthCache::new(300).await;
        let now_unix = AuthCache::now_unix();
        cache
            .put("k1", "s1", &identity(None, Some(now_unix + 5)))
            .await;
        let entry = stored_entry(&cache, "k1", "s1").expect("条目应已写入");
        let remaining = entry
            .expires_at
            .checked_duration_since(Instant::now())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        assert!(
            remaining <= 5,
            "条目 TTL 必须被 key 剩余寿命钳制，实际 {remaining}s"
        );
    }

    /// 100 并发 miss 同 key：回源恰好 1 次，全部等待者拿到同一决策。
    #[tokio::test(flavor = "multi_thread")]
    async fn test_concurrent_miss_loads_source_exactly_once() {
        let cache = Arc::new(AuthCache::new(300).await);
        let loads = Arc::new(AtomicUsize::new(0));
        // 用 watch 闸门把首个回源者拖在 OnceCell 里，直到全部任务完成登记，
        // 保证 100 个 miss 真正并发（否则串行到达会各自建新单元）。闸门
        // 超时后无条件放行：即使 single-flight 失效也不会死锁，只是断言失败。
        let (gate_tx, gate_rx) = tokio::sync::watch::channel(false);

        let mut handles = Vec::with_capacity(100);
        for _ in 0..100 {
            let cache = Arc::clone(&cache);
            let loads = Arc::clone(&loads);
            let gate_rx = gate_rx.clone();
            handles.push(tokio::spawn(async move {
                cache
                    .get_or_load("k1", "s1", || {
                        let loads = Arc::clone(&loads);
                        let mut gate_rx = gate_rx.clone();
                        async move {
                            loads.fetch_add(1, Ordering::SeqCst);
                            while !*gate_rx.borrow_and_update() {
                                if gate_rx.changed().await.is_err() {
                                    break;
                                }
                            }
                            Some(identity(Some(Uuid::nil()), None))
                        }
                    })
                    .await
            }));
        }
        // 给全部任务进入加载器的窗口，然后放行。
        tokio::time::sleep(Duration::from_millis(200)).await;
        let _ = gate_tx.send(true);
        for handle in handles {
            let result = handle.await.unwrap();
            assert_eq!(
                result,
                Some(identity(Some(Uuid::nil()), None)),
                "每个并发等待者都必须拿到同一决策"
            );
        }
        assert_eq!(
            loads.load(Ordering::SeqCst),
            1,
            "并发 100 miss 同 key 必须恰好回源 1 次"
        );
        assert!(cache.inflight.lock().is_empty(), "在途表必须清理");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_get_or_load_cache_hit_skips_loader() {
        let cache = AuthCache::new(300).await;
        let loads = Arc::new(AtomicUsize::new(0));
        let loads_for_loader = Arc::clone(&loads);

        let first = cache
            .get_or_load("k1", "s1", || {
                let loads = Arc::clone(&loads_for_loader);
                async move {
                    loads.fetch_add(1, Ordering::SeqCst);
                    Some(identity(None, None))
                }
            })
            .await;
        assert_eq!(first, Some(identity(None, None)));

        // 第二次调用必须走缓存，回源函数不得再执行。
        let second = cache
            .get_or_load("k1", "s1", || {
                let loads = Arc::clone(&loads_for_loader);
                async move {
                    loads.fetch_add(1, Ordering::SeqCst);
                    Some(identity(None, None))
                }
            })
            .await;
        assert_eq!(second, Some(identity(None, None)));
        assert_eq!(loads.load(Ordering::SeqCst), 1, "命中缓存后不得再次回源");
        assert!(cache.inflight.lock().is_empty(), "回源完成后在途表必须清理");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_single_flight_negative_result_is_not_cached() {
        // 回源失败（None，如凭证错误）不得写入缓存：认证失败被钉住会把
        // 「稍后修复的凭证」永久挡在门外。并发的同 key miss 仍只回源一次。
        let cache = Arc::new(AuthCache::new(300).await);
        let loads = Arc::new(AtomicUsize::new(0));
        let (gate_tx, gate_rx) = tokio::sync::watch::channel(false);

        let mut handles = Vec::with_capacity(10);
        for _ in 0..10 {
            let cache = Arc::clone(&cache);
            let loads = Arc::clone(&loads);
            let gate_rx = gate_rx.clone();
            handles.push(tokio::spawn(async move {
                cache
                    .get_or_load("k1", "s1", || {
                        let loads = Arc::clone(&loads);
                        let mut gate_rx = gate_rx.clone();
                        async move {
                            loads.fetch_add(1, Ordering::SeqCst);
                            while !*gate_rx.borrow_and_update() {
                                if gate_rx.changed().await.is_err() {
                                    break;
                                }
                            }
                            None
                        }
                    })
                    .await
            }));
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
        let _ = gate_tx.send(true);
        for handle in handles {
            assert_eq!(handle.await.unwrap(), None);
        }
        assert_eq!(
            loads.load(Ordering::SeqCst),
            1,
            "并发 miss 的失败回源同样只执行一次"
        );
        assert!(cache.entries.lock().is_empty(), "失败结果不得写入缓存");
        assert!(cache.inflight.lock().is_empty(), "在途表必须清理");

        // 失败未被缓存：下一次调用重新回源。
        cache
            .get_or_load("k1", "s1", || {
                let loads = Arc::clone(&loads);
                async move {
                    loads.fetch_add(1, Ordering::SeqCst);
                    None
                }
            })
            .await;
        assert_eq!(loads.load(Ordering::SeqCst), 2, "None 不得被缓存");
    }
}
