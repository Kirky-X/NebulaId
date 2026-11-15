# ⚡ Nebula ID Performance Guide

> This document describes Nebula ID's benchmark methodology, hot-path design highlights, and performance optimization advice. Everything here is grounded in benchmarks and code that actually exist in the repository; the document does not publish throughput numbers that were not produced by the benchmark harness.

[🏠 Home](../../README.md) • [🏗️ Architecture](ARCHITECTURE.md) • [🧪 Test Scenario Matrix](TEST_SCENARIOS.md)

---

## 📋 Table of Contents

- [Benchmark Methodology and Honesty Statement](#benchmark-methodology-and-honesty-statement)
- [Benchmark Suite (i18n Hot Path)](#benchmark-suite-i18n-hot-path)
- [Benchmark Suite (ID-Generation Hot Path)](#benchmark-suite-id-generation-hot-path)
- [How to Run](#how-to-run)
- [Hot-Path Design Highlights](#hot-path-design-highlights)
- [Build and Runtime Performance Configuration](#build-and-runtime-performance-configuration)
- [Baseline Recording Conventions](#baseline-recording-conventions)
- [Recorded Baselines](#recorded-baselines)
- [Optimization Tips](#optimization-tips)
- [Related Documents](#related-documents)

---

## Benchmark Methodology and Honesty Statement

This repository's `Cargo.toml` declares **four** benchmarks (all `harness = false`; one of them is a one-off performance gate, not a Criterion benchmark):
- `[[bench]] name = "i18n"` (`benches/i18n.rs`): i18n hot path, pinning the baseline for performance fixes so future i18n changes are quantifiable;
- `[[bench]] name = "algorithms"` (`benches/algorithms.rs`): ID-generation / rate-limit / auth-cache hot paths, covering Snowflake single and batch generation, the routing chain, the token bucket, and auth-cache hits.
- `[[bench]] name = "segment"` (`benches/segment.rs`, `required-features = ["segment"]`): Segment steady-state single / batch amortized / segment-boundary crossing / routing chain.

- `[[bench]] name = "auth_cache_adoption"` (`benches/auth_cache_adoption.rs`, `required-features = ["server", "garrison-auth"]`) — the **auth-cache adoption performance gate** of selfdev-lib-adoption-r1 item 7. It compares the baseline `AuthCache` (typed direct storage) against the candidate oxcache `Cache` on the auth-hit path (key construction + hit read) in P99 and throughput, with thresholds of P99 growth ≤ 10% and throughput loss ≤ 5%. Verdict: reject (2026-10-19 rerun: `get` path P99 362ns→2.72µs (+652.2%), throughput 3.284M→1.433M ops/s (−56.4%); `get_or` path P99 +646.4%, throughput −57.4%. The +597% / +690% recorded in rc.3 were same-magnitude results from an earlier batch; `target/auth_cache_adoption-latest.json` is authoritative. The root cause is oxcache's `V: Serialize + DeserializeOwned`, forcing a JSON deserialization on every hit); setting `NEBULAID_AUTH_CACHE_GATE=1` makes the run hard-fail when the gate rejects, and the machine-readable result is written to `target/auth_cache_adoption-latest.json`.
**Methodology boundary: the first three are algorithm / middleware-layer microbenchmarks and the fourth is a one-off adoption gate; none covers the full HTTP chain (auth middleware, TLS, serialization, network), and the baseline numbers are not SLA commitments.** The Segment benchmark draws segments from an in-process stub with **no real database round trip**; measure end-to-end throughput against your own workload (see [Optimization Tips](#optimization-tips)).

---

## Benchmark Suite (i18n Hot Path)

`benches/i18n.rs` defines 4 benchmark functions with 12 benchmark cases in total:

| Benchmark function | Cases | Covered path |
|--------------------|-------|--------------|
| `bench_translate_with_locale` | `translate_with_locale/en/no_args`, `/zh-CN/no_args`, `/fr_fallback/no_args` | Argument-less table lookup (including the unsupported-locale fallback path) |
| `bench_translate_with_locale_args` | `translate_with_locale_args/en/1_arg`, `/zh-CN/1_arg` | Argumented translation (`error.invalid_id_format`) |
| `bench_to_localized_string` | `to_localized_string/InvalidIdFormat/{en,zh-CN}`, `/RateLimitExceeded/en`, `/ClockMovedBackward/en` | `CoreError::to_localized_string` (`i18n_key()` + `i18n_args()` + the full argumented-translation chain) |
| `bench_parse_accept_language` | `parse_accept_language/typical_2`, `/longer_5`, `/unsupported_only` | Accept-Language negotiation in `negotiate_locale_str` (2 candidates / 5 candidates with q-values / unsupported locales only) |

Notes:

- The benchmarks deliberately avoid `set_locale` (global state) and only measure the concurrency-safe per-call `translate_with_locale*` APIs.
- criterion 0.8 deprecated its own `black_box`; the benchmarks use the stable standard-library implementation (`std::hint::black_box`) directly.

---

## Benchmark Suite (ID-Generation Hot Path)

`benches/algorithms.rs` defines 8 benchmark functions with 10 benchmark cases in total (the batch-capacity function registers 3 `sequence_bits` variants):

| Benchmark function | Cases | Covered path |
|--------------------|-------|--------------|
| `bench_snowflake_generate` | `snowflake/generate` | Snowflake single generation — via a `Box<dyn IdAlgorithm>` built from the public `SnowflakeFactory`, the exact path used by production `AlgorithmBuilder::build` |
| `bench_snowflake_batch_generate` | `snowflake/batch_generate_100` | Batch generation (100 IDs per batch) |
| `bench_snowflake_batch_generate_10000` | `snowflake/batch_generate_10000` | Extra-large batch (10000 per batch = the config-validation cap; the repo default `max_batch_size` is 100), one `block_on` amortized over 10k IDs — approximating the "pure reserve path" lower bound |
| `bench_snowflake_batch_capacity` | `snowflake/batch_generate_10000_seq_bits_{10,12,14}` | Bit-width capacity sweep: `2^bits - 1` is the per-millisecond cap; the default 10 bits is unchanged, data for capacity planning only |
| `bench_router_generate` | `router/generate_snowflake_primary` | Routing chain: default algorithm snowflake + fallback chain `[UuidV8]`, including table lookup, observability recording, and dispatch overhead |
| `bench_router_generate_uuid` | `router/generate_with_algorithm_uuid_v8` | Routing straight to UUID v8 |
| `bench_rate_limiter` | `rate_limiter/check_rate_limit` | Token-bucket hit path |
| `bench_auth_cache_hit` | `auth_cache/get_hit` | Auth-cache hit (sha256 key + oxcache L1 hit + deserialization) |

Notes:

- Every iteration drives the async entry point via `rt.block_on` (matching the production call shape), so numbers include per-iteration async dispatch overhead; exceeding the pure CAS cost is expected.
- The Segment hot path lives in `benches/segment.rs`: `SegmentAlgorithm` / `SegmentLoader` / `SegmentData` are all public, and both the stub loader and `with_segment_repository` are injectable; the pure-algorithm methodology contains no database round trip.

### Benchmark Suite (Segment Hot Path)

`benches/segment.rs` defines 4 benchmark functions with 7 benchmark cases in total:

| Benchmark function | Cases | Covered path |
|--------------------|-------|--------------|
| `bench_segment_generate` | `segment/generate_steady` | Steady-state single: key construction + buffer read lock + in-segment consumption + metrics counting (span=1e6, no boundary crossing) |
| `bench_segment_batch_generate` | `segment/batch_generate_{10,100,1000,10000}` | Batch amortization curve (span=1e6) |
| `bench_segment_boundary` | `segment/generate_boundary_span_1024` | Segment-boundary crossing: loader invocation and double-buffer switch every 1024 ids |
| `bench_router_segment` | `segment_router/generate_via_router` | Routing chain: `with_segment_repository` + `DbSegmentLoader` adapter (production assembly shape) |

Note: segments are generated monotonically by the in-process `MonotonicLoader` / `MonotonicRepo`, with **no real DB round trip**; the `segment_router` case includes routing table lookup, degradation observability, and adapter overhead.

---

## How to Run

```bash
# Run the i18n benchmark (results are written to target/criterion/; view them with the criterion summary report)
cargo bench --bench i18n

# Run the ID-generation / rate-limit / auth-cache benchmarks
cargo bench --bench algorithms

# Run the Segment steady-state / batch / boundary / routing benchmarks
cargo bench --bench segment
```

`[profile.bench]` uses `opt-level = 3` and `lto = true`, close to release, and can be treated as the reference compilation methodology for the ID-generation hot path.

---

## Hot-Path Design Highlights

The following highlights come from the code and change records — actual designs that affect minting throughput/latency:

| Design | Location | Notes |
|--------|----------|-------|
| Segment double buffering + dynamic step | `src/core/algorithm/segment.rs` | Segment pre-fetching and dynamic adjustment within `[base_step, min_step, max_step]` (triggered by `switch_threshold`), amortizing database round trips to segment boundaries |
| Snowflake lock-free CAS state word | `src/core/algorithm/snowflake.rs` | Generation state compressed into a single atomic word `(timestamp << seq_bits) \| sequence`; batch generation reserves a range with one CAS, replacing the historical `gen_lock` global mutex (for the fix history of the concurrent-duplicate-ID race, see [CHANGELOG](CHANGELOG.md) Fixed) |
| Snowflake hybrid waiting (spin-then-sleep) | `src/core/algorithm/snowflake.rs` | Millisecond-rollover waiting spins first (≤2ms budget) then falls back to a 1ms sleep: spinning tightens sub-millisecond rollovers to the true millisecond boundary, eliminating timer-granularity overshoot; sleep happens only after the budget is exhausted (clock-rollback waits), avoiding long idle waits. Sustained throughput is about 2.1×, hitting the 1023 ID/ms bit-width ceiling in practice (see [Recorded Baselines](#recorded-baselines)) |
| Batch generation | `IdAlgorithm::batch_generate` | Request N ids in one call (capped by `[batch_generate].max_batch_size`, 1..=10000), amortizing per-request auth/rate-limit/routing overhead |
| Degradation chain | `src/core/algorithm/degradation_manager.rs` | Falls back along `[Snowflake, UuidV8]` when the primary algorithm is unavailable, preventing a segment failure from amplifying into a total failure |
| Ring-buffer percentiles | `src/core/types/metrics.rs` | p50/p99/p999 come from one sort over the most recent 1024 samples in a ring buffer, replacing the "historical maximum" methodology, with bounded metrics overhead |
| Pure algorithms, zero DB | `src/core/algorithm/{snowflake,uuid_v8}.rs` | snowflake / uuid_v8 need no database; embedded scenarios can mint with `Config::default()` |
| Auth cache | `[auth].cache_ttl_seconds` | The garrison ApiKeyHandler checks the in-process cache first and falls back to the DB on miss; with `cache_ttl_seconds = 0` no cache instance is assembled |
| Lock-free / fine-grained concurrency primitives | `arc-swap`, `parking_lot` | Config hot-update snapshots and internal locks use low-overhead primitives |

---

## Build and Runtime Performance Configuration

Release methodology in `Cargo.toml`:

```toml
[profile.release]
opt-level = 3
lto = "thin"          # thin LTO
codegen-units = 4
strip = true
panic = "abort"       # server-style binary: smaller size, no unwinding overhead

[profile.bench]
opt-level = 3
lto = true
```

Runtime performance-relevant config items (full table in the [Config Migration Guide · config reference](CONFIG_MIGRATION_GUIDE.md#config-reference-all-options-and-validation-rules)):

- `[database].max_connections` / `min_connections`: pool size; Segment draws segments from the database, and an undersized pool queues at segment boundaries.
- `[rate_limit].default_rps` / `burst_size`: token-bucket capacity, `burst_size ≤ 10 × default_rps`.
- `[auth].cache_ttl_seconds`: auth-cache TTL — the trade-off between per-request auth cost and revocation convergence speed.
- `[batch_generate].max_batch_size`: batch minting cap (1..=10000).

---

## Baseline Recording Conventions

1. After adding or changing a benchmark, collect criterion medians on the same development machine under the same compilation methodology.
2. When recording into this file, note the date, commit, machine spec, and criterion version.
3. Throughput / latency numbers may **only** appear in the "Recorded Baselines" section below and must be criterion-produced medians (converted values must be labeled); no other section may cite numbers that are not on record.

---

## Recorded Baselines

### ID-Generation Hot Path (`benches/algorithms.rs`)

| Item | Value |
|------|-------|
| Date | 2026-09-18 |
| commit | `e6acb2c` (e6acb2c771ab43cb6960087bd49bbe319268f27c) |
| Machine | AMD Ryzen 9 9950X (16C32T); WSL2 (kernel 6.6.87.2-microsoft-standard-WSL2) with 12 logical CPUs, 70.7 GiB RAM |
| Toolchain | rustc / cargo 1.97.1 |
| criterion | 0.8.2, default methodology (100 samples / 3s warmup / 5s measurement) |
| Compilation | `[profile.bench]` (opt-level = 3, lto = true), default features |
| Environment | Development machine nearly idle (load average ≈ 1, resident services only, no parallel compilation); WSL2 has no CPU frequency governor control |

Medians are the criterion median point estimates from `target/criterion/**/new/estimates.json`; the "converted throughput" column is a reference value derived from the medians (not directly produced by criterion):

| Benchmark case | Median | Converted throughput |
|----------------|-------:|---------------------:|
| `snowflake/generate` | 2.083 µs | ≈ 480K ops/s |
| `snowflake/batch_generate_100` | 209.7 µs | ≈ 477K ids/s (≈ 2.10 µs/id) |
| `router/generate_snowflake_primary` | 2.151 µs | ≈ 465K ops/s |
| `router/generate_with_algorithm_uuid_v8` | 419.3 ns | ≈ 2.38M ops/s |
| `rate_limiter/check_rate_limit` | 198.6 ns | ≈ 5.03M ops/s |
| `auth_cache/get_hit` | 313.2 ns | ≈ 3.19M ops/s |

Interpretation:

- The theoretical ceiling of a single Snowflake instance is 1023 ids per millisecond (default 10-bit sequence, ≈ 1.02M ids/s). The table shows single-caller serial numbers including per-iteration `block_on` dispatch overhead, so falling short of the theoretical ceiling is expected; batching + concurrent multi-callers can approach the ceiling.
- The routing chain (2.151 µs) is about 3% above the direct algorithm call (2.083 µs) — routing table lookup and observability are not bottlenecks.
- Both rate-limit evaluation and auth-cache hits are in the hundreds of nanoseconds; fixed per-request overhead is controlled.

### ID-Generation Hot Path (`benches/algorithms.rs`, 2026-09-18, after the hybrid-wait optimization)

Environment and methodology identical to the first minting baseline (same day, same machine, same commit `e6acb2c` workspace, criterion defaults); the difference is the spin-then-sleep hybrid wait introduced in `wait_for_next_ms` (see [Hot-Path Design Highlights](#hot-path-design-highlights)):

| Benchmark case | Median | Converted throughput | vs. before optimization |
|----------------|-------:|---------------------:|------------------------:|
| `snowflake/generate` | 977.5 ns | ≈ 1.023M ops/s | **2.13×** |
| `snowflake/batch_generate_100` | 97.76 µs | ≈ 1.023M ids/s | **2.14×** |
| `snowflake/batch_generate_10000` | 9.778 ms | ≈ 1.023M ids/s | **2.12×** (pre-optimization snapshot 20.72 ms) |
| `router/generate_snowflake_primary` | 986.8 ns | ≈ 1.013M ops/s | **2.18×** |
| `router/generate_with_algorithm_uuid_v8` | 382.1 ns | ≈ 2.61M ops/s | ≈ flat |
| `rate_limiter/check_rate_limit` | 183.2 ns | ≈ 5.46M ops/s | ≈ flat |
| `auth_cache/get_hit` | 277.7 ns | ≈ 3.60M ops/s | ≈ flat |

Interpretation: before the optimization, all three Snowflake cases were stuck on "millisecond-rollover 1ms sleep granularity overshoot" (measured ≈ 47% of the theoretical ceiling); after the hybrid wait tightened rollover waiting to the true millisecond boundary, serial / batch / extra-large batch all converged to the 1023 ID/ms bit-width ceiling (977.5 ns per id = 1ms ÷ 1023, i.e. the benchmark now measures the bit-width ceiling itself rather than waiting overhead). The UUID / rate-limit / auth-cache paths involve no millisecond rollover, so their numbers stayed flat vs. before (differences within noise).

### i18n Hot Path (`benches/i18n.rs`)

Measured since 0.3.0-rc.3 (data collected 2026-10-11, same commit `fbe5c32`+T010 gate remediation, WSL2 release) — the implementation switched from the dual-bundle FluentBundle to sdforge registration + OnceLock snapshot lock-free rendering (selfdev-lib-adoption-r1); the table below is the first-hand baseline after the switch. The pre-switch same-machine old values (the 41.0 / 54.1 / 89.1 ns tier) survive only in git history and no longer represent the current implementation:

| Benchmark case | Median |
|----------------|-------:|
| `translate_with_locale/en/no_args` | 52.1 ns |
| `translate_with_locale/zh-CN/no_args` | 51.0 ns |
| `translate_with_locale/fr_fallback/no_args` | 51.4 ns |
| `translate_with_locale_args/en/1_arg` | 127.6 ns |
| `translate_with_locale_args/zh-CN/1_arg` | 120.0 ns |
| `to_localized_string/InvalidIdFormat/en` | 99.6 ns |
| `to_localized_string/InvalidIdFormat/zh-CN` | 99.8 ns |
| `to_localized_string/RateLimitExceeded/en` | 76.1 ns |
| `to_localized_string/ClockMovedBackward/en` | 154.9 ns |
| `parse_accept_language/typical_2` | 74.2 ns |
| `parse_accept_language/longer_5` | 182.0 ns |
| `parse_accept_language/unsupported_only` | 64.8 ns |

### HTTP End-to-End (single instance, 2026-09-18)

Environment and methodology as in the table above (same day, same machine, same commit `e6acb2c` workspace); the service ran a `--release` build, PostgreSQL 16 (docker, local machine); reproduction configs are `config/bench_a.toml` (no rate limiting) and `config/bench_b.toml` (auth + cache + rate-limit evaluation on, bucket not throttling), load client oha 1.16, keep-alive, c=64.

| Scenario | Throughput | p50 | p99 |
|----------|-----------:|----:|----:|
| `GET /health` | 13,675 rps | 4.0 ms | 16.2 ms |
| `POST /api/v1/generate` (auth-cache hit + rate-limit evaluation + tenant validation + full minting chain) | 4,974 rps | 10.8 ms | 38.9 ms |
| `POST /api/v1/generate/batch` (100 ids per batch) | 2,842 rps ≈ **284K ids/s** | 21.9 ms | 46.7 ms |

Composition of fixed per-request overhead (microbenchmark methodology): auth-cache hit ~0.28 µs + rate-limit evaluation ~0.18 µs + routing dispatch ~0.99 µs, totaling ≈ 1.5 µs — the bulk of the millisecond-scale end-to-end latency is the HTTP stack and the per-request tenant workspace validation DB lookup (~0.3–3 ms, docker PG local RTT), not the minting path itself. Reference frame: Meituan Leaf's official figure is ~50K rps per 4C8G machine (TP999 1ms).

#### Resolved capacity-level blocker (historical incident record)

These numbers were once uncollectable after an incident: the inklog main async channel is `bounded(channel_capacity)` (default 10000), and when the file sink has no explicitly configured path the channel has no consumer; after the service emitted ~10k log lines the channel backlog filled, and from then on **every log line blocked for `send_timeout(100ms)` before degrading**. The audit `log_shared` also emitted its ring-full `warn!` across the mutex, so every request's audit write serialized at 100ms apiece — measured throughput collapsed to ~6 rps with ~11s fixed per request (c=64), while server CPU sat idle and the connection pool was entirely idle. "Performance cliff after ~10k requests" shared the same root as the "201ms/402ms per request" anomaly observed earlier that day (generate emits two audit events per request = 2×~200ms).

Fix (two layers, pinned by the regression test `tests/audit_ring_stall_repro.rs`):

1. `init_observability` explicitly configures the inklog file sink (`logs/nebula.log`), giving the async channel a consumer that drains continuously;
2. `log_shared` moves its drop warning outside the mutex — however slow the log backend gets, it no longer serializes requests.

Known residue (non-blocking, on record): inklog's default semantics of "unconfigured file_sink (None) counts as enabled" combined with the fact of "no path, no worker" is a hazard — an upstream design issue in `../base/inklog`; the recommendation is for upstream to make None mean explicit disable, or to use an immediately-disconnected dummy channel for the no-path case; currently mitigated by the host side's explicit configuration.

### Segment Hot Path (`benches/segment.rs`, 2026-10-19)

| Item | Value |
|------|-------|
| Date | 2026-10-19 |
| commit | Workspace (uncommitted, based on `442703a`) |
| Machine | AMD Ryzen 9 9950X (16C32T); WSL2 (kernel 6.6.87.2-microsoft-standard-WSL2) with 16 logical CPUs, 66 GiB RAM |
| Toolchain | rustc / cargo 1.97.1 |
| criterion | 0.8.2, default methodology (100 samples / 3s warmup / 5s measurement) |
| Compilation | `[profile.bench]` (opt-level = 3, lto = true), default features (including `segment`) |

Medians are the criterion median point estimates from `estimates.json`; "converted throughput" is a reference value derived from the medians:

| Benchmark case | Median | Converted throughput |
|----------------|-------:|---------------------:|
| `segment/generate_steady` | 138.33 ns | ≈ 7.23M ops/s |
| `segment/batch_generate_10` | 182.60 ns | ≈ 54.77M ids/s (≈ 18.3 ns/id) |
| `segment/batch_generate_100` | 223.62 ns | ≈ 447M ids/s (≈ 2.24 ns/id) |
| `segment/batch_generate_1000` | 395.90 ns | ≈ 2.53B ids/s (≈ 0.40 ns/id) |
| `segment/batch_generate_10000` | 2.089 µs | ≈ 4.79B ids/s (≈ 0.21 ns/id) |
| `segment/generate_boundary_span_1024` | 144.45 ns | ≈ 6.92M ops/s (loader amortized) |
| `segment_router/generate_via_router` | 419.48 ns | ≈ 2.38M ops/s |

Interpretation:

- Steady-state single Segment (138 ns) is markedly faster than Snowflake (977 ns): the latter is throttled by the `1023 ID/ms` bit width, while this table is pure in-memory segment consumption.
- Batch amortization is pronounced: at a 10000 batch, each id costs ≈ 0.21 ns, about 1/660 of single; throughput-oriented callers should prefer batching.
- Boundary crossing (span=1024) is only ~4% above steady state — loader calls amortize over 1024 ids; real DB round trips would significantly inflate this figure, so do not extrapolate from it.
- The routing chain (419 ns) is roughly 3× the direct algorithm call (138 ns); the increment comes from table lookup + degradation observability + the `DbSegmentLoader` adapter.

### Snowflake Batch Capacity vs `sequence_bits` (2026-10-19)

Same machine, same methodology (default features), 10000 per batch:

| `sequence_bits` | Median | Converted throughput | Bit-width cap per ms | Timestamp bits | Available from epoch (2024-01-01) |
|:---------------:|-------:|---------------------:|:--------------------:|:--------------:|:---------------------------------:|
| 10 (default) | 9.7747 ms | ≈ 1.023M ids/s | 1023 | 43 | ≈ 278 years |
| 12 | 2.4418 ms | ≈ 4.095M ids/s | 4095 | 41 | ≈ 70 years |
| 14 | 610.37 µs | ≈ 16.383M ids/s | 16383 | 39 | ≈ 17 years |

Interpretation: throughput scales strictly linearly with `2^bits` (4× / 16×), confirming the bottleneck is the bit width itself; **the default 10 bits stays unchanged** — raising `sequence_bits` is a deployment-level trade of timestamp lifetime for throughput (16 bits would leave only ~4 years; not recommended).

---

## Optimization Tips

- **Batch first**: high-throughput callers should fetch a batch per `batch_generate` call instead of looping single requests.
- **Algorithm selection**: strong ordering → Snowflake (pure in-memory, zero DB); database-audit semantics or step control → Segment; standard UUID interoperability → UUID v8.
- **Auth cache**: keep `auth.cache_ttl_seconds > 0` in production; understand that revocation takes effect immediately in this process and lags by at most one TTL across nodes.
- **Connection pool**: for Segment-heavy workloads, raise `max_connections` appropriately and watch `/metrics` and database-side waits.
- **Release builds**: always deploy with `cargo build --release` (the dev profile is unoptimized); benchmark data follows the `[profile.bench]` methodology.

---

## Related Documents

- [Architecture](ARCHITECTURE.md): algorithm optimization design and module dependencies
- [Config Migration Guide](CONFIG_MIGRATION_GUIDE.md): config reference and validation rules
- [Test Scenario Matrix](TEST_SCENARIOS.md): how benchmarks and tests layer
- [CHANGELOG](CHANGELOG.md): performance-related change records
