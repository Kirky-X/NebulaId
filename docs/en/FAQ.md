# ❓ Nebula ID FAQ

> Quick answers to frequently asked questions about Nebula ID.

[🏠 Home](../../README.md) • [📖 User Guide](USER_GUIDE.md) • [🔧 API Reference](API_REFERENCE.md)

---

## 📋 Table of Contents

- [General](#-general)
- [Installation and Configuration](#-installation-and-configuration)
- [Usage and Features](#-usage-and-features)
- [Performance](#-performance)
- [Security](#-security)
- [Troubleshooting](#-troubleshooting)
- [Contributing](#-contributing)
- [License](#-license)

---

## 🧭 General

<div align="center">

### 🤔 About This Project

</div>

<details>
<summary><b>❓ What is Nebula ID?</b></summary>

<br>

**Nebula ID** is an enterprise-grade distributed ID generation system for high-performance applications. It provides:

- ✅ **Multiple ID algorithms** - Segment, Snowflake, UUID v8
- ✅ **Distributed coordination** - etcd-based leader election and coordination
- ✅ **High availability** - datacenter health monitoring and automatic failover
- ✅ **Type-safe design** - full Rust type safety with async/await patterns

It is designed for **distributed systems** that need unique, ordered, high-throughput identifier generation.

**Learn more:** the [User Guide](USER_GUIDE.md)

</details>

<details>
<summary><b>❓ Why choose it over other solutions?</b></summary>

<br>

<table>
<tr>
<th>Feature</th>
<th>Nebula ID</th>
<th>Snowflake</th>
<th>UUID</th>
</tr>
<tr>
<td>Time-ordered</td>
<td>✅ Yes</td>
<td>✅ Yes</td>
<td>⚠️ v7 only</td>
</tr>
<tr>
<td>High throughput</td>
<td>✅ 1M+ ids/s</td>
<td>✅ 1M+ ids/s</td>
<td>✅ 1M+ ids/s</td>
</tr>
<tr>
<td colspan="4">口径限定：100 万+ ID/秒 为 Snowflake / UUID v8 微基准口径；Segment 与 HTTP e2e 吞吐见 <a href="PERFORMANCE.md">性能指南</a>。</td>
</tr>
<tr>
<td>No clock sync needed</td>
<td>✅ Segment</td>
<td>❌ No</td>
<td>✅ Yes</td>
</tr>
<tr>
<td>Fault tolerance</td>
<td>✅ Built-in</td>
<td>⚠️ Manual</td>
<td>✅ Yes</td>
</tr>
</table>

**Core advantages:**
- 🚀 **Multi-algorithm**: Segment for DB-backed ordering, Snowflake for speed, UUID for simplicity
- 🔄 **Automatic failover**: datacenter health monitoring with automatic recovery
- 🛡️ **Enterprise-ready**: API authentication, rate limiting, and audit logging
- 📊 **Built-in monitoring**: health checks and metrics collection

</details>

<details>
<summary><b>❓ Can it be used in production?</b></summary>

<br>

**Current status:** ✅ **Production-ready!**

<table>
<tr>
<td width="50%">

**Ready:**
- ✅ Core ID generation algorithms (Segment, Snowflake, UUID v8)
- ✅ etcd-based distributed coordination
- ✅ Datacenter health monitoring and failover
- ✅ HTTP/HTTPS and gRPC/gRPCS APIs
- ✅ API key authentication and rate limiting

</td>
<td width="50%">

**Maturity indicators:**
- 📊 85%+ test coverage
- 🔄 Actively maintained
- 🛡️ Security-centered design
- 📖 Complete documentation

</td>
</tr>
</table>

> **Note:** always consult the [CHANGELOG](CHANGELOG.md) before upgrading.

</details>

<details>
<summary><b>❓ Which platforms are supported?</b></summary>

<br>

<table>
<tr>
<th>Platform</th>
<th>Architecture</th>
<th>Status</th>
<th>Notes</th>
</tr>
<tr>
<td rowspan="2"><b>Linux</b></td>
<td>x86_64</td>
<td>✅ Fully supported</td>
<td>The primary platform</td>
</tr>
<tr>
<td>ARM64</td>
<td>✅ Fully supported</td>
<td>Tested on ARM servers</td>
</tr>
<tr>
<td rowspan="2"><b>macOS</b></td>
<td>x86_64</td>
<td>✅ Fully supported</td>
<td>Intel Macs</td>
</tr>
<tr>
<td>ARM64</td>
<td>✅ Fully supported</td>
<td>Apple Silicon (M1/M2/M3)</td>
</tr>
<tr>
<td><b>Windows</b></td>
<td>x86_64</td>
<td>✅ Fully supported</td>
<td>Windows 10+</td>
</tr>
</table>

</details>

<details>
<summary><b>❓ Which programming languages are supported?</b></summary>

<br>

**Nebula ID** is a native **Rust** library with multi-protocol service support:

- **Rust**: the native library (the `nebula-id` crate)
- **HTTP/REST**: any language with an HTTP client
- **gRPC**: any gRPC-capable language (Python, Java, Go, etc.)

**Docs:**
- [Rust API docs](https://docs.rs/nebulaid)
- [API Reference](API_REFERENCE.md)

</details>

<details>
<summary><b>❓ Which ID algorithms are supported?</b></summary>

<br>

<table>
<tr>
<th>Algorithm</th>
<th>Format</th>
<th>Time-ordered</th>
<th>Throughput</th>
<th>Best for</th>
</tr>
<tr>
<td>Segment</td>
<td>64-bit</td>
<td>✅ Yes</td>
<td>Not benchmarked (see the performance guide's methodology boundary)</td>
<td>Database primary keys</td>
</tr>
<tr>
<td>Snowflake</td>
<td>64-bit</td>
<td>✅ Yes</td>
<td>1M+/s</td>
<td>High-performance systems</td>
</tr>
<tr>
<td>UUID v8</td>
<td>128-bit</td>
<td>✅ Yes</td>
<td>500K+/s</td>
<td>Distributed systems</td>
</tr>
</table>

> Throughput figures are indicative only — this repository has no `benches/` coverage for the UUID path yet.

</details>

---

## 📦 Installation and Configuration

<div align="center">

### 🚀 Quick Start

</div>

<details>
<summary><b>❓ How do I install it?</b></summary>

<br>

**Rust projects:**

Add to `Cargo.toml`:

```toml
[dependencies]
nebulaid = "0.3"                       # the Cargo package name is `nebulaid`
tokio = { version = "1.0", features = ["full"] }
```

Or use cargo:

```bash
cargo add nebulaid tokio
```

**Optional features** (`[features]` in `Cargo.toml`):

```toml
# default = ["server", "segment", "snowflake", "uuid-v8", "uuid-v7", "uuid-v5", "uuid-v4", "postgresql", "garrison-auth", "etcd"]
nebulaid = { version = "0.3", features = ["etcd"] }  # distributed coordination
# nebulaid = { version = "0.3", features = ["sdk"] } # the NebulaIdKit facade
# There are no `monitoring` / `audit` / `tls` features: metrics, audit logs,
# and TLS are all runtime configuration ([monitoring] / [auth] / [tls]).
```

**Verification:**

```rust
use nebulaid::core::Config;
use nebulaid::sdk::NebulaIdKitBuilder; // requires the `sdk` feature

#[tokio::main]
async fn main() -> nebulaid::core::Result<()> {
    let mut config = Config::default();
    config.algorithm.default = "snowflake".to_string();

    let kit = NebulaIdKitBuilder::new(config).build().await?;
    let generator = kit.id_generator()?;
    let id = generator.generate("verify", "install", "order").await?;
    println!("✅ Generated ID: {id}");

    kit.shutdown().await;
    Ok(())
}
```

**See also:** the [User Guide](USER_GUIDE.md#installation)

</details>

<details>
<summary><b>❓ What are the system requirements?</b></summary>

<br>

**Minimum requirements:**

<table>
<tr>
<th>Component</th>
<th>Requirement</th>
<th>Recommended</th>
</tr>
<tr>
<td>Rust version</td>
<td>1.75+</td>
<td>Latest stable</td>
</tr>
<tr>
<td>Memory</td>
<td>256MB</td>
<td>1GB+</td>
</tr>
<tr>
<td>Disk space</td>
<td>50MB</td>
<td>100MB+</td>
</tr>
<tr>
<td>Database</td>
<td>PostgreSQL</td>
<td>PostgreSQL 13+</td>
</tr>
</table>

**Optional dependencies:**
- 🔧 **etcd**: for distributed coordination (v3.4+)
- ☁️ **Redis**: for caching (v6+)
- 📊 **Prometheus**: for metrics visualization

</details>

<details>
<summary><b>❓ What about compile errors?</b></summary>

<br>

**Common fixes:**

1. **Check the Rust version:**
   ```bash
   rustc --version
   # Should be 1.75.0 or higher
   ```

2. **Make sure the required features are enabled** (defaults to
   `["server", "segment", "snowflake", "uuid-v8", "uuid-v7", "uuid-v5", "uuid-v4", "postgresql", "garrison-auth", "etcd"]`):
   ```toml
   nebulaid = "0.3"
   ```

3. **Clean build artifacts:**
   ```bash
   cargo clean
   cargo build
   ```

**Still stuck?**
- 📝 Read [Troubleshooting](#-troubleshooting)
- 🐛 [Open an issue](https://github.com/Kirky-X/NebulaId/issues) with the error details

</details>

<details>
<summary><b>❓ Can I use it with Docker?</b></summary>

<br>

**Yes!** Nebula ID runs perfectly in containerized environments.

**Dockerfile example:**

```dockerfile
FROM rust:1.97.1-bookworm AS builder
WORKDIR /app
COPY . .
RUN cargo build --release --features etcd

FROM debian:bookworm-slim
COPY --from=builder /app/target/release/nebula-id /usr/local/bin/
CMD ["nebula-id"]
```

**Docker Compose with dependencies:**

```yaml
services:
  nebula-id:
    image: nebula-id:latest
    ports:
      - "8080:8080"
    depends_on:
      - postgres
      - etcd
    environment:
      - DATABASE_URL=postgresql://user:pass@postgres/nebula
      - ETCD_ENDPOINTS=http://etcd:2379

  postgres:
    image: postgres:15
    environment:
      POSTGRES_DB: nebula

  etcd:
    image: etcd:v3.5
```

</details>

<details>
<summary><b>❓ How do I configure Nebula ID?</b></summary>

<br>

**Config file (`config.toml`):**

All ten config sections below are **required** — `Config` provides no
`#[serde(default)]` for them (`src/core/config/app_config.rs:37-64`). A parse failure now aborts
startup with exit code 1 instead of falling back to `Config::default()`; unknown keys are rejected
the same way, since every config struct carries `deny_unknown_fields`. Built-in defaults are used only when no `--config` was given *and*
`config/config.toml` does not exist, with a `warn` emitted.
Only `[redis]` and `[hot_reload]` may be omitted.

```toml
[app]
name = "nebula-id"
host = "0.0.0.0"
http_port = 8080
grpc_port = 9091
dc_id = 0
worker_id = 0

[database]
engine = "postgresql"
host = "localhost"
port = 5432
username = "idgen"
password = "${NEBULA_DATABASE_PASSWORD}"
database = "idgen"
max_connections = 50
min_connections = 5
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
default = "segment"

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
level = "info"
format = "json"
include_location = true

[rate_limit]
enabled = true
default_rps = 10000
burst_size = 100

[tls]
enabled = false
cert_path = ""
key_path = ""
http_enabled = false
grpc_enabled = false

[batch_generate]
max_batch_size = 100

# Optional:
# [redis] url = "redis://localhost:6379"
# [hot_reload] auto_watch_enabled = false
```

> ⚠️ `Config::merge()` overlays the env-derived config onto the file at startup,
> unconditionally replacing `algorithm.segment` / `algorithm.snowflake` /
> `algorithm.uuid_v8` (`src/core/config/app_config.rs:393-395`,
> `src/main.rs:559`) — in practice these three sub-tables always end up as defaults.
> Only `algorithm.default` survives. Adjust them in code for now.

**Environment variables:**

There is no `NEBULA_DATABASE_URL` / `NEBULA_AUTH_API_KEY` family. The real environment variables are:

```bash
# Overlaid on top of the file by `Config::load_from_env()`:
export APP_HOST="0.0.0.0"
export APP_HTTP_PORT="8080"
export APP_GRPC_PORT="9091"
export DC_ID="0"
export WORKER_ID="0"
export DATABASE_URL="postgresql://idgen:pass@localhost:5432/idgen"
export ETCD_ENDPOINTS="http://localhost:2379"
export RUST_LOG="info"

# Expanded inside the file to ${VAR} before parsing:
export NEBULA_DATABASE_PASSWORD="..."   # [database].password / url
export NEBULA_API_KEY_SALT="..."        # fallback for [auth].api_key_salt
```

**See also:** the [Config Migration Guide](CONFIG_MIGRATION_GUIDE.md#config-reference-all-options-and-validation-rules)

</details>

---

## 🎯 Usage and Features

<div align="center">

### 💡 API Usage

</div>

<details>
<summary><b>❓ How do I get started with basic usage?</b></summary>

<br>

**5-minute quick start:**

```rust
use nebulaid::core::Config;
use nebulaid::sdk::NebulaIdKitBuilder; // the `sdk` feature

#[tokio::main]
async fn main() -> nebulaid::core::Result<()> {
    // Segment needs `NebulaIdKitBuilder::with_repository(..)` because it
    // allocates segments from the database; pure algorithms need no extra setup.
    let mut config = Config::default();
    config.algorithm.default = "snowflake".to_string();

    let kit = NebulaIdKitBuilder::new(config).build().await?;
    let generator = kit.id_generator()?;

    // Generate a single ID
    let id = generator.generate("prod", "core", "order").await?;
    println!("Generated ID: {} (u128: {})", id, id.as_u128());

    // Batch-generate IDs
    let batch = generator.batch_generate("prod", "core", "order", 100).await?;
    println!("Generated {} IDs", batch.len());

    kit.shutdown().await;
    Ok(())
}
```

**Next steps:**
- 📖 The [User Guide](USER_GUIDE.md)
- 💻 [Examples](../../examples/)

</details>

<details>
<summary><b>❓ How do I choose the right algorithm?</b></summary>

<br>

**Algorithm selection guide:**

| Scenario | Recommended algorithm | Why |
|----------|----------------------|-----|
| Database primary keys | Segment | Ordered, DB-backed, reliable |
| High-throughput microservices | Snowflake | Fast, no database dependency |
| Time-ordered distributed IDs | UUID v8 | RFC 9562 §5.8 layout, sortable by time, embedded dc/worker/shard |
| Mixed needs | Multiple algorithms | Use different algorithms per scenario |

**Config code example:**

```rust
use nebulaid::core::algorithm::AlgorithmBuilder;
use nebulaid::core::types::AlgorithmType;
use nebulaid::core::Config;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::default();

    // Algorithm structs (SegmentAlgorithm / SnowflakeAlgorithm / UuidV8Impl) are crate-internal;
    // build them through the public AlgorithmBuilder.
    // Snowflake and UuidV8 are pure algorithms and need no database; Segment needs a
    // repository wired up first (see the SDK notes in src/sdk/client.rs).
    let snowflake = AlgorithmBuilder::new(AlgorithmType::Snowflake)
        .build(&config)
        .await?;
    let uuid_v8 = AlgorithmBuilder::new(AlgorithmType::UuidV8)
        .build(&config)
        .await?;

    Ok(())
}
```

</details>

<details>
<summary><b>❓ How does the Segment algorithm work?</b></summary>

<br>

The Segment algorithm pre-allocates ID ranges from the database for efficient batch minting:

```
┌─────────────────────────────────────────────────────────────┐
│                    Segment Algorithm                         │
├─────────────────────────────────────────────────────────────┤
│  1. Request ID range from database                          │
│  2. Pre-allocate range (e.g., 1-10000)                      │
│  3. Generate IDs from local cache                           │
│  4. When approaching limit, pre-fetch next range            │
└─────────────────────────────────────────────────────────────┘
```

**Core advantages:**
- 🚀 **High throughput**: IDs are minted from local memory
- 📦 **Batch efficiency**: pre-allocation reduces database round trips
- 🔄 **Fault tolerance**: automatic failover to healthy datacenters

**Code example:**

```rust
use nebulaid::core::algorithm::{AlgorithmBuilder, GenerateContext, IdAlgorithm};
use nebulaid::core::types::AlgorithmType;
use nebulaid::core::Config;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // `dc_id` comes from [app]; the concrete SegmentAlgorithm type is
    // crate-internal, so build it through the public AlgorithmBuilder.
    let mut config = Config::default();
    config.app.dc_id = 1;

    let segment = AlgorithmBuilder::new(AlgorithmType::Segment)
        .build(&config)
        .await?;

    let ctx = GenerateContext::default();

    // Generate a single ID (from the pre-allocated segment)
    let id = segment.generate(&ctx).await?;
    println!("Generated ID: {}", id);

    // Batch generation (one database round-trip for `size` IDs)
    let batch = segment.batch_generate(&ctx, 1000).await?;
    println!("Generated {} IDs", batch.ids.len());

    Ok(())
}
```

</details>

<details>
<summary><b>❓ How does the Snowflake algorithm work?</b></summary>

<br>

The Snowflake algorithm generates 64-bit IDs with a configurable bit allocation
(`construct_id` in `src/core/algorithm/snowflake.rs` shifts
`timestamp | datacenter | worker | sequence`, no sign bit):

```
┌────────────────────────────────────────────────────────────────┐
│              Snowflake ID Structure (defaults)                  │
├────────────────────────────────────────────────────────────────┤
│  43 bits   │  3 bits    │  8 bits  │  10 bits                   │
│  timestamp │  datacenter│  worker  │  sequence                  │
└────────────────────────────────────────────────────────────────┘
```

**Core advantages:**
- 🚀 **Fast**: no database dependency
- 📈 **Scalable**: the default bit layout supports 8 datacenters × 256 workers
  (`datacenter_id_bits` / `worker_id_bits` / `sequence_bits` are all configurable;
  their sum must stay < 64)
- 🎯 **Ordered**: sorted by time within a millisecond

**Code example:**

```rust
use nebulaid::core::algorithm::{AlgorithmBuilder, GenerateContext, IdAlgorithm};
use nebulaid::core::types::AlgorithmType;
use nebulaid::core::Config;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // dc/worker come from [app]; the concrete SnowflakeAlgorithm type is
    // crate-internal, so build it through the public AlgorithmBuilder.
    let mut config = Config::default();
    config.app.dc_id = 1;
    config.app.worker_id = 1;

    let snowflake = AlgorithmBuilder::new(AlgorithmType::Snowflake)
        .build(&config)
        .await?;

    let id = snowflake.generate(&GenerateContext::default()).await?;
    println!("Snowflake ID: {} (u128: {})", id, id.as_u128());

    // The bit layout is readable from the config; the remaining bits of the
    // 64 form the timestamp field.
    let s = &config.algorithm.snowflake;
    println!(
        "timestamp({}) | dc({}) | worker({}) | seq({})",
        s.timestamp_bits(),
        s.datacenter_id_bits,
        s.worker_id_bits,
        s.sequence_bits
    );

    Ok(())
}
```

</details>

<details>
<summary><b>❓ What is UUID v8, and when should I use it?</b></summary>

<br>

Nebula ID provides a **time-sorted UUID v8** generator (RFC 9562 §5.8 custom layout). The standard
UUID v7 also carries a 48-bit millisecond timestamp, but the remaining bits are fixed to
clock-seq + node; Nebula's v8 composition instead uses the vendor-customized fields to embed
tenant/region context directly in the ID (UUIDP "Cluster" style: a per-instance random start plus a
strictly monotonic counter):

```
┌────────────────────────────────────────────────────────────────┐
│                    UUID v8 Structure (128 bits)                 │
├────────────────────────────────────────────────────────────────┤
│  48 bits   │  16 bits (incl. version=0b1000 + variant=0b10)    │
│  timestamp │  dc(3) | worker(8) | counter_hi(1) ...            │
│            │  62 bits: shard(16) | counter_lo(20) | rand(26)   │
└────────────────────────────────────────────────────────────────┘
```

**Advantages:**
- ✅ **Time-ordered**: lexicographically sortable by creation time
- ✅ **Self-describing**: `dc` / `worker` / `shard` readable directly from the ID
- ✅ **Collision-resistant**: monotonic counter + per-instance random start
- ⚠️ **Version nibble is `8`**: validators strictly checking "version == 7" will reject it

**Code example:**

```rust
use nebulaid::core::algorithm::{AlgorithmBuilder, GenerateContext, IdAlgorithm};
use nebulaid::core::types::{AlgorithmType, Id};
use nebulaid::core::Config;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::default();
    let uuid = AlgorithmBuilder::new(AlgorithmType::UuidV8)
        .build(&config)
        .await?;

    let id = uuid.generate(&GenerateContext::default()).await?;
    println!("UUID v8: {}", id);

    // Round-trip through the uuid crate's representation
    let as_uuid = id.to_uuid_v8();
    let back = Id::from_uuid_v8(as_uuid);
    assert_eq!(back, id);

    Ok(())
}
```

**When to use:**
- You need UUID-shaped identifiers that stay index-friendly
- Time ordering matters
- You want dc / worker / shard observable directly from the ID

> **Version selection (after add-uuid-v4-v7)**: `uuid_v7` and `uuid_v4` were once input aliases resolving to `UuidV8`
> and have now reversed into **true algorithms** — `uuid_v7` produces the RFC 9562 §5.7 standard time-ordered
> UUID, and `uuid_v4` produces a random UUID. The output layout of these two strings in historical configs changes accordingly (measured usage of the legacy strings
> is zero; the CHANGELOG carries the breaking statement).

</details>

<details>
<summary><b>❓ How does distributed coordination work?</b></summary>

<br>

Nebula ID uses etcd for distributed coordination:

```
┌─────────────────────────────────────────────────────────────┐
│              Distributed Coordination                         │
├─────────────────────────────────────────────────────────────┤
│  1. Leader Election (etcd)                                  │
│  2. Datacenter Health Monitoring                            │
│  3. Automatic Failover                                      │
│  4. Segment Range Locking                                   │
└─────────────────────────────────────────────────────────────┘
```

**Components:**

1. **EtcdClusterHealthMonitor**: monitors the etcd cluster's health (public, feature `etcd`)
2. **DcFailureDetector**: tracks datacenter health — **crate-internal**
   (`src/core/algorithm/segment.rs`, accessible only through `SegmentAlgorithm`); it has no
   public constructor, so it cannot be assembled externally
3. **Automatic failover**: routes traffic to healthy datacenters

**Code example:**

```rust
use nebulaid::core::algorithm::{AlgorithmBuilder, GenerateContext, IdAlgorithm};
use nebulaid::core::coordinator::EtcdClusterHealthMonitor; // the `etcd` feature
use nebulaid::core::config::EtcdConfig;
use nebulaid::core::types::AlgorithmType;
use nebulaid::core::Config;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // new(config: EtcdConfig, cache_file_path: String) -> Self
    // Uses the cache file when etcd is unreachable.
    let health_monitor = Arc::new(EtcdClusterHealthMonitor::new(
        EtcdConfig::default(),
        "./etcd-cache.json".to_string(),
    ));

    // Hand it to the algorithm via the public builder.
    let segment = AlgorithmBuilder::new(AlgorithmType::Segment)
        .with_etcd_health_monitor(health_monitor)
        .build(&Config::default())
        .await?;

    let id = segment.generate(&GenerateContext::default()).await?;
    println!("Generated ID: {}", id);

    Ok(())
}
```

</details>

<details>
<summary><b>❓ How do I handle errors correctly?</b></summary>

<br>

**Recommended pattern:**

```rust
use nebulaid::core::algorithm::{AlgorithmBuilder, GenerateContext, IdAlgorithm};
use nebulaid::core::types::{AlgorithmType, CoreError, Id};
use nebulaid::core::Config;

async fn run() -> Result<Id, CoreError> {
    let snowflake = AlgorithmBuilder::new(AlgorithmType::Snowflake)
        .build(&Config::default())
        .await?;
    snowflake.generate(&GenerateContext::default()).await
}

#[tokio::main]
async fn main() {
    match run().await {
        Ok(id) => println!("Generated ID: {}", id.as_u128()),
        // Variant names and payload shapes are exactly as declared in
        // src/core/types/error.rs.
        Err(CoreError::ClockMovedBackward { last_timestamp }) => {
            eprintln!("❌ System clock regressed to {last_timestamp}, NTP sync required");
        }
        Err(CoreError::DatabaseError(msg, _)) => {
            eprintln!("❌ Database error: {msg}");
        }
        Err(CoreError::SegmentExhausted { max_id }) => {
            eprintln!("❌ ID segment exhausted at {max_id}, refreshing…");
        }
        Err(CoreError::EtcdError(msg, _)) => {
            eprintln!("❌ Etcd error: {msg} — falling back to the local cache");
        }
        Err(e) => eprintln!("❌ Error: {e}"),
    }
}
```

**Error types:**

| Error | Payload | Meaning | Recovery |
|-------|---------|---------|----------|
| `ClockMovedBackward` | `{ last_timestamp }` | The system clock moved backward | NTP sync required |
| `DatabaseError` | `(String, Option<ErrorSource>)` | The database is unavailable or a query failed | Check the connection; use the cache |
| `SegmentExhausted` | `{ max_id }` | The ID range is exhausted | The segment refreshes automatically |
| `EtcdError` | `(String, Option<ErrorSource>)` | etcd is unavailable | Use the local cache |
| `SequenceOverflow` | `{ timestamp }` | Snowflake sequence overflow | Wait for the next millisecond (the algorithm already sleeps 1ms and retries) |
| `ConfigurationError` | `(String)` | Required config is missing or invalid | Fix the config |

There are no `DatabaseConnectionFailed` / `EtcdConnectionFailed` variants — those are
outdated names; the database and etcd paths both report through `DatabaseError` / `EtcdError`
(`src/core/types/error.rs:102,137`; the second field is the `#[source]` error source, server logs only).

</details>

<details>
<summary><b>❓ Is async/await supported?</b></summary>

<br>

**Yes!** Nebula ID is async/await-first by design.

```rust
use nebulaid::core::algorithm::{AlgorithmBuilder, GenerateContext, IdAlgorithm};
use nebulaid::core::types::AlgorithmType;
use nebulaid::core::Config;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let segment = AlgorithmBuilder::new(AlgorithmType::Segment)
        .build(&Config::default())
        .await?;

    let ctx = GenerateContext::default();

    // Generate a single ID asynchronously
    let id = segment.generate(&ctx).await?;
    println!("Generated ID: {}", id);

    // Batch-generate asynchronously
    let batch = segment.batch_generate(&ctx, 100).await?;
    println!("Generated {} IDs", batch.len());

    Ok(())
}
```

**Runtime requirements:**

- ✅ **Tokio — required.** Algorithms spawn background tasks and use tokio
  primitives internally (segment health checks, `tokio::sync` channels,
  `tokio::time::sleep` while waiting on clocks/sequences), so they must run inside a tokio
  runtime.
- ❌ Async-Std / smol: not supported; there is no runtime abstraction layer.

</details>

---

## 📈 Performance

<div align="center">

### ⚡ Speed and Optimization

</div>

<details>
<summary><b>❓ How fast is it?</b></summary>

<br>

**Benchmark results** (Criterion microbenchmarks, collected 2026-09-18 at commit `e6acb2c`,
AMD Ryzen 9 9950X / WSL2; the full methodology and confidence intervals are in the recorded baselines of [PERFORMANCE.md](PERFORMANCE.md)):

| Benchmark case | Median | Converted throughput |
|----------------|-------:|---------------------:|
| `snowflake/generate` | 977.5 ns | ≈ 1.023M ops/s |
| `snowflake/batch_generate_100` | 97.76 µs | ≈ 1.023M ids/s |
| `router/generate_snowflake_primary` | 986.8 ns | ≈ 1.013M ops/s |
| `router/generate_with_algorithm_uuid_v8` | 382.1 ns | ≈ 2.61M ops/s |
| `rate_limiter/check_rate_limit` | 183.2 ns | ≈ 5.46M ops/s |
| `auth_cache/get_hit` | 277.7 ns | ≈ 3.60M ops/s |

Methodology boundary:

- The Segment algorithm is not benchmarked (`SegmentAlgorithm` / `SegmentLoader` are `pub(crate)`;
  external benches cannot inject an in-memory stub segment loader); measure Segment's end-to-end throughput against your own workload.
- These are algorithm/middleware-layer microbenchmarks (including per-iteration `block_on` dispatch overhead); they do not cover the full HTTP chain,
  and they are not SLA commitments. HTTP end-to-end load-test data is also in [PERFORMANCE.md](PERFORMANCE.md).
- This repo's convention is not to publish throughput numbers that were not produced by the benchmark harness.

**Run the benchmarks yourself:**

```bash
cargo bench
```

</details>

<details>
<summary><b>❓ How can I improve performance?</b></summary>

<br>

**Optimization tips:**

1. **Enable release mode:**
   ```bash
   cargo build --release
   ```

2. **Use batch generation:**
   ```rust
   // Do not generate IDs one by one (`IdAlgorithm::batch_generate`)
   let batch = segment.batch_generate(&ctx, 1000).await?;
   ```

3. **Configure sensible segment sizes:**
   ```toml
   # Keys of `SegmentAlgorithmConfig` — all four keys are required by the parser.
   # base_step must stay within [min_step, max_step].
   [algorithm.segment]
   base_step = 10000  # a larger step = fewer database round-trips
   min_step = 500
   max_step = 100000
   switch_threshold = 0.1
   ```
   > ⚠️ 服务器启动时 `Config::merge()` 会把该子表重置为默认值
   > （`src/core/config/app_config.rs:393-395`）；在该问题修复之前，请在代码中
   > 调整（在 `AlgorithmBuilder::build` 之前设置
   > `Config { algorithm: AlgorithmConfig { segment: .. } }`）。

4. **Choose Snowflake for speed:**
   - No database dependency
   - Minted in memory
   - ~1M ids/s per instance

5. **Enable the connection pool:**
   ```toml
   [database]
   max_connections = 20
   ```

</details>

<details>
<summary><b>❓ What about memory usage?</b></summary>

<br>

The repository currently has no memory benchmarks, so it publishes no concrete memory numbers (consistent with
[PERFORMANCE.md](PERFORMANCE.md)'s convention: no data without a benchmark harness).
Evaluate deployment memory against your own workload, for example:

```bash
# peak process RSS
/usr/bin/time -v ./target/release/nebula-id 2>&1 | grep "Maximum resident"
```

**Memory design points:**
- Segment cache footprint scales with the `[algorithm.segment]` step (larger steps = more resident IDs)
- Connection pool footprint scales with `[database].max_connections`

**Memory safety:**
- ✅ No memory leaks (verified by continuous testing)
- ✅ Efficient batch processing
- ✅ Connection pooling
- ✅ Efficient async runtime

</details>

<details>
<summary><b>❓ How does it handle high concurrency?</b></summary>

<br>

Nebula ID is built for high concurrency:

**Concurrency features:**
- 🚀 **Async/Await**: non-blocking operations
- 🔀 **Sharded RwLock (parking_lot) + arc-swap**: thread-safe, low-overhead concurrency primitives (the rate-limit table shards by key hash, removing write-lock hotspots)
- 📊 **Connection pooling**: efficient database connections
- ⚡ **Lock-free**: Snowflake's generation state is a single atomic CAS word, minimizing contention points

**Best practices:**

```rust
use nebulaid::core::algorithm::{AlgorithmBuilder, GenerateContext, IdAlgorithm};
use nebulaid::core::types::{AlgorithmType, Id};
use nebulaid::core::Config;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // `IdAlgorithm: Send + Sync`, so one shared handle can serve multiple tasks.
    let snowflake: Arc<dyn IdAlgorithm> = Arc::from(
        AlgorithmBuilder::new(AlgorithmType::Snowflake)
            .build(&Config::default())
            .await?,
    );

    // Spawn concurrent tasks
    let mut handles = Vec::new();
    for _ in 0..100 {
        let snowflake = Arc::clone(&snowflake);
        handles.push(tokio::spawn(async move {
            snowflake.generate(&GenerateContext::default()).await
        }));
    }

    // Collect results (both JoinError and CoreError convert into Box<dyn Error>)
    let mut ids: Vec<Id> = Vec::with_capacity(handles.len());
    for handle in handles {
        ids.push(handle.await??);
    }
    println!("{} IDs generated concurrently", ids.len());

    Ok(())
}
```

</details>

---

## 🔐 Security

<div align="center">

### 🔒 Security Features

</div>

<details>
<summary><b>❓ What security features are included?</b></summary>

<br>

**Yes!** Security is a core concern of Nebula ID.

**Security features:**

<table>
<tr>
<td width="50%">

**Authentication**
- ✅ API key authentication
- ✅ Constant-time comparison (timing-attack protection)
- ✅ Token-based access
- ✅ Configurable key rotation

</td>
<td width="50%">

**Protection**
- ✅ Rate limiting (max batch size: 100)
- ✅ Request validation
- ✅ Audit logging (with IP-spoofing protection)
- ✅ CORS restrictions
- ✅ Security response headers

</td>
</tr>
</table>

**Encryption:**
- ✅ TLS/HTTPS support (TLS 1.2/1.3)
- ✅ gRPCS support
- ✅ Secure communication

**Security response headers:**
- X-Content-Type-Options: nosniff
- X-Frame-Options: DENY
- Content-Security-Policy: default-src 'self'
- Strict-Transport-Security: max-age=31536000; includeSubDomains
- X-XSS-Protection: 1; mode=block
- Referrer-Policy: strict-origin-when-cross-origin

**More details:** the [Security doc](SECURITY.md#security-best-practices)

</details>

<details>
<summary><b>❓ How do I configure API authentication?</b></summary>

<br>

**Configuration:**

```toml
[auth]
enabled = true                     # required
cache_ttl_seconds = 300            # required
# Static bootstrap keys; runtime keys live in the database / garrison.
# Every field of ApiKeyEntry is required.
# Only the first entry is provisioned at startup (`src/main.rs:105` takes `keys.first()`);
# entries 2..N are parsed and validated but never created, without any warning.
# `workspace` must be a UUID string or the literal "global" (any other value makes
# `Uuid::parse_str` fail and it is silently replaced with an all-zero UUID).
# `role` is admin only when the value is exactly "admin" (case-insensitive); any
# other value becomes user. When NEBULA_ADMIN_API_KEY_SECRET is set,
# the whole configuration section is ignored.
api_keys = [
  { key_id = "svc-billing", key_secret = "replace-me", workspace = "0f5f6c8e-1f2e-4a7b-9c3d-2b1a4e5f6071",
    role = "user", rate_limit = 1000, name = "Billing service" },
]
api_key_salt = "${NEBULA_API_KEY_SALT}"   # optional; production rejects an empty salt
key_rotation_grace_period_seconds = 0 # optional; 0 (default) = grace period off; >0 requires the two
                                      # grace-period columns, which the startup migration adds itself

[rate_limit]
enabled = true
default_rps = 10000
burst_size = 100                   # validate(): <= 10 × default_rps

[batch_generate]
max_batch_size = 100               # maximum batch size, guarding against DoS
```

> 不存在 `[auth].api_key` 字符串键，也没有 `token_expiry_hours`；凭据始终是
> `key_id` + `key_secret` 对，过期时间不是配置概念。

**Usage:**

```rust
use nebulaid::core::Config;

// API key verification happens inside the HTTP/gRPC servers assembled by `src/main.rs`.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::load_from_file("config/config.toml")?;

    // `auth.enabled` is the switch controlling the middleware.
    println!("auth enabled: {}", config.auth.enabled);

    Ok(())
}
```

**HTTP request headers:**

`parse_authorization_header_detailed` accepts only two schemes
(`src/server/middleware/api_key_auth.rs:414-432`) — `Bearer` is rejected:

```
Authorization: ApiKey <key_id>:<key_secret>
Authorization: Basic base64(<key_id>:<key_secret>)
```

</details>

<details>
<summary><b>❓ How do I report security vulnerabilities?</b></summary>

<br>

**Please report security issues responsibly:**

1. **Do not** create a public GitHub issue
2. **Use the GitHub [Security Advisories](https://github.com/Kirky-X/NebulaId/security/policy) private disclosure channel** to submit your report (the full policy is in [SECURITY.md](SECURITY.md))
3. **Include:**
   - Affected versions
   - The vulnerability description and reproduction steps (or a proof of concept)
   - The potential impact

**Response timeline:**
- 📮 Follow-up assessment and fixes after receipt is confirmed
- 📢 Public disclosure: after the fix ships (disclosed in the `Security` section of the [CHANGELOG](CHANGELOG.md))

</details>

<details>
<summary><b>❓ How does rate limiting work?</b></summary>

<br>

Nebula ID ships built-in rate limiting:

**Configuration:**

```toml
[rate_limit]
enabled = true
default_rps = 10000
burst_size = 100
```

**Per-API-key rate limiting:**

Rate limiting is controlled globally via the `[rate_limit]` section, with per-API-key granularity overrides supported (set through the management API).

| Parameter | Default | Meaning |
|-----------|---------|---------|
| `default_rps` | 10000 | Requests per second |
| `burst_size` | 100 | Burst capacity |

**Response headers:**

```
x-ratelimit-limit: 1000
x-ratelimit-remaining: 999
```

When a request is rate-limited (HTTP 429), the response additionally carries:

```
x-ratelimit-remaining: 0
retry-after: 1
```

</details>

---

## 🔍 Troubleshooting

<div align="center">

### 🔧 Common Issues

</div>

<details>
<summary><b>❓ Hitting a "ClockMovedBackward" error</b></summary>

<br>

**Problem:**
```
Error: system clock moved backward
```

**Cause:** a system clock rollback was detected, which can cause duplicate IDs.

**Solution:**
1. **Sync the system time:**
   ```bash
   # Linux
   sudo ntpdate pool.ntp.org
   
   # macOS
   sudo sntp -sS pool.ntp.org
   ```

2. **Configure NTP auto-sync:**
   ```bash
   # append to /etc/chrony.conf
   server pool.ntp.org iburst
   ```

3. **For virtualized environments:**
   - Make sure the host clock is synced
   - Use VMware Tools time sync
   - Configure Hyper-V time sync

**Prevention:**
- Run an NTP daemon (chronyd, ntpd)
- Monitor clock drift
- Alert on significant drift

</details>

<details>
<summary><b>❓ Hitting a "DatabaseError" error</b></summary>

<br>

**Problem:**
```
Error: failed to connect to database
```

**Cause:** a database connectivity issue.

**Solution:**
1. **Confirm the database is running:**
   ```bash
   # PostgreSQL (currently the only database this service supports)
   pg_isready -h localhost -p 5432
   ```

2. **Check the connection string:**
   ```toml
   [database]
   url = "postgresql://user:pass@localhost/nebula"
   ```

3. **Test network connectivity:**
   ```bash
   telnet localhost 5432
   ```

4. **Check credentials:**
   ```bash
   psql -U user -d nebula
   ```

5. **Enable the local-cache fallback:**
   ```rust
   let health_monitor = EtcdClusterHealthMonitor::new(config, "./cache.json");
   ```

</details>

<details>
<summary><b>❓ IDs are not time-ordered</b></summary>

<br>

**Problem:**
Generated IDs are not monotonically increasing.

**Cause:** multiple instances minting IDs simultaneously.

**Solution:**

1. **For Snowflake:** make sure clocks are synced across instances

2. **For Segment:** check the segment refresh logic

3. **Use UUID v8 for time ordering:**
   ```rust
    // Inside an `async fn`:
   use nebulaid::core::algorithm::{AlgorithmBuilder, GenerateContext, IdAlgorithm};
   use nebulaid::core::types::AlgorithmType;
   use nebulaid::core::Config;

   let uuid = AlgorithmBuilder::new(AlgorithmType::UuidV8)
       .build(&Config::default())
       .await?;
   let id = uuid.generate(&GenerateContext::default()).await?;
   ```

**Note:** Snowflake IDs are ordered by millisecond within one instance.

</details>

<details>
<summary><b>❓ How do I debug ID generation problems?</b></summary>

<br>

**Enable debug logging:**

```rust
use nebulaid::core::algorithm::{AlgorithmBuilder, GenerateContext, IdAlgorithm};
use nebulaid::core::types::AlgorithmType;
use nebulaid::core::Config;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    // The concrete algorithm structs (SegmentAlgorithm / SnowflakeAlgorithm / UuidV8Impl)
    // are crate-internal; the public entry points are AlgorithmBuilder + the IdAlgorithm trait.
    let config = Config::default();
    let algorithm = AlgorithmBuilder::new(AlgorithmType::Snowflake)
        .build(&config)
        .await?;
    let id = algorithm.generate(&GenerateContext::default()).await?;
    println!("generated: {id}");
    Ok(())
}
```

Set the environment variable (the crate/module path is `nebulaid`, not the binary
name `nebula-id`):

```bash
RUST_LOG=nebulaid=debug
```

**Common debug commands:**

```bash
# Check etcd health
etcdctl endpoint health

# Check the database connection
SELECT count(*) FROM pg_stat_activity;

# Monitoring metrics
curl http://localhost:8080/metrics
```

</details>

<details>
<summary><b>❓ Performance degradation</b></summary>

<br>

**Problem:** ID generation is slower than expected.

**Diagnostic steps:**

1. **Check database performance:**
   ```sql
   EXPLAIN ANALYZE SELECT * FROM nebula_segments;
   ```

2. **Monitor the connection pool:**
   ```bash
   # Check active connections
   SELECT count(*) FROM pg_stat_activity WHERE datname = 'nebula';
   ```

3. **Check etcd latency:**
   ```bash
   etcdctl put test && etcdctl get test --cluster
   ```

**Solutions:**

1. **Increase database connections:**
   ```toml
   [database]
   max_connections = 20
   ```

2. **Enlarge the segment step:**
   ```toml
   [algorithm.segment]
   base_step = 10000
   ```

3. **Add a Redis cache:**
   ```toml
   [redis]
   url = "redis://localhost"
   ```

</details>

**More questions?** See the troubleshooting section in [DEPLOYMENT.md](DEPLOYMENT.md), or ask in [GitHub Issues](https://github.com/Kirky-X/NebulaId/issues).

---

## 👥 Contributing

<div align="center">

### 🤝 Join the Community

</div>

<details>
<summary><b>❓ How can I contribute?</b></summary>

<br>

**Ways to contribute:**

<table>
<tr>
<td width="50%">

**Code contributions**
- 🐛 Fix bugs
- ✨ Add features
- 📝 Improve docs
- ✅ Write tests

</td>
<td width="50%">

**Non-code contributions**
- 📖 Write tutorials
- 🎨 Design assets
- 🌍 Translate docs
- 💬 Answer questions

</td>
</tr>
</table>

**Getting started:**

1. 🍴 Fork the repository
2. 🌱 Create a branch: `git checkout -b feature/amazing-feature`
3. ✏️ Modify the code
4. ✅ Add tests: `cargo test --package nebulaid --features etcd`
5. 📤 Submit a PR

**Guide:** [CONTRIBUTING.md](CONTRIBUTING.md)

</details>

<details>
<summary><b>❓ I found a bug — what should I do?</b></summary>

<br>

**Before reporting:**

1. ✅ Check [existing issues](https://github.com/Kirky-X/NebulaId/issues)
2. ✅ Try the latest version
3. ✅ Read the [troubleshooting guide](#-troubleshooting)

**Writing a high-quality bug report:**

```markdown
### Description
Describe the bug clearly

### Steps to reproduce
1. First step
2. Second step
3. See the error

### Expected behavior
What should happen

### Actual behavior
What actually happened

### Environment
- OS: Ubuntu 22.04
- Rust version: 1.75.0
- Nebula ID version: 0.1.0
- Database: PostgreSQL 15

### Additional context
Any other relevant information
```

**Submit:** [Create an issue](https://github.com/Kirky-X/NebulaId/issues/new)

</details>

<details>
<summary><b>❓ Where can I get help?</b></summary>

<br>

<div align="center">

### 💬 Support Channels

</div>

<table>
<tr>
<td width="33%" align="center">

**🐛 Issues**

[GitHub Issues](https://github.com/Kirky-X/NebulaId/issues)

Bug reports and feature requests

</td>
<td width="33%" align="center">

**💬 Discussions**

[GitHub Discussions](https://github.com/Kirky-X/NebulaId/discussions)

Q&A and idea exchange

</td>
<td width="33%" align="center">

**📖 Documentation**

[User Guide](USER_GUIDE.md)

API docs and tutorials

</td>
</tr>
</table>

**Response times:**
- 🐛 Critical bugs: 24 hours
- 🔧 Feature requests: 1 week
- 💬 Questions: 2-3 days

</details>

---

## 📜 License

<div align="center">

### 📄 License Information

</div>

<details>
<summary><b>❓ What license does this project use?</b></summary>

<br>

**Apache License 2.0**

[![License](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](../../LICENSE)

**Permissions:**
- ✅ Commercial use
- ✅ Modification
- ✅ Distribution
- ✅ Patent grant
- ⚠️ Must include the license text and copyright notice
- ⚠️ Must state changes made

</details>

<details>
<summary><b>❓ Can I use it in commercial projects?</b></summary>

<br>

**Yes!** The Apache 2.0 license allows commercial use.

**What you must do:**
1. ✅ Include the license text
2. ✅ Include the copyright notice
3. ✅ State changes made

**What you don't need to do:**
- ❌ Open-source your code
- ❌ Open-source the project
- ❌ Pay royalties

**Questions?** Contact: legal@nebula-id.io

</details>

---

<div align="center">

### 🎯 Still have questions?

<table>
<tr>
<td width="33%" align="center">
<a href="../../../issues">
<img src="https://img.icons8.com/fluency/96/000000/bug.png" width="48"><br>
<b>提交 Issue</b>
</a>
</td>
<td width="33%" align="center">
<a href="../../../discussions">
<img src="https://img.icons8.com/fluency/96/000000/chat.png" width="48"><br>
<b>发起讨论</b>
</a>
</td>
<td width="33%" align="center">
<a href="https://docs.rs/nebulaid">
<img src="https://img.icons8.com/fluency/96/000000/documentation.png" width="48"><br>
<b>阅读 API 文档</b>
</a>
</td>
</tr>
</table>

---

**[📖 User Guide](USER_GUIDE.md)** • **[🔧 API Reference](API_REFERENCE.md)** • **[🏠 Home](../../README.md)**

Built with ❤️ by Kirky.X

[⬆ Back to top](#-nebula-id-faq)
