# 📋 Nebula ID Changelog

This file records all notable changes to Nebula ID.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and versioning follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

[🏠 Home](../../README.md) • [🔒 Security](SECURITY.md) • [⚡ Performance Guide](PERFORMANCE.md)

---

## 📋 Table of Contents

- [0.3.0 - 2026-11-14](#030---2026-11-14)
- [0.3.0-rc.3 - 2026-10-15](#030-rc3---2026-10-15)
- [0.3.0-rc.1 - 2026-09-22](#030-rc1---2026-09-22)
- [0.2.0 - 2026-07-23](#020---2026-07-23)
- [🗺️ Development Roadmap](#️-development-roadmap)

---

## [0.3.0] - 2026-11-14

### feat (cloud native and DR: cross-DC failover / Operator / lease-watch wrap-up)

- **`etcd` joins the `default` features (⚠️ breaking, see the Changed section below)**: the standard build now carries `etcd-client` and the cluster coordination path; `--no-default-features` remains the zero-etcd lightweight target (the compilation surface is pinned by ci.yml's all-targets leg).
- **Lease keep-alive watchdog and idempotent release**: `EtcdWorkerAllocator` holds the lease handle and calls `keep_alive` periodically (period = TTL/3); `EtcdDistributedLock` gains a watchdog — a TTL-expiry event (`LeaseInvalid`) triggers a CAS re-acquire and lease rebind, and if someone else has taken over, ownership is judged lost; `LockGuard::release` is idempotent (repeated calls simply succeed; after a rebind, the revoked lease is the new one).
- **Worker-key watch incremental subscription**: adds `EtcdClientOps::watch_prefix` plus `WorkerKeyEvent`/`ClusterEvent`; `EtcdWorkerAllocator::subscribe_worker_key_changes` converges key changes into the unified `EtcdClusterHealthMonitor` event stream as incremental events, replacing kv_get polling (tests pin "zero polling calls").
- **Tri-state health delegated to garrison `HealthRegistry`**: `EtcdClusterHealthCheck` maps Healthy/Degraded/Failed onto garrison Healthy/Degraded/Unhealthy; local-cache degradation (`is_using_cache`) stays a NebulaId-specific behavior and is not outsourced.
- **Cross-datacenter automatic failover**: `DcFailoverController` (conservative predicate: switch to the smallest dc number only when active is judged `Failed` and a `Healthy` candidate exists; concurrency goes through an atomic CAS that admits exactly one switch); segment IDs are partitioned by the dc-bit prefix (top 8 bits, local space 2^56, `dc=0` identical to existing IDs), `GenerateContext` gains a dc dimension filled by `AlgorithmRouter` with `active_dc()`; real two-etcd test `tests/etcd_failover.rs` (`etcd-a`/`etcd-b` containers, triggered with `--ignored`).
- **Kubernetes Operator (new member crate `crates/nebulaid-operator`)**: the `NebulaIdCluster` CRD (spec: replicas/datacenters/version; status: phase/activeDc/perDcHealth/endpoints) + a `kube-runtime` reconcile loop (per-DC StatefulSet + Headless Service, SSA apply, ownerReferences, periodic `/health` probes writing back status and maintaining activeDc by best choice) + graceful shutdown; the `kube`/`kube-runtime`/`k8s-openapi` dependencies live only in that member (zero impact on the main crate's dependency tree, verified with `cargo tree -p nebulaid`).
- **Deliverables**: `deploy/crd/`, `deploy/rbac/` (least privilege; no secrets/configmaps/pods granted), `deploy/manifest/` (Deployment + 2-DC example); `docs/DEPLOYMENT.md` gains the "Kubernetes Operator Installation" and "Cross-Datacenter Failover" sections (including the real-multi-cloud verification Deferred boundary statement).
- **Supply chain**: two pre-existing vulnerabilities surfaced by audit under the unified workspace lock fixed at patch level — `ctutils` 0.4.3→0.4.4, `pulldown-cmark` 0.13.4→0.13.5.

### Changed (⚠️ breaking: `default` features now include `etcd`)

- **Self-developed dependencies pinned to stable releases**: confers `0.6`, trait-kit `0.5`, oxcache `0.5`, dbnexus `0.6`, inklog `0.3`, sdforge `0.5`, limiteron `0.3`, garrison `0.9` all converge from rc-pinned versions to minor-precision stable releases (published on crates.io), shedding rc-era transitive redundancy such as icu/fluent/num-rational.

- `default = [..., "garrison-auth", "etcd"]`: the default build now carries `etcd-client` (coexisting at the same version as tonic 0.14, no second gRPC stack) and the cluster coordination path, growing the artifact; embedding scenarios that want a lean tree should use `--no-default-features` and add `sdk`/algorithm features as needed.
- Behavioral surface: builds that previously "configured `etcd.endpoints` without enabling the `etcd` feature" silently fell back to an in-process local lock (duplicate minting across instances); the default build no longer has that branch, and an explicit `--no-default-features` trim now fail-closes at startup (`src/main.rs` guard and SDK `build()` return `ConfigurationError`).
- **Workspace-ization**: the root `Cargo.toml` gains `[workspace] members=["crates/nebulaid-operator"]` (the main crate remains the `[package]` root); the release chain is unchanged, only `nebulaid` releases (the operator member has `publish = false`, asserted in reverse by a release.yml dry-run exclusion); `smallvec` converges from 1.16 to 1.15 (kube 4.2 transitively hard-constrains `smallvec <1.16.0` via serde-saphyr).

### Roadmap status alignment

- The bilingual README "Cloud native and DR" row moves from 📋 to 🚧 with sub-status expanded: dynamic algorithm switching ✅, Segment routing benchmark ✅ (noted "shipping with the next release"), etcd enabled by default ✅ + lease/watch wrap-up ✅, health delegated to garrison ✅, cross-DC failover ✅ and Operator ✅ (real multi-cloud cluster verification pending a window).

### fix (SDK / etcd productionization wrap-up)

- **SDK Segment repository wiring fix (P0)**: `RouterModule` previously discarded the injected repository, so in production builds Segment fell to `UnconfiguredSegmentLoader` (unit tests falsely passed because `cfg(test)` injected `TestSegmentLoader`). It now wires `AlgorithmRouter::with_segment_repository` through the TypeMap `RepositoryInput`, and also removes the waste of "booting a second connection pool despite an injected repository"; `test_id_generator_segment_with_repository_succeeds` now asserts with MockDatabase differentiation.
- **Non-etcd builds fail closed**: when endpoints are configured without the `etcd` feature, the main service refuses startup and SDK `build()` returns `ConfigurationError` (previously it silently fell back to `LocalDistributedLock`, which would mint duplicate IDs across instances).
- **SDK etcd operation timeout wiring**: `operation_timeout_secs` now takes effect on the SDK path (same source as the main service).
- **Image and sample config consistency**: `docker/Dockerfile` builds with `--features etcd`; the default `etcd.endpoints` in `config/config.toml` becomes `[]` (compose still injects via `ETCD_ENDPOINTS`).
- **Docker build chain fix**: the builder moves up to `rust:1.97.1-bookworm` (confers 0.6.0-rc.10 is edition 2024 / rustc ≥ 1.97.1), and the runtime aligns to `bookworm-slim` + `libssl3`; dependencies all come from crates.io now, so `vendor-deps.sh` and the BuildKit named context were removed and `docker build -f docker/Dockerfile .` builds directly.

### test (real etcd and script methodology)

- Adds 3 `#[ignore]` real-etcd lock cases (exclusivity / release re-entry / TTL expiry) with one-click orchestration via `make test-etcd`; `health-check.yml` gains a push-triggered `etcd-integration` job.
- `tests/distributed_test.sh` is renamed for what it is — an HTTP minting consistency smoke (no etcd dependency): strict mode is explicitly turned off after `source lib.sh`, numeric ID assertions and algorithm-switch-failure skipping are added, and `SKIPPED` is tallied.
- **Runtime-measured fixes**: `tests/lib.sh` health assertions become case-insensitive (the server's `/health` returns PascalCase `Healthy`); adds `TEST_ADMIN_AUTH_HEADER` / `TEST_ADMIN_KEY_ID`+`TEST_ADMIN_KEY_SECRET` to separate the minting User key from the algorithm-switch Admin key; `TEST_WORKSPACE` is now truly passed into minting requests; timestamp normalization switches to a `sed` character set, fixing the arithmetic-comparison warning caused by `tr -d '[:-]'` being parsed as a character range.

### perf (Segment benchmark and batch capacity)

- Adds `benches/segment.rs` (required-features `segment`): 7 cases covering steady-state single, batch 10/100/1000/10000, segment-boundary crossing, and the routing chain; `SegmentData` is made public to support external stub loaders.
- `benches/algorithms.rs` adds the `snowflake/batch_generate_10000_seq_bits_{10,12,14}` capacity sweep; `required-features` gains `snowflake`.
- **errata**: the rc.3 entry's auth-cache gate "P99 +690%" and PERFORMANCE.md's "+597%" were inconsistent in methodology; this rerun measures the `get` path at +652.2% and the `get_or` path at +646.4% (throughput −56.4% / −57.4%), and PERFORMANCE.md now labels the two paths separately per the gate JSON (`target/auth_cache_adoption-latest.json` is authoritative).

### docs

- PERFORMANCE.md: benchmark count 3→4; the Segment blocking rationale (`pub(crate)`) corrected to publicly injectable; `batch_10000` cap wording corrected.

---

## [0.3.0-rc.3] - 2026-10-15

### feat (add-uuid-v5-derive: RFC 9562 v5 deterministic derivation)

- **New `uuid-v5` zero-DB leaf feature** (on by default): the two pure functions `derive_uuid_v5(namespace, name)` and `derive_uuid_v5_batch(pairs)` (stateless / clockless / IO-free), feature-gated; the SDK surface `IdGenerator::derive_uuid_v5` is a synchronous thin wrapper (the `sdk` feature).
- **Design boundary**: v5 deliberately stays out of `AlgorithmType` / `AlgorithmRouter` / `/generate` — a derivation failure has no fallback semantics (falling back to a minting algorithm would produce a numerically wrong non-v5 ID). ~~UUID v4/v7 are likewise not algorithm-family members in this repo (only `FromStr` input aliases)~~ — **that sentence is a historical snapshot**: add-uuid-v4-v7 has already reversed v4/v7 into true algorithms (see the next entry); v5's "outside the algorithm family" boundary is unchanged.
- **Numeric compatibility anchor**: bit-identical to `Uuid::new_v5` of the same crate and version; tests cross-check against both the pinned values of etyma's 340,566 legacy IDs (`ecdict|abandon|en` → `21e7d32f-5500-5e4a-a943-219059f73c94`) and an independent Python hashlib SHA-1 baseline. etyma-side adoption (unified generation entry) follows its 0.6.0 iteration.
- **T010 release gate remediation**: the dependency-tree size and test count figures are pinned to reproducible methodologies (`cargo tree -e normal --prefix none | sort -u | wc -l` and an exactly-escaped `grep -rnE`; the docs' two earlier grep methodologies went unescaped and inflated counts by ~30 items); the spec checklists (bilingual README / API_REFERENCE / USER_GUIDE / CONTRIBUTING / FAQ) gain `uuid-v5`; API_REFERENCE adds a v5 API section (including the v5-only direct-call path and namespace conventions); TEST_SCENARIOS adds the ALG-09 scenario row.

### feat (add-uuid-v4-v7: UUID v4 / v7 true algorithms + ⚠️ breaking alias reversal)

- **`AlgorithmType` gains `UuidV4` / `UuidV7`**: two zero-DB leaf features (`uuid-v4 = []` / `uuid-v7 = []`, in default) + true implementations (`src/core/algorithm/uuid_v4.rs` stateless random; `uuid_v7.rs` holds no state of its own and gets in-process ordering from the uuid crate's process-level monotonic context), both implementing `IdAlgorithm` and entering the router (true minters of the same family as v8).
- **⚠️ Breaking change (semantic reversal)**: `from_str` maps `"uuid_v4"/"uuidv4"/"uuid4"` → `AlgorithmType::UuidV4` and `"uuid_v7"/"uuidv7"/"uuid7"` → `AlgorithmType::UuidV7` — previously they were input aliases of `UuidV8` (configuring uuid_v7 actually produced v8). The output layout of these two strings in historical configs/API requests changes accordingly; **measured usage of the legacy strings is zero** (no use in `config/*.toml`), and the `uuidv8`/`uuid8` aliases are unaffected. The reversal also means people configuring uuidv4 finally get a random UUID.
- **Exclusive capabilities listed** (no version can substitute for another; see the USER_GUIDE division table): v4 = 122 random bits + fully zero metadata (leaks neither creation time nor deployment structure); v7 = RFC 9562 §5.7 standard time-ordered (timestamp directly readable by any standard tool) + zero config (no dc/worker/shard); v8 keeps the tenant-context private layout exclusively.
- **Knock-on surface**: `/parse` gains `uuid_v7` (standard timestamp extraction) and `uuid_v4` (explicitly zero metadata, not unknown) branches; `set_algorithm`'s allowlist now uses `AlgorithmType::from_str` as the single mapping source (the error message lists the five algorithms); the PostgreSQL `algorithm_type` enum is widened via a standalone DO block `ADD VALUE` (idempotent on existing databases; PG 12+ in-transaction ADD VALUE semantics); the DB entity enum `AlgorithmTypeDb` gains the two variants in sync.
- **Documentation methodology reversal**: the lib.rs module header, USER_GUIDE (v4/v7 usage sections + four-version division table + breaking notice), API_REFERENCE (UUID v4/v7 sections + alias note rewrite), the bilingual README feature table, and the CONTRIBUTING/FAQ default feature lists all sync to the five algorithms.

### feat (selfdev-lib-adoption-r1: self-developed library adoption)

- **i18n folded into sdforge**: the fluent-bundle / unic-langid direct dependencies are removed, and the FTL key set is registered into sdforge `register_translation`; the lookup path uses a registration-time `OnceLock` snapshot for lock-free lookup + local rendering (bypassing sdforge's process-level registry global `Mutex`), with local rendering cross-checked against `translate_for` over the full key set (argumented / argument-less / multi-arg).
- **garrison takes over API key brute-force protection**: `InMemoryDao` is consumed via this repo's `server::auth` facade, with an outer `BoundedDao` capacity valve (100k keys FIFO — restoring the key-space upper bound missing from garrison's DAO: its expired keys are only lazily cleaned, with no sweeper); HTTP and gRPC share one failure bucket, and observed faults fail closed.
- **confers HotReloader hot-reload wiring**: a notify event-driven watcher with explicit reporting of dropped increments (the in-loop per-event path and the pre-close final drain share one reporting path); adds the benchmark `benches/auth_cache_adoption.rs` (auth-cache adoption performance gate, verdict=reject: oxcache get P99 +690%, gate codified: P99 ≤ +10% / throughput ≥ −5%).
- **T010 release gate remediation**: removes two zero-usage dependency features (dbnexus `migration` — pulls heavy sqlparser deps but run_migrations uses sea-orm's own SQL; limiteron `audit-log`/`circuit-breaker` — only unconditionally consumed error/limiters modules), shrinking the default dependency tree 526→519; PERFORMANCE.md's i18n benchmark numbers are re-measured and refreshed for the new implementation (snapshot rendering at the 51ns/126ns tier); the `sdforge_version` field semantics (returning this crate's version) are tracked separately, not in this batch.
- **Pre-release triple-review remediation** (security / architecture / performance parallel subagents, rule 10): brute-force observation faults fail-open → fail-closed (ErrDao stub test pins the polarity); unbounded DAO growth → capacity valve (BoundedDao); the i18n hot path bypasses the global lock; `test_support` restores `#[cfg(test)]` (no longer enters the public surface of the release library); garrison features trimmed to `firewall-bruteforce` (web-axum / protocol-apikey / cache-memory unused); bench `required-features` completed (trim combinations now skip correctly instead of failing to compile).
- **Known trade-offs**: ① the core layer now depends on sdforge unconditionally (a declared consequence of the adoption decision: pure-algorithm consumers also compile the sdforge tree); sdforge's i18n registry is a process-level shared mutable global — NebulaId's lookup path bypasses it (local snapshot), but other sdforge users in the same binary share the key table with NebulaId, and future key naming must guard against collisions. ② i18n missing-argument rendering differs cosmetically from the old Fluent (`{$name}` vs `{ $name }`), visible only when a call site omits an argument (itself a bug). ③ gRPC's error-code distinction for disabled / expired / unknown keys constitutes a key-state enumeration side channel (pre-existing before this batch, not introduced here; tracked separately).

### fix (uuid_v8 uniqueness defense fix)

- **Counter random-start failure fix**: the counter seed in `UuidV8Impl::new` was `rand26 << 31 then & 21-bit mask`, which is always 0 — UUIDP's "random start" mechanism never actually took effect, and the cross-restart same-millisecond collision defense had degenerated to only the 26-bit rand. Now takes a 21-bit random start directly, restoring the design guarantee (a new randomness test pins it).
- **Batch minting counter wrap guard**: `batch_inner` explicitly rejects with `SequenceOverflow` when size exceeds the per-millisecond counter space (2^21) (in the worst case an entire batch landing in the same millisecond would wrap and duplicate) — static rejection rather than silent duplicates.

### deps (rustls-pemfile retirement)

- **Removes the `rustls-pemfile` dependency** (RUSTSEC-2025-0134: the upstream is archived and unmaintained; the official pointer is rustls-pki-types' `PemObject` API): the three TLS load sites — certificate/key/CA — migrate to `rustls::pki_types::pem::PemObject` iterators (via the rustls re-export, zero new dependencies). Semantics preserved one by one: take the first target item, skip non-target items, unchanged parse-error mapping, and unchanged empty-file error messages (`Empty certificate chain` / `Empty private key` / `Empty CA certificate`).

### fix-audit-defects-r1 (audit fix round)

An audit against the "distributed ID interview question list" found 10 defects (FMEA-ranked RPN 60–320); this round fixes all of them:

**Correctness (P1)**
- **Snowflake bit-width validation**: startup rejects when `dc_id`/`worker_id` exceed the configured bit width (the old `dc_id ≤ 31` check both allowed bit-overflow configs of 8..=31 and wrongly rejected legal values at wide bit widths; out-of-range values pollute the high timestamp bits during ID assembly, causing cross-DC combination collisions). Intercepted at both `Config::validate` and `SnowflakeAlgorithm::initialize`.
- **Segment batch fill-up**: `try_consume` becomes "consume at most" semantics, and `batch_generate` loops across segments to fill up, so `Ok` responses satisfy the hard invariant `ids.len() == size` (previously a silent short-batch exit existed).
- **Restart clock guard** (`algorithm.snowflake.state_guard`, on by default): every 30s at runtime the last minted timestamp is persisted to `./data/snowflake_state_{dc}_{worker}.json`; at startup, cross-restart clock rollback is detected and startup is refused fail-closed.
- **update_segment gains a dc_id filter**: multi-DC rows no longer stomp on each other (breaking signature change; only in-repo call sites affected).

**Concurrency health (P2)**
- Segment loading waits change from synchronous spinning to async 1ms polling (eliminating a current_thread runtime deadlock risk).
- Single and batch paths unify on `ensure_next_segment` (the batch path previously had no loading CAS, so concurrent segment fetches overwrote each other and wasted whole segments).

**DB layer (P2/P3)**
- The atomic allocation UPDATE maintains the `max_id` column in sync (previously written only once on INSERT, distorting observability).
- Adds a `ceiling` cap column (NULL = uncapped): the allocation SQL carries a guard and returns `SegmentExhausted` when the cursor tops out. **Existing deployments must run `scripts/migrations/001_add_segments_ceiling.sql` before upgrading** (the allocation SQL references the column).

**Capability completion (P3)**
- **Guess-resistance obfuscation**: adds `IdFormat::Obfuscated` (a 4-round Feistel u64 bijection, configured via `algorithm.obfuscation.key_hex` as 16 hex chars, explicit error when unconfigured; `Id::render` is the rendering entry, internal storage unchanged).
- **Dynamic step truly wired**: `DbSegmentLoader::with_dynamic_step` computes the step within `[min_step, max_step]` by CPU usage (previously the CpuMonitor samples had no consumer, and the README claim did not match the implementation).
- **Batch fallback logging symmetry**: the batch path's fallback hits now emit an info log (matching the single path).

### deps (self-developed dependencies upgraded to the latest RC wave)

- **All 8 self-developed crates upgraded**: confers `0.6.0-rc.6`→`0.6.0-rc.10`, oxcache `0.5.0-rc.4`→`0.5.0-rc.8`, dbnexus `0.6.0-rc.5`→`0.6.0-rc.9`, sdforge `0.5.0-rc.5`→`0.5.0-rc.8`, inklog `0.3.0-rc.5`→`0.3.0-rc.9`, garrison `0.9.0-rc.2`→`0.9.0-rc.3`, trait-kit `0.5.0-rc.6`→`0.5.0-rc.9`, limiteron `0.3.0-rc.5`→`0.3.0-rc.7`.
- **utoipa `5.5`→`6.0` alignment**: sdforge rc.8 internally upgrades utoipa to 6.0, and this crate's direct dependency upgrades in sync to eliminate dual-version type conflicts.
- **Automatic gains**: confers' YAML backend migrates to serde-saphyr (eliminating the RUSTSEC-2024-0449 root cause) and fixes strict_booleans generic value parsing; inklog's redaction engine defects fixed (IPv6 compressed-form redaction, api_key single-marker idempotency, expanded directory-mode error details); sdforge removes the StructuredLogger stack, converging inklog.LoggerManager's public surface (zero usage in this repo).
- **Verification**: `cargo check --all-features` with zero errors; `cargo test --lib` (default 1692 / all-features 1971) passes / 0 failures.

## [0.3.0-rc.1] - 2026-09-22

The first release candidate toward 0.3.0. All changes accumulated since v0.2.0 ship with this version: beyond the wiring fixes, SDK hardening, key-rotation grace period, and config fail-fast detailed below (**including multiple behavior changes deployers should note**), the final pre-release batch of "ecosystem RC-wave alignment" changes is summarized here.

### RC wave alignment (pre-release wrap-up)

- **Self-developed ecosystem fully switched to the crates.io registry**: removes the `../base`, `../garrison`, and other sibling-repo path dependencies; confers `0.6.0-rc.5` / oxcache `0.5.0-rc.4` / dbnexus `0.6.0-rc.5` / sdforge
  `0.5.0-rc.5` / inklog `0.3.0-rc.5` / garrison `0.9.0-rc.1` / trait-kit `0.5.0-rc.6` /
  limiteron `0.3.0-rc.4` align to the same RC wave; the repository builds standalone from now on, CI no longer needs to check out sibling directories, and under pre-release semantics dependencies float automatically per semver.
- **i18n stack migrated to the Fluent/ICU stack (fluent-bundle + unic-langid + sys-locale, aligned with
  the unify-rust-i18n baseline; fluent-bundle as a direct dependency, without introducing the unify-rust-i18n crate)**: rust-i18n's YAML resources
  `locales/en.yml` / `locales/zh-CN.yml` are replaced by `locales/{en,zh}/messages.ftl` (embedded at compile time via
  `include_str!`, with the key-set alignment guard now applying to the FTL resources); the process default locale is configurable
  (detection chain `NEBULA_LOCALE` → `[app].locale` → `LC_ALL`/`LC_MESSAGES`/`LANG` →
  sys-locale → `en`); OpenAPI doc descriptions localize per language.
- **validator direct dependency removed**: consumed via the `sdforge::validator` re-export; sdforge upgrades to
  `0.5.0-rc.5`, and the swagger UI switches back to the sdforge re-export.
- **Connection establishment migrates to dbnexus' unified `DbPool` connection pool** (adopting parallel-session work).
- **CI fixes**: fixes 8 rustc 1.98 clippy lints on the etcd feature leg; gates the sdforge route-count test under no-default builds; clears pre-existing fmt violations before enabling the lefthook pre-commit hook.
- **Docs and engineering wrap-up**: cleans internal task-number references (T0xx/Phase N/Lane) from docs; normalizes the LICENSE title block and the proto module declaration file's copyright header.

### Breaking (breaking API changes)

- **The embedded SDK fully migrates to trait-kit assembly (change `sdk-trait-kit-migration`)**:
  `NebulaIdClient` / `NebulaIdClientBuilder` are removed; the new facade is `NebulaIdKit` /
  `NebulaIdKitBuilder` (assembly validated by the trait-kit AsyncKit dependency graph, with missing dependencies/cycles
  erroring in `build()`). Migration mapping:

  | Old API (removed) | New API |
  |---|---|
  | `NebulaIdClientBuilder::new(config)` | `NebulaIdKitBuilder::new(config)` |
  | `.with_audit_logger(logger)` | `.with_audit_logger(logger)` (same name) |
  | `.with_repository(repo)` | `.with_repository(repo)` (same name) |
  | `.build().await?` → `NebulaIdClient` | `.build().await?` → `NebulaIdKit` |
  | `client.generate(ws, group, tag).await?` | `kit.id_generator()?.generate(ws, group, tag).await?` |
  | `client.batch_generate(ws, group, tag, n).await?` | `kit.id_generator()?.batch_generate(ws, group, tag, n).await?` |
  | `client.generate_with_algorithm(alg, ..).await?` | `kit.id_generator()?.generate_with_algorithm(alg, ..).await?` |
  | `client.health_check().await` | `kit.health_check().await` (algorithm-level snapshot, semantics unchanged; for module-level reports use `kit.health_report()`) |
  | `client.shutdown().await` | `kit.shutdown().await` |
  | `client.has_repository()` | `kit.has_repository()` |
  | `client.router()` (escape hatch) | **removed**: `AlgorithmRouter` no longer appears on the SDK public surface, and the Segment-without-repository guard can no longer be bypassed |

  Generation semantics are unchanged: pure algorithms (snowflake/uuid_v8) work with zero DB; Segment needs
  `.with_repository(..)` and returns `CoreError::ConfigurationError` when not injected.
  The `IdGenerator` handle is `Clone + Send + Sync` and shareable across tasks (lighter than caching the whole Kit).

### Changed (behavior changes)

- **Snowflake millisecond-rollover waiting becomes a hybrid spin-then-sleep**: `wait_for_next_ms` previously
  did a fixed 1ms-period `tokio::time::sleep`, and timer-granularity overshoot pressed sustained minting
  throughput to about half the bit-width theoretical ceiling (1023 ID/ms ≈ 1.02M IDs/s at the default 10-bit sequence); now it
  spins first (≤2ms budget, covering the sub-millisecond wait of a millisecond rollover), and only when the budget is exhausted without crossing the target millisecond
  (clock-rollback waits) does it fall back to a 1ms sleep, avoiding long idle waits. Benchmarks confirm serial / batch
  / extra-large batch all converge to the bit-width ceiling (≈ 2.1×, see
  [PERFORMANCE.md · Recorded baselines](PERFORMANCE.md#recorded-baselines)); semantics unchanged —
  it still strictly waits for the real clock to cross the target millisecond before minting. Also adds the
  `snowflake/batch_generate_10000` benchmark case (one `block_on` amortized over 10k IDs,
  approximating the "pure reserve path" lower-bound methodology).

- **HTTP workspace/group read endpoints gain tenant isolation**: `GET /api/v1/groups` previously did no
  role validation at all and listed any tenant's groups per the client-provided `?workspace=`; it now narrows to a
  User-only endpoint (Admin calls return 403), and a User key can only list its own workspace
  (another's workspace returns 403 `workspace_mismatch`). `GET /api/v1/workspaces`
  filters to the caller's own workspace for User keys (`total` converges accordingly), and `GET
  /api/v1/workspaces/{name}` admits only the caller's own workspace for User keys (another's → 403);
  **Admin keeps full cross-tenant reads (behavior unchanged)**. Ownership determination reuses the
  single-point `authorize_workspace_access` shared with gRPC.
- **Unified error envelope gains `business_code` (Minor, non-breaking)**: all HTTP error
  responses (including rate limiting, body-size limits, API version errors) unify into a single envelope
  `ErrorResponse { code, business_code, message, details, request_id,
  timestamp }`. `business_code` is a four-digit string (e.g. `"4001"`) sharing a single classification table
  `CoreError → (status code, business code)` with the HTTP status code; clients should branch on it
  rather than parsing the `message` copy; `request_id` (UUID v4) and `timestamp` (millisecond Unix time)
  are generated at response assembly and can be correlated with audit logs. The OpenAPI error-code table
  is aligned with the actual mapping (`2002`/`2004` marked as reserved, currently never returned). Rationale for the minor grade: for existing clients the JSON fields are **purely additive** (unaffected if unknown fields are tolerated), and the removed
  `ApiErrorResponse` never appeared in any endpoint response or on the SDK surface — only Rust code directly referencing
  the server's `models::ApiErrorResponse` is affected (source level, already absorbed by the single envelope).
- **HTTP TLS fail-fast**: with `tls.enabled=true` and a missing/unparseable certificate, startup is refused
  (no silent fallback to plaintext); only `enabled=false` allows plaintext.
- **gRPC enables API key authentication**: every NebulaIdService RPC (including bidirectional streams) is verified at each entry by the
  single-point `authenticate()` on `authorization` (Basic/ApiKey). The design draft originally planned tonic
  `with_interceptor`, but the interceptor `call` has a synchronous signature while credential verification must query the DB asynchronously
  (Argon2id + DB), so each RPC entry instead makes a one-line call to the same helper — the single-point implementation is unchanged.
  **gRPC clients without credentials will receive `Unauthenticated`**; with `auth.enabled=false` requests pass.
- **gRPC auth failure-code distinction**: a disabled or expired key → `PermissionDenied`; invalid credentials
  / unknown key → `Unauthenticated` (implementing R-auth-003). Previously always 401.
- **Global rate limiting genuinely takes effect**: `RateLimitMiddleware` changes from "Extension injection only (dead code)"
  to genuinely mounted on the HTTP stack; `POST /config/rate-limit` hot updates act on real traffic.
- **Rate-limit layering, switch, and response headers (with breaking changes)**:
  - The rate-limit layer moves **inside** CORS / security headers: in the old order, 429 was short-circuited at the outermost layer, so clients never got CORS headers, and OPTIONS preflights consumed real quota.
  - `[rate_limit].enabled = false` (the top-level section, not `auth.rate_limit.enabled`) now genuinely skips mounting
    the rate-limit layer (`src/main.rs` reads `config.rate_limit.enabled`; the setting previously had no consumer).
  - `POST /config/rate-limit` reuses the startup-time `Config::validate`; inconsistent runtime combos such as `burst > 10×rps`
    are rejected (previously only per-field range validation existed).
  - The rate-limit response header name source is unified: previously CORS exposed `x-rate-limit-remaining` while the middleware wrote
    `x-ratelimit-remaining` (header names are hyphen-sensitive), so browsers could never read it; both now share the
    `rate_limit::middleware` constants, writing the canonical lowercase `x-ratelimit-limit` /
    `x-ratelimit-remaining`, with 429 additionally carrying `x-ratelimit-remaining: 0`.
- **Auth cache (`auth.cache_ttl_seconds`) wired**: the store is this project's own
  `MemoryGarrisonDao` (an in-process HashMap implementing garrison's `GarrisonDao` interface);
  verification checks the cache first and falls back to the DB on miss; cached values contain only workspace_id/role/expiry (no secret).
  Revocation/rotation/disable takes effect immediately **in this process**; in multi-node deployments other nodes lag by up to one
  `cache_ttl`, and `last_used_at` lags the same way (ops methodology in docs/DEPLOYMENT.md 7.1).
  With `cache_ttl_seconds = 0` no cache instance is assembled.
- **The auth cache no longer accepts glob-shaped key_ids**: `invalidate` becomes exact prefix matching,
  so key_ids containing `*`/`?` can no longer clear all other principals' entries at once.
- **Single-source HTTP auth-header parsing**: Basic/ApiKey parsing of `Authorization` merges into one implementation
  (adding the reason-carrying `parse_authorization_header_detailed`); previously HTTP and gRPC each had one,
  with inconsistent empty-credential handling. Audit reasons and i18n copy are unchanged.
- **biz-tags tenant isolation (IDOR fix)**: on `GET /api/v1/biz-tags` the User role sees only its own
  workspace (ignoring workspace-parameter overrides); Admin filters by parameter; without a repository it no longer silently
  falls back to the nil query.
- **Requests attributed to real IPs**: the HTTP serve injects the connection peer address (DualListener carries `ConnectInfo` in this crate's new `PeerAddr` type). Previously `get_client_ip` was always `None` in production,
  so rate-limit keys, auth failure counters, and the audit `client_ip` all collapsed into the shared `anonymous` bucket —
  a single attacker could push `/health`, `/metrics`, and friends into 429 as well.
- **TLS handshakes no longer serialize**: each connection gets its own task + a 10s handshake timeout, so unauthenticated clients can no longer freeze the entire listening port by "connecting without sending a
  ClientHello" (unauthenticated remote DoS); accept errors gain backoff.
- **Metrics end-to-end closed loop**: latency samples now come from the routing layer (HTTP / gRPC / SDK share one recording point, attributed by the algorithm actually served), and `/metrics` exposes per-algorithm p50/p99/p999 and
  `clock_backwards`; the `clock_backward` alert rule now uses the real rollback counter (previously "this algorithm has delayed samples" triggered it, i.e. always true once any request ran). **Breaking change**: `AlgorithmMetrics`'s
  public atomic fields `p50/p99/p999` are removed, replaced by `latency_percentiles_ns()` /
  `get_p*_latency_ms()`; `MetricsSnapshot` gains a `clock_backwards` field.
- **`p50/p99` semantics**: changed from "historical maximum (only increases)" to real percentiles over the most recent 1024 samples in a ring buffer (one sort yields all three tiers).
- **`DatabaseConfig::default` no longer panics**: defaults no longer `.expect()` the
  `NEBULA_DATABASE_PASSWORD` environment variable (pure-algorithm embedders only need `Config::default()`); establishing a connection
  still fail-fasts with `ConfigurationError`.
- **`tls.min_tls_version` is now genuinely enforced**: previously the setting only wrote logs and never entered `ServerConfig`,
  effectively always rustls' default TLS 1.2+1.3 (the old comment already admitted "deferred to v0.3.0"). The HTTP side switches to
  `ServerConfig::builder_with_protocol_versions`, and with `tls13`, ClientHello messages below TLS 1.3 are rejected outright (**behavior change**: old clients may start failing handshakes). The gRPC-side
  `ServerConfig` is assembled by tonic and cannot inject a version set; startup marks the boundary honestly with a `warn`.
- **`algorithm_type` ENUM aligned with the code (needs a DB migration)**: `scripts/init.sql` previously still created
  `('segment','snowflake','uuid_v7','uuid_v4')`, while the code side only had
  `segment/snowflake/uuid_v8`, so new databases writing `uuid_v8` were rejected. Existing databases follow
  the ENUM migration section of `docs/CONFIG_MIGRATION_GUIDE.md` (including NULL backfill and rollback).
- **Bad config no longer silently degrades to defaults**: previously `main.rs` logged one `error!` on `Config::load_from_file`'s
  `Err` branch and then started anyway with `Config::default()`, running the process on an
  "imagined config" (e.g. a misspelled `[algorithm.uuid_v8]` section name meant the default algorithm actually ran). Now a present file that fails to read or parse is always a fatal exit (exit 1), with the error reaching stderr via
  `Termination` and naming the failing field. Only the case of "no explicit `--config` and the default path file
  does not exist" still falls back to built-in defaults with a warn.
- **Unknown config keys always rejected**: adds 17 `#[serde(deny_unknown_fields)]` annotations (spread across
  10 files in `src/core/config/`). Previously the repo had **none at all** — the top-level `Config`
  included — so any misspelled section/key name was silently dropped by serde, silently voiding whole settings.
  **Breaking change**: any miswritten key in existing configs now blocks startup;
  run the preflight commands from `docs/CONFIG_MIGRATION_GUIDE.md` before upgrading.
- **The key-rotation grace period genuinely takes effect and defaults to off**: the configured value was indeed read and passed into the repository, but
  `rotate_api_key`'s parameter was written as `_grace_period_seconds` (underscore prefix, never read),
  so rotation directly overwrote `key_secret_hash` → the old key died instantly and the grace period was a sham. Now with
  `>0`, rotation persists the previous hash and the window deadline, the old key stays verifiable within the grace window and expires automatically (lazily judged by `rotate_expires_at`), and configs beyond 30 days are rejected outright at startup validation (fail-fast);
  `key_rotation_grace_period_seconds`' default changes from 7 days to `0` (grace period off),
  because the old behavior was equivalent to "no grace period", and defaulting it on would change existing deployments' security-window semantics.
  The `api_keys` table gains the `prev_secret_hash` / `rotate_expires_at` columns; both are backfilled automatically and idempotently by startup-time
  `run_migrations` (`ADD COLUMN IF NOT EXISTS`, aborting startup on failure,
  `src/core/database/connection.rs:310-339`) — **regular deployments need no manual DDL**. Manual execution is needed only when the database account lacks DDL rights or the schema is managed by an external tool — see
  the "key rotation grace period" section of `docs/CONFIG_MIGRATION_GUIDE.md`.
- **A second admin key is genuinely rejected for the first time**: the create-API-key and revoke-API-key admin guards
  previously used a `list_api_keys(workspace_id = NULL)` paged query + in-memory scan, but the global admin
  key's `workspace_id` is NULL and `NULL = nil_uuid` never holds in SQL → the query was always empty →
  the guard never worked, and anyone with admin credentials could open another admin key as a persistence backdoor. It now uses
  exact row-id lookup and an SQL-side `role='admin' AND enabled=true` count (no workspace
  filter, no 1000-row pagination cap). **Note the scope**: the guard applies only on HTTP `POST /api-keys`;
  admin keys bootstrapped from environment/config at startup go through repository-level direct inserts and bypass the guard.
- **The feature model unlocks `--all-features`**: dbnexus' `pub use sea_orm` no longer binds to a driver
  feature (`dbnexus` 40580df; the sea-orm prelude depends only on macros/with-chrono/with-uuid,
  driver-independent), so driver-less builds no longer report 187 errors; the `sqlite` feature is deleted (previously
  the default always included dbnexus/postgres, which inevitably tripped the engine-exclusivity `compile_error!` — it was never buildable), and `--all-features` (= default + etcd + alerting + sdk + integration-tests
  + openapi) is buildable from now on. CI (clippy/test/coverage), lefthook, the preflight scripts, and all
  docs converge on the `--all-features` methodology; the fmt-clippy job adds a
  `--no-default-features --lib --bins` compile check pinning the garrison fallback paths;
  the test matrix changes from default/postgresql/etcd (postgresql was equivalent to default — redundant) to default/all, with the standalone sdk job folded into the all leg (tests only; the coverage gate remains authoritative on the
  default leg at ≥95% — sdk code's test density is low, and gating on the all leg would fail);
  the local coverage gate adds the proto exclusion regex and switches to the all-features methodology with a measured threshold of 90%;
  the text-guard test `repo_docs_guards_tests.rs` premised on the old constraint and 2 dead sqlite tests are deleted. At runtime the `DatabaseEngine`
  enum keeps the `Sqlite`/`Mysql` variants, but the build contains no matching drivers, so connections fail.
- **Fixes the migration drift of the segment unique constraint (real-DB bug)**: `scripts/init.sql`'s
  `nebula_segments` DDL carries `uq_nebula_segments_ws_tag_dc UNIQUE
  (workspace_id, biz_tag, dc_id)`, but `run_migrations`' DDL for the same table omitted it;
  the production `allocate_segment` first-query no-row path `INSERT ... ON CONFLICT
  (workspace_id, biz_tag, dc_id) DO NOTHING` depends on that constraint, so on databases whose tables were created by auto-migration, the first segment allocation was guaranteed to error. The migration now inlines the constraint in the table DDL and backfills existing databases idempotently
  (`DO $$ ... EXCEPTION WHEN duplicate_object`); backfill failure aborts startup
  (missing constraint = first allocation unusable across instances, fail-closed). This fix also turns 7 migration tests that had long failed due to mock quota debt (mocks not updated alongside migration statements) green,
  **the full test suite now has 0 failures**, and the local coverage gate drops its `--skip`.
- **License statement unified as Apache-2.0**: the repo's only license text, `LICENSE`, has always been
  Apache-2.0; `LICENSE-MIT` never existed (the `../LICENSE-MIT` referenced by old FAQ copy
  was a dead link), and the dual-license claims (Cargo.toml metadata, README badges and footer) were empty promises.
  The Cargo.toml `license` field, the bilingual README badges and footer, and the FAQ license section unify on
  Apache-2.0.
- **Factual doc corrections** (closing out leftover WIP): config docs for `database.engine`/pool defaults/
  the `dc_id` range (0-7) and friends aligned with the implementation; the `nebula_segments` manual DDL aligned with `init.sql` (VARCHAR(255), the `dc_id` column, the unique constraint); the deployment architecture diagram
  corrected so metrics are exposed via the main port's `/metrics` (`METRICS_PORT` and similar env vars currently have no
  code consumers — leftover dead config); FAQ's Rust base image, rate-limit description, license
  section, and links corrected.

### Added

- **Embedded SDK (feature `sdk`)**: the new trait-kit-assembled facade `NebulaIdKitBuilder` /
  `NebulaIdKit` (the old `NebulaIdClient` / `NebulaIdClientBuilder` were removed in the same version;
  migration mapping above in Breaking); `build()` gathers distributed-lock injection / router initialization / degradation background
  tasks; `examples/embedded.rs` (zero DB, zero network) and `examples/sdk_server.rs`
  (sdforge `#[forge]` wrapper + OpenAPI).
- **CI/build**: `ci.yml`'s clippy and test matrix switch to single features (`--features etcd`,
  because dbnexus forbids mixing sqlite+postgres); `docker/Dockerfile` adapts to the single-package
  `nebulaid` build; the new `openapi` mirror feature.
- **Config error classification**: `ConfigError::FileNotFound` (distinguishing "file missing" from "file present but
  read/parse failure"), so the startup decision matrix can fall back to defaults only on the former.
- **The startup config decision policy function** `resolve_startup_config(path, explicit_path)` and
  `StartupConfig`: the trade-offs across the four combinations of "explicit path / default path / file exists / parse succeeded"
  converge into one unit-testable pure function (`src/core/config/app_config.rs`);
  `main.rs` only consumes its return value.
- **Grace-period observability**: `ApiKeyWithSecret` gains a `grace_expires_at` field (`None` when the grace period
  is disabled or not in the window); new i18n copy keys for the grace period and admin guard.
  All fields of this struct are `pub` with no `#[non_exhaustive]`, so for **external constructors this is a source-level
  breaking change** (the new field must be added); in-crate construction sites are synced.

### Fixed

- **Capacity-level log-pipeline stall causing service-wide stalls**: the inklog main async channel is
  `bounded(channel_capacity)` (default 10000), and when the file sink has no explicitly configured path there is no
  consumer to drain it; after the service emitted ~10k log lines, every log line blocked for `send_timeout(100ms)` before degrading. The audit `log_shared` also emitted its ring-full `warn!` across the mutex, serializing every request's
  audit write at 100ms apiece — measured HTTP throughput collapsed from ~14k rps to ~6 rps after ~10k requests
  (~11s per request, server CPU idle). Fixed at two layers: `init_observability` explicitly configures the inklog file sink (the channel drains continuously); the drop warning moves outside the mutex (however slow the log backend, requests never serialize). The regression test `tests/audit_ring_stall_repro.rs` fills the channel
  and asserts log() does not block. Post-fix, measured at c=64: /health 13,675 rps (p99 16ms),
  /generate 4,974 rps (p99 39ms), batch 100 ids at ~284K ids/s.
- **Fresh-database migrations no longer fail**: the segment unique-constraint backfill DO block only
  caught `duplicate_object` (42710), but on the fresh-database path the table DDL's inline same-named constraint
  `uq_nebula_segments_ws_tag_dc` had already created the same-named index first, so `ADD CONSTRAINT` raised
  "relation already exists" (42P07 `duplicate_table`), which was not swallowed → the first startup migration on a fresh database always failed and exited. Now both conditions are caught, and both the existing-database upgrade and fresh-database initialization
  paths are idempotent (the pinning test is synced).
- **Shutdown no longer leaks background tasks**: the `rate_limit_cleanup` and degradation sweep tasks were previously reclaimed only on the
  `shutdown_signal` branch; when the server exited first (normal stop or an error return) they leaked,
  and dropping the tokio runtime would wait forever on that never-self-terminating loop task. Now a select over all branches only produces results and reclamation happens uniformly after exit (the etcd / non-etcd blocks are isomorphic).
- **Contradictory TLS config no longer silently goes plaintext**: when `tls.enabled = false` with
  `http_enabled`/`grpc_enabled = true`, the per-port switches were ignored and ports started in plaintext with no notice; now an explicit `tls_config_conflict` warn is emitted. The misleading startup log copy
  "HTTPS is enabled but using HTTP fallback for now" is replaced with an honest description
  (that port really does terminate TLS).
- **`docker/vendor-deps.sh` no longer packages credentials**: when copying dependencies into `.docker-vendor/`, `.env*` / `local_settings*` exclusions are added (before the fix, the dependency repo's
  `.env.test` / `.env.example` were verifiably included).
- **SDK examples no longer echo internal errors**: `to_api_error` previously sent `e.to_string()` straight to the client
  (sdforge forwards the message verbatim); it now returns a fixed external summary + `error_id`, with internal details going to logs only.
- **Locale key-set alignment guard**: adds a test asserting that `locales/en.yml` and `locales/zh-CN.yml`
  top-level keys are fully identical (a single-sided missing key would only silently fall back in that language); also deletes three keys orphaned by the `min_tls_version` rework (en + zh). The guard later follows the i18n stack migration to apply to
  `locales/{en,zh}/messages.ftl` (see "RC wave alignment" above).
- **Snowflake concurrent duplicate-ID race**: `(last_timestamp, sequence)` migration is serialized;
  concurrency no longer produces duplicates.
- **Degradation chain dedup**: the fallback chain `[Snowflake, UuidV8]` (duplicate placeholders removed).
- **main.rs hygiene**: deletes the duplicate router construction in the non-etcd branch; the hot_reload watcher's start condition is decoupled from features.
- **Config files with non-ASCII no longer panic**: `Config::load_from_file` byte-sliced `[auth]` with `&expanded[start..start+100]` before debug logging; when the config file contained Chinese comments or
  Chinese values, that index did not land on a character boundary → the process panicked before TOML parsing (regardless of log level,
  measured exit 101). Now truncates by characters. Regression case: `load_from_file_with_non_ascii_comments_does_not_panic` in `app_config.rs`.
- **Deletes dead config keys with no consumer**: `[logging] backtrace = false` in `config/config.toml` and `config/config_test.toml` — `LoggingConfig` has no such field and the repo never referenced it;
  it was silently ignored until now, but under unknown-key rejection it would prevent these two configs from starting, so it is removed first.
- **`test_uuid_v8_counter_resets_on_millisecond_rollover` is no longer flaky**: the case originally assumed "two consecutive generations must cross a millisecond", failing 2 of 8 isolated runs on fast machines. It now
  determines whether samples share a millisecond from the timestamp embedded in generated values, asserts the counter +1 only for same-millisecond samples, and explicitly asserts
  "at least one same-millisecond pair among 4 samples" as a test precondition.

## [0.2.0] - 2026-07-23

v0.2.0 is the first release since v0.1.1, shipping 11 phases of hardening,
refactor, and developer-experience work plus subsequent security hardening.
Highlights: three strix security fixes, a dbnexus/sdforge/confers architecture
takeover, a 1829-test e2e suite at 95% coverage, garrison DAO infrastructure,
and redundant-comment cleanup.

### Added

- **garrison DAO infrastructure** (`src/server/auth/memory_dao.rs`): full
  in-memory `GarrisonDao` implementation (TTL, glob, atomic get_and_delete /
  incr / decr / CAS) for garrison `ApiKeyHandler`. Feature-gated under
  `garrison-auth`; not yet wired into the request path (migration design in
  `temp/garrison-migration-plan.md`, deferred to a later change).
- **e2e test suite** (commits 36c8bf8, fe10013, f7ac3be): 1829 end-to-end
  tests covering 76 functional scenarios (95% module coverage); fixed
  `snowflake.rs` `batch_generate(0)` boundary bug and `audit/logger.rs`
  per-event `sync_all` performance bottleneck (added `flush()` interface).
- **distributed analysis report** (`specmark/reports/distributed-analysis.md`):
  2.5/5-star assessment with P0/P1/P2 improvement roadmap (EtcdWorkerAllocator
  dead code, Dockerfile etcd feature, WORKER_ID env var).

### Changed

- **Architecture takeover** (commit bc4980c): database / HTTP / gRPC / config
  fully delegated to `dbnexus` / `sdforge` / `confers`; Cargo.lock now tracked
  in VCS.
- **strix security fixes** (commit 9a26926): IDOR in BizTag endpoints (added
  workspace verification), config mutation (moved endpoints to admin routes),
  metrics leak (replaced DB error strings); plus `inklog` EnvFilter fix and
  `trait-kit` DI.
- **Cargo.toml cleanup** (commits 4e022af, d573317): removed unused deps,
  updated all deps to latest, fixed 9 version-format violations (rule 25).
- **Redundant-comment cleanup** (`src/core/coordinator/etcd.rs`): removed 39
  decorative separators and code-restating comments; 63 etcd tests still pass.

### Fixed

- **TOCTOU race** in `DatabaseConfig::default` (commit 9b9b884).
- **worker_id start from 1** in etcd allocator + HangingPingClient coverage
  (commit 240b9eb).
- **i18n locale test isolation** via `LOCALE_LOCK` (commit 70bfc3e).

### Security

- **strix-0001 (IDOR)**: BizTag endpoints lacked workspace verification.
- **strix-0002 (config mutation)**: config endpoints exposed to non-admin.
- **strix-0003 (metrics leak)**: DB error strings leaked in metrics.

### Deprecated

- Nothing deprecated in v0.2.0.

---

## 🗺️ Development Roadmap

<details>
<summary>📋 Table</summary>

<table style="width:100%; border-collapse: collapse">
<tr><th style="text-align:center">Status</th><th style="text-align:left">Area</th><th style="text-align:left">Items</th></tr>
<tr><td align="center">✅</td><td>Core algorithms</td><td>Segment double buffering with dynamic step, Snowflake, UUID v8, algorithm router and degradation chain</td></tr>
<tr><td align="center">✅</td><td>Service and protocols</td><td>HTTP + gRPC dual protocol, OpenAPI docs, TLS, rate limiting, garrison API key auth, audit logging</td></tr>
<tr><td align="center">✅</td><td>Observability and i18n</td><td>Per-algorithm p50/p99/p999 metrics, health checks, inklog structured logging, en / zh-CN i18n</td></tr>
<tr><td align="center">✅</td><td>Quality gates</td><td>Five-stage CI gate, coverage ≥ 95%, cargo-deny / cargo-audit / CodeQL, ~1900 tests</td></tr>
<tr><td align="center">🚧</td><td>SDK and distributed coordination</td><td>Continued hardening of the <code>sdk</code> facade (trait-kit assembly); <code>etcd</code> coordination is now enabled by default (see the "Cloud native and DR" row)</td></tr>
<tr><td align="center">🚧</td><td>Performance engineering</td><td>Batch generation tuning, Segment routing benchmark integration (the ID-generation throughput benchmark harness and release baselines have landed — see the [📈 Performance Guide](PERFORMANCE.md))</td></tr>
<tr><td align="center">🚧</td><td>Cloud native and DR</td><td>Dynamic algorithm switching ✅ (<code>AlgorithmRouter::set_algorithm</code>), Segment routing benchmark ✅ (<code>benches/segment.rs</code>, shipping with the next release), <code>etcd</code> coordination enabled by default ✅ (<code>default</code> includes <code>etcd</code>) plus lease keep-alive watchdog and watch-based incremental subscription ✅, cluster health tri-state delegated to garrison <code>HealthRegistry</code> ✅, cross-datacenter automatic failover ✅ and a Kubernetes operator ✅ (delivered in this change; real multi-cloud validation pending a deployment window)</td></tr>
</table>

</details>
