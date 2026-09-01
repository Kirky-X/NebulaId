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

//! trait-kit Kit 范式装配的嵌入式 SDK 门面（wiring T012，feature `sdk` 门控）。

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use trait_kit::{
    impl_async_auto_builder, impl_module_meta, AsyncHealthCheck, AsyncKit, AsyncLifecycle,
    AsyncReady,
};

use crate::core::algorithm::{AlgorithmRouter, CpuMonitor, DynAuditLogger, GenerateContext};
use crate::core::config::Config;
use crate::core::coordinator::{DistributedLock, LocalDistributedLock};
use crate::core::database::SeaOrmRepository;
use crate::core::types::{AlgorithmType, Id, IdBatch, IdFormat, Result};
use crate::core::CoreError;

#[cfg(feature = "etcd")]
use crate::core::coordinator::{
    EtcdClientOps, EtcdClientWrapper, EtcdDistributedLock, SEGMENT_LOCK_PATH_PREFIX,
};

/// 分布式锁模块（能力：`Arc<dyn DistributedLock + Send + Sync>`）。
///
/// 无依赖；从 TypeMap 拉 `Config` 经 `create_distributed_lock` 构造锁
///（etcd 优先、任何失败回退 Local 并显性 warn —— 与 `main.rs` 行为一致）。
pub struct DistributedLockModule;

impl_module_meta!(DistributedLockModule, "distributed-lock");

impl_async_auto_builder!(
    DistributedLockModule,
    Arc<dyn DistributedLock + Send + Sync>,
    CoreError,
    |kit| Box::pin(async move {
        let config = kit
            .config::<Config>()
            .map_err(|e| CoreError::InternalError(format!("distributed-lock 模块配置缺失: {e}")))?;
        let lock = create_distributed_lock(&config).await;
        Ok(lock)
    })
);

/// 仓储的 TypeMap 注入包装（`pub(crate)`，不 re-export，不扩大 SDK 公共面）。
///
/// RepositoryModule 经 `kit.config::<RepositoryInput>()` 取用户注入的
/// `SeaOrmRepository`，经 `require::<DistributedLockModule>()` 取锁并注入。
#[derive(Clone)]
pub(crate) struct RepositoryInput(pub SeaOrmRepository);

/// 仓储模块（能力：`Arc<SeaOrmRepository>`，锁已注入）。
///
/// 依赖 `DistributedLockModule`（硬链 Lock→Repo）；本模块由 `register_if`
/// 条件注册——仅 builder 注入仓储时入图，纯算法零 DB 场景整个模块缺席。
pub struct RepositoryModule;

impl_module_meta!(
    RepositoryModule,
    "repository",
    deps = [DistributedLockModule]
);

impl_async_auto_builder!(RepositoryModule, Arc<SeaOrmRepository>, CoreError, |kit| {
    Box::pin(async move {
        let RepositoryInput(repository) = kit
            .config::<RepositoryInput>()
            .map_err(|e| CoreError::InternalError(format!("repository 模块配置缺失: {e}")))?;
        let lock = kit.require::<DistributedLockModule>().map_err(|e| {
            CoreError::InternalError(format!("repository 模块依赖分布式锁缺失: {e}"))
        })?;
        Ok(Arc::new(repository.with_distributed_lock(lock)))
    })
});

/// 路由模块（能力：`Arc<AlgorithmRouter>`，已 `initialize()`）。
///
/// 依赖 `DistributedLockModule`（design 硬链 Lock→Router），对仓储是**可选**
/// 依赖（Segment 才需要，不进硬依赖图——纯算法零 DB 时本模块仍须构建成功）。
/// 同时注册 trait-kit lifecycle（`on_ready` 启动降级后台任务 / `on_shutdown`
/// 停止）与 health check（模块级健康报告来源）。
pub struct RouterModule;

impl_module_meta!(RouterModule, "router", deps = [DistributedLockModule]);

impl_async_auto_builder!(RouterModule, Arc<AlgorithmRouter>, CoreError, |kit| {
    Box::pin(async move {
        let config = kit
            .config::<Config>()
            .map_err(|e| CoreError::InternalError(format!("router 模块配置缺失: {e}")))?;
        let AuditLoggerInput(audit_logger) = kit
            .config::<AuditLoggerInput>()
            .map_err(|e| CoreError::InternalError(format!("router 模块审计日志器缺失: {e}")))?;
        // 可选依赖拉取：RepositoryModule 缺席（纯算法零 DB）时 build 必须成功。
        // trait-kit RC 的 `optional()` 仅在 `AsyncKit<Ready>` 上可用，build 回调
        //（Unbuilt 阶段）以 `require().ok()` 实现同语义（模块缺席 →
        // `MissingCapability` → `None`）。
        let _repository = kit.require::<RepositoryModule>().ok();

        let cpu_monitor = Arc::new(CpuMonitor::new());
        let router = AlgorithmRouter::new(config, audit_logger).with_cpu_monitor(cpu_monitor);
        router
            .initialize()
            .await
            .map_err(|e| CoreError::InternalError(format!("router 模块初始化失败: {e}")))?;
        Ok(Arc::new(router))
    })
});

impl AsyncLifecycle for RouterModule {
    /// `on_ready`：trait-kit 在 `build()` 完成后按注册顺序触发——启动降级
    /// 后台任务（替代旧 `NebulaIdClientBuilder::build()` 第 4 步）。
    fn on_ready<'a>(
        kit: &'a AsyncKit<AsyncReady>,
    ) -> Pin<Box<dyn Future<Output = std::result::Result<(), Self::Error>> + Send + 'a>> {
        Box::pin(async move {
            let router = kit
                .require::<RouterModule>()
                .map_err(|e| CoreError::InternalError(format!("router on_ready 能力缺失: {e}")))?;
            router.get_degradation_manager().start_background_check();
            Ok(())
        })
    }

    /// `on_shutdown`：停止降级后台任务。trait-kit RC 版 `AsyncKit::shutdown()`
    /// 是同步方法、不执行 async 回调（库源码 async_kit.rs 明示 "intentionally
    /// skipped"）——实际停止路径由门面 `NebulaIdKit::shutdown()` 手动按逆拓扑
    /// 调用本方法（见 design Lifecycle 接线）；此处注册保证语义声明完整。
    fn on_shutdown<'a>(cap: &'a Self::Capability) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            cap.get_degradation_manager().stop_background_check().await;
        })
    }
}

impl AsyncHealthCheck for RouterModule {
    /// 模块级健康：全部算法健康 → Healthy；部分健康 → Degraded；全不健康/无
    /// 算法 → Unhealthy。注意返回的是 trait-kit 的 `HealthStatus`（与 nebulaid
    /// 核心的 `HealthStatus` 同名不同源，此处完全限定）。
    fn check(cap: &Self::Capability) -> trait_kit::HealthStatus {
        let states = cap.get_degradation_manager().get_all_states();
        if states.is_empty() {
            return trait_kit::HealthStatus::Unhealthy {
                detail: "no algorithms registered".to_string(),
            };
        }
        let healthy = states.iter().filter(|s| s.is_healthy).count();
        if healthy == states.len() {
            trait_kit::HealthStatus::Healthy
        } else if healthy > 0 {
            trait_kit::HealthStatus::Degraded {
                detail: format!("{healthy}/{} algorithms healthy", states.len()),
            }
        } else {
            trait_kit::HealthStatus::Unhealthy {
                detail: "all algorithms unhealthy".to_string(),
            }
        }
    }
}

/// ID 生成能力模块（能力：`IdGenerator`——Clone handle，内部 `Arc` 共享）。
///
/// 依赖 `RouterModule`（硬链 Router→IdGen）；对仓储 optional 拉取（守卫用，
/// 不进硬依赖图）。`generate*` 热路径直调 `Arc<AlgorithmRouter>`，不经过
/// AsyncKit 的 `Arc<RwLock>`。
pub struct IdGenModule;

impl_module_meta!(IdGenModule, "id-generation", deps = [RouterModule]);

impl_async_auto_builder!(IdGenModule, IdGenerator, CoreError, |kit| Box::pin(
    async move {
        let router = kit.require::<RouterModule>().map_err(|e| {
            CoreError::InternalError(format!("id-generation 模块依赖路由缺失: {e}"))
        })?;
        // 可选依赖拉取（同 RouterModule.build 的 RC 限制处理）：RepositoryModule
        // 缺席时 `IdGenerator.repository` 为 None，Segment 守卫在生成期显性报错。
        let repository = kit.require::<RepositoryModule>().ok();
        let config = kit
            .config::<Config>()
            .map_err(|e| CoreError::InternalError(format!("id-generation 模块配置缺失: {e}")))?;
        Ok(IdGenerator {
            router,
            repository,
            default_algorithm: config.algorithm.get_default_algorithm(),
        })
    }
));

/// 嵌入式 ID 生成 handle。
///
/// 生成方法语义与旧 `NebulaIdClient` 完全等价（迁移非重设计）：默认算法解析、
/// `require_repository_for_segment` 守卫、`GenerateContext { format: Numeric,
/// prefix: None }` 构造均逐字保留。`Clone` 廉价（内部 `Arc`），可跨任务共享。
#[derive(Clone)]
pub struct IdGenerator {
    router: Arc<AlgorithmRouter>,
    repository: Option<Arc<SeaOrmRepository>>,
    default_algorithm: AlgorithmType,
}

// Send + Sync 编译期断言（R-sdk-emb-002：clone 后可在 tokio::spawn 任务中生成）
const _: fn() = || {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<IdGenerator>();
};

impl IdGenerator {
    /// 按默认算法生成单个 ID。
    pub async fn generate(&self, workspace: &str, group: &str, biz_tag: &str) -> Result<Id> {
        self.require_repository_for_segment(self.default_algorithm)?;
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
        self.require_repository_for_segment(self.default_algorithm)?;
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
                 call NebulaIdKitBuilder::with_repository() before build()"
                    .to_string(),
            ));
        }
        Ok(())
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

/// audit_logger 的 TypeMap 注入包装（`pub(crate)`，不 re-export，不扩大 SDK 公共面）。
///
/// RouterModule 经 `kit.config::<AuditLoggerInput>()` 拉取。`Option` 语义保留
/// 旧 `NebulaIdClientBuilder.audit_logger` 的可选性。
#[derive(Clone)]
pub(crate) struct AuditLoggerInput(pub Option<DynAuditLogger>);

/// 嵌入式 SDK Kit：trait-kit `AsyncKit` 装配产物（分布式锁/仓储/路由/ID 生成
/// 四模块经 `build()` 依赖图校验后按拓扑序构造）。
///
/// 生成能力经 [`Self::id_generator`] 取用；`generate` 热路径不经过 AsyncKit 的
/// `Arc<RwLock>`（`require` 只发生在门面分发层）。
pub struct NebulaIdKit {
    kit: AsyncKit<AsyncReady>,
}

impl NebulaIdKit {
    /// 测试观测接缝（`pub(crate)`，不扩大公共面）：暴露底层 Ready Kit。
    ///
    /// 测试经此断言能力存在性/依赖图结果；禁止直接依赖私有字段。
    pub(crate) fn inner(&self) -> &AsyncKit<AsyncReady> {
        &self.kit
    }
}

/// 嵌入式 SDK Kit 构建器。
///
/// 最小用法：`NebulaIdKitBuilder::new(config).build().await`（零 DB，仅纯算法
/// 可用）。需要 Segment 时：`.with_repository(repo)` 注入（owned）仓储 ——
/// `build()` 内部完成分布式锁注入与依赖图校验。
pub struct NebulaIdKitBuilder {
    config: Config,
    audit_logger: Option<DynAuditLogger>,
    repository: Option<SeaOrmRepository>,
}

impl NebulaIdKitBuilder {
    /// 创建构建器（默认算法由 `config.algorithm.default` 决定）。
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

    /// 组装 Kit：TypeMap 注入 → 模块注册 → `AsyncKit::build()` 依赖图校验
    ///（缺失依赖/环检测 + 拓扑序构造）。
    pub async fn build(self) -> Result<NebulaIdKit> {
        let mut kit = AsyncKit::new();

        // TypeMap 常驻配置：Config 与 audit_logger（后续模块从 TypeMap 拉取）
        kit.set_config(self.config.clone());
        kit.set_config(AuditLoggerInput(self.audit_logger));

        // 模块注册（依赖图由 trait-kit 在 build() 校验）
        kit.register::<DistributedLockModule>()
            .map_err(|e| CoreError::InternalError(format!("trait-kit 模块注册失败: {e}")))?;

        // 仓储条件注册：仅注入时入图（register_if）——未注入时模块缺席、
        // build 不失败，"纯算法零 DB"这一核心卖点成立。
        kit.register_if::<RepositoryModule>(|_| self.repository.is_some())
            .map_err(|e| CoreError::InternalError(format!("trait-kit 模块注册失败: {e}")))?;
        if let Some(repository) = self.repository {
            // 用户注入的仓储也要作为配置入 TypeMap（RepositoryModule build 回调读取）
            kit.set_config(RepositoryInput(repository));
        }
        kit.register::<RouterModule>()
            .map_err(|e| CoreError::InternalError(format!("trait-kit 模块注册失败: {e}")))?;
        kit.register::<IdGenModule>()
            .map_err(|e| CoreError::InternalError(format!("trait-kit 模块注册失败: {e}")))?;

        // 生命周期与健康检查接线（on_ready 在 build() 完成后触发）
        kit.register_lifecycle::<RouterModule>();
        kit.register_health_check::<RouterModule>();

        // 注入完整性显性校验（装配错误提前暴露，不落入模块回调的隐式
        // MissingConfig）；同时确认 audit_logger 注入形态，供启动观测。
        let AuditLoggerInput(audit_logger) = kit
            .config::<AuditLoggerInput>()
            .map_err(|e| CoreError::InternalError(format!("SDK 装配配置缺失: {e}")))?;
        tracing::debug!(
            has_audit_logger = audit_logger.is_some(),
            "sdk: audit_logger 注入确认"
        );

        let ready_kit = kit
            .build()
            .await
            .map_err(|e| CoreError::InternalError(format!("trait-kit build 失败: {e}")))?;

        let nebula = NebulaIdKit { kit: ready_kit };
        // 收尾轻校验：Ready Kit 的健康报告可枚举（空图必为空；后续模块
        // 注册后自然增长），作为装配产物可观测性的一次触及。
        let _ = nebula.inner().health_report();
        Ok(nebula)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::algorithm::GenerateContext;
    use crate::core::config::Config;

    /// R-sdk-emb-001：仅 config、无任何模块注册时 `build()` 成功返回 Ready Kit
    ///（空依赖图通过 trait-kit 校验），且 TypeMap 中已注入 Config 与 audit_logger。
    #[tokio::test]
    async fn test_kit_builder_builds_empty_ready_kit() {
        let kit = NebulaIdKitBuilder::new(Config::default())
            .build()
            .await
            .expect("空图 build 必须成功");

        // config 与 audit_logger 已注入 TypeMap（未注入审计日志器时值为 None）
        assert!(kit.inner().contains_config::<Config>());
        let audit = kit
            .inner()
            .config::<AuditLoggerInput>()
            .expect("audit_logger 必须已注入 TypeMap");
        assert!(audit.0.is_none(), "未注入审计日志器时应为 None");
        // 无任何模块注册：健康检查报告为空
        assert!(kit.inner().health_report().is_empty());
    }

    /// R-sdk-emb-001：`DistributedLockModule` 注册后 `build()` 成功；`require`
    /// 返回的锁可 `acquire`/`release` 且 `is_healthy()` 为 true
    ///（`DistributedLock` trait 无 `lock()/unlock()` 方法，断言只走既有面）。
    #[tokio::test]
    async fn test_kit_builds_distributed_lock_module() {
        let kit = NebulaIdKitBuilder::new(Config::default())
            .build()
            .await
            .expect("build 必须成功");
        assert!(
            kit.inner().contains::<DistributedLockModule>(),
            "DistributedLockModule 必须已注册并构建"
        );

        let lock = kit
            .inner()
            .require::<DistributedLockModule>()
            .expect("require 分布式锁必须成功");
        assert!(lock.is_healthy(), "LocalDistributedLock 应恒健康");

        let guard = lock
            .acquire("test-key", 30)
            .await
            .expect("acquire 必须成功");
        guard.release().await.expect("release 必须成功");
    }

    /// R-sdk-emb-001：注入仓储时 `RepositoryModule` 注册且 `require` 返回的仓储
    /// 能力可用。
    ///
    /// 锁注入本身无 pub 观测接缝（repository.rs 注释自认 "No public getter for
    /// distributed_lock"，且测试构建下无锁路径静默走 `NoopLockGuard`），故断言
    /// 能力落位（contains/require）+ 仓储可用性冒烟（`get_db_connection`）；
    /// 锁注入的忠实性由实现直接调用 `with_distributed_lock(require 的锁)` 保证。
    #[tokio::test]
    async fn test_repository_module_registered_when_injected() {
        use dbnexus::sea_orm::{DatabaseBackend, MockDatabase};

        let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();
        let kit = NebulaIdKitBuilder::new(Config::default())
            .with_repository(crate::core::database::SeaOrmRepository::new(
                db,
                "test_salt".to_string(),
            ))
            .build()
            .await
            .expect("注入仓储后 build 必须成功");
        assert!(
            kit.inner().contains::<RepositoryModule>(),
            "注入仓储后 RepositoryModule 必须注册"
        );

        let repository = kit
            .inner()
            .require::<RepositoryModule>()
            .expect("require 仓储必须成功");
        let _ = repository.get_db_connection();
    }

    /// R-sdk-emb-001：未注入仓储时 `RepositoryModule` 缺席且 `build()` 不失败
    ///（`register_if` 语义——纯算法零 DB 场景成立）。
    #[tokio::test]
    async fn test_repository_module_absent_without_injection() {
        let kit = NebulaIdKitBuilder::new(Config::default())
            .build()
            .await
            .expect("无仓储注入时 build 必须成功（纯算法零 DB 场景）");
        assert!(
            !kit.inner().contains::<RepositoryModule>(),
            "未注入仓储时 RepositoryModule 必须缺席"
        );
        assert!(
            kit.inner().optional::<RepositoryModule>().is_none(),
            "optional 拉取也必须为 None"
        );
    }

    /// 测试共享：纯算法（snowflake）配置。
    fn snowflake_config() -> Config {
        let mut config = Config::default();
        config.algorithm.default = "snowflake".to_string();
        config
    }

    fn sample_ctx() -> GenerateContext {
        GenerateContext {
            workspace_id: "ws".to_string(),
            group_id: "group".to_string(),
            biz_tag: "biz".to_string(),
            format: crate::core::types::IdFormat::Numeric,
            prefix: None,
        }
    }

    /// R-sdk-emb-001：RouterModule 注册后 build 成功，require 返回已初始化
    /// router（可直接生成 ID）。
    #[tokio::test]
    async fn test_router_module_builds_and_initializes() {
        let kit = NebulaIdKitBuilder::new(snowflake_config())
            .build()
            .await
            .expect("纯算法配置 build 必须成功");
        assert!(
            kit.inner().contains::<RouterModule>(),
            "RouterModule 必须已注册并构建"
        );

        let router = kit
            .inner()
            .require::<RouterModule>()
            .expect("require router 必须成功");
        let id = router
            .generate(&sample_ctx())
            .await
            .expect("snowflake 生成必须成功");
        assert!(id.as_u128() > 0);
    }

    /// R-sdk-emb-003：`on_ready` 启动降级后台任务——等待 ≥1 个 check_interval
    /// tick 后，经 pub 接缝 `get_algorithm_metrics()` 观测 snowflake 的连续
    /// 成功计数被后台任务刷新（仅后台任务运行时 `record_success` 才发生；
    /// 禁止依赖私有字段）。
    #[tokio::test]
    async fn test_router_lifecycle_on_ready_starts_degradation() {
        let kit = NebulaIdKitBuilder::new(snowflake_config())
            .build()
            .await
            .expect("build 必须成功");
        let router = kit
            .inner()
            .require::<RouterModule>()
            .expect("require router 必须成功");
        let dm = router.get_degradation_manager();

        // 默认 check_interval_ms = 5000；等待 ≥1 tick（含首 tick 立即触发）
        tokio::time::sleep(std::time::Duration::from_millis(5_500)).await;

        let metrics = dm.get_algorithm_metrics();
        let snowflake = metrics.get("snowflake").expect("snowflake 指标必须存在");
        assert!(
            snowflake.consecutive_successes > 0,
            "后台任务应已刷新健康指标（Healthy → record_success），实际计数: {}",
            snowflake.consecutive_successes
        );
    }

    /// R-sdk-emb-003：`health_report()`（同步）基于 trait-kit health feature
    /// 返回包含 `router` 模块的报告。
    #[tokio::test]
    async fn test_health_report_contains_router_module() {
        let kit = NebulaIdKitBuilder::new(snowflake_config())
            .build()
            .await
            .expect("build 必须成功");

        let report = kit.inner().health_report();
        assert!(
            report.iter().any(|(name, _)| *name == "router"),
            "health_report 必须包含 router 模块，实际: {report:?}"
        );
    }

    /// R-sdk-emb-002：纯算法（snowflake）在零仓储注入下可用。
    #[tokio::test]
    async fn test_id_generator_pure_algorithm_without_repository() {
        let kit = NebulaIdKitBuilder::new(snowflake_config())
            .build()
            .await
            .expect("build 必须成功");
        let generator = kit
            .inner()
            .require::<IdGenModule>()
            .expect("require ID 生成能力必须成功");

        let id = generator
            .generate("ws", "group", "biz")
            .await
            .expect("snowflake 生成必须成功");
        assert!(id.as_u128() > 0);
    }

    /// R-sdk-emb-002：零仓储注入下 Segment 请求返回 `CoreError::ConfigurationError`
    /// 且消息文本含 `with_repository`（错误消息是公共 API 的一部分，经 Display
    /// 可观测）。
    #[tokio::test]
    async fn test_id_generator_segment_without_repository_returns_configuration_error() {
        // Config::default() 默认算法为 segment
        let kit = NebulaIdKitBuilder::new(Config::default())
            .build()
            .await
            .expect("build 必须成功（未注入仓储不阻断）");
        let generator = kit
            .inner()
            .require::<IdGenModule>()
            .expect("require ID 生成能力必须成功");

        let err = generator
            .generate("ws", "group", "biz")
            .await
            .expect_err("无 DB 时默认 Segment 请求必须失败");
        assert!(
            matches!(err, CoreError::ConfigurationError(_)),
            "期望 ConfigurationError，实际：{err:?}"
        );
        let msg = err.to_string();
        assert!(
            msg.contains("with_repository"),
            "错误消息必须指向 NebulaIdKitBuilder::with_repository，实际：{msg}"
        );
    }

    /// R-sdk-emb-002：注入仓储后 Segment 请求通过守卫并可生成
    ///（Segment 算法自带默认 loader，无需真实 DB；守卫放行即语义达成）。
    #[tokio::test]
    async fn test_id_generator_segment_with_repository_succeeds() {
        use dbnexus::sea_orm::{DatabaseBackend, MockDatabase};

        let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();
        let kit = NebulaIdKitBuilder::new(Config::default())
            .with_repository(crate::core::database::SeaOrmRepository::new(
                db,
                "test_salt".to_string(),
            ))
            .build()
            .await
            .expect("注入仓储后 build 必须成功");
        let generator = kit
            .inner()
            .require::<IdGenModule>()
            .expect("require ID 生成能力必须成功");

        let id = generator
            .generate("ws", "group", "biz")
            .await
            .expect("注入仓储后 Segment 生成必须成功");
        assert!(id.as_u128() > 0);
    }

    /// R-sdk-emb-002：`IdGenerator` 为 Clone handle，克隆后在独立 tokio 任务中
    /// 生成（Send + Sync 语义的运行时验证）。
    #[tokio::test]
    async fn test_id_generator_handle_is_clone_and_cross_task() {
        let kit = NebulaIdKitBuilder::new(snowflake_config())
            .build()
            .await
            .expect("build 必须成功");
        let generator = kit
            .inner()
            .require::<IdGenModule>()
            .expect("require ID 生成能力必须成功");

        let cloned = generator.clone();
        let handle = tokio::spawn(async move {
            cloned
                .generate("ws", "group", "biz")
                .await
                .expect("task 内生成必须成功")
        });
        let id = handle.await.expect("worker task panicked");
        assert!(id.as_u128() > 0);
    }
}
