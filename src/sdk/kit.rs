// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! trait-kit Kit 范式装配的嵌入式 SDK 门面（feature `sdk` 门控）。

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use trait_kit::{
    impl_async_auto_builder, impl_module_meta, AsyncHealthCheck, AsyncKit, AsyncLifecycle,
    AsyncReady,
};

#[cfg(feature = "segment")]
use crate::core::algorithm::CpuMonitor;
use crate::core::algorithm::{AlgorithmRouter, DynAuditLogger, GenerateContext};
use crate::core::config::Config;
use crate::core::coordinator::{DistributedLock, LocalDistributedLock};
#[cfg(feature = "database")]
use crate::core::database::SeaOrmRepository;
use crate::core::types::{AlgorithmType, Id, IdBatch, IdFormat, Result};
use crate::core::CoreError;

#[cfg(feature = "etcd")]
use crate::core::coordinator::{
    EtcdClientOps, EtcdClientWrapper, EtcdDistributedLock, SEGMENT_LOCK_PATH_PREFIX,
};

/// 分布式锁模块（能力：`Arc<dyn DistributedLock + Send + Sync>`）。
///
/// 从 TypeMap 拉 `Config` 经 `create_distributed_lock` 构造（etcd 优先、fail-closed）。
pub struct DistributedLockModule;

impl_module_meta!(DistributedLockModule, "distributed-lock");

impl_async_auto_builder!(
    DistributedLockModule,
    Arc<dyn DistributedLock + Send + Sync>,
    CoreError,
    |kit| Box::pin(async move {
        let config = kit.config::<Config>().map_err(|e| {
            CoreError::InternalError(t!(
                "error.sdk.module_config_missing",
                module = "distributed-lock",
                reason = e
            ))
        })?;
        create_distributed_lock(&config).await
    })
);

/// 仓储的 TypeMap 注入包装（`pub(crate)`，不扩大 SDK 公共面）。
/// RepositoryModule 经此取用户注入的 `SeaOrmRepository`，并注入锁。
#[cfg(feature = "database")]
#[derive(Clone)]
pub(crate) struct RepositoryInput(pub SeaOrmRepository);

/// 仓储模块（能力：`Arc<SeaOrmRepository>`，锁已注入）。
/// 依赖 `DistributedLockModule`；仅 builder 注入仓储时入图（`register_if`）。
#[cfg(feature = "database")]
pub struct RepositoryModule;

#[cfg(feature = "database")]
impl_module_meta!(
    RepositoryModule,
    "repository",
    deps = [DistributedLockModule]
);

#[cfg(feature = "database")]
impl_async_auto_builder!(RepositoryModule, Arc<SeaOrmRepository>, CoreError, |kit| {
    Box::pin(async move {
        let RepositoryInput(repository) = kit.config::<RepositoryInput>().map_err(|e| {
            CoreError::InternalError(t!(
                "error.sdk.module_config_missing",
                module = "repository",
                reason = e
            ))
        })?;
        let lock = kit.require::<DistributedLockModule>().map_err(|e| {
            CoreError::InternalError(t!(
                "error.sdk.module_dependency_missing",
                module = "repository",
                dependency = "distributed-lock",
                reason = e
            ))
        })?;
        Ok(Arc::new(repository.with_distributed_lock(lock)))
    })
});

/// 路由模块（能力：`Arc<AlgorithmRouter>`，已 `initialize()`）。
///
/// 依赖锁模块（硬链）、对仓储可选；注册 lifecycle（on_ready 启动降级后台任务 /
/// on_shutdown 停止）与健康检查。
pub struct RouterModule;

impl_module_meta!(RouterModule, "router", deps = [DistributedLockModule]);

impl_async_auto_builder!(RouterModule, Arc<AlgorithmRouter>, CoreError, |kit| {
    Box::pin(async move {
        let config = kit.config::<Config>().map_err(|e| {
            CoreError::InternalError(t!(
                "error.sdk.module_config_missing",
                module = "router",
                reason = e
            ))
        })?;
        let AuditLoggerInput(audit_logger) = kit.config::<AuditLoggerInput>().map_err(|e| {
            CoreError::InternalError(t!(
                "error.sdk.module_audit_logger_missing",
                module = "router",
                reason = e
            ))
        })?;
        // 可选依赖拉取：RepositoryModule 缺席（纯算法零 DB）时 build 必须成功。
        // trait-kit RC 的 `optional()` 仅对 Ready Kit 可用，build 回调以 `require().ok()`
        // 实现同语义（模块缺席 → MissingCapability → None）；`database` 关闭时随门控消失。
        #[cfg(feature = "database")]
        let _repository = kit.require::<RepositoryModule>().ok();

        let router = AlgorithmRouter::new(config, audit_logger);
        #[cfg(feature = "segment")]
        let router = router.with_cpu_monitor(Arc::new(CpuMonitor::new()));
        router.initialize().await.map_err(|e| {
            CoreError::InternalError(t!(
                "error.sdk.module_init_failed",
                module = "router",
                reason = e
            ))
        })?;
        Ok(Arc::new(router))
    })
});

impl AsyncLifecycle for RouterModule {
    /// `on_ready`：build() 完成后按注册顺序触发，启动降级后台任务。
    fn on_ready<'a>(
        kit: &'a AsyncKit<AsyncReady>,
    ) -> Pin<Box<dyn Future<Output = std::result::Result<(), Self::Error>> + Send + 'a>> {
        Box::pin(async move {
            let router = kit.require::<RouterModule>().map_err(|e| {
                CoreError::InternalError(t!(
                    "error.sdk.on_ready_capability_missing",
                    module = "router",
                    reason = e
                ))
            })?;
            router.get_degradation_manager().start_background_check();
            Ok(())
        })
    }

    /// `on_shutdown`：停止降级后台任务（`shutdown_async()` 按逆拓扑序 drain 本钩子）。
    fn on_shutdown<'a>(cap: &'a Self::Capability) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            cap.get_degradation_manager().stop_background_check().await;
        })
    }
}

impl AsyncHealthCheck for RouterModule {
    /// 模块级健康：全健康 → Healthy；部分 → Degraded；全不健康/无算法 → Unhealthy。
    /// 返回 trait-kit 的 `HealthStatus`（与核心同名不同源，此处完全限定）。
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

/// ID 生成能力模块（能力：`IdGenerator`，Clone handle 内部 Arc 共享）。
/// 依赖 RouterModule（硬链）；对仓储 optional 拉取（守卫用）；热路径直调 `Arc<AlgorithmRouter>`。
pub struct IdGenModule;

impl_module_meta!(IdGenModule, "id-generation", deps = [RouterModule]);

impl_async_auto_builder!(IdGenModule, IdGenerator, CoreError, |kit| Box::pin(
    async move {
        let router = kit.require::<RouterModule>().map_err(|e| {
            CoreError::InternalError(t!(
                "error.sdk.module_dependency_missing",
                module = "id-generation",
                dependency = "router",
                reason = e
            ))
        })?;
        // 可选依赖拉取（同 RouterModule）：RepositoryModule 缺席时 repository 为 None，
        // Segment 守卫在生成期显性报错；`database` 关闭时随门控消失。
        #[cfg(feature = "database")]
        let repository = kit.require::<RepositoryModule>().ok();
        #[cfg(feature = "database")]
        let config = kit.config::<Config>().map_err(|e| {
            CoreError::InternalError(t!(
                "error.sdk.module_config_missing",
                module = "id-generation",
                reason = e
            ))
        })?;
        Ok(IdGenerator {
            router,
            #[cfg(feature = "database")]
            repository,
            #[cfg(feature = "database")]
            default_algorithm: config.algorithm.get_default_algorithm(),
        })
    }
));

/// 嵌入式 ID 生成 handle：生成语义与旧 SDK 客户端等价（默认算法解析 +
/// `require_repository_for_segment` 守卫 + `GenerateContext { Numeric, None }`）；Clone 廉价。
#[derive(Clone)]
pub struct IdGenerator {
    router: Arc<AlgorithmRouter>,
    #[cfg(feature = "database")]
    repository: Option<Arc<SeaOrmRepository>>,
    // 唯一读者是 require_repository_for_segment 守卫，随仓储面同门控。
    #[cfg(feature = "database")]
    default_algorithm: AlgorithmType,
}

// Send + Sync 编译期断言（clone 后可在 tokio::spawn 任务中生成）
const _: fn() = || {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<IdGenerator>();
};

impl IdGenerator {
    /// 按默认算法生成单个 ID。
    pub async fn generate(&self, workspace: &str, group: &str, biz_tag: &str) -> Result<Id> {
        #[cfg(feature = "database")]
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
        #[cfg(feature = "database")]
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
        #[cfg(feature = "database")]
        self.require_repository_for_segment(algorithm)?;
        self.router
            .generate_with_algorithm(algorithm, workspace, group, biz_tag)
            .await
    }

    /// RFC 9562 v5 确定性派生（**非发号**：同输入恒同输出）。
    ///
    /// 同步方法（纯 SHA-1 无 IO）；不经 router——派生无换算法语义，也无配额/审计面；
    /// 语义与直调 [`crate::core::algorithm::derive_uuid_v5`] 逐位一致。
    #[cfg(feature = "uuid-v5")]
    pub fn derive_uuid_v5(&self, namespace: uuid::Uuid, name: &str) -> Result<Id> {
        crate::core::algorithm::derive_uuid_v5(namespace, name)
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
    /// `database` 关闭时由 router 分支显性拒绝。
    #[cfg(feature = "database")]
    fn require_repository_for_segment(&self, algorithm: AlgorithmType) -> Result<()> {
        if algorithm == AlgorithmType::Segment && self.repository.is_none() {
            return Err(CoreError::ConfigurationError(t!(
                "error.sdk.repository_required_for_segment"
            )));
        }
        Ok(())
    }
}

/// 分布式锁创建（fail-closed）：etcd 且 endpoints 已配置 → `EtcdDistributedLock`（ping 探活）；
/// 任一步失败返回 `Err`，**不再静默回退**进程内锁（多实例部署静默回退必然重复 ID）。
/// 未配置 endpoints → `LocalDistributedLock`（单机合法）并 warn。
#[cfg(feature = "etcd")]
async fn create_distributed_lock(
    config: &Config,
) -> Result<Arc<dyn DistributedLock + Send + Sync>> {
    if config.etcd.endpoints.is_empty() {
        tracing::warn!(
            "sdk: etcd endpoints not configured, using LocalDistributedLock (single-process only)"
        );
        return Ok(Arc::new(LocalDistributedLock::new()));
    }

    let wrapper = EtcdClientWrapper::new(config.etcd.endpoints.clone()).await.map_err(|e| {
        CoreError::ConfigurationError(format!(
            "sdk: etcd endpoints are configured but the client is unavailable (endpoints: {:?}): {e}; refusing to fall back to LocalDistributedLock",
            config.etcd.endpoints
        ))
    })?;
    let client: Arc<dyn EtcdClientOps> = Arc::new(wrapper);

    // lazy connect 探活：ping 不通 = etcd 实际不可用，与连接失败同口径 fail-closed。
    let probe_timeout = std::time::Duration::from_millis(config.etcd.connect_timeout_ms.max(1));
    match tokio::time::timeout(probe_timeout, client.ping()).await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            return Err(CoreError::ConfigurationError(format!(
                "sdk: etcd ping failed (endpoints: {:?}): {e}; refusing to fall back to LocalDistributedLock",
                config.etcd.endpoints
            )));
        }
        Err(_) => {
            return Err(CoreError::ConfigurationError(format!(
                "sdk: etcd ping timed out after {}ms (endpoints: {:?}); refusing to fall back to LocalDistributedLock",
                config.etcd.connect_timeout_ms, config.etcd.endpoints
            )));
        }
    }

    let etcd_lock = EtcdDistributedLock::new(client, SEGMENT_LOCK_PATH_PREFIX.to_string())
        .await
        .map_err(|e| {
            CoreError::ConfigurationError(format!("sdk: failed to create EtcdDistributedLock: {e}"))
        })?;
    Ok(Arc::new(etcd_lock))
}

#[cfg(not(feature = "etcd"))]
async fn create_distributed_lock(
    _config: &Config,
) -> Result<Arc<dyn DistributedLock + Send + Sync>> {
    Ok(Arc::new(LocalDistributedLock::new()))
}

/// audit_logger 的 TypeMap 注入包装（`pub(crate)`，不扩大 SDK 公共面）。
/// `Option` 语义保留旧构建器 audit_logger 字段的可选性。
#[derive(Clone)]
pub(crate) struct AuditLoggerInput(pub Option<DynAuditLogger>);

/// 嵌入式 SDK Kit：trait-kit `AsyncKit` 装配产物（四模块按拓扑序构造）。
///
/// 生成能力经 [`Self::id_generator`] 取用；热路径不经过 AsyncKit 的 `Arc<RwLock>`。
pub struct NebulaIdKit {
    kit: AsyncKit<AsyncReady>,
}

impl NebulaIdKit {
    /// 测试观测接缝（`pub(crate)`）：暴露底层 Ready Kit，测试禁止直接依赖私有字段。
    pub(crate) fn inner(&self) -> &AsyncKit<AsyncReady> {
        &self.kit
    }

    /// 取用 ID 生成能力（`require::<IdGenModule>()` 薄封装）。
    /// `require` 的读锁成本只在分发瞬间；此后 generate 直调内部 Arc，热路径零锁。
    pub fn id_generator(&self) -> Result<IdGenerator> {
        self.kit.require::<IdGenModule>().map_err(|e| {
            CoreError::InternalError(t!(
                "error.sdk.capability_missing",
                module = "id-generation",
                reason = e
            ))
        })
    }

    /// 各算法健康状态快照（直调 router）。
    /// Ready Kit 上 RouterModule 必然存在，`expect` 是内部不变量断言（显性失败，不吞错）。
    pub async fn health_check(&self) -> Vec<(AlgorithmType, crate::core::types::HealthStatus)> {
        let router = self
            .kit
            .require::<RouterModule>()
            .expect(&t!("error.sdk.router_module_must_exist_on_ready_kit"));
        router.health_check().await
    }

    /// 模块级健康报告（返回 trait-kit 的 `HealthStatus`，与核心同名不同源）。
    pub fn health_report(&self) -> Vec<(&'static str, trait_kit::HealthStatus)> {
        self.kit.health_report()
    }

    /// 聚合健康摘要（worst-of：unhealthy > degraded > healthy）+ 逐模块明细。
    /// 返回值可直接作为嵌入式宿主 `/ready` 端点的 JSON 负载。
    pub fn health_summary(&self) -> serde_json::Value {
        let worst_rank = |s: &trait_kit::HealthStatus| match s {
            trait_kit::HealthStatus::Healthy => 0,
            trait_kit::HealthStatus::Degraded { .. } => 1,
            trait_kit::HealthStatus::Unhealthy { .. } => 2,
        };
        let status_name = |s: &trait_kit::HealthStatus| match s {
            trait_kit::HealthStatus::Healthy => "healthy",
            trait_kit::HealthStatus::Degraded { .. } => "degraded",
            trait_kit::HealthStatus::Unhealthy { .. } => "unhealthy",
        };
        let detail_of = |s: &trait_kit::HealthStatus| match s {
            trait_kit::HealthStatus::Degraded { detail } => Some(detail.clone()),
            trait_kit::HealthStatus::Unhealthy { detail } => Some(detail.clone()),
            trait_kit::HealthStatus::Healthy => None,
        };

        let report = self.health_report();
        let overall = report
            .iter()
            .map(|(_, status)| worst_rank(status))
            .max()
            .unwrap_or(0);
        let status = ["healthy", "degraded", "unhealthy"][overall];

        serde_json::json!({
            "status": status,
            "healthy": status == "healthy",
            "modules": report
                .into_iter()
                .map(|(name, hs)| {
                    serde_json::json!({
                        "module": name,
                        "status": status_name(&hs),
                        "detail": detail_of(&hs),
                    })
                })
                .collect::<Vec<_>>(),
        })
    }

    /// 停机：await `shutdown_async()`，trait-kit 按逆拓扑序执行各模块 `on_shutdown`
    /// （RouterModule 停止降级后台任务；二次调用 no-op）。
    pub async fn shutdown(self) {
        self.kit.shutdown_async().await;
    }

    /// 是否注入了数据库仓储（决定 Segment 可用性；仅随 `database` 特性编译）。
    #[cfg(feature = "database")]
    pub fn has_repository(&self) -> bool {
        self.kit.optional::<RepositoryModule>().is_some()
    }
}

/// 嵌入式 SDK Kit 构建器：最小用法 `NebulaIdKitBuilder::new(config).build().await`（零 DB）；
/// 需要 Segment 时 `.with_repository(repo)` 注入（随 `database` 编译）。
pub struct NebulaIdKitBuilder {
    config: Config,
    audit_logger: Option<DynAuditLogger>,
    #[cfg(feature = "database")]
    repository: Option<SeaOrmRepository>,
}

impl NebulaIdKitBuilder {
    /// 创建构建器（默认算法由 `config.algorithm.default` 决定）。
    pub fn new(config: Config) -> Self {
        Self {
            config,
            audit_logger: None,
            #[cfg(feature = "database")]
            repository: None,
        }
    }

    /// 注入审计日志器（可选）。
    pub fn with_audit_logger(mut self, audit_logger: DynAuditLogger) -> Self {
        self.audit_logger = Some(audit_logger);
        self
    }

    /// 注入数据库仓储（可选，owned；仅随 `database` 特性编译）。
    /// `build()` 会把分布式锁注入该仓储（etcd 失败时 fail-closed，不回退）。
    #[cfg(feature = "database")]
    pub fn with_repository(mut self, repository: SeaOrmRepository) -> Self {
        self.repository = Some(repository);
        self
    }

    /// 组装 Kit：TypeMap 注入 → 模块注册 → `AsyncKit::build()` 依赖图校验（缺依赖/环检测 + 拓扑序）。
    pub async fn build(self) -> Result<NebulaIdKit> {
        let mut kit = AsyncKit::new();

        // TypeMap 常驻配置：Config 与 audit_logger（后续模块从 TypeMap 拉取）
        kit.set_config(self.config.clone());
        kit.set_config(AuditLoggerInput(self.audit_logger));

        // 模块注册（依赖图由 trait-kit 在 build() 校验）
        kit.register::<DistributedLockModule>().map_err(|e| {
            CoreError::InternalError(t!("error.sdk.module_register_failed", reason = e))
        })?;

        // 仓储条件注册：仅注入时入图（register_if）——未注入时模块缺席、build 不失败，
        // "纯算法零 DB"成立；`database` 关闭时本段随门控消失。
        #[cfg(feature = "database")]
        kit.register_if::<RepositoryModule>(|_| self.repository.is_some())
            .map_err(|e| {
                CoreError::InternalError(t!("error.sdk.module_register_failed", reason = e))
            })?;
        #[cfg(feature = "database")]
        if let Some(repository) = self.repository {
            // 用户注入的仓储也要作为配置入 TypeMap（RepositoryModule build 回调读取）
            kit.set_config(RepositoryInput(repository));
        }
        kit.register::<RouterModule>().map_err(|e| {
            CoreError::InternalError(t!("error.sdk.module_register_failed", reason = e))
        })?;
        kit.register::<IdGenModule>().map_err(|e| {
            CoreError::InternalError(t!("error.sdk.module_register_failed", reason = e))
        })?;

        // 生命周期与健康检查接线（on_ready 在 build() 完成后触发）
        kit.register_lifecycle::<RouterModule>();
        kit.register_health_check::<RouterModule>();

        // 注入完整性显性校验（装配错误提前暴露，不落入隐式 MissingConfig）；
        // 同时确认 audit_logger 注入形态，供启动观测。
        let AuditLoggerInput(audit_logger) = kit.config::<AuditLoggerInput>().map_err(|e| {
            CoreError::InternalError(t!("error.sdk.assembly_config_missing", reason = e))
        })?;
        tracing::debug!(
            has_audit_logger = audit_logger.is_some(),
            "{}",
            t!("log.sdk.audit_logger_injection_confirmed")
        );

        let ready_kit = kit
            .build()
            .await
            .map_err(|e| CoreError::InternalError(t!("error.sdk.kit_build_failed", reason = e)))?;

        let nebula = NebulaIdKit { kit: ready_kit };
        // 收尾轻校验：Ready Kit 的健康报告可枚举（空图必为空），触及一次可观测性。
        let _ = nebula.inner().health_report();
        Ok(nebula)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::algorithm::GenerateContext;
    use crate::core::config::Config;

    /// 默认 Config 下 build() 成功返回 Ready Kit，TypeMap 已注入 Config 与 audit_logger。
    #[cfg(any(feature = "snowflake", feature = "uuid-v8"))]
    #[tokio::test]
    async fn test_kit_builder_builds_empty_ready_kit() {
        let kit = NebulaIdKitBuilder::new(local_config())
            .build()
            .await
            .expect("build 必须成功");

        // config 与 audit_logger 已注入 TypeMap（未注入审计日志器时值为 None）
        assert!(kit.inner().contains_config::<Config>());
        let audit = kit
            .inner()
            .config::<AuditLoggerInput>()
            .expect("audit_logger 必须已注入 TypeMap");
        assert!(audit.0.is_none(), "未注入审计日志器时应为 None");
    }

    /// DistributedLockModule 注册后 build 成功；锁可 acquire/release 且 is_healthy() 为 true。
    #[cfg(any(feature = "snowflake", feature = "uuid-v8"))]
    #[tokio::test]
    async fn test_kit_builds_distributed_lock_module() {
        let kit = NebulaIdKitBuilder::new(local_config())
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

    /// 注入仓储时 RepositoryModule 注册且能力可用：锁注入无 pub 观测接缝，
    /// 故断言能力落位（contains/require）+ 仓储可用性冒烟（get_db_connection）。
    #[cfg(feature = "database")]
    #[tokio::test]
    async fn test_repository_module_registered_when_injected() {
        use dbnexus::sea_orm::{DatabaseBackend, MockDatabase};

        let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();
        let kit = NebulaIdKitBuilder::new(local_config())
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

    /// 未注入仓储时 RepositoryModule 缺席且 build() 不失败（register_if 语义）。
    #[cfg(feature = "database")]
    #[tokio::test]
    async fn test_repository_module_absent_without_injection() {
        let kit = NebulaIdKitBuilder::new(local_config())
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

    /// 测试共享：单机配置（清空 etcd endpoints → 走 LocalDistributedLock，零外部依赖）。
    /// 注意 `Config::default()` 的 endpoints 非空，未配置可达 etcd 时 build 会显性失败。
    fn local_config() -> Config {
        let mut config = Config::default();
        config.etcd.endpoints = Vec::new();
        config
    }

    /// 测试共享：纯算法（snowflake）配置。
    fn snowflake_config() -> Config {
        let mut config = local_config();
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

    /// RouterModule 注册后 build 成功，require 返回已初始化 router（可直接生成 ID）。
    #[cfg(feature = "snowflake")]
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

    /// `on_ready` 启动降级后台任务：等待 ≥1 个 check_interval tick 后，
    /// 经 `get_algorithm_metrics()` 观测 snowflake 连续成功计数被刷新。
    #[cfg(feature = "snowflake")]
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

    /// `health_report()`（同步）返回包含 `router` 模块的报告。
    // kit build 要求 ≥1 个路由算法在场（空注册表 initialize 报错）；v5 不是路由算法，
    // 不计数（与 ci.yml「uuid-v8 only」同款门控）。
    #[cfg(any(feature = "snowflake", feature = "uuid-v8"))]
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

    /// health_summary 聚合：全模块健康时 status=healthy 且明细齐全。
    #[cfg(any(feature = "snowflake", feature = "uuid-v8"))]
    #[tokio::test]
    async fn test_health_summary_aggregates_worst_of() {
        let kit = NebulaIdKitBuilder::new(snowflake_config())
            .build()
            .await
            .expect("build 必须成功");

        let summary = kit.health_summary();
        assert_eq!(summary["status"], "healthy");
        assert_eq!(summary["healthy"], true);

        let modules = summary["modules"].as_array().expect("modules 必须为数组");
        assert!(
            modules
                .iter()
                .any(|m| m["module"] == "router" && m["status"] == "healthy"),
            "聚合明细必须含 router 模块，实际: {summary}"
        );
    }

    /// 纯算法（snowflake）在零仓储注入下可用。
    #[cfg(feature = "snowflake")]
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

    /// 零仓储注入下 Segment 请求返回 ConfigurationError，消息含 `with_repository`
    /// （错误消息是公共 API 的一部分，经 Display 可观测）。
    #[cfg(feature = "database")]
    #[tokio::test]
    async fn test_id_generator_segment_without_repository_returns_configuration_error() {
        // local_config()（同 Config::default()）默认算法为 segment
        let kit = NebulaIdKitBuilder::new(local_config())
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

    /// 注入仓储后 Segment 请求通过守卫并可生成（Segment 自带默认 loader，无需真实 DB）。
    #[cfg(feature = "database")]
    #[tokio::test]
    async fn test_id_generator_segment_with_repository_succeeds() {
        use dbnexus::sea_orm::{DatabaseBackend, MockDatabase};

        let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();
        let kit = NebulaIdKitBuilder::new(local_config())
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

    /// database 关闭组合下 Segment 请求由 router 显性拒绝（InvalidAlgorithmType）：
    /// 组合级运行时契约——不 panic、不静默产出 ID。
    #[cfg(all(
        not(feature = "database"),
        any(feature = "snowflake", feature = "uuid-v8")
    ))]
    #[tokio::test]
    async fn test_segment_request_rejected_without_database_feature() {
        let kit = NebulaIdKitBuilder::new(local_config())
            .build()
            .await
            .expect("build 必须成功（无 database 特性时仓储面不存在）");
        let generator = kit.id_generator().expect("id_generator() 必须成功");

        let err = generator
            .generate_with_algorithm(AlgorithmType::Segment, "ws", "group", "biz")
            .await
            .expect_err("无 database 特性时 Segment 请求必须显性失败");
        assert!(
            matches!(err, CoreError::InvalidAlgorithmType(_)),
            "期望 InvalidAlgorithmType（router 显性拒绝），实际：{err:?}"
        );
    }

    /// `IdGenerator` 为 Clone handle，克隆后可在独立 tokio 任务中生成（Send+Sync 运行时验证）。
    #[cfg(feature = "snowflake")]
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

    /// 完整 build 后四模块（除未注入的仓储）全部在场。
    #[cfg(any(feature = "snowflake", feature = "uuid-v8"))]
    #[tokio::test]
    async fn test_kit_full_build_all_modules_present() {
        let kit = NebulaIdKitBuilder::new(snowflake_config())
            .build()
            .await
            .expect("纯算法配置 build 必须成功");
        assert!(kit.inner().contains::<DistributedLockModule>());
        assert!(kit.inner().contains::<RouterModule>());
        assert!(kit.inner().contains::<IdGenModule>());
        #[cfg(feature = "database")]
        assert!(!kit.inner().contains::<RepositoryModule>());
    }

    /// 门面 `id_generator()` 取用的 handle 可 generate 与 batch_generate。
    #[cfg(feature = "snowflake")]
    #[tokio::test]
    async fn test_kit_id_generator_generates_and_batches() {
        let kit = NebulaIdKitBuilder::new(snowflake_config())
            .build()
            .await
            .expect("build 必须成功");
        let generator = kit.id_generator().expect("id_generator() 必须成功");

        let id = generator
            .generate("ws", "group", "biz")
            .await
            .expect("snowflake 生成必须成功");
        assert!(id.as_u128() > 0);

        let batch = generator
            .batch_generate("ws", "group", "biz", 10)
            .await
            .expect("snowflake 批量生成必须成功");
        assert_eq!(batch.ids.len(), 10);
    }

    /// 指定算法（uuid_v8）生成走通。
    #[cfg(feature = "uuid-v8")]
    #[tokio::test]
    async fn test_kit_generate_with_algorithm_uuid_v8() {
        let kit = NebulaIdKitBuilder::new(snowflake_config())
            .build()
            .await
            .expect("build 必须成功");
        let generator = kit.id_generator().expect("id_generator() 必须成功");

        let id = generator
            .generate_with_algorithm(AlgorithmType::UuidV8, "ws", "group", "biz")
            .await
            .expect("uuid_v8 生成必须成功");
        assert!(id.as_u128() > 0);
    }

    /// SDK 薄包装契约：kit 的 `derive_uuid_v5` 与 core 直调逐位一致；锚点值钉死跨仓兼容（etyma 存量 ID）。
    ///
    /// `snowflake` 门控：kit `build()` 要求 ≥1 个路由算法在场；纯委托的覆盖由 uuid_v5.rs 承载。
    #[cfg(all(feature = "uuid-v5", feature = "snowflake"))]
    #[tokio::test]
    async fn test_kit_derive_uuid_v5_wrapper_matches_core() {
        let kit = NebulaIdKitBuilder::new(snowflake_config())
            .build()
            .await
            .expect("build 必须成功");
        let generator = kit.id_generator().expect("id_generator() 必须成功");

        let namespace = uuid::Uuid::from_bytes([
            0x47, 0x9e, 0x67, 0xb5, 0x8d, 0x48, 0x51, 0x9e, 0xb7, 0x5c, 0x3c, 0x17, 0x65, 0xc4,
            0x5e, 0x9b,
        ]);
        let id = generator
            .derive_uuid_v5(namespace, "ecdict|abandon|en")
            .expect("derive 必须成功");
        assert_eq!(
            id.render(&IdFormat::Uuid, None, None).unwrap(),
            "21e7d32f-5500-5e4a-a943-219059f73c94"
        );
    }

    /// `health_check()` 保留算法级快照语义（`Vec<(AlgorithmType, HealthStatus)>`，与旧 API 一致）。
    #[cfg(feature = "snowflake")]
    #[tokio::test]
    async fn test_kit_health_check_returns_algorithm_snapshot() {
        let kit = NebulaIdKitBuilder::new(snowflake_config())
            .build()
            .await
            .expect("build 必须成功");

        let snapshot = kit.health_check().await;
        assert!(!snapshot.is_empty(), "算法级快照不可为空");
        assert!(
            snapshot.iter().any(|(t, _)| *t == AlgorithmType::Snowflake),
            "快照必须含 snowflake"
        );
    }

    /// 门面 `shutdown()` 后降级后台任务停止：先证明任务在运行，再等待一个完整
    /// interval 断言计数不再增长（经 `get_algorithm_metrics()` 观测）。
    #[cfg(feature = "snowflake")]
    #[tokio::test]
    async fn test_kit_shutdown_stops_background_check() {
        let kit = NebulaIdKitBuilder::new(snowflake_config())
            .build()
            .await
            .expect("build 必须成功");
        let router = kit
            .inner()
            .require::<RouterModule>()
            .expect("require router 必须成功");
        let dm = router.get_degradation_manager();

        // 首个 tick 立即触发；给足调度时间后确认后台任务确在运行
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let before = dm.get_algorithm_metrics()["snowflake"].consecutive_successes;
        assert!(before > 0, "shutdown 前后台任务应处于运行状态");

        kit.shutdown().await;

        // 等待超过一个完整 interval（默认 5000ms），若任务未停止计数会增长
        tokio::time::sleep(std::time::Duration::from_millis(5_500)).await;
        let after = dm.get_algorithm_metrics()["snowflake"].consecutive_successes;
        assert_eq!(
            before, after,
            "shutdown 后降级后台任务必须停止（指标不得再刷新）"
        );
    }

    /// 依赖图校验核心价值：裸 `AsyncKit` 只注册 `IdGenModule` 时 `build()` 必须 Err（缺依赖检测）。
    #[tokio::test]
    async fn test_kit_build_fails_on_missing_dependency() {
        let mut kit = trait_kit::AsyncKit::new();
        kit.register::<IdGenModule>()
            .expect("单模块注册本身应成功（图校验发生在 build()）");

        let result = kit.build().await;
        assert!(
            result.is_err(),
            "缺失依赖链（RouterModule/DistributedLockModule 未注册）时 build() 必须 Err"
        );
    }

    /// 8 并发 × 1000 次 snowflake 生成，去重后零重复。
    #[cfg(feature = "snowflake")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
    async fn test_kit_concurrent_snowflake_generation_is_unique() {
        use std::collections::HashSet;

        let kit = Arc::new(
            NebulaIdKitBuilder::new(snowflake_config())
                .build()
                .await
                .expect("build 必须成功"),
        );
        let generator = Arc::new(kit.id_generator().expect("id_generator() 必须成功"));

        let mut handles = Vec::new();
        for _ in 0..8 {
            let generator = generator.clone();
            handles.push(tokio::spawn(async move {
                let mut ids = Vec::with_capacity(1000);
                for _ in 0..1000 {
                    let id = generator
                        .generate("ws", "group", "biz")
                        .await
                        .expect("snowflake generate 必须成功");
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

    /// 显式指定 Segment 在零仓储注入下同样被守卫拒绝（generate_with_algorithm 路径）。
    #[cfg(feature = "database")]
    #[tokio::test]
    async fn test_kit_generate_with_algorithm_segment_without_repository_returns_configuration_error(
    ) {
        let kit = NebulaIdKitBuilder::new(snowflake_config())
            .build()
            .await
            .expect("build 必须成功");
        let generator = kit.id_generator().expect("id_generator() 必须成功");

        let err = generator
            .generate_with_algorithm(AlgorithmType::Segment, "ws", "group", "biz")
            .await
            .expect_err("显式 Segment 请求在无仓储时必须失败");
        assert!(matches!(err, CoreError::ConfigurationError(_)));
    }

    /// uuid_v8 批量生成零 DB 可用（旧 `client.rs` 用例迁移）。
    #[cfg(feature = "uuid-v8")]
    #[tokio::test]
    async fn test_kit_batch_generate_uuid_v8_without_repository() {
        let mut config = snowflake_config();
        config.algorithm.default = "uuid_v8".to_string();
        let kit = NebulaIdKitBuilder::new(config)
            .build()
            .await
            .expect("uuid_v8 批量应零 DB 可用");
        let generator = kit.id_generator().expect("id_generator() 必须成功");

        let batch = generator
            .batch_generate("ws", "group", "biz", 10)
            .await
            .expect("uuid_v8 批量生成必须成功");
        assert_eq!(batch.ids.len(), 10);
    }

    // ==================== 分布式锁 fail-closed 与号段装配契约 ====================

    /// 未配置 endpoints → 本地锁（单机合法，构建不依赖外部 etcd）。
    #[cfg(feature = "etcd")]
    #[tokio::test]
    async fn test_create_distributed_lock_local_when_no_endpoints() {
        let config = local_config();
        let lock = create_distributed_lock(&config)
            .await
            .expect("未配置 etcd 时必须放行本地锁");
        assert!(lock.is_healthy());
        let guard = lock
            .acquire("kit-local-key", 5)
            .await
            .expect("本地锁 acquire 必须成功");
        guard.release().await.expect("本地锁 release 必须成功");
    }

    /// fail-closed：配置要求 etcd 但端点不可达 → Err（lazy connect 可能 Ok，由 ping 探活
    /// 确定性拒绝），错误显性声明拒绝回退。
    #[cfg(feature = "etcd")]
    #[tokio::test]
    async fn test_create_distributed_lock_fails_closed_when_etcd_unreachable() {
        let mut config = local_config();
        config.etcd.endpoints = vec!["http://127.0.0.1:1".to_string()];
        config.etcd.connect_timeout_ms = 300;

        let result = create_distributed_lock(&config).await;
        let msg = match result {
            Err(e) => e.to_string(),
            Ok(_) => panic!("配置要求 etcd 但不可达时必须 Err（fail-closed）"),
        };
        assert!(
            msg.contains("refusing to fall back"),
            "错误必须显性声明拒绝回退，实际: {msg}"
        );
    }

    /// fail-closed 完整 kit build：etcd 不可达时锁模块失败必须让 build() 显性失败。
    #[cfg(feature = "etcd")]
    #[tokio::test]
    async fn test_kit_build_fails_when_etcd_required_but_unreachable() {
        let mut config = snowflake_config();
        config.etcd.endpoints = vec!["http://127.0.0.1:1".to_string()];
        config.etcd.connect_timeout_ms = 300;

        let result = NebulaIdKitBuilder::new(config).build().await;
        assert!(
            result.is_err(),
            "分布式锁模块 fail-closed 失败必须让 build() 显性失败，实际: {:?}",
            result.ok().map(|_| ())
        );
    }

    /// 号段装配契约：`DbSegmentLoader` 注入后 generate 消费 `allocate_segment` 的区间。
    /// 门控用 `segment` 而非 `database`（函数体引用的类型随 `segment` 编译）。
    #[cfg(feature = "segment")]
    #[tokio::test]
    async fn test_db_backed_segment_assembly_contract() {
        use crate::core::algorithm::{DbSegmentLoader, IdAlgorithm, SegmentAlgorithm};
        use crate::core::database::SegmentRepository;
        use crate::core::types::SegmentInfo;

        /// 受控 mock 仓储：allocate_segment 固定返回 [1000, 2000) 号段。
        struct MockRepo;

        #[async_trait::async_trait]
        impl SegmentRepository for MockRepo {
            async fn get_segment(
                &self,
                _workspace_id: &str,
                _biz_tag: &str,
            ) -> Result<Option<SegmentInfo>> {
                Ok(None)
            }

            async fn allocate_segment(
                &self,
                _workspace_id: &str,
                _biz_tag: &str,
                _step: i32,
            ) -> Result<SegmentInfo> {
                Ok(SegmentInfo {
                    id: 1,
                    workspace_id: "ws".to_string(),
                    biz_tag: "biz".to_string(),
                    current_id: 1000,
                    max_id: 2000,
                    step: 1000,
                    delta: 1,
                    created_at: chrono::Utc::now(),
                    updated_at: chrono::Utc::now(),
                })
            }

            async fn allocate_segment_with_dc(
                &self,
                _workspace_id: &str,
                _biz_tag: &str,
                _step: i32,
                _dc_id: i32,
            ) -> Result<SegmentInfo> {
                unimplemented!("装配契约单测只走 allocate_segment");
            }

            async fn update_segment(
                &self,
                _workspace_id: &str,
                _biz_tag: &str,
                _dc_id: i32,
                _current_id: i64,
                _max_id: i64,
            ) -> Result<()> {
                unimplemented!("装配契约单测只走 allocate_segment");
            }

            async fn create_segment(
                &self,
                _workspace_id: &str,
                _biz_tag: &str,
                _start_id: i64,
                _max_id: i64,
                _step: i32,
                _delta: i32,
            ) -> Result<SegmentInfo> {
                unimplemented!("装配契约单测只走 allocate_segment");
            }

            async fn list_segments(&self, _workspace_id: &str) -> Result<Vec<SegmentInfo>> {
                unimplemented!("装配契约单测只走 allocate_segment");
            }

            async fn delete_segment(&self, _workspace_id: &str, _biz_tag: &str) -> Result<()> {
                unimplemented!("装配契约单测只走 allocate_segment");
            }
        }

        let algo = SegmentAlgorithm::new(0)
            .with_segment_loader(Arc::new(DbSegmentLoader::new(Arc::new(MockRepo))));
        let id = algo
            .generate(&sample_ctx())
            .await
            .expect("DB 号段注入后 generate 必须成功");
        let value = id.as_u128();
        assert!(
            (1000..2000).contains(&value),
            "生成值必须落在 allocate_segment 返回区间 [1000, 2000) 内，实际: {value}"
        );
    }
}
