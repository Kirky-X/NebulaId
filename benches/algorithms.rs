// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Criterion 微基准：核心 ID 生成 / 限流 / 认证缓存热路径（Lane W1 基线）。
//!
//! 运行：`cargo bench --bench algorithms`
//!
//! 覆盖：Snowflake 单条与批量（100、10000）、`AlgorithmRouter` 路由链路
//! （默认 snowflake、fallback [UuidV8]）、`RateLimiter::check_rate_limit`、
//! `AuthCache::get` 命中路径。
//!
//! Segment 路由未纳入：`SegmentAlgorithm` / `SegmentLoader` 均为 `pub(crate)`，
//! 外部 bench 无法注入内存 stub，待接入 `DbSegmentLoader` 后再评估。
use std::hint::black_box;

use criterion::{criterion_group, criterion_main, Criterion};
use nebulaid::core::algorithm::{
    AlgorithmBuilder, AlgorithmFactory, AlgorithmRouter, GenerateContext, SnowflakeFactory,
};
use nebulaid::core::config::Config;
use nebulaid::core::database::ApiKeyRole;
use nebulaid::core::types::AlgorithmType;
use nebulaid::server::auth::{AuthCache, CachedIdentity};
use nebulaid::server::rate_limit::RateLimiter;

/// 需 multi-thread runtime：oxcache `_sync` 后端与路由 `initialize` 的后台健康检查均要求。
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
        format: nebulaid::core::types::IdFormat::Numeric,
        prefix: None,
    }
}

/// Snowflake 单条生成。`SnowflakeAlgorithm` 为 `pub(crate)`，经公开的
/// `SnowflakeFactory` + `AlgorithmBuilder` 构建，即生产路径的实例形态。
fn bench_snowflake_generate(c: &mut Criterion) {
    let rt = bench_runtime();
    let algo = rt.block_on(async {
        let builder = AlgorithmBuilder::new(AlgorithmType::Snowflake);
        SnowflakeFactory
            .build(&builder, &Config::default())
            .await
            .expect("snowflake build must succeed")
    });
    let ctx = bench_ctx();

    c.bench_function("snowflake/generate", |b| {
        b.iter(|| {
            let id = rt.block_on(algo.generate(black_box(&ctx))).unwrap();
            black_box(id)
        })
    });
}

/// Snowflake 批量生成（单批 100 个 ID）。
fn bench_snowflake_batch_generate(c: &mut Criterion) {
    let rt = bench_runtime();
    let algo = rt.block_on(async {
        let builder = AlgorithmBuilder::new(AlgorithmType::Snowflake);
        SnowflakeFactory
            .build(&builder, &Config::default())
            .await
            .expect("snowflake build must succeed")
    });
    let ctx = bench_ctx();

    c.bench_function("snowflake/batch_generate_100", |b| {
        b.iter(|| {
            let batch = rt
                .block_on(algo.batch_generate(black_box(&ctx), 100))
                .unwrap();
            black_box(batch)
        })
    });
}

/// Snowflake 超大批量（单批 10000 = `[batch_generate].max_batch_size` 上限）：
/// 单次 block_on 摊薄到 10k 个 ID 上，近似「纯 reserve 路径」的下限口径。
fn bench_snowflake_batch_generate_10000(c: &mut Criterion) {
    let rt = bench_runtime();
    let algo = rt.block_on(async {
        let builder = AlgorithmBuilder::new(AlgorithmType::Snowflake);
        SnowflakeFactory
            .build(&builder, &Config::default())
            .await
            .expect("snowflake build must succeed")
    });
    let ctx = bench_ctx();

    c.bench_function("snowflake/batch_generate_10000", |b| {
        b.iter(|| {
            let batch = rt
                .block_on(algo.batch_generate(black_box(&ctx), 10_000))
                .unwrap();
            black_box(batch)
        })
    });
}

/// AlgorithmRouter 路由链路：默认算法 snowflake（fallback [UuidV8]），
/// 覆盖 get_algorithm 查表 + 观测面记录 + 算法分发的完整开销。
///
/// 默认算法设为 snowflake：`SegmentLoader` 无法从外部 crate 注入 stub（见
/// 模块文档），避免 Segment 构建态影响测量语义。
fn bench_router_generate(c: &mut Criterion) {
    let rt = bench_runtime();
    let mut config = Config::default();
    config.algorithm.default = "snowflake".to_string();
    let router = AlgorithmRouter::new(config, None);
    rt.block_on(router.initialize())
        .expect("router initialize must succeed");
    let ctx = bench_ctx();

    c.bench_function("router/generate_snowflake_primary", |b| {
        b.iter(|| {
            let id = rt.block_on(router.generate(black_box(&ctx))).unwrap();
            black_box(id)
        })
    });
}

/// AlgorithmRouter 指定算法直选（UUID v8）：`generate_with_algorithm` 的
/// per-call ctx 构造 + 路由分发开销。
fn bench_router_generate_uuid(c: &mut Criterion) {
    let rt = bench_runtime();
    let mut config = Config::default();
    config.algorithm.default = "snowflake".to_string();
    let router = AlgorithmRouter::new(config, None);
    rt.block_on(router.initialize())
        .expect("router initialize must succeed");

    c.bench_function("router/generate_with_algorithm_uuid_v8", |b| {
        b.iter(|| {
            let id = rt
                .block_on(router.generate_with_algorithm(
                    AlgorithmType::UuidV8,
                    black_box("bench"),
                    black_box("bench"),
                    black_box("bench"),
                ))
                .unwrap();
            black_box(id)
        })
    });
}

/// RateLimiter 令牌桶命中路径：rate/capacity 拉高以避开拒绝分支，稳定测量「查桶 + 扣 token + 快照读取」。
fn bench_rate_limiter(c: &mut Criterion) {
    let rt = bench_runtime();
    let limiter = RateLimiter::new(10_000_000, 1_000_000);

    c.bench_function("rate_limiter/check_rate_limit", |b| {
        b.iter(|| {
            let result = rt.block_on(limiter.check_rate_limit(
                black_box("bench-key"),
                black_box(None),
                black_box(None),
            ));
            black_box(result)
        })
    });
}

/// AuthCache 命中路径：put 一次后反复 get（sha256 键派生 + oxcache L1 查找 + JSON 反序列化 + 过期复核）。
fn bench_auth_cache_hit(c: &mut Criterion) {
    let rt = bench_runtime();
    let cache = rt.block_on(AuthCache::new(300));
    let identity = CachedIdentity {
        workspace_id: Some(uuid::Uuid::new_v4()),
        role: ApiKeyRole::User,
        key_expires_at: None,
    };
    rt.block_on(cache.put("bench-key", "bench-secret", &identity));

    c.bench_function("auth_cache/get_hit", |b| {
        b.iter(|| {
            let hit = rt.block_on(cache.get(black_box("bench-key"), black_box("bench-secret")));
            black_box(hit)
        })
    });
}

criterion_group!(
    benches,
    bench_snowflake_generate,
    bench_snowflake_batch_generate,
    bench_snowflake_batch_generate_10000,
    bench_router_generate,
    bench_router_generate_uuid,
    bench_rate_limiter,
    bench_auth_cache_hit,
);
criterion_main!(benches);
