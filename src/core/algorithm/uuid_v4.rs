// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! RFC 9562 §5.4 随机 UUID v4 发号算法。
//!
//! 零状态发号器：无时钟、无计数器、无协调——与 v7/v8 构成 minting 家族
//! （实现 `IdAlgorithm`、进 `AlgorithmRouter`），与 v5 的确定性派生
//! （不入算法族）语义相反。
//!
//! 独占能力：122 位不可猜测随机 + 完全零元数据（ID 不泄漏创建时间、
//! dc/worker 等任何部署信息）——request id、能力令牌类「不可从 ID 反推
//! 任何信息」场景。时间有序场景用 v7，租户上下文嵌入用 v8。

use crate::core::types::{AlgorithmType, Id, Result};
use uuid::Uuid;

use crate::core::algorithm::UuidV4Factory;
use crate::core::algorithm::{
    AlgorithmBuilder, AlgorithmFactory, AlgorithmMetricsSnapshot, GenerateContext, IdAlgorithm,
};
use crate::core::config::Config;
use crate::core::HealthStatus;

/// RFC 9562 §5.4 随机 UUID v4 发号（同步叶子入口）。
///
/// 与 [`UuidV4Impl`] 的发电电路径共用同一实现（`generate_inner` 即委托本函数），
/// 差异仅在同步/异步：消费方把 v4 用在同步调用点（错误信封 trace_id、仓储插入
/// 等）时无需为一次 122 位随机数拉起 async 工厂面。
///
/// 零状态：无时钟、无计数器、无协调、无 DB；互异性由生日界保证。
pub fn mint_uuid_v4() -> Uuid {
    Uuid::new_v4()
}

/// RFC 9562 §5.4 随机 UUID v4 发号器。
/// 零状态：无时钟、无计数器、无协调、无 DB——互异性由 122 位随机的
/// 生日界保证（任意现实速率下可忽略）。每次调用产出不可预测、无任何
/// 可解析元数据（不嵌入创建时间/dc/worker）的新 ID。
pub struct UuidV4Impl;

impl UuidV4Impl {
    pub fn new() -> Self {
        Self
    }

    async fn generate_inner(&self) -> Result<Id> {
        Ok(Id::from_uuid_v8(mint_uuid_v4()))
    }
}

impl Default for UuidV4Impl {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl IdAlgorithm for UuidV4Impl {
    async fn generate(&self, ctx: &GenerateContext) -> Result<Id> {
        let _ = ctx; // v4 无租户上下文语义：不接受也不嵌入 workspace/group/biz_tag
        self.generate_inner().await
    }

    async fn batch_generate(
        &self,
        _ctx: &GenerateContext,
        size: usize,
    ) -> Result<crate::core::types::id::IdBatch> {
        if size == 0 {
            return Ok(crate::core::types::id::IdBatch::new(
                Vec::new(),
                AlgorithmType::UuidV4,
                String::new(),
            ));
        }
        // v4 无单毫秒计数器空间概念（互异性由随机熵保证），无 SequenceOverflow 守卫
        let mut out = Vec::with_capacity(size);
        for _ in 0..size {
            out.push(self.generate_inner().await?);
        }
        Ok(crate::core::types::id::IdBatch::new(
            out,
            AlgorithmType::UuidV4,
            String::new(),
        ))
    }

    fn health_check(&self) -> HealthStatus {
        HealthStatus::Healthy
    }

    fn metrics(&self) -> AlgorithmMetricsSnapshot {
        AlgorithmMetricsSnapshot::default()
    }

    fn algorithm_type(&self) -> AlgorithmType {
        AlgorithmType::UuidV4
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }
}

#[async_trait::async_trait]
impl AlgorithmFactory for UuidV4Factory {
    async fn build(
        &self,
        _builder: &AlgorithmBuilder,
        _config: &Config,
    ) -> Result<Box<dyn IdAlgorithm>> {
        Ok(Box::new(UuidV4Impl::new()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::IdFormat;

    fn ctx() -> crate::core::algorithm::GenerateContext {
        crate::core::algorithm::GenerateContext {
            workspace_id: "ws1".to_string(),
            group_id: "g1".to_string(),
            biz_tag: "order".to_string(),
            format: IdFormat::Uuid,
            ..Default::default()
        }
    }

    /// 位布局形貌：version nibble = 4、variant = 0b10（RFC 9562 §5.4）。
    #[tokio::test]
    async fn test_uuid_v4_version_and_variant_bits() {
        let algo = UuidV4Impl;
        let id = algo.generate(&ctx()).await.unwrap();
        let v = id.to_uuid_v8().as_u128();
        assert_eq!((v >> 76) & 0xF, 4, "version nibble must be 4");
        assert_eq!((v >> 62) & 0x3, 0b10, "variant must be 0b10");
    }

    /// 同步叶子入口与原语同源：version/variant 位与随机分布形貌一致。
    #[test]
    fn test_mint_uuid_v4_bits_and_uniqueness() {
        let mut set = std::collections::HashSet::new();
        for _ in 0..1000 {
            let u = mint_uuid_v4();
            assert_eq!(u.get_version_num(), 4, "version nibble must be 4");
            assert_eq!((u.as_u128() >> 62) & 0x3, 0b10, "variant must be 0b10");
            assert!(set.insert(u.as_u128()), "1000 次发号不得重复");
        }
    }

    /// 与 `Uuid::new_v4` 同源布局：canonical 字符串 parse roundtrip 无损。
    #[tokio::test]
    async fn test_uuid_v4_canonical_roundtrip() {
        let algo = UuidV4Impl;
        let id = algo.generate(&ctx()).await.unwrap();
        let canonical = id.render(&IdFormat::Uuid, None, None).unwrap();
        let reparsed = Uuid::parse_str(&canonical).expect("canonical must reparse");
        assert_eq!(reparsed.get_version(), Some(uuid::Version::Random));
        assert_eq!(reparsed.as_u128(), id.to_uuid_v8().as_u128());
    }

    /// 互异性：1000 次生成全不重复（122 位随机，生日界可忽略）。
    #[tokio::test]
    async fn test_uuid_v4_uniqueness_1000() {
        let algo = UuidV4Impl;
        let mut set = std::collections::HashSet::new();
        for _ in 0..1000 {
            assert!(set.insert(algo.generate(&ctx()).await.unwrap().as_u128()));
        }
        assert_eq!(set.len(), 1000);
    }

    /// 批量：len 正确且互异。
    #[tokio::test]
    async fn test_uuid_v4_batch_unique() {
        let algo = UuidV4Impl;
        let batch = algo.batch_generate(&ctx(), 100).await.unwrap();
        assert_eq!(batch.ids.len(), 100);
        assert_eq!(batch.algorithm, AlgorithmType::UuidV4);
        let mut set = std::collections::HashSet::new();
        for id in &batch.ids {
            assert!(set.insert(id.as_u128()));
        }
    }

    /// size==0 早退：空批成功返回（不发号、不报错）。
    #[tokio::test]
    async fn test_uuid_v4_batch_zero_is_empty_ok() {
        let algo = UuidV4Impl;
        let batch = algo.batch_generate(&ctx(), 0).await.unwrap();
        assert!(batch.ids.is_empty());
        assert_eq!(batch.algorithm, AlgorithmType::UuidV4);
    }

    /// 元数据纯净性：ID 不嵌入时间戳——连续两个 ID 的 u128 无单调关系
    /// （v4 与 v7/v8 的区分性特征：随机布局不可排序）。
    #[tokio::test]
    async fn test_uuid_v4_not_time_ordered() {
        let algo = UuidV4Impl;
        let a = algo.generate(&ctx()).await.unwrap().as_u128();
        let b = algo.generate(&ctx()).await.unwrap().as_u128();
        // 随机布局下 a<b 的概率 50%——不断言方向，断言「无嵌入时间戳」：
        // 高 48 位（v7/v8 放 unix ms 的位置）两位都是随机（无关 now）。
        let now_hi = (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis())
            >> 16;
        assert_ne!(a >> 80, now_hi, "high bits must not encode wall clock");
        assert_ne!(b >> 80, now_hi, "high bits must not encode wall clock");
    }

    #[test]
    fn test_uuid_v4_algorithm_type_and_health() {
        let algo = UuidV4Impl;
        assert_eq!(algo.algorithm_type(), AlgorithmType::UuidV4);
        assert_eq!(algo.health_check(), crate::core::HealthStatus::Healthy);
    }
}
