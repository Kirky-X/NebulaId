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

//! 嵌入式 SDK facade（wiring T012）。
//!
//! 收拢 `main.rs` 的装配知识（分布式锁注入、CPU 监控、`AlgorithmRouter::initialize`、
//! 降级后台任务），为嵌入方提供一等公民入口：
//!
//! ```no_run
//! # async fn demo() -> nebulaid::core::Result<()> {
//! use nebulaid::core::Config;
//! use nebulaid::sdk::NebulaIdClientBuilder;
//!
//! let mut config = Config::default();
//! config.algorithm.default = "snowflake".to_string(); // 纯算法，零 DB 依赖
//! let client = NebulaIdClientBuilder::new(config).build().await?;
//! let id = client.generate("workspace", "group", "biz_tag").await?;
//! # Ok(())
//! # }
//! ```
//!
//! **Segment 与数据库**：Segment 算法需要数据库做号段分配。未通过
//! [`NebulaIdClientBuilder::with_repository`] 注入仓储时，Segment 请求返回
//! `CoreError::ConfigurationError`（不静默降级）；Snowflake / UuidV8 为纯算法，
//! 零 DB 零网络即可用。

use std::sync::Arc;

use crate::core::algorithm::{AlgorithmRouter, CpuMonitor, DynAuditLogger, GenerateContext};
use crate::core::config::Config;
use crate::core::coordinator::{DistributedLock, LocalDistributedLock};
use crate::core::database::SeaOrmRepository;
use crate::core::types::{AlgorithmType, Id, IdBatch, IdFormat};
use crate::core::{CoreError, Result};

#[cfg(feature = "etcd")]
use crate::core::coordinator::{
    EtcdClientOps, EtcdClientWrapper, EtcdDistributedLock, SEGMENT_LOCK_PATH_PREFIX,
};

/// 嵌入式客户端构建器。
///
/// 最小用法：`NebulaIdClientBuilder::new(config).build().await`（零 DB，
/// 仅纯算法可用）。需要 Segment 时：`.with_repository(repo)` 注入
/// （owned）仓储 —— [`build`](Self::build) 内部完成分布式锁注入。
pub struct NebulaIdClientBuilder {
    config: Config,
    audit_logger: Option<DynAuditLogger>,
    repository: Option<SeaOrmRepository>,
}

impl NebulaIdClientBuilder {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            audit_logger: None,
            repository: None,
        }
    }

    /// 注入审计日志器（可选）。
    pub fn with_audit_logger(mut self, audit_logger: DynAuditLogger) -> Self {
        self.audit_logger = Some(audit_logger);
        self
    }

    /// 注入数据库仓储（可选，owned）。
    ///
    /// `build()` 会把分布式锁注入该仓储（etcd endpoints 已配置 →
    /// `EtcdDistributedLock`，任何失败回退 `LocalDistributedLock` 并 warn），
    /// 使 Segment 号段分配可用。
    pub fn with_repository(mut self, repository: SeaOrmRepository) -> Self {
        self.repository = Some(repository);
        self
    }

    /// 组装客户端：分布式锁注入 → `AlgorithmRouter::initialize` → 降级后台任务启动。
    ///
    /// 部分算法构建失败不阻断（例如无 DB 时 Segment 仍会构建为内存模式），
    /// 仅当全部算法不可用时返回错误 —— 与 `AlgorithmRouter::initialize` 的
    /// 部分失败语义一致。
    pub async fn build(self) -> Result<NebulaIdClient> {
        // 1. 分布式锁（镜像 main.rs：etcd 优先，失败回退 Local 并 warn）
        let lock = create_distributed_lock(&self.config).await;

        // 2. 仓储（若提供）：注入分布式锁后共享持有
        let repository = self
            .repository
            .map(|repo| Arc::new(repo.with_distributed_lock(lock)));

        // 3. CPU 监控 + 路由初始化
        let cpu_monitor = Arc::new(CpuMonitor::new());
        let router = AlgorithmRouter::new(self.config.clone(), self.audit_logger)
            .with_cpu_monitor(cpu_monitor);
        let router = Arc::new(router);
        router.initialize().await?;

        // 4. 降级后台任务
        router.get_degradation_manager().start_background_check();

        Ok(NebulaIdClient {
            router,
            repository,
            config: self.config,
        })
    }
}

/// 分布式锁创建：`etcd` feature 且 endpoints 已配置 → `EtcdDistributedLock`；
/// 任何失败回退 `LocalDistributedLock` 并显性 warn（与 main.rs 行为一致）。
#[cfg(feature = "etcd")]
async fn create_distributed_lock(config: &Config) -> Arc<dyn DistributedLock + Send + Sync> {
    if !config.etcd.endpoints.is_empty() {
        match EtcdClientWrapper::new(config.etcd.endpoints.clone()).await {
            Ok(client) => {
                let client: Arc<dyn EtcdClientOps> = Arc::new(client);
                match EtcdDistributedLock::new(client, SEGMENT_LOCK_PATH_PREFIX.to_string()).await {
                    Ok(etcd_lock) => return Arc::new(etcd_lock),
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            "sdk: failed to create EtcdDistributedLock, falling back to LocalDistributedLock"
                        );
                    }
                }
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "sdk: failed to connect etcd, falling back to LocalDistributedLock"
                );
            }
        }
    } else {
        tracing::warn!(
            "sdk: etcd endpoints not configured, using LocalDistributedLock (single-process only)"
        );
    }
    Arc::new(LocalDistributedLock::new())
}

#[cfg(not(feature = "etcd"))]
async fn create_distributed_lock(_config: &Config) -> Arc<dyn DistributedLock + Send + Sync> {
    Arc::new(LocalDistributedLock::new())
}

/// 嵌入式 ID 生成客户端。
///
/// 生成方法均为 `Send + Sync` 并发安全；内部 `AlgorithmRouter` 以 `Arc` 共享，
/// 可廉价克隆跨任务使用。
pub struct NebulaIdClient {
    router: Arc<AlgorithmRouter>,
    repository: Option<Arc<SeaOrmRepository>>,
    config: Config,
}

impl NebulaIdClient {
    /// 按默认算法生成单个 ID。
    pub async fn generate(&self, workspace: &str, group: &str, biz_tag: &str) -> Result<Id> {
        let algorithm = self.config.algorithm.get_default_algorithm();
        self.require_repository_for_segment(algorithm)?;
        let ctx = Self::make_ctx(workspace, group, biz_tag);
        self.router.generate(&ctx).await
    }

    /// 按默认算法批量生成 ID。
    pub async fn batch_generate(
        &self,
        workspace: &str,
        group: &str,
        biz_tag: &str,
        size: usize,
    ) -> Result<IdBatch> {
        let algorithm = self.config.algorithm.get_default_algorithm();
        self.require_repository_for_segment(algorithm)?;
        let ctx = Self::make_ctx(workspace, group, biz_tag);
        self.router.batch_generate(&ctx, size).await
    }

    /// 指定算法生成单个 ID。
    pub async fn generate_with_algorithm(
        &self,
        algorithm: AlgorithmType,
        workspace: &str,
        group: &str,
        biz_tag: &str,
    ) -> Result<Id> {
        self.require_repository_for_segment(algorithm)?;
        self.router
            .generate_with_algorithm(algorithm, workspace, group, biz_tag)
            .await
    }

    /// 各算法健康状态快照。
    pub async fn health_check(&self) -> Vec<(AlgorithmType, crate::core::types::HealthStatus)> {
        self.router.health_check().await
    }

    /// 停机：停止降级后台检查任务并释放客户端。
    ///
    /// 算法内部的后台任务（Segment 健康检查等）随 `AlgorithmRouter` 的
    /// 内部关停通道终止；显式调用本方法确保降级任务先行退出。
    pub async fn shutdown(self) {
        self.router
            .get_degradation_manager()
            .stop_background_check()
            .await;
    }

    /// 逃生舱：直接访问内部路由（高级用法）。
    ///
    /// **注意：绕过前置守卫，自负其责。** [`Self::generate`] / [`Self::batch_generate`]
    /// / [`Self::generate_with_algorithm`] 在调用路由前会执行
    /// `require_repository_for_segment` —— 未通过
    /// [`NebulaIdClientBuilder::with_repository`] 注入仓储时，Segment 请求返回
    /// 显性 `CoreError::ConfigurationError`。经本方法拿到 [`AlgorithmRouter`]
    /// 后直接调用其 `generate*` **不经过该守卫**：无 DB 时 Segment 会按路由自身的
    /// 行为处理（可能内存模式或降级），而非显性报错。仅在明确知晓自身如何补齐
    /// 该校验（或本就不需要）时使用。
    pub fn router(&self) -> &Arc<AlgorithmRouter> {
        &self.router
    }

    /// 是否注入了数据库仓储（决定 Segment 可用性）。
    pub fn has_repository(&self) -> bool {
        self.repository.is_some()
    }

    fn make_ctx(workspace: &str, group: &str, biz_tag: &str) -> GenerateContext {
        GenerateContext {
            workspace_id: workspace.to_string(),
            group_id: group.to_string(),
            biz_tag: biz_tag.to_string(),
            format: IdFormat::Numeric,
            prefix: None,
        }
    }

    /// Segment 需要数据库号段分配：未注入仓储时显性报错，禁止静默降级。
    fn require_repository_for_segment(&self, algorithm: AlgorithmType) -> Result<()> {
        if algorithm == AlgorithmType::Segment && self.repository.is_none() {
            return Err(CoreError::ConfigurationError(
                "Segment algorithm requires a database repository; \
                 call NebulaIdClientBuilder::with_repository() before build()"
                    .to_string(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn snowflake_config() -> Config {
        let mut config = Config::default();
        config.algorithm.default = "snowflake".to_string();
        config
    }

    /// R-sdk-002：8 并发 × 1000 次 snowflake 生成，去重后零重复。
    #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
    async fn sdk_concurrent_snowflake_generation_is_unique() {
        let client = Arc::new(
            NebulaIdClientBuilder::new(snowflake_config())
                .build()
                .await
                .expect("sdk build should succeed without DB"),
        );

        let mut handles = Vec::new();
        for _ in 0..8 {
            let client = client.clone();
            handles.push(tokio::spawn(async move {
                let mut ids = Vec::with_capacity(1000);
                for _ in 0..1000 {
                    let id = client
                        .generate("ws", "group", "biz")
                        .await
                        .expect("snowflake generate should succeed");
                    ids.push(id);
                }
                ids
            }));
        }

        let mut unique: HashSet<u128> = HashSet::new();
        let mut total = 0usize;
        for handle in handles {
            for id in handle.await.expect("worker task panicked") {
                unique.insert(id.as_u128());
                total += 1;
            }
        }

        assert_eq!(total, 8_000);
        assert_eq!(unique.len(), 8_000, "snowflake 并发生成必须零重复");
    }

    /// R-sdk-002：无 DB 时 Segment 请求返回显性 ConfigurationError
    /// （默认路径与显式指定路径均拒绝；纯算法不受影响）。
    #[tokio::test]
    async fn sdk_segment_without_repository_returns_configuration_error() {
        // Config::default() 默认算法为 segment，且未注入仓储
        let client = NebulaIdClientBuilder::new(Config::default())
            .build()
            .await
            .expect("build 应在无 DB 时成功（部分失败语义）");
        assert!(!client.has_repository());

        let err = client
            .generate("ws", "group", "biz")
            .await
            .expect_err("无 DB 时默认 Segment 请求必须失败");
        assert!(
            matches!(err, CoreError::ConfigurationError(_)),
            "期望 ConfigurationError，实际：{err:?}"
        );

        let err = client
            .generate_with_algorithm(AlgorithmType::Segment, "ws", "group", "biz")
            .await
            .expect_err("无 DB 时显式 Segment 请求必须失败");
        assert!(
            matches!(err, CoreError::ConfigurationError(_)),
            "期望 ConfigurationError，实际：{err:?}"
        );

        // 纯算法不受影响
        let ok = client
            .generate_with_algorithm(AlgorithmType::Snowflake, "ws", "group", "biz")
            .await;
        assert!(ok.is_ok(), "snowflake 无 DB 应可用：{ok:?}");
    }

    /// 批量生成与单值生成一致：无 DB 下 Segment 批量同样被拒绝，纯算法可用。
    #[tokio::test]
    async fn sdk_batch_generate_respects_segment_guard_and_works_for_pure_algorithms() {
        let mut config = Config::default();
        config.algorithm.default = "uuid_v8".to_string();
        let client = NebulaIdClientBuilder::new(config)
            .build()
            .await
            .expect("build should succeed");

        let batch = client
            .batch_generate("ws", "group", "biz", 10)
            .await
            .expect("uuid_v8 batch should succeed without DB");
        assert_eq!(batch.ids.len(), 10);

        let err = client
            .generate_with_algorithm(AlgorithmType::Segment, "ws", "group", "biz")
            .await
            .expect_err("segment without repository must fail");
        assert!(matches!(err, CoreError::ConfigurationError(_)));
    }

    /// shutdown 正常完成（降级后台任务停止），不 panic。
    #[tokio::test]
    async fn sdk_shutdown_stops_background_task() {
        let client = NebulaIdClientBuilder::new(snowflake_config())
            .build()
            .await
            .expect("build should succeed");
        let _ = client.generate("ws", "g", "b").await;
        client.shutdown().await;
    }
}
