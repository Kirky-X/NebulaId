# 📘 Nebula ID API Reference

> The complete Nebula ID API documentation: HTTP / gRPC endpoints, headers, error codes, and type definitions.

[🏠 Home](../../README.md) • [📖 User Guide](USER_GUIDE.md) • [🏗️ Architecture](ARCHITECTURE.md)

---

## 📋 Table of Contents

- [Overview](#-overview)
  - [🎯 API Design Principles](#-api-design-principles)
- [🚪 Feature Gates](#-feature-gates)
- [Core API](#-core-api)
  - [TLS Configuration](#tls-configuration)
  - [Smart Segmenting (Dynamic Step)](#smart-segmenting-dynamic-step)
  - [SegmentAlgorithm](#segmentalgorithm)
  - [SnowflakeAlgorithm](#snowflakealgorithm)
  - [UUID Generation](#uuid-generation)
  - [IdAlgorithm Trait](#idalgorithm-trait)
  - [IdGenerator Trait](#idgenerator-trait)
- [Coordinator API](#-coordinator-api)
  - [EtcdClusterHealthMonitor](#etcdclusterhealthmonitor)
  - [DcFailureDetector](#dcfailuredetector)
  - [DcHealthState](#dchealthstate)
- [Type Definitions](#-type-definitions)
  - [`Id`](#id)
  - [`IdBatch`](#idbatch)
  - [`AlgorithmType`](#algorithmtype)
  - [`DcStatus`](#dcstatus)
  - [`EtcdClusterStatus`](#etcdclusterstatus)
  - [`HealthStatus`](#healthstatus)
  - [`GenerateContext`](#generatecontext)
  - [`AlgorithmMetricsSnapshot`](#algorithmmetricssnapshot)
  - [`SegmentInfo`](#segmentinfo)
  - [`ApiKeyWithSecret`](#apikeywithsecret)
- [Error Handling](#-error-handling)
  - [HTTP API Error Response Format](#http-api-error-response-format)
  - [`CoreError`](#coreerror)
  - [Localized Error Responses (v0.2.0+)](#localized-error-responses-v020)
- [HTTP Request Headers](#-http-request-headers)
  - [`X-API-Version` Header](#x-api-version-header)
  - [`Authorization` Header](#authorization-header)
  - [`Accept-Language` Header](#accept-language-header)
- [HTTP Response Headers](#-http-response-headers)
  - [Rate-Limit Response Headers](#rate-limit-response-headers)
- [gRPC Status Codes](#-grpc-status-codes)
- [HTTP Endpoints](#-http-endpoints)
  - [Public Probes and Docs Endpoints](#public-probes-and-docs-endpoints)
  - [ID Generation and Parsing (`ids`)](#id-generation-and-parsing-ids)
  - [Workspace Management (`workspaces`)](#workspace-management-workspaces)
  - [Group Management (`groups`)](#group-management-groups)
  - [BizTag Management (`biz-tags`)](#biztag-management-biz-tags)
  - [API Key Management (`api-keys`, Admin only)](#api-key-management-api-keys-admin-only)
  - [Config Management (`config`)](#config-management-config)
  - [`/health/sdforge`](#healthsdforge)
- [Parameter Validation](#-parameter-validation)
  - [Validation Strategy](#validation-strategy)
  - [Request Parameter Validation](#request-parameter-validation)
  - [Security Validation](#security-validation)
- [Usage Examples](#-usage-examples)
  - [Segment Algorithm Basics](#segment-algorithm-basics)
  - [Snowflake Algorithm](#snowflake-algorithm)
  - [UUID Generation](#uuid-generation-1)
  - [Using the IdAlgorithm Trait](#using-the-idalgorithm-trait)
  - [With Health Monitoring](#with-health-monitoring)
  - [Batch Generation](#batch-generation)

---

## 🎯 Overview

<div align="center">

### 🎯 API Design Principles

</div>

<table>
<tr>
<td width="25%" align="center">
<img src="https://img.icons8.com/fluency/96/000000/easy.png" width="64"><br>
<b>Simple</b><br>
Intuitive and easy to use
</td>
<td width="25%" align="center">
<img src="https://img.icons8.com/fluency/96/000000/security-checked.png" width="64"><br>
<b>Type-safe</b><br>
Thanks to Rust's strong type system
</td>
<td width="25%" align="center">
<img src="https://img.icons8.com/fluency/96/000000/module.png" width="64"><br>
<b>Async-first</b><br>
Built for high-concurrency scenarios
</td>
<td width="25%" align="center">
<img src="https://img.icons8.com/fluency/96/000000/documentation.png" width="64"><br>
<b>Distributed</b><br>
Enterprise-grade scalability
</td>
</tr>
</table>

---

## 🚪 Feature Gates

Module-level feature isolation; library consumers trim the dependency tree as needed:

| feature | Default | Implies | Notes |
|---|:---:|---|---|
| `server` | ✅ | `database` + `http` + `grpc` | The full server layer; the `nebula-id` binary builds with this feature |
| `segment` | ✅ | `database` | The segment algorithm (DB-backed segments) |
| `snowflake` / `uuid-v8` / `uuid-v7` / `uuid-v5` / `uuid-v4` | ✅ | — | Zero-DB leaf algorithms (v8 private layout with tenant context, v7 standard time-ordered zero-config, v4 random zero-metadata; `uuid-v5` is deterministic derivation, not minting) |
| `database` | ➖ | — | The repository layer (dbnexus + sea-orm), implied by `segment` / `server` |
| `postgresql` | ✅ | — | The dbnexus PostgreSQL driver switch |
| `garrison-auth` | ✅ | — | garrison API key authentication |
| `sdk` | ➖ | `openapi` | The embedded SDK facade; does not imply `database` (the repository surface is gated on the `database` cfg — combinations needing a repository explicitly enable `sdk,database`) |
| `alerting` | ➖ | Pulls in `reqwest` | The alerting subsystem and webhook dispatch |
| `etcd` | ✅ | — | etcd distributed coordination (joined `default` in 0.3.0) |
| `openapi` / `integration-tests` | ➖ | — | The sdforge OpenAPI mirror / the real-DB test gate |

Behavioral constraints:

- Generating / batch-generating a non-enabled algorithm returns `CoreError::InvalidAlgorithmType` — no panic, no fallback to another algorithm.
- `IdGenerator::snowflake_layout` exists only with the `snowflake` feature; when it is off, `/parse` returns `unknown` placeholder metadata for snowflake inputs.
- The `nebulaid::core::Cache` public re-export is visible only with the `server` feature.

```toml
# Minimal dependency set with only UUID v8 (197 crates; full default is 521; measured via cargo tree -e normal --prefix none | sort -u | wc -l)
nebulaid = { version = "0.3", default-features = false, features = ["uuid-v8"] }
```

---

## 🧱 Core API

### TLS Configuration

Nebula ID provides TLS 1.2/1.3 encryption for both the HTTP and gRPC servers.

#### `TlsConfig`

The TLS configuration struct.

```rust
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TlsConfig {
    pub enabled: bool,
    pub cert_path: String,
    pub key_path: String,
    pub ca_path: Option<String>,
    pub http_enabled: bool,
    pub grpc_enabled: bool,
    pub min_tls_version: TlsVersion,
    pub alpn_protocols: Vec<String>,
}
```

**Fields:**
- `enabled`: enables TLS globally
- `cert_path`: the TLS certificate file path
- `key_path`: the TLS private key file path
- `ca_path`: the optional CA certificate path, for client authentication
- `http_enabled`: enables HTTPS for the HTTP server
- `grpc_enabled`: enables TLS for the gRPC server
- `min_tls_version`: the minimum TLS version (TLSv12 or TLSv13)
- `alpn_protocols`: the ALPN protocol list (e.g. ["h2", "http/1.1"])

#### `TlsVersion`

The supported TLS versions.

```rust
pub enum TlsVersion {
    Tls12,
    Tls13,
}
```

#### `TlsManager`

The TLS certificate and configuration manager. Implements `Clone` (required when used in `axum::State`).

```rust
#[derive(Clone)]
pub struct TlsManager {
    config: TlsConfig,
    http_acceptor: Option<TlsAcceptor>,
    grpc_tls_config: Option<Arc<ServerTlsConfig>>,
}
```

**Methods:**

```rust
pub fn new(config: TlsConfig) -> Self
pub fn is_http_enabled(&self) -> bool
pub fn is_grpc_enabled(&self) -> bool
pub fn http_acceptor(&self) -> Option<&TlsAcceptor>
pub fn grpc_tls_config(&self) -> Option<&Arc<ServerTlsConfig>>
pub async fn initialize(&mut self) -> TlsResult<()>
```

**Usage:** construct via `TlsManager::new(config)`, then call `manager.initialize().await?` to load the certificate/private-key files and populate `http_acceptor` / `grpc_tls_config`. When `config.enabled == false`, `initialize()` is a no-op (returns `Ok(())` directly). Until the manager has been mutably initialized, `is_http_enabled` / `is_grpc_enabled` never return `true`.

---

### Smart Segmenting (Dynamic Step)

Nebula ID implements CPU-usage-driven dynamic step sizing: each time the segment loader fetches a segment from the database,
it computes that fetch's step from the current CPU usage.

#### `dynamic_step` (step computation)

```
step = clamp(base_step × (1 + 2 × cpu_usage), min_step, max_step)
```

- `base_step` / `min_step` / `max_step`: taken from the `base_step` / `min_step` / `max_step` fields of the `algorithm.segment` config
- `cpu_usage`: the current CPU usage (0-1; out-of-range values are clamped into the interval)
- The busier the CPU, the larger the step: one DB round trip's cost is amortized over more segment ids, lowering the fetch frequency under high load
- The result converges into the `i32` value domain (the DB step column type); a pure function, deterministic and testable —
  see `dynamic_step` in `src/core/algorithm/segment.rs`

#### Wiring: `DbSegmentLoader::with_dynamic_step`

The dynamic step is wired into the segment loader via `DbSegmentLoader::with_dynamic_step(repository, segment_config, cpu_monitor)`
(see [DbSegmentLoader](#dbsegmentloader)): with a `CpuMonitor` passed in, every `load_segment` samples the CPU usage and computes that fetch's step by the formula above. Without this constructor,
the loader fetches at a fixed step (`new` defaults to 1000, or specify with `with_step`).

#### `QpsWindow`

The sliding-window QPS calculator (a double-buffered lock-free implementation; all state is atomic counters).

```rust
#[derive(Debug, Clone)]
pub struct QpsWindow {
    window_secs: u64,
    current_second_count: Arc<std::sync::atomic::AtomicU64>,
    last_second_count: Arc<std::sync::atomic::AtomicU64>,
    current_second: Arc<std::sync::atomic::AtomicU64>,
}
```

**Methods:**

```rust
pub fn new(window_secs: u64) -> Self
pub fn record(&self)
pub fn record_batch(&self, count: usize)
pub fn get_qps(&self) -> u64
pub fn cleanup(&self)
pub fn window_size(&self) -> u64
```

#### `DbSegmentLoader`

The segment loader that connects to the real database segment table (generic constraint `R: SegmentRepository`, defined in
`src/core/algorithm/segment.rs` and re-exported via the `core::algorithm` module).

```rust
pub struct DbSegmentLoader<R: SegmentRepository> {
    repository: Arc<R>,
    step: i32,
    dynamic: Option<DynamicStepPolicy>, // private policy struct: when Some, it overrides the fixed step
}
```

**Constructors:**

```rust
pub fn new(repository: Arc<R>) -> Self
pub fn with_step(repository: Arc<R>, step: i32) -> Self
pub fn with_dynamic_step(
    repository: Arc<R>,
    segment_config: SegmentAlgorithmConfig,
    cpu_monitor: Arc<CpuMonitor>,
) -> Self
```

- `new`: fetches at the default step (`DEFAULT_STEP = 1000`, aligned with
  `SegmentAlgorithmConfig::default().base_step`)
- `with_step`: fetches at the specified step; the step should fall within the biz_tag's `base_step`–`max_step`
  constraints — an oversized step widens the segment hole if the DB crashes
- `with_dynamic_step`: the dynamic-step policy, computing each fetch's step within
  `[min_step, max_step]` by CPU usage (see [Smart Segmenting (Dynamic Step)](#smart-segmenting-dynamic-step)),
  overriding the fixed step

`DbSegmentLoader` implements the `SegmentLoader` trait (`load_segment(&self, ctx, worker_id)`;
the `worker_id` parameter takes no part in the allocation key and is kept only for trait-signature compatibility); after construction, inject it into the segment algorithm via
`SegmentAlgorithm::with_segment_loader` (or `AlgorithmBuilder::with_segment_loader`).

---

### SegmentAlgorithm

`SegmentAlgorithm` is the high-performance distributed ID generator based on the segment algorithm. It pre-requests ID ranges from the database to mint batches efficiently.

#### `SegmentAlgorithm::new(dc_id: u8)`

Creates a new segment algorithm instance with the given datacenter ID.

```rust
pub fn new(dc_id: u8) -> Self
```

**Parameters:**
- `dc_id`: the datacenter ID (`u8` fully valid; the config entry `app.dc_id` is constrained by the
  `datacenter_id_bits` width validation — default width 3, i.e. 0-7)

#### `with_segment_loader(loader: Arc<dyn SegmentLoader + Send + Sync>)`

Injects a custom segment loader (builder style, returns `Self`). Production assembly uses this to replace
`new()`'s default loader with a `DbSegmentLoader` (a real DB-backed segment loader); tests may inject
an in-memory segment maker.

```rust
pub fn with_segment_loader(mut self, loader: Arc<dyn SegmentLoader + Send + Sync>) -> Self
```

#### `with_cpu_monitor(cpu_monitor: Arc<CpuMonitor>)`

Mounts a CPU monitor (builder style). Sampling starts during `initialize`, for the dynamic-step
policy to consume (see [Smart Segmenting (Dynamic Step)](#smart-segmenting-dynamic-step)).

```rust
pub fn with_cpu_monitor(mut self, cpu_monitor: Arc<CpuMonitor>) -> Self
```

#### `with_etcd_cluster_health_monitor(monitor: Arc<EtcdClusterHealthMonitor>)`

Mounts an etcd cluster health monitor (builder style) for distributed coordination. Exists only under the `etcd`
feature.

```rust
#[cfg(feature = "etcd")]
pub fn with_etcd_cluster_health_monitor(
    mut self,
    monitor: Arc<EtcdClusterHealthMonitor>,
) -> Self
```

#### Minting entry points (the `IdAlgorithm` trait)

All public minting goes through the `IdAlgorithm` trait (the full trait in [IdAlgorithm Trait](#idalgorithm-trait)):

```rust
async fn generate(&self, ctx: &GenerateContext) -> Result<Id>
async fn batch_generate(&self, ctx: &GenerateContext, size: usize) -> Result<IdBatch>
```

**Parameters:**
- `ctx`: a `GenerateContext`, locating the segment buffer by `workspace_id` / `group_id` / `biz_tag`
- `size`: how many IDs to generate (the core layer itself does no upper-bound truncation; the cap is enforced at the access layer per the
  `batch_generate.max_batch_size` config — default 100, see [Parameter Validation](#-parameter-validation))

**Returns:** `Result<Id>` / `Result<IdBatch>` — the generated ID or ID batch; when segments are exhausted it
returns `CoreError::SegmentExhausted`.

**Note:** the batch-size cap is an **access-layer policy**, not a core-layer hardcode: the HTTP/gRPC layer rejects oversized requests per the
`batch_generate.max_batch_size` config (default `100`, configurable, validation cap 10000),
guarding against DoS attacks while keeping optimal performance.

---

### SnowflakeAlgorithm

`SnowflakeAlgorithm` implements the Twitter Snowflake algorithm with configurable datacenter, worker, and sequence bit allocations.

> **Reachability:** the `SnowflakeAlgorithm` type itself is not publicly exported from the crate (the `snowflake`
> module is `pub(crate)`, re-exporting only `ParsedSnowflakeId` / `SnowflakeLayoutInfo`).
> External code should build through the public factory: `AlgorithmBuilder::new(AlgorithmType::Snowflake).build(&config)`.
> The method signatures below aid understanding of the internals; for construction, `AlgorithmBuilder` is authoritative.

#### `SnowflakeAlgorithm::new(datacenter_id: u8, worker_id: u8)` (crate-internal)

Creates a new Snowflake algorithm instance.

```rust
pub fn new(datacenter_id: u8, worker_id: u8) -> Self
```

**Parameters:**
- `datacenter_id`: the datacenter ID (default width 3, 0-7)
- `worker_id`: the worker ID (default width 8, 0-255)

#### `generate_id()` (crate-private)

Generates a single ID using the Snowflake algorithm.

```rust
async fn generate_id(&self) -> Result<Id>
```

`generate_id` is a crate-private async method (`src/core/algorithm/snowflake.rs`),
not a public API; `IdAlgorithm::generate` delegates single generation to it internally. For the public minting
entry, see "Public entry points (the `IdAlgorithm` trait)" below.

**Errors:**
- `CoreError::ClockMovedBackward` — the system clock moved backward
- `CoreError::SequenceOverflow` — the sequence number overflowed within one millisecond

#### Public entry points (the `IdAlgorithm` trait)

```rust
async fn generate(&self, ctx: &GenerateContext) -> Result<Id>
async fn batch_generate(&self, ctx: &GenerateContext, size: usize) -> Result<IdBatch>
```

Timestamp / datacenter / worker / sequence all come from the instance config and the system clock; `ctx` only carries
the caller's context and takes no part in the bit layout. The bit allocation is decided by the `algorithm.snowflake` config; generated
IDs can be parsed with `ParsedSnowflakeId` / `SnowflakeLayoutInfo` (re-exported by `nebulaid::core::algorithm`).

---

### UUID Generation

#### `UuidV8Impl`

The time-ordered RFC 9562 **v8** UUID generator (`src/core/algorithm/uuid_v8.rs`) — a private bit layout
(dc/worker/counter + shard); embedded tenant context is its exclusive capability. Random v4 and standard time-ordered v7 are
parallel true algorithms (see the "UUID v4 / v7" section at the end of this chapter).

Reachability: the `uuid_v8` module is `pub(crate)` (`src/core/algorithm/mod.rs`), so
`UuidV8Impl` **cannot** be imported from outside the crate. Use the public factory instead:

```rust
// Inside an `async fn` (the builder's `build` is async):
use nebulaid::core::algorithm::AlgorithmBuilder;
use nebulaid::core::types::AlgorithmType;
use nebulaid::core::Config;

let config = Config::default();
let uuid_alg = AlgorithmBuilder::new(AlgorithmType::UuidV8)
    .build(&config)
    .await?;
```

**Constructor (crate-internal):**

```rust
pub fn new(dc_id: u64, worker_id: u64, clock_drift_threshold_ms: u64) -> Self
```

`dc_id` / `worker_id` come from `Config.app`, and `clock_drift_threshold_ms` from
`Config.algorithm.snowflake` (see `UuidV8Factory::build`). `Default` is equivalent to `new(0, 0, 1000)`.

**Methods (via the `IdAlgorithm` trait):**

```rust
pub async fn generate(&self, ctx: &GenerateContext) -> Result<Id>
pub async fn batch_generate(&self, ctx: &GenerateContext, size: usize) -> Result<IdBatch>
```

> **Version selection (after add-uuid-v4-v7)**: the `uuid_v4` /
> `uuid_v7` name families of `AlgorithmType::from_str` have reversed from "input aliases mapping to `UuidV8`" into **true algorithms**
> (`UuidV4` / `UuidV7`); v8's own aliases (`uuidv8` / `uuid8`) stay folded.
> The `Display` output and the `from_str` roundtrip are self-consistent (spec in change `add-uuid-v4-v7`).
> UUID version division: v4 random zero-metadata, v7 standard time-ordered zero-config, v8 tenant-context
> private layout, v5 deterministic derivation (see next section).

#### `derive_uuid_v5` / `derive_uuid_v5_batch`

RFC 9562 §5.5 **deterministic derivation** (`src/core/algorithm/uuid_v5.rs`, gated by feature `uuid-v5`,
on by default). The same `(namespace, name)` always yields the same output — serving scenarios that derive stably from a business identity
(e.g. etyma's dictionary entities derived from `(lemma, language)`; its 340,566 legacy IDs are standard
v5 outputs, bit-compatible with this function).

```rust
pub fn derive_uuid_v5(namespace: Uuid, name: &str) -> Result<Id>
pub fn derive_uuid_v5_batch(pairs: &[(Uuid, &str)]) -> Result<Vec<Id>>
```

Synchronous pure functions (stateless / clockless / IO-free): internally just `Id::from_uuid_v8(Uuid::new_v5(namespace,
name.as_bytes()))`, the same crate and version as `Uuid::new_v5`, numerically bit-identical.

**Boundary with v8**: v5 does **not** implement `IdAlgorithm`, never enters `AlgorithmRouter`/`AlgorithmType`, and
bypasses `/generate` — a derivation failure has no actionable "switch minting algorithm" semantics (a fallback would produce a numerically wrong
non-v5 ID); it also has no quota/audit surface. The SDK surface's `IdGenerator::derive_uuid_v5` (feature `sdk`)
is a synchronous thin wrapper; v5-only embedding (e.g. `features = ["uuid-v5"]`) uses the direct-call path — kit
`build()` requires at least one minting algorithm to be present (`AlgorithmRouter::initialize` errors explicitly on an empty registry),
and v5 is not a routing algorithm and is not counted.

> **Output note**: v5's `Id` is UUID-shaped, but the `Display` version whitelist is {v4, v7, v8}
> — calling `to_string()` directly yields the decimal numeric value; for the canonical hyphenated string use
> `Id::render(&IdFormat::Uuid, None, None)`. Callers should use their own business namespace
> (never the nil namespace), and the name space must be kept globally unique by the caller.

---

#### `UuidV4Impl` / `UuidV7Impl` (add-uuid-v4-v7)

Two zero-state minters (`src/core/algorithm/uuid_v4.rs` / `uuid_v7.rs`, gated by feature `uuid-v4` / `uuid-v7`).

```rust
pub fn generate(&self, ctx: &GenerateContext) -> Result<Id>   // via IdAlgorithm
pub async fn batch_generate(&self, ctx: &GenerateContext, size: usize) -> Result<IdBatch>
```

- `UuidV4Impl`: `generate` = `Id::from_uuid_v8(Uuid::new_v4())` — 122 random bits, fully zero metadata. Zero state (no clock/counter/coordination). Batch has no overflow guard (uniqueness is guaranteed by random entropy).
- `UuidV7Impl`: `generate` = `Id::from_uuid_v8(Uuid::now_v7())` — RFC 9562 §5.7 standard time-ordered; in-process ordering is provided by the uuid crate's shared counter context (this crate holds zero state of its own). Batch has no overflow guard (the crate's counter carries into +1ms on overflow to preserve ordering — physically unreachable).

**The dividing line with v8**: v4/v7 accept and embed no `workspace/group/biz_tag` context (embedded tenant context is v8's exclusive capability). Select them via `AlgorithmRouter::generate_with_algorithm(AlgorithmType::UuidV4 | UuidV7, ...)` or the SDK's `generate_with_algorithm`; the `uuid_v4`/`uuid_v7` strings of `AlgorithmType::from_str` map directly to the true algorithms (breaking reversal — see the CHANGELOG).

**Ordering boundary**: v7 is strictly ordered within a process; across processes only to millisecond granularity — strict cross-process ordering is v8's (dc/worker + wall clock) territory.

---

### IdAlgorithm Trait

The core trait every ID generation algorithm must implement.

```rust
pub trait IdAlgorithm: Send + Sync {
    async fn generate(&self, ctx: &GenerateContext) -> Result<Id>;
    async fn batch_generate(&self, ctx: &GenerateContext, size: usize) -> Result<IdBatch>;
    fn health_check(&self) -> HealthStatus;
    fn metrics(&self) -> AlgorithmMetricsSnapshot;
    fn algorithm_type(&self) -> AlgorithmType;
    async fn shutdown(&self) -> Result<()>;
}
```

> **Note:** the `async fn initialize(&mut self, config: &Config)` method was removed from this trait in L13 (see `src/core/algorithm/traits.rs:37-48`). Algorithm initialization now happens through each algorithm struct's own method (e.g. `SnowflakeAlgorithm::initialize`), called by `AlgorithmBuilder::build` before handing over the `Box<dyn IdAlgorithm>`. This keeps the trait object-safe and avoids requiring `&mut self` on a shared `Arc<dyn IdAlgorithm>` reference.

#### `generate()`

Generates a single ID.

```rust
async fn generate(&self, ctx: &GenerateContext) -> Result<Id>
```

#### `batch_generate()`

Generates multiple IDs in a batch.

```rust
async fn batch_generate(&self, ctx: &GenerateContext, size: usize) -> Result<IdBatch>
```

#### `health_check()`

Checks the algorithm's health state.

```rust
fn health_check(&self) -> HealthStatus
```

**Returns:** `HealthStatus` — one of `Healthy`, `Degraded(reason)`, or `Unhealthy(reason)`

#### `metrics()`

Fetches the algorithm's performance metrics.

```rust
fn metrics(&self) -> AlgorithmMetricsSnapshot
```

#### `algorithm_type()`

Fetches the algorithm type.

```rust
fn algorithm_type(&self) -> AlgorithmType
```

**Returns:** `AlgorithmType` — one of `Segment`, `Snowflake`, `UuidV8`

#### `shutdown()`

Shuts the algorithm down gracefully and releases resources.

```rust
async fn shutdown(&self) -> Result<()>
```

---

### IdGenerator Trait

The high-level ID generator interface, organized by workspace/group/tag.

```rust
pub trait IdGenerator: Send + Sync {
    async fn generate(&self, workspace: &str, group: &str, biz_tag: &str) -> Result<Id>;
    async fn batch_generate(
        &self,
        workspace: &str,
        group: &str,
        biz_tag: &str,
        size: usize,
    ) -> Result<Vec<Id>>;
    async fn generate_with_algorithm(
        &self,
        algorithm: AlgorithmType,
        workspace: &str,
        group: &str,
        biz_tag: &str,
    ) -> Result<Id>;
    async fn batch_generate_with_algorithm(
        &self,
        algorithm: AlgorithmType,
        workspace: &str,
        group: &str,
        biz_tag: &str,
        size: usize,
    ) -> Result<Vec<Id>>;
    async fn get_algorithm_name(
        &self,
        workspace: &str,
        group: &str,
        biz_tag: &str,
    ) -> Result<String>;
    async fn health_check(&self) -> HealthStatus;
    async fn get_primary_algorithm(&self) -> String;
    fn get_degradation_manager(&self) -> &Arc<DegradationManager>;
}
```

> **Note:** the `set_algorithm` method has been removed (it does not exist in `src/core/algorithm/traits.rs:58-99`). Per-algorithm routing is handled through `generate_with_algorithm` / `batch_generate_with_algorithm`, passing the `AlgorithmType` argument at call time.

---

## 🌐 Coordinator API

### EtcdClusterHealthMonitor

Monitors the etcd cluster's health, with a local-cache fallback.

#### `EtcdClusterHealthMonitor::new(config: EtcdConfig, cache_file_path: String)`

Creates a new health monitor.

```rust
pub fn new(config: EtcdConfig, cache_file_path: String) -> Self
```

#### `get_status()`

Fetches the current cluster status.

```rust
pub fn get_status(&self) -> EtcdClusterStatus
```

**Returns:** `EtcdClusterStatus` — one of `Healthy`, `Degraded`, `Failed`

#### `set_status(status: EtcdClusterStatus)`

Sets the cluster status manually.

```rust
pub fn set_status(&self, status: EtcdClusterStatus)
```

#### `record_success()`

Records one successful operation.

```rust
pub async fn record_success(&self)
```

#### `record_failure()`

Records one failed operation.

```rust
pub fn record_failure(&self)
```

#### `is_using_cache()`

Checks whether the local-cache fallback is currently in use.

```rust
pub fn is_using_cache(&self) -> bool
```

#### `load_local_cache()`

Loads cached data from the local file.

```rust
pub async fn load_local_cache(&self) -> Result<()>
```

#### `save_local_cache()`

Saves the current cached data to the local file.

```rust
pub async fn save_local_cache(&self) -> Result<()>
```

---

### DcFailureDetector

Detects and manages datacenter health states. Defined in `src/core/algorithm/segment.rs`;
the `segment` module is `pub(crate)` and this type is not re-exported — it is a crate-internal type,
built and driven inside `SegmentAlgorithm`; external code cannot import it.

```rust
pub struct DcFailureDetector {
    dc_states: Arc<RwLock<HashMap<u8, Arc<DcHealthState>>>>,
    failure_threshold: u64,
    recovery_timeout: Duration,
}
```

**Methods:**

```rust
pub fn new(failure_threshold: u64, recovery_timeout: Duration) -> Self
pub fn add_dc(&self, dc_id: u8)
pub async fn start_health_check_with_shutdown(
    &self,
    check_interval: Duration,
    shutdown_rx: tokio::sync::watch::Receiver<bool>,
)
```

- `new`: `failure_threshold` is the number of consecutive failures allowed before declaring a fault, and `recovery_timeout`
  is how long to wait before attempting recovery
- `add_dc`: registers a datacenter to monitor (idempotent — an existing DC is not rebuilt)
- `start_health_check_with_shutdown`: starts the background health-check loop, performing recovery checks every `check_interval`;
  the loop exits once `shutdown_rx` receives the shutdown signal

`SegmentAlgorithm::new` internally builds the detector with `DcFailureDetector::new(5, Duration::from_secs(300))`
and registers the local DC; `initialize` starts the health check with the shutdown watch channel passed in.

---

### DcHealthState

Represents one datacenter's health state.

```rust
pub struct DcHealthState {
    pub dc_id: u8,
    pub status: AtomicU8,
    pub last_success: Arc<Mutex<Instant>>,
}
```

**Methods:**

```rust
pub fn new(dc_id: u8) -> Self
pub fn get_status(&self) -> DcStatus
pub fn set_status(&self, status: DcStatus)
```

`status` atomically stores and loads the `u8` discriminant of `DcStatus`
(`Healthy` / `Degraded` / `Failed`).

---

## 📐 Type Definitions

### `Id`

The core ID type in Nebula ID.

```rust
pub struct Id {
    // Internal representation (u128)
}
```

**Methods:**

```rust
pub fn from_u128(value: u128) -> Self
pub fn as_u128(&self) -> u128
pub fn from_i64(value: i64) -> Self
pub fn as_i64(&self) -> i64
pub fn from_string(s: &str) -> Result<Self, CoreError>
pub fn from_uuid_v8(uuid: Uuid) -> Self
pub fn to_uuid_v8(&self) -> Uuid
pub fn to_prefixed(&self, prefix: &str) -> String
pub fn to_hex(&self) -> String
pub fn to_base36(&self) -> String
// `Display` renders a standard UUID string when the version nibble is 4/7/8,
// otherwise the plain u128 (`src/core/types/id.rs:55-70`).
```

> `Id` has no `from_uuid_v7` / `from_uuid_v4` constructors — `from_uuid_v8` (a historical name)
> carries any Uuid → Id wrapping; UUID v4/v7 enter the runtime through their own `UuidV4Impl` / `UuidV7Impl`
> algorithm entities (see the "UUID Generation" chapter).

### `IdBatch`

A batch of generated IDs.

```rust
pub struct IdBatch {
    pub ids: Vec<Id>,
    pub algorithm: AlgorithmType,
    pub biz_tag: String,
    pub generated_at: chrono::DateTime<chrono::Utc>,
}
```

**Methods:**

```rust
pub fn new(ids: Vec<Id>, algorithm: AlgorithmType, biz_tag: String) -> Self
pub fn from_u64s(values: &[u64]) -> Self
pub fn len(&self) -> usize
pub fn is_empty(&self) -> bool
```

### `AlgorithmType`

The enum of supported algorithm types.

```rust
pub enum AlgorithmType {
    #[default]
    Segment,
    Snowflake,
    UuidV8,
    UuidV4,
    UuidV7,
}
```

The string surface of `AlgorithmType::from_str` (since add-uuid-v4-v7): `uuid_v4`/`uuidv4`/`uuid4` → `UuidV4`,
`uuid_v7`/`uuidv7`/`uuid7` → `UuidV7` (**no longer** input aliases of `UuidV8` — a breaking reversal,
see the CHANGELOG), while `uuid_v8`/`uuidv8`/`uuid8` keep → `UuidV8`; case-insensitive, unknown strings return
`InvalidAlgorithmType`. `Display` outputs the canonical names (`segment` / `snowflake` / `uuid_v8` /
`uuid_v4` / `uuid_v7`), self-consistent with the `from_str` roundtrip.

### `DcStatus`

The datacenter health state.

```rust
pub enum DcStatus {
    Healthy,
    Degraded,
    Failed,
}
```

### `EtcdClusterStatus`

The etcd cluster health state.

```rust
pub enum EtcdClusterStatus {
    Healthy,
    Degraded,
    Failed,
}
```

### `HealthStatus`

The algorithm health state.

```rust
pub enum HealthStatus {
    Healthy,
    Degraded(String),
    Unhealthy(String),
}
```

### `GenerateContext`

The context of an ID generation request.

```rust
#[derive(Debug, Clone)]
pub struct GenerateContext {
    pub workspace_id: String,
    pub group_id: String,
    pub biz_tag: String,
    pub format: IdFormat,
    pub prefix: Option<String>,
}
```

### `AlgorithmMetricsSnapshot`

The performance metrics snapshot.

```rust
#[derive(Debug, Clone, Default)]
pub struct AlgorithmMetricsSnapshot {
    pub total_generated: u64,
    pub total_failed: u64,
    pub current_qps: u64,
    pub p50_latency_us: u64,
    pub p99_latency_us: u64,
    pub p999_latency_us: u64,
    pub clock_backwards: u64,
    pub cache_hit_rate: Option<f64>,
}
```

> `p50/p99/p999_latency_us` and `clock_backwards` are filled by
> `AlgorithmRouter::metrics()` from the routing layer's observed ring buffer; the algorithm itself returns `0`,
> meaning "not yet observed by the router", not "zero latency".
> `cache_hit_rate = None` means the algorithm has no cache concept (Snowflake / UUID v8) —
> using `Option` avoids counting "no cache" as a 0% hit rate in averages
> (`src/core/algorithm/traits.rs:138-159`).

### `SegmentInfo`

The database segment information.

```rust
pub struct SegmentInfo {
    pub id: i64,
    pub workspace_id: String,
    pub biz_tag: String,
    pub current_id: i64,
    pub max_id: i64,
    pub step: u32,
    pub delta: u32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
```

### `ApiKeyWithSecret`

The return result of creating or rotating an API key — the plaintext credential is returned once only.

```rust
pub struct ApiKeyWithSecret {
    pub key: ApiKeyResponse,
    pub key_secret: String,
    pub grace_expires_at: Option<DateTime>,
}
```

> `grace_expires_at` (`src/core/database/api_key_entity.rs:189`) is non-`None` only when a rotation happened with
> `auth.key_rotation_grace_period_seconds > 0`: it is an absolute
> UTC deadline before which the **previous** key's secret still authenticates
> (`prev_secret_hash` + `rotate_expires_at`, lazily compared inside `validate_api_key`).
> `None` means "no active grace window" — the default (`0`) and all newly created keys are in this state.
> Adding the field to this `pub` struct is a **breaking change** for external code constructing
> `ApiKeyWithSecret` with a struct literal.
>
> The wire model is `ApiKeyWithSecretResponse` (`src/server/models.rs:749`), where the same
> field is an RFC 3339 string or `null`. Stating the boundary honestly: `ApiHandlers::rotate_api_key`
> (`src/server/handlers/api_key_handlers.rs:274`) **has no corresponding HTTP route** — the key-related management
> routes are `POST /api-keys`, `GET /api-keys`, `DELETE /api-keys/{id}`, and
> `POST /workspaces/{name}/regenerate-user-key`, and the latter two take the *create* /
> *delete-then-recreate* paths, so `grace_expires_at` is always `null` there. There is no
> callable `POST /api-keys/{id}/rotate`.

---

## 🚨 Error Handling

### HTTP API Error Response Format

All HTTP API endpoints (including API version negotiation failures) return errors in the **unified error envelope**
(`src/server/models.rs::ErrorResponse`):

```json
{
  "code": 400,
  "business_code": "3002",
  "message": "Validation failed",
  "details": "size must be between 1 and 100",
  "request_id": "3f2504e0-4f89-41d3-9a0c-0305e82c3301",
  "timestamp": 1758096000000
}
```

**Fields:**
- `code`: the HTTP status code (e.g. 400, 401, 404, 500)
- `business_code`: the stable machine-readable business code (a four-digit string, e.g. `"3002"`).
  Clients should **branch on this field**, not parse the `message` copy (the copy localizes with
  `Accept-Language`). See the [`CoreError`](#coreerror) table below for the mapping.
- `message`: the human-readable error message (translated per the negotiated `Locale`)
- `details`: optional detailed error information
- `request_id`: the request tracing ID (a UUID v4 generated at response assembly, correlating with the audit log)
- `timestamp`: when the error occurred (a millisecond Unix timestamp)

**Common HTTP status codes:**

| Status | Meaning | Examples |
|--------|---------|----------|
| 400 | Bad request | Invalid parameters, validation failure, unsupported API version |
| 401 | Unauthorized | Missing or invalid API key |
| 403 | Forbidden | Insufficient privileges (Admin required) / cross-workspace access |
| 404 | Not found | The resource does not exist |
| 429 | Too many requests | Rate-limit threshold exceeded |
| 500 | Internal server error | Server-side error |
| 503 | Service unavailable | Downstream dependency timeout |

> **The single exception**: when the global rate-limit middleware (`src/server/rate_limit/middleware.rs`) rejects a request, its 429 uses the standalone body `{"code": 429, "message": "Rate limit exceeded", "retry_after": <n|null>}`
> (without `business_code` or other envelope fields; for `Retry-After` semantics see [Rate-Limit Response Headers](#rate-limit-response-headers)).
> The core layer's `CoreError::RateLimitExceeded`, when it goes through the handler assembly path, still uses the unified envelope
> (429 + `business_code "4001"`).

### `CoreError`

**All** variants of `CoreError` in `src/core/types/error.rs` (the code is authoritative), with their HTTP status codes and
`business_code` as mapped by the single table `core_error_classification`
(`src/server/handlers/helpers.rs`):

| Variant | Meaning | HTTP | `business_code` |
|---------|---------|------|-----------------|
| `InvalidIdFormat(String)` | Illegal ID format | 400 | `3001` |
| `InvalidIdString(String)` | The ID string cannot be parsed | 400 | `3001` |
| `InvalidAlgorithmType(String)` | Illegal algorithm type | 400 | `3001` |
| `InvalidInput(String)` | Illegal input (e.g. batch size 0/over cap) | 400 | `3001` |
| `ParseError(String)` | Parse error (e.g. numeric parse failure) | 400 | `3001` |
| `AuthenticationError(String)` | Authentication failure | 401 | `1001` |
| `InvalidApiKeySignature` | Illegal API key signature/format | 401 | `1003` |
| `ApiKeyExpired` | The API key has expired | 401 | `1004` |
| `ApiKeyDisabled` | The API key is disabled | 401 | `1005` |
| `WorkspaceDisabled(String)` | The workspace is disabled | 403 | `1002` |
| `NotFound(String)` | The resource was not found (defaults to a workspace 404) | 404 | `2001` |
| `BizTagNotFound(String)` | The biz-tag was not found | 404 | `2003` |
| `RateLimitExceeded` | Rate limiting triggered | 429 | `4001` |
| `TimeoutError` | Downstream dependency timeout | 503 | `5004` |
| `DatabaseError(String, Option<ErrorSource>)` | A database operation failed | 500 | `5002` |
| `CacheError(String)` | The cache service failed | 500 | `5003` |
| `InternalError(String)` | An internal error with a description | 500 | `5001` |
| `ConfigurationError(String)` | A configuration error | 500 | `5001` |
| `EtcdError(String, Option<ErrorSource>)` | An etcd coordination-layer error | 500 | `5001` |
| `IoError(String, Option<ErrorSource>)` | An I/O error | 500 | `5001` |
| `ClockMovedBackward { last_timestamp }` | The system clock moved backward; duplicate IDs possible | 500 | `5001` |
| `SequenceOverflow { timestamp }` | The sequence overflowed within one millisecond | 500 | `5001` |
| `SegmentExhausted { max_id }` | The segment is fully exhausted | 500 | `5001` |
| `Unknown` | An unknown error | 500 | `5001` |

> The algorithm variants (`ClockMovedBackward` / `SequenceOverflow` / `SegmentExhausted`)
> and the remaining unlisted 5xx-class variants all converge to 500 + `5001` (`InternalError`),
> never leaking internal details to the client.
>
> The second field of `DatabaseError` / `EtcdError` / `IoError`, `Option<ErrorSource>`,
> is the `#[source]` underlying error source: `.0` is the first-layer human-readable copy, and `.1` goes to server logs only
> (`src/core/types/error.rs:99-100,134-145`), never into the HTTP response body.

### Localized Error Responses (v0.2.0+)

Since v0.2.0, every HTTP error response's `message` and `details` fields
are translated per the request's negotiated `Locale` (see
[the `Accept-Language` header](#accept-language-header)). The HTTP status code,
`business_code`, and JSON structure stay the same across languages; only the human-readable text differs.

**Example - 400 bad request (English, default):**

```http
HTTP/1.1 400 Bad Request
Content-Type: application/json

{
  "code": 400,
  "business_code": "3001",
  "message": "Invalid input: negative",
  "details": "size must be between 1 and 100",
  "request_id": "3f2504e0-4f89-41d3-9a0c-0305e82c3301",
  "timestamp": 1758096000000
}
```

**Example - 400 bad request (Simplified Chinese, `Accept-Language: zh-CN`):**

```http
HTTP/1.1 400 Bad Request
Content-Type: application/json

{
  "code": 400,
  "business_code": "3001",
  "message": "无效输入：negative",
  "details": "size 必须在 1 到 100 之间",
  "request_id": "3f2504e0-4f89-41d3-9a0c-0305e82c3301",
  "timestamp": 1758096000000
}
```

**Example - 500 internal server error (English):**

```json
{
  "code": 500,
  "business_code": "5002",
  "message": "Internal server error",
  "details": null,
  "request_id": "3f2504e0-4f89-41d3-9a0c-0305e82c3302",
  "timestamp": 1758096000001
}
```

**Example - 500 internal server error (Simplified Chinese):**

```json
{
  "code": 500,
  "business_code": "5002",
  "message": "服务器内部错误",
  "details": null,
  "request_id": "3f2504e0-4f89-41d3-9a0c-0305e82c3302",
  "timestamp": 1758096000001
}
```

> 5xx responses are sanitized by `core_error_to_response`: `message` converges to generic copy,
> and original error details go to server logs only, avoiding leaking internals to the client.

Translation keys live in `locales/en/messages.ftl` and `locales/zh/messages.ftl`, in the
`error.*` namespace. A missing key first falls back to the default language (`en`),
then to the key itself (never an empty string).

---

## 📨 HTTP Request Headers

### `X-API-Version` Header

The `X-API-Version` header (`src/server/api_version.rs`) negotiates the API version,
handled by `api_version_middleware` on **all `/api/v1/*` routes**
(`/health`, `/ready`, `/metrics`, `/api-docs/openapi.json` bypass this middleware).

**Negotiation mechanism (the code is authoritative):**

1. Reads the `X-API-Version` header; **a missing or illegal value falls back to the default version `v1`**
   (fail-open, never 4xx).
2. The value is case-insensitive and accepts numeric forms: `v1` / `1` → `ApiVersion::V1`,
   `v2` / `2` → `ApiVersion::V2`.
3. Only `v1` is supported (`is_supported()`): `v2` parses but is not yet open, and such requests are rejected.
4. Unsupported versions return `400` with the unified error envelope body
   (`business_code = "3001"`, `message = "Unsupported API version"`,
   `details` listing the requested and supported versions).
5. When the request passes, the response writes back the `X-API-Version: v1` header.

**Examples:**

```bash
# Explicitly request v1 (equivalent to omitting the header)
curl -H "X-API-Version: v1" http://localhost:8080/api/v1/generate ...

# Unsupported version → 400 + business_code=3001
curl -i -H "X-API-Version: v2" http://localhost:8080/api/v1/config
# The x-api-version response header is missing; the body is:
# {"code":400,"business_code":"3001","message":"Unsupported API version",
#  "details":"Requested version: 'v2'. Supported versions: v1",...}
```

### `Authorization` Header

Except for the public probes (`/health`, `/ready`, `/metrics`, `/api-docs/openapi.json`,
`GET /api/v1`), every endpoint requires API key authentication. Credentials travel in the `Authorization`
header (`src/server/middleware/api_key_auth.rs::parse_authorization_header_detailed`),
with two equivalent formats supported:

```text
Authorization: Basic base64(key_id:key_secret)
Authorization: ApiKey key_id:key_secret
```

- `key_id` looks like `nino_...`; `key_secret` is returned in plaintext once only, at creation/regeneration.
- The role is decided by the key itself (`admin` / `user`): Admin may access the `api-keys` management plane and
  the `config` mutation plane; User may only access the data plane (generation/parsing/its own tenant resources).
- Key expiry (`1004`), disabling (`1005`), or a signature mismatch (`1003`) map to distinct
  `business_code` values — see the [`CoreError`](#coreerror) table.

### `Accept-Language` Header

Nebula ID follows the HTTP `Accept-Language` header (see
[RFC 7231 §5.3.5](https://www.rfc-editor.org/rfc/rfc7231#section-5.3.5))
to negotiate the natural language of error-response messages. The header is parsed by `locale_middleware`
(see `src/server/middleware/locale.rs`) on every `/api/v1/*` request,
and the negotiated `Locale` is injected to downstream handlers as `Extension<Locale>`.

**Supported locale matrix:**

| Locale tag | Language | Locales file | Status |
|------------|----------|--------------|--------|
| `en` | English (default) | `locales/en/messages.ftl` | ✅ Complete |
| `zh-CN` | Simplified Chinese | `locales/zh/messages.ftl` | ✅ Complete |

**Negotiation rules:**

1. Each `<language-tag>[;q=<weight>]` item is parsed one by one. Per RFC 7231
   §5.3.1, entries with `q=0` (explicitly not accepted) or an illegal q-value format are dropped.
2. Surviving candidates sort by descending q-value, with a stable sort for equal q-values
   to preserve the original header order.
3. Each candidate tries an exact match first (`zh-CN` -> `ZhCn`),
   then a prefix match (`zh` -> `ZhCn`, `en-US` -> `En`). The wildcard `*`
   never matches a concrete locale.
4. If no candidate matches, the default locale `en` is used.
5. A missing or malformed `Accept-Language` header also falls back to `en`.
6. The header is capped at 4 KiB to defend against DoS via oversized values.

**curl examples:**

```bash
# Request Chinese responses
curl -H "Accept-Language: zh-CN" http://localhost:8080/api/v1/generate \
    -H "Content-Type: application/json" \
    -H "Authorization: ApiKey <key_id>:<key_secret>" \
    -d '{"workspace":"demo","group":"order","biz_tag":"order"}'

# Request English responses (explicit)
curl -H "Accept-Language: en-US,en;q=0.9" http://localhost:8080/api/v1/generate \
    -H "Content-Type: application/json" \
    -H "Authorization: ApiKey <key_id>:<key_secret>" \
    -d '{"workspace":"demo","group":"order","biz_tag":"order"}'

# Mixed q-value negotiation (zh-CN wins because q=0.9 > en q=0.8)
curl -H "Accept-Language: zh-CN;q=0.9, en;q=0.8" http://localhost:8080/api/v1/invalid
```

> **Security note**: `Locale` derives from user input (the `Accept-Language`
> header) and is forgeable. It must never (MUST NOT) be used for any authentication,
> authorization, or security decision — only for content negotiation (translating error messages).

The locale middleware applies only to `/api/v1/*` routes. The root-level
`/health`, `/ready`, `/metrics`, and `/api-docs/openapi.json` endpoints
do not consume `Extension<Locale>`, sparing them the `Accept-Language` parsing cost.

---

## 📤 HTTP Response Headers

### Rate-Limit Response Headers

Written by `RateLimitMiddleware` onto **every** response it handles. The header names are the canonical lowercase constants at
`src/server/rate_limit/middleware.rs:24-25`:

| Header | Written on admit (2xx/4xx/5xx) | Written on reject (429) |
|--------|--------------------------------|-------------------------|
| `x-ratelimit-limit` | The quota window size (`result.limit`) | The quota window size (`result.limit`) |
| `x-ratelimit-remaining` | The remaining quota (`result.remaining`) | The literal `0` |
| `Retry-After` | not written | Seconds, `retry_after.unwrap_or(1)` |

The 429 body is JSON: `{"code": 429, "message": "Rate limit exceeded", "retry_after": <n|null>}`.

**The CORS exposure scope matters.** `EXPOSED_HEADERS` in `src/server/config/cors.rs:25` is
`["x-request-id", "x-ratelimit-remaining"]`. So browsers can only read `x-ratelimit-remaining`; `x-ratelimit-limit` and `Retry-After` exist in
the response but are **not** exposed to scripts — read them server-side.

**Rate-limit key priority** (`middleware.rs:78-83`): the request extension's
`workspace_id` → client IP → `"anonymous"`. Only peers listed in `with_trusted_proxies`
fall the client IP back to `X-Forwarded-For`; without a trusted-proxy configuration, all
unauthenticated requests share one `"anonymous"` bucket.

**Scope**: the rate limiter is mounted on the HTTP stack only. gRPC traffic currently
bypasses the rate-limit layer, so gRPC clients
have no mechanism corresponding to HTTP 429.

---

## 🔢 gRPC Status Codes

Authentication runs once per RPC via `GrpcServer::authenticate`
(`src/server/grpc.rs:85`). The mapping follows spec R-auth-003:

| Situation | Status code | Message |
|-----------|-------------|---------|
| Missing `authorization` metadata | `Unauthenticated` | `missing authorization metadata` |
| Unsupported authorization format | `Unauthenticated` | `invalid authorization format` |
| The key exists but `enabled = false` | `PermissionDenied` | `CoreError::ApiKeyDisabled` |
| The key exists but has expired | `PermissionDenied` | `CoreError::ApiKeyExpired` |
| The secret mismatches, or the key does not exist | `Unauthenticated` | `invalid or unknown api key` |
| `auth.enabled = false` | (passes through) | logs `auth_disabled_request` at warn level |

Request-level validation and failures:

| Situation | Status code |
|-----------|-------------|
| `count == 0` on batch generation | `InvalidArgument` |
| `count > batch_generate.max_batch_size` | `InvalidArgument` |
| `parse_id` on an unparseable ID | `InvalidArgument` |
| Generator/handler errors returned by the core layer | `Internal` |

The "secret mismatch" and "key does not exist" situations deliberately share one message,
so responses never reveal which `key_id` values are valid.

---

## 🌍 HTTP Endpoints

> The full endpoint inventory (cross-checked one by one against the registrations in `src/server/router.rs`; for the OpenAPI side see
> `/api-docs/openapi.json` and the `paths(...)` in `src/server/openapi.rs`).
> Auth labels: **Public** = no credentials needed; **Authenticated** = any valid API key;
> **User** = User role only; **Admin** = Admin role only.
> All error responses use the unified envelope (see
> [HTTP API Error Response Format](#http-api-error-response-format)); protected endpoints'
> 4xx/5xx bodies are not listed one by one. The example methodology shares the OpenAPI schema's `example` source:
> workspace `demo`, group `order`, biz_tag `order`.

### Public Probes and Docs Endpoints

| Method | Path | Description |
|--------|------|-------------|
| GET | `/health` | Liveness probe (the algorithm-perspective health state) |
| GET | `/ready` | Readiness probe (database/cache dependency states) |
| GET | `/metrics` | Aggregated runtime metrics |
| GET | `/api/v1/` | API meta-info: `name` / `version` / `description` / the `endpoints` list |
| GET | `/api-docs/openapi.json` | The OpenAPI 3 spec (utoipa-generated) |
| GET | `/health/sdforge` | The sdforge plugin health check (see below) |

**`GET /health`:**

```json
{ "status": "healthy", "algorithm": "segment" }
```

**`GET /ready`:**

```json
{ "ready": true, "database": true, "cache": true, "message": "service is ready" }
```

**`GET /metrics`:**

Outputs **Prometheus text** (`text/plain; version=0.0.4`), not JSON —
the internal `MetricsResponse` snapshot is fed into the exporter's private registry at scrape time and rendered.
Main series (grep/awk as needed; labeled series aggregate by metric name):

```
nebula_id_requests_total 5
nebula_id_successful_generations_total 5
nebula_id_failed_generations_total 0
nebula_id_uptime_seconds 16
nebula_id_ids_generated_total{algorithm="snowflake"} 5
nebula_id_generate_failures_total{algorithm="segment"} 1
nebula_id_generate_latency_p50_seconds{algorithm="snowflake"} 0.000015
nebula_id_generate_latency_p99_seconds{algorithm="snowflake"} 0.000032
nebula_id_generate_latency_p999_seconds{algorithm="snowflake"} 0.000032
nebula_id_generate_latency_seconds_bucket{algorithm="snowflake",le="0.00005"} 3
nebula_id_cache_hit_rate{algorithm="segment"} 1
nebula_id_algorithm_degraded{algorithm="segment"} 0
nebula_id_circuit_breaker_open{algorithm="segment"} 0
nebula_id_circuit_breaker_half_open{algorithm="segment"} 0
```

- `*_total` and the percentiles share one source (routing-layer observation): `ids_generated_total` accumulates by the IDs actually produced
  (a batch of 10 counts as 10), while failures count per request.
- The three latency percentile tiers are gauges (seconds; `0` = no samples yet); histogram buckets are approximated incrementally from the three percentile tiers.
- Algorithms with no cache concept (Snowflake/UUID) emit no `nebula_id_cache_hit_rate` series.

> In OpenAPI, `/metrics`'s response model is still registered as `MetricsResponse` (the internal snapshot's
> JSON structure; `get_database_metrics`/`get_cache_metrics` and friends consume the same data);
> the HTTP wire format follows the Prometheus text above.

### ID Generation and Parsing (`ids`)

**`POST /api/v1/generate`** (Authenticated, User role) — generates a single ID.

```bash
curl -X POST http://localhost:8080/api/v1/generate \
    -H "Content-Type: application/json" \
    -H "Authorization: ApiKey <key_id>:<key_secret>" \
    -d '{"workspace":"demo","group":"order","biz_tag":"order","algorithm":"segment"}'
```

```json
{ "id": "123456789012345", "algorithm": "segment", "timestamp": "2026-09-17T08:00:00+00:00" }
```

**`POST /api/v1/generate/batch`** (Authenticated, User role) — batch generation.
`size` defaults to 10, capped by the `batch_generate.max_batch_size` config
(default `100`, configurable, validation cap 10000 — see [Parameter Validation](#-parameter-validation)).

```json
{
  "ids": ["123456789012346", "123456789012347", "123456789012348"],
  "size": 3,
  "algorithm": "segment",
  "timestamp": "2026-09-17T08:00:00+00:00"
}
```

**`POST /api/v1/parse`** (Authenticated) — parses ID metadata.
`algorithm` is optional (auto-detected from the ID shape).

```json
{
  "original_id": "123456789012345",
  "numeric_value": "123456789012345",
  "algorithm": "segment",
  "metadata": {
    "timestamp": 1758096000000,
    "datacenter_id": 0,
    "worker_id": 0,
    "sequence": 1,
    "algorithm": "segment",
    "biz_tag": "order"
  },
  "timestamp": "2026-09-17T08:00:00+00:00"
}
```

### Workspace Management (`workspaces`)

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| GET | `/api/v1/workspaces` | Authenticated | List: Admin sees all, User only its own workspace |
| POST | `/api/v1/workspaces` | Admin | Creates a workspace (returns the User API key plaintext, once only) |
| GET | `/api/v1/workspaces/{name}` | Authenticated | Single lookup: User only its own; another's → 403 |
| POST | `/api/v1/workspaces/{name}/regenerate-user-key` | Admin | Regenerates that workspace's User API key |

**`POST /api/v1/workspaces` request/response:**

```json
{ "name": "demo", "description": "Demo workspace", "max_groups": 10, "max_biz_tags": 100 }
```

```json
{
  "id": "6fa459ea-ee8a-3ca4-894e-db77e160355e",
  "name": "demo",
  "description": "Demo workspace",
  "status": "active",
  "max_groups": 10,
  "max_biz_tags": 100,
  "created_at": "2026-09-17T08:00:00+00:00",
  "updated_at": "2026-09-17T08:00:00+00:00",
  "user_api_key": {
    "key_id": "nino_5f8a2c1d9b3e4f6a",
    "key_secret": "sk_live_9d2f...",
    "key_prefix": "nino_"
  }
}
```

**`GET /api/v1/workspaces/{name}` response:** same as above, but `user_api_key` is `null`
(the plaintext credential never travels with a query).

### Group Management (`groups`)

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| POST | `/api/v1/groups` | User | Creates a group (`workspace` must be the caller's own workspace name) |
| GET | `/api/v1/groups?workspace={name}&page=1&page_size=20` | User | Lists the workspace's groups (own workspace only) |

**`POST /api/v1/groups` request/response:**

```json
{ "workspace": "demo", "name": "order", "description": "Order IDs", "max_biz_tags": 50 }
```

```json
{
  "id": "00914487-0f9b-4cbf-9c7b-2b0c1d3f4a5b",
  "workspace_id": "6fa459ea-ee8a-3ca4-894e-db77e160355e",
  "workspace_name": "demo",
  "name": "order",
  "description": "Order IDs",
  "max_biz_tags": 50,
  "created_at": "2026-09-17T08:00:00+00:00",
  "updated_at": "2026-09-17T08:00:00+00:00"
}
```

**`GET /api/v1/groups` response:**

```json
{ "groups": [ { "id": "...", "workspace_name": "demo", "name": "order" } ], "total": 1 }
```

### BizTag Management (`biz-tags`)

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| POST | `/api/v1/biz-tags` | User | Creates (the `workspace_id` is forced to match the caller — IDOR protection) |
| GET | `/api/v1/biz-tags?workspace_id={uuid}&page=1&page_size=20` | Authenticated | Paginated list: User forced to its own workspace; Admin must pass `workspace_id` explicitly |
| GET | `/api/v1/biz-tags/{id}` | User | Single lookup (cross-workspace → 403) |
| PUT | `/api/v1/biz-tags/{id}` | User | Updates (ownership verified first, then update) |
| DELETE | `/api/v1/biz-tags/{id}` | User | Deletes; success returns `204 No Content` |

**`POST /api/v1/biz-tags` request:**

```json
{
  "workspace_id": "6fa459ea-ee8a-3ca4-894e-db77e160355e",
  "group_id": "00914487-0f9b-4cbf-9c7b-2b0c1d3f4a5b",
  "name": "order",
  "description": "Order business tag",
  "algorithm": "segment",
  "format": "numeric",
  "prefix": "ord_",
  "base_step": 100,
  "max_step": 1000,
  "datacenter_ids": [0]
}
```

**Response (the same shape for `POST`/`GET {id}`/`PUT {id}`):**

```json
{
  "id": "1b671a64-40d5-491e-99b0-da01ff1f3341",
  "workspace_id": "6fa459ea-ee8a-3ca4-894e-db77e160355e",
  "group_id": "00914487-0f9b-4cbf-9c7b-2b0c1d3f4a5b",
  "name": "order",
  "description": "Order business tag",
  "algorithm": "segment",
  "format": "numeric",
  "prefix": "ord_",
  "base_step": 100,
  "max_step": 1000,
  "datacenter_ids": [0],
  "created_at": "2026-09-17T08:00:00+00:00",
  "updated_at": "2026-09-17T08:00:00+00:00"
}
```

**`GET /api/v1/biz-tags` response:**

```json
{ "biz_tags": [ { "id": "...", "name": "order" } ], "total": 1, "page": 1, "page_size": 20 }
```

### API Key Management (`api-keys`, Admin only)

| Method | Path | Description |
|--------|------|-------------|
| POST | `/api/v1/api-keys` | Creates an API key (`role: "admin"` is a global key; `"user"` must carry `workspace_id`). The plaintext `key_secret` is returned once only |
| GET | `/api/v1/api-keys?workspace_id={uuid}&page=1&page_size=20` | Paginated list; a missing/illegal `workspace_id` falls back to the nil UUID (the cross-full semantics) |
| DELETE | `/api/v1/api-keys/{id}` | Revokes the key |

**`POST /api/v1/api-keys` request/response:**

```json
{ "workspace_id": "6fa459ea-ee8a-3ca4-894e-db77e160355e", "name": "ci-key", "description": "CI pipeline", "role": "user", "rate_limit": 1000, "expires_at": "2027-01-01T00:00:00+00:00" }
```

```json
{
  "key": {
    "id": "1b671a64-40d5-491e-99b0-da01ff1f3341",
    "key_id": "nino_5f8a2c1d9b3e4f6a",
    "key_prefix": "nino_",
    "name": "ci-key",
    "description": "CI pipeline",
    "role": "user",
    "rate_limit": 1000,
    "enabled": true,
    "expires_at": "2027-01-01T00:00:00+00:00",
    "created_at": "2026-09-17T08:00:00+00:00"
  },
  "key_secret": "sk_live_9d2f...",
  "grace_expires_at": null
}
```

**`DELETE /api/v1/api-keys/{id}` response:**

```json
{ "success": true, "message": "API key revoked" }
```

### Config Management (`config`)

| Method | Path | Auth | Description |
|--------|------|------|-------------|
| GET | `/api/v1/config` | Authenticated | Reads the current config (sanitized: no database connection info) |
| POST | `/api/v1/config/rate-limit` | Admin | Adjusts global rate limiting (`default_rps` 1..=1000000, `burst_size` 1..=1000) |
| POST | `/api/v1/config/logging` | Admin | Adjusts the log level (`level`, length 1..=20) |
| POST | `/api/v1/config/reload` | Admin | Hot-reloads the config from disk |
| POST | `/api/v1/config/algorithm` | Admin | Binds a default algorithm to a biz_tag |

**`POST /api/v1/config/rate-limit` request/response (the same shape for `config/logging` and `config/algorithm`):**

```json
{ "default_rps": 2000, "burst_size": 200 }
```

```json
{ "success": true, "message": "rate limit updated", "config": { "app": { }, "algorithm": { }, "monitoring": { }, "logging": { }, "rate_limit": { "enabled": true, "default_rps": 2000, "burst_size": 200 } } }
```

**`POST /api/v1/config/algorithm` request/response:**

```json
{ "biz_tag": "order", "algorithm": "snowflake" }
```

```json
{ "success": true, "biz_tag": "order", "algorithm": "snowflake", "message": "algorithm updated" }
```

### `/health/sdforge`

```
GET /health/sdforge
```

**Description:**

The sdforge integration health check. The endpoint registers through the
`#[forge]` macro (see `src/server/sdforge_adapter.rs::sdforge_health`),
and is exposed via sdforge 0.5.0's inventory route merger. It returns
the Nebula ID crate's version number, so callers can confirm this route is served by
the running binary.

> The endpoint joined in v0.1.x but was only documented in v0.2.0. It
> differs from the application-level `/health` route and mainly serves to
> verify that sdforge's anti-linker-stripping mechanism works.

**Auth:** none (a public endpoint, like `/health`).

**Request:** no parameters, no body.

**Response - 200 OK:**

```json
{
  "status": "ok",
  "sdforge_version": "0.3.0"
}
```

**Fields:**

| Field | Type | Description |
|-------|------|-------------|
| `status` | string | Always `"ok"` on a 200 response |
| `sdforge_version` | string | The Nebula ID crate version (taken at compile time from `CARGO_PKG_VERSION`) |

**curl example:**

```bash
curl http://localhost:8080/health/sdforge
# {"status":"ok","sdforge_version":"0.3.0"}
```

**Notes:**

- The route registers via the `#[forge]` macro annotation:
  `#[forge(name = "sdforge_health", version = "v1", description = "sdforge integration health check", path = "/health/sdforge", method = "GET")]`.
  The `name` argument uses the underscore-separated form `sdforge_health`
  (rather than the hyphenated `sdforge-health`), because `sdforge-macros`
  0.5.0 validates that `name` must be a legal Rust identifier. The HTTP route
  path is unaffected.
- At startup, `init_sdforge()` must be called once (see `src/main.rs`),
  lest the inventory-registered routes be stripped by the linker.

---

## ✅ Parameter Validation

### Validation Strategy

Nebula ID enforces strict parameter validation to keep the system stable and secure:

**Validation principles:**
1. **Fail fast**: reject invalid requests immediately with clear error messages
2. **Security first**: defend against DoS attacks through size and rate limits
3. **User-friendly**: provide descriptive error messages for easy debugging

### Request Parameter Validation

| Parameter | Type | Validation rule | Error response |
|-----------|------|-----------------|----------------|
| `workspace` | String | 1-64 characters | `400: "workspace length must be between 1 and 64"` |
| `group` | String | 1-64 characters | `400: "group length must be between 1 and 64"` |
| `biz_tag` | String | 1-64 characters | `400: "biz_tag length must be between 1 and 64"` |
| `size` | Integer | `1..=batch_generate.max_batch_size` (a config item; default **100**, configurable, validation cap 10000) | `400: "Batch size {size} exceeds maximum allowed value of {max}"` |
| `id` | String | A valid ID format | `400: "Invalid ID format"` |

> The static validator keeps only the lower bound (`size >= 1`); the upper bound is validated at runtime in the handler layer per
> `batch_generate.max_batch_size` (`src/server/handlers/id_handlers.rs`),
> so resizing is a config change, not a code change. The same config governs the gRPC surface
> (`count > batch_generate.max_batch_size` → `InvalidArgument`).

### Security Validation

- **Batch size cap**: at most `batch_generate.max_batch_size` IDs per batch (default 100,
  configurable), guarding against DoS attacks
- **Rate limiting**: rate-limit thresholds configurable per API key
- **Input sanitization**: all string inputs get length and format validation
- **Type safety**: strong compile-time type checks

---

## 💡 Usage Examples

### Segment Algorithm Basics

```rust
use std::sync::Arc;

use nebulaid::core::algorithm::{
    CpuMonitor, DbSegmentLoader, GenerateContext, IdAlgorithm, SegmentAlgorithm,
};
use nebulaid::core::{database, Config};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::default();

    // Production assembly: a DB-backed segment loader + CPU-usage-driven dynamic step.
    let conn = database::create_connection(&config.database).await?;
    let repo = Arc::new(database::SeaOrmRepository::new(
        conn,
        config.auth.api_key_salt.clone(),
    ));
    let segment = SegmentAlgorithm::new(1).with_segment_loader(Arc::new(
        DbSegmentLoader::with_dynamic_step(
            repo,
            config.algorithm.segment.clone(),
            Arc::new(CpuMonitor::new()),
        ),
    ));

    let ctx = GenerateContext::default();
    let id = segment.generate(&ctx).await?;
    println!("Generated ID: {}", id.as_u128());

    let batch = segment.batch_generate(&ctx, 100).await?;
    println!("Generated batch of {} IDs", batch.len());

    Ok(())
}
```

### Snowflake Algorithm

```rust
use nebulaid::core::algorithm::{AlgorithmBuilder, GenerateContext, IdAlgorithm};
use nebulaid::core::types::AlgorithmType;
use nebulaid::core::Config;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // dc_id / worker_id come from Config.app, constrained by the algorithm.snowflake bit widths
    // (defaults: datacenter_id_bits=3, i.e. 0-7; worker_id_bits=8, i.e. 0-255).
    let mut config = Config::default();
    config.app.dc_id = 1;
    config.app.worker_id = 1;

    let snowflake = AlgorithmBuilder::new(AlgorithmType::Snowflake)
        .build(&config)
        .await?;

    let ctx = GenerateContext::default();
    let id = snowflake.generate(&ctx).await?;
    println!("Generated Snowflake ID: {}", id.as_u128());

    Ok(())
}
```

### UUID Generation

```rust
use nebulaid::core::algorithm::{AlgorithmBuilder, GenerateContext, IdAlgorithm};
use nebulaid::core::types::AlgorithmType;
use nebulaid::core::Config;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::default();

    // The only UUID algorithm is uuid_v8 (time-ordered RFC 9562 v8 layout).
    let uuid = AlgorithmBuilder::new(AlgorithmType::UuidV8)
        .build(&config)
        .await?;

    let ctx = GenerateContext::default();
    let id = uuid.generate(&ctx).await?;
    println!("UUID v8: {}", id);

    let batch = uuid.batch_generate(&ctx, 10).await?;
    println!("Generated {} UUIDs", batch.len());

    Ok(())
}
```

### Using the IdAlgorithm Trait

```rust
use nebulaid::core::algorithm::{AlgorithmBuilder, GenerateContext, IdAlgorithm};
use nebulaid::core::types::AlgorithmType;
use nebulaid::core::Config;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::default();
    let snowflake = AlgorithmBuilder::new(AlgorithmType::Snowflake)
        .build(&config)
        .await?;
    let ctx = GenerateContext::default();

    let id = snowflake.generate(&ctx).await?;
    println!("Generated ID via trait: {}", id.as_u128());

    let health = snowflake.health_check();
    println!("Health status: {:?}", health);

    let metrics = snowflake.metrics();
    println!("Total generated: {}", metrics.total_generated);

    Ok(())
}
```

### With Health Monitoring

Requires the `etcd` feature (`cargo build --features etcd`).

```rust
use std::sync::Arc;

use nebulaid::core::algorithm::{GenerateContext, IdAlgorithm, SegmentAlgorithm};
use nebulaid::core::coordinator::EtcdClusterHealthMonitor;
use nebulaid::core::config::EtcdConfig;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let etcd_config = EtcdConfig::default();
    let health_monitor = Arc::new(EtcdClusterHealthMonitor::new(
        etcd_config,
        "./cache.json".to_string(),
    ));

    let segment =
        SegmentAlgorithm::new(1).with_etcd_cluster_health_monitor(health_monitor.clone());

    let ctx = GenerateContext::default();
    let id = segment.generate(&ctx).await?;
    println!("Generated ID with health monitoring: {}", id.as_u128());

    Ok(())
}
```

### Batch Generation

```rust
use nebulaid::core::algorithm::{AlgorithmBuilder, GenerateContext, IdAlgorithm};
use nebulaid::core::types::AlgorithmType;
use nebulaid::core::Config;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::default();

    // Batch minting likewise goes through IdAlgorithm::batch_generate; the Segment
    // algorithm works the same after injecting DbSegmentLoader, capped by batch_generate.max_batch_size.
    let algo = AlgorithmBuilder::new(AlgorithmType::Snowflake)
        .build(&config)
        .await?;

    let ctx = GenerateContext::default();
    let batch = algo.batch_generate(&ctx, 100).await?;

    for (i, id) in batch.ids.into_iter().enumerate().take(5) {
        println!("ID {}: {}", i + 1, id.as_u128());
    }

    println!("... and {} more IDs", 95);

    Ok(())
}
```

---

<div align="center">

**[📖 User Guide](USER_GUIDE.md)** • **[🏠 Home](../../README.md)** • **[🐛 Report an issue](https://github.com/Kirky-X/NebulaId/issues)**

Built with ❤️ by Kirky.X

</div>
