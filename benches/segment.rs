// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Criterion 微基准：Segment 号段算法与路由链路（feature `segment`）。
//!
//! 运行：`cargo bench --bench segment`
//!
//! 覆盖：稳态单条/批量（buffer 内命中）、跨段边界（含 loader 调用摊销）、
//! `AlgorithmRouter` 经 `with_segment_repository` 的 Segment 分发。
//! 号段来源为进程内 stub，**不含真实数据库往返**；端到端口径见 PERFORMANCE.md。
use std::hint::black_box;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use criterion::{criterion_group, criterion_main, Criterion};
use nebulaid::core::algorithm::{
    AlgorithmRouter, GenerateContext, IdAlgorithm, SegmentAlgorithm, SegmentData, SegmentLoader,
};
use nebulaid::core::config::Config;
use nebulaid::core::database::SegmentRepository;
use nebulaid::core::types::{IdFormat, Result, SegmentInfo};

/// 需 multi-thread runtime：路由 `initialize` 与 Segment 健康巡检均要求。
fn bench_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("bench tokio runtime must build")
}

fn bench_ctx() -> GenerateContext {
    GenerateContext {
        workspace_id: "bench".to_string(),
        group_id: "bench".to_string(),
        biz_tag: "bench".to_string(),
        format: IdFormat::Numeric,
        prefix: None,
    }
}

/// 进程内号段装载器：每次返回 `[start, start + span)`，起点单调递增。
///
/// `span` 决定跨段频率：大 span 测稳态热路径；小 span（如 1024）把
/// `load_segment` 调用摊销进单条生成，覆盖双缓冲切换成本。
struct MonotonicLoader {
    next_start: AtomicU64,
    span: u64,
}

impl MonotonicLoader {
    fn new(span: u64) -> Self {
        Self {
            next_start: AtomicU64::new(1),
            span,
        }
    }
}

#[async_trait]
impl SegmentLoader for MonotonicLoader {
    async fn load_segment(&self, _ctx: &GenerateContext, _worker_id: u8) -> Result<SegmentData> {
        let start = self.next_start.fetch_add(self.span, Ordering::Relaxed);
        Ok(SegmentData {
            start_id: start,
            max_id: start + self.span,
        })
    }
}

/// 构建并初始化一个经 stub loader 装配的 Segment 算法（与工厂路径同构）。
fn segment_algorithm(rt: &tokio::runtime::Runtime, span: u64) -> SegmentAlgorithm {
    let mut algo =
        SegmentAlgorithm::new(0).with_segment_loader(Arc::new(MonotonicLoader::new(span)));
    rt.block_on(algo.initialize(&Config::default()))
        .expect("segment initialize must succeed");
    algo
}

/// 稳态单条生成：key 构造 + buffers 读锁 + 段内消费 + 指标计数。
fn bench_segment_generate(c: &mut Criterion) {
    let rt = bench_runtime();
    let algo = segment_algorithm(&rt, 1_000_000);
    let ctx = bench_ctx();

    c.bench_function("segment/generate_steady", |b| {
        b.iter(|| {
            let id = rt.block_on(algo.generate(black_box(&ctx))).unwrap();
            black_box(id)
        })
    });
}

/// 批量生成摊销曲线：10 / 100 / 1000 / 10000。
fn bench_segment_batch_generate(c: &mut Criterion) {
    let rt = bench_runtime();
    let algo = segment_algorithm(&rt, 1_000_000);
    let ctx = bench_ctx();

    for size in [10usize, 100, 1_000, 10_000] {
        c.bench_function(&format!("segment/batch_generate_{size}"), |b| {
            b.iter(|| {
                let batch = rt
                    .block_on(algo.batch_generate(black_box(&ctx), size))
                    .unwrap();
                black_box(batch)
            })
        });
    }
}

/// 跨段边界：span=1024，每 1024 条触发一次 loader 与双缓冲切换。
fn bench_segment_boundary(c: &mut Criterion) {
    let rt = bench_runtime();
    let algo = segment_algorithm(&rt, 1_024);
    let ctx = bench_ctx();

    c.bench_function("segment/generate_boundary_span_1024", |b| {
        b.iter(|| {
            let id = rt.block_on(algo.generate(black_box(&ctx))).unwrap();
            black_box(id)
        })
    });
}

/// 进程内号段仓储：`allocate_segment` 返回单调区间，其余方法空实现。
struct MonotonicRepo {
    next_start: AtomicU64,
    span: i64,
}

impl MonotonicRepo {
    fn new(span: i64) -> Self {
        Self {
            next_start: AtomicU64::new(1),
            span,
        }
    }

    fn next_segment(&self, workspace_id: &str, biz_tag: &str) -> SegmentInfo {
        let start = self
            .next_start
            .fetch_add(self.span as u64, Ordering::Relaxed) as i64;
        SegmentInfo {
            id: 1,
            workspace_id: workspace_id.to_string(),
            biz_tag: biz_tag.to_string(),
            current_id: start,
            max_id: start + self.span,
            step: self.span as u32,
            delta: 1,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }
}

#[async_trait]
impl SegmentRepository for MonotonicRepo {
    async fn get_segment(
        &self,
        _workspace_id: &str,
        _biz_tag: &str,
    ) -> Result<Option<SegmentInfo>> {
        Ok(None)
    }

    async fn allocate_segment(
        &self,
        workspace_id: &str,
        biz_tag: &str,
        _step: i32,
    ) -> Result<SegmentInfo> {
        Ok(self.next_segment(workspace_id, biz_tag))
    }

    async fn allocate_segment_with_dc(
        &self,
        workspace_id: &str,
        biz_tag: &str,
        _step: i32,
        _dc_id: i32,
    ) -> Result<SegmentInfo> {
        Ok(self.next_segment(workspace_id, biz_tag))
    }

    async fn update_segment(
        &self,
        _workspace_id: &str,
        _biz_tag: &str,
        _dc_id: i32,
        _current_id: i64,
        _max_id: i64,
    ) -> Result<()> {
        Ok(())
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
        unimplemented!("bench 只走 allocate_segment")
    }

    async fn list_segments(&self, _workspace_id: &str) -> Result<Vec<SegmentInfo>> {
        Ok(Vec::new())
    }

    async fn delete_segment(&self, _workspace_id: &str, _biz_tag: &str) -> Result<()> {
        Ok(())
    }
}

/// 路由链路：`with_segment_repository` 装配（生产形态），覆盖路由查表、
/// 降级观测与 `DbSegmentLoader` 适配开销。
fn bench_router_segment(c: &mut Criterion) {
    let rt = bench_runtime();
    let mut config = Config::default();
    config.algorithm.default = "segment".to_string();
    // 基准进程不落盘 Snowflake 守卫状态。
    config.algorithm.snowflake.state_guard = false;

    let router = AlgorithmRouter::new(config, None)
        .with_segment_repository(Arc::new(MonotonicRepo::new(1_000_000)));
    rt.block_on(router.initialize())
        .expect("router initialize must succeed");
    let ctx = bench_ctx();

    c.bench_function("segment_router/generate_via_router", |b| {
        b.iter(|| {
            let id = rt.block_on(router.generate(black_box(&ctx))).unwrap();
            black_box(id)
        })
    });
}

criterion_group!(
    benches,
    bench_segment_generate,
    bench_segment_batch_generate,
    bench_segment_boundary,
    bench_router_segment,
);
criterion_main!(benches);
