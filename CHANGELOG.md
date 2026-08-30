# Changelog

All notable changes to Nebula ID are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

接线修复与 SDK 强化（specmark change `wiring-and-sdk-hardening`）。**含多项
行为变更，部署方需注意。**

### Changed（行为变更）

- **HTTP TLS fail-fast**：`tls.enabled=true` 且证书缺失/解析失败时拒绝启动
  （不再静默降级明文）；仅 `enabled=false` 允许明文。
- **gRPC 启用 API key 认证**：NebulaIdService 全 RPC（含双向流）经拦截器校验
  `authorization`（Basic/ApiKey）。**未带凭证的 gRPC 客户端将收到
  `Unauthenticated`**；`auth.enabled=false` 时放行。
- **全局限流真实生效**：`RateLimitMiddleware` 从"仅 Extension 注入（死代码）"
  改为真实挂载到 HTTP 栈；`POST /config/rate-limit` 热更新作用于实际流量。
- **认证缓存（`auth.cache_ttl_seconds`）接线**：校验先查 garrison cache-memory
  KV，未命中回源 DB；缓存值仅含 workspace_id/role/过期时间（不含 secret）。
  吊销/轮换即时失效缓存。
- **biz-tags 租户隔离（IDOR 修复）**：`GET /api/v1/biz-tags` User 角色仅见本
  workspace（忽略 workspace 参数覆盖）；Admin 按参数过滤；无仓储时不再静默
  回退 nil 查询。
- **metrics p50/p99 语义**：由"历史最大值（只增不降）"改为最近 1024 样本环形
  缓冲的真实分位数。

### Added

- **嵌入式 SDK（feature `sdk`）**：`NebulaIdClientBuilder` / `NebulaIdClient`，
  `build()` 收拢分布式锁注入 / 路由初始化 / 降级后台任务；`examples/embedded.rs`
  （零 DB 零网络）与 `examples/sdk_server.rs`（sdforge `#[forge]` 封装 +
  OpenAPI）。
- **CI/构建**：`ci.yml` clippy 与 test matrix 改单特性（`--features etcd`，
  因 dbnexus 禁止 sqlite+postgres 混用）；`docker/Dockerfile` 适配单包
  `nebulaid` 构建；新增 `openapi` 镜像特性。

### Fixed

- **snowflake 并发重复 ID 竞态**：串行化 `(last_timestamp, sequence)` 迁移，
  并发下不再产生重复。
- **降级链去重**：fallback 链 `[Snowflake, UuidV8]`（去除重复占位）。
- **main.rs 卫生**：删除非 etcd 分支重复 router 构造；hot_reload watcher 启动
  条件与 feature 解耦。

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
