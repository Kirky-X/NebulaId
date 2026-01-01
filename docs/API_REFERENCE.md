<div align="center">

# 📘 API Reference

### Complete API Documentation for Nebula ID

[🏠 Home](../README.md) • [📖 User Guide](../USER_GUIDE.md) • [🏗️ Architecture](ARCHITECTURE.md)

---

</div>

## 📋 Table of Contents

- [Overview](#overview)
- [Core API](#core-api)
  - [SegmentAlgorithm](#segmentalgorithm)
  - [SnowflakeAlgorithm](#snowflakealgorithm)
  - [UUID Generation](#uuid-generation)
  - [IdAlgorithm Trait](#idalgorithm-trait)
  - [IdGenerator Trait](#idgenerator-trait)
- [Coordinator API](#coordinator-api)
  - [EtcdClusterHealthMonitor](#etcdclusterhealthmonitor)
  - [DcFailureDetector](#dcfailuredetector)
- [Type Definitions](#type-definitions)
- [Error Handling](#error-handling)
- [Examples](#examples)

---

## Overview

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
<b>Type-Safe</b><br>
Rust's strong type system
</td>
<td width="25%" align="center">
<img src="https://img.icons8.com/fluency/96/000000/module.png" width="64"><br>
<b>Async-First</b><br>
Built for high concurrency
</td>
<td width="25%" align="center">
<img src="https://img.icons8.com/fluency/96/000000/documentation.png" width="64"><br>
<b>Distributed</b><br>
Enterprise-grade scalability
</td>
</tr>
</table>

---

## Core API

### TLS Configuration

Nebula ID supports TLS 1.2/1.3 encryption for both HTTP and gRPC servers.

#### `TlsConfig`

TLS configuration structure.

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
- `enabled`: Enable TLS globally
- `cert_path`: Path to TLS certificate file
- `key_path`: Path to TLS private key file
- `ca_path`: Optional CA certificate path for client authentication
- `http_enabled`: Enable HTTPS for HTTP server
- `grpc_enabled`: Enable TLS for gRPC server
- `min_tls_version`: Minimum TLS version (TLSv12 or TLSv13)
- `alpn_protocols`: ALPN protocol list (e.g., ["h2", "http/1.1"])

#### `TlsVersion`

Supported TLS versions.

```rust
pub enum TlsVersion {
    Tls12,
    Tls13,
}
```

#### `TlsManager`

TLS certificate and configuration manager.

```rust
pub struct TlsManager {
    config: TlsConfig,
    tls_config: Arc<ServerTlsConfig>,
}
```

**Methods:**

```rust
pub fn initialize(config: TlsConfig) -> Result<Self>
pub fn is_http_enabled(&self) -> bool
pub fn is_grpc_enabled(&self) -> bool
pub fn http_acceptor(&self) -> Result<TlsAcceptor>
pub fn grpc_tls_config(&self) -> Arc<ServerTlsConfig>
pub async fn serve_https(incoming: TlsIncoming, router: axum::Router) -> std::io::Result<()>
```

---

### Smart Segment (Dynamic Step)

Nebula ID implements dynamic step adjustment based on QPS and system load.

#### `StepCalculator`

Dynamic step calculation based on the formula:

```
next_step = base_step × (1 + α × velocity) × (1 + β × pressure)

Where:
- velocity = current_qps / step
- pressure = cpu_usage (0-1)
- α = 0.5 (velocity factor)
- β = 0.3 (pressure factor)
```

```rust
#[derive(Debug, Clone)]
pub struct StepCalculator {
    velocity_factor: f64,  // α = 0.5
    pressure_factor: f64,  // β = 0.3
}
```

**Methods:**

```rust
pub fn new(velocity_factor: f64, pressure_factor: f64) -> Self
pub fn calculate(&self, qps: u64, current_step: u64, config: &SegmentAlgorithmConfig) -> u64
pub fn get_adjustment_direction(&self, qps: u64, current_step: u64, config: &SegmentAlgorithmConfig) -> &'static str
```

**Adjustment Direction:**
- `"up"`: High QPS detected, increase step
- `"down"`: Low QPS detected, decrease step
- `"stable"`: QPS is stable, keep current step

#### `QpsWindow`

Sliding window QPS calculator.

```rust
#[derive(Debug, Clone)]
pub struct QpsWindow {
    window_secs: u64,
    timestamps: Arc<parking_lot::Mutex<Vec<std::time::Instant>>>,
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

#### `DatabaseSegmentLoader`

Segment loader with dynamic step calculation.

```rust
pub struct DatabaseSegmentLoader {
    repository: Arc<dyn SegmentRepository>,
    dc_failure_detector: Arc<DcFailureDetector>,
    local_dc_id: u8,
    etcd_cluster_health_monitor: Option<Arc<EtcdClusterHealthMonitor>>,
    step_calculator: StepCalculator,
    segment_config: SegmentAlgorithmConfig,
}
```

**Methods:**

```rust
pub fn new(
    repository: Arc<dyn SegmentRepository>,
    dc_failure_detector: Arc<DcFailureDetector>,
    local_dc_id: u8,
    config: SegmentAlgorithmConfig,
) -> Self
pub fn with_etcd_cluster_health_monitor(mut self, monitor: Arc<EtcdClusterHealthMonitor>) -> Self
pub fn get_current_step(&self) -> u64
```

---

### SegmentAlgorithm

`SegmentAlgorithm` is a high-performance distributed ID generator based on the segment algorithm. It pre-allocates ID ranges from the database for efficient batch generation.

#### `SegmentAlgorithm::new(dc_id: u8)`

Create a new segment algorithm instance with the specified datacenter ID.

```rust
pub fn new(dc_id: u8) -> Self
```

**Parameters:**
- `dc_id`: Datacenter ID (0-255)

#### `SegmentAlgorithm::new_with_loader(dc_id: u8, dc_failure_detector: Arc<DcFailureDetector>)`

Create a segment algorithm with a custom DC failure detector.

```rust
pub fn new_with_loader(
    dc_id: u8,
    dc_failure_detector: Arc<DcFailureDetector>,
) -> Self
```

#### `with_etcd_cluster_health_monitor(monitor: Arc<EtcdClusterHealthMonitor>)`

Attach an etcd cluster health monitor for distributed coordination.

```rust
pub fn with_etcd_cluster_health_monitor(
    mut self,
    monitor: Arc<EtcdClusterHealthMonitor>,
) -> Self
```

#### `with_loader(loader: Arc<dyn SegmentLoader>)`

Attach a custom segment loader for database interactions.

```rust
pub fn with_loader(mut self, loader: Arc<dyn SegmentLoader>) -> Self
```

#### `generate_id()`

Generate a single ID asynchronously.

```rust
pub async fn generate_id(&self) -> Result<Id>
```

**Returns:** `Result<Id>` - The generated ID or an error.

#### `generate_batch(size: usize)`

Generate a batch of IDs efficiently.

```rust
pub async fn generate_batch(&self, size: usize) -> Result<IdBatch>
```

**Parameters:**
- `size`: Number of IDs to generate (recommended: 10-100, maximum: 100)

**Returns:** `Result<IdBatch>` - Batch of generated IDs.

**Note:** Batch size is limited to 100 to prevent DoS attacks and ensure optimal performance.

#### `get_dc_failure_detector()`

Get the DC failure detector instance.

```rust
pub fn get_dc_failure_detector(&self) -> &Arc<DcFailureDetector>
```

---

### SnowflakeAlgorithm

`SnowflakeAlgorithm` implements the Twitter Snowflake algorithm with configurable bit allocation for datacenter, worker, and sequence.

#### `SnowflakeAlgorithm::new(datacenter_id: u8, worker_id: u8)`

Create a new Snowflake algorithm instance.

```rust
pub fn new(datacenter_id: u8, worker_id: u8) -> Self
```

**Parameters:**
- `datacenter_id`: Datacenter ID (0-31 by default)
- `worker_id`: Worker ID (0-31 by default)

#### `generate_id()`

Generate a single ID using the Snowflake algorithm.

```rust
pub fn generate_id(&self) -> Result<Id>
```

**Returns:** `Result<Id>` - The generated 64-bit ID.

**Errors:**
- `CoreError::ClockMovedBackward` - System clock moved backward
- `CoreError::SequenceOverflow` - Sequence number overflow within the same millisecond

#### `generate_id_with_timestamp(timestamp: u64, sequence_mask: u64)`

Generate an ID with a specific timestamp (internal use).

```rust
fn generate_id_with_timestamp(&self, timestamp: u64, sequence_mask: u64) -> Result<Id>
```

#### `get_datacenter_id()`

Get the configured datacenter ID.

```rust
pub fn get_datacenter_id(&self) -> u8
```

#### `get_worker_id()`

Get the configured worker ID.

```rust
pub fn get_worker_id(&self) -> u8
```

#### `get_last_timestamp()`

Get the last used timestamp.

```rust
pub fn get_last_timestamp(&self) -> u64
```

#### `get_sequence()`

Get the current sequence number.

```rust
pub fn get_sequence(&self) -> u64
```

---

### UUID Generation

#### `UuidV7Impl`

UUID v7 generator implementing timestamp-ordered UUIDs.

**Constructor:**

```rust
pub fn new() -> Self
```

**Methods:**

```rust
pub fn generate() -> Result<Uuid>
pub async fn generate(&self, ctx: &GenerateContext) -> Result<Id>
pub async fn batch_generate(&self, ctx: &GenerateContext, size: usize) -> Result<IdBatch>
```

#### `UuidV4Impl`

UUID v4 generator for random UUIDs.

**Constructor:**

```rust
pub fn new() -> Self
```

**Methods:**

```rust
pub fn generate() -> Result<Uuid>
pub async fn generate(&self, ctx: &GenerateContext) -> Result<Id>
pub async fn batch_generate(&self, ctx: &GenerateContext, size: usize) -> Result<IdBatch>
```

---

### IdAlgorithm Trait

The core trait that all ID generation algorithms must implement.

```rust
pub trait IdAlgorithm: AsAny + Send + Sync {
    async fn generate(&self, ctx: &GenerateContext) -> Result<Id>;
    async fn batch_generate(&self, ctx: &GenerateContext, size: usize) -> Result<IdBatch>;
    fn health_check(&self) -> HealthStatus;
    fn metrics(&self) -> AlgorithmMetricsSnapshot;
    fn algorithm_type(&self) -> AlgorithmType;
    async fn initialize(&mut self, config: &Config) -> Result<()>;
    async fn shutdown(&self) -> Result<()>;
}
```

#### `generate()`

Generate a single ID.

```rust
async fn generate(&self, ctx: &GenerateContext) -> Result<Id>
```

#### `batch_generate()`

Generate multiple IDs in a batch.

```rust
async fn batch_generate(&self, ctx: &GenerateContext, size: usize) -> Result<IdBatch>
```

#### `health_check()`

Check the health status of the algorithm.

```rust
fn health_check(&self) -> HealthStatus
```

**Returns:** `HealthStatus` - One of `Healthy`, `Degraded(reason)`, or `Unhealthy(reason)`

#### `metrics()`

Get algorithm performance metrics.

```rust
fn metrics(&self) -> AlgorithmMetricsSnapshot
```

#### `algorithm_type()`

Get the algorithm type.

```rust
fn algorithm_type(&self) -> AlgorithmType
```

**Returns:** `AlgorithmType` - One of `Segment`, `Snowflake`, `UuidV7`, `UuidV4`

#### `initialize()`

Initialize the algorithm with configuration.

```rust
async fn initialize(&mut self, config: &Config) -> Result<()>
```

#### `shutdown()`

Gracefully shutdown the algorithm and release resources.

```rust
async fn shutdown(&self) -> Result<()>
```

---

### IdGenerator Trait

High-level ID generator interface supporting workspace/group/tag organization.

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
    async fn get_algorithm_name(
        &self,
        workspace: &str,
        group: &str,
        biz_tag: &str,
    ) -> Result<String>;
    async fn health_check(&self) -> HealthStatus;
    async fn get_primary_algorithm(&self) -> String;
    fn get_degradation_manager(&self) -> &Arc<DegradationManager>;
    fn set_algorithm(&self, biz_tag: &str, algorithm: AlgorithmType);
}
```

---

## Coordinator API

### EtcdClusterHealthMonitor

Monitors etcd cluster health and provides fallback to local cache.

#### `EtcdClusterHealthMonitor::new(config: EtcdConfig, cache_file_path: String)`

Create a new health monitor.

```rust
pub fn new(config: EtcdConfig, cache_file_path: String) -> Self
```

#### `get_status()`

Get current cluster status.

```rust
pub fn get_status(&self) -> EtcdClusterStatus
```

**Returns:** `EtcdClusterStatus` - One of `Healthy`, `Degraded`, `Failed`

#### `set_status(status: EtcdClusterStatus)`

Manually set the cluster status.

```rust
pub fn set_status(&self, status: EtcdClusterStatus)
```

#### `record_success()`

Record a successful operation.

```rust
pub async fn record_success(&self)
```

#### `record_failure()`

Record a failed operation.

```rust
pub fn record_failure(&self)
```

#### `is_using_cache()`

Check if currently using local cache fallback.

```rust
pub fn is_using_cache(&self) -> bool
```

#### `load_local_cache()`

Load cached data from local file.

```rust
pub async fn load_local_cache(&self) -> Result<()>
```

#### `save_local_cache()`

Save current cache data to local file.

```rust
pub async fn save_local_cache(&self) -> Result<()>
```

---

### DcFailureDetector

Detects and manages datacenter health state.

#### `DcFailureDetector::new(failure_threshold: u64, recovery_timeout: Duration)`

Create a new failure detector.

```rust
pub fn new(failure_threshold: u64, recovery_timeout: Duration) -> Self
```

**Parameters:**
- `failure_threshold`: Number of consecutive failures before marking as failed
- `recovery_timeout`: Duration before attempting recovery

#### `add_dc(dc_id: u8)`

Add a datacenter to monitor.

```rust
pub fn add_dc(&self, dc_id: u8)
```

#### `get_dc_state(dc_id: u8)`

Get the health state of a specific datacenter.

```rust
pub fn get_dc_state(&self, dc_id: u8) -> Option<Arc<DcHealthState>>
```

#### `get_healthy_dcs()`

Get list of healthy datacenters.

```rust
pub fn get_healthy_dcs(&self) -> Vec<u8>
```

#### `select_best_dc(preferred_dc: u8)`

Select the best datacenter to use.

```rust
pub fn select_best_dc(&self, preferred_dc: u8) -> u8
```

#### `start_health_check(check_interval: Duration)`

Start background health check loop.

```rust
pub async fn start_health_check(&self, check_interval: Duration)
```

---

### DcHealthState

Represents the health state of a datacenter.

```rust
pub struct DcHealthState {
    pub dc_id: u8,
    pub status: AtomicU8,
    pub last_success: Arc<Mutex<Instant>>,
    pub failure_count: AtomicU64,
    pub consecutive_failures: AtomicU64,
}
```

**Methods:**

```rust
pub fn new(dc_id: u8) -> Self
pub fn get_status(&self) -> DcStatus
pub fn set_status(&self, status: DcStatus)
pub fn record_success(&self)
pub fn record_failure(&self)
pub fn should_use_dc(&self) -> bool
```

---

## Type Definitions

### `Id`

The primary ID type in Nebula ID.

```rust
pub struct Id {
    // Internal representation (u128)
}
```

**Methods:**

```rust
pub fn from_u128(value: u128) -> Self
pub fn from_uuid_v7(uuid: Uuid) -> Self
pub fn from_uuid_v4(uuid: Uuid) -> Self
pub fn to_u128(&self) -> u128
pub fn to_string(&self) -> String
pub fn to_hex(&self) -> String
```

### `IdBatch`

A batch of generated IDs.

```rust
pub struct IdBatch {
    pub ids: Vec<Id>,
    pub algorithm_type: AlgorithmType,
    pub trace_id: String,
}
```

**Methods:**

```rust
pub fn new(ids: Vec<Id>, algorithm_type: AlgorithmType, trace_id: String) -> Self
pub fn len(&self) -> usize
pub fn is_empty(&self) -> bool
pub fn into_vec(self) -> Vec<Id>
```

### `AlgorithmType`

Enumeration of supported algorithm types.

```rust
pub enum AlgorithmType {
    Segment,
    Snowflake,
    UuidV7,
    UuidV4,
}
```

### `DcStatus`

Datacenter health status.

```rust
pub enum DcStatus {
    Healthy,
    Degraded,
    Failed,
}
```

### `EtcdClusterStatus`

Etcd cluster health status.

```rust
pub enum EtcdClusterStatus {
    Healthy,
    Degraded,
    Failed,
}
```

### `HealthStatus`

Algorithm health status.

```rust
pub enum HealthStatus {
    Healthy,
    Degraded(String),
    Unhealthy(String),
}
```

### `GenerateContext`

Context for ID generation requests.

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

Performance metrics snapshot.

```rust
#[derive(Debug, Clone, Default)]
pub struct AlgorithmMetricsSnapshot {
    pub total_generated: u64,
    pub total_failed: u64,
    pub current_qps: u64,
    pub p50_latency_us: u64,
    pub p99_latency_us: u64,
    pub cache_hit_rate: f64,
}
```

### `SegmentInfo`

Database segment information.

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

---

## Error Handling

### HTTP API Error Response Format

All HTTP API endpoints return errors in a consistent format:

```json
{
  "code": 400,
  "message": "Invalid request parameter",
  "details": "Batch size must be between 1 and 100"
}
```

**Fields:**
- `code`: HTTP status code (e.g., 400, 401, 404, 500)
- `message`: Human-readable error message
- `details`: Optional detailed error information

**Common HTTP Status Codes:**

| Code | Description | Example |
|------|-------------|---------|
| 400 | Bad Request | Invalid parameters, validation errors |
| 401 | Unauthorized | Missing or invalid API key |
| 404 | Not Found | Resource not found |
| 429 | Too Many Requests | Rate limit exceeded |
| 500 | Internal Server Error | Server-side error |

### `CoreError`

Common error variants encountered during ID generation.

| Variant | Description |
|---------|-------------|
| `ClockMovedBackward` | System clock moved backward, may cause duplicate IDs |
| `SequenceOverflow` | Sequence number overflow within same millisecond |
| `DatabaseConnectionFailed` | Failed to connect to the database |
| `SegmentExhausted` | ID segment has been fully consumed |
| `EtcdConnectionFailed` | Failed to connect to etcd cluster |
| `CacheUnavailable` | Local cache is unavailable |
| `InternalError` | Internal error with description |
| `ConfigError` | Configuration error |

---

## Parameter Validation

### Validation Strategy

Nebula ID implements strict parameter validation to ensure system stability and security:

**Validation Principles:**
1. **Fail Fast**: Reject invalid requests immediately with clear error messages
2. **Security First**: Prevent DoS attacks through size and rate limiting
3. **User Friendly**: Provide descriptive error messages for debugging

### Request Parameter Validation

| Parameter | Type | Validation | Error Response |
|-----------|------|------------|----------------|
| `workspace` | String | 1-64 characters | `400: "workspace length must be between 1 and 64"` |
| `group` | String | 1-64 characters | `400: "group length must be between 1 and 64"` |
| `biz_tag` | String | 1-64 characters | `400: "biz_tag length must be between 1 and 64"` |
| `size` | Integer | 1-100 | `400: "size must be between 1 and 100"` |
| `id` | String | Valid ID format | `400: "Invalid ID format"` |

### Security Validations

- **Batch Size Limit**: Maximum 100 IDs per batch to prevent DoS attacks
- **Rate Limiting**: Configurable rate limits per API key
- **Input Sanitization**: All string inputs are validated for length and format
- **Type Safety**: Strong type checking at compile time

---

## Examples

### Basic Segment Algorithm

```rust
use nebula_id::algorithm::SegmentAlgorithm;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let segment = SegmentAlgorithm::new(1);
    
    let id = segment.generate_id().await?;
    println!("Generated ID: {}", id.to_u128());
    
    let batch = segment.generate_batch(100).await?;
    println!("Generated batch of {} IDs", batch.len());
    
    Ok(())
}
```

### Snowflake Algorithm

```rust
use nebula_core::algorithm::SnowflakeAlgorithm;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let snowflake = SnowflakeAlgorithm::new(1, 1);
    
    let id = snowflake.generate_id()?;
    println!("Generated Snowflake ID: {}", id.to_u128());
    
    println!("Datacenter ID: {}", snowflake.get_datacenter_id());
    println!("Worker ID: {}", snowflake.get_worker_id());
    
    Ok(())
}
```

### UUID Generation

```rust
use nebula_id::algorithm::uuid_v7::{UuidV7Impl, UuidV4Impl};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let v7 = UuidV7Impl::new();
    let v4 = UuidV4Impl::new();
    
    let uuid_v7 = v7.generate()?;
    let uuid_v4 = v4.generate()?;
    
    println!("UUID v7: {}", uuid_v7);
    println!("UUID v4: {}", uuid_v4);
    
    Ok(())
}
```

### Using IdAlgorithm Trait

```rust
use nebula_core::algorithm::traits::IdAlgorithm;
use nebula_core::algorithm::SnowflakeAlgorithm;
use nebula_core::algorithm::GenerateContext;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let snowflake = SnowflakeAlgorithm::new(1, 1);
    let ctx = GenerateContext::default();
    
    let id = snowflake.generate(&ctx).await?;
    println!("Generated ID via trait: {}", id.to_u128());
    
    let health = snowflake.health_check();
    println!("Health status: {:?}", health);
    
    let metrics = snowflake.metrics();
    println!("Total generated: {}", metrics.total_generated);
    
    Ok(())
}
```

### With Health Monitoring

```rust
use nebula_id::algorithm::segment::{SegmentAlgorithm, DcFailureDetector};
use nebula_id::coordinator::EtcdClusterHealthMonitor;
use nebula_id::config::EtcdConfig;
use std::sync::Arc;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dc_failure_detector = Arc::new(DcFailureDetector::new(5, Duration::from_secs(300)));
    dc_failure_detector.add_dc(1);
    
    let etcd_config = EtcdConfig::default();
    let health_monitor = Arc::new(EtcdClusterHealthMonitor::new(
        etcd_config,
        "./cache.json".to_string(),
    ));
    
    let segment = SegmentAlgorithm::new_with_loader(1, dc_failure_detector)
        .with_etcd_cluster_health_monitor(health_monitor.clone());
    
    let id = segment.generate_id().await?;
    println!("Generated ID with health monitoring: {}", id.to_u128());
    
    Ok(())
}
```

### Batch Generation

```rust
use nebula_core::algorithm::SegmentAlgorithm;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let segment = SegmentAlgorithm::new(1);

    let batch = segment.generate_batch(100).await?;

    for (i, id) in batch.into_vec().into_iter().enumerate().take(5) {
        println!("ID {}: {}", i + 1, id.to_u128());
    }

    println!("... and {} more IDs", 95);

    Ok(())
}
```

---

<div align="center">

**[📖 User Guide](../USER_GUIDE.md)** • **[🏠 Home](../README.md)** • **[🐛 Report Issue](../../issues)**

Built with ❤️ by Nebula ID Team

</div>
