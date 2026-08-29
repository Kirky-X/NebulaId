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

//! Time-ordered RFC 9562 v8 structured UUID algorithm.
//!
//! Layout (RFC 9562 §5.8, 128 bits):
//! - `custom_a` (bits 0..48): 48-bit Unix millisecond timestamp (sortable).
//! - `custom_b` (bits 48..60): `dc(3) << 9 | worker(8) << 1 | counter_hi(1)`.
//! - `custom_c` (bits 66..128): `shard(16) << 46 | counter_lo(20) << 26 | rand(26)`.
//!
//! Composed manually into a `u128`, with version = `0b1000` (v8) at bits 48..52
//! and variant = `0b10` at bits 64..66, then wrapped via `Uuid::from_u128`.
//!
//! This design follows the UUIDP "Cluster" scheme (random per-instance start +
//! strictly monotonic counter, O(nd/m) collision probability) and embeds
//! tenant/zone context (shard, dc, worker) directly into the ID for
//! observability and partition tolerance �?matching the GenoID v8 composition.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Mutex;
use uuid::Uuid;

use crate::core::algorithm::traits::{
    AlgorithmBuilder, AlgorithmFactory, AlgorithmMetricsSnapshot, GenerateContext, IdAlgorithm,
};
use crate::core::algorithm::UuidV8Factory;
use crate::core::config::Config;
use crate::core::types::id::{AlgorithmType, Id, IdBatch};
use crate::core::types::{CoreError, Result};
use crate::core::HealthStatus;

const NODE_DC_BITS: u32 = 3;
const NODE_WORKER_BITS: u32 = 8;
const COUNTER_HI_BITS: u32 = 1;
const SHARD_BITS: u32 = 16;
const COUNTER_LO_BITS: u32 = 20;
const RAND_BITS: u32 = 26;

const NODE_DC_MAX: u64 = (1u64 << NODE_DC_BITS) - 1;
const NODE_WORKER_MAX: u64 = (1u64 << NODE_WORKER_BITS) - 1;
const SHARD_MASK: u64 = (1u64 << SHARD_BITS) - 1;
const COUNTER_LO_MASK: u64 = (1u64 << COUNTER_LO_BITS) - 1;
const RAND_MASK: u64 = (1u64 << RAND_BITS) - 1;
const COUNTER_TOTAL_BITS: u64 = (COUNTER_HI_BITS + COUNTER_LO_BITS) as u64;
const COUNTER_TOTAL_MASK: u64 = (1u64 << COUNTER_TOTAL_BITS) - 1;

/// 时间回拨最大容忍（毫秒）。超过则视为严重异常�?
const MAX_CLOCK_BACKWARD_MS: u64 = 5000;

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn shard_of(workspace_id: &str, group_id: &str, biz_tag: &str) -> u64 {
    if workspace_id.is_empty() && group_id.is_empty() && biz_tag.is_empty() {
        return 0;
    }
    let mut buf = Vec::with_capacity(workspace_id.len() + group_id.len() + biz_tag.len() + 4);
    buf.extend_from_slice(workspace_id.as_bytes());
    buf.push(b' ');
    buf.extend_from_slice(group_id.as_bytes());
    buf.push(b' ');
    buf.extend_from_slice(biz_tag.as_bytes());
    // 内联 FNV-1a 64 位哈希，避免额外依赖；取低 16 位作为分片索引。
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in &buf {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash & SHARD_MASK
}

fn fast_rand_26() -> u64 {
    // 使用高分辨率计时器的低 26 位作为轻量随机源。
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    (t ^ (t >> 27)).wrapping_mul(0x2545F4914F6CDD1D) & RAND_MASK
}

/// 时间有序 v8 UUID 算法实现。
pub struct UuidV8Impl {
    last_timestamp: AtomicU64,
    counter: AtomicU64,
    dc_id: u64,
    worker_id: u64,
    shard_cache: Mutex<HashMap<(String, String, String), u64>>,
}

impl UuidV8Impl {
    pub fn new(dc_id: u64, worker_id: u64) -> Self {
        let dc = dc_id & NODE_DC_MAX;
        let worker = worker_id & NODE_WORKER_MAX;
        let seed = fast_rand_26() << (COUNTER_LO_BITS + NODE_WORKER_BITS + NODE_DC_BITS);
        Self {
            last_timestamp: AtomicU64::new(0),
            counter: AtomicU64::new(seed & COUNTER_TOTAL_MASK),
            dc_id: dc,
            worker_id: worker,
            shard_cache: Mutex::new(HashMap::new()),
        }
    }

    fn shard_for(&self, ctx: &GenerateContext) -> u64 {
        let key = (
            ctx.workspace_id.clone(),
            ctx.group_id.clone(),
            ctx.biz_tag.clone(),
        );
        if let Some(v) = self.shard_cache.lock().unwrap().get(&key) {
            return *v;
        }
        let s = shard_of(&ctx.workspace_id, &ctx.group_id, &ctx.biz_tag);
        self.shard_cache.lock().unwrap().insert(key, s);
        s
    }

    /// 生成单个时间有序 v8 UUID。
    fn generate_inner(&self, ctx: &GenerateContext) -> Result<Id> {
        let ts = now_unix_ms();

        let mut last = self.last_timestamp.load(Ordering::Acquire);
        let mut effective_ts = ts;
        loop {
            if effective_ts < last {
                if last - effective_ts > MAX_CLOCK_BACKWARD_MS {
                    return Err(CoreError::ClockMovedBackward {
                        last_timestamp: last - effective_ts,
                    });
                }
                effective_ts = last;
            }
            match self.last_timestamp.compare_exchange_weak(
                last,
                effective_ts,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => break,
                Err(x) => {
                    last = x;
                }
            }
        }

        let counter = self.counter.fetch_add(1, Ordering::Relaxed) & COUNTER_TOTAL_MASK;

        let custom_b = (self.dc_id << (COUNTER_HI_BITS + NODE_WORKER_BITS))
            | (self.worker_id << COUNTER_HI_BITS)
            | (counter >> COUNTER_LO_BITS);

        let shard = self.shard_for(ctx);
        let rand = fast_rand_26();
        let custom_c = (shard << (COUNTER_LO_BITS + RAND_BITS))
            | ((counter & COUNTER_LO_MASK) << RAND_BITS)
            | rand;

        // RFC 9562 v8 位布局（u128 大端）：
        //  bits 80..128 (48) = custom_a (unix_ts_ms)
        //  bits 64..80  (16) = time_hi_and_version = (version=8)<<12 | custom_b(12)
        //  bits 56..64  (8)  = clock_seq_hi_and_reserved = (variant=10)<<6 | custom_c[56..62]
        //  bits  0..56  (56) = node = custom_c[0..56]
        let version_field: u128 = (8u128 << 12) | (custom_b as u128 & 0xFFF);
        let variant_field: u128 = ((0b10u128 << 6) | ((custom_c as u128 >> 56) & 0x3F)) << 56;
        let node_field: u128 = (custom_c as u128) & 0x00FF_FFFF_FFFF_FFFF;

        let mut value: u128 = ((effective_ts as u128) & 0xFFFF_FFFFFFFF) << 80;
        value |= version_field << 64;
        value |= variant_field;
        value |= node_field;

        Ok(Id::from_uuid_v8(Uuid::from_u128(value)))
    }

    fn batch_inner(&self, ctx: &GenerateContext, n: usize) -> Result<Vec<Id>> {
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            out.push(self.generate_inner(ctx)?);
        }
        Ok(out)
    }
}

impl Default for UuidV8Impl {
    fn default() -> Self {
        Self::new(0, 0)
    }
}

#[async_trait]
impl IdAlgorithm for UuidV8Impl {
    async fn generate(&self, ctx: &GenerateContext) -> Result<Id> {
        self.generate_inner(ctx)
    }

    async fn batch_generate(&self, ctx: &GenerateContext, size: usize) -> Result<IdBatch> {
        if size == 0 {
            return Ok(IdBatch::new(
                Vec::new(),
                AlgorithmType::UuidV8,
                String::new(),
            ));
        }
        let ids = self.batch_inner(ctx, size)?;
        Ok(IdBatch::new(ids, AlgorithmType::UuidV8, String::new()))
    }

    fn health_check(&self) -> HealthStatus {
        HealthStatus::Healthy
    }

    fn metrics(&self) -> AlgorithmMetricsSnapshot {
        AlgorithmMetricsSnapshot {
            total_generated: 0,
            total_failed: 0,
            current_qps: 0,
            p50_latency_us: 0,
            p99_latency_us: 0,
            cache_hit_rate: None,
        }
    }

    fn algorithm_type(&self) -> AlgorithmType {
        AlgorithmType::UuidV8
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }
}

/// UUID v8 算法工厂，注册在 `algorithm_factories()`。

#[async_trait]
impl AlgorithmFactory for UuidV8Factory {
    async fn build(
        &self,
        _builder: &AlgorithmBuilder,
        _config: &Config,
    ) -> Result<Box<dyn IdAlgorithm>> {
        Ok(Box::new(UuidV8Impl::new(
            _config.app.dc_id as u64,
            _config.app.worker_id as u64,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> GenerateContext {
        GenerateContext {
            workspace_id: "ws1".to_string(),
            group_id: "g1".to_string(),
            biz_tag: "order".to_string(),
            format: crate::core::types::id::IdFormat::Uuid,
            ..Default::default()
        }
    }

    #[test]
    fn test_uuid_v8_is_time_ordered_and_v8() {
        let algo = UuidV8Impl::new(1, 1);
        let a = algo.generate_inner(&ctx()).unwrap();
        let b = algo.generate_inner(&ctx()).unwrap();
        assert_eq!(a.to_uuid_v8().get_version(), Some(uuid::Version::Custom));
        // 单调递增（同毫秒 counter 递增）
        let av = a.to_uuid_v8().as_u128();
        let bv = b.to_uuid_v8().as_u128();
        assert!(bv > av, "uuid_v8 must be monotonic increasing");
    }

    #[test]
    fn test_uuid_v8_layout_fields() {
        // 直接验证位布局：custom_a = ts, version=v8, variant=10
        let algo = UuidV8Impl::new(1, 1);
        let id = algo.generate_inner(&ctx()).unwrap();
        let v = id.to_uuid_v8().as_u128();
        // version bits 48..52
        let version = ((v >> 76) & 0xF) as u8;
        assert_eq!(version, 8);
        // variant bits 64..66 == 0b10
        let variant = ((v >> 62) & 0x3) as u8;
        assert_eq!(variant, 0b10);
    }

    #[test]
    fn test_uuid_v8_unique_across_workers_and_time_ordered() {
        // K 个不同节点，每个生成 N 个 ID，全局必须唯一；单节点内必须单调非递减
        const K: u64 = 4;
        const N: usize = 1000;
        let mut all: std::collections::HashSet<u128> = std::collections::HashSet::new();
        for w in 0..K {
            let algo = UuidV8Impl::new((w % 2) as u64, w as u64);
            let mut last: u128 = 0;
            for _ in 0..N {
                let id = algo.generate_inner(&ctx()).unwrap();
                let v = id.to_uuid_v8().as_u128();
                assert!(
                    all.insert(v),
                    "uuid_v8 must be globally unique across nodes"
                );
                assert!(v >= last, "uuid_v8 must be non-decreasing within a node");
                last = v;
            }
        }
        assert_eq!(all.len(), (K as usize) * N);
    }

    #[test]
    fn test_uuid_v8_embedded_timestamp_matches_now() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let algo = UuidV8Impl::new(1, 1);
        let id = algo.generate_inner(&ctx()).unwrap();
        let v = id.to_uuid_v8().as_u128();
        // custom_a 位于最高 48 位
        let ts = (v >> 80) as u64;
        assert!(
            ts.abs_diff(now) <= 1000,
            "embedded ts {ts} should be near now {now}"
        );
    }

    #[test]
    fn test_uuid_v8_batch_generate_unique() {
        let algo = UuidV8Impl::new(3, 7);
        let ids = algo.batch_inner(&ctx(), 500).unwrap();
        let mut set: std::collections::HashSet<u128> = std::collections::HashSet::new();
        let mut last: u128 = 0;
        for id in &ids {
            let v = id.to_uuid_v8().as_u128();
            assert!(set.insert(v), "batch ids must be unique");
            assert!(v >= last, "batch ids must be non-decreasing");
            last = v;
        }
        assert_eq!(set.len(), 500);
    }
}
