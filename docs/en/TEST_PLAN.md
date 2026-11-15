# 📐 NebulaId Master Test Plan: Module Analysis · Exhaustive Scenarios · Feature Combinations · Execution Plan

> Applicable version: the NebulaId **0.2.x workspace** (Rust edition 2021, single package `nebulaid`).
> Written against (read-only verification): all `src/` module sources, the 12 files in `src/core/tests/` (11 E2E test modules + `mod.rs`), the `tests/` scripts, 11 documents in `docs/`, `docker/`, `scripts/`, `.github/workflows/ci.yml`, and `protos/nebula_id.proto`.
> Relationship with [TEST_SCENARIOS.md](TEST_SCENARIOS.md): that document is the **scenario-matrix overview** (what is covered); this document is the **in-depth plan** — module interaction analysis, normal+abnormal scenario exhaustion, feature-combination behavior differences, and each scenario's prerequisites/steps/expectations/verification methods. Their numbering systems are independent.

[🏠 Home](../../README.md) • [Architecture](ARCHITECTURE.md) • [Scenario Matrix](TEST_SCENARIOS.md) • [API Reference](API_REFERENCE.md)

---

## 📋 Table of Contents

- [Reading Conventions](#reading-conventions)
- [1. Functional Module Analysis and Interactions](#1-functional-module-analysis-and-interactions)
- [2. Feature Combination Analysis](#2-feature-combination-analysis)
- [3. Exhaustive Scenarios and Test Cases](#3-exhaustive-scenarios-and-test-cases)
- [4. Cross-Domain Fault Injection](#4-cross-domain-fault-injection)
- [5. Test Execution Plan](#5-test-execution-plan)
- [6. Gap Summary and New-Test Priorities](#6-gap-summary-and-new-test-priorities)

---

## Reading Conventions

- **Scenario numbering**: `<domain>-<serial>`. Domain prefixes: CFG config / ALG algorithms / DEG degradation and circuit breaking / AUTH auth and authorization / IDGEN minting API / RES resource management / RATE rate limiting / GRPC / OBS observability and audit / TLS transport security / COORD distributed coordination / DB database / I18N internationalization / SEC security hardening / SDK embedded / LIFE lifecycle / FAULT fault injection.
- **Status markers**: ✅ already covered by automation (location attached); ⚠️ partially covered or the coverage method is flawed; ❌ gap (Section 6 gives new-test suggestions).
- **Feature switch**: `--all-features` **is buildable** (the sqlite feature was removed; dbnexus's embedded/server exclusivity no longer triggers; = default + etcd + alerting + sdk + integration-tests + openapi). CI, hooks, and this document uniformly adopt that switch; `--no-default-features` gets a separate light compile check (compile only, no tests; a real PostgreSQL is needed at runtime).
- ~~**Baseline exemption**~~ (obsolete, fixed 2026-09-18): the clean baseline (2026-09-17, since c3bc165) once had **7 failing tests** — 6 `core::database::connection::tests::test_run_migrations_*` + 1 `infra_e2e_tests::e2e_database_run_migrations_creates_tables`, because the mock quotas were not updated along with the growth of the migration statement list (enum types +2). Fixed together with the segment unique-constraint backfill statement; **the full suite now has 0 failures**.

---

## 1. Functional Module Analysis and Interactions

### 1.1 Layered Overview

```
┌─────────────────────────── Presentation Layer src/server/ ────────────────────────────┐
│ router.rs (route assembly + Prometheus bridge + middleware order)                     │
│ handlers/ (system·workspace·biz_tag·id·api_key)  grpc.rs (5 RPCs)                     │
│ middleware/ (request_id→size_limit→locale→api_key_auth)                               │
│ rate_limit/  audit/  auth/ (garrison cache)  config/ (cors·hot_reload·tls)            │
│ api_version.rs  models.rs (error envelope)  openapi.rs  sdforge_adapter.rs            │
┌───────────────────────────────────────────▼───────────────────────────────────────────┐
│                           Core Layer src/core/                                        │
│ algorithm/ (router·segment·snowflake·uuid_v8·degradation_manager·traits)              │
│ coordinator/ (etcd│local: worker allocation · distributed lock · cluster health)      │
│ database/ (connection·migrations·4 domain repos)  config/ (13 sections+22 validations)│
│ auth/ (KeyHasher→Argon2id)  monitoring/ (alerting subsystem, gated)  i18n.rs          │
│ types/ (CoreError 24 variants·Id·Metrics)                                             │
┌───────────────────────────────────────────▼───────────────────────────────────────────┐
│ Infrastructure (self-developed ecosystem, all via crates.io registry, no path deps):  │
│ dbnexus(PostgreSQL) · oxcache(cache) · confers(config/interpolation/watch)            │
│ sdforge(HTTP/gRPC/OpenAPI macros) · inklog(logs/masking) · limiteron(token bucket)    │
│ garrison(auth cache) · trait-kit(sdk only) · etcd-client(etcd only)                   │
└───────────────────────────────────────────────────────────────────────────────────────┘
```

### 1.2 Core Module Inventory and Responsibilities

| Module | Key files (lines) | Responsibilities | High-value internal behaviors |
|---|---|---|---|
| Algorithm routing | `core/algorithm/router.rs` (2364) | Algorithm registry, per-biz_tag routing, per-call override, fallback chain, observability | Primary algorithm failure → walk the chain → on total failure return the **primary algorithm's original error**; initialize logs a warn (without aborting) when a single algorithm's build fails; health aggregation ranks Unhealthy above Degraded |
| Segment | `core/algorithm/segment.rs` (1780) | Segment double buffering, CPU monitoring, DC failure detector (half-finished), three Loaders | 3 retries exhausted → `SegmentExhausted`; batch size=0 also reports `SegmentExhausted` (opposite semantics to the other two algorithms); production default Loader=Unconfigured (explicit error prevents silence) |
| Snowflake | `core/algorithm/snowflake.rs` (1392) | Lock-free CAS state word `(ts<<seq)|seq`, clock rollback + drift decay | Per-millisecond capacity = mask (the mask itself is never minted); rollback > threshold → `ClockMovedBackward{state-word ts}`; drift beyond 60s decays to zero; batch retries 100 times exhausted → `InternalError` (never a short batch) |
| UUID v8 | `core/algorithm/uuid_v8.rs` (480) | RFC 9562 v8 time-ordered, 21-bit counter + 26-bit random entropy | Reuses Snowflake's `clock_drift_threshold_ms` (a coupling point); rollback payload = the **drift delta** (different semantics from Snowflake); dc/worker silently truncated |
| Degradation and circuit breaking | `core/algorithm/degradation_manager.rs` (2113) | Circuit-breaker state machine Closed→Open→HalfOpen, degradation/recovery, audit linkage | In sweep mode, `failure_threshold` counts by **sweep rounds**, not requests; degraded algorithms skip health checks; `auto_recovery`/`recovery_check_interval_ms` are dead config |
| Coordinator | `core/coordinator/etcd.rs` (3154, gated) / `local.rs` (583) | worker_id lease allocation, distributed lock, etcd cluster health monitoring + file cache | etcd: fail-stop (3 consecutive renew failures → triggers shutdown, preventing dual-primary duplicate IDs); release validates ownership to prevent mis-deletion; local: no lease, always returns the configured worker_id (**multi-instance must collide**), lock TTL ignored |
| Configuration | `core/config/app_config.rs` + 13 section files | Aggregate root `Config` (all 13 sections `deny_unknown_fields`), 22 validations, env interpolation/override, merge, startup decision matrix | After env override, merge **resets** the `algorithm.segment/snowflake/uuid_v8` sub-tables to defaults, keeping only `default`; `key_rotation_grace_period_seconds` deliberately stays out of merge |
| Database | `core/database/` (connection + 4 domain repositories) | Connection pool, idempotent migrations (schema `nebula_id`, 2 enums, 5 tables), grace-period two-column auto-backfill | Segment allocation = single-statement `UPDATE…RETURNING` + `ON CONFLICT` retry (PG row locks guarantee concurrency); statement timeout maps to `TimeoutError` (not DatabaseError) |
| Auth core | `core/auth/key_hasher.rs` | `KeyHasher` trait + Argon2id (m=19456,t=2,p=1) | Key material = `{salt}|{key_id}:{key_secret}`, single source; verify returns false (not Err) on malformed hashes |
| Monitoring | `core/monitoring/core.rs` (gated) + the `server/router.rs` Prometheus bridge | Alert rules/state machine/notification channels (SSRF protection)/broadcast event stream; the /metrics sampling-render bridge | Zero production references, **always compiled in test builds** (needed by the alerting e2e imports); webhook SSRF known residue: `https://[::1]` can bypass |
| HTTP service | `server/router.rs` (3936) + handlers | Route assembly, middleware stack, error envelope, rate-limit mounting, security headers | Middleware order (outer→inner): size_limit(main layer)→request_id→sdforge context→audit→CORS→security headers ×6→rate limit→routing→locale→api_version→auth→role guards |
| gRPC | `server/grpc.rs` + `proto/` | 5 RPCs (Generate/BatchGenerate/BatchGenerateStream/Parse/HealthCheck) | Shares `ApiKeyAuth` and the failure bucket with HTTP; gRPC-only `classify_miss` (disabled/expired → PermissionDenied, others → Unauthenticated with the same copy to prevent probing); count validation precedes stream consumption |
| Rate limiting | `server/rate_limit/` | limiteron token bucket, 16 shards, 5min idle reclamation | Key priority workspace_id→IP→"anonymous", but production assembly **never injects the workspace_id extension** (in practice everything keys by IP); limiter internal errors fail open |
| Audit | `server/audit/` | In-memory ring + optional file persistence (dedicated writer task), PII redaction | Audit failures **never block requests**; file-open failure falls back to the in-memory ring; paths with `..` traversal rejected |
| Hot reload | `server/config/hot_reload.rs` | `ArcSwap<Config>` atomic config swap + confers FsWatcher (debounce = interval) | Parse/read failure → `Ok(false)`, **keeping the old config**; the change diff writes only 5 fields to audit |
| SDK | `sdk/kit.rs` (1321, gated) | AsyncKit four-module assembly (lock→repository→router→minting), dependency-graph validation, health aggregation | etcd connection failure fails build outright (fail-closed, no local-lock fallback); Segment without a repository errors explicitly |
| i18n | `core/i18n.rs` + `server/middleware/locale.rs` | Compile-time embedded en/zh-CN, RFC 7231 negotiation | Missing key → en → still missing → return the key name (never empty); locale must not be used for auth decisions |

### 1.3 HTTP Request Lifecycle (the backbone tests must understand)

```
TCP(TLS?) → size_limit(1MiB→413) → request_id(UUIDv7 passthrough/mint) → sdforge context
→ audit middleware (after response, records Success/Failure/Partial by status) → CORS(ALLOWED_ORIGINS env)
→ security headers ×6(nosniff/DENY/CSP/HSTS/XSS/Referrer) → rate limit (optional, 429 special body + x-ratelimit-* headers)
→ route match ┬ public: /health /ready /metrics /api-docs/openapi.json /health/sdforge /swagger-ui /api/v1(info)
           └ /api/v1 → locale(Accept-Language) → api_version(v2→400)
              ├ admin group: auth_middleware → admin_required(non-Admin→403)
              └ authenticated group: auth_middleware → anonymous_block(Anonymous→401 fail-closed)
                 → handler → core_error_classification single-table error-envelope mapping
envelope: {code, business_code(4-digit string), message(i18n+5xx sanitized), details, request_id, timestamp(ms)}
```

Key invariants (each should be pinned by a test):

1. Auth-exempt routes (health/ready/metrics/openapi/info) **still pass through** rate limiting, audit, security headers, and request_id.
2. OPTIONS preflight short-circuits at the CORS layer, **consuming no rate-limit quota**, and a 429 response always carries CORS headers (rate limiting sits inside CORS).
3. Auth middleware 401/429 responses are **bare JSON** (not the envelope); handler-level errors are envelopes.
4. 5xx messages are sanitized into fixed i18n copy; the original error goes to tracing logs only (CWE-209).
5. `Authorization` header parsing has a single repo-wide implementation, `parse_authorization_header_detailed` (shared by HTTP/gRPC); `Bearer` is always rejected.

### 1.4 gRPC Chain

Per RPC: `authenticate` (shared failure bucket, 10 per 5min per IP → ResourceExhausted) → `authorize_namespace` (Admin → minting denied; User cross-tenant → PermissionDenied; unknown ns → NotFound) → business → `core_error_grpc_code` mapping. `BatchGenerateStream` validates request initialization before consuming the stream, and an in-stream rejected item terminates the whole stream with a Status.

### 1.5 Startup/Shutdown Lifecycle (main.rs)

```
init_observability → init_sdforge(prevents inventory stripping) → load_config(fail-fast matrix)
→ production-enforced TLS validation (exit 1, escape hatch NEBULA_ALLOW_INSECURE_TLS=1)
→ init_repository(DB connect fail exit1→migration fail exit1→[etcd]fail-closed assembly→missing salt panic)
→ init_auth_stack(garrison cache[ttl>0]→ApiKeyAuth[no DB exit1]→key loading→audit→hot reload)
→ run_servers: [etcd] health patrol + worker lease (fail exit1) → degradation patrol background → spawn HTTP+gRPC
→ select! four arms: HTTP end/GRPC end/shutdown signal(SIGINT·SIGTERM)/[etcd] lease renewal fail(→fail-stop)
→ teardown: stop patrols→abort rate-limit cleanup→[etcd] stop keepalive + release worker_id
```

---

## 2. Feature Combination Analysis

### 2.1 Feature Gate Panorama

| Feature | What it gates | Default | Notes |
|---|---|---|---|
| `postgresql` | `dbnexus/postgres` | ✅ | default member; can be turned off with `--no-default-features` (compile-only guarantee; a real PostgreSQL is needed at runtime) |
| `http` / `grpc` / `openapi` | sdforge macro-expansion mirror features (empty) | http/grpc ✅ | Without `openapi`, `#[forge]`'s OpenAPI route info is not registered |
| `garrison-auth` | garrison taking over verification + `AuthCache` wiring (`server/auth/cache.rs`, the `api_key_auth.rs` cache path, main.rs assembly) | ✅ | When off, the hand-written Argon2id direct-lookup path runs (no cache) |
| `etcd` | all of `core/coordinator/etcd.rs`, the router's etcd health-monitor wiring, main.rs coordination runtime and the fail-stop select arm | ❌ | When on: worker_id lease allocation, distributed lock, health sweeps + file cache |
| `alerting` | the `core/monitoring/core.rs` alerting subsystem (AlertManager/rule state machine/notification channels/event stream) | ❌ | **Always included in test builds** (`cfg(any(test, feature))`); zero production references |
| `integration-tests` | gates `#[ignore]` tests that need a real database (8 repository + 1 etcd) | ❌ | Needs real PostgreSQL / etcd |
| `sdk` | `src/sdk/` (kit.rs, 1211 lines) + the trait-kit dependency, implies `openapi` | ❌ | Zero dependency-tree impact when off |

### 2.2 Buildable Combination Matrix and Behavior Differences

| Combination | Buildable | CI coverage | Key behavior differences |
|---|---|---|---|
| `default` | ✅ | ✅ test job | LocalDistributedLock + static worker_id + startup warn; garrison cache effective |
| `--all-features` | ✅ | ✅ test job (all leg) | Covers etcd lease-allocated worker_id, TTL-carrying locks, cluster health monitoring + cache degradation, lease failure triggering shutdown; also compiles the sdk and both examples, alerting, and the integration-tests gated tests |
| `--no-default-features` | ✅ | ✅ fmt-clippy job (compile only) | The postgres driver can be turned off; the garrison hand-written Argon2id fallback path and the mirror-feature-off path participate in compilation; no tests run |
| `etcd,integration-tests` | ✅ | ❌ (manual) | The 8+1 real-DB `#[ignore]` tests become executable |
| `default,alerting` | ✅ | ❌ (covered by the all leg) | Production binary grows; runtime behavior identical to default (zero production-path references) |

### 2.3 Differences of the Same Code Path Under Different Features (tests must verify separately)

| Difference point | etcd off | etcd on |
|---|---|---|
| worker_id source | static `Config.app.worker_id` | etcd lease allocation (1..=255, skipping the 0 sentinel) |
| Distributed lock | `LocalDistributedLock` (in-process HashMap, TTL ignored) | `EtcdDistributedLock` (CAS+lease, Drop releases in background) |
| Startup fail-closed | none (missing endpoints only warns) | endpoints configured but unreachable → exit 1 |
| Health degradation | `EtcdClusterHealthMonitor` stub (always Failed, set_status no-op) | Real ping: 3 consecutive failures Degraded / 5 consecutive failures Failed+cache mode |
| Shutdown select arm | lease-failure arm always pending | 3 consecutive renewal failures → **proactive shutdown** (fail-stop prevents duplicate IDs) |

| Difference point | garrison-auth off | garrison-auth on (default) |
|---|---|---|
| Verification path | Argon2id direct DB lookup every time | AuthCache checked first (hits skip DB+Argon2), miss falls back to source |
| Grace-period credentials | prev_hash verified every time | **never written to the cache** (preventing the TTL from indirectly extending the rotation window) |
| Revocation propagation | immediate (direct lookup) | up to one TTL (default 300s); handlers proactively invalidate the cache on revoke/rotate |

---

## 3. Exhaustive Scenarios and Test Cases

> Each table lists: ID | Scenario (prerequisites) | Steps | Expected | Verification | Status.
> All cargo commands in "Verification" respect the feature constraints (CI methodology is `--all-features`).

### 3.1 CFG — Configuration Management Domain

**Normal flows**

| ID | Scenario (prereq) | Steps | Expected | Verification | Status |
|---|---|---|---|---|---|
| CFG-01 | No config file and no explicit path | Start the service | Falls back to `Config::default()` + warn (DefaultsBecauseMissing), startup succeeds | Delete the file and start; observe logs and `/health` | ✅ app_config unit test |
| CFG-02 | Valid `config/config.toml` | Start with `--config` | All 13 sections load, validate passes | Start + spot-check `/api/v1/config` | ✅ supporting_layer_e2e |
| CFG-03 | File contains `${VAR}` and `${VAR:default}` | Load with/without VAR set | Interpolation applies; missing vars keep the literal | Unit test `expand_env_vars` | ✅ app_config |
| CFG-04 | Environment variable overrides | Set `APP_HTTP_PORT/DC_ID/DATABASE_URL/ETCD_ENDPOINTS` (comma-split) etc., then start | Values override the file; illegal values → `InvalidValue` naming the variable | Unit test `load_from_env` | ✅ app_config |
| CFG-05 | env+file merge (hot-reload path) | After `merge`, inspect the algorithm sub-tables | `segment/snowflake/uuid_v8` sub-tables **are reset to defaults**, only `default` survives; the grace-period field stays out of merge | Unit test merge priority | ✅ (FAQ records this behavior) |
| CFG-06 | `hot_reload.auto_watch_enabled=true` | Modify the config file after startup | Reload succeeds after the 2s debounce, callback fires, diff written to audit | e2e: temp file + watch | ✅ hot_reload inline |
| CFG-07 | Admin authenticated, rate limiting enabled | `POST /api/v1/config/rate-limit` with legal values | 200 success:true; **the running limiter synchronizes immediately** (old buckets cleared) | Load-test immediately after the HTTP call and observe the new thresholds | ✅ server_layer_e2e |
| CFG-08 | Admin authenticated | `POST /api/v1/config/logging` with a legal level | 200; adjusts the inklog level in real time (observe the log change immediately) | HTTP + log assertions | ✅ |
| CFG-09 | Admin authenticated | `POST /api/v1/config/reload` | Re-reads `config/config.toml` successfully | HTTP | ⚠️ smoke only (router.rs:2376) |
| CFG-10 | A biz_tag exists | `POST /api/v1/config/algorithm` binding `biz_tag→snowflake` | success:true; subsequent minting for that tag uses snowflake | HTTP + verify ID shape | ✅ api_test.sh / e2e |
| CFG-11 | Any authenticated user | `GET /api/v1/config` | Returns the sanitized config: **no database section**, TLS as booleans only | HTTP + JSON assertions | ⚠️ no role-branch test |
| CFG-12 | Startup decision matrix, four cells | Test: explicit path missing / default path missing / file permission denied / bad TOML | Err(FileNotFound) / default fallback / Err(FileError) / Err(InvalidValue); all but the second exit | Unit test `resolve_startup_config` | ✅ all 8 cells covered |

**Validation rejection branches (22 items; uniform method: build TOML → load → assert the rejection message and exit code 1; `Config::validate` unit tests cover all ✅)**

| # | Branch | # | Branch |
|---|---|---|---|
| 1 | `http_port=0` | 12 | `algorithm.default` ∉ {segment,snowflake,uuid_v8} |
| 2 | `grpc_port=0` | 13 | `segment.min_step > max_step` |
| 3 | `shutdown_timeout_seconds=0` | 14 | `base_step` outside [min,max] |
| 4 | `dc_id > 31` | 15 | `switch_threshold` outside [0,1] |
| 5 | `max_connections=0` | 16 | snowflake's three u32 bit widths sum ≥64 (wrap prevention) |
| 6 | `min > max_connections` | 17 | `clock_drift_threshold_ms=0` |
| 7 | `acquire_timeout_seconds=0` | 18 | `batch_generate.max_batch_size=0` |
| 8 | `idle_timeout_seconds=0` | 19 | `max_batch_size > 10000` |
| 9 | Rate limiting on and `default_rps=0` | 20 | Grace period > 30 days (fail-fast, not clamped) |
| 10 | Rate limiting on and `burst_size=0` | 21 | `api_key_salt`/`key_secret` containing unexpanded `${` literals |
| 11 | `burst_size > 10×rps` (saturating wrap prevention admits it) | 22 | Unknown keys/sections (deny_unknown_fields), duplicate sections, TOML syntax errors, missing required fields |

**Abnormal flows**

| ID | Scenario (prereq) | Steps | Expected | Verification | Status |
|---|---|---|---|---|---|
| CFG-20 | Hot reload meets bad TOML | Write a syntactically broken config while the watch is active | Reload returns `Ok(false)`, **the running config is kept**, error log | e2e: write a bad file → assert the old config still applies | ✅ hot_reload |
| CFG-21 | Watch establishment fails | Watcher initialization error (e.g. unreadable directory) | Rebuild retried at intervals, no panic | Unit test | ✅ |
| CFG-22 | Hot-update API service-layer validation failure | `POST /config/rate-limit` with `burst>10×rps` | handler-level validate → 400/3002; **service-layer** validation failure → **200-wrapped** `success:false` (the two layers differ; pin both) | HTTP with the two illegal payloads, asserted separately | ⚠️ no explicit test for the service-layer 200 wrap |
| CFG-23 | Non-admin calls the hot-update API | User key calls `POST /config/*` | 403 admin_required | HTTP | ✅ admin middleware tests |
| CFG-24 | Grace-period hot update | Attempt to change `key_rotation_grace_period_seconds` at runtime | The field stays out of merge/hot-reload (a deployment-time decision); takes effect after restart | Unit-test pin | ✅ merge unit test |
| CFG-25 | `NEBULA_ENV` missing or illegal | Start without the variable | Treated as **production** (fail-closed): forced TLS validation, salt validation, CORS tightening; `FromStr` unknown value → Err | Unit test environment | ✅ |

### 3.2 ALG — Algorithm Core Domain

**Segment**

| ID | Scenario (prereq) | Steps | Expected | Verification | Status |
|---|---|---|---|---|---|
| ALG-01 | DbSegmentLoader injected (test Loader) | Mint consecutively for the same workspace/group/biz_tag | Monotonic increase, non-overlapping ranges | `algorithm_e2e_tests` + inline | ✅ |
| ALG-02 | Consumed below `switch_threshold` remaining | Keep minting to trigger a segment switch | Next segment preloaded in background, minting uninterrupted | Inline DoubleBuffer tests | ✅ |
| ALG-03 | Segment exhausted and 3 retries fail | Simulate a persistently failing loader | `SegmentExhausted{max_id}` | Inline | ⚠️ the exhaustion path depends on concurrent injection; the spin-wait branch is untested directly |
| ALG-04 | No repository injected | `generate` | `ConfigurationError` (hinting to assemble DbSegmentLoader), **no panic, no silence** | sdk-surface e2e + inline | ✅ |
| ALG-05 | `batch_generate(size=0)` | Call | **`SegmentExhausted`** (note: opposite semantics to Snowflake/UUID returning an empty batch — an existing pinned behavior) | Inline segment.rs:1040 | ✅ |
| ALG-06 | DB returns a negative current_id (dirty data) | load_segment | Defensively converges to 0, no bitwise wrap | Inline DbSegmentLoader | ✅ |
| ALG-07 | Loader raises `DatabaseError` | generate/batch | Error passed through as-is (not swallowed or rewritten) | Inline | ✅ |
| ALG-08 | No buffers at all / most recent load failed | `health_check` | Degraded("No active buffers") / Degraded("Last segment load failed"); a success after failure restores Healthy | Inline, 4 branches | ✅ |
| ALG-09 | No requests happened | `metrics().cache_hit_rate` | Defaults to 1.0 (total=0 special case) | Inline | ✅ |
| ALG-10 | Concurrent single minting (multiple tasks, same key) | Concurrent generate | The CAS loading flag guarantees a single load; spinners wait | ⚠️ needs a concurrency stress test (the shell db_concurrency has a black-box version) | ⚠️ |
| ALG-11 | Concurrent batch | Concurrent batch_generate | Correct behavior (status quo: the batch path has **no loading CAS**, so repeated load_segment is possible — wasted segments/DB pressure) | Concurrency test asserting DB load counts | ❌ gap |

**Snowflake**

| ID | Scenario (prereq) | Steps | Expected | Verification | Status |
|---|---|---|---|---|---|
| ALG-20 | Normal build | Serial/concurrent minting | Monotonic, no duplicates (16 concurrent tasks) | Inline + e2e | ✅ |
| ALG-21 | Millisecond saturated | Consume up to the mask within one millisecond | Per-millisecond capacity = sequence_mask (1023; the mask itself is reserved, never minted) | Inline sequence wrap test | ✅ |
| ALG-22 | Rollback > threshold (default 1000ms) | Inject rollback | `ClockMovedBackward{last_timestamp=state-word ts}` + drift record + `clock_backwards` metric +1 | Inline | ✅ |
| ALG-23 | Rollback ≤ threshold | Inject a small rollback | Waits to catch up, then mints successfully | Inline | ✅ |
| ALG-24 | `sequence_bits=0` | Build then mint | `SequenceOverflow{timestamp}` short-circuits (prevents idle CAS spinning) | Unit test | ❌ gap (branch has no direct test) |
| ALG-25 | Batch under pressure | Large batch_generate | 100 retries exhausted → `InternalError` (with generated/requested details), **never a short batch** | Inline | ✅ |
| ALG-26 | `batch_generate(0)` | Call | Empty `IdBatch` Ok (opposite of Segment) | e2e + inline | ✅ |
| ALG-27 | Idle >60s after rollback (drift_decay) | Inject rollback → advance the monotonic clock → mint | Drift decays to zero, health recovers; a new rollback event blocks decay | Inline, three cases | ✅ |
| ALG-28 | drift == threshold (boundary) | Inject exactly | Health **still Healthy** (only strictly greater is Unhealthy) | Inline boundary | ✅ |
| ALG-29 | dc/worker beyond the bit width | `worker_id=300` (8-bit layout) | Status quo: **no validation; bit pollution of high bits** (validate does not check worker_id against the width either) | A pinning test exposes the behavior | ❌ gap |

**UUID v8**

| ID | Scenario (prereq) | Steps | Expected | Verification | Status |
|---|---|---|---|---|---|
| ALG-30 | Multi-node minting | 4 nodes ×1000 | Globally unique + time-ordered + version nibble=8 + embedded timestamp ±1s | Inline | ✅ |
| ALG-31 | Rollback > threshold | Inject rollback | `ClockMovedBackward{last_timestamp=**drift delta**}` (payload semantics differ from Snowflake; assertions must be handled separately) | Inline | ✅ |
| ALG-32 | Millisecond rollover | Inject last_ts=now-10 | Counter resets to a new random start | Inline (not flaky: judged by in-sample embedded timestamps) | ✅ |
| ALG-33 | dc/worker beyond width | dc=8/worker=300 | **Silent truncation** (`& MAX`) — behavior pinned | Unit test | ❌ gap |
| ALG-34 | batch size=0 / 500 | Call | Empty Ok batch / all unique | Inline | ✅ |

**Algorithm routing and factory**

| ID | Scenario (prereq) | Steps | Expected | Verification | Status |
|---|---|---|---|---|---|
| ALG-40 | Default Segment | Build the Router | fallback chain=[Snowflake,UuidV8]; default Snowflake→[UuidV8]; default UuidV8→empty chain | Inline dedup/build tests | ✅ |
| ALG-41 | biz_tag bound to an algorithm (CFG-10) | generate(ctx with tag) | per-biz_tag override takes precedence over the config default | e2e ALG routing test | ✅ |
| ALG-42 | `generate_with_algorithm` | Pin the algorithm per call | Overrides the default, effective for this call only | Inline + e2e | ✅ |
| ALG-43 | Primary fails, first chain member available | Inject primary failure | fallback succeeds + degradation counting/audit linkage | router inline, 6 combination types + degradation_tests | ✅ |
| ALG-44 | Primary fails and the whole chain fails | Inject failure everywhere | Returns the **primary algorithm's original error** (not "All algorithms failed") | Inline | ✅ |
| ALG-45 | Specified algorithm unregistered | `generate_with_algorithm(un-built algorithm)` | Walks the fallback; empty chain → `InternalError("All algorithms failed")` | Inline | ✅ |
| ALG-46 | initialize single-algorithm build failure | Factory injection failure | **Only warn, errors collected, the rest stay usable**; total failure → `InternalError("No algorithms available")` | Factory failure injection | ❌ the partial-failure branch has no test |
| ALG-47 | Health aggregation | Mixed health states across algorithms | Unhealthy > Degraded > Healthy priority; all empty → Unhealthy | Inline, 4 branches | ✅ |
| ALG-48 | shutdown | Any algorithm's shutdown fails | error log but continues other algorithms, returns no Err | Inline, 3 branches | ✅ |
| ALG-49 | Production bootstrap of the Segment Loader | Default algorithm=Segment, no explicit repository (production build) | Process-level OnceCell bootstrap (success or failure tried once only; failure warns and minting errors explicitly) | ⚠️ needs a real DB (no test for the bootstrap-failure branch) | ⚠️ |

**Types and errors (types/)**

| ID | Scenario | Expected | Verification | Status |
|---|---|---|---|---|
| ALG-50 | `Id::from_string` 6 branches (UUID/numeric/overflow 2^128/whitespace/36-char non-UUID fallback) | Each branch correct or `InvalidIdString(original)` | Inline | ✅ |
| ALG-51 | `AlgorithmType::from_str` nine-string mapping (uuid_v8 group→UuidV8; uuid_v7 group→UuidV7, uuid_v4 group→UuidV4, the add-uuid-v4-v7 breaking reversal) + illegal values | Mapping applies; illegal → `InvalidAlgorithmType` | e2e + inline | ✅ |
| ALG-52 | `Id::Display` version-nibble sniffing | bits76-79==4/7/8 render as UUID, otherwise numeric — **numeric IDs whose high bits happen to hit the version nibble are mis-rendered** (inherent ambiguity) | Pinning test | ❌ gap |
| ALG-53 | `CoreError` 24 variants → HTTP code/business_code/i18n (en+zh-CN) | Single-table mapping all correct (see SEC-30 sanitization in 3.14) | helpers all-variant matrix + error.rs Display tests | ✅ |
| ALG-54 | `ErrorSource` chain preservation | `DatabaseError`/`EtcdError`/`IoError` underlying cause preserved via source(); top-level copy and i18n share one source | Chain-preservation test | ✅ |
| ALG-55 | Latency ring 1024 slots, percentiles, 8-thread concurrent writes | Wrapping correct, p50/p99/p999 nearest-rank, clock_backwards counted | metrics inline | ✅ |
| ALG-56 | QpsWindow second rollover/clock rollback | Second rollover double buffer correct; **`expect` panics when the clock is earlier than the epoch** (inconsistent with snowflake's fault tolerance) | Pinning of the rollback branch | ❌ gap (rollback branch) |

### 3.3 DEG — Degradation and Circuit-Breaking Domain

| ID | Scenario (prereq) | Steps | Expected | Verification | Status |
|---|---|---|---|---|---|
| DEG-01 | Consecutive failures reach `failure_threshold` (5) | record_failure | trigger_degradation + audit `algorithm_degradation` | Inline, all branches | ✅ |
| DEG-02 | After degradation, consecutive successes reach `recovery_threshold` (10) | record_success | Only a **primary** recovery recomputes current_state; non-primary recoveries leave the state alone (pinned behavior) | Inline | ✅ |
| DEG-03 | Unknown algorithm event | record_generation_result | Silent no-op | Inline | ✅ |
| DEG-04 | Full circuit-breaker lifecycle | Closed→(sweep Unhealthy reaches threshold)Open→timeout(60s)HalfOpen→≥2 consecutive successes Closed / a failure reopens Open | State transitions and `is_circuit_open(timeout)` tri-state correct; timeout=0 half-opens immediately | Inline, 10+ cases | ✅ |
| DEG-05 | Sweep over an already-degraded algorithm | check_all_health | **Completely skips** its health_check | Inline | ✅ |
| DEG-06 | `enable_circuit_breaker=false` | Sweep Unhealthy reaches threshold | Only trigger_degradation, no breaker | Inline | ✅ |
| DEG-07 | `enabled=false` / background task | start/stop/double-start | enabled=false sweeps idly; CAS prevents double start; stop joins gracefully | Inline, 3 cases | ✅ |
| DEG-08 | `determine_effective_algorithm` | primary healthy / first available on chain / all degraded | Normal / Degraded(fallback) / Critical | Inline+e2e (critical scenario) | ✅ |
| DEG-09 | Audit previous_state | Degradation event | Status quo: **always "Normal"** (hardcoded; state chains not starting at Normal are distorted) | Pinning test exposes it | ❌ gap |
| DEG-10 | Counters under long runs | Consecutive failures/successes >255 | AtomicU8 **wraps**; `should_degrade` may misjudge | Pinning/fix | ❌ gap |
| DEG-11 | Dead-config pinning | Set `auto_recovery=false`, `recovery_check_interval_ms`, `min_step/max_step` | Status quo: no consumer (config legal but ineffective) — pin to prevent assuming effect | Pinning test | ❌ gap |
| DEG-12 | e2e degradation chain | 12 degradation_tests (including 4 etcd-gated state-machine tests) | Degradation → fallback switch → recovery, full chain | `--features etcd` | ✅ |

### 3.4 AUTH — Authentication and Authorization Domain

**Credential parsing and verification**

| ID | Scenario (prereq) | Steps | Expected | Verification | Status |
|---|---|---|---|---|---|
| AUTH-01 | Valid key | `Authorization: Basic base64(id:secret)` / `ApiKey id:secret` (secret may contain colons, split at the first) | 200; role/workspace injected as extensions | auth_handlers_e2e (35 tests) | ✅ |
| AUTH-02 | 6 malformed header types | Bearer prefix/bad base64/non-UTF-8/Basic without colon/ApiKey format error/empty credentials | All 401 bare JSON `{"code":401,"message":"Invalid or missing API key"}` (reason not distinguished, anti-enumeration) + failure recorded | e2e six-pack | ✅ |
| AUTH-03 | `auth.enabled=false` | Header-less request | Anonymous injected, reaches `anonymous_block` → **401 fail-closed** (missing role extension also 401) | e2e four states | ✅ |
| AUTH-04 | Same-IP consecutive auth failures | 10 per 5min window | The 11th → 429 (independent of the business rate-limit bucket); window expiry unblocks; bucket capacity 10000 evicts oldest | e2e + inline | ✅ |
| AUTH-05 | garrison cache hit | Second request with the same key | Second time skips DB+Argon2 (observable: DB call count); **wrong secrets are never cached** (no negative caching) | auth_cache_wiring 7 tests | ✅ |
| AUTH-06 | Grace-period credentials | Old secret verifies within the window | `used_previous_credential=true` → **no cache write** | e2e | ✅ |
| AUTH-07 | Cache residue after revocation | Request immediately after revoking a cached key | 401 (handler proactively invalidates the cache on revoke/rotate) | e2e | ✅ |
| AUTH-08 | `cache_ttl_seconds=0` | Startup | No AuthCache assembled; all direct lookups | e2e zero-TTL | ✅ |
| AUTH-09 | Cache TTL clamping | key remaining lifetime < cache_ttl | TTL clamped to the remaining lifetime (±10% jitter) | cache inline | ✅ (single-flight 100 concurrent misses tested) |

**Roles and tenants**

| ID | Scenario | Expected | Verification | Status |
|---|---|---|---|---|
| AUTH-20 | Admin accesses a User-only endpoint (generate/biz-tags etc.) | 403 `admin_cannot_perform`/1002 | e2e matrix | ✅ |
| AUTH-21 | User accesses Admin-only (api-keys, workspace creation, config writes) | 403 `Admin access required` | e2e + inline mini router | ⚠️ no real-router penetration test (see GAP) |
| AUTH-22 | Anonymous accesses authed endpoints (5 methods) | 401 fail-closed | e2e | ✅ |
| AUTH-23 | User accesses another's workspace resources | biz-tag CRUD, all 4 methods 403 (IDOR protection: verify then compare workspace) | e2e, 4 cases + isolation | ✅ |
| AUTH-24 | Admin `GET /biz-tags` without workspace_id | **400/rejected** (must be explicit; CWE-639 fix); User forcibly bound to its own | e2e | ✅ |
| AUTH-25 | Workspace ownership when auth is disabled | Key without a workspace binding → any workspace admitted (existing semantics) | Inline verify_* matrix | ✅ |

**Key lifecycle (repository layer)**

| ID | Scenario (prereq) | Steps | Expected | Verification | Status |
|---|---|---|---|---|---|
| AUTH-30 | Admin creates a key | `POST /api-keys` | Prefix niad_/nino_ by role; secret plaintext **returned once only**; default 30-day expiry, rate_limit=10000 | e2e + repository inline | ✅ |
| AUTH-31 | Secret length out of bounds | <8 or >128 | `InvalidInput` (DoS protection); `Anonymous` role fail-fast rejects persistence | Repository inline | ✅ |
| AUTH-32 | A second admin key | Create another admin | Rejected (SQL `role='admin' AND enabled=true` full-table count guard); **revoking the last enabled admin is also rejected** | e2e, both directions | ✅ |
| AUTH-33 | Rotation grace period >0 | Verify the old secret after rotation | Usable within the window (prev_hash+rotate_expires_at lazy expiry); dead at the deadline; grace=0 expires immediately; grace_expires_at non-null only for "rotated and >0" | Repository inline window boundaries + e2e | ✅ |
| AUTH-34 | last_used throttling | High-frequency verification | DB writes throttled per key in a 60s window, 10000 capacity evicting oldest | Repository inline | ✅ |
| AUTH-35 | Argon2id parameter stability | hash → PHC string | v=19,m=19456,t=2,p=1; material=`salt|key_id:secret`; malformed hash verify → false (not Err) | Repository vector tests | ⚠️ key_hasher itself has zero inline tests (indirect coverage) |
| AUTH-36 | Config-preset api_keys | Startup with multiple `[auth].api_keys` | **Only the first takes effect** (keys.first()), the rest silently ignored; the whole section is overridden when `NEBULA_ADMIN_API_KEY_SECRET` is set; illegal workspace values silently replaced with the nil UUID; role only takes effect for the exact string `admin` | main.rs key loading (⚠️ integration pinning recommended) | ⚠️ recorded in FAQ, no automation |
| AUTH-37 | Production salt validation | Empty salt/weak defaults in production | Startup panic (fail-fast); a weak `NEBULA_DATABASE_PASSWORD` only warns | main.rs tests | ✅ |

### 3.5 IDGEN — Minting API Domain (HTTP)

| ID | Scenario (prereq) | Steps | Expected | Verification | Status |
|---|---|---|---|---|---|
| IDGEN-01 | User key + existing workspace/biz_tag | `POST /api/v1/generate` per algorithm | 200 + ID; snowflake/uuid shape validation (parse bit extraction) | api_test.sh + e2e | ✅ |
| IDGEN-02 | Request with `algorithm` override | Pin uuid_v7 (true algorithm, produces version=7 layout) | Per-call override applies | api_test.sh | ✅ |
| IDGEN-03 | Admin calls generate | Same | **403** (generate/batch are User-only) | e2e | ✅ |
| IDGEN-04 | Cross-tenant namespace | User B mints into A's workspace | 403 workspace_mismatch | e2e + grpc | ✅ |
| IDGEN-05 | workspace/biz_tag does not exist | Mint | 404 (business codes 2001/2003) | e2e | ✅ |
| IDGEN-06 | batch boundaries | size=1 / size=max(default 100) / size=0 / size>max | 200 / 200 / 400 / 400 (3001) | e2e (1..=100 boundaries) + grpc | ✅ |
| IDGEN-07 | Missing fields/wrong types in body | Empty body, wrong types | 400 + validator-structured field+rule (no constraint values leaked) | e2e request-validation boundaries | ✅ |
| IDGEN-08 | `POST /api/v1/parse` | IDs per algorithm / empty string / non-ID | 200 metadata / 400 InvalidIdString | e2e combination boundaries | ✅ |
| IDGEN-09 | Body >1MiB | 2MB POST | 413 PayloadTooLarge (3001) | ⚠️ IntoResponse unit test exists; no real-router penetration | ⚠️ |
| IDGEN-10 | Wrong Content-Type / nonexistent path / nonexistent method | 415/404/405 | Envelope or axum default body, asserted per status quo | api_test.sh | ✅ (black box) |
| IDGEN-11 | `X-API-Version: v2` | Any /api/v1 request | 400 + 3001; illegal/missing → fail-open treated as v1; response writes back `X-API-Version: v1` | server_layer, 8 cases | ✅ |
| IDGEN-12 | `format`/`prefix` request fields | Pass prefixed/custom prefix | Status quo: **no consumer** (unimplemented, fields ignored) — pin to prevent misunderstanding | Pinning | ❌ gap |

### 3.6 RES — Resource Management Domain

| ID | Scenario (prereq) | Steps | Expected | Verification | Status |
|---|---|---|---|---|---|
| RES-01 | Admin creates a workspace | `POST /workspaces` | 200 + **the User key plaintext returned once only**; defaults max_groups=10/max_biz_tags=100/status=Active | e2e + api_test.sh | ✅ |
| RES-02 | Tenant-scoped list views | User list / Admin list | User filtered item-by-item to its own; Admin sees all | e2e | ✅ |
| RES-03 | Query by name | `GET /workspaces/{name}` | User only its own (look up the row first, then compare); another's → 403; missing → 404 | e2e | ✅ |
| RES-04 | Reset the user key | `POST /workspaces/{name}/regenerate-user-key` | Old user key deleted + cache invalidated + new secret returned once | e2e/api_test.sh | ✅ |
| RES-05 | group create/list | User + ownership validation | 200/pagination; cross-tenant 403 | e2e | ✅ |
| RES-06 | biz_tag full CRUD | Create/read/update/delete | DELETE returns 204; IDOR protection on all 4 methods (AUTH-23); defaults segment/Numeric/base_step=100 | e2e | ✅ |
| RES-07 | Cascade deletes | Delete a group / delete a workspace with tags | Group deletion cascades biz_tags in a transaction; workspace CASCADE | Repository inline (mock) + ignore real-DB | ⚠️ real DB covered by ignore tests only |
| RES-08 | api-key management | list (illegal UUID param → warn, fallback nil)/revoke | Response bodies correct; guards in AUTH-32 | e2e | ⚠️ handler inline only 5 tests |
| RES-09 | Repository missing rows/bad params | update/delete nonexistent rows | `NotFound`; illegal inputs `InvalidInput` | Repository inline | ✅ |
| RES-10 | Pagination boundaries | limit/offset 0, oversized | No panic, empty page returned | Repository inline | ✅ |

### 3.7 RATE — Rate-Limiting Domain

| ID | Scenario (prereq) | Steps | Expected | Verification | Status |
|---|---|---|---|---|---|
| RATE-01 | 429 genuinely produced | Push through the burst | 429 special-case body `{"code":429,"message":"Rate limit exceeded","retry_after":N}` (**not the envelope**) | server_layer, 12 cases | ✅ |
| RATE-02 | Canonical response headers | Admit/reject | Always `x-ratelimit-limit`; on rejection `remaining:0` + `Retry-After` (= bucket refill seconds, not hardcoded) | Inline | ✅ |
| RATE-03 | Layer ordering | OPTIONS preflight | Preflight consumes no quota; 429 carries CORS headers (rate limiting inside CORS) | Router layer-order contract test | ✅ |
| RATE-04 | Key isolation | Different IPs / same IP | Buckets independent; 16 shards even | Inline per-key | ✅ |
| RATE-05 | workspace key | — | **Production assembly never injects the workspace_id extension** → in practice everything buckets by IP (tests pin a key strategy inconsistent with production — a test-production drift) | Drift note + pinning recommended | ⚠️ |
| RATE-06 | Hot update | Load test after `POST /config/rate-limit` | Table swapped, buckets cleared, new thresholds immediate | e2e hot-update effectiveness | ✅ |
| RATE-07 | Limiter internal errors | Inject limiter errors | **Fail-open admits** (availability first) | Inline | ✅ |
| RATE-08 | Idle bucket reclamation | 5min without access | Background reclaim every 60s | e2e | ✅ |
| RATE-09 | Forged XFF | XFF from a non-trusted proxy | Only proxies in `NEBULA_TRUSTED_PROXIES` are trusted; direct connections use PeerAddr (DualListener-injected) | infra_e2e, 5 cases | ✅ |
| RATE-10 | gRPC | — | **gRPC has no business rate limiting** (only the auth failure bucket) — doc-behavior consistency pin | Pinning | ⚠️ |
| RATE-11 | Token refill | Retry after waiting | Refilled at the configured rate | Inline | ✅ |

### 3.8 GRPC — gRPC Domain

| ID | Scenario (prereq) | Steps | Expected | Verification | Status |
|---|---|---|---|---|---|
| GRPC-01 | Valid key | Generate / Parse / HealthCheck | Normal responses; **HealthCheck is also authenticated** (probes should use HTTP /health) | grpc_e2e (real tonic transport) | ✅ |
| GRPC-02 | count boundaries | 0 / max+1 / 1 / max | `InvalidArgument` on both sides of the endpoint, both sides pass | e2e | ✅ |
| GRPC-03 | Auth failure classification | Missing metadata/bad format → Unauthenticated; disabled/expired → **PermissionDenied** (classify_miss re-checks the row); unknown/wrong secret → Unauthenticated (same copy as unknown, anti-probing) | e2e, all branches | ✅ |
| GRPC-04 | Shared failure bucket | Mixed HTTP+gRPC failures | Same bucket accumulates, cross-protocol 429/ResourceExhausted | ⚠️ single-protocol tested; the cross-protocol combination is missing | ⚠️ |
| GRPC-05 | authorize_namespace | admin minting → 403; cross-tenant → 403; unknown ns → 404; repository fault → Internal fixed copy | e2e, 5 cases | ✅ |
| GRPC-06 | BatchGenerateStream | Illegal request initialization | Rejected before stream consumption; a cross-tenant item in-stream → Status **terminates the whole stream** | e2e stream test | ✅ |
| GRPC-07 | Full error mapping | CoreError's 24 variants → gRPC codes | 4xx keeps the localized message (truncated at 200 bytes); 5xx → Internal("internal error") fixed copy (sanitized) | helpers matrix | ✅ |
| GRPC-08 | proto contract | Field names `namespace`/`tag` correspond to HTTP workspace/biz_tag; GenerateResponse has no datacenter field | Contract test prevents proto drift | ⚠️ generated artifacts maintained by hand (build.rs removed), no contract guard | ⚠️ |
| GRPC-09 | gRPC TLS/mTLS | ca_path configured | tonic ServerTlsConfig applies | ⚠️ TLS assembly unit-tested; transport-level mTLS handshake missing | ⚠️ |

### 3.9 OBS — Observability and Audit Domain

| ID | Scenario (prereq) | Steps | Expected | Verification | Status |
|---|---|---|---|---|---|
| OBS-01 | Scrape after minting | `GET /metrics` | Prometheus text 0.0.4; the full metric set: ids_generated_total/failures/clock_backwards (algorithm label), latency p50/p99/p999 gauges + histogram (50µs–5s buckets), cache_hit_rate, algorithm_degraded, circuit_breaker_open/half_open, requests_total, uptime_seconds | Render-bridge inline + api_test.sh | ✅ |
| OBS-02 | Zero-traffic scrape | Scrape without minting | The histogram is skipped when all percentiles are 0 (prevents fabricated observations) | Inline | ✅ |
| OBS-03 | Probes | /health /ready /health/sdforge | healthy/ready component states/sdforge version | api_test.sh + e2e | ✅ |
| OBS-04 | OpenAPI guard | `GET /api-docs/openapi.json` | paths and route registration **bidirectionally consistent** (EXPECTED_V1_PATHS guard); paths empty without the openapi feature | openapi inline | ✅ |
| OBS-05 | Audit middleware | 2xx/4xx/5xx requests | Event result=Success/Failure/Partial; includes duration, client_ip, user_agent, request_id/trace_id | e2e, 4 cases | ✅ |
| OBS-06 | Audit file persistence | Production+file_logging | Writer task async writes, 50ms flush, sync_all confirmation; client_ip keeps the first 3/4 segments, UA redacted | logger inline, 15 cases | ✅ |
| OBS-07 | Audit anomalies | Ring full (10000)/file unopenable/write failure/path containing `..` | Evict oldest+warn+count / fall back to the in-memory ring / broken count / path rejected (CWE-22) | Inline | ✅ |
| OBS-08 | Alert state machine | Rules continuously satisfied | Pending→(for_duration)Firing→resolved Resolved; promotions u8 saturating; history 1000 FIFO | Inline, 130+ tests (alerting/test always compiles) | ✅ |
| OBS-09 | Alert expressions | 9 DSL kinds (qps/latency/error_rate/segment_exhausted etc.) | Parse failure/unknown → warn + no trigger | Inline | ✅ |
| OBS-10 | webhook SSRF | Intranet/loopback/CGNAT/multicast targets | Rejected (http(s) only, userinfo/private nets banned; redirects disabled); **known residue: `https://[::1]` bypass** (pinned on record) | Inline, all branches | ✅ (residue known) |
| OBS-11 | Notification channel hot swap | update_channels | ArcSwap atomic effect | Inline | ✅ |
| OBS-12 | Log redaction | Credential/key fields entering logs | inklog fast-masking redacts automatically | ⚠️ a dependency-library capability, no in-repo assertion (README_TEST_CONFIG has a black-box check idea) | ⚠️ |

### 3.10 TLS — Transport Security Domain

| ID | Scenario (prereq) | Steps | Expected | Verification | Status |
|---|---|---|---|---|---|
| TLS-01 | enabled=true, missing certificate/missing private key | Startup | init fail-fast refuses startup | infra_e2e, 3 cases | ✅ |
| TLS-02 | min_tls_version=tls13 | Lower-version ClientHello | HTTP side rejects the handshake; **gRPC-side exception** (tonic defaults to 1.2+, startup warn marks the boundary) | rcgen self-signed measured | ✅ |
| TLS-03 | DualListener | Same port, plaintext/TLS self-selected | Both HTTPS end-to-end and plain HTTP without TLS work | infra_e2e | ✅ |
| TLS-04 | Slow handshake | ClientHello stalling | 10s handshake timeout + depth-128 queue, **accept never freezes** | infra_e2e | ✅ |
| TLS-05 | PeerAddr injection | Request over a TLS connection | Rate limiting/audit get the real peer address | infra_e2e | ✅ |
| TLS-06 | Production enforcement | `NEBULA_ENV=production`+tls disabled | exit 1; the `NEBULA_ALLOW_INSECURE_TLS=1` escape hatch (warn) | main.rs tests | ✅ |
| TLS-07 | ALPN | Default ["h2","http/1.1"] | Negotiation correct | Unit test | ✅ |
| TLS-08 | Contradictory config | Globally disabled but per-port enabled | warn tls_config_conflict, effectively all off | Unit test | ✅ |
| TLS-09 | CORS unconfigured in production | Start without ALLOWED_ORIGINS | `CorsLayer::new()` (rejects all cross-origin) + 3 error logs; development default only localhost:3000 | server_layer CORS, 9 cases | ✅ |
| TLS-10 | CORS exposure | Preflight/response headers | expose only x-request-id/x-ratelimit-remaining (browsers cannot read limit/Retry-After) | Unit test | ✅ |

### 3.11 COORD — Distributed Coordination Domain (etcd gated)

| ID | Scenario (prereq) | Steps | Expected | Verification | Status |
|---|---|---|---|---|---|
| COORD-01 | Healthy etcd | allocate | Allocates from 1..=255 (skipping the 0 sentinel), writes `/idgen/workers/{dc}/{worker}`+lease | mock, 12 cases + real-DB ignore | ✅ |
| COORD-02 | All occupied | All 255 taken | `NoAvailableId` | mock | ✅ |
| COORD-03 | CAS conflict/preflight failure | Key exists/kv_get Err | Skips that id and continues (preflight failure treated as allocatable) | mock | ✅ |
| COORD-04 | release semantics | Own release/someone else's key/nonexistent/delete failure | Succeeds and clears state / **deletion refused (ownership validation)** / idempotent success / Err | mock | ✅ |
| COORD-05 | keepalive | Period=TTL/3 (10s) | 3 consecutive failures → fail-stop report + exit; intermittent success resets the count; the stop signal exits gracefully | mock, 4 cases | ✅ |
| COORD-06 | Health state machine | ping 3/5 consecutive failures | Degraded→Failed+cache mode; success recovers and exits cache mode; empty endpoints **leave the state unchanged** | mock (4 degradation_tests cases + etcd.rs inline) | ✅ |
| COORD-07 | Cache file | State persisted/read across instances | JSON roundtrip; bad JSON → InternalError; missing directory → write-failure error | mock, 8 cases | ✅ |
| COORD-08 | Distributed lock | acquire conflict | 3 retries ×100ms → AcquireFailed; TTL floor clamped to 1s; CAS failure revokes the granted lease to prevent leakage; Drop spawns release / warns only without a runtime | mock, 12 cases | ✅ |
| COORD-09 | Lease leak window | Network error in txn after a successful grant | Status quo: the lease is not revoked, relying on TTL — **a known window** | Pinning/fix | ❌ gap |
| COORD-10 | Timeouts | Operations exceeding `operation_timeout_secs` (3s) | `EtcdError::Network("timed out")` | mock timeout mapping | ✅ |
| COORD-11 | Startup fail-closed | endpoints configured but unreachable | exit 1 (no silent fallback to the local lock) | main.rs tests | ✅ |
| COORD-12 | local mode (no etcd) | allocate×N | Always returns the configured worker_id (multi-instance collision — semantics pinned); lock TTL ignored, a forgotten release deadlocks forever | local inline | ⚠️ TOCTOU concurrency gap |
| COORD-13 | Stub monitor (no etcd) | set_status(Healthy) | Always Failed (no-op pinned) | local inline, 8 cases | ✅ |
| COORD-14 | Clone semantics | EtcdWorkerAllocator.clone | Atomic snapshot copied but allocated_value is a shared Arc (inconsistent with the monitor's all-Arc) — behavior pinned | Pinning | ❌ gap |

### 3.12 DB — Database Domain

| ID | Scenario (prereq) | Steps | Expected | Verification | Status |
|---|---|---|---|---|---|
| DB-01 | Password missing/containing unexpanded `${` | Non-URL and non-SQLite connection | `ConfigurationError` (hints NEBULA_DATABASE_PASSWORD) | Inline + infra_e2e | ✅ |
| DB-02 | PG URL without options | create_connection | Auto-injects `search_path=nebula_id,public` (the sea-orm enum CAST depends on it) | Inline | ✅ |
| DB-03 | Migration idempotence | run_migrations repeatedly | already-exists on enums/tables/constraints swallowed and continues; **ALTER backfill failure → aborts startup**; failure messages generalized (CWE-209) | Inline, 10-statement mock | ✅ (mock all green; the real-DB path remains ignore integration tests) |
| DB-04 | Concurrent first allocation | Multiple instances' first allocate for the same tag | `ON CONFLICT DO NOTHING`+retry guarantees a single insert, no duplicate segments | **Real-DB concurrency test** | ❌ automation gap (the shell script partially covers it) |
| DB-05 | Atomic allocation | allocate_segment | Single-statement `UPDATE…RETURNING` for the start/end range; row locks serialize | SQL-shape mock | ✅ |
| DB-06 | Statement timeout | Slow query > statement_timeout_secs | `CoreError::TimeoutError` (semantically separates "slow" from "broken") | mock injection | ✅ |
| DB-07 | DbErr mapping | Any DB error | `DatabaseError(first-layer copy, ErrorSource chain preserved)`; redact_db_url masks the password → `***` | Inline | ✅ |
| DB-08 | update_segment 0 rows | Update a nonexistent segment | `NotFound` | Inline | ✅ |
| DB-09 | Entity enum roundtrip | AlgorithmTypeDb/IdFormatDb | DB enums and domain types convert both ways | Inline | ✅ |
| DB-10 | Debug redaction | api_key Model Debug | hash field redacted in output | Inline | ✅ |

### 3.13 I18N — Internationalization Domain

| ID | Scenario (prereq) | Steps | Expected | Verification | Status |
|---|---|---|---|---|---|
| I18N-01 | Accept-Language negotiation | zh-CN / en / no header / fr,ja (unsupported) / q=0 / `*` / >4KiB | The matching language / default en / fallback en / q=0 discarded / `*` never matches / whole header rejected → en | locale inline, 32 cases + i18n_e2e, 7 cases | ✅ |
| I18N-02 | Error envelope translation | Same error, requests with two different headers | Both en/zh-CN correct; `%{name}` argument substitution | i18n_e2e + error.rs | ✅ |
| I18N-03 | Fallback chain | Missing key | en → still missing → return the key name itself (never empty) | Inline | ✅ |
| I18N-04 | Key-set alignment | `locales/{en,zh}/messages.ftl` | Key-set equality guard (prevents missed translations) | Inline guard | ✅ |
| I18N-05 | Locale stays out of auth | Forge Accept-Language | No effect on authentication/authorization decisions (security boundary) | ⚠️ design comments exist; no explicit negative test | ⚠️ |

### 3.14 SEC — Malicious Input and Protection Domain

| ID | Scenario (prereq) | Steps | Expected | Verification | Status |
|---|---|---|---|---|---|
| SEC-01 | 5xx information leakage | DatabaseError carrying a connection string with a password | The body contains no `postgres://`/secret/host:5432; fixed copy + full detail only in logs (CWE-209) | i18n_e2e password-leak test | ✅ |
| SEC-02 | IDOR | User operates another's biz_tag (4 methods) | 403 | e2e | ✅ |
| SEC-03 | Auth probing | Existing/nonexistent key, wrong secret | Unified 401 copy (HTTP) / "invalid or unknown api key" (gRPC) | e2e | ✅ |
| SEC-04 | Path traversal | Audit log path `../x` | Rejects empty paths and `..` | logger inline | ✅ |
| SEC-05 | SSRF | webhook pointing intranet | Rejected (the `https://[::1]` residue is known, OBS-10) | Inline | ✅ |
| SEC-06 | Timing side channel | Secret comparison | Argon2's built-in constant-time comparison (verify path) | ⚠️ guaranteed by the dependency library | ⚠️ |
| SEC-07 | Large requests | >1MiB | 413 (IDGEN-09) | ⚠️ | ⚠️ |
| SEC-08 | Header injection | Illegal request_id header | Only a valid UUIDv7 passes through, otherwise a new one is generated | request_id inline | ✅ |
| SEC-09 | Bearer rejection | `Authorization: Bearer x` | 401 (part of AUTH-02) | ✅ | ✅ |
| SEC-10 | Log PII | Audit rows | IP truncated, UA redacted (OBS-06) | ✅ | ✅ |
| SEC-11 | Unknown fields | JSON body with extra fields | The config layer rejects via deny_unknown_fields; **at the HTTP handler layer serde ignores unknown fields by default** (different methodologies; pinned with an explanation) | ⚠️ | ⚠️ |

### 3.15 SDK — Embedded Domain (feature `sdk`)

| ID | Scenario (prereq) | Steps | Expected | Verification | Status |
|---|---|---|---|---|---|
| SDK-01 | Full assembly | NebulaIdKitBuilder.build | Four-module topological order (lock→repository→router→idgen); missing/cyclic dependencies → build Err | kit inline, 24 tests | ✅ |
| SDK-02 | No repository injected | Default algorithm=Segment | The repository module is absent; minting **errors explicitly** (require_repository_for_segment) | Inline | ✅ |
| SDK-03 | etcd fail-closed | endpoints unreachable | build returns Err directly (no local-lock fallback) | Inline | ✅ |
| SDK-04 | Health aggregation | health_summary | worst-of aggregation JSON (Healthy/Degraded/Unhealthy/no algorithms) | Inline | ✅ |
| SDK-05 | Lifecycle | on_ready/on_shutdown | Degradation sweep start/stop, reverse-topological drain | Inline | ✅ |
| SDK-06 | Examples runnable | `--example embedded --features sdk` / `--example sdk_server --features sdk,http` | Zero-DB minting / the SDK HTTP service's two endpoints | Manual/script | ⚠️ no automation |

### 3.16 LIFE — Lifecycle Domain

| ID | Scenario (prereq) | Steps | Expected | Verification | Status |
|---|---|---|---|---|---|
| LIFE-01 | Startup with DB unreachable | Stop PG and start | Connection failure exit 1; migration failure exit 1 | Manual/script | ⚠️ no automation |
| LIFE-02 | Graceful shutdown | SIGINT/SIGTERM | The select arms fire: stop sweeps → abort rate-limit cleanup → [etcd] stop keepalive + release worker; HTTP graceful shutdown | main.rs tests (shrink scenario) + docker compose stop observation | ⚠️ |
| LIFE-03 | etcd lease failure shutdown | Cut etcd for ≥3 renewal periods | **The process exits proactively** (fail-stop prevents dual-primary duplicate IDs) — one of this system's most critical safety properties | Real etcd fault injection | ❌ automation gap (mock has single steps; no end-to-end) |
| LIFE-04 | HTTP/gRPC listen failure | Port occupied | The select arm ends the process and wraps up | Manual | ❌ |
| LIFE-05 | Background task cleanup | After shutdown | Rate-limit cleanup task aborted, degradation sweeps stopped, audit writer flushed and exited | Inline for each; no combined ordering test | ⚠️ |
| LIFE-06 | Dual-instance worker conflict (no etcd) | Two processes, same config | Same worker_id always → Snowflake can theoretically collide — the docs declare etcd a prerequisite for multi-instance | Doc-methodology pinning | ⚠️ |

---

## 4. Cross-Domain Fault Injection

> The scenarios below cut across multiple domains and need the three-phase "fault injection → behavior assertion → recovery assertion" verification. Rust-level single steps already have mock coverage; **the end-to-end recovery chain is the main gap**.

| ID | Fault | Injection method | Expected (degradation) | Expected (recovery) | Status |
|---|---|---|---|---|---|
| FAULT-01 | PostgreSQL disconnect | docker stop nebula-postgres | Minting (segment) reports 5xx sanitized copy; snowflake/uuid_v8 **unaffected** (zero DB); `/ready` database=false | Auto-recovers after restart, no app restart needed | ⚠️ mock single steps exist; the shell degradation script injects no real fault |
| FAULT-02 | PostgreSQL slow queries | `pg_sleep` background sessions / pool exhaustion | Statement timeout → `TimeoutError`/503-5004; acquire-timeout errors | Normal after recovery | ❌ |
| FAULT-03 | etcd disconnect (--features etcd) | docker stop nebula-etcd | Health monitor 3 consecutive failures Degraded→5 consecutive failures Failed+**cache mode** (reads the local cache file); lock operations fail | Ping recovery exits cache mode | ⚠️ mocks cover everything; no real-machine chain |
| FAULT-04 | etcd disconnect, prolonged | Continuous outage >3 renewal periods | worker keepalive fail-stop → **process exit** (better to stop than mint duplicates) | — (orchestration restarts, re-acquiring the lease) | ❌ a gate-closing scenario; real-machine automation recommended |
| FAULT-05 | Clock rollback | System level (privileged) or algorithm injection | >threshold: error + metric + possible degradation-chain takeover; ≤threshold: silent waiting | Drift's 60s decay restores health | ✅ algorithm-level injection; no system-level |
| FAULT-06 | Audit filesystem full/read-only | Mount a read-only directory | File unopenable → **falls back to the in-memory ring**, requests unaffected | — | ✅ inline |
| FAULT-07 | Limiter internal errors | Inject limiter errors | Fail-open admits (availability first) | — | ✅ inline |
| FAULT-08 | Redis failure | docker stop nebula-redis | Status quo: the Redis backend has **no production consumer** (caching uses in-process oxcache/garrison memory) — verify the service is entirely unaffected + doc consistency | — | ⚠️ methodology confirmation is the test |
| FAULT-08 closing statement | — | — | **"The Redis cache layer is an implementation-reserved surface with no production consumer; cross-instance segment sharing risks duplicate IDs and is out of scope for this fix"**; the `[redis]` section / `REDIS_URL` / compose's redis service keep the deployment shape only; wiring a cache layer requires a separate effort evaluating ID uniqueness | — | ✅ adjudicated (2026-10-15) |
| FAULT-09 | Disk full (segment persistence) | Fill a tmpfs | allocate reports DatabaseError (5xx sanitized); no panic | Normal after recovery | ❌ |

---

## 5. Test Execution Plan

### 5.1 Layered Execution Strategy

| Layer | Content | Scale (measured 2026-09-17) | External dependencies | Frequency |
|---|---|---|---|---|
| L0 inline units | `#[cfg(test)]` in src modules | core 892 + server 717 + sdk 24 + main 18 ≈ 1651 | none (MockDatabase/rcgen self-signed/tempfile) | Every commit (pre-commit `cargo test --lib`) |
| L1 in-crate E2E | the 12 modules in `src/core/tests/` + `tests/i18n_e2e.rs` | 221 + 7 | none | Every commit |
| L2 real infrastructure | `#[ignore]` ×9 (8 repository + 1 etcd) | 9 | PostgreSQL / etcd | Manual before merge / nightly |
| L3 shell black box | the 4 `tests/*.sh` scripts (against a running service) | 4 | docker compose full stack | Before releases |
| L4 benchmarks | `benches/i18n.rs` (the only one), `benches/algorithms.rs` | 2 files | none | On performance regressions |

### 5.2 Command Manual

```bash
# L0+L1 full (default features, zero external dependencies)
cargo test --package nebulaid --no-fail-fast
# Baseline cross-reference: exactly 7 known failures (6× test_run_migrations_* + 1× e2e_database_run_migrations_creates_tables)

# All-features full set (pre-push / coverage gate / CI all leg, same switch)
cargo test --package nebulaid --all-features --no-fail-fast

# Light compile check (same as the CI fmt-clippy job; compile only, no tests)
cargo check --package nebulaid --no-default-features --lib --bins

# L2 real PostgreSQL (docker compose starts postgres first)
docker compose -f docker/docker-compose.yml up -d postgres
DATABASE_URL=postgresql://idgen:idgen123@localhost:5432/idgen \
  cargo test --package nebulaid --features integration-tests -- --ignored --no-fail-fast

# L2 real etcd
ETCD_TEST_ENDPOINTS=http://127.0.0.1:2379 \
  cargo test --package nebulaid --features etcd,integration-tests -- --ignored

# L3 full-stack black box
docker compose -f docker/docker-compose.yml --env-file docker/.env up -d
export ADMIN_API_KEY_ID=... ADMIN_API_KEY_SECRET=...
./scripts/run.sh api-test http://localhost:8080        # main chain (health/minting/parse/validation/performance)
./tests/db_concurrency_test.sh                          # concurrency and uniqueness
./tests/degradation_test.sh                             # health-path observation (note: injects no real fault)
./tests/distributed_test.sh                             # distributed uniqueness/ordering/bit structure

# Coverage gate (pre-push 90%; CI 95% excluding server/proto/)
cargo llvm-cov --package nebulaid --fail-under-lines 90

# Benchmarks
cargo bench --bench i18n
```

> ⚠️ Known script defects (manually review when using L3 results): `db_concurrency_test.sh` collects no IDs during the high-concurrency phase (uniqueness is only truly validated in the final segment) and unconditionally prints a pass summary; `degradation_test.sh` injects no real faults and only warns on failure without changing the exit code; `distributed_test.sh` ordering assertions vacuously pass when responses lack a timestamp field; `api_test.sh` defaults to an external production URL (always pass localhost explicitly), its `GET /api/v1/metrics` case fails by design because the route does not exist, and created workspaces are not cleaned up.

### 5.3 CI Gate Cross-Reference (.github/workflows/ci.yml)

| Stage | Command essentials | Threshold |
|---|---|---|
| fmt+clippy | `--all-features` `-D warnings` (plus a `--no-default-features` compile check) | zero warnings |
| deny+audit | cargo-deny / cargo-audit | deny warnings |
| Multi-feature matrix tests | matrix = default / all (`--all-features`, covering sdk+etcd+alerting+integration-tests) | all green |
| Coverage | `llvm-cov --fail-under-lines 95 --ignore-filename-regex "server/proto/"` (authoritative gate on the default leg; the all leg runs tests only, not the gate — sdk code's test density is low, consistent with the old standalone sdk job policy) | 95% line coverage |
| lefthook pre-push | `--all-features` test + coverage `--fail-under-lines 90` (measured locally ≈93.6%: diluted by sdk code) | 90% |

### 5.4 Placement Conventions for New Tests

- Pure logic branches/boundaries → inline `#[cfg(test)]` in the corresponding module (mock injection reuses the existing stand-ins in `testing.rs`: `DummyDistributedLock`/`FailingDistributedLock`/`MockDatabase`).
- Cross-middleware HTTP behavior → `src/core/tests/server_layer_e2e_tests.rs` (mini-router pattern).
- Real-infrastructure semantics (concurrent allocation, fail-stop) → `#[ignore]` + the `integration-tests` gate, reusing production `run_migrations` as the schema source of truth.
- Black-box regression → extend `tests/*.sh` (prefer fixing the script defects listed in 5.2 over writing new scripts).

---

## 6. Gap Summary and New-Test Priorities

### P0 — Correctness / asset-loss risk (fix immediately)

| # | Gap | Corresponding scenario | Suggested placement |
|---|---|---|---|
| 1 | No automation for real-PG concurrent first-allocation duplicate prevention (the ON CONFLICT path) | DB-04 | `#[ignore]` concurrent allocate + uniqueness assertions (repository.rs integration module) |
| 2 | etcd lease fail-stop end-to-end (cut etcd → process exits) | LIFE-03 / FAULT-04 | ignore + a real etcd container |
| 3 | The full HTTP middleware stacking chain (rate limit→CORS→audit→locale→auth combined order) tested at only one point | §1.3 invariants 1-4 | server_layer_e2e combined router |
| 4 | `size_limit` 413 and admin 403 penetration through the **real router** | IDGEN-09 / AUTH-21 | server_layer_e2e |
| 5 | batch size=0 semantic split across three algorithms + Segment batch lacking CAS duplicate prevention | ALG-05/11 | Behavior pinning (pin the status quo first, then discuss a fix) |

### P1 — Behavior-drift protection (pinning against regression)

| # | Gap | Scenario |
|---|---|---|
| 6 | Dead-config pinning: `min_step/max_step`, `auto_recovery`, `recovery_check_interval_ms`, `GenerateContext.format/prefix`, the Redis backend | DEG-11 / IDGEN-12 / FAULT-08 |
| 7 | Payload-semantics split pinning: `ClockMovedBackward.last_timestamp` (Snowflake=state-word ts vs UUIDv8=drift delta) | ALG-22/31 |
| 8 | `SequenceOverflow` (seq_bits=0), `Id::Display` version-nibble misjudgment, dc/worker out-of-width truncation | ALG-24/29/33/52 |
| 9 | Degradation audit `previous_state` always "Normal", AtomicU8 wraparound | DEG-09/10 |
| 10 | `QpsWindow` clock-rollback panic risk, LocalDistributedLock TOCTOU, the etcd lease leak window | ALG-56 / COORD-09/12 |
| 11 | The config service-layer 200-wrap on validation failure, `GET /config` role branches | CFG-22/11 |
| 12 | Direct key_hasher unit tests (material encoding/empty-salt boundaries) | AUTH-35 |
| 13 | Cross-protocol shared auth failure bucket (mixed HTTP+gRPC counting) | GRPC-04 |

### P2 — Engineering and documentation consistency

| # | Gap | Notes |
|---|---|---|
| 14 | ~~Docker build chain broken: `vendor-deps.sh` still looks for the old `../sdforge`, `../inklog` paths (now `../base/*` + `../garrison`), so `docker/build.sh` and compose build were guaranteed to fail~~ **fixed**: dependencies all come from crates.io, the vendor mechanism removed; the builder moved up to `rust:1.97.1-bookworm` (confers edition 2024 requires it), the runtime aligned to bookworm, and the image builds with `--features etcd` | The DEPLOYMENT quick start works; `docker build -f docker/Dockerfile .` measured |
| 15 | Metrics port contradictions in three places (compose 9092 mapping / Dockerfile EXPOSE+dead ENV / docs saying "no separate metrics port") | Clean up compose and the Dockerfile |
| 16 | Doc drift: the `to_u128()` example (actually `as_u128`), DEPLOYMENT §8.5's old endpoint list, FAQ throughput numbers with no benchmark backing, two versions of the vulnerability reporting channel, dc_id 0-7 vs ≤31, USER_GUIDE still listing sqlite/MySQL | Fix item by item and track in the FAQ's known issues |
| 17 | `api_test.sh`'s `/api/v1/metrics` case, default production URL, and workspaces not cleaned up | Script fix |
| 18 | Proto artifacts hand-maintained with no contract guard; examples have no automated run | Contract tests + an sdk example smoke |

---

## Statistics Summary

- Scenario total: **~190 across 16 domains** (normal + abnormal + boundary + fault injection), roughly 80% already automated (L0+L1 ≈1879 test functions), with 18 explicit gaps (P0×5 / P1×8 / P2×5).
- Feature combinations: all combinations buildable (`--all-features` unlocked; `--no-default-features` is a compile check only), with 11 behavior-difference points (§2.3).
- Coverage baseline: local `llvm-cov` with all features ≈93.6% (diluted by sdk code's test density); the CI default leg's ≥95% is the authoritative gate.
