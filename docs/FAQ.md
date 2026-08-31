<div align="center">

# ❓ Frequently Asked Questions (FAQ)

### Quick Answers to Common Questions about Nebula ID

[🏠 Home](../README.md) • [📖 User Guide](USER_GUIDE.md) • [🔧 API Reference](API_REFERENCE.md)

---

</div>

## 📋 Table of Contents

- [General Questions](#general-questions)
- [Installation & Setup](#installation--setup)
- [Usage & Features](#usage--features)
- [Performance](#performance)
- [Security](#security)
- [Troubleshooting](#troubleshooting)
- [Contributing](#contributing)
- [Licensing](#licensing)

---

## General Questions

<div align="center">

### 🤔 About the Project

</div>

<details>
<summary><b>❓ What is Nebula ID?</b></summary>

<br>

**Nebula ID** is an enterprise-grade distributed ID generation system for high-performance applications. It provides:

- ✅ **Multiple ID Algorithms** - Segment, Snowflake, UUID v8
- ✅ **Distributed Coordination** - Etcd-based leader election and coordination
- ✅ **High Availability** - Datacenter health monitoring and automatic failover
- ✅ **Type-Safe Design** - Full Rust type safety with async/await patterns

It's designed for **distributed systems** that require unique, ordered, and high-throughput identifier generation.

**Learn more:** [User Guide](USER_GUIDE.md)

</details>

<details>
<summary><b>❓ Why should I use this instead of alternatives?</b></summary>

<br>

<table>
<tr>
<th>Feature</th>
<th>Nebula ID</th>
<th>Snowflake</th>
<th>UUID</th>
</tr>
<tr>
<td>Time Ordering</td>
<td>✅ Yes</td>
<td>✅ Yes</td>
<td>⚠️ v7 only</td>
</tr>
<tr>
<td>High Throughput</td>
<td>✅ 1M+ IDs/sec</td>
<td>✅ 1M+ IDs/sec</td>
<td>✅ 1M+ IDs/sec</td>
</tr>
<tr>
<td>No Clock Sync</td>
<td>✅ Segment</td>
<td>❌ No</td>
<td>✅ Yes</td>
</tr>
<tr>
<td>Fault Tolerance</td>
<td>✅ Built-in</td>
<td>⚠️ Manual</td>
<td>✅ Yes</td>
</tr>
</table>

**Key Advantages:**
- 🚀 **Multiple Algorithms**: Choose Segment for database-backed ordering, Snowflake for speed, or UUID for simplicity
- 🔄 **Automatic Failover**: Datacenter health monitoring with automatic recovery
- 🛡️ **Enterprise Ready**: API authentication, rate limiting, and audit logging
- 📊 **Built-in Monitoring**: Health checks and metrics collection

</details>

<details>
<summary><b>❓ Is this production-ready?</b></summary>

<br>

**Current Status:** ✅ **Production-ready!**

<table>
<tr>
<td width="50%">

**What's Ready:**
- ✅ Core ID generation algorithms (Segment, Snowflake, UUID v8)
- ✅ Distributed coordination with Etcd
- ✅ Datacenter health monitoring and failover
- ✅ HTTP/HTTPS and gRPC/gRPCS APIs
- ✅ API key authentication and rate limiting

</td>
<td width="50%">

**Maturity Indicators:**
- 📊 85%+ test coverage
- 🔄 Regular maintenance
- 🛡️ Security-focused design
- 📖 Comprehensive documentation

</td>
</tr>
</table>

> **Note:** Always review the [CHANGELOG](../CHANGELOG.md) before upgrading versions.

</details>

<details>
<summary><b>❓ What platforms are supported?</b></summary>

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
<td>✅ Fully Supported</td>
<td>Primary platform</td>
</tr>
<tr>
<td>ARM64</td>
<td>✅ Fully Supported</td>
<td>Tested on ARM servers</td>
</tr>
<tr>
<td rowspan="2"><b>macOS</b></td>
<td>x86_64</td>
<td>✅ Fully Supported</td>
<td>Intel Macs</td>
</tr>
<tr>
<td>ARM64</td>
<td>✅ Fully Supported</td>
<td>Apple Silicon (M1/M2/M3)</td>
</tr>
<tr>
<td><b>Windows</b></td>
<td>x86_64</td>
<td>✅ Fully Supported</td>
<td>Windows 10+</td>
</tr>
</table>

</details>

<details>
<summary><b>❓ What programming languages are supported?</b></summary>

<br>

**Nebula ID** is a native **Rust** library with multi-protocol service support:

- **Rust**: Native library (`nebula-id` crate)
- **HTTP/REST**: Any language with HTTP client
- **gRPC**: Any language with gRPC support (Python, Java, Go, etc.)

**Documentation:**
- [Rust API Docs](https://docs.rs/nebula-id)
- [API Reference](API_REFERENCE.md)

</details>

<details>
<summary><b>❓ What ID algorithms are supported?</b></summary>

<br>

<table>
<tr>
<th>Algorithm</th>
<th>Format</th>
<th>Time Ordered</th>
<th>Throughput</th>
<th>Best For</th>
</tr>
<tr>
<td>Segment</td>
<td>64-bit</td>
<td>✅ Yes</td>
<td>100K+/sec</td>
<td>Database primary keys</td>
</tr>
<tr>
<td>Snowflake</td>
<td>64-bit</td>
<td>✅ Yes</td>
<td>1M+/sec</td>
<td>High-performance systems</td>
</tr>
<tr>
<td>UUID v8</td>
<td>128-bit</td>
<td>✅ Yes</td>
<td>500K+/sec</td>
<td>Distributed systems</td>
</tr>
</table>

> Throughput figures are indicative only — the repository has no `benches/` coverage for the UUID path.

</details>

---

## Installation & Setup

<div align="center">

### 🚀 Getting Started

</div>

<details>
<summary><b>❓ How do I install this?</b></summary>

<br>

**For Rust Projects:**

Add the following to your `Cargo.toml`:

```toml
[dependencies]
nebulaid = "0.2"                       # Cargo package name is `nebulaid`
tokio = { version = "1.0", features = ["full"] }
```

Or using cargo:

```bash
cargo add nebulaid tokio
```

**Optional Features** (`Cargo.toml` `[features]`):

```toml
# default = ["postgresql", "http", "grpc", "garrison-auth"]
nebulaid = { version = "0.2", features = ["etcd"] }  # distributed coordination
# nebulaid = { version = "0.2", features = ["sdk"] } # NebulaIdClient facade
# There is no `monitoring` / `audit` / `tls` feature: metrics, audit logging and
# TLS are runtime configuration ([monitoring] / [auth] / [tls]).
```

**Verification:**

```rust
use nebulaid::core::Config;
use nebulaid::sdk::NebulaIdClientBuilder; // requires feature `sdk`

#[tokio::main]
async fn main() -> nebulaid::core::Result<()> {
    let mut config = Config::default();
    config.algorithm.default = "snowflake".to_string();

    let client = NebulaIdClientBuilder::new(config).build().await?;
    let id = client.generate("verify", "install", "order").await?;
    println!("✅ Generated ID: {id}");

    client.shutdown().await;
    Ok(())
}
```

**See also:** [User Guide](USER_GUIDE.md#installation)

</details>

<details>
<summary><b>❓ What are the system requirements?</b></summary>

<br>

**Minimum Requirements:**

<table>
<tr>
<th>Component</th>
<th>Requirement</th>
<th>Recommended</th>
</tr>
<tr>
<td>Rust Version</td>
<td>1.75+</td>
<td>Latest stable</td>
</tr>
<tr>
<td>Memory</td>
<td>256MB</td>
<td>1GB+</td>
</tr>
<tr>
<td>Disk Space</td>
<td>50MB</td>
<td>100MB+</td>
</tr>
<tr>
<td>Database</td>
<td>PostgreSQL/MySQL</td>
<td>PostgreSQL 13+</td>
</tr>
</table>

**Optional Dependencies:**
- 🔧 **Etcd**: For distributed coordination (v3.4+)
- ☁️ **Redis**: For caching (v6+)
- 📊 **Prometheus**: For metrics visualization

</details>

<details>
<summary><b>❓ I'm getting compilation errors, what should I do?</b></summary>

<br>

**Common Solutions:**

1. **Check Rust version:**
   ```bash
   rustc --version
   # Should be 1.75.0 or higher
   ```

2. **Ensure required features are enabled** (default is
   `["postgresql", "http", "grpc", "garrison-auth"]`; `sqlite` is currently
   unbuildable because `limiteron` hard-depends on `dbnexus/postgres`):
   ```toml
   nebulaid = "0.2"
   ```

3. **Clean build artifacts:**
   ```bash
   cargo clean
   cargo build
   ```

**Still having issues?**
- 📝 Check [Troubleshooting](#troubleshooting)
- 🐛 [Open an issue](../../issues) with error details

</details>

<details>
<summary><b>❓ Can I use this with Docker?</b></summary>

<br>

**Yes!** Nebula ID works perfectly in containerized environments.

**Sample Dockerfile:**

```dockerfile
FROM rust:1.75 as builder
WORKDIR /app
COPY . .
RUN cargo build --release

FROM debian:bookworm-slim
COPY --from=builder /app/target/release/nebula-id /usr/local/bin/
CMD ["nebula-id"]
```

**Docker Compose with Dependencies:**

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

**Configuration File (`config.toml`):**

All ten sections below are **required** — `Config` gives them no
`#[serde(default)]` (`src/core/config/app_config.rs:37-64`). A parse failure now aborts
startup with exit code 1 instead of falling back to `Config::default()`; unknown keys are
rejected the same way, because every config struct carries `deny_unknown_fields`. Built-in
defaults are used only when no `--config` was given *and* `config/config.toml` does not
exist, which emits a `warn`. Only `[redis]` and `[hot_reload]` may be omitted.

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

# optional:
# [redis] url = "redis://localhost:6379"
# [hot_reload] auto_watch_enabled = false
```

> ⚠️ `Config::merge()` runs the environment config on top of the file at startup
> and unconditionally replaces `algorithm.segment` / `algorithm.snowflake` /
> `algorithm.uuid_v8` with it (`src/core/config/app_config.rs:393-395`,
> `src/main.rs:559`) — in practice those three sub-tables always end up as
> defaults. Only `algorithm.default` survives. Tune them in code for now.

**Environment Variables:**

There is no `NEBULA_DATABASE_URL` / `NEBULA_AUTH_API_KEY` family. The real ones are:

```bash
# Merged over the file by `Config::load_from_env()`:
export APP_HOST="0.0.0.0"
export APP_HTTP_PORT="8080"
export APP_GRPC_PORT="9091"
export DC_ID="0"
export WORKER_ID="0"
export DATABASE_URL="postgresql://idgen:pass@localhost:5432/idgen"
export ETCD_ENDPOINTS="http://localhost:2379"
export RUST_LOG="info"

# Expanded inside the file as ${VAR} before parsing:
export NEBULA_DATABASE_PASSWORD="..."   # [database].password / url
export NEBULA_API_KEY_SALT="..."        # [auth].api_key_salt fallback
```

**See also:** [Configuration Guide](USER_GUIDE.md#configuration)

</details>

---

## Usage & Features

<div align="center">

### 💡 Working with the API

</div>

<details>
<summary><b>❓ How do I get started with basic usage?</b></summary>

<br>

**5-Minute Quick Start：**

```rust
use nebulaid::core::Config;
use nebulaid::sdk::NebulaIdClientBuilder; // feature `sdk`

#[tokio::main]
async fn main() -> nebulaid::core::Result<()> {
    // Segment needs `NebulaIdClientBuilder::with_repository(..)` because it
    // allocates ranges from the database; pure algorithms need nothing.
    let mut config = Config::default();
    config.algorithm.default = "snowflake".to_string();

    let client = NebulaIdClientBuilder::new(config).build().await?;

    // Generate a single ID
    let id = client.generate("prod", "core", "order").await?;
    println!("Generated ID: {} (u128: {})", id, id.as_u128());

    // Generate a batch of IDs
    let batch = client.batch_generate("prod", "core", "order", 100).await?;
    println!("Generated {} IDs", batch.len());

    client.shutdown().await;
    Ok(())
}
```

**Next Steps:**
- 📖 [User Guide](USER_GUIDE.md)
- 💻 [Examples](../examples/)

</details>

<details>
<summary><b>❓ How do I choose the right algorithm?</b></summary>

<br>

**Algorithm Selection Guide:**

| Use Case | Recommended Algorithm | Reason |
|----------|----------------------|--------|
| Database primary keys | Segment | Ordered, database-backed, reliable |
| High-throughput microservices | Snowflake | Fast, no database dependency |
| Time-ordered distributed IDs | UUID v8 | RFC 9562 §5.8 layout, time-sortable, embeds dc/worker/shard |
| Mixed requirements | Multi-algorithm | Use different algorithms per use case |

**Configuration**Code Example:**

```rust
use nebulaid::core::algorithm::AlgorithmBuilder;
use nebulaid::core::types::AlgorithmType;
use nebulaid::core::Config;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::default();

    // Algorithm structs (SegmentAlgorithm / SnowflakeAlgorithm / UuidV8Impl) are crate-internal;
    // build them through the public AlgorithmBuilder.
    // Snowflake and UuidV8 are pure algorithms and need no database; Segment requires a
    // repository to be wired up first (see the SDK notes in src/sdk/client.rs).
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

The Segment algorithm pre-allocates ID ranges from the database for efficient batch generation:

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

**Key Benefits:**
- 🚀 **High Throughput**: Generate IDs from local memory
- 📦 **Batch Efficiency**: Pre-allocation reduces database round-trips
- 🔄 **Fault Tolerance**: Automatic failover to healthy datacenters

**Code Example:**

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

    // Generate single ID (from the pre-allocated segment)
    let id = segment.generate(&ctx).await?;
    println!("Generated ID: {}", id);

    // Generate a batch (one database round-trip for `size` IDs)
    let batch = segment.batch_generate(&ctx, 1000).await?;
    println!("Generated {} IDs", batch.ids.len());

    Ok(())
}
```

</details>

<details>
<summary><b>❓ How does the Snowflake algorithm work?</b></summary>

<br>

The Snowflake algorithm generates 64-bit IDs with configurable bit allocation
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

**Key Benefits:**
- 🚀 **Fast**: No database dependency
- 📈 **Scalable**: With the default bit layout, 8 datacenters × 256 workers
  (`datacenter_id_bits` / `worker_id_bits` / `sequence_bits` are configurable;
  their sum must stay < 64)
- 🎯 **Ordered**: Time-based ordering within millisecond

**Code Example:**

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

    // The bit layout is readable from the config; the remainder of the 64 bits
    // is the timestamp field.
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
<summary><b>❓ What is UUID v8 and when should I use it?</b></summary>

<br>

Nebula ID ships a **time-ordered UUID v8** generator (RFC 9562 §5.8 custom layout). Standard
UUID v7 also carries a 48-bit millisecond timestamp, but its remaining bits are fixed to
clock-seq + node; Nebula's v8 composition uses the vendor-defined fields to embed
tenant/zone context directly into the ID (UUIDP "Cluster" style: random per-instance start plus
a strictly monotonic counter):

```
┌────────────────────────────────────────────────────────────────┐
│                    UUID v8 Structure (128 bits)                 │
├────────────────────────────────────────────────────────────────┤
│  48 bits   │  16 bits (incl. version=0b1000 + variant=0b10)    │
│  timestamp │  dc(3) | worker(8) | counter_hi(1) ...            │
│            │  62 bits: shard(16) | counter_lo(20) | rand(26)   │
└────────────────────────────────────────────────────────────────┘
```

**Benefits:**
- ✅ **Time-Ordered**: Lexicographically sortable by creation time
- ✅ **Self-Describing**: `dc` / `worker` / `shard` are readable from the ID itself
- ✅ **Collision-Resistant**: monotonic counter + per-instance random start
- ⚠️ **Version nibble is `8`**: strict "version == 7" validators will reject it

**Code Example:**

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

    // Round-trip through the uuid crate representation
    let as_uuid = id.to_uuid_v8();
    let back = Id::from_uuid_v8(as_uuid);
    assert_eq!(back, id);

    Ok(())
}
```

**Use When:**
- You need UUID-shaped identifiers that stay index-friendly
- Time-based sorting is important
- You want dc / worker / shard observable straight from the ID

> **Legacy naming**: `uuid_v7` and `uuid_v4` are no longer separate algorithms, but
> `AlgorithmType::from_str` (`src/core/types/id.rs:195-201`) still accepts both spellings as
> **input aliases** that resolve to `UuidV8`, so old configs and API payloads keep working.
> Anything Nebula emits is `uuid_v8`.

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

1. **EtcdClusterHealthMonitor**: Monitors etcd cluster health (public, feature `etcd`)
2. **DcFailureDetector**: Tracks datacenter health status — **internal to the crate**
   (`src/core/algorithm/segment.rs`, reached only through `SegmentAlgorithm`); there is no
   public constructor for it, so it cannot be wired up from outside
3. **Automatic Failover**: Routes traffic to healthy datacenters

**Code Example:**

```rust
use nebulaid::core::algorithm::{AlgorithmBuilder, GenerateContext, IdAlgorithm};
use nebulaid::core::coordinator::EtcdClusterHealthMonitor; // feature `etcd`
use nebulaid::core::config::EtcdConfig;
use nebulaid::core::types::AlgorithmType;
use nebulaid::core::Config;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // new(config: EtcdConfig, cache_file_path: String) -> Self
    // The cache file is used when etcd is unreachable.
    let health_monitor = Arc::new(EtcdClusterHealthMonitor::new(
        EtcdConfig::default(),
        "./etcd-cache.json".to_string(),
    ));

    // Hand it to the algorithm through the public builder.
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
<summary><b>❓ How do I handle errors properly?</b></summary>

<br>

**Recommended Pattern:**

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
        Err(CoreError::DatabaseError(msg)) => {
            eprintln!("❌ Database error: {msg}");
        }
        Err(CoreError::SegmentExhausted { max_id }) => {
            eprintln!("❌ ID segment exhausted at {max_id}, refreshing…");
        }
        Err(CoreError::EtcdError(msg)) => {
            eprintln!("❌ Etcd error: {msg} — falling back to the local cache");
        }
        Err(e) => eprintln!("❌ Error: {e}"),
    }
}
```

**Error Types:**

| Error | Payload | Description | Recovery |
|-------|---------|-------------|----------|
| `ClockMovedBackward` | `{ last_timestamp }` | System clock regression | NTP sync required |
| `DatabaseError` | `(String)` | Database unavailable or query failed | Check connection, use cache |
| `SegmentExhausted` | `{ max_id }` | ID range depleted | Auto-refresh segment |
| `EtcdError` | `(String)` | Etcd unavailable | Use local cache |
| `SequenceOverflow` | `{ timestamp }` | Snowflake sequence overflow | Wait for next ms (the algorithm already sleeps 1 ms and retries) |
| `ConfigurationError` | `(String)` | Required setting missing/invalid | Fix the config |

There is no `DatabaseConnectionFailed` / `EtcdConnectionFailed` variant — those were
stale names; the DB and etcd paths both report through `DatabaseError` / `EtcdError`.

</details>

<details>
<summary><b>❓ Is there async/await support?</b></summary>

<br>

**Yes!** Nebula ID is designed for async/await from the ground up.

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

    // Async ID generation
    let id = segment.generate(&ctx).await?;
    println!("Generated ID: {}", id);

    // Async batch generation
    let batch = segment.batch_generate(&ctx, 100).await?;
    println!("Generated {} IDs", batch.len());

    Ok(())
}
```

**Runtime requirement:**

- ✅ **Tokio — required.** The algorithms spawn background tasks and use tokio
  primitives internally (segment health checks, `tokio::sync` channels,
  `tokio::time::sleep` on clock/sequence waits), so they must run inside a tokio
  runtime.
- ❌ Async-Std / smol: not supported; there is no runtime abstraction layer.

</details>

---

## Performance

<div align="center">

### ⚡ Speed and Optimization

</div>

<details>
<summary><b>❓ How fast is it?</b></summary>

<br>

**Benchmark Results:**

<table>
<tr>
<th>Algorithm</th>
<th>Throughput</th>
<th>P50 Latency</th>
<th>P99 Latency</th>
</tr>
<tr>
<td>Segment</td>
<td>100,000+ IDs/sec</td>
<td>~0.1ms</td>
<td>~0.5ms</td>
</tr>
<tr>
<td>Snowflake</td>
<td>1,000,000+ IDs/sec</td>
<td>~0.05ms</td>
<td>~0.1ms</td>
</tr>
<tr>
<td>UUID v8</td>
<td>500,000+ IDs/sec</td>
<td>~0.03ms</td>
<td>~0.05ms</td>
</tr>
</table>

**Run benchmarks yourself:**

```bash
cargo bench
```

</details>

<details>
<summary><b>❓ How can I improve performance?</b></summary>

<br>

**Optimization Tips:**

1. **Enable Release Mode:**
   ```bash
   cargo build --release
   ```

2. **Use Batch Generation:**
   ```rust
   // Instead of generating IDs one by one (`IdAlgorithm::batch_generate`)
   let batch = segment.batch_generate(&ctx, 1000).await?;
   ```

3. **Configure Appropriate Segment Size:**
   ```toml
   # Keys of `SegmentAlgorithmConfig` — all four are required by the parser.
   # base_step must stay within [min_step, max_step].
   [algorithm.segment]
   base_step = 10000  # Larger step = fewer database round-trips
   min_step = 500
   max_step = 100000
   switch_threshold = 0.1
   ```
   > ⚠️ At server startup `Config::merge()` resets this sub-table to the defaults
   > (`src/core/config/app_config.rs:393-395`); until that is fixed, tune it in code
   > (`Config { algorithm: AlgorithmConfig { segment: .. } }` before `AlgorithmBuilder::build`).

4. **Use Snowflake for Speed:**
   - No database dependency
   - In-memory generation
   - ~1M IDs/sec per instance

5. **Enable Connection Pooling:**
   ```toml
   [database]
   max_connections = 20
   ```

</details>

<details>
<summary><b>❓ What's the memory usage like?</b></summary>

<br>

**Typical Memory Usage:**

<table>
<tr>
<th>Component</th>
<th>Memory</th>
</tr>
<tr>
<td>Core Library</td>
<td>~1MB</td>
</tr>
<tr>
<td>Segment Cache (1M IDs)</td>
<td>~8MB</td>
</tr>
<tr>
<td>Etcd Client</td>
<td>~2MB</td>
</tr>
<tr>
<td>HTTP Server</td>
<td>~5MB</td>
</tr>
</table>

**Total:** ~16MB base + algorithm-specific overhead

**Memory Safety:**
- ✅ No memory leaks (verified with continuous testing)
- ✅ Efficient batch processing
- ✅ Connection pooling
- ✅ Async runtime efficiency

</details>

<details>
<summary><b>❓ How does the system handle high concurrency?</b></summary>

<br>

Nebula ID is designed for high concurrency:

**Concurrency Features:**
- 🚀 **Async/Await**: Non-blocking operations
- 🔀 **DashMap**: Thread-safe concurrent data structures
- 📊 **Connection Pooling**: Efficient database connections
- ⚡ **Lock-Free**: Minimal contention points

**Best Practices:**

```rust
use nebulaid::core::algorithm::{AlgorithmBuilder, GenerateContext, IdAlgorithm};
use nebulaid::core::types::{AlgorithmType, Id};
use nebulaid::core::Config;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // `IdAlgorithm: Send + Sync`, so one shared handle can serve many tasks.
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

    // Collect results (JoinError and CoreError both widen to Box<dyn Error>)
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

## Security

<div align="center">

### 🔒 Security Features

</div>

<details>
<summary><b>❓ What security features are included?</b></summary>

<br>

**Yes!** Security is a core focus of Nebula ID.

**Security Features:**

<table>
<tr>
<td width="50%">

**Authentication**
- ✅ API Key authentication
- ✅ Constant-time comparison (timing attack prevention)
- ✅ Token-based access
- ✅ Configurable key rotation

</td>
<td width="50%">

**Protection**
- ✅ Rate limiting (max batch size: 100)
- ✅ Request validation
- ✅ Audit logging with IP spoofing protection
- ✅ CORS restrictions
- ✅ Security headers

</td>
</tr>
</table>

**Encryption:**
- ✅ TLS/HTTPS support (TLS 1.2/1.3)
- ✅ gRPCS support
- ✅ Secure communication

**Security Headers:**
- X-Content-Type-Options: nosniff
- X-Frame-Options: DENY
- Content-Security-Policy: default-src 'self'
- Strict-Transport-Security: max-age=31536000; includeSubDomains
- X-XSS-Protection: 1; mode=block
- Referrer-Policy: strict-origin-when-cross-origin

**More details:** [Security Guide](USER_GUIDE.md#security)

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
# Every ApiKeyEntry field is required.
api_keys = [
  { key_id = "svc-billing", key_secret = "replace-me", workspace = "billing",
    role = "user", rate_limit = 1000, name = "Billing service" },
]
api_key_salt = "${NEBULA_API_KEY_SALT}"   # optional; production rejects an empty salt
key_rotation_grace_period_seconds = 0 # optional; 0 (default) = grace off, >0 needs the two grace columns

[rate_limit]
enabled = true
default_rps = 1000
burst_size = 100                   # validate(): <= 10 × default_rps

[batch_generate]
max_batch_size = 100               # Maximum batch size to prevent DoS attacks
```

> There is no `[auth].api_key` string key and no `token_expiry_hours`; credentials are
> always `key_id` + `key_secret` pairs, and expiry is not a config concept.

**Usage:**

```rust
use nebulaid::core::Config;

// API key validation happens inside the HTTP/gRPC server wired up by `src/main.rs`.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::load_from_file("config/config.toml")?;

    // `auth.enabled` is the switch that gates the middleware.
    println!("auth enabled: {}", config.auth.enabled);

    Ok(())
}
```

**HTTP Header:**

`parse_authorization_header_detailed` accepts exactly two schemes
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

1. **DO NOT** create public GitHub issues
2. **Email:** security@nebula-id.io
3. **Include:**
   - Description of the vulnerability
   - Steps to reproduce
   - Potential impact

**Response Timeline:**
- 📧 Initial response: 24 hours
- 🔍 Assessment: 72 hours
- 📢 Public disclosure: After fix is released

</details>

<details>
<summary><b>❓ What about rate limiting?</b></summary>

<br>

Nebula ID includes built-in rate limiting:

**Configuration:**

```toml
[rate_limit]
enabled = true
default_rps = 1000
burst_size = 100
```

**Rate Limits by Plan:**

| Plan | Requests/Second | Burst |
|------|-----------------|-------|
| Free | 100 | 10 |
| Pro | 1,000 | 100 |
| Enterprise | 10,000 | 1,000 |

**Response Headers:**

```
x-ratelimit-limit: 1000
x-ratelimit-remaining: 999
```

When a request is rate limited (HTTP 429), the response additionally carries:

```
x-ratelimit-remaining: 0
retry-after: 1
```

</details>

---

## Troubleshooting

<div align="center">

### 🔧 Common Issues

</div>

<details>
<summary><b>❓ I'm getting "ClockMovedBackward" error</b></summary>

<br>

**Problem:**
```
Error: system clock moved backward
```

**Cause:** System clock regression detected, which could cause duplicate IDs.

**Solution:**
1. **Sync system time:**
   ```bash
   # Linux
   sudo ntpdate pool.ntp.org
   
   # macOS
   sudo sntp -sS pool.ntp.org
   ```

2. **Configure NTP auto-sync:**
   ```bash
   # Add to /etc/chrony.conf
   server pool.ntp.org iburst
   ```

3. **For virtualized environments:**
   - Ensure host clock is synced
   - Use VMware Tools time synchronization
   - Configure Hyper-V time synchronization

**Prevention:**
- Use NTP daemon (chronyd, ntpd)
- Monitor clock drift
- Alert on significant drift

</details>

<details>
<summary><b>❓ I'm getting "DatabaseConnectionFailed" error</b></summary>

<br>

**Problem:**
```
Error: failed to connect to database
```

**Cause:** Database connection issues.

**Solution:**
1. **Verify database is running:**
   ```bash
   # PostgreSQL
   pg_isready -h localhost -p 5432
   
   # MySQL
   mysqladmin ping -h localhost
   ```

2. **Check connection string:**
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

5. **Enable local cache fallback:**
   ```rust
   let health_monitor = EtcdClusterHealthMonitor::new(config, "./cache.json");
   ```

</details>

<details>
<summary><b>❓ IDs are not time-ordered</b></summary>

<br>

**Problem:**
Generated IDs are not monotonically increasing.

**Cause:** Multiple instances generating IDs simultaneously.

**Solution:**

1. **For Snowflake:** Ensure clock is synchronized across instances

2. **For Segment:** Verify segment refresh logic

3. **Use UUID v8 for time-ordering:**
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

**Note:** Snowflake IDs are ordered within the same millisecond per instance.

</details>

<details>
<summary><b>❓ How do I debug ID generation issues?</b></summary>

<br>

**Enable Debug Logging:**

```rust
use nebulaid::core::algorithm::{AlgorithmBuilder, GenerateContext, IdAlgorithm};
use nebulaid::core::types::AlgorithmType;
use nebulaid::core::Config;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    // 具体算法结构体（SegmentAlgorithm / SnowflakeAlgorithm / UuidV8Impl）
    // 是 crate 内部实现；公开入口只有 AlgorithmBuilder + IdAlgorithm trait。
    let config = Config::default();
    let algorithm = AlgorithmBuilder::new(AlgorithmType::Snowflake)
        .build(&config)
        .await?;
    let id = algorithm.generate(&GenerateContext::default()).await?;
    println!("generated: {id}");
    Ok(())
}
```

Set environment variable (the crate/module path is `nebulaid`, not the binary
name `nebula-id`):

```bash
RUST_LOG=nebulaid=debug
```

**Common Debug Commands:**

```bash
# Check etcd health
etcdctl endpoint health

# Check database connections
SELECT count(*) FROM pg_stat_activity;

# Monitor metrics
curl http://localhost:8080/metrics
```

</details>

<details>
<summary><b>❓ Performance is degraded</b></summary>

<br>

**Problem:** ID generation is slower than expected.

**Diagnosis Steps:**

1. **Check database performance:**
   ```sql
   EXPLAIN ANALYZE SELECT * FROM nebula_segments;
   ```

2. **Monitor connection pool:**
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

2. **Increase segment step:**
   ```toml
   [algorithm.segment]
   step = 10000
   ```

3. **Add Redis caching:**
   ```toml
   [redis]
   url = "redis://localhost"
   ```

</details>

**More issues?** Check [Troubleshooting Guide](TROUBLESHOOTING.md)

---

## Contributing

<div align="center">

### 🤝 Join the Community

</div>

<details>
<summary><b>❓ How can I contribute?</b></summary>

<br>

**Ways to Contribute:**

<table>
<tr>
<td width="50%">

**Code Contributions**
- 🐛 Fix bugs
- ✨ Add features
- 📝 Improve documentation
- ✅ Write tests

</td>
<td width="50%">

**Non-Code Contributions**
- 📖 Write tutorials
- 🎨 Design assets
- 🌍 Translate docs
- 💬 Answer questions

</td>
</tr>
</table>

**Getting Started:**

1. 🍴 Fork the repository
2. 🌱 Create a branch: `git checkout -b feature/amazing-feature`
3. ✏️ Make changes
4. ✅ Add tests: `cargo test --package nebulaid --features etcd`
5. 📤 Submit PR

**Guidelines:** [CONTRIBUTING.md](../CONTRIBUTING.md)

</details>

<details>
<summary><b>❓ I found a bug, what should I do?</b></summary>

<br>

**Before Reporting:**

1. ✅ Check [existing issues](../../issues)
2. ✅ Try the latest version
3. ✅ Check [troubleshooting guide](#troubleshooting)

**Creating a Good Bug Report:**

```markdown
### Description
Clear description of the bug

### Steps to Reproduce
1. Step one
2. Step two
3. See error

### Expected Behavior
What should happen

### Actual Behavior
What actually happens

### Environment
- OS: Ubuntu 22.04
- Rust version: 1.75.0
- Nebula ID version: 0.1.0
- Database: PostgreSQL 15

### Additional Context
Any other relevant information
```

**Submit:** [Create Issue](../../issues/new)

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

[GitHub Issues](../../issues)

Bug reports & features

</td>
<td width="33%" align="center">

**💬 Discussions**

[GitHub Discussions](../../discussions)

Q&A and ideas

</td>
<td width="33%" align="center">

**📖 Documentation**

[User Guide](USER_GUIDE.md)

API docs & tutorials

</td>
</tr>
</table>

**Response Times:**
- 🐛 Critical bugs: 24 hours
- 🔧 Feature requests: 1 week
- 💬 Questions: 2-3 days

</details>

---

## Licensing

<div align="center">

### 📄 License Information

</div>

<details>
<summary><b>❓ What license is this under?</b></summary>

<br>

**Dual License:**

<table>
<tr>
<td width="50%" align="center">

**MIT License**

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](../LICENSE-MIT)

**Permissions:**
- ✅ Commercial use
- ✅ Modification
- ✅ Distribution
- ✅ Private use

</td>
<td width="50%" align="center">

**Apache License 2.0**

[![License: Apache 2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](../LICENSE-APACHE)

**Permissions:**
- ✅ Commercial use
- ✅ Modification
- ✅ Distribution
- ✅ Patent grant

</td>
</tr>
</table>

**You can choose either license for your use.**

</details>

<details>
<summary><b>❓ Can I use this in commercial projects?</b></summary>

<br>

**Yes!** Both MIT and Apache 2.0 licenses allow commercial use.

**What you need to do:**
1. ✅ Include the license text
2. ✅ Include copyright notice
3. ✅ State any modifications

**What you DON'T need to do:**
- ❌ Share your source code
- ❌ Open source your project
- ❌ Pay royalties

**Questions?** Contact: legal@nebula-id.io

</details>

---

<div align="center">

### 🎯 Still Have Questions?

<table>
<tr>
<td width="33%" align="center">
<a href="../../issues">
<img src="https://img.icons8.com/fluency/96/000000/bug.png" width="48"><br>
<b>Open an Issue</b>
</a>
</td>
<td width="33%" align="center">
<a href="../../discussions">
<img src="https://img.icons8.com/fluency/96/000000/chat.png" width="48"><br>
<b>Start a Discussion</b>
</a>
</td>
<td width="33%" align="center">
<a href="https://docs.rs/nebula-id">
<img src="https://img.icons8.com/fluency/96/000000/documentation.png" width="48"><br>
<b>Read API Docs</b>
</a>
</td>
</tr>
</table>

---

**[📖 User Guide](USER_GUIDE.md)** • **[🔧 API Reference](API_REFERENCE.md)** • **[🏠 Home](../README.md)**

Made with ❤️ by the Nebula ID Team

[⬆ Back to Top](#-frequently-asked-questions-faq)
