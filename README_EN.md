<div align="center">

# 🚀 Nebula ID

[![GitHub release](https://img.shields.io/github/v/release/Kirky-X/NebulaId)](https://github.com/Kirky-X/NebulaId/releases) [![License](https://img.shields.io/badge/license-Apache--2.0-green)](./LICENSE) [![CI](https://img.shields.io/github/actions/workflow/status/Kirky-X/NebulaId/ci.yml?branch=main)](https://github.com/Kirky-X/NebulaId/actions/workflows/ci.yml) [![Security](https://img.shields.io/github/actions/workflow/status/Kirky-X/NebulaId/codeql.yml?branch=main&label=security)](https://github.com/Kirky-X/NebulaId/actions/workflows/codeql.yml)

**[中文](README.md)** | English

**Enterprise-grade high-performance distributed ID generation system**

[✨ Features](#-features) • [🚀 Quick Start](#-quick-start) • [📚 Documentation](#-documentation) • [💻 Examples](#-examples) • [🤝 Contributing](#-contributing)

</div>

---

<div align="center">

### 🎯 One Service, Three ID Algorithms

Double-buffered segments, drift-guarded bit slicing, custom UUID v8 layouts — served over HTTP and gRPC alike:

<table style="width:100%; border-collapse: collapse">
<tr>
<td align="center" width="25%">🧮<br><b>Segment</b><br><span style="color:#64748B">Double buffer · dynamic step</span></td>
<td align="center" width="25%">❄️<br><b>Snowflake</b><br><span style="color:#64748B">Bit slicing · clock-drift guard</span></td>
<td align="center" width="25%">🧬<br><b>UUID v8</b><br><span style="color:#64748B">Custom layout · trend-ordered</span></td>
<td align="center" width="25%">🔐<br><b>API Security</b><br><span style="color:#64748B">Key auth · rate limit · audit</span></td>
</tr>
</table>

</div>

---

## 📋 Table of Contents

- [✨ Features](#-features)
- [🎯 Use Cases](#-use-cases)
- [🚀 Quick Start](#-quick-start)
- [📚 Documentation](#-documentation)
- [💻 Examples](#-examples)
- [🏗️ Architecture](#️-architecture)
- [⚙️ Configuration](#️-configuration)
- [🌐 Internationalization](#-internationalization)
- [🛠️ scripts/run.sh Usage](#️-scriptsrunsh-usage)
- [🧪 Testing](#-testing)
- [📊 Performance](#-performance)
- [🔒 Security](#-security)
- [🗺️ Roadmap](#️-roadmap)
- [🤝 Contributing](#-contributing)
- [📋 Changelog](#-changelog)
- [📄 License](#-license)
- [🙏 Acknowledgments](#-acknowledgments)
- [📞 Contact & Support](#-contact--support)
- [⭐ Star History](#-star-history)

---

## ✨ Features

<table style="width:100%; border-collapse: collapse">
<tr>
<td width="50%" style="vertical-align:top; padding: 12px">🔢 <b>Multi-Algorithm Engine</b><br><span style="color:#64748B">Segment (double buffering + dynamic step), Snowflake (configurable bit layout + clock-drift protection), UUID v8 (RFC 9562 §5.8), routed at runtime by <code>workspace / group / biz_tag</code></span></td>
<td width="50%" style="vertical-align:top; padding: 12px">🌐 <b>Dual-Protocol Access</b><br><span style="color:#64748B">HTTP/HTTPS REST and gRPC/gRPCS share one algorithm router; the sdforge <code>#[forge]</code> macro emits OpenAPI documentation automatically</span></td>
</tr>
<tr>
<td width="50%" style="vertical-align:top; padding: 12px">🧩 <b>Embedded SDK</b><br><span style="color:#64748B">The <code>sdk</code> feature provides <code>NebulaIdKit</code>: trait-kit AsyncKit dependency-graph assembly with missing-dependency errors raised in <code>build()</code>; the <code>embedded</code> example runs with zero DB and zero network</span></td>
<td width="50%" style="vertical-align:top; padding: 12px">🏗️ <b>Distributed Coordination</b><br><span style="color:#64748B">The <code>etcd</code> feature adds worker allocation and coordination, falling back to in-process local locks when endpoints are empty</span></td>
</tr>
<tr>
<td width="50%" style="vertical-align:top; padding: 12px">🔐 <b>garrison Authentication</b><br><span style="color:#64748B">The <code>garrison-auth</code> feature takes over API key verification: Argon2id hashing, constant-time comparison, in-process cache, and rotation grace period</span></td>
<td width="50%" style="vertical-align:top; padding: 12px">🚦 <b>Global Rate Limiting</b><br><span style="color:#64748B">Token bucket genuinely mounted on the HTTP stack, <code>burst ≤ 10 × rps</code> startup validation, <code>x-ratelimit-*</code> response headers, hot updates via the management endpoint</span></td>
</tr>
<tr>
<td width="50%" style="vertical-align:top; padding: 12px">📊 <b>Observability</b><br><span style="color:#64748B">Prometheus <code>/metrics</code> exposes per-algorithm p50/p99/p999 and <code>clock_backwards</code>, plus OTLP tracing and health check endpoints</span></td>
<td width="50%" style="vertical-align:top; padding: 12px">🌍 <b>ICU Internationalization</b><br><span style="color:#64748B">unify-rust-i18n (Fluent/ICU stack) with <code>Accept-Language</code> negotiation (RFC 7231 §5.3.5); full error and log translations for <code>en</code> and <code>zh-CN</code></span></td>
</tr>
<tr>
<td width="50%" style="vertical-align:top; padding: 12px">🛡️ <b>Transport & Response Security</b><br><span style="color:#64748B">rustls TLS 1.2/1.3 (<code>min_tls_version</code> enforced), security headers, strict CORS, trusted-proxy IP attribution</span></td>
<td width="50%" style="vertical-align:top; padding: 12px">🧾 <b>Audit Logging</b><br><span style="color:#64748B">Full audit trail for ID generation and key operations, with client IPs resolved to real peer connection addresses</span></td>
</tr>
<tr>
<td width="50%" style="vertical-align:top; padding: 12px">⚙️ <b>Config fail-fast</b><br><span style="color:#64748B">All 17 config structs carry <code>deny_unknown_fields</code>; a bad config aborts startup with exit code 1; <code>${VAR}</code> environment expansion built in</span></td>
<td width="50%" style="vertical-align:top; padding: 12px">🧰 <b>Unified Script Entry</b><br><span style="color:#64748B"><code>scripts/run.sh</code> dispatches deploy / lint / redis-test / api-test / install-hooks, identical locally and in CI</span></td>
</tr>
</table>

Beyond the core capabilities above, engine declarations other than PostgreSQL (the runtime config enum keeps `sqlite`/`mysql` variants, but the build contains no matching driver, so connecting fails), the `integration-tests` gate for tests that need a real database, hot-reload watching, and Redis caching are also provided via configuration or features; see the [⚙️ Configuration](#️-configuration) section and the [Config Migration Guide](docs/CONFIG_MIGRATION_GUIDE.md) for itemized details.

---

## 🎯 Use Cases

### 💼 Embed into Existing Rust Services

Ship ID generation as an in-process library with no separate deployment (adapted from [`examples/embedded.rs`](examples/embedded.rs)):

```rust
use nebulaid::core::Config;
use nebulaid::sdk::NebulaIdKitBuilder; // feature `sdk`

#[tokio::main]
async fn main() -> nebulaid::core::Result<()> {
    let mut config = Config::default();
    config.algorithm.default = "snowflake".to_string();

    let kit = NebulaIdKitBuilder::new(config).build().await?;
    let generator = kit.id_generator()?;

    // Default algorithm (`config.algorithm.default`)
    let id = generator.generate("prod", "core", "order").await?;

    // Or pin one algorithm per call
    let uuid = generator
        .generate_with_algorithm(nebulaid::core::types::AlgorithmType::UuidV8,
                                 "prod", "core", "trace")
        .await?;

    println!("snowflake={id} uuid_v8={uuid}");
    kit.shutdown().await;
    Ok(())
}
```

### 🔧 Unique Identifiers for Microservices

`Id` is a zero-cost wrapper around UUID that converts losslessly in both directions — ideal for passing and storing identifiers across services:

```rust
use nebulaid::core::types::Id;
use uuid::Uuid;

// Any Uuid is wrapped into a Nebula `Id` via the single constructor `from_uuid_v8`
let id = Id::from_uuid_v8(Uuid::now_v7());
let id_string = id.to_string(); // renders as a standard 36-char UUID string

// Converts back losslessly
let uuid = id.to_uuid_v8();
```

### ⚡ High-Throughput Batch Generation

Segment's double buffering is internal — from the outside you just ask for N ids in one call (`IdAlgorithm::batch_generate`):

```rust
let batch = generator.batch_generate("prod", "core", "order", 1000).await?;
println!("{} ids via {:?}", batch.len(), batch.algorithm);
```

---

## 🚀 Quick Start

### 📦 Installation

```bash
# Clone and build (default features: postgresql + http + grpc + garrison-auth)
git clone https://github.com/Kirky-X/NebulaId.git
cd NebulaId
cargo build --release

# Run the server (reads config/config.toml by default; --config overrides the path)
./target/release/nebula-id
```

| Feature | Default | Description |
|---------|:----:|------|
| `postgresql` | ✅ | dbnexus PostgreSQL storage backend (can be turned off with `--no-default-features`; compile-only guarantee, a real PostgreSQL is needed at runtime) |
| `http` / `grpc` | ✅ | REST and gRPC access (sdforge mirror features) |
| `garrison-auth` | ✅ | garrison takes over API key verification (falling back to the hand-written Argon2id path when disabled) |
| `etcd` | ➖ | etcd distributed coordination |
| `sdk` | ➖ | Embedded SDK facade (`NebulaIdKit`, implies `openapi`) |
| `integration-tests` | ➖ | Gates `#[ignore]` tests that need a real database |
| `alerting` | ➖ | Gates the alerting subsystem (`src/core/monitoring`, not compiled into production builds by default) |

> 💡 `--all-features` **is buildable** (= default + etcd + alerting + sdk + integration-tests + openapi); CI and docs uniformly adopt that switch.

### 💡 Minimal Example

The example below is adapted from [`examples/embedded.rs`](examples/embedded.rs) and generates IDs with zero DB and zero network:

```rust
use nebulaid::core::Config;
use nebulaid::sdk::NebulaIdKitBuilder;

#[tokio::main]
async fn main() -> nebulaid::core::Result<()> {
    // Pure algorithms (snowflake/uuid_v8) work with Config::default();
    // segment requires NebulaIdKitBuilder::with_repository(..) because it
    // allocates number ranges from the database.
    let mut config = Config::default();
    config.algorithm.default = "snowflake".to_string();

    let kit = NebulaIdKitBuilder::new(config).build().await?;
    let generator = kit.id_generator()?;

    for _ in 0..5 {
        let id = generator.generate("embedded", "demo", "order").await?;
        println!("Generated ID: {id}");
    }

    kit.shutdown().await;
    Ok(())
}
```

```bash
cargo run --package nebulaid --example embedded --features sdk
```

### 🧭 Core Concepts

- **Triple namespace**: `generate(workspace_id, group_id, biz_tag)`; the workspace is the tenant isolation boundary (biz-tags queries are filtered by role).
- **Algorithm selection**: `[algorithm].default` is the global default and `generate_with_algorithm` overrides per call; pure algorithms (snowflake/uuid_v8) work with zero DB, while segment requires an injected repository.
- **Dual protocol, one source**: HTTP and gRPC share the same algorithm router and auth middleware; unauthenticated gRPC requests receive `Unauthenticated`.
- **Kit assembly**: the SDK validates a trait-kit AsyncKit dependency graph (missing dependencies or cycles fail in `build()`), and the `IdGenerator` handle is `Clone`-able for sharing across tasks.
- **Degradation fallback**: when the primary algorithm is unavailable, generation falls back along the `[Snowflake, UuidV8]` chain.

---

## 📚 Documentation

| Document | Description |
|------|------|
| [📖 User Guide](docs/USER_GUIDE.md) | Complete tutorial from installation to advanced usage |
| [📘 API Reference](docs/API_REFERENCE.md) | HTTP / gRPC endpoints, headers, error codes, and type definitions |
| [🏗️ Architecture](docs/ARCHITECTURE.md) | Module dependencies, external library roles, and algorithm design |
| [🔧 Config Migration Guide](docs/CONFIG_MIGRATION_GUIDE.md) | Full config table, validation rules, and version migrations |
| [🚀 Deployment Guide](docs/DEPLOYMENT.md) | Docker deployment, environment variables, monitoring, and script subcommands |
| [📈 Performance Guide](docs/PERFORMANCE.md) | Benchmark methodology, hot-path design, and optimization tips |
| [🔒 Security](docs/SECURITY.md) | Security design, supply chain gates, and vulnerability handling |
| [🧪 Test Scenario Matrix](docs/TEST_SCENARIOS.md) | Layered test strategy and scenario matrix |
| [❓ FAQ](docs/FAQ.md) | Frequently asked questions |
| [📋 Changelog](docs/CHANGELOG.md) | Change records for every release |
| [🤝 Contributing Guide](docs/CONTRIBUTING.md) | How to participate in project development |

---

## 💻 Examples

All examples live in the [`examples/`](examples/) directory and are built only with the `sdk` feature enabled:

```bash
# Embedded SDK: generate IDs with zero DB and zero network
cargo run --package nebulaid --example embedded --features sdk

# SDK server: sdforge #[forge] wrapper + OpenAPI documentation
cargo run --package nebulaid --example sdk_server --features sdk,http
```

### 🧩 embedded.rs

`NebulaIdKitBuilder` assembly → `id_generator()` handle → `generate` / `batch_generate` → `shutdown` for graceful teardown. Ideal for embedding ID generation directly into an existing Rust service.

### 🚀 sdk_server.rs

The same Kit exposed as an HTTP service via the sdforge `#[forge]` macro with OpenAPI routes registered (`/api-docs/openapi.json`) — a one-line switch from "library" to "service" for validating dual-protocol access.

---

## 🏗️ Architecture

Nebula ID follows a "server → core → self-developed ecosystem" layered design: `src/server/` hosts the HTTP (axum) and gRPC (tonic) endpoints, middleware (auth, rate limiting, CORS, locale, security headers), and handlers; `src/core/` contains the algorithm router with the three algorithm implementations, database repositories, the etcd coordinator, monitoring, and i18n; databases, configuration, logging, caching, rate limiting, service discovery, and authentication are delegated to the same-author ecosystem of `dbnexus` / `confers` / `inklog` / `oxcache` / `limiteron` / `sdforge` / `garrison` / `trait-kit`. Segment draws number ranges from the database with double buffering and a dynamic step, Snowflake serializes `(last_timestamp, sequence)` migrations to prevent concurrent duplicates, and a degradation chain takes over when the primary algorithm is unavailable.

For the architecture diagram, module dependencies, external library roles, the `mod.rs` interface isolation standard (rule 25), and trait relationship diagrams, see the [Architecture doc](docs/ARCHITECTURE.md).

---

## ⚙️ Configuration

`Config` spans the ten sections `app`, `database`, `etcd`, `auth`, `algorithm`, `monitoring`, `logging`, `rate_limit`, `tls`, and `batch_generate`, all **required** (only `[redis]` and `[hot_reload]` may be omitted entirely); all 17 config structs carry `#[serde(deny_unknown_fields)]`, so unknown keys and missing required keys alike fail the whole file and abort startup with exit code 1. Environment variables work in two ways: `APP_HOST`, `DATABASE_URL`, `ETCD_ENDPOINTS`, and friends override the file config at startup, while `NEBULA_DATABASE_PASSWORD`, `NEBULA_API_KEY_SALT`, and friends are referenced inside the file as `${VAR}` and expanded before parsing.

The smallest fully parseable config ships as [`config/config.toml`](config/config.toml) and looks like:

```toml
[app]
name = "nebula-id"
host = "0.0.0.0"
http_port = 8080            # there is no `app.port`
grpc_port = 9091
dc_id = 0                   # 0..=31
worker_id = 0

[database]
engine = "postgresql"
host = "localhost"
port = 5432
username = "idgen"
password = "${NEBULA_DATABASE_PASSWORD}"
database = "idgen"
max_connections = 100
min_connections = 10
acquire_timeout_seconds = 30
idle_timeout_seconds = 300

[etcd]
endpoints = ["http://localhost:2379"]
connect_timeout_ms = 5000
watch_timeout_ms = 5000

[auth]
enabled = true
cache_ttl_seconds = 300

[algorithm]
default = "segment"         # segment | snowflake | uuid_v8

[monitoring]
metrics_enabled = true
metrics_path = "/metrics"
tracing_enabled = false
otlp_endpoint = ""

[logging]
level = "info"
format = "json"
include_location = true

[rate_limit]
enabled = true
default_rps = 10000
burst_size = 100            # validate(): must be <= 10 × default_rps

[tls]
enabled = false
cert_path = ""
key_path = ""
http_enabled = false
grpc_enabled = false

[batch_generate]
max_batch_size = 100        # validate(): 1..=10000
```

> ⚠️ Note: at server startup `Config::merge()` overwrites the `algorithm.segment` / `algorithm.snowflake` / `algorithm.uuid_v8` sub-tables with environment-derived values (only `algorithm.default` survives); do not rely on those sub-tables until that merge is fixed.

**The full options table (type / default / required), the `Config::validate()` rule list, and per-version migration steps live in the [Config Migration Guide](docs/CONFIG_MIGRATION_GUIDE.md).**

---

## 🌐 Internationalization

Since v0.2.0 Nebula ID ships built-in ICU internationalization (unify-rust-i18n, Fluent/ICU stack) covering runtime translation of error messages and logs:

| Locale tag | Language | Locales file | Status |
|------------|----------|--------------|--------|
| `en` | English (default) | `locales/en.yml` | ✅ Complete |
| `zh-CN` | Simplified Chinese | `locales/zh-CN.yml` | ✅ Complete |

Negotiation: `locale_middleware` parses the HTTP `Accept-Language` header (RFC 7231 §5.3.5), matches the first supported locale by descending q-value (exact match wins, then prefix match), and falls back to `en` when the header is missing; business handlers read the result via `Extension<Locale>` and translate error responses. `Locale` derives from user input and is forgeable — do **not** use it for authentication, authorization, or any security decision.

For curl examples, header semantics, and the i18n module's place in the architecture, see the [API Reference · Accept-Language](docs/API_REFERENCE.md#accept-language-请求头) and the [Architecture doc · i18n module](docs/ARCHITECTURE.md#8-i18n-模块位置).

---

## 🛠️ scripts/run.sh Usage

Since v0.2.0 all development/deployment scripts are merged into the single entry point `scripts/run.sh` (legacy scripts were renamed to `_*_impl.sh` internals and are no longer invoked directly):

| Subcommand | Alias | Purpose |
|--------|------|------|
| `deploy` | — | Deploy the full stack via docker-compose (PostgreSQL + Redis + Etcd + App) |
| `lint` / `pre-commit` | aliases of each other | Local CI pre-checks (fmt + clippy + test + security/docs/coverage) |
| `redis-test` | — | Redis integration tests (requires Redis on 6379) |
| `api-test` | — | API endpoint tests, optional `server_url` argument |
| `install-hooks` | — | Install git pre-commit hooks |
| `help` | `--help`, `-h` | Show usage |

```bash
./scripts/run.sh pre-commit            # run before every commit
./scripts/run.sh api-test http://localhost:8080
```

CI (`ci.yml` / `release.yml` / `health-check.yml`) calls the same entry point, keeping local and CI behavior identical. For the internal implementations and full arguments of each subcommand, see the [Deployment Guide · scripts/run.sh subcommands](docs/DEPLOYMENT.md#8-scriptsrunsh-子命令).

---

## 🧪 Testing

### 🎯 Test Strategy

Testing is layered: inline unit tests in `src/` (`#[cfg(test)]`), E2E modules under `src/core/tests/` organized by layer (algorithms, auth, cache, degradation, gRPC monitoring, infrastructure, server layer, and more — 13 files), the end-to-end i18n test in `tests/i18n_e2e.rs`, shell end-to-end scripts in `tests/*.sh` (API, degradation, distributed, database concurrency), and Criterion benchmarks (`benches/i18n.rs`, `benches/algorithms.rs`). For the per-domain scenario matrix and file mapping, see the [test scenario doc](docs/TEST_SCENARIOS.md).

### ▶️ Commands (matching CI)

```bash
# Full test run (CI matrix runs default / all (--all-features))
cargo test --package nebulaid --all-features

# Lint and format gates
cargo fmt --package nebulaid -- --check
cargo clippy --package nebulaid --all-features -- -D warnings

# Coverage gate: at least 95% line coverage (authoritative gate on the CI default leg,
# excluding server/proto/ generated code)
cargo llvm-cov --package nebulaid \
  --fail-under-lines 95 --ignore-filename-regex "server/proto/"

# Light compile check (no-default paths such as the garrison fallback; same as CI fmt-clippy job)
cargo check --package nebulaid --no-default-features --lib --bins

# Benchmarks
cargo bench --bench i18n
cargo bench --bench algorithms

# Shell end-to-end scripts
./scripts/run.sh api-test
```

### 📊 Test Scale

As of the v0.2.x workspace: about 1780 Rust test functions (inline in `src/` plus the `src/core/tests/` E2E modules plus `tests/i18n_e2e.rs`), 4 shell end-to-end scripts, and 2 Criterion benchmark groups (i18n hot paths with 4 benchmark functions plus ID-generation / rate-limit / auth-cache hot paths with 7 benchmark functions). The CI coverage gate requires at least 95% line coverage, and the pre-push hook enforces a local 80% gate; actual line coverage at the v0.2.0 release was 89.91%. For per-module counts and methodology, see the [test scenario doc · statistics](docs/TEST_SCENARIOS.md#统计汇总).

---

## 📊 Performance

Two Criterion benchmarks are declared in this repository: the i18n hot paths (`benches/i18n.rs`, covering translation lookups, argumented translation, error localization, and Accept-Language parsing) and the ID-generation / rate-limit / auth-cache hot paths (`benches/algorithms.rs`, covering Snowflake single and batch generation, the routing chain, the token bucket, and auth-cache hits). Reproduce with `cargo bench --bench i18n` and `cargo bench --bench algorithms`; baseline medians live in the [Performance Guide · Recorded baselines](docs/PERFORMANCE.md#已记录基线). Design-level hot-path highlights — Segment double buffering with a dynamic step, the Snowflake lock-free CAS state word, the ring-buffer p50/p99/p999 percentiles, and the release profile (thin LTO + `panic = "abort"`) — are covered in the same guide.

---

## 🔒 Security

### 🛡️ Security Design

Security design follows "authenticate → throttle → transport → audit": garrison API key authentication (Argon2id hashing + constant-time comparison + cache + rotation grace period, clamped beyond 30 days), token-bucket rate limiting (genuinely mounted on the HTTP stack with normalized 429 headers), rustls TLS (handshakes below TLS 1.3 rejected when `tls13` is selected), security headers with strict CORS, trusted-proxy X-Forwarded-For validation, gRPC auth failure code distinction (`Unauthenticated` / `PermissionDenied`), the single-admin-key guard, and biz-tags tenant isolation (IDOR fix). Mechanism-level details live in the [Security doc](docs/SECURITY.md).

### ⛓️ Supply Chain and Gates

The five-stage CI gate (fmt + clippy → cargo-deny → cargo-audit `--deny warnings` → multi-feature test matrix with coverage ≥ 95% → aggregate gate), CodeQL static analysis, the pre-commit gitleaks private-key scan, and the pre-push test + coverage gate together form the supply chain defense; for the full checklist, see the [Security doc · Supply Chain and Gates](docs/SECURITY.md#供应链与门禁).

### 🚨 Reporting Security Issues

Please do not report security vulnerabilities through public issues. Use the private GitHub [Security Advisories](https://github.com/Kirky-X/NebulaId/security/advisories/new) disclosure channel instead. See the full policy in [SECURITY.md](docs/SECURITY.md).

---

## 🗺️ Roadmap

<table style="width:100%; border-collapse: collapse">
<tr><th style="text-align:center">Status</th><th style="text-align:left">Area</th><th style="text-align:left">Items</th></tr>
<tr><td align="center">✅</td><td>Core algorithms</td><td>Segment double buffering with dynamic step, Snowflake, UUID v8, algorithm router and degradation chain</td></tr>
<tr><td align="center">✅</td><td>Service and protocols</td><td>HTTP + gRPC dual protocol, OpenAPI docs, TLS, rate limiting, garrison API key auth, audit logging</td></tr>
<tr><td align="center">✅</td><td>Observability and i18n</td><td>Per-algorithm p50/p99/p999 metrics, health checks, OTLP tracing, en / zh-CN i18n</td></tr>
<tr><td align="center">✅</td><td>Quality gates</td><td>Five-stage CI gate, coverage ≥ 95%, cargo-deny / cargo-audit / CodeQL, ~1780 tests</td></tr>
<tr><td align="center">🚧</td><td>SDK and distributed coordination</td><td>Continued hardening of the <code>sdk</code> facade (trait-kit assembly); production validation of <code>etcd</code> coordination (feature off by default)</td></tr>
<tr><td align="center">🚧</td><td>Performance engineering</td><td>Batch generation tuning, Segment routing benchmark integration (the ID-generation throughput benchmark harness and release baselines have landed — see the [Performance Guide](docs/PERFORMANCE.md))</td></tr>
<tr><td align="center">📋</td><td>Cloud native and DR</td><td>Kubernetes operator, multi-datacenter with automatic failover, dynamic algorithm switching</td></tr>
</table>

---

## 🤝 Contributing

For the detailed contribution workflow and code standards, see the [🤝 Contributing Guide](docs/CONTRIBUTING.md).

### 🛠️ Development Environment

Run `./scripts/run.sh pre-commit` before committing (or the equivalent lefthook hooks: pre-commit runs rustfmt, clippy, and the gitleaks private-key scan; pre-push runs `cargo test --package nebulaid --all-features` and the coverage gate); commit messages follow Conventional Commits. For the full environment setup, see the [Contributing Guide · getting started](docs/CONTRIBUTING.md#快速开始).

### 💖 Ways to Contribute

<table style="width:100%; border-collapse: collapse">
<tr>
<td width="33%" align="center" style="padding: 16px">

### 🐛 Report Bugs

Found an issue?<br>
<a href="https://github.com/Kirky-X/NebulaId/issues/new">Create Issue</a>

</td>
<td width="33%" align="center" style="padding: 16px">

### 💡 Feature Suggestions

Have a great idea?<br>
<a href="https://github.com/Kirky-X/NebulaId/discussions">Start Discussion</a>

</td>
<td width="33%" align="center" style="padding: 16px">

### 🔧 Submit PR

Want to contribute code?<br>
<a href="https://github.com/Kirky-X/NebulaId/pulls">Fork & PR</a>

</td>
</tr>
</table>

---

## 📋 Changelog

For the full version history, see the [📋 Changelog](docs/CHANGELOG.md) (following the [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) format and semantic versioning).

| Version | Date | Highlights |
|------|------|------|
| Unreleased | — | SDK migrated to trait-kit assembly (`NebulaIdKit`), gRPC auth on all RPCs, rate limiting genuinely mounted, TLS fail-fast, config `deny_unknown_fields` + exit code 1, key rotation grace period |
| 0.2.0 | 2026-07-23 | garrison DAO infrastructure, expanded e2e suite (95% module coverage), dbnexus / sdforge / confers architecture takeover, 3 strix security fixes |

---

## 📄 License

This project is licensed under Apache-2.0. See [LICENSE](LICENSE).

---

## 🙏 Acknowledgments

### 🌟 Core Dependencies

Nebula ID stands on the shoulders of these excellent open source projects:

| Dependency | Purpose |
|------|------|
| [tokio](https://github.com/tokio-rs/tokio) | Async runtime |
| [axum](https://github.com/tokio-rs/axum) | HTTP framework |
| [tonic](https://github.com/hyperium/tonic) | gRPC framework |
| [sea-orm](https://github.com/SeaQL/sea-orm) | Database ORM |
| [uuid](https://github.com/uuid-rs/uuid) | UUID generation |
| [rustls](https://github.com/rustls/rustls) | TLS implementation |
| [confers](https://crates.io/crates/confers) | Configuration management (Kirky.X ecosystem) |
| [dbnexus](https://crates.io/crates/dbnexus) | Database abstraction (Kirky.X ecosystem) |
| [sdforge](https://crates.io/crates/sdforge) | Service framework and OpenAPI (Kirky.X ecosystem) |
| [garrison](https://crates.io/crates/garrison) | API key authentication (Kirky.X ecosystem) |
| [trait-kit](https://crates.io/crates/trait-kit) | Module interfaces and Kit assembly (Kirky.X ecosystem) |
| [limiteron](https://crates.io/crates/limiteron) | Rate-limiting primitives (Kirky.X ecosystem) |
| [oxcache](https://crates.io/crates/oxcache) | Multi-level cache (Kirky.X ecosystem) |
| [fluent-bundle](https://crates.io/crates/fluent-bundle) | ICU internationalization (unify-rust-i18n baseline) |

### 💝 Special Thanks

Thanks to the Rust community and all [contributors](https://github.com/Kirky-X/NebulaId/graphs/contributors).

---

## 📞 Contact & Support

<table style="width:100%; max-width: 600px">
<tr>
<td align="center" width="33%">
<a href="https://github.com/Kirky-X/NebulaId/issues"><b style="color:#991B1B">Issues</b></a><br>
<span style="color:#64748B">Report bugs & issues</span>
</td>
<td align="center" width="33%">
<a href="https://github.com/Kirky-X/NebulaId/discussions"><b style="color:#1E40AF">Discussions</b></a><br>
<span style="color:#64748B">Ask questions & share ideas</span>
</td>
<td align="center" width="33%">
<a href="https://github.com/Kirky-X/NebulaId"><b style="color:#1E293B">GitHub</b></a><br>
<span style="color:#64748B">View source code</span>
</td>
</tr>
</table>

---

## ⭐ Star History

[![Star History Chart](https://api.star-history.com/svg?repos=Kirky-X/NebulaId&type=Date)](https://star-history.com/#Kirky-X/NebulaId&Date)

If you find this project useful, please consider giving it a ⭐️!

**Built by Kirky.X**

---

<sub>© 2026 Kirky.X. All rights reserved.</sub>
