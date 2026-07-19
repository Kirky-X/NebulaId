# Changelog

All notable changes to Nebula ID are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0] - 2026-07-18

The v0.2.0 release ships 11 phases of hardening, refactor, and developer-experience
work on top of v0.1.x. Highlights: a unified script entry (`scripts/run.sh`), an
ICU i18n layer with `Accept-Language` negotiation, two new trait abstractions
(`EtcdClientOps`, `ConfigManagementService`) for testable business logic, a
0-warning / 0-clippy-alert baseline, and a 95%+ coverage gate enforced in CI.

### Phase 1 - Quick Start Baseline

- Added `docs/archive/` to `.gitignore` so archived reports no longer pollute
  the working tree.
- Introduced `scripts/run.sh` as the single entry point dispatching to six
  subcommands (`deploy`, `lint`/`pre-commit`, `redis-test`, `api-test`,
  `install-hooks`, `help`).
- Renamed the v0.1.x scattered scripts to `_*_impl.sh` internal implementations
  (`deploy.sh` -> `_deploy_impl.sh`, `pre-commit-check.sh` -> `_pre_commit_impl.sh`,
  `redis_test.sh` -> `_redis_test_impl.sh`, `test_api.sh` -> `_api_test_impl.sh`,
  `install-pre-commit-hooks.sh` -> `_install_hooks_impl.sh`) using `git mv` to
  preserve history.
- Bumped `Cargo.toml` version `0.1.1` -> `0.2.0`.

### Phase 2 - GitHub Actions CI Repair

- Diagnosed and fixed all five workflows (`ci.yml`, `codeql.yml`,
  `code-review.yml`, `health-check.yml`, `release.yml`) that were failing on
  `main` after the v0.1.x push.
- Temporarily lowered the coverage gate from `--fail-under-lines 80` to
  `--fail-under-lines 50` so CI could go green while trait-refactor and test
  backfill phases caught up (the gate is restored to `95` in Phase 11).
- Rewrote every `bash scripts/<old-name>.sh` invocation in `.github/workflows/*.yml`
  to `bash scripts/run.sh <subcommand>` so CI and local workflows share the same
  entry point.

### Phase 3 - Zero-Warning Baseline

- Achieved 0 warnings / 0 errors on `cargo build --all-features`.
- Achieved 0 warnings / 0 errors on `cargo clippy --all-features --all-targets -- -D warnings`.
- Achieved 373 passed / 0 failed on `cargo test --all-features`.
- Verified `cargo fmt --check` reports no drift.

### Phase 4 - Dead Code Audit

- Ran `cargo udeps --all-features --all-targets` and removed every confirmed
  unused dependency from `Cargo.toml`.
- Ran `cargo rustc --all-features -- -W dead_code` and audited each `never used`
  finding: dead symbols were deleted, kept symbols were annotated with
  `#[allow(dead_code)]` plus a comment explaining why they are retained.
- Audited every `TODO` / `FIXME` / `todo!()` / `unimplemented!()` occurrence in
  `src/`: each was either implemented in scope or removed, no placeholders
  remain.

### Phase 5 - Dedup Review

- Ran an architecture audit (diting) to enumerate duplicated implementations
  across error types, repository traits, config loaders, and validation
  helpers.
- Consolidated duplicates into the canonical module (e.g. `src/core/types/error.rs`
  for shared error variants) and updated all callers.
- Re-ran the audit to confirm zero remaining duplicates.

### Phase 6 - EtcdClientOps Trait Refactor

- Introduced `#[async_trait] pub trait EtcdClientOps: Send + Sync` in
  `src/core/coordinator/etcd.rs` with six methods (`kv_get`, `kv_delete`,
  `lease_grant`, `lease_revoke`, `txn_check_create_rev_and_put`, `ping`).
  `kv_put` was deliberately omitted after architecture review found no
  business caller (rule 2 - simplicity).
- Introduced `EtcdError` enum (`Network` / `KeyNotFound` / `LeaseInvalid` /
  `Internal`).
- Added `EtcdClientWrapper` newtype wrapping `tokio::sync::Mutex<etcd_client::Client>`
  to provide interior mutability so the production client satisfies the
  `&self` contract of the trait.
- Reworked `EtcdDistributedLock`, `EtcdWorkerAllocator`, and
  `EtcdClusterHealthMonitor` to hold `Arc<dyn EtcdClientOps>` and added
  `new_with_client` constructors for mock injection.
- Re-exported `EtcdClientOps` and `EtcdError` from
  `src/core/coordinator/mod.rs` (rule 25 - `mod.rs` only exposes traits +
  pub types).
- Added 25 mock-based unit tests covering `EtcdWorkerAllocator` (10),
  `EtcdDistributedLock` (8), `EtcdClusterHealthMonitor` (5), and `EtcdError`
  Display (2). Test count: 373 -> 398.

### Phase 7 - ConfigManagementService Trait Refactor

- Introduced `#[async_trait] pub trait ConfigManagementService: Send + Sync`
  in `src/server/config/management.rs` covering the ~18 methods actually
  invoked by handlers (config get/update, biz-tag CRUD, workspace/group
  CRUD, metrics, algorithm switching).
- Renamed the concrete struct `ConfigManagementService` -> `ConfigManager`
  so the trait could claim the canonical name; updated all call sites.
- Modified `ApiHandlers.config_service` field from
  `Arc<ConfigManagementService>` to `Arc<dyn ConfigManagementService>`,
  including `ApiHandlers::new` and `with_api_key_repository` signatures.
- Updated the six caller sites (`src/main.rs`, `src/server/handlers/*.rs`,
  `tests/`) to coerce `Arc::new(ConfigManager::new(...))` into
  `Arc<dyn ConfigManagementService>`.
- Added 55 mock-based handler tests (generate/batch_generate/parse,
  health/ready/metrics, biz-tag/workspace/group/api-key CRUD, key rotation).
  Test count: 398 -> 453.

### Phase 8 - ICU i18n Migration

- Added `rust-i18n = { version = "3.1", default-features = false }` to
  `Cargo.toml` (rule 28 - explicit features; rule 29 - Major.Minor pin).
- Created `locales/en.yml` and `locales/zh-CN.yml` skeletons.
- Added `src/core/i18n.rs` with `init_i18n(locale)`, `translate_with_locale`,
  `translate_with_locale_cow`, `translate_with_locale_args`, and
  `translate_with_locale_args_cow` APIs that do not mutate global locale
  state, making them safe for concurrent per-request use.
- Extracted ~20 `CoreError` Display strings into `t!("error.<variant>")`
  keys, with synchronized `en.yml` / `zh-CN.yml` entries.
- Extracted ~180 `tracing::{warn,error,info,debug}!` event strings into
  `t!("log.<module>.<event>")` keys.
- Added `src/server/middleware/locale.rs` implementing `locale_middleware`:
  parses `Accept-Language` per RFC 7231 5.3.5, drops `q=0` and malformed
  entries, sorts by descending q-value (stable on ties), supports exact and
  prefix matching (`zh` -> `zh-CN`), and falls back to `en`. Capped header
  size at 4 KiB to prevent DoS.
- Updated `CoreError -> HTTP response` translation in handlers/helpers.rs
  to read `Extension<Locale>` and call `translate_with_locale_args` so error
  bodies are localized per request without mutating global locale state.
- Verified 453 passed / 0 failed on `cargo test --all-features` and 0
  warnings on `cargo clippy --all-features -- -D warnings`.

### Phase 9 - Three-Axis Audit (diting + tiangang + kueiku)

- **diting** full five-dimension review (security / performance / quality /
  architecture / simplification): all CRITICAL / HIGH / MEDIUM / LOW
  findings fixed (4 CRITICAL + 10 HIGH + 17 MEDIUM + 21 LOW, per user
  rule "MEDIUM and LOW must be resolved").
- **tiangang** SAST scan (Semgrep + CodeQL): all CRITICAL vulnerabilities
  (hardcoded secrets, SQL injection, unsafe eval, buffer overflow, insecure
  deserialization) resolved.
- **kueiku** methodology analysis: confirmed no missing callers in the
  Phase 6/7 trait refactors, no i18n key typos causing key-as-string
  fallback, `Accept-Language` middleware coverage on all `/api/v1/*`
  routes, and no missing argument forwarding in `scripts/run.sh`
  subcommands.
- Re-ran diting + tiangang to confirm 0 CRITICAL / 0 HIGH, and `cargo
  test --all-features` + `cargo clippy -- -D warnings` are both green.

### Phase 10 - Documentation Sync

- Updated `README.md` (English) and `README_zh.md` (Chinese) with new
  Internationalization and `scripts/run.sh` Usage sections, refreshed the
  test-coverage table to 95%+, and removed leftover `0.1.1` version
  references and old script-name invocations.
- Created this `CHANGELOG.md`.
- Updated `docs/API_REFERENCE.md` with the `Accept-Language` header
  documentation, localized error-response examples, and the previously
  undocumented `/health/sdforge` endpoint.
- Updated `docs/ARCHITECTURE.md` with mermaid diagrams for the
  `EtcdClientOps` and `ConfigManagementService` trait hierarchies and the
  i18n module's position in the system.
- Updated `docs/DEPLOYMENT.md` with full `scripts/run.sh` subcommand
  documentation and the new `LOCALE` environment variable.
- Verified `cargo doc --no-deps --all-features` produces zero warnings
  and that no stale `0.1.1` version strings or legacy script names remain
  in `docs/`, `README.md`, `README_zh.md`, or `CHANGELOG.md` (outside this
  historical section).

### Phase 11 - 0.2.0 Release

- Pushed all v0.2.0 commits to `main` and confirmed all five GitHub
  Actions workflows are green.
- Raised the CI coverage gate back to `--fail-under-lines 95` as the
  final quality gate for v0.2.0.
- Tagged `v0.2.0` with annotated tag
  `Release v0.2.0: coverage 95%+, ICU i18n, dead code cleanup, scripts consolidation, CI green`
  and pushed the tag.
- Published the GitHub release via `gh release create v0.2.0 --notes-file CHANGELOG.md`.
- Verified `gh release view v0.2.0 --json tagName,name,publishedAt,url`
  returns non-empty fields and `cargo run -- --version` prints
  `nebulaid 0.2.0`.
