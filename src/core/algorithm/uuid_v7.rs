// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! RFC 9562 §5.7 时间有序 UUID v7 发号算法；零自持状态，进程内严格保序：
//! 48 位 unix ms 时间戳与单调计数器由 uuid crate 的 now_v7 共享上下文提供。
//! ⚠️ now_v7 的进程级 Mutex 计数器多线程吞吐负扩展，高频多线程场景须权衡。

use crate::core::types::{AlgorithmType, Id, Result};
use uuid::Uuid;

use crate::core::algorithm::UuidV7Factory;
use crate::core::algorithm::{
    AlgorithmBuilder, AlgorithmFactory, AlgorithmMetricsSnapshot, GenerateContext, IdAlgorithm,
};
use crate::core::config::Config;
use crate::core::HealthStatus;

/// RFC 9562 §5.7 时间有序 UUID v7 发号器；零自持状态，结构体不带字段。
pub struct UuidV7Impl;

impl UuidV7Impl {
    pub fn new() -> Self {
        Self
    }

    async fn generate_inner(&self) -> Result<Id> {
        Ok(Id::from_uuid_v8(Uuid::now_v7()))
    }
}

impl Default for UuidV7Impl {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl IdAlgorithm for UuidV7Impl {
    async fn generate(&self, _ctx: &GenerateContext) -> Result<Id> {
        // v7 无租户上下文语义：不接受也不嵌入 workspace/group/biz_tag。
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
                AlgorithmType::UuidV7,
                String::new(),
            ));
        }
        // 同毫秒保序由 crate 计数器保证；批量无计数器空间守卫（74 位随机+计数器空间）
        let mut out = Vec::with_capacity(size);
        for _ in 0..size {
            out.push(self.generate_inner().await?);
        }
        Ok(crate::core::types::id::IdBatch::new(
            out,
            AlgorithmType::UuidV7,
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
        AlgorithmType::UuidV7
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }
}

#[async_trait::async_trait]
impl AlgorithmFactory for UuidV7Factory {
    async fn build(
        &self,
        _builder: &AlgorithmBuilder,
        _config: &Config,
    ) -> Result<Box<dyn IdAlgorithm>> {
        Ok(Box::new(UuidV7Impl::new()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::IdFormat;

    fn ctx() -> GenerateContext {
        GenerateContext {
            workspace_id: "ws1".to_string(),
            group_id: "g1".to_string(),
            biz_tag: "order".to_string(),
            format: IdFormat::Uuid,
            ..Default::default()
        }
    }

    /// 位布局形貌：version nibble = 7、variant = 0b10（RFC 9562 §5.7）。
    #[tokio::test]
    async fn test_uuid_v7_version_and_variant_bits() {
        let algo = UuidV7Impl;
        let id = algo.generate(&ctx()).await.unwrap();
        let v = id.to_uuid_v8().as_u128();
        assert_eq!((v >> 76) & 0xF, 7, "version nibble must be 7");
        assert_eq!((v >> 62) & 0x3, 0b10, "variant must be 0b10");
    }

    /// 独占能力 1：标准布局时间戳可被 `get_timestamp` 直取，且与墙钟一致。
    #[tokio::test]
    async fn test_uuid_v7_standard_timestamp_extractable() {
        let algo = UuidV7Impl;
        let before = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let id = algo.generate(&ctx()).await.unwrap();
        let after = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let uuid = id.to_uuid_v8();
        let ts = uuid
            .get_timestamp()
            .expect("v7 must carry an extractable timestamp");
        let unix_secs = ts.to_unix().0;
        assert!(
            unix_secs >= before && unix_secs <= after,
            "embedded timestamp {unix_secs} must be within [{before}, {after}]"
        );
    }

    /// 独占能力 2：零配置零自持状态——`new` 不消费任何配置（对比 v8 从 Config.app 取 dc/worker）。
    #[test]
    fn test_uuid_v7_zero_config_zero_state() {
        let algo = UuidV7Impl::new();
        assert_eq!(algo.algorithm_type(), AlgorithmType::UuidV7);
        assert_eq!(algo.health_check(), crate::core::HealthStatus::Healthy);
    }

    /// 进程内保序：连续 1000 次生成的 u128 非递减（crate 共享计数器保序）。
    #[tokio::test]
    async fn test_uuid_v7_monotonic_1000() {
        let algo = UuidV7Impl;
        let mut last: u128 = 0;
        for _ in 0..1000 {
            let v = algo.generate(&ctx()).await.unwrap().as_u128();
            assert!(v >= last, "v7 must be non-decreasing within process");
            last = v;
        }
    }

    /// 批量：len 正确、互异、保序。
    #[tokio::test]
    async fn test_uuid_v7_batch_unique_and_ordered() {
        let algo = UuidV7Impl;
        let batch = algo.batch_generate(&ctx(), 100).await.unwrap();
        assert_eq!(batch.ids.len(), 100);
        assert_eq!(batch.algorithm, AlgorithmType::UuidV7);
        let mut set = std::collections::HashSet::new();
        let mut last: u128 = 0;
        for id in &batch.ids {
            assert!(set.insert(id.as_u128()), "batch ids must be unique");
            assert!(id.as_u128() >= last, "batch must stay ordered");
            last = id.as_u128();
        }
    }

    /// size==0 早退：空批成功返回（不发号、不报错）。
    #[tokio::test]
    async fn test_uuid_v7_batch_zero_is_empty_ok() {
        let algo = UuidV7Impl;
        let batch = algo.batch_generate(&ctx(), 0).await.unwrap();
        assert!(batch.ids.is_empty());
        assert_eq!(batch.algorithm, AlgorithmType::UuidV7);
    }

    /// canonical roundtrip：标准连字符字符串可 reparse 且值无损。
    #[tokio::test]
    async fn test_uuid_v7_canonical_roundtrip() {
        let algo = UuidV7Impl;
        let id = algo.generate(&ctx()).await.unwrap();
        let canonical = id.render(&IdFormat::Uuid, None, None).unwrap();
        let reparsed = Uuid::parse_str(&canonical).expect("canonical must reparse");
        assert_eq!(reparsed.get_version(), Some(uuid::Version::SortRand));
        assert_eq!(reparsed.as_u128(), id.to_uuid_v8().as_u128());
    }
}
