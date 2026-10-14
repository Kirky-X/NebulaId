// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! 进程内 `GarrisonDao` 实现：re-export garrison 库 `dao::InMemoryDao`。
//!
//! 适用单实例部署 / 集成测试 / 开发环境；多实例需 Redis 后端（`cache-redis` feature）。

/// garrison `dao::InMemoryDao` 的 re-export，保持 `server::auth::InMemoryDao` 门面路径稳定。
pub use garrison::dao::InMemoryDao;

/// garrison `InMemoryDao` 的有界装饰器：容量阀 + FIFO 淘汰。
///
/// 防爆破桶键空间受攻击者控制，而 InMemoryDao 无容量上限、过期键只惰性清理；
/// 超限淘汰最早插入的存活键（丢计数＝少封禁，丢封禁＝需再失败满阈值重建）。
/// `order` 只登记首次写入，队列以 `max_keys` 为上界，不产生第二个无界结构。
pub struct BoundedDao {
    inner: InMemoryDao,
    max_keys: usize,
    order: std::sync::Mutex<OrderIndex>,
}

/// 插入序索引：FIFO 队列 + 已登记键集合。
/// 集合使 `track` 只对新键入队，避免热键重复登记后被 FIFO 自淘汰。
struct OrderIndex {
    queue: std::collections::VecDeque<String>,
    present: std::collections::HashSet<String>,
}

impl OrderIndex {
    fn new() -> Self {
        Self {
            queue: std::collections::VecDeque::new(),
            present: std::collections::HashSet::new(),
        }
    }

    /// 登记新键；已登记过的键直接跳过（返回是否真的入队）。
    fn register(&mut self, key: &str) -> bool {
        if self.present.contains(key) {
            return false;
        }
        self.present.insert(key.to_string());
        self.queue.push_back(key.to_string());
        true
    }

    fn queue_len(&self) -> usize {
        self.queue.len()
    }

    /// 仅测试断言「登记集合与队列长度一致」时使用。
    #[cfg(test)]
    fn present_len(&self) -> usize {
        self.present.len()
    }
}

impl BoundedDao {
    pub fn new(inner: InMemoryDao, max_keys: usize) -> Self {
        // 0 会让「写入即淘汰」，退化成不可用缓存；钳到至少保留一个键位
        let max_keys = max_keys.max(1);
        Self {
            inner,
            max_keys,
            order: std::sync::Mutex::new(OrderIndex::new()),
        }
    }

    /// 登记新键插入意图，把超限部分按 FIFO 淘汰回 inner。
    /// 锁不跨 await：临界区内取出待淘汰键，释放锁后再删除。
    async fn track(&self, new_keys: &[&str]) {
        use garrison::dao::GarrisonDao;

        let victims = {
            let mut order = self.order.lock().unwrap_or_else(|e| e.into_inner());
            for key in new_keys {
                order.register(key);
            }
            let mut victims = Vec::new();
            while order.queue_len() > self.max_keys {
                match order.queue.pop_front() {
                    Some(victim) => {
                        // 淘汰后移除登记态，该键再次写入时可重新登记
                        order.present.remove(&victim);
                        victims.push(victim);
                    }
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

/// BoundedDao 只登记新键：反复 `set` 同一热键不得重复入队，否则热键会被 FIFO 自淘汰。
#[tokio::test]
async fn test_bounded_dao_reset_hot_key_is_not_evicted() {
    use garrison::dao::GarrisonDao;

    let dao = BoundedDao::new(InMemoryDao::new(), 4);

    // 填满容量
    for i in 0..4 {
        dao.set(&format!("cold-{i}"), "v", 0).await.unwrap();
    }
    // 反复重写热键远超容量轮次：若重复入队，热键会先被淘汰
    for _ in 0..50 {
        dao.set("hot", "v", 0).await.unwrap();
    }
    assert_eq!(
        dao.get("hot").await.unwrap(),
        Some("v".to_string()),
        "反复重写的热键不得被容量阀自淘汰"
    );

    // 冷键按 FIFO 被淘汰（容量阀本职）
    let mut evicted = 0;
    for i in 0..4 {
        if dao.get(&format!("cold-{i}")).await.unwrap().is_none() {
            evicted += 1;
        }
    }
    assert!(evicted > 0, "容量阀应淘汰最旧的冷键");
}

/// 天然去重后队列长度 == 存活登记数，两者都不超过 max_keys。
#[tokio::test]
async fn test_bounded_dao_queue_stays_bounded_under_reset_storm() {
    use garrison::dao::GarrisonDao;

    let dao = BoundedDao::new(InMemoryDao::new(), 8);
    for i in 0..8 {
        dao.set(&format!("k-{i}"), "v", 0).await.unwrap();
    }
    for _ in 0..200 {
        dao.set("k-0", "v", 0).await.unwrap();
    }
    let order = dao.order.lock().unwrap_or_else(|e| e.into_inner());
    assert!(
        order.queue_len() <= 8,
        "队列长度 {} 超过 max_keys 8（重复登记通胀）",
        order.queue_len()
    );
    assert_eq!(
        order.present_len(),
        order.queue_len(),
        "登记集合与队列长度必须一致"
    );
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

        // 惰性过期时机由库实现决定，轮询而非固定 sleep，避免 CI 抖动。
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
        // get_and_delete 第二读必须为 None（已删除）。
        assert_eq!(d.get("k1").await.unwrap(), None);

        assert_eq!(d.incr("counter", 100).await.unwrap(), 1);
        assert_eq!(d.incr("counter", 100).await.unwrap(), 2);
        assert_eq!(d.incr("counter", 100).await.unwrap(), 3);
        // incr 的 ttl_seconds 生效路径由库内部保证，这里只钉住计数累计语义。
    }

    /// glob 语义：`*` / `?` 在直读 key 时按字面量，只有扫描参数才解释为模式。
    #[tokio::test]
    async fn test_glob_chars_literal_on_direct_access_pattern_on_scan() {
        let d = dao();
        // 含 glob 字符的 key 按字面量存储与直读。
        d.set("key:*", "star", 0).await.unwrap();
        d.set("key:?", "question", 0).await.unwrap();
        assert_eq!(d.get("key:*").await.unwrap(), Some("star".to_string()));
        assert_eq!(d.get("key:?").await.unwrap(), Some("question".to_string()));

        // 扫描参数按 glob 解释：`*` 命中 glob 字面 key 与文字 key。
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

    /// 容量阀：超过 max_keys 时 FIFO 淘汰最早插入的存活键（防爆破桶无界增长）。
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

    /// 只有新建键路径登记插入：对既有键重复 incr 不重复记账，不提前触发淘汰。
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
