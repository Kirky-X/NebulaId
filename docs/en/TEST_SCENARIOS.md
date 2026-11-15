# 🧪 Nebula ID Test Scenario Matrix

> Applicable version: the Nebula ID **0.2.x workspace** (Rust edition 2021, single package `nebulaid`).
> Written against (read-only verification): `Cargo.toml [features]` and `[[bench]]`, the `src/core/tests/mod.rs` module registry, the `tests/` directory, `.github/workflows/ci.yml`, `lefthook.yml`, `scripts/_coverage_gate.sh`, and `CHANGELOG.md`.
> All module-level test counts were verified with `grep -c '#\[test\]|#\[tokio::test'`.

[🏠 Home](../../README.md) • [Architecture](ARCHITECTURE.md) • [Performance Guide](PERFORMANCE.md) • **In-depth plan**: [Master Test Plan](TEST_PLAN.md) (module interaction analysis · exhaustive scenarios · feature combinations · execution plan)

---

## 📋 Table of Contents

- [Reading Conventions](#reading-conventions)
- [Test Layering Overview](#test-layering-overview)
- [Scenario Matrix](#scenario-matrix)
- [Shell End-to-End Scripts](#shell-end-to-end-scripts)
- [Commands and Gates](#commands-and-gates)
- [Statistics Summary](#statistics-summary)
- [Related Documents](#related-documents)

---

## Reading Conventions

- **Numbering**: `<domain>-<serial>` (e.g. `ALG-01`); domain prefixes are shown in each scenario-matrix table.
- **Coverage location**: points to a test file / module that actually exists; test function names are never invented — for function-level details, consult the referenced file.
- **Feature switch**: `--all-features` is buildable (the sqlite feature was removed; dbnexus's embedded/server exclusivity no longer triggers), and every run command in the matrix uses that switch.

---

## Test Layering Overview

| Layer | Location | Scale (as of the 0.3.0-rc.3 workspace) | How to run |
|-------|----------|----------------------------------------|------------|
| Inline unit tests | `#[cfg(test)]` in each `src/` module | ~1779 test functions | `cargo test --package nebulaid` |
| E2E test modules | `src/core/tests/` (11 test modules + `mod.rs`) | 220 test functions | `cargo test --package nebulaid --all-features` |
| i18n end-to-end | `tests/i18n_e2e.rs` | 1 file (the full chain: middleware → Extension → translated response) | Same as above |
| Audit-pipeline stall regression | `tests/audit_ring_stall_repro.rs` | 1 test (fills the inklog async channel, then asserts the audit `log()` does not block) | Same as above |
| Shell end-to-end | `tests/*.sh` | 4 scripts (see next section) | `./scripts/run.sh api-test` and friends |
| Criterion benchmark (i18n hot path) | `benches/i18n.rs` | 1 group (4 benchmark functions / 12 cases) | `cargo bench --bench i18n` |
| Criterion benchmark (ID-generation hot path) | `benches/algorithms.rs` | 1 group (8 benchmark functions / 10 cases) | `cargo bench --bench algorithms` |
| Criterion benchmark (Segment hot path) | `benches/segment.rs` | 1 group (4 benchmark functions / 7 cases) | `cargo bench --bench segment` |

The E2E modules inside `src/core/tests/` are organized by layer, with the following counts (`grep -c` methodology):

| Module | Test functions | Coverage domain |
|--------|:--------------:|-----------------|
| `server_layer_e2e_tests.rs` | 56 | HTTP server-layer end-to-end |
| `auth_handlers_e2e_tests.rs` | 28 | Auth handlers |
| `grpc_monitoring_e2e_tests.rs` | 40 | gRPC and monitoring |
| `supporting_layer_e2e_tests.rs` | 28 | Supporting layer (config / logging / cache assembly) |
| `infra_e2e_tests.rs` | 21 | Infrastructure (database connection / migrations) |
| `degradation_tests.rs` | 12 | Degradation chain |
| `remaining_e2e_tests.rs` | 12 | Wrap-up scenarios |
| `algorithm_e2e_tests.rs` | 12 | Algorithm end-to-end |
| `cache_tests.rs` | 5 | Cache |
| `integration_tests.rs` | 4 | Integration smoke |
| `segment_monitoring_e2e_tests.rs` | 2 | Segment monitoring |

---

## Scenario Matrix

### Algorithms (ALG)

| ID | Scenario | Verification points | Coverage location |
|----|----------|---------------------|-------------------|
| ALG-01 | Segment normal minting | Monotonic increase for the same workspace/group/biz_tag | `src/core/tests/algorithm_e2e_tests.rs` |
| ALG-02 | Segment missing-repository guard | Returns `CoreError::ConfigurationError` when no repository is injected | Same (with the `sdk` surface tests) |
| ALG-03 | Segment double-buffer switchover | Dynamic step adjustment effective within `min_step ≤ base_step ≤ max_step` | Inline tests in `src/core/algorithm/segment.rs` |
| ALG-04 | Snowflake concurrency uniqueness | No duplicates under concurrency after `(last_timestamp, sequence)` serialized migration | Inline tests in `src/core/algorithm/snowflake.rs` |
| ALG-05 | Snowflake clock rollback | `ClockMovedBackward` triggered beyond `clock_drift_threshold_ms` | Same |
| ALG-06 | UUID v8 millisecond rollover | `test_uuid_v8_counter_resets_on_millisecond_rollover` (not flaky: same-millisecond determined by in-sample embedded timestamps) | Inline tests in `src/core/algorithm/uuid_v8.rs` |
| ALG-07 | Algorithm routing | `generate_with_algorithm` overrides `algorithm.default` per call | Inline tests in `src/core/algorithm/router.rs` |
| ALG-08 | Batch generation boundaries | `batch_generate(0)` boundary and the `max_batch_size` cap (1..=10000) | Algorithm-module inline + `validate` tests |
| ALG-09 | UUID v5 deterministic derivation | Same (namespace, name) always yields the same output; version/variant bit layout; etyma legacy anchor cross-checked against an independent Python hashlib baseline; batch equals one-by-one | Inline tests in `src/core/algorithm/uuid_v5.rs` |

### Authentication and Keys (AUTH)

| ID | Scenario | Verification points | Coverage location |
|----|----------|---------------------|-------------------|
| AUTH-01 | API key full lifecycle | Create / verify / revoke / enable-disable | `src/core/tests/auth_handlers_e2e_tests.rs` |
| AUTH-02 | Argon2id hashing and constant-time comparison | Credentials never stored in plaintext; comparison via `subtle` | auth-module inline tests |
| AUTH-03 | Rotation grace period | With `>0`, old keys work inside the window and expire lazily at the deadline; `>30 days` is rejected at startup validation (fail-fast, not clamped; `src/core/config/app_config.rs:420-427`) | Same (CHANGELOG "key rotation grace period") |
| AUTH-04 | Auth cache | Hit-first, miss falls back to DB; cached values contain no secret; `cache_ttl_seconds=0` skips assembly | Inline tests in `src/server/auth/memory_dao.rs` |
| AUTH-05 | Cache invalidation exact matching | key_ids containing `*`/`?` cannot bulk-clear other entries | Same |
| AUTH-06 | gRPC auth failure codes | Disabled/expired → `PermissionDenied`; invalid/unknown → `Unauthenticated` | `src/core/tests/grpc_monitoring_e2e_tests.rs` |
| AUTH-07 | Single-source HTTP auth-header parsing | One implementation for Basic/ApiKey parsing (`parse_authorization_header_detailed`) | server-layer inline tests |
| AUTH-08 | Single admin key guard | A second admin key is rejected on `POST /api-keys` (SQL-side `role='admin' AND enabled=true` count) | `auth_handlers_e2e_tests.rs` |
| AUTH-09 | biz-tags tenant isolation | User sees only its own workspace; Admin may filter by parameter (IDOR regression) | Same |

### Rate Limiting and Request Attribution (RATE)

| ID | Scenario | Verification points | Coverage location |
|----|----------|---------------------|-------------------|
| RATE-01 | Rate limiting genuinely mounted | 429 truly originates from the HTTP stack (not dead Extension code) | Inline tests of the rate-limit middleware in `src/server/` |
| RATE-02 | Canonical response headers | `x-ratelimit-limit` / `x-ratelimit-remaining` canonical lowercase; 429 carries `remaining: 0` | Same |
| RATE-03 | Layer ordering | Rate limiting sits inside CORS / security headers; preflight consumes no quota | Same |
| RATE-04 | Hot-update validation | `POST /config/rate-limit` reuses `Config::validate` (`burst > 10×rps` rejected) | config management tests |
| RATE-05 | Client IP attribution | With `PeerAddr`/`ConnectInfo` injected, rate-limit keys / audit IP are distinguished per peer | server-layer inline tests |

### Transport Security (TLS)

| ID | Scenario | Verification points | Coverage location |
|----|----------|---------------------|-------------------|
| TLS-01 | Missing certificate fail-fast | `tls.enabled=true` with a missing/bad-format certificate refuses startup | Inline tests in `src/server/config/tls.rs` |
| TLS-02 | min_tls_version enforcement | `tls13` rejects handshakes below 1.3 (HTTP side) | Same |
| TLS-03 | Contradictory config warns explicitly | `enabled=false` + per-port true warns as `tls_config_conflict` | Same |
| TLS-04 | Handshake cannot freeze the listener | Per-connection task + 10s handshake timeout (unauthenticated DoS regression) | server listener tests |

### Configuration (CFG)

| ID | Scenario | Verification points | Coverage location |
|----|----------|---------------------|-------------------|
| CFG-01 | Unknown-key rejection | 18 structs with `deny_unknown_fields`; a misspelled section name fails startup | Inline tests in `src/core/config/` |
| CFG-02 | Bad config fail-fast | Parse failure exits with code 1 — no fallback to `Config::default()`; only "no `--config` given and default path missing" falls back to defaults with a warn | `app_config.rs` (`resolve_startup_config`) |
| CFG-03 | Full validation-rule set | All 10 rules of `Config::validate` triggered one by one (see [Config Migration Guide · validation rules](CONFIG_MIGRATION_GUIDE.md#validation-rules)) | Inline tests in `app_config.rs` |
| CFG-04 | Non-ASCII config does not panic | Config files with Chinese comments/values parse without panic (byte-slice regression) | `load_from_file_with_non_ascii_comments_does_not_panic` |
| CFG-05 | Grace-period two-column auto migration | Startup `ADD COLUMN IF NOT EXISTS` idempotent backfill; failure aborts startup | Inline tests in `src/core/database/connection.rs` |
| CFG-06 | Dual env-var mechanism | `Config::load_from_env` override + `${VAR}` expansion | Inline tests in `app_config.rs` |

### i18n and Observability (OBS)

| ID | Scenario | Verification points | Coverage location |
|----|----------|---------------------|-------------------|
| OBS-01 | Accept-Language full chain | header → `locale_middleware` → `Extension<Locale>` → translated response | `tests/i18n_e2e.rs` |
| OBS-02 | Locale key-set alignment | `locales/{en,zh}/messages.ftl` key sets identical (compared after the `-`→`.` mapping; guard test) | i18n-module inline tests |
| OBS-03 | Per-algorithm percentiles | p50/p99/p999 from a 1024-sample ring buffer (real percentile methodology) | Inline tests in `src/core/types/metrics.rs` |
| OBS-04 | Clock rollback observable | `clock_backwards` genuinely counted to drive alerts (not "any delayed sample counts") | `grpc_monitoring_e2e_tests.rs` |
| OBS-05 | Health and metrics endpoints | Semantics of `/health`, `/health/sdforge`, `/metrics` | `server_layer_e2e_tests.rs` |

### Distribution and Degradation (DST)

| ID | Scenario | Verification points | Coverage location |
|----|----------|---------------------|-------------------|
| DST-01 | Degradation chain fallback | `[Snowflake, UuidV8]` deduplicated, takes over in order | `src/core/tests/degradation_tests.rs` |
| DST-02 | etcd coordination (feature `etcd`) | Worker allocation / distributed-lock exclusivity, release and re-entry, TTL expiry (real etcd, `#[ignore]`); `endpoints=[]` falls back to `LocalDistributedLock`; endpoints configured in a non-etcd build refuse startup | coordinator inline tests + `make test-etcd` |
| DST-03 | Shutdown leaks no background tasks | `rate_limit_cleanup` and degradation sweeps are reclaimed on every exit branch | server-layer inline tests |

### SDK and Examples (SDK)

| ID | Scenario | Verification points | Coverage location |
|----|----------|---------------------|-------------------|
| SDK-01 | Kit assembly validation | Missing dependencies / cycles in the trait-kit AsyncKit graph fail in `build()` | Inline tests in `src/sdk/kit.rs` (feature `sdk`) |
| SDK-02 | Pure algorithms, zero DB | `Config::default()` + snowflake/uuid_v8 can mint IDs | Inline tests in `src/sdk` |
| SDK-03 | No error leakage | `to_api_error` returns a fixed summary + `error_id` externally | Inline tests in `src/sdk` |
| SDK-04 | Examples build | `embedded` / `sdk_server` compile gated by `required-features` | CI test job (`--all-features` leg) |
| SDK-05 | Segment repository wiring | After `with_repository(..)`, generated values come from the injected repository's segments (MockDatabase differentiated assertions) | Inline tests in `src/sdk/kit.rs` |
| SDK-06 | Non-etcd endpoint guard | Endpoints configured without the etcd feature → `build()` returns `ConfigurationError` | Inline tests in `src/sdk/kit.rs` (CI sdk trim leg) |

---

## Shell End-to-End Scripts

| Script | Lines | Scenarios | Prerequisites |
|--------|------:|-----------|---------------|
| `tests/api_test.sh` | 1037 | API endpoint regression (auth, minting, management plane) | Service running; `./scripts/run.sh api-test [server_url]` |
| `tests/degradation_test.sh` | 326 | Degradation behavior under dependency failures | Prepare dependencies per the script header |
| `tests/distributed_test.sh` | 495 | HTTP minting concurrency / ordering / batch consistency (no etcd required) | Service running; User key (minting) + Admin key (algorithm switching, optional); workspace exists |
| `tests/db_concurrency_test.sh` | 302 | Database concurrency correctness | PostgreSQL available |
| `tests/lib.sh` | 591 | Shared assertion/utility library for the scripts above | — |

---

## Commands and Gates

```bash
# Full test run (CI matrix runs default / all (--all-features))
cargo test --package nebulaid --all-features

# Light compile check (same as the CI fmt-clippy job: no-default paths still compile)
cargo check --package nebulaid --no-default-features --lib --bins

# Coverage gate: CI ≥ 95% (default leg, excluding server/proto/ generated code)
cargo llvm-cov --package nebulaid \
  --fail-under-lines 95 --ignore-filename-regex "server/proto/"

# Local gates: pre-commit (fmt + clippy + gitleaks) and pre-push (test + coverage ≥ 90%)
./scripts/run.sh pre-commit

# Benchmark
cargo bench --bench i18n
```

Gate ownership: the CI five stages (fmt-clippy → deny → audit → test matrix → gate) live in `.github/workflows/ci.yml`; the pre-commit / pre-push hooks live in `lefthook.yml`; the pre-push coverage threshold is ≥ 90% (`scripts/_coverage_gate.sh`), and CI is ≥ 95%.

---

## Statistics Summary

As of the 0.3.0-rc.3 workspace (methodology: `grep -rnE '#\[(tokio::)?test\]' --include='*.rs' src tests | wc -l`):

| Category | Count |
|----------|------:|
| Rust test functions total | ~2006 |
| ├─ `src/` inline unit tests | ~1779 |
| ├─ `src/core/tests/` E2E modules | 220 |
| └─ `tests/i18n_e2e.rs` | 7 (full-chain i18n) |
| Shell end-to-end scripts | 4 (plus 1 shared library) |
| Criterion benchmark groups | 3 (i18n: 4 functions / 12 cases; ID-generation hot path: 8 functions / 10 cases; Segment: 4 functions / 7 cases) |

Coverage: the CI gate is line coverage ≥ 95% (`ci.yml`), and the pre-push local gate is ≥ 90% (`_coverage_gate.sh`); actual line coverage at the v0.2.0 release was 89.91% (gate values are floors, not current values). Historical methodology: the v0.2.0 release reported 1829 e2e cases (see the [CHANGELOG](CHANGELOG.md)); later evolution follows the grep methodology in the table above.

---

## Related Documents

- [Performance Guide](PERFORMANCE.md): benchmark methodology and hot paths
- [Security](SECURITY.md): supply-chain and gate checklist
- [Contributing Guide](CONTRIBUTING.md): testing standards and commit requirements
- [CHANGELOG](CHANGELOG.md): test-related change records
