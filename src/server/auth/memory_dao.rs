// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! 进程内 `GarrisonDao` 实现。
//!
//! 该实现已收敛到 garrison 库：本模块 re-export `garrison::dao::InMemoryDao`
//! 作为进程内 DAO 的正名，`MemoryGarrisonDao` 类型与本地重复实现已删除。
//! garrison `InMemoryDao` 与本文档此前记录的接口契约一致（`GarrisonDao`
//! trait 的 `get` / `set` / `set_permanent` / `keys` / `get_and_delete` /
//! `incr` / `decr` / `set_if_absent` / `compare_and_swap`），差异点集中在
//! 两处需要钉桩的行为：`keys(pattern)` 走 glob 匹配（含 `*` / `?` 的 key 在
//! `set` / `get` 直读时按字面量访问，只有扫描参数才被解释为模式），
//! `set(..., 0)` 表示永久驻留。
//!
//! 适用场景：单实例部署、集成测试、开发环境。多实例部署需使用 Redis 后端
//! （garrison `cache-redis` feature）。

/// 进程内 `GarrisonDao` 实现（ garrison 库 `dao::InMemoryDao` 的 re-export）。
///
/// `pub use` 保持 `nebulaid::server::auth::InMemoryDao` 门面路径稳定；
/// 需直接以 trait object 使用处改引 `garrison::dao::GarrisonDao`。
pub use garrison::dao::InMemoryDao;

/// garrison `InMemoryDao` 的有界装饰器：容量阀 + FIFO 淘汰。
///
/// 防爆破桶的键空间受攻击者控制（可信代理场景下 XFF 首段可任意伪造，
/// `client_ip_to_context_ip` 把任意文本映射为独立桶），而 garrison
/// `InMemoryDao` 无容量上限、过期键只在同键再次被访问时惰性清理
/// （`keys()` 只过滤不删除，无后台 sweeper）——轮换 IP 的分布式爆破会让
/// 永不回访的计数键 `bf:{ip}:count` 与封禁键常驻不删。旧实现（剪裁构建
/// 的进程内滑窗）有 `MAX_TRACKED_AUTH_FAILURE_IPS = 10_000` 容量阀；
/// 本装饰器把同一约束补回 garrison 路径（审查实测 30 万条无界 ≈ +52MB）。
///
/// 淘汰策略：超过 `max_keys` 时删除最早插入的存活键（FIFO）。淘汰一个
/// 计数键只是丢掉该 IP 的失败计数（安全方向：少封禁）；淘汰一个封禁键
/// 会让被封者重新获得失败预算（可接受：重建封禁仍需再失败满阈值）。
///
/// `order` 双端队列记录插入意图，与 `inner` 的存在性允许漂移（键可能在
/// inner 侧已过期）：淘汰一个已消失的键是一次无害空删。队列自身长度以
/// `max_keys` 为上界，不会成为第二个无界结构。只有「新建键」路径
/// （`set` / `set_permanent` / `rename` / CAS 成功 / `set_if_absent`
/// 返回 created / `incr` 归一到 1）登记插入，避免对既有键重复记账。
///
/// 委托面与 `InMemoryDao` 的实现面一致（16 个方法）；其余 trait 成员
/// 继承默认实现，行为与直连 `InMemoryDao` 相同（默认即 NotImplemented 或
/// 经本装饰器的 get 系列转发）。
pub struct BoundedDao {
    inner: InMemoryDao,
    max_keys: usize,
    order: std::sync::Mutex<std::collections::VecDeque<String>>,
}

impl BoundedDao {
    pub fn new(inner: InMemoryDao, max_keys: usize) -> Self {
        Self {
            inner,
            max_keys,
            order: std::sync::Mutex::new(std::collections::VecDeque::new()),
        }
    }

    /// 登记新键插入意图，并把超限部分按 FIFO 淘汰回 inner。
    ///
    /// 锁不跨 await：先在临界区内取出待淘汰键，释放锁后再逐个删除。
    async fn track(&self, new_keys: &[&str]) {
        use garrison::dao::GarrisonDao;

        let victims = {
            let mut order = self.order.lock().unwrap_or_else(|e| e.into_inner());
            for key in new_keys {
                order.push_back((*key).to_string());
            }
            let mut victims = Vec::new();
            while order.len() > self.max_keys {
                match order.pop_front() {
                    Some(victim) => victims.push(victim),
                    None => break,
                }
            }
            victims
        };
        for victim in victims {
            if let Err(e) = self.inner.delete(&victim).await {
                tracing::debug!(
                    event = "bounded_dao_evict_failed",
                    key = %victim,
                    error = %e,
                );
            }
        }
    }
}

#[async_trait::async_trait]
impl garrison::dao::GarrisonDao for BoundedDao {
    async fn get(&self, key: &str) -> garrison::error::GarrisonResult<Option<String>> {
        self.inner.get(key).await
    }

    async fn set(
        &self,
        key: &str,
        value: &str,
        ttl_seconds: u64,
    ) -> garrison::error::GarrisonResult<()> {
        self.inner.set(key, value, ttl_seconds).await?;
        self.track(&[key]).await;
        Ok(())
    }

    async fn update(&self, key: &str, value: &str) -> garrison::error::GarrisonResult<()> {
        self.inner.update(key, value).await
    }

    async fn expire(&self, key: &str, seconds: u64) -> garrison::error::GarrisonResult<()> {
        self.inner.expire(key, seconds).await
    }

    async fn delete(&self, key: &str) -> garrison::error::GarrisonResult<()> {
        self.inner.delete(key).await
    }

    async fn set_permanent(&self, key: &str, value: &str) -> garrison::error::GarrisonResult<()> {
        self.inner.set_permanent(key, value).await?;
        self.track(&[key]).await;
        Ok(())
    }

    async fn get_timeout(
        &self,
        key: &str,
    ) -> garrison::error::GarrisonResult<Option<std::time::Duration>> {
        self.inner.get_timeout(key).await
    }

    async fn keys(&self, pattern: &str) -> garrison::error::GarrisonResult<Vec<String>> {
        self.inner.keys(pattern).await
    }

    async fn rename(&self, old_key: &str, new_key: &str) -> garrison::error::GarrisonResult<()> {
        self.inner.rename(old_key, new_key).await?;
        self.track(&[new_key]).await;
        Ok(())
    }

    async fn get_and_delete(&self, key: &str) -> garrison::error::GarrisonResult<Option<String>> {
        self.inner.get_and_delete(key).await
    }

    async fn set_if_absent(
        &self,
        key: &str,
        value: &str,
        ttl_seconds: u64,
    ) -> garrison::error::GarrisonResult<bool> {
        let created = self.inner.set_if_absent(key, value, ttl_seconds).await?;
        if created {
            self.track(&[key]).await;
        }
        Ok(created)
    }

    /// `incr` 在键缺失时初始化为 1——返回值 1 即新键登记时机。
    async fn incr(&self, key: &str, ttl_seconds: u64) -> garrison::error::GarrisonResult<u64> {
        let count = self.inner.incr(key, ttl_seconds).await?;
        if count == 1 {
            self.track(&[key]).await;
        }
        Ok(count)
    }

    async fn decr(&self, key: &str) -> garrison::error::GarrisonResult<u64> {
        self.inner.decr(key).await
    }

    async fn compare_and_update_if_greater(
        &self,
        key: &str,
        new_value: u64,
        ttl_seconds: u64,
    ) -> garrison::error::GarrisonResult<bool> {
        self.inner
            .compare_and_update_if_greater(key, new_value, ttl_seconds)
            .await
    }

    async fn compare_and_swap(
        &self,
        key: &str,
        expected: Option<&str>,
        new_value: &str,
        ttl_seconds: u64,
    ) -> garrison::error::GarrisonResult<bool> {
        let applied = self
            .inner
            .compare_and_swap(key, expected, new_value, ttl_seconds)
            .await?;
        if applied {
            self.track(&[key]).await;
        }
        Ok(applied)
    }

    async fn eval_lua(
        &self,
        script: &str,
        keys: Vec<String>,
        args: Vec<String>,
    ) -> garrison::error::GarrisonResult<Vec<String>> {
        self.inner.eval_lua(script, keys, args).await
    }
}

#[cfg(test)]
mod tests {
    use super::InMemoryDao;
    use garrison::dao::GarrisonDao;

    fn dao() -> InMemoryDao {
        InMemoryDao::new()
    }

    /// per-entry TTL：过期后读返回 None，永久键（TTL = 0）驻留。
    #[tokio::test]
    async fn test_per_entry_ttl_expiry_and_permanent_keys() {
        let d = dao();
        d.set("ttl-key", "v", 1).await.unwrap();
        d.set("perm-key", "v", 0).await.unwrap();

        assert_eq!(d.get("ttl-key").await.unwrap(), Some("v".to_string()));
        assert_eq!(d.get("perm-key").await.unwrap(), Some("v".to_string()));

        // 等待 TTL 到期：过期键不再可见（惰性过期时机由库实现决定，
        // 断言用轮询而非固定 sleep，避免 CI 抖动）。
        let start = std::time::Instant::now();
        while start.elapsed() < std::time::Duration::from_secs(5) {
            if d.get("ttl-key").await.unwrap().is_none() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert_eq!(d.get("ttl-key").await.unwrap(), None);
        assert_eq!(d.get("perm-key").await.unwrap(), Some("v".to_string()));
    }

    /// 原子 get_and_delete / incr：值一次性读出并删除；自增跨调用累计。
    #[tokio::test]
    async fn test_atomic_get_and_delete_and_incr() {
        let d = dao();
        d.set("k1", "v1", 0).await.unwrap();
        assert_eq!(
            d.get_and_delete("k1").await.unwrap(),
            Some("v1".to_string())
        );
        // 原子 get_and_delete 的第二读必须为 None（已删除）。
        assert_eq!(d.get("k1").await.unwrap(), None);

        assert_eq!(d.incr("counter", 100).await.unwrap(), 1);
        assert_eq!(d.incr("counter", 100).await.unwrap(), 2);
        assert_eq!(d.incr("counter", 100).await.unwrap(), 3);
        // 计数器带 TTL：set(...,0) 之外 incr 的 ttl_seconds 参数生效路径
        // 由库内部保证；这里只钉住计数累计语义。
    }

    /// glob 语义差异点：`*` / `?` 在直读 key 时按字面量，只有扫描参数
    /// 才解释为模式。含 glob 字符的 key 不能通过 `*` 模式误命中，
    /// 反之 `keys` 的模式匹配照常工作。
    #[tokio::test]
    async fn test_glob_chars_literal_on_direct_access_pattern_on_scan() {
        let d = dao();
        // 含 glob 字符的 key 按字面量存储与直读。
        d.set("key:*", "star", 0).await.unwrap();
        d.set("key:?", "question", 0).await.unwrap();
        assert_eq!(d.get("key:*").await.unwrap(), Some("star".to_string()));
        assert_eq!(d.get("key:?").await.unwrap(), Some("question".to_string()));

        // 扫描参数按 glob 解释：`*` 命中含 glob 字符的两条（以及文字 key）。
        d.set("key:plain", "plain", 0).await.unwrap();
        let mut scanned = d.keys("key:*").await.unwrap();
        scanned.sort();
        assert_eq!(scanned, vec!["key:*", "key:?", "key:plain"]);
    }

    /// `set_permanent` 与 `set(..., 0)` 等价：均无 TTL 过期。
    #[tokio::test]
    async fn test_permanent_keys_survive_when_bounded_keys_expire() {
        let d = dao();
        d.set_permanent("permanent", "v").await.unwrap();
        d.set("bounded", "v", 1).await.unwrap();

        let start = std::time::Instant::now();
        while start.elapsed() < std::time::Duration::from_secs(5) {
            if d.get("bounded").await.unwrap().is_none() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert_eq!(d.get("bounded").await.unwrap(), None);
        assert_eq!(d.get("permanent").await.unwrap(), Some("v".to_string()));
    }

    /// BoundedDao 容量阀：超过 max_keys 的新建把最早插入的存活键 FIFO
    /// 淘汰，inner 存活键数以阀为上界（防爆破桶无界增长的修复锚点）。
    #[tokio::test]
    async fn test_bounded_dao_evicts_oldest_beyond_cap() {
        use super::BoundedDao;

        let d = BoundedDao::new(dao(), 3);
        for i in 0..5 {
            d.set(&format!("k{i}"), "v", 0).await.unwrap();
        }
        assert_eq!(d.get("k0").await.unwrap(), None);
        assert_eq!(d.get("k1").await.unwrap(), None);
        assert_eq!(d.get("k2").await.unwrap(), Some("v".to_string()));
        assert_eq!(d.get("k4").await.unwrap(), Some("v".to_string()));
        assert!(d.keys("*").await.unwrap().len() <= 3);
    }

    /// 只有「新建键」路径登记插入：对既有键重复 incr 不重复记账，
    /// 不提前触发淘汰。
    #[tokio::test]
    async fn test_bounded_dao_incr_existing_key_not_double_tracked() {
        use super::BoundedDao;

        let d = BoundedDao::new(dao(), 2);
        d.set("a", "1", 0).await.unwrap();
        assert_eq!(d.incr("b", 100).await.unwrap(), 1);
        assert_eq!(d.incr("b", 100).await.unwrap(), 2);
        // 阀=2 恰好满员：再插入 c 只淘汰最早的 a，b 的计数保留
        d.set("c", "v", 0).await.unwrap();
        assert_eq!(d.get("a").await.unwrap(), None);
        assert_eq!(d.get("b").await.unwrap(), Some("2".to_string()));
        assert_eq!(d.get("c").await.unwrap(), Some("v".to_string()));
    }
}
