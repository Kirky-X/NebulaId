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

use trait_kit::{AsyncKit, AsyncReady};

use crate::core::algorithm::DynAuditLogger;
use crate::core::config::Config;
use crate::core::database::SeaOrmRepository;
use crate::core::types::Result;
use crate::core::CoreError;

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
        let kit = AsyncKit::new();

        // TypeMap 常驻配置：Config 与 audit_logger（后续模块从 TypeMap 拉取）
        kit.set_config(self.config.clone());
        kit.set_config(AuditLoggerInput(self.audit_logger));

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
        assert!(kit.kit.health_report().is_empty());
    }
}
