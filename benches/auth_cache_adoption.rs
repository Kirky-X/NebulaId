// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! 认证缓存收编性能门禁基准。
//!
//! 对比认证命中路径（键构造 + 命中读取）在两种实现下的 P99 延迟与吞吐：
//! 基线 `AuthCache`（类型化直存 + `Arc` 条目，命中即克隆，无序列化往返）
//! 与候选 oxcache `Cache<String, CachedIdentity>`（经 `get_or` 读取，TTL 由
//! `set_with_ttl` 施加，命中含一次 JSON 反序列化）。
//!
//! 门禁阈值（proposal D3 / R-auth-002）：P99 增幅 ≤ 10%、吞吐降幅 ≤ 5%，
//! 判定在 `gate` 函数，默认输出数据与判定，设 `NEBULAID_AUTH_CACHE_GATE=1`
//! 时否决即 panic 供 CI/复核复跑。
//!
//! 运行：`cargo bench --bench auth_cache_adoption`

// `t!` 经 #[macro_use] 拉入文本作用域（同 src/main.rs），取默认 locale 翻译。
#[macro_use]
extern crate nebulaid;

use std::hint::black_box;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use nebulaid::server::auth::{AuthCache, CachedIdentity};
use oxcache::Cache;

/// 命中路径 key 基数：摊掉 `HashMap` 冷启动，又不让工作集超出 L2。
const KEY_COUNT: usize = 1_024;
/// 每实现采样次数（单线程逐次计时，支撑 P99 分位数）。
const SAMPLES: usize = 40_000;
/// 吞吐批次：每批连续命中数，批次计时摊掉单次 `Instant::now` 开销。
const BATCH: usize = 2_000;
const BATCHES: usize = 40;
/// 预热次数：不计入统计，令 `HashMap`/后端与分支预测进入稳态。
const WARMUP: usize = 20_000;

/// P99 延迟增幅上限（proposal D3）。
const P99_MAX_INCREASE_RATIO: f64 = 0.10;
/// 吞吐降幅上限（proposal D3）。
const THROUGHPUT_MAX_DECREASE_RATIO: f64 = 0.05;

/// 轮转计数器：两侧实现走完全相同的 key 序列，消除 key 选择偏差。
static NEXT_KEY: AtomicUsize = AtomicUsize::new(0);

fn identity_for(i: usize) -> CachedIdentity {
    CachedIdentity {
        // 稳定 uuid：同序号恒等，避免不同实现看到不同条目
        workspace_id: Some(uuid::Uuid::from_u128(i as u128)),
        role: nebulaid::core::database::ApiKeyRole::User,
        key_expires_at: Some(4_102_444_800), // 2100-01-01，绝不触发过期复验
    }
}

fn key_pair(i: usize) -> (String, String) {
    (format!("key-{i:06}"), format!("secret-{i:06}"))
}

/// 单次计时的延迟样本（纳秒）：整段循环跑在同一 `block_on` 内，逐次只付
/// 一次 `Instant::now`——否则两侧被加同一常量，稀释真实百分比差。
async fn sample_p99<F, Fut>(mut f: F) -> (Duration, Duration, Duration)
where
    F: FnMut(usize) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    for _ in 0..WARMUP {
        f(NEXT_KEY.fetch_add(1, Ordering::Relaxed) % KEY_COUNT).await;
    }
    let mut v = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let i = NEXT_KEY.fetch_add(1, Ordering::Relaxed) % KEY_COUNT;
        let t = Instant::now();
        f(i).await;
        v.push(t.elapsed());
    }
    v.sort_unstable();
    let p50 = v[v.len() / 2];
    let p99 = v[(v.len() as f64 * 0.99) as usize];
    let sum: u64 = v.iter().map(|d| d.as_nanos() as u64).sum();
    (p50, p99, Duration::from_nanos(sum))
}

/// 吞吐测量：连续 BATCH 次命中只取两次时间戳。
async fn measure_throughput<F, Fut>(mut f: F) -> f64
where
    F: FnMut(usize) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    for _ in 0..(WARMUP / BATCH).max(1) {
        for _ in 0..BATCH {
            f(NEXT_KEY.fetch_add(1, Ordering::Relaxed) % KEY_COUNT).await;
        }
    }
    let mut best = f64::MAX;
    for _ in 0..BATCHES {
        let t = Instant::now();
        for _ in 0..BATCH {
            f(NEXT_KEY.fetch_add(1, Ordering::Relaxed) % KEY_COUNT).await;
        }
        let elapsed = t.elapsed().as_secs_f64();
        best = best.min(BATCH as f64 / elapsed);
    }
    best
}

fn pct(new: f64, old: f64) -> f64 {
    if old == 0.0 {
        return 0.0;
    }
    (new - old) / old * 100.0
}

/// 门禁判定：默认输出数据与判定，`NEBULAID_AUTH_CACHE_GATE=1` 时否决即
/// panic。返回 (p99 增幅 %, 吞吐降幅 %)。
fn gate(p99_base: Duration, p99_cand: Duration, tput_base: f64, tput_cand: f64) -> (f64, f64) {
    let p99_increase = pct(p99_cand.as_secs_f64(), p99_base.as_secs_f64());
    let tput_decrease = -pct(tput_cand, tput_base);

    println!("{}", t!("log.bench.auth_cache_adoption_gate_header"));
    println!(
        "{}",
        t!(
            "log.bench.auth_cache_adoption_gate_baseline",
            p99 = format!("{:>10.2?}", p99_base),
            tput = format!("{:>12.0}", tput_base),
        )
    );
    println!(
        "{}",
        t!(
            "log.bench.auth_cache_adoption_gate_candidate",
            p99 = format!("{:>10.2?}", p99_cand),
            tput = format!("{:>12.0}", tput_cand),
        )
    );
    println!(
        "{}",
        t!(
            "log.bench.auth_cache_adoption_gate_p99_increase",
            pct = format!("{p99_increase:+.2}"),
            limit = format!("{:.0}", P99_MAX_INCREASE_RATIO * 100.0),
        )
    );
    println!(
        "{}",
        t!(
            "log.bench.auth_cache_adoption_gate_throughput_decrease",
            pct = format!("{tput_decrease:+.2}"),
            limit = format!("{:.0}", THROUGHPUT_MAX_DECREASE_RATIO * 100.0),
        )
    );

    // 门禁 opt-in（NEBULAID_AUTH_CACHE_GATE=1）：默认只输出数据与判定，不因
    // 一次否决炸掉 `cargo bench`；设了则按 proposal D3「判定标准代码化」硬失败。
    let enforce = std::env::var_os("NEBULAID_AUTH_CACHE_GATE").is_some();
    let pass = p99_increase <= P99_MAX_INCREASE_RATIO * 100.0
        && tput_decrease <= THROUGHPUT_MAX_DECREASE_RATIO * 100.0;
    if pass {
        println!("{}", t!("log.bench.auth_cache_adoption_gate_pass"));
    } else if enforce {
        panic!(
            "{}",
            t!(
                "log.bench.auth_cache_adoption_gate_reject_enforced",
                p99 = format!("{p99_increase:.2}"),
                tput = format!("{tput_decrease:.2}"),
            )
        );
    } else {
        println!(
            "{}",
            t!("log.bench.auth_cache_adoption_gate_reject_recorded")
        );
        println!("{}", t!("log.bench.auth_cache_adoption_gate_rerun_hint"));
    }
    println!("===================================================\n");
    (p99_increase, tput_decrease)
}

fn main() {
    let baseline = std::thread::spawn(|| {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        rt.block_on(async {
            let cache = AuthCache::new(3_600).await;
            for i in 0..KEY_COUNT {
                let (kid, sec) = key_pair(i);
                cache.put(&kid, &sec, &identity_for(i)).await;
            }
            let keys = (0..KEY_COUNT).map(key_pair).collect::<Vec<_>>();
            let (p50, p99, _) = sample_p99(|i| {
                let (cache, keys) = (&cache, &keys);
                async move {
                    black_box(cache.get(&keys[i].0, &keys[i].1).await);
                }
            })
            .await;
            let tput = measure_throughput(|i| {
                let (cache, keys) = (&cache, &keys);
                async move {
                    black_box(cache.get(&keys[i].0, &keys[i].1).await);
                }
            })
            .await;
            (p50, p99, tput)
        })
    })
    .join()
    .expect("baseline panicked");
    let (p50_b, p99_b, tput_b) = baseline;

    let candidate = std::thread::spawn(|| {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        rt.block_on(async {
            let cache: Cache<String, CachedIdentity> = Cache::builder()
                .capacity(KEY_COUNT as u64)
                .build()
                .await
                .expect("oxcache build");
            for i in 0..KEY_COUNT {
                let (kid, sec) = key_pair(i);
                let key = format!("cb::{kid}:{sec}");
                cache
                    .set_with_ttl(&key, &identity_for(i), Some(Duration::from_secs(3_600)))
                    .await
                    .expect("oxcache warm");
            }
            let keys = (0..KEY_COUNT).map(key_pair).collect::<Vec<_>>();
            let (p50, p99, _) = sample_p99(|i| {
                let (cache, keys) = (&cache, &keys);
                async move {
                    let key = format!("cb::{}:{}", keys[i].0, keys[i].1);
                    let out = cache
                        .get_or(&key, || async {
                            unreachable!("{}", t!("log.bench.auth_cache_adoption_hit_path_only"))
                        })
                        .await;
                    let _ = black_box(out);
                }
            })
            .await;
            let tput = measure_throughput(|i| {
                let (cache, keys) = (&cache, &keys);
                async move {
                    let key = format!("cb::{}:{}", keys[i].0, keys[i].1);
                    let out = cache
                        .get_or(&key, || async {
                            unreachable!("{}", t!("log.bench.auth_cache_adoption_hit_path_only"))
                        })
                        .await;
                    let _ = black_box(out);
                }
            })
            .await;

            // 公平性复核：`get_or` 的账本（single-flight）非命中读取必需成本，
            // `get` 是纯命中路径的诚实对照，门禁以 `get` 判定；若仅 `get_or`
            // 不达标，结论须为「可吸收但须改用 `get`」而非直接否决。
            let (p50_g, p99_g, _) = sample_p99(|i| {
                let (cache, keys) = (&cache, &keys);
                async move {
                    let key = format!("cb::{}:{}", keys[i].0, keys[i].1);
                    let _ = black_box(cache.get(&key).await);
                }
            })
            .await;
            let tput_g = measure_throughput(|i| {
                let (cache, keys) = (&cache, &keys);
                async move {
                    let key = format!("cb::{}:{}", keys[i].0, keys[i].1);
                    let _ = black_box(cache.get(&key).await);
                }
            })
            .await;
            (p50, p99, tput, p50_g, p99_g, tput_g)
        })
    })
    .join()
    .expect("candidate panicked");
    let (p50_c, p99_c, tput_c, p50_g, p99_g, tput_g) = candidate;

    println!(
        "{}",
        t!(
            "log.bench.auth_cache_adoption_diag_p50",
            baseline = format!("{p50_b:?}"),
            get_or = format!("{p50_c:?}"),
            get = format!("{p50_g:?}"),
        )
    );

    println!("{}", t!("log.bench.auth_cache_adoption_fairness_header"));
    println!(
        "{}",
        t!(
            "log.bench.auth_cache_adoption_fairness_baseline",
            p99 = format!("{:>10.2?}", p99_b),
            tput = format!("{:>12.0}", tput_b),
        )
    );
    println!(
        "{}",
        t!(
            "log.bench.auth_cache_adoption_fairness_candidate",
            p99 = format!("{:>10.2?}", p99_g),
            tput = format!("{:>12.0}", tput_g),
        )
    );
    println!(
        "{}",
        t!(
            "log.bench.auth_cache_adoption_fairness_p99_increase",
            pct = format!("{:+.2}", pct(p99_g.as_secs_f64(), p99_b.as_secs_f64())),
        )
    );
    println!(
        "{}",
        t!(
            "log.bench.auth_cache_adoption_fairness_throughput_decrease",
            pct = format!("{:+.2}", -pct(tput_g, tput_b)),
        )
    );

    println!("{}", t!("log.bench.auth_cache_adoption_designated_header"));
    gate(p99_b, p99_c, tput_b, tput_c);

    println!("{}", t!("log.bench.auth_cache_adoption_verdict_header"));
    let (p99_inc, tput_dec) = gate(p99_b, p99_g, tput_b, tput_g);

    // 结果以机器可读形式落盘，供变更记录与 PR 引用。写盘放在 gate 之后：
    // enforce 模式下否决会 panic，写在其前则两分支数据不一致。
    let pass = p99_inc <= 10.0 && tput_dec <= 5.0;
    let report = format!(
        "{{\n  \"bench\": \"auth_cache_adoption\",\n  \"samples\": {SAMPLES},\n           \"key_count\": {KEY_COUNT},\n  \"p50_baseline_ns\": {},\n           \"p99_baseline_ns\": {},\n  \"p99_candidate_get_ns\": {},\n           \"p99_candidate_get_or_ns\": {},\n  \"throughput_baseline_ops\": {:.0},\n           \"throughput_candidate_get_ops\": {:.0},\n  \"throughput_candidate_get_or_ops\": {:.0},\n           \"p99_increase_pct\": {p99_inc:.4},\n  \"throughput_decrease_pct\": {tput_dec:.4},\n           \"thresholds\": {{\"p99_max_increase_pct\": 10.0, \"throughput_max_decrease_pct\": 5.0}},\n           \"verdict\": \"{}\"\n}}\n",
        p50_b.as_nanos(),
        p99_b.as_nanos(),
        p99_g.as_nanos(),
        p99_c.as_nanos(),
        tput_b,
        tput_g,
        tput_c,
        if pass { "adopt" } else { "reject" },
    );
    let path = "target/auth_cache_adoption-latest.json";
    if let Err(e) = std::fs::write(path, &report) {
        eprintln!(
            "{}",
            t!(
                "log.bench.auth_cache_adoption_report_write_failed",
                path = path,
                error = e,
            )
        );
    } else {
        println!(
            "{}",
            t!("log.bench.auth_cache_adoption_report_written", path = path)
        );
        if pass {
            println!("{}", t!("log.bench.auth_cache_adoption_verdict_adopt"));
        } else {
            println!("{}", t!("log.bench.auth_cache_adoption_verdict_reject"));
        }
    }
}
