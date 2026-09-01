<div align="center">

# 🚀 Nebula ID

[![GitHub release](https://img.shields.io/github/v/release/Kirky-X/NebulaId)](https://github.com/Kirky-X/NebulaId/releases) [![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-green)](./LICENSE) [![CI](https://img.shields.io/github/actions/workflow/status/Kirky-X/NebulaId/ci.yml?branch=main)](https://github.com/Kirky-X/NebulaId/actions/workflows/ci.yml) [![Security](https://img.shields.io/github/actions/workflow/status/Kirky-X/NebulaId/codeql.yml?branch=main&label=security)](https://github.com/Kirky-X/NebulaId/actions/workflows/codeql.yml)

<p align="center"><a href="./README_zh.md">中文文档</a> | <b>English</b></p>

<p align="center">
  <strong>Enterprise-grade distributed ID generation system for high-performance applications</strong>
</p>

<p align="center">
  <a href="#-features">Features</a> •
  <a href="#-quick-start">Quick Start</a> •
  <a href="#-documentation">Documentation</a> •
  <a href="#-examples">Examples</a> •
  <a href="#-contributing">Contributing</a>
</p>

</div>

---

## 📋 Table of Contents

<details open>
<summary>Click to expand</summary>

- [✨ Features](#-features)
- [🎯 Use Cases](#-use-cases)
- [🚀 Quick Start](#-quick-start)
  - [Installation](#installation)
  - [Basic Usage](#basic-usage)
- [📚 Documentation](#-documentation)
- [🎨 Examples](#-examples)
- [🏗️ Architecture](#️-architecture)
- [⚙️ Configuration](#️-configuration)
- [🧪 Testing](#-testing)
- [📊 Performance](#-performance)
- [🔒 Security](#-security)
- [🌐 Internationalization](#-internationalization)
- [🛠️ scripts/run.sh Usage](#️-scriptsrunsh-usage)
- [🗺️ Roadmap](#️-roadmap)
- [🤝 Contributing](#-contributing)
- [📄 License](#-license)
- [🙏 Acknowledgments](#-acknowledgments)

</details>

---

## ✨ Features

<table>
<tr>
<td width="50%">

### 🎯 Core Features

- ✅ **Multiple ID Algorithms** - Segment, Snowflake, UUID v8
- ✅ **Distributed Coordination** - Etcd-based leader election and coordination
- ✅ **High Availability** - Datacenter health monitoring and automatic failover
- ✅ **Type-Safe Design** - Full Rust type safety with async/await patterns

</td>
<td width="50%">

### ⚡ Advanced Features

- 🚀 **High Performance** - Million+ IDs per second with concurrent access
- 🔐 **API Security** - API key authentication and rate limiting
- 📊 **Monitoring** - Built-in metrics, health checks, and alerting
- 🌐 **Multi-Protocol** - HTTP/HTTPS REST API and gRPC/gRPCS support

</td>
</tr>
<tr>
<td width="50%">

### 🌟 v0.2.0 New Features

- 🌍 **ICU i18n** - `rust-i18n 3.1` with `Accept-Language` negotiation (RFC 7231 §5.3.5), `en` + `zh-CN` locales, 1989 `t!()` call sites
- 🔧 **Trait Abstractions** - `EtcdClientOps` & `ConfigManagementService` traits for mock-injectable business logic
- 🛡️ **SAST Hardened** - `tiangang` SAST + `diting` three-axis review, 0 CRITICAL / 0 HIGH
- 📦 **Unified Script Entry** - `scripts/run.sh` dispatches to `deploy` / `lint` / `redis-test` / `api-test` / `install-hooks` / `help`

</td>
<td width="50%">

### 🎯 v0.2.0 Quality Gates

- ✅ **0 warnings** on `cargo build --package nebulaid --features etcd` & `cargo clippy --features etcd -D warnings`
- ✅ **4000+ tests** with 89.91% line coverage (CI gate: `--fail-under-lines 95`)
- ✅ **0 dead code** findings (`cargo udeps` + `cargo rustc -W dead_code`)
- ✅ **mod.rs interface isolation** enforced (rule 25 — `mod.rs` only exposes traits + pub types)

</td>
</tr>
</table>

<div align="center">

### 🎨 Feature Highlights

</div>

```mermaid
graph LR
    A[Client Applications] --> B[Nebula ID Service]
    B --> C[Algorithm Router]
    C --> D[Segment Algorithm]
    C --> E[Snowflake Algorithm]
    C --> F[UUID v8 Algorithm]
    B --> G[Distributed Coordination]
    G --> H[Etcd]
    B --> I[Monitoring]
    I --> J[Health Checks]
    I --> K[Metrics]
```

---

## 🎯 Use Cases

<details>
<summary><b>💼 Distributed Systems</b></summary>

<br>

```rust
use nebulaid::core::types::AlgorithmType;
use nebulaid::core::Config;
use nebulaid::sdk::NebulaIdClientBuilder; // feature `sdk`

#[tokio::main]
async fn main() -> nebulaid::core::Result<()> {
    // Segment allocates number ranges from the database and needs
    // `NebulaIdClientBuilder::with_repository(..)`; pure algorithms do not.
    let mut config = Config::default();
    config.algorithm.default = "snowflake".to_string();

    let client = NebulaIdClientBuilder::new(config).build().await?;

    // Default algorithm (`config.algorithm.default`)
    let id = client.generate("prod", "core", "order").await?;

    // Or pin one algorithm per call
    let uuid = client
        .generate_with_algorithm(AlgorithmType::UuidV8, "prod", "core", "trace")
        .await?;

    println!("snowflake={id} uuid_v8={uuid}");
    client.shutdown().await;
    Ok(())
}
```

Perfect for large-scale distributed systems requiring unique, ordered identifiers with high availability.

</details>

<details>
<summary><b>🔧 Microservices</b></summary>

<br>

```rust
use nebulaid::core::types::Id;
use uuid::Uuid;

// Wrap any Uuid into a Nebula `Id` (the only constructor is `from_uuid_v8`)
let id = Id::from_uuid_v8(Uuid::now_v7());
let id_string = id.to_string(); // renders as a standard 36-char UUID string

// Random identifiers use the same constructor
let id_v4 = Id::from_uuid_v8(Uuid::new_v4());

// And convert back losslessly
let uuid = id_v4.to_uuid_v8();
```

Ideal for microservices requiring unique identifiers with different ordering guarantees.

</details>

<details>
<summary><b>🌐 High-Performance Applications</b></summary>

<br>

```rust
use nebulaid::core::Config;
use nebulaid::sdk::NebulaIdClientBuilder;

#[tokio::main]
async fn main() -> nebulaid::core::Result<()> {
    let mut config = Config::default();
    config.algorithm.default = "snowflake".to_string();
    let client = NebulaIdClientBuilder::new(config).build().await?;

    // One call, one batch. Segment's double buffering is internal — from the
    // outside you just ask for N ids at a time (`IdAlgorithm::batch_generate`).
    let batch = client.batch_generate("prod", "core", "order", 1000).await?;
    println!("{} ids via {:?}", batch.len(), batch.algorithm);

    client.shutdown().await;
    Ok(())
}
```

Great for high-performance applications requiring millions of IDs per second with low latency.

</details>

---

## 🚀 Quick Start

### Installation

<table>
<tr>
<td width="50%">

#### 🦀 Build from Source

```bash
# Clone the repository
git clone https://github.com/Kirky-X/NebulaId.git
cd NebulaId

# Build (default features: postgresql + http + grpc + garrison-auth)
cargo build --release

# Run the server
./target/release/nebula-id
```

</td>
<td width="50%">

#### 📦 Feature Flags

```toml
# Cargo.toml features
[features]
default = ["postgresql", "http", "grpc", "garrison-auth"]
postgresql = ["dbnexus/postgres"]
sqlite    = ["dbnexus/sqlite"]   # 见下方说明：当前不可单独构建
etcd      = ["dep:etcd-client"]
garrison-auth = ["dep:garrison"]
sdk       = ["openapi"]          # 嵌入式 SDK facade
# 镜像 feature：sdforge #[forge] 宏在下游 crate 求值 cfg(feature=...)
http = []
grpc = []
openapi = []
integration-tests = []           # 需要真实数据库的 #[ignore] 测试
```

**Build with specific features:**
```bash
# Default (PostgreSQL + HTTP + gRPC + garrison auth)
cargo build --release

# Maximal buildable feature set
cargo build --release --features etcd

# Embedded SDK facade (src/sdk + examples/{embedded,sdk_server})
cargo build --release --features sdk

# NOTE: sqlite is currently NOT buildable — the default feature set always
# enables dbnexus/postgres while dbnexus forbids mixing sqlite and postgres
# (compile_error). The same constraint makes the all-features build invalid.
```

</td>
</tr>
</table>

### Basic Usage

<div align="center">

#### 🎬 5-Minute Quick Start

</div>

<table>
<tr>
<td width="50%">

**Step 1: Create Configuration**

```bash
# Start from the repository sample — it is the smallest config that parses.
cp config/config.toml my-config.toml

# Then edit at least: [database].password (via ${NEBULA_DATABASE_PASSWORD}),
# [database].url / host / port, and [algorithm].default
```

</td>
<td width="50%">

**Step 2: Start The Service**

```bash
# The binary reads config/config.toml by default; --config overrides the path.
./target/release/nebula-id --config my-config.toml &

# Probe it
curl -s http://localhost:8080/health
curl -s http://localhost:8080/metrics
```

Embedding the crate as a library instead of running the server is covered by the
`examples/embedded.rs` snippet in the Complete Example below.

</td>
</tr>
</table>

<details>
<summary><b>📖 Complete Example</b></summary>

<br>

```rust
// Mirrors examples/embedded.rs — run it with:
//   cargo run --package nebulaid --example embedded --features sdk
use nebulaid::core::Config;
use nebulaid::sdk::NebulaIdClientBuilder;

#[tokio::main]
async fn main() -> nebulaid::core::Result<()> {
    // Pure algorithms only: `segment` would additionally require
    // NebulaIdClientBuilder::with_repository(..) because it allocates
    // number ranges from the database.
    let mut config = Config::default();
    config.algorithm.default = "snowflake".to_string();

    let client = NebulaIdClientBuilder::new(config).build().await?;

    for _ in 0..5 {
        let id = client.generate("embedded", "demo", "order").await?;
        println!("Generated ID: {id}");
    }

    client.shutdown().await;
    Ok(())
}
```

</details>

---

## 📚 Documentation

<div align="center">

<table>
<tr>
<td align="center" width="25%">
<a href="https://docs.rs/nebula-id">
<img src="https://img.icons8.com/fluency/96/000000/api.png" width="64" height="64"><br>
<b>API Reference</b>
</a><br>
Full API documentation
</td>
<td align="center" width="25%">
<a href="examples/">
<img src="https://img.icons8.com/fluency/96/000000/code.png" width="64" height="64"><br>
<b>Examples</b>
</a><br>
Code examples
</td>
<td align="center" width="25%">
<a href="https://github.com/nebula-id/nebula-id">
<img src="https://img.icons8.com/fluency/96/000000/github.png" width="64" height="64"><br>
<b>GitHub</b>
</a><br>
Source code
</td>
<td align="center" width="25%">
<a href="https://crates.io/crates/nebula-id">
<img src="https://img.icons8.com/fluency/96/000000/package.png" width="64" height="64"><br>
<b>Crates.io</b>
</a><br>
Package registry
</td>
</tr>
</table>

</div>

### 📖 Additional Resources

- 🎓 **Algorithm Selection** - Choosing the right ID generation algorithm
- 🔧 **Configuration Guide** - Complete configuration reference
- ❓ **FAQ** - Frequently asked questions about distributed ID generation

---

## 🎨 Examples

<div align="center">

### 💡 Real-world Examples

</div>

<table>
<tr>
<td width="50%">

#### 📝 Example 1: Segment Algorithm

```rust
use nebulaid::core::algorithm::{AlgorithmBuilder, GenerateContext, IdAlgorithm};
use nebulaid::core::types::AlgorithmType;
use nebulaid::core::Config;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // `dc_id` comes from [app]; the concrete SegmentAlgorithm type is
    // crate-internal, so it is built through the public AlgorithmBuilder.
    let mut config = Config::default();
    config.app.dc_id = 1;

    let segment = AlgorithmBuilder::new(AlgorithmType::Segment)
        .build(&config)
        .await?;

    let ctx = GenerateContext {
        workspace_id: "prod".into(),
        group_id: "core".into(),
        biz_tag: "order".into(),
        ..Default::default()
    };
    let id = segment.generate(&ctx).await?;

    println!("Generated ID: {}", id);
    Ok(())
}
```

<details>
<summary>View output</summary>

```
Generated ID: 17731488000000
```

Without an injected repository the built-in loader starts each segment at
`unix_seconds × 10000`; with a repository the ranges are allocated in the database.

</details>

</td>
<td width="50%">

#### 🔥 Example 2: Snowflake Algorithm

```rust
use nebulaid::core::algorithm::{AlgorithmBuilder, GenerateContext, IdAlgorithm};
use nebulaid::core::types::AlgorithmType;
use nebulaid::core::Config;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // dc/worker come from [app]; the bit layout comes from [algorithm.snowflake]
    let mut config = Config::default();
    config.app.dc_id = 1;
    config.app.worker_id = 1;

    let snowflake = AlgorithmBuilder::new(AlgorithmType::Snowflake)
        .build(&config)
        .await?;

    let id = snowflake.generate(&GenerateContext::default()).await?;
    println!("Generated Snowflake ID: {}", id);

    let s = &config.algorithm.snowflake;
    println!(
        "layout: timestamp({}) | dc({}) | worker({}) | seq({})",
        s.timestamp_bits(),
        s.datacenter_id_bits,
        s.worker_id_bits,
        s.sequence_bits
    );
    Ok(())
}
```

<details>
<summary>View output</summary>

```
Generated Snowflake ID: <64-bit numeric, grows with the clock>
layout: timestamp(43) | dc(3) | worker(8) | seq(10)
```

</details>

</td>
</tr>
</table>

<div align="center">

**[📂 View All Examples →](examples/)**

</div>

---

## 🏗️ Architecture

<div align="center">

### System Overview

</div>

```mermaid
graph TB
    A[Client Applications] --> B[API Gateway]
    B --> C[HTTP REST API]
    B --> D[gRPC Service]
    C --> E[Algorithm Router]
    D --> E
    E --> F[Segment Algorithm]
    E --> G[Snowflake Algorithm]
    E --> H[UUID v8]
    F --> I[(Database)]
    G --> J[Distributed Coordination]
    J --> K[Etcd]
    H --> L[(Cache)]
    E --> M[Monitoring]
    M --> N[Health Checks]
    M --> O[Metrics]
    
    style A fill:#e1f5ff
    style B fill:#b3e5fc
    style C fill:#81d4fa
    style D fill:#4fc3f7
    style E fill:#29b6f6
    style F fill:#03a9f4
    style G fill:#03a9f4
    style H fill:#03a9f4
```

<details>
<summary><b>📐 Component Details</b></summary>

<br>

| Component | Description | Status |
|-----------|-------------|--------|
| **Algorithm Router** | Routes ID generation requests to appropriate algorithm | ✅ Stable |
| **Segment Algorithm** | Database-based segment ID generation with double buffering | ✅ Stable |
| **Snowflake Algorithm** | Twitter Snowflake variant for distributed unique IDs | ✅ Stable |
| **UUID Generator** | UUID v8 (RFC 9562 §5.8 custom structured) implementation | ✅ Stable |
| **Distributed Coordination** | Etcd-based leader election and coordination | ✅ Stable |
| **Monitoring** | Health checks, metrics collection, and alerting | ✅ Stable |
| **API Gateway** | HTTP/HTTPS and gRPC/gRPCS endpoint management | ✅ Stable |

</details>

---

## ⚙️ Configuration

<div align="center">

### 🎛️ Configuration Options

</div>

**Minimal configuration that parses (`config.toml`)**

`Config` declares `app`, `database`, `etcd`, `auth`, `algorithm`, `monitoring`,
`logging`, `rate_limit`, `tls` and `batch_generate` **without** `#[serde(default)]`
(`src/core/config/app_config.rs:37-64`). A missing required field fails the whole file, and so
does any **unknown** key — every config struct carries `deny_unknown_fields`. A failed parse
aborts startup with exit code 1 (`resolve_startup_config`, `src/main.rs:542`); it no longer
degrades to `Config::default()`. Built-in defaults apply only when no `--config` was given
*and* `config/config.toml` does not exist, and that fallback emits a `warn`. Copy this shape,
do not trim it. Only `[redis]` and `[hot_reload]` may be omitted.

```toml
[app]
name = "nebula-id"
host = "0.0.0.0"
http_port = 8080                 # there is no `app.port`
grpc_port = 9091
dc_id = 0                        # 0..=31
worker_id = 0
# shutdown_timeout_seconds = 30  # optional

[database]
engine = "postgresql"            # postgresql | postgres | mysql | sqlite
host = "localhost"
port = 5432
username = "idgen"
password = "${NEBULA_DATABASE_PASSWORD}"
database = "idgen"
max_connections = 100
min_connections = 10
acquire_timeout_seconds = 30
idle_timeout_seconds = 300
# url = "postgresql://idgen:pw@localhost:5432/idgen"   # optional alternative

[etcd]
endpoints = ["http://localhost:2379"]
connect_timeout_ms = 5000
watch_timeout_ms = 5000

[auth]
enabled = true
cache_ttl_seconds = 300
# api_keys = []                  # optional; only the FIRST entry is provisioned at startup; needs key_id/key_secret/workspace (UUID or "global")/role/rate_limit/name
# api_key_salt = "..."           # optional (falls back to $NEBULA_API_KEY_SALT)
# key_rotation_grace_period_seconds = 0      # optional; 0 (default) = grace disabled

[algorithm]
default = "segment"              # segment | snowflake | uuid_v8 (no `type` key)

[algorithm.segment]
base_step = 1000
min_step = 500
max_step = 100000
switch_threshold = 0.1

[algorithm.snowflake]
datacenter_id_bits = 3
worker_id_bits = 8
sequence_bits = 10
clock_drift_threshold_ms = 1000

[algorithm.uuid_v8]
enabled = true

[monitoring]
metrics_enabled = true
metrics_path = "/metrics"
tracing_enabled = false
otlp_endpoint = ""

[logging]
level = "info"                   # trace | debug | info | warn | error
format = "json"                  # json | pretty
include_location = true

[rate_limit]
enabled = true
default_rps = 10000
burst_size = 100                 # validate(): must be <= 10 × default_rps

[tls]
enabled = false
cert_path = ""
key_path = ""
http_enabled = false
grpc_enabled = false
# ca_path = ""                   # optional
# min_tls_version = "tls13"      # optional: tls12 | tls13
# alpn_protocols = ["h2", "http/1.1"]   # optional

[batch_generate]
max_batch_size = 100             # validate(): 1..=10000

# Fully optional sections:
# [redis]
# url = "redis://localhost:6379"
# pool_size = 16                 # optional
# key_prefix = "nebula:id:"      # optional
# ttl_seconds = 600              # optional
# [hot_reload]
# auto_watch_enabled = false
```

> ⚠️ **Code reality check**: at server startup `Config::merge()` overwrites
> `algorithm.segment` / `algorithm.snowflake` / `algorithm.uuid_v8` with the values of the
> environment-derived config, which are always the defaults
> (`src/core/config/app_config.rs:393-395`, called from `src/main.rs:559`). Only
> `algorithm.default` survives; tune the three sub-tables in code until that merge is fixed.

**Environment Variables**

There is no `NEBULA_APP_*` / `NEBULA_AUTH_API_KEY` family. Two mechanisms exist:

```bash
# 1. Overridden onto the file config by `Config::load_from_env()` at startup
#    (only values that differ from the default are applied):
export APP_HOST="0.0.0.0"
export APP_HTTP_PORT="8080"
export APP_GRPC_PORT="9091"
export DC_ID="0"
export WORKER_ID="0"
export DATABASE_URL="postgresql://idgen:pass@localhost:5432/idgen"
export ETCD_ENDPOINTS="http://localhost:2379,http://localhost:22379"
export RUST_LOG="info"

# 2. Referenced as ${VAR} inside the file; expanded before parsing
#    (`Config::expand_env_vars`):
export NEBULA_DATABASE_PASSWORD="..."   # [database].password / url
export NEBULA_API_KEY_SALT="..."        # [auth].api_key_salt fallback
```

<details>
<summary><b>🔧 All Configuration Options</b></summary>

<br>

| Option | Type | `Config::default()` | Required in file | Description |
|--------|------|---------------------|------------------|-------------|
| `app.name` | String | `"nebula-id"` | ✅ | Application name |
| `app.host` | String | `"0.0.0.0"` | ✅ | Server bind address |
| `app.http_port` | u16 | `8080` | ✅ | HTTP port, must be > 0 |
| `app.grpc_port` | u16 | `9091` | ✅ | gRPC port, must be > 0 |
| `app.dc_id` | u8 | `0` | ✅ | Datacenter ID, must be ≤ 31 |
| `app.worker_id` | u8 | `0` | ✅ | Worker ID |
| `app.shutdown_timeout_seconds` | u64 | `30` | ➖ | Graceful shutdown timeout, must be > 0 |
| `database.engine` | String | `"postgresql"` | ✅ | `postgresql` / `postgres` / `mysql` / `sqlite` |
| `database.host` / `port` / `username` / `password` / `database` | — | `localhost` / `5432` / `idgen` / `$NEBULA_DATABASE_PASSWORD` / `idgen` | ✅ | Individual connection settings |
| `database.url` | String | `""` | ➖ | Full-URL alternative to the fields above |
| `database.max_connections` | u32 | `100` | ✅ | Pool size, must be > 0 |
| `database.min_connections` | u32 | `10` | ✅ | Must be ≤ `max_connections` |
| `database.acquire_timeout_seconds` | u64 | `30` | ✅ | Must be > 0 |
| `database.idle_timeout_seconds` | u64 | `300` | ✅ | Idle connection timeout |
| `redis` | table | — | ➖ | Whole section is optional |
| `redis.url` | String | `redis://$REDIS_URL` or `redis://localhost:6379` | ✅ if `[redis]` is present | Redis connection URL |
| `redis.pool_size` / `key_prefix` / `ttl_seconds` | u32 / String / u64 | `16` / `"nebula:id:"` / `600` | ➖ | Cache tuning |
| `etcd.endpoints` | Vec&lt;String&gt; | `["etcd:2379"]` | ✅ | `[]` disables etcd → `LocalDistributedLock` |
| `etcd.connect_timeout_ms` / `watch_timeout_ms` | u64 | `5000` / `5000` | ✅ | etcd timeouts |
| `auth.enabled` | bool | `true` | ✅ | Gates the API-key middleware |
| `auth.cache_ttl_seconds` | u64 | `300` | ✅ | Auth cache TTL |
| `auth.api_keys` | array | `[]` | ➖ | Entry = `key_id`, `key_secret`, `workspace`, `role`, `rate_limit`, `name` (all required). Startup provisions **only the first** entry; `workspace` must be a UUID string or `global`; `role` is admin only for the exact value `admin` |
| `auth.api_key_salt` | String | `$NEBULA_API_KEY_SALT` or `""` | ➖ | Salt for key hashing |
| `auth.key_rotation_grace_period_seconds` | u64 | `0` (grace disabled) | ➖ | Set `> 0` to keep the previous credential valid that long after a rotation; `> 30 days` is clamped to 30 days with a warning; requires the two grace columns, which startup migrations add automatically (manual DDL only if the DB role lacks DDL grants; see `docs/CONFIG_MIGRATION_GUIDE.md`) |
| `algorithm.default` | String | `"segment"` | ✅ | `segment` / `snowflake` / `uuid_v8` |
| `algorithm.segment.base_step` / `min_step` / `max_step` / `switch_threshold` | u64 / u64 / u64 / f64 | `1000` / `500` / `100000` / `0.1` | ✅ | Dynamic step sizing (see note above about `merge`) |
| `algorithm.snowflake.datacenter_id_bits` / `worker_id_bits` / `sequence_bits` / `clock_drift_threshold_ms` | u8 / u8 / u8 / u64 | `3` / `8` / `10` / `1000` | ✅ | Bit layout; remainder = timestamp bits |
| `algorithm.uuid_v8.enabled` | bool | `true` | ✅ | UUID v8 switch |
| `monitoring.metrics_enabled` / `metrics_path` / `tracing_enabled` / `otlp_endpoint` | bool / String / bool / String | `true` / `"/metrics"` / `false` / `""` | ✅ | Prometheus + OTLP |
| `logging.level` / `format` / `include_location` | String / String / bool | `"info"` / `"json"` / `true` | ✅ | `level`: trace…error, `format`: json/pretty |
| `rate_limit.enabled` | bool | `true` | ✅ | Enable rate limiting |
| `rate_limit.default_rps` | u32 | `10000` | ✅ | Requests per second, must be > 0 when enabled |
| `rate_limit.burst_size` | u32 | `100` | ✅ | Must be ≤ 10 × `default_rps` when enabled |
| `hot_reload` | table | `auto_watch_enabled = false` | ➖ | Whole section is optional |
| `tls.enabled` / `cert_path` / `key_path` / `http_enabled` / `grpc_enabled` | bool / String / String / bool / bool | `false` / `""` / `""` / `false` / `false` | ✅ | TLS for HTTP and gRPC |
| `tls.ca_path` | String? | `null` | ➖ | Optional CA bundle |
| `tls.min_tls_version` | String | `"tls13"` | ➖ | `tls12` / `tls13` |
| `tls.alpn_protocols` | Vec&lt;String&gt; | `["h2", "http/1.1"]` | ➖ | ALPN list |
| `batch_generate.max_batch_size` | u32 | `100` | ✅ | Must be in 1..=10000 |

Defaults are the values of `Config::default()`; **required** columns show whether the key
carries a serde default. All 17 config structs carry `#[serde(deny_unknown_fields)]`, so an
unknown key is rejected exactly like a missing required key: both fail parsing of the **entire**
file and abort startup. A mistyped section name can no longer be silently dropped.

### Validation Rules

`Config::validate()` (`src/core/config/app_config.rs:173-293`) runs right after parsing; a
violation makes `Config::load_from_file` return `ConfigError::InvalidValue`, which at server
startup aborts the process with exit code 1, naming both the file path and the violated rule
(e.g. `failed to load configuration from 'config/config.toml': Invalid configuration value:
HTTP port must be between 1 and 65535`):

| Rule | Source |
|------|--------|
| `http_port > 0`, `grpc_port > 0`, `shutdown_timeout_seconds > 0` | `Config::validate` |
| `dc_id <= 31` | `Config::validate` |
| `max_connections > 0`, `min_connections <= max_connections`, `acquire_timeout_seconds > 0` | `Config::validate` |
| `rate_limit.enabled` ⇒ `default_rps > 0`, `burst_size > 0`, `burst_size <= 10 × default_rps` | `Config::validate` |
| `algorithm.default ∈ {segment, snowflake, uuid_v8}` | `Config::validate` |
| `segment.min_step <= segment.max_step` and `min_step <= base_step <= max_step` | `Config::validate` |
| `0.0 <= segment.switch_threshold <= 1.0` | `Config::validate` |
| `snowflake.datacenter_id_bits + worker_id_bits + sequence_bits < 64` (default ⇒ 43 timestamp bits) | `Config::validate` |
| `snowflake.clock_drift_threshold_ms > 0` | `Config::validate` |
| `1 <= batch_generate.max_batch_size <= 10000` | `Config::validate` |

> Full reference: [`config/config.toml`](config/config.toml) and
> [CONFIG_MIGRATION_GUIDE.md](docs/CONFIG_MIGRATION_GUIDE.md).

</details>

---

## 🧪 Testing

<div align="center">

### 🎯 Test Coverage

</div>

```bash
# Run all tests
cargo test --features etcd

# Run with coverage
cargo tarpaulin --out Html

# Run specific test
cargo test test_name

# Run integration tests
cargo test --test integration

# Run pre-commit checks (format, lint, build, test, security, docs, coverage)
./scripts/run.sh pre-commit
```

<details>
<summary><b>📊 Test Statistics</b></summary>

<br>

| Category | Tests | Coverage |
|----------|-------|----------|
| Unit Tests | 4000+ | 89.91% |
| Integration Tests | 42 | 89.91% |
| **Total** | **4000+** | **89.91%** |

> Since v0.2.0, the CI coverage gate has been raised to `--fail-under-lines 95` (see `.github/workflows/ci.yml`). Actual line coverage as of v0.2.0 release is 89.91%; the gate enforces the floor, not the current value.

</details>

---

## 📊 Performance

<div align="center">

### ⚡ Benchmark Results

</div>

<table>
<tr>
<td width="50%">

**ID Generation Throughput**

```
Segment: 100,000+ IDs/sec
Snowflake: 1,000,000+ IDs/sec
UUID v8: 500,000+ IDs/sec
```

</td>
<td width="50%">

**Latency (P99)**

```
Segment: ~0.5ms
Snowflake: ~0.1ms
UUID v8: ~0.05ms
```

</td>
</tr>
</table>

<details>
<summary><b>📈 Detailed Benchmarks</b></summary>

<br>

```bash
# Run the benchmarks shipped by this repository
cargo bench --bench i18n
```

The only Criterion target declared in `Cargo.toml` is `i18n` (`benches/i18n.rs`);
there is no ID-generation benchmark harness, so no `*_next_id` numbers are published here.

</details>

---

## 🔒 Security

<div align="center">

### 🛡️ Security Features

</div>

<table>
<tr>
<td align="center" width="33%">
<img src="https://img.icons8.com/fluency/96/000000/lock.png" width="64" height="64"><br>
<b>API Authentication</b><br>
API key-based authentication with timing attack protection
</td>
<td align="center" width="33%">
<img src="https://img.icons8.com/fluency/96/000000/security-checked.png" width="64" height="64"><br>
<b>Rate Limiting</b><br>
Configurable rate limits to prevent abuse
</td>
<td align="center" width="33%">
<img src="https://img.icons8.com/fluency/96/000000/privacy.png" width="64" height="64"><br>
<b>Audit Logging</b><br>
Track all ID generation operations
</td>
</tr>
</table>

<details>
<summary><b>🔐 Security Details</b></summary>

<br>

### Security Measures

- ✅ **API Key Authentication** - Secure API access with API key authentication using constant-time comparison to prevent timing attacks
- ✅ **Rate Limiting** - Configurable rate limits to prevent abuse and DoS attacks (max batch size: 100)
- ✅ **Audit Logging** - Full operation tracking for compliance and monitoring with IP spoofing protection
- ✅ **TLS Support** - HTTPS and gRPCS for encrypted communication (TLS 1.2/1.3)
- ✅ **CORS Restrictions** - Strict cross-origin resource sharing policies
- ✅ **Security Headers** - X-Content-Type-Options, X-Frame-Options, CSP, HSTS, X-XSS-Protection, Referrer-Policy
- ✅ **IP Spoofing Protection** - Trusted proxy validation for X-Forwarded-For headers

### Feature Flags

```toml
# Cargo package name is `nebulaid`; the audit logger and TLS are runtime
# configuration ([auth] / [tls]), not Cargo features.
[dependencies]
nebulaid = { version = "0.2", features = ["sdk"] }      # embedded client facade
# nebulaid = { version = "0.2", features = ["etcd"] }   # distributed coordination
# Available features: postgresql (default), etcd, garrison-auth (default),
# sdk, http/grpc/openapi (sdforge mirrors, http+grpc default), integration-tests.
```

</details>

---

## 🌐 Internationalization

<div align="center">

### 🌍 ICU i18n Support (new in v0.2.0)

</div>

Nebula ID ships with built-in ICU internationalization since v0.2.0, powered by [`rust-i18n`](https://crates.io/crates/rust-i18n) `3.1`. It covers runtime translation of error messages and log entries.

**Supported locale matrix:**

| Locale tag | Language | Locales file | Status |
|------------|----------|--------------|--------|
| `en` | English (default) | `locales/en.yml` | ✅ Complete |
| `zh-CN` | Simplified Chinese | `locales/zh-CN.yml` | ✅ Complete |

**Negotiation flow:**

1. The client declares preferred languages via the HTTP `Accept-Language` header (per [RFC 7231 5.3.5](https://www.rfc-editor.org/rfc/rfc7231#section-5.3.5)), e.g. `Accept-Language: zh-CN,zh;q=0.9,en;q=0.8`.
2. `locale_middleware` (`src/server/middleware/locale.rs`) parses the header, sorts candidates by descending q-value, and picks the first supported locale (exact match wins; otherwise prefix match such as `zh` → `zh-CN`).
3. On missing header or no match, the default locale `en` is used.
4. Business handlers read the negotiated result via `Extension<Locale>` and translate error response messages with `translate_with_locale_args`.

**curl examples:**

```bash
# Chinese error response
curl -H "Accept-Language: zh-CN" http://localhost:8080/api/v1/invalid
# {
#   "code": 404,
#   "message": "未找到路径",
#   "details": "..."
# }

# English error response (default)
curl http://localhost:8080/api/v1/invalid
# {
#   "code": 404,
#   "message": "Path not found",
#   "details": "..."
# }
```

> **Security note**: `Locale` is derived from user input (the `Accept-Language` header) and is forgeable. Do **not** use it for any authentication, authorization, or security decision it is intended solely for content negotiation.

See [API Reference  Accept-Language](docs/API_REFERENCE.md#accept-language-header) and [Architecture  i18n module](docs/ARCHITECTURE.md#8-i18n-module-position) for details.

---

## 🛠️ scripts/run.sh Usage

<div align="center">

### 📦 Unified Script Entry (new in v0.2.0)

</div>

Since v0.2.0 all development/deployment scripts are merged into a single entry point `scripts/run.sh`, replacing the scattered v0.1.x scripts (`deploy`, `pre-commit-check`, `redis_test`, `test_api`, `install-pre-commit-hooks`, etc.). The legacy scripts have been renamed to `_*_impl.sh` internal implementations and are no longer invoked directly.

**Subcommand overview:**

| Subcommand | Alias | Purpose | Internal impl |
|------------|-------|---------|---------------|
| `deploy` | — | Deploy Nebula ID via docker-compose | `_deploy_impl.sh` |
| `lint` | `pre-commit` | Run local CI pre-checks (fmt + clippy + test + security/docs/coverage) | `_pre_commit_impl.sh` |
| `redis-test` | — | Run Redis integration tests | `_redis_test_impl.sh` |
| `api-test` | — | Run API endpoint tests, optional `server_url` argument | `tests/api_test.sh` |
| `install-hooks` | — | Install git pre-commit hooks | `_install_hooks_impl.sh` |
| `pre-commit` | `lint` | Same as `lint`, runs local CI pre-checks | `_pre_commit_impl.sh` |
| `help` | `--help`, `-h` | Show usage information | — |

**Examples:**

```bash
# Show help
./scripts/run.sh help

# Deploy (docker-compose full stack)
./scripts/run.sh deploy

# Local CI pre-checks (must run before commit)
./scripts/run.sh pre-commit
# Or the equivalent alias
./scripts/run.sh lint

# Redis integration tests (requires Redis listening on 6379)
./scripts/run.sh redis-test

# API endpoint tests (defaults to http://localhost:8080)
./scripts/run.sh api-test
# Specify server URL
./scripts/run.sh api-test http://localhost:8080

# Install git pre-commit hooks
./scripts/run.sh install-hooks
```

**GitHub Actions integration:**

CI calls go through the same entry point (see `.github/workflows/ci.yml`, `release.yml`, `health-check.yml`), keeping local and CI behavior identical.

See [Deployment Guide  scripts/run.sh Subcommands](docs/DEPLOYMENT.md#8-scriptsrunsh-subcommands) for details.

---

## 🗺️ Roadmap

<div align="center">

### 🎯 Development Timeline

</div>

<table>
<tr>
<td width="50%">

### ✅ Completed

- [x] Core ID generation algorithms
- [x] Segment algorithm with double buffering
- [x] Snowflake algorithm
- [x] UUID v8 implementation
- [x] Distributed coordination with Etcd

</td>
<td width="50%">

### 🚧 In Progress

- [ ] Enhanced monitoring and alerting
- [ ] Multi-datacenter support
- [ ] Performance optimization
- [ ] Client SDK improvements

</td>
</tr>
<tr>
<td width="50%">

### 📋 Planned

- [ ] Automatic failover
- [ ] Dynamic algorithm switching
- [ ] Custom ID format support
- [ ] Cloud provider integrations

</td>
<td width="50%">

### 💡 Future Ideas

- [ ] Kubernetes operator
- [ ] Multi-region deployment
- [ ] GraphQL API
- [ ] ID namespace management

</td>
</tr>
</table>

---

## 🤝 Contributing

<div align="center">

### 💖 We Love Contributors!

</div>

<table>
<tr>
<td width="33%" align="center">

### 🐛 Report Bugs

Found a bug?<br>
[Create an Issue](https://github.com/nebula-id/nebula-id/issues)

</td>
<td width="33%" align="center">

### 💡 Request Features

Have an idea?<br>
[Start a Discussion](https://github.com/nebula-id/nebula-id/discussions)

</td>
<td width="33%" align="center">

### 🔧 Submit PRs

Want to contribute?<br>
[Fork & PR](https://github.com/nebula-id/nebula-id/pulls)

</td>
</tr>
</table>

<details>
<summary><b>📝 Contribution Guidelines</b></summary>

<br>

### How to Contribute

1. **Fork** the repository
2. **Clone** your fork: `git clone https://github.com/yourusername/nebula-id.git`
3. **Create** a branch: `git checkout -b feature/amazing-feature`
4. **Make** your changes
5. **Test** your changes: `cargo test --features etcd`
6. **Commit** your changes: `git commit -m 'Add amazing feature'`
7. **Push** to branch: `git push origin feature/amazing-feature`
8. **Create** a Pull Request

### Code Style

- Follow Rust standard coding conventions
- Run `cargo fmt` and `cargo clippy` before committing
- Write comprehensive tests
- Update documentation

</details>

---

## 📄 License

<div align="center">

This project is licensed under dual license:

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE-MIT)
[![License: Apache 2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](LICENSE-APACHE)

You may choose either license for your use.

</div>

---

## 🙏 Acknowledgments

<div align="center">

### Built With Amazing Tools

</div>

<table>
<tr>
<td align="center" width="25%">
<a href="https://www.rust-lang.org/">
<img src="https://www.rust-lang.org/static/images/rust-logo-blk.svg" width="64" height="64"><br>
<b>Rust</b>
</a>
</td>
<td align="center" width="25%">
<a href="https://github.com/">
<img src="https://github.githubassets.com/images/modules/logos_page/GitHub-Mark.png" width="64" height="64"><br>
<b>GitHub</b>
</a>
</td>
<td align="center" width="25%">
<img src="https://img.icons8.com/fluency/96/000000/code.png" width="64" height="64"><br>
<b>Open Source</b>
</td>
<td align="center" width="25%">
<img src="https://img.icons8.com/fluency/96/000000/community.png" width="64" height="64"><br>
<b>Community</b>
</td>
</tr>
</table>

### Special Thanks

- 🌟 **Dependencies** - Built on these amazing projects:
  - [tokio](https://github.com/tokio-rs/tokio) - Async runtime
  - [axum](https://github.com/tokio-rs/axum) - HTTP framework
  - [tonic](https://github.com/hyperium/tonic) - gRPC framework
  - [sea-orm](https://github.com/SeaQL/sea-orm) - Database ORM
  - [etcd-client](https://github.com/etcd-rs/etcd-client) - Etcd client (optional, `etcd` feature)
  - [uuid](https://github.com/uuid-rs/uuid) - UUID generation
  - [confers](https://crates.io/crates/confers) - Configuration management
  - [oxcache](https://crates.io/crates/oxcache) - Multi-level cache
  - [dbnexus](https://crates.io/crates/dbnexus) - Database abstraction
  - [limiteron](https://crates.io/crates/limiteron) - Rate limiting
  - [sdforge](https://crates.io/crates/sdforge) - Service discovery
  - [prometheus-client](https://github.com/prometheus/client_rust) - Metrics

- 👥 **Contributors** - Thanks to all our amazing contributors!

---

## 📞 Contact & Support

<div align="center">

<table>
<tr>
<td align="center" width="50%">
<a href="https://github.com/nebula-id/nebula-id/issues">
<img src="https://img.icons8.com/fluency/96/000000/bug.png" width="48" height="48"><br>
<b>Issues</b>
</a><br>
Report bugs & issues
</td>
<td align="center" width="50%">
<a href="https://github.com/nebula-id/nebula-id/discussions">
<img src="https://img.icons8.com/fluency/96/000000/chat.png" width="48" height="48"><br>
<b>Discussions</b>
</a><br>
Ask questions & share ideas
</td>
</tr>
</table>

### Stay Connected

[![GitHub](https://img.shields.io/badge/GitHub-Follow-181717?style=for-the-badge&logo=github&logoColor=white)](https://github.com/nebula-id)
[![Crates.io](https://img.shields.io/badge/Crates.io-Version-DF5500?style=for-the-badge&logo=rust&logoColor=white)](https://crates.io/crates/nebula-id)

</div>

---

## ⭐ Star History

<div align="center">

[![Star History Chart](https://api.star-history.com/svg?repos=nebula-id/nebula-id&type=Date)](https://star-history.com/#nebula-id/nebula-id&Date)

</div>

---

<div align="center">

### 💝 Support This Project

If you find this project useful, please consider giving it a ⭐️!

**Built with ❤️ by the Nebula ID Team**

[⬆ Back to Top](#-nebula-id)

---

<sub>© 2025 Nebula ID. All rights reserved.</sub>
