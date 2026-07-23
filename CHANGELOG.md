# Changelog

All notable changes to Nebula ID are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
