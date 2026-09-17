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

//! trait-kit Kit 范式装配的嵌入式 SDK 门面（feature `sdk` 门控）。

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
///（etcd 优先、fail-closed：配置要求 etcd 但构建失败 → build 返回错误，
/// 与 `main.rs` T016 行为一致）。
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
        create_distributed_lock(&config).await
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
    /// 后台任务（替代旧 SDK 构建器手工装配的第 4 步）。
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

    /// `on_shutdown`：停止降级后台任务。trait-kit 0.5.0-rc.5 起
    /// `AsyncKit::shutdown_async()` 接管执行——按逆拓扑序 drain 本钩子
    /// （依赖者先于被依赖者），门面 `NebulaIdKit::shutdown()` 直接 await。
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
/// 生成方法语义与旧 SDK 客户端完全等价（迁移非重设计）：默认算法解析、
/// `require_repository_for_segment` 守卫、`GenerateContext { format: Numeric,
/// prefix: None }` 构造均逐字保留。`Clone` 廉价（内部 `Arc`），可跨任务共享。
#[derive(Clone)]
pub struct IdGenerator {
    router: Arc<AlgorithmRouter>,
    repository: Option<Arc<SeaOrmRepository>>,
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

/// 分布式锁创建（T016 fail-closed）：`etcd` feature 且 endpoints 已配置 →
/// `EtcdDistributedLock`（构造后 ping 探活，lazy connect 不代表可达）；
/// 任一步失败返回 `Err`（SDK 宿主 `build()` 显性失败），**不再静默回退**
/// 进程内锁 —— 多实例部署静默回退必然重复 ID。未配置 endpoints →
/// `LocalDistributedLock`（单机合法）并 warn。
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

/// audit_logger 的 TypeMap 注入包装（`pub(crate)`，不 re-export，不扩大 SDK 公共面）。
///
/// RouterModule 经 `kit.config::<AuditLoggerInput>()` 拉取。`Option` 语义保留
/// 旧 SDK 构建器 audit_logger 字段的可选性。
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

    /// 取用 ID 生成能力（`require::<IdGenModule>()` 的薄封装）。
    ///
    /// `require` 的读锁成本只发生在本次分发的瞬间（O(1) map 查找）；此后
    /// `IdGenerator::generate*` 直调内部 `Arc<AlgorithmRouter>`，热路径零锁。
    pub fn id_generator(&self) -> Result<IdGenerator> {
        self.kit
            .require::<IdGenModule>()
            .map_err(|e| CoreError::InternalError(format!("id-generation 能力缺失: {e}")))
    }

    /// 各算法健康状态快照（直调 router，语义与旧 API 一致）。
    ///
    /// RouterModule 能力在构建成功的 Ready Kit 上必然存在（`build()` 依赖图
    /// 校验已保证依赖链完整）——此处 `expect` 是内部不变量断言，显性失败，
    /// 不吞错。
    pub async fn health_check(&self) -> Vec<(AlgorithmType, crate::core::types::HealthStatus)> {
        let router = self
            .kit
            .require::<RouterModule>()
            .expect("RouterModule 能力在 Ready Kit 上必然存在（build 图校验已保证）");
        router.health_check().await
    }

    /// 模块级健康报告（trait-kit `health` feature，同步方法）。
    ///
    /// 返回 trait-kit 的 `HealthStatus`（与 nebulaid 核心类型同名不同源）。
    pub fn health_report(&self) -> Vec<(&'static str, trait_kit::HealthStatus)> {
        self.kit.health_report()
    }

    /// 聚合健康摘要（吸收 trait-kit sync 侧 `health_aggregate`/`health_json`
    /// 语义到 async 侧：worst-of 全局状态 + 模块明细）。
    ///
    /// `status` 取全部模块中最差者（unhealthy > degraded > healthy）；
    /// `healthy` 为 `status == "healthy"` 的便捷位；`modules` 为逐模块明细。
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

    /// 停机：await `AsyncKit::shutdown_async()`，由 trait-kit 按逆拓扑序
    /// 执行各模块 `on_shutdown` 钩子（RouterModule 执行
    /// `stop_background_check().await`，语义等价旧 SDK 客户端 shutdown）。
    ///
    /// **迁移注**：trait-kit 0.5.0-rc.1 的 `AsyncKit::shutdown()` 是同步方法、
    /// 不执行 async 回调，彼时门面需手动调用 `RouterModule::on_shutdown`；
    /// rc.5 提供 one-shot 的 `shutdown_async()`（二次调用 no-op）后回归纯钩子。
    pub async fn shutdown(self) {
        self.kit.shutdown_async().await;
    }

    /// 是否注入了数据库仓储（决定 Segment 可用性；语义 = 旧
    /// 旧 SDK 客户端 has_repository，经 TypeMap optional 拉取判定）。
    pub fn has_repository(&self) -> bool {
        self.kit.optional::<RepositoryModule>().is_some()
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

    /// 默认 Config 下 `build()` 成功返回 Ready Kit，TypeMap 中
    /// 已注入 Config 与 audit_logger（空图断言随模块演进由
    /// `test_health_report_contains_router_module` / `test_kit_full_build_*`
    /// 取代）。
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

    /// `DistributedLockModule` 注册后 `build()` 成功；`require`
    /// 返回的锁可 `acquire`/`release` 且 `is_healthy()` 为 true
    ///（`DistributedLock` trait 无 `lock()/unlock()` 方法，断言只走既有面）。
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

    /// 注入仓储时 `RepositoryModule` 注册且 `require` 返回的仓储
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

    /// 未注入仓储时 `RepositoryModule` 缺席且 `build()` 不失败
    ///（`register_if` 语义——纯算法零 DB 场景成立）。
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

    /// 测试共享：单机配置（清空 etcd endpoints → T016 fail-closed 语义下
    /// `create_distributed_lock` 走 LocalDistributedLock，构建零外部依赖）。
    /// 注意 `Config::default()` 的 etcd.endpoints 非空（["etcd:2379"]），
    /// 在 T016 之后未配置可达 etcd 的默认配置会让 build 显性失败。
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

    /// RouterModule 注册后 build 成功，require 返回已初始化
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

    /// `on_ready` 启动降级后台任务 ——等待 ≥1 个 check_interval
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

    /// `health_report()`（同步）基于 trait-kit health feature
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

    /// health_summary 聚合：全模块健康时 status=healthy 且明细齐全。
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

    /// 零仓储注入下 Segment 请求返回 `CoreError::ConfigurationError`
    /// 且消息文本含 `with_repository`（错误消息是公共 API 的一部分，经 Display
    /// 可观测）。
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

    /// 注入仓储后 Segment 请求通过守卫并可生成
    ///（Segment 算法自带默认 loader，无需真实 DB；守卫放行即语义达成）。
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

    /// `IdGenerator` 为 Clone handle，克隆后在独立 tokio 任务中
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

    /// 完整 build 后四模块（除未注入的仓储）全部在场。
    #[tokio::test]
    async fn test_kit_full_build_all_modules_present() {
        let kit = NebulaIdKitBuilder::new(snowflake_config())
            .build()
            .await
            .expect("纯算法配置 build 必须成功");
        assert!(kit.inner().contains::<DistributedLockModule>());
        assert!(kit.inner().contains::<RouterModule>());
        assert!(kit.inner().contains::<IdGenModule>());
        assert!(!kit.inner().contains::<RepositoryModule>());
    }

    /// 门面 `id_generator()` 取用的 handle 可 generate 与
    /// batch_generate。
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

    /// `health_check()` 保留算法级快照语义（返回
    /// `Vec<(AlgorithmType, HealthStatus)>`，与旧 API 一致）。
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

    /// 门面 `shutdown()` 后降级后台任务停止 ——先证明任务在运行
    ///（≥1 tick 后计数刷新），再 shutdown，等待一个完整 interval 断言计数不再
    /// 增长（pub 接缝 `get_algorithm_metrics()`，禁止依赖私有字段）。
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

    /// 依赖图校验核心价值 ——裸 `AsyncKit` 只注册 `IdGenModule`
    /// 而不注册其依赖链（RouterModule→DistributedLockModule），`build()` 必须
    /// 返回 `Err`（缺依赖检测，不允许绕过）。
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

    /// 8 并发 × 1000 次 snowflake 生成，去重后零重复
    ///（旧 `client.rs` `sdk_concurrent_snowflake_generation_is_unique` 迁移）。
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

    /// 显式指定 Segment 算法在零仓储注入下同样被守卫拒绝
    ///（`generate_with_algorithm` 路径，旧 `client.rs` 用例覆盖）。
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

    // ==================== T016: 分布式锁 fail-closed 与号段装配契约 ====================

    /// T016 —— 未配置 endpoints → 本地锁（单机合法，构建不依赖外部 etcd）。
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

    /// T016 fail-closed —— 配置要求 etcd 但端点不可达 → `create_distributed_lock`
    /// 返回 `Err`（lazy connect 下 `EtcdClientWrapper::new` 可能 Ok，由 ping 探活
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

    /// T016 fail-closed —— 完整 kit build 路径：配置要求 etcd 但不可达 →
    /// `DistributedLockModule` 构建失败必须让 `build()` 显性失败。
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

    /// T016 verify 钉 —— 号段装配契约：`DbSegmentLoader`（真连仓储）经
    /// `SegmentAlgorithm::new(dc).with_segment_loader(...)` 注入后，generate
    /// 消费的就是 `allocate_segment` 返回的号段区间（生产装配的算法级语义）。
    #[tokio::test]
    async fn test_db_backed_segment_assembly_contract() {
        use crate::core::algorithm::{
            DbSegmentLoader, IdAlgorithm, SegmentAlgorithm,
        };
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
