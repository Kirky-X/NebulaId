# 📖 Nebula ID User Guide

> The complete Nebula ID usage guide: from installation basics to advanced usage and best practices.

[🏠 Home](../../README.md) • [🎯 Examples](../../examples/) • [❓ FAQ](FAQ.md)

---

## 📋 Table of Contents

- [Introduction](#introduction)
- [Getting Started](#getting-started)
  - [Prerequisites](#prerequisites)
  - [Installation](#installation)
  - [First Steps](#first-steps)
- [Core Concepts](#core-concepts)
- [Feature Combinations](#feature-combinations)
- [Basic Usage](#basic-usage)
  - [Segment Algorithm](#segment-algorithm)
  - [Snowflake Algorithm](#snowflake-algorithm)
  - [UUID Generation](#uuid-generation)
- [Advanced Usage](#advanced-usage)
  - [Distributed Coordination](#distributed-coordination)
  - [Health Monitoring](#health-monitoring)
  - [Performance Tuning](#performance-tuning)
  - [SDK Embedding](#sdk-embedding)
- [Best Practices](#best-practices)
- [Troubleshooting](#troubleshooting)
- [Next Steps](#next-steps)

---

## Introduction

<div align="center">

### 🎯 What You Will Learn

</div>

<table>
<tr>
<td width="25%" align="center">
<img src="https://img.icons8.com/fluency/96/000000/rocket.png" width="64"><br>
<b>Quick Start</b><br>
Integrate ID generation in 5 minutes
</td>
<td width="25%" align="center">
<img src="https://img.icons8.com/fluency/96/000000/settings.png" width="64"><br>
<b>Multiple Algorithms</b><br>
Segment, Snowflake, UUID
</td>
<td width="25%" align="center">
<img src="https://img.icons8.com/fluency/96/000000/code.png" width="64"><br>
<b>Best Practices</b><br>
Learn distributed ID generation
</td>
<td width="25%" align="center">
<img src="https://img.icons8.com/fluency/96/000000/rocket-take-off.png" width="64"><br>
<b>Advanced Features</b><br>
Master distributed coordination and monitoring
</td>
</tr>
</table>

**Nebula ID** is a powerful enterprise-grade distributed ID generation system offering multiple high-performance, highly available ID generation algorithms, including Segment (number segments), Snowflake, and a time-ordered UUID v8 implementation (RFC 9562 §5.8 custom layout). It is designed for distributed systems, with datacenter health monitoring, automatic failover, and millisecond-level latency.

> 💡 **Tip**: this guide assumes basic Rust knowledge. If you are new to Rust, start with the [official Rust tutorial](https://doc.rust-lang.org/book/).

---

## Getting Started

### Prerequisites

Before starting, make sure you have the following tools installed:

<table>
<tr>
<td width="50%">

**Required**
- ✅ Rust 1.75+ (stable)
- ✅ Cargo (installed with Rust)
- ✅ Git

</td>
<td width="50%">

**Optional**
- 🔧 A Rust-capable IDE (e.g. VS Code + rust-analyzer)
- 🔧 Docker (for containerized deployment)
- 🔧 PostgreSQL (for Segment algorithm persistence)

</td>
</tr>
</table>

<details>
<summary><b>🔍 Verify the installation</b></summary>

```bash
# Check the Rust version
rustc --version
# Expected: rustc 1.75.0 (or higher)

# Check the Cargo version
cargo --version
# Expected: cargo 1.75.0 (or higher)
```

</details>

### Installation

Add `nebulaid` to your `Cargo.toml`:

```toml
[dependencies]
nebulaid = { version = "0.3", features = ["postgresql"] }

# For etcd distributed coordination (recommended in production)
# nebulaid = { version = "0.3", features = ["postgresql", "etcd"] }
```

Or use the command line (adds the latest version to Cargo.toml automatically):

```bash
cargo add nebulaid
```

> 💡 **Available features**: `postgresql` (default), `etcd`, `alerting`, `garrison-auth` (default), `sdk`, `integration-tests`, `http` / `grpc` (default, sdforge mirrors). `--all-features` is buildable. The project is a single package (no `crates/` subdirectory); do not use `path = "./crates/core"`.

### First Steps

Let's verify the installation with a simple example. We will generate a distributed ID with the Segment algorithm (via the unified `AlgorithmBuilder` entry point):

```rust
use nebulaid::core::algorithm::{AlgorithmBuilder, GenerateContext, IdAlgorithm};
use nebulaid::core::types::AlgorithmType;
use nebulaid::core::Config;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load config (datacenter_id and friends live in the [app] section; see config/config.toml)
    let config = Config::default();

    // Build the algorithm instance via the unified AlgorithmBuilder (returns Box<dyn IdAlgorithm>)
    let segment = AlgorithmBuilder::new(AlgorithmType::Segment)
        .build(&config)
        .await?;

    // Generate an ID (requires a GenerateContext)
    let ctx = GenerateContext::default();
    let id = segment.generate(&ctx).await?;

    println!("Generated ID: {}", id);
    Ok(())
}
```

---

## Core Concepts

Understanding these core concepts will help you use `Nebula ID` effectively.

### 1️⃣ ID Generation Algorithms

`Nebula ID` provides five core algorithms:
- **Segment (number segments)**: database-backed segment allocation with high-concurrency batch fetching
- **Snowflake**: Twitter-style distributed IDs, time-ordered, coordination-free
- **UUID v8 (universally unique identifier)**: time-ordered private bit layout (RFC 9562 §5.8, embedding dc/worker/shard, exclusive tenant context)
- **UUID v7**: RFC 9562 §5.7 standard time-ordered — the timestamp is directly readable by any standard tool, zero dc/worker configuration (a true algorithm as of add-uuid-v4-v7, no longer an alias)
- **UUID v4**: 122 random bits + fully zero metadata (leaks neither creation time nor deployment structure; a true algorithm as of add-uuid-v4-v7)

### 2️⃣ Datacenter

The Segment algorithm supports multi-datacenter deployment, with a unique DCID per datacenter used to:
- Isolate ID ranges across datacenters
- Balance load across datacenters
- Support automatic datacenter failover

### 3️⃣ Distributed Coordination

`Nebula ID` ships built-in distributed coordination:
- **Health monitoring**: real-time monitoring of every datacenter's state
- **Failure detection**: automatic detection and marking of failed datacenters
- **Smart scheduling**: traffic automatically moves to healthy datacenters

---

## Feature Combinations

`Nebula ID` uses module-level feature gates to trim the dependency tree per consumption shape (numbers are unique crates measured by `cargo tree -e normal`):

| Consumption shape | Recommended combination | Dependency tree size |
|---|---|---|
| Deploy the full service | default (`server` + five algorithms + `postgresql` + `garrison-auth`) | 519 |
| Embedded SDK | `sdk` (implies `openapi`; does not imply `database`, enable the repository surface explicitly) | 197 (283 for the `sdk,database` combination) |
| Segment only | `segment` (implies `database`) | 282 |
| Snowflake / UUID v8 only | `snowflake` or `uuid-v8` | 196 |
| UUID v7 / v4 only | `uuid-v7` or `uuid-v4` | 196 |
| UUID v5 deterministic derivation only | `uuid-v5` | 196 (the kit is not involved: call `derive_uuid_v5` directly) |

Dependency-tree methodology: `cargo tree -e normal --prefix none | sort -u | wc -l` (unique name+version packages, measured 2026-10-11).

```toml
# Cargo.toml —— a trim example using only UUID v8
[dependencies]
nebulaid = { version = "0.3", default-features = false, features = ["uuid-v8"] }
```

Key points:

- Requesting a non-enabled algorithm returns an `InvalidAlgorithmType` error — no panic, no silent fallback to another algorithm.
- The `nebula-id` binary is only built with the `server` feature (`required-features`); pure-algorithm trim combinations skip the binary target automatically.
- The `alerting` feature gates the alerting subsystem, webhook dispatch, and the `reqwest` dependency together; running alerting unit tests locally needs `cargo test --features alerting` (CI covers it via `--all-features`).
- Trim combinations are pinned by the CI `Check feature trim` step, preventing out-of-scope references from silently rotting the trim capability.

---

## Basic Usage

### Segment Algorithm

The Segment algorithm achieves high-performance ID generation by pre-allocating number segments:

```rust
use nebulaid::core::algorithm::{AlgorithmBuilder, GenerateContext, IdAlgorithm};
use nebulaid::core::types::AlgorithmType;
use nebulaid::core::Config;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // datacenter_id and friends are configured in Config.app (see config/config.toml)
    let config = Config::default();

    // Build the Segment algorithm via AlgorithmBuilder
    let segment = AlgorithmBuilder::new(AlgorithmType::Segment)
        .build(&config)
        .await?;

    // Generate a single ID
    let ctx = GenerateContext::default();
    let id = segment.generate(&ctx).await?;
    println!("Generated ID: {}", id);

    // Batch generation (more efficient — one database interaction)
    let batch = segment.batch_generate(&ctx, 100).await?;
    println!("Generated {} IDs", batch.ids.len());

    Ok(())
}
```

### Snowflake Algorithm

The Snowflake algorithm generates 64-bit ordered IDs with no database coordination:

```rust
use nebulaid::core::algorithm::{AlgorithmBuilder, GenerateContext, IdAlgorithm};
use nebulaid::core::types::AlgorithmType;
use nebulaid::core::Config;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // worker_id and datacenter_id are configured in Config.app
    let config = Config::default();

    // Build the Snowflake algorithm (no database coordination; depends on the clock and worker_id uniqueness)
    let snowflake = AlgorithmBuilder::new(AlgorithmType::Snowflake)
        .build(&config)
        .await?;

    // Generate an ID (Snowflake and Segment share the IdAlgorithm trait interface)
    let ctx = GenerateContext::default();
    let id = snowflake.generate(&ctx).await?;
    println!("Generated ID: {}", id);

    // Batch generation
    let batch = snowflake.batch_generate(&ctx, 100).await?;
    println!("Generated {} IDs", batch.ids.len());

    Ok(())
}
```

### UUID Generation

Time-ordered UUID v8 is supported (RFC 9562 §5.8 custom layout, embedding dc/worker/shard, strictly monotonic):

```rust
use nebulaid::core::algorithm::{AlgorithmBuilder, GenerateContext, IdAlgorithm};
use nebulaid::core::types::AlgorithmType;
use nebulaid::core::Config;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::default();

    // UUID v8 - time-ordered, great for database primary keys
    let v8 = AlgorithmBuilder::new(AlgorithmType::UuidV8)
        .build(&config)
        .await?;

    let ctx = GenerateContext::default();
    let uuid_v8 = v8.generate(&ctx).await?;
    println!("UUID v8: {}", uuid_v8);

    // Batch generation
    let batch = v8.batch_generate(&ctx, 100).await?;
    println!("Generated {} UUIDs", batch.ids.len());

    Ok(())
}
```

> 💡 **About uuid_v7 / uuid_v4** (add-uuid-v4-v7, 2026-10): both were once input aliases of `uuid_v8`
> (configuring `uuid_v7` actually produced v8); they are now reversed into **true algorithms** —
> `"uuid_v7"` resolves to `AlgorithmType::UuidV7` (standard time-ordered UUID) and
> `"uuid_v4"` resolves to `AlgorithmType::UuidV4` (random UUID). The output layout of these
> two strings in historical configs changes accordingly (measured usage of the legacy strings is zero;
> the CHANGELOG carries the breaking statement). v8's own aliases (`uuidv8` / `uuid8`) stay folded.

#### UUID v5 Deterministic Derivation (Not Minting)

Unlike v8's minting semantics, v5 is a **name-based deterministic derivation** (RFC 9562 §5.5):
the same `(namespace, name)` always yields the same ID, fitting scenarios that derive stably from a business identity —
for example dictionary entities derived from `(lemma, language)` (etyma's 340,566 legacy IDs are standard v5
outputs, reproducible bit-for-bit via `derive_uuid_v5`; see the in-repo cross-check test).

Direct-call path (the `uuid-v5` feature, on by default):

```rust
use nebulaid::core::algorithm::derive_uuid_v5;
use nebulaid::core::types::IdFormat;
use uuid::Uuid;

let namespace = Uuid::from_bytes([/* 调用方业务命名空间（16 字节） */]);
let id = derive_uuid_v5(namespace, "ecdict|abandon|en")?;
// The canonical hyphenated string must go through render (Id's Display version whitelist is
// {v4, v7, v8}; calling to_string() directly on v5 prints the decimal numeric value)
println!("{}", id.render(&IdFormat::Uuid, None, None)?);
```

Embedded SDK users call through the unified entry point (the `sdk` feature; a thin wrapper over the same implementation):

```rust
let id = kit
    .id_generator()?
    .derive_uuid_v5(namespace, "ecdict|abandon|en")?;
```

> 💡 v5 does not go through the `/generate` HTTP endpoint or the `AlgorithmRouter` — a derivation failure has no
> actionable "switch to another minting algorithm" semantics (a fallback would produce a numerically wrong non-v5 ID);
> it also has no quota/audit surface (pure computation, no resource-consumption decision).
>
> 💡 **v5-only embedding uses the direct-call path**: kit `build()` requires at least one minting algorithm
> to be present (`AlgorithmRouter::initialize` errors explicitly on an empty registry), and v5 is not a routing
> algorithm and is not counted — embedders who only consume v5 (e.g. `features = ["uuid-v5"]`) call
> `nebulaid::core::algorithm::derive_uuid_v5` directly and need no kit assembly.
>
> 💡 **Namespace convention**: callers should use their own business namespace (avoid the nil
> namespace — different businesses sharing nil with close name spaces will collide);
> the name space must be kept globally unique by the caller (v5 derives identity; it does not guarantee uniqueness).

#### UUID v4 / v7: The Minter's Two Other Versions (add-uuid-v4-v7)

`AlgorithmType` now carries **five true algorithms**. How the four UUID versions divide the work — choose by scenario, not by version-number age:

| Version | Exclusive capability | When to choose it |
|---|---|---|
| `uuid_v4` | 122 random bits, fully zero metadata (leaks neither creation time nor deployment structure) | request ids, capability tokens — "nothing can be inferred from the ID" |
| `uuid_v7` | Standard time-ordered (RFC 9562 §5.7, timestamp directly readable by any standard tool) + zero config (no dc/worker) | Database primary keys needing index locality without binding to the NebulaId private layout |
| `uuid_v8` | Private layout embedding dc/worker/shard (tenant context) | Multi-tenant observability, shard routing, `/parse` topology resolution |
| v5 derivation | Same input, same output | Stable derivation from business identity (e.g. `(lemma, language)`) |

Both versions are used isomorphically with v8 (factory build / config `algorithm.default` / HTTP `algorithm` field all work):

```rust
use nebulaid::core::algorithm::{AlgorithmBuilder, GenerateContext, IdAlgorithm};
use nebulaid::core::types::AlgorithmType;
use nebulaid::core::Config;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::default();

    // UUID v7 —— standard time-ordered, strictly ordered in-process
    let v7 = AlgorithmBuilder::new(AlgorithmType::UuidV7).build(&config).await?;
    // UUID v4 —— random, zero metadata
    let v4 = AlgorithmBuilder::new(AlgorithmType::UuidV4).build(&config).await?;

    let ctx = GenerateContext::default();
    println!("v7: {}", v7.generate(&ctx).await?);
    println!("v4: {}", v4.generate(&ctx).await?);
    Ok(())
}
```

> ⚠️ **Breaking change notice (2026-10)**: the strings `"uuid_v7"` / `"uuid_v4"` in historical configs/API requests
> were previously input aliases of `uuid_v8` (configure v7, get v8); they are now reversed into true algorithms —
> the output layout of these same strings changes accordingly. v8's own aliases
> (`uuidv8` / `uuid8`) are unaffected. See the CHANGELOG breaking entries.

---

## Advanced Usage

### Distributed Coordination

`Nebula ID` supports multi-datacenter deployment for load balancing and failover:

```rust
use nebulaid::core::coordinator::{EtcdClientOps, EtcdClientWrapper, EtcdWorkerAllocator, WorkerIdAllocator};
use nebulaid::core::config::EtcdConfig;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Create the etcd client (use EtcdClientWrapper in production)
    let client: Arc<dyn EtcdClientOps> = Arc::new(
        EtcdClientWrapper::new(vec!["http://localhost:2379".into()]).await?
    );

    // 2. Allocate a unique worker_id in the distributed environment via EtcdWorkerAllocator
    //    Signature: new(client, datacenter_id, etcd_config) -> Result<Self, WorkerAllocatorError>
    let allocator = EtcdWorkerAllocator::new(
        client,
        1,                       // datacenter_id
        EtcdConfig::default(),
    )
    .await?;

    let worker_id = allocator.allocate().await?;
    println!("Allocated worker_id: {}", worker_id);

    // 3. worker_id is normally written into Config.app.worker_id and injected into AlgorithmBuilder by the main.rs startup flow.
    //    Business code usually does not call EtcdWorkerAllocator directly; it consumes the IdGenerator service started by main.rs.

    Ok(())
}
```

### Health Monitoring

Monitor datacenter health in real time:

```rust
use nebulaid::core::coordinator::{EtcdClusterHealthMonitor, EtcdClusterStatus};
use nebulaid::core::config::EtcdConfig;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create the cluster health monitor
    // Signature: new(config: EtcdConfig, cache_file_path: String) -> Self
    // (degrades to the local cache file at cache_file_path when etcd is unavailable)
    let monitor = EtcdClusterHealthMonitor::new(
        EtcdConfig::default(),
        "etcd-cache.json".to_string(),
    );

    // Get the cluster status (synchronous method, returns the EtcdClusterStatus enum)
    let status: EtcdClusterStatus = monitor.get_status();
    println!("Cluster status: {:?}", status);

    // Check whether it degraded to the local cache because etcd was unavailable
    if monitor.is_using_cache() {
        println!("⚠️ etcd 不可用，已降级到本地缓存");
    }

    Ok(())
}
```

### Performance Tuning

Performance configuration for high-concurrency scenarios:

```rust
use nebulaid::core::algorithm::{AlgorithmBuilder, GenerateContext, IdAlgorithm};
use nebulaid::core::config::SegmentAlgorithmConfig;
use nebulaid::core::types::AlgorithmType;
use nebulaid::core::Config;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Tune the Segment algorithm via Config.algorithm.segment
    // Actual fields: base_step / min_step / max_step / switch_threshold
    // (the dynamic-step algorithm auto-switches within [min_step, max_step] based on CPU usage)
    let mut config = Config::default();
    config.algorithm.segment = SegmentAlgorithmConfig {
        base_step: 10000,
        min_step: 1000,
        max_step: 100000,
        switch_threshold: 0.8,
    };

    let segment = AlgorithmBuilder::new(AlgorithmType::Segment)
        .build(&config)
        .await?;

    // Use a channel for async batch generation (batch_generate fetches one batch per call, reducing database interactions)
    let (tx, mut rx) = tokio::sync::mpsc::channel(1000);
    let ctx = GenerateContext::default();

    let handle = tokio::spawn(async move {
        let batch = segment.batch_generate(&ctx, 1000).await?;
        for id in batch.ids {
            tx.send(id).await.map_err(|e| e.to_string())?;
        }
        Ok::<_, Box<dyn std::error::Error>>(())
    });

    let mut ids = Vec::new();
    while let Some(id) = rx.recv().await {
        ids.push(id);
    }

    handle.await??;
    println!("Generated {} IDs", ids.len());

    Ok(())
}
```

---

### SDK Embedding

Nebula ID provides a first-class embedded SDK (feature `sdk`): no need to hand-assemble
distributed locks / routing / degradation background tasks and other wiring details —
`NebulaIdKitBuilder::build()` validates the trait-kit dependency graph and gathers everything internally.

Enable the feature:

```toml
[dependencies]
nebulaid = { version = "0.3", features = ["sdk"] }
```

**Pure-algorithm embedding (zero DB, zero network)** —— see `examples/embedded.rs`:

```rust
use nebulaid::core::Config;
use nebulaid::sdk::NebulaIdKitBuilder;

#[tokio::main]
async fn main() -> nebulaid::core::Result<()> {
    // Pure-algorithm embedding: set the default algorithm to snowflake, avoiding the DB-backed segment
    let mut config = Config::default();
    config.algorithm.default = "snowflake".to_string();

    let kit = NebulaIdKitBuilder::new(config).build().await?;
    let generator = kit.id_generator()?;
    let id = generator.generate("workspace", "group", "biz_tag").await?;
    println!("{id}");

    kit.shutdown().await;
    Ok(())
}
```

> **Note**: the Segment algorithm needs a database for segment allocation. Without a repository injected via
> `with_repository(...)`, Segment requests return
> `CoreError::ConfigurationError`; Snowflake / UuidV8 are pure algorithms and work with zero DB.

**Wrap as a web service (sdforge `#[forge]`)** —— see `examples/sdk_server.rs`:
declare `generate` / `batch_generate` as `POST /generate` and
`POST /generate/batch` with `#[forge]`, and OpenAPI is emitted automatically (`GET /api-docs/openapi.json`).

```bash
cargo run --package nebulaid --example embedded   --features sdk
cargo run --package nebulaid --example sdk_server --features sdk,http
```

---

## Best Practices

<div align="center">

### 🌟 Recommended Design Patterns

</div>

### ✅ Recommended

- **Choose the right algorithm**: pick per requirements - Segment for high concurrency, Snowflake for low latency, UUID for distributed identifiers.
- **Pre-configure the segment size**: set a sensible `base_step` (`SegmentAlgorithmConfig`) based on growth projections to avoid frequent database access.
- **Health monitoring**: always enable health monitoring in production for automatic failover.
- **Batch generation**: for bulk operations (e.g. data imports), use `IdAlgorithm::batch_generate(&ctx, size)` for better performance.
- **Async processing**: handle ID generation requests through async channels in high-concurrency scenarios.

### ❌ Avoid

- **Single-datacenter deployment**: production should deploy multiple datacenters to avoid single points of failure.
- **Ignoring clock rollback**: the Snowflake algorithm needs clock synchronization care to avoid duplicate IDs from clock rollback.
- **Overly small segments**: tiny segments cause frequent database access and hurt performance.
- **Missing monitoring**: production should monitor ID generation latency, error rates, and similar metrics.

---

## Troubleshooting

<details>
<summary><b>❓ Problem: ID generation latency is too high</b></summary>

**Solution:**
1. Check whether `base_step` (`SegmentAlgorithmConfig`) is too small; increase the segment step to reduce database access.
2. Confirm the `max_step` / `switch_threshold` settings are sensible, letting the dynamic-step algorithm enlarge steps automatically under high CPU usage.
3. Check the database connection pool configuration; make sure there are enough connections.
4. Consider `batch_generate(&ctx, size)` to fetch many IDs at once and reduce network overhead.

</details>

<details>
<summary><b>❓ Problem: Datacenter failover does not trigger</b></summary>

**Solution:**
1. Check the health monitoring configuration; make sure heartbeat detection is correctly enabled.
2. Confirm every datacenter is correctly registered with the coordinator.
3. Check the failure detection thresholds; make sure they are within a sensible range.
4. Verify network connectivity; make sure cross-datacenter communication works.

</details>

<details>
<summary><b>❓ Problem: Duplicate Snowflake IDs</b></summary>

**Solution:**
1. Check whether the system clock rolled back; sync time with `ntpdate`.
2. Confirm `datacenter_id` and `worker_id` are unique within the cluster.
3. Check the timestamp acquisition logic; make sure it is monotonically increasing.
4. Enable the clock-rollback protection mechanism.

</details>

<details>
<summary><b>❓ Problem: Database connection pool exhausted</b></summary>

**Solution:**
1. Increase the database connection pool size.
2. Enlarge `base_step` / `max_step` appropriately so one segment load serves more ID requests, reducing concurrent access pressure.
3. Use connection pool reuse to avoid frequent connection creation.
4. Consider read/write splitting, pointing Segment loads at replicas.

</details>

<div align="center">

**💬 Still need help?** [Open an issue](https://github.com/Kirky-X/NebulaId/issues) or [visit the documentation hub](https://github.com/Kirky-X/NebulaId)

</div>

---

## Next Steps

<div align="center">

### 🎯 Keep Exploring

</div>

<table>
<tr>
<td width="33%" align="center">
<a href="API_REFERENCE.md">
<img src="https://img.icons8.com/fluency/96/000000/graduation-cap.png" width="64"><br>
<b>📚 API Reference</b>
</a><br>
Detailed interface documentation
</td>
<td width="33%" align="center">
<a href="ARCHITECTURE.md">
<img src="https://img.icons8.com/fluency/96/000000/settings.png" width="64"><br>
<b>🔧 Architecture</b>
</a><br>
Understand the internals in depth
</td>
<td width="33%" align="center">
<a href="../../examples/">
<img src="https://img.icons8.com/fluency/96/000000/code.png" width="64"><br>
<b>💻 Example Code</b>
</a><br>
Code samples for real scenarios
</td>
</tr>
</table>

---

<div align="center">

**[📖 API docs](https://docs.rs/nebulaid)** • **[❓ FAQ](FAQ.md)** • **[🐛 Report an issue](https://github.com/Kirky-X/NebulaId/issues)**

Made with ❤️ by Kirky.X

[⬆ Back to top](#-nebula-id-user-guide)

</div>
