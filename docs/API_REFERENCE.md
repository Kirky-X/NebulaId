# 📘 Nebula ID API 参考

> Nebula ID 完整 API 文档：HTTP / gRPC 端点、请求头、错误码与类型定义。

[🏠 首页](../README.md) • [📖 用户指南](USER_GUIDE.md) • [🏗️ 架构文档](ARCHITECTURE.md)

---

## 📋 目录

- [概述](#-概述)
  - [🎯 API 设计原则](#-api-设计原则)
- [核心 API](#-核心-api)
  - [TLS 配置](#tls-配置)
  - [智能分段（动态步长）](#智能分段动态步长)
  - [SegmentAlgorithm](#segmentalgorithm)
  - [SnowflakeAlgorithm](#snowflakealgorithm)
  - [UUID 生成](#uuid-生成)
  - [IdAlgorithm Trait](#idalgorithm-trait)
  - [IdGenerator Trait](#idgenerator-trait)
- [协调器 API](#-协调器-api)
  - [EtcdClusterHealthMonitor](#etcdclusterhealthmonitor)
  - [DcFailureDetector](#dcfailuredetector)
  - [DcHealthState](#dchealthstate)
- [类型定义](#-类型定义)
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
- [错误处理](#-错误处理)
  - [HTTP API 错误响应格式](#http-api-错误响应格式)
  - [`CoreError`](#coreerror)
  - [本地化错误响应（v0.2.0+）](#本地化错误响应v020)
- [HTTP 请求头](#-http-请求头)
  - [`X-API-Version` 请求头](#x-api-version-请求头)
  - [`Authorization` 请求头](#authorization-请求头)
  - [`Accept-Language` 请求头](#accept-language-请求头)
- [HTTP 响应头](#-http-响应头)
  - [限流响应头](#限流响应头)
- [gRPC 状态码](#-grpc-状态码)
- [HTTP 端点](#-http-端点)
  - [公开探针与文档端点](#公开探针与文档端点)
  - [ID 生成与解析（`ids`）](#id-生成与解析ids)
  - [Workspace 管理（`workspaces`）](#workspace-管理workspaces)
  - [Group 管理（`groups`）](#group-管理groups)
  - [BizTag 管理（`biz-tags`）](#biztag-管理biz-tags)
  - [API Key 管理（`api-keys`，仅 Admin）](#api-key-管理api-keys仅-admin)
  - [配置管理（`config`）](#配置管理config)
  - [`/health/sdforge`](#healthsdforge)
- [参数校验](#-参数校验)
  - [校验策略](#校验策略)
  - [请求参数校验](#请求参数校验)
  - [安全校验](#安全校验)
- [使用示例](#-使用示例)
  - [Segment 算法基础用法](#segment-算法基础用法)
  - [Snowflake 算法](#snowflake-算法)
  - [UUID 生成](#uuid-生成-1)
  - [使用 IdAlgorithm Trait](#使用-idalgorithm-trait)
  - [结合健康监测](#结合健康监测)
  - [批量生成](#批量生成)

---

## 🎯 概述

<div align="center">

### 🎯 API 设计原则

</div>

<table>
<tr>
<td width="25%" align="center">
<img src="https://img.icons8.com/fluency/96/000000/easy.png" width="64"><br>
<b>简单</b><br>
直观易用
</td>
<td width="25%" align="center">
<img src="https://img.icons8.com/fluency/96/000000/security-checked.png" width="64"><br>
<b>类型安全</b><br>
得益于 Rust 的强类型系统
</td>
<td width="25%" align="center">
<img src="https://img.icons8.com/fluency/96/000000/module.png" width="64"><br>
<b>异步优先</b><br>
为高并发场景而生
</td>
<td width="25%" align="center">
<img src="https://img.icons8.com/fluency/96/000000/documentation.png" width="64"><br>
<b>分布式</b><br>
企业级可扩展性
</td>
</tr>
</table>

---

## 🧱 核心 API

### TLS 配置

Nebula ID 为 HTTP 与 gRPC 服务器提供 TLS 1.2/1.3 加密支持。

#### `TlsConfig`

TLS 配置结构体。

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

**字段：**
- `enabled`：全局启用 TLS
- `cert_path`：TLS 证书文件路径
- `key_path`：TLS 私钥文件路径
- `ca_path`：可选的 CA 证书路径，用于客户端认证
- `http_enabled`：为 HTTP 服务器启用 HTTPS
- `grpc_enabled`：为 gRPC 服务器启用 TLS
- `min_tls_version`：最低 TLS 版本（TLSv12 或 TLSv13）
- `alpn_protocols`：ALPN 协议列表（如 ["h2", "http/1.1"]）

#### `TlsVersion`

支持的 TLS 版本。

```rust
pub enum TlsVersion {
    Tls12,
    Tls13,
}
```

#### `TlsManager`

TLS 证书与配置管理器。实现了 `Clone`（在 `axum::State` 中使用时所必需）。

```rust
#[derive(Clone)]
pub struct TlsManager {
    config: TlsConfig,
    http_acceptor: Option<TlsAcceptor>,
    grpc_tls_config: Option<Arc<ServerTlsConfig>>,
}
```

**方法：**

```rust
pub fn new(config: TlsConfig) -> Self
pub fn is_http_enabled(&self) -> bool
pub fn is_grpc_enabled(&self) -> bool
pub fn http_acceptor(&self) -> Option<&TlsAcceptor>
pub fn grpc_tls_config(&self) -> Option<&Arc<ServerTlsConfig>>
pub async fn initialize(&mut self) -> TlsResult<()>
```

**用法：** 通过 `TlsManager::new(config)` 构造，随后调用 `manager.initialize().await?` 加载证书/私钥文件并填充 `http_acceptor` / `grpc_tls_config`。当 `config.enabled == false` 时，`initialize()` 为空操作（直接返回 `Ok(())`）。在管理器以可变方式完成初始化之前，`is_http_enabled` / `is_grpc_enabled` 不会返回 `true`。

---

### 智能分段（动态步长）

Nebula ID 实现了基于 QPS 与系统负载的动态步长调整。

#### `StepCalculator`

基于以下公式进行动态步长计算：

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

**方法：**

```rust
pub fn new(velocity_factor: f64, pressure_factor: f64) -> Self
pub fn calculate(&self, qps: u64, current_step: u64, config: &SegmentAlgorithmConfig) -> u64
pub fn get_adjustment_direction(&self, qps: u64, current_step: u64, config: &SegmentAlgorithmConfig) -> &'static str
```

**调整方向：**
- `"up"`：检测到高 QPS，增大步长
- `"down"`：检测到低 QPS，减小步长
- `"stable"`：QPS 稳定，保持当前步长

#### `QpsWindow`

滑动窗口 QPS 计算器。

```rust
#[derive(Debug, Clone)]
pub struct QpsWindow {
    window_secs: u64,
    timestamps: Arc<parking_lot::Mutex<Vec<std::time::Instant>>>,
}
```

**方法：**

```rust
pub fn new(window_secs: u64) -> Self
pub fn record(&self)
pub fn record_batch(&self, count: usize)
pub fn get_qps(&self) -> u64
pub fn cleanup(&self)
pub fn window_size(&self) -> u64
```

#### `DatabaseSegmentLoader`

带动态步长计算的号段加载器。

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

**方法：**

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

`SegmentAlgorithm` 是基于号段算法的高性能分布式 ID 生成器。它预先从数据库申请 ID 区间，以高效地进行批量生成。

#### `SegmentAlgorithm::new(dc_id: u8)`

以指定的数据中心 ID 创建新的号段算法实例。

```rust
pub fn new(dc_id: u8) -> Self
```

**参数：**
- `dc_id`：数据中心 ID（0-255）

#### `SegmentAlgorithm::new_with_loader(dc_id: u8, dc_failure_detector: Arc<DcFailureDetector>)`

创建号段算法并指定自定义的 DC 故障检测器。

```rust
pub fn new_with_loader(
    dc_id: u8,
    dc_failure_detector: Arc<DcFailureDetector>,
) -> Self
```

#### `with_etcd_cluster_health_monitor(monitor: Arc<EtcdClusterHealthMonitor>)`

挂载 etcd 集群健康监视器，用于分布式协调。

```rust
pub fn with_etcd_cluster_health_monitor(
    mut self,
    monitor: Arc<EtcdClusterHealthMonitor>,
) -> Self
```

#### `with_loader(loader: Arc<dyn SegmentLoader>)`

挂载自定义号段加载器，用于与数据库交互。

```rust
pub fn with_loader(mut self, loader: Arc<dyn SegmentLoader>) -> Self
```

#### `generate_id()`

异步生成单个 ID。

```rust
pub async fn generate_id(&self) -> Result<Id>
```

**返回：** `Result<Id>` —— 生成的 ID 或错误。

#### `generate_batch(size: usize)`

高效地批量生成 ID。

```rust
pub async fn generate_batch(&self, size: usize) -> Result<IdBatch>
```

**参数：**
- `size`：要生成的 ID 数量（核心层本身不做上限截断；上限由接入面按
  `batch_generate.max_batch_size` 配置强制，默认 100，见[参数校验](#-参数校验)）

**返回：** `Result<IdBatch>` —— 生成的 ID 批次。

**注意：** 批量大小上限是**接入面策略**而非核心层硬编码：HTTP/gRPC 层按
配置项 `batch_generate.max_batch_size`（默认 `100`，可配，校验上限 10000）
拒绝超限请求，以防范 DoS 攻击并保证最佳性能。

#### `get_dc_failure_detector()`

获取 DC 故障检测器实例。

```rust
pub fn get_dc_failure_detector(&self) -> &Arc<DcFailureDetector>
```

---

### SnowflakeAlgorithm

`SnowflakeAlgorithm` 实现 Twitter Snowflake 算法，数据中心、工作者与序列号的位分配均可配置。

#### `SnowflakeAlgorithm::new(datacenter_id: u8, worker_id: u8)`

创建新的 Snowflake 算法实例。

```rust
pub fn new(datacenter_id: u8, worker_id: u8) -> Self
```

**参数：**
- `datacenter_id`：数据中心 ID（默认 0-31）
- `worker_id`：工作者 ID（默认 0-31）

#### `generate_id()`

使用 Snowflake 算法生成单个 ID。

```rust
pub fn generate_id(&self) -> Result<Id>
```

**返回：** `Result<Id>` —— 生成的 64 位 ID。

**错误：**
- `CoreError::ClockMovedBackward` —— 系统时钟回拨
- `CoreError::SequenceOverflow` —— 同一毫秒内序列号溢出

#### `generate_id_with_timestamp(timestamp: u64, sequence_mask: u64)`

以指定时间戳生成 ID（内部使用）。

```rust
fn generate_id_with_timestamp(&self, timestamp: u64, sequence_mask: u64) -> Result<Id>
```

#### `get_datacenter_id()`

获取所配置的数据中心 ID。

```rust
pub fn get_datacenter_id(&self) -> u8
```

#### `get_worker_id()`

获取所配置的工作者 ID。

```rust
pub fn get_worker_id(&self) -> u8
```

#### `get_last_timestamp()`

获取最近使用的时间戳。

```rust
pub fn get_last_timestamp(&self) -> u64
```

#### `get_sequence()`

获取当前序列号。

```rust
pub fn get_sequence(&self) -> u64
```

---

### UUID 生成

#### `UuidV8Impl`

按时间排序的 RFC 9562 **v8** UUID 生成器（`src/core/algorithm/uuid_v8.rs`）。它是本 crate 中唯一的
UUID 算法 —— 没有独立的随机 UUID（v4）实现。

可达性：`uuid_v8` 模块为 `pub(crate)`（`src/core/algorithm/mod.rs:21`），因此
`UuidV8Impl` **无法**从 crate 外部导入。请改用公开的工厂构建：

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

**构造函数（crate 内部）：**

```rust
pub fn new(dc_id: u64, worker_id: u64, clock_drift_threshold_ms: u64) -> Self
```

`dc_id` / `worker_id` 取自 `Config.app`，`clock_drift_threshold_ms` 取自
`Config.algorithm.snowflake`（参见 `UuidV8Factory::build`）。`Default` 等价于 `new(0, 0, 1000)`。

**方法（经由 `IdAlgorithm` trait）：**

```rust
pub async fn generate(&self, ctx: &GenerateContext) -> Result<Id>
pub async fn batch_generate(&self, ctx: &GenerateContext, size: usize) -> Result<IdBatch>
```

> **旧名称：** `uuid_v7` / `uuid_v4` 仍会被 `AlgorithmType::from_str`
> （`src/core/types/id.rs:197-198`）**作为输入**接受，并映射到 `AlgorithmType::UuidV8`。
> 它们既不是类型名、构造函数，也不是存储值 —— `Display` 只会输出 `uuid_v8`。

---

### IdAlgorithm Trait

所有 ID 生成算法都必须实现的核心 trait。

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

> **注意：** `async fn initialize(&mut self, config: &Config)` 方法已在 L13 中从该 trait 移除（见 `src/core/algorithm/traits.rs:44-56`）。算法初始化现在通过各算法结构体自身的方法（如 `SnowflakeAlgorithm::initialize`）完成，由 `AlgorithmBuilder::build` 在交出 `Box<dyn IdAlgorithm>` 之前调用。这样既保持了 trait 的对象安全性，也避免了在共享的 `Arc<dyn IdAlgorithm>` 引用上要求 `&mut self`。

#### `generate()`

生成单个 ID。

```rust
async fn generate(&self, ctx: &GenerateContext) -> Result<Id>
```

#### `batch_generate()`

批量生成多个 ID。

```rust
async fn batch_generate(&self, ctx: &GenerateContext, size: usize) -> Result<IdBatch>
```

#### `health_check()`

检查算法的健康状态。

```rust
fn health_check(&self) -> HealthStatus
```

**返回：** `HealthStatus` —— `Healthy`、`Degraded(reason)` 或 `Unhealthy(reason)` 之一

#### `metrics()`

获取算法性能指标。

```rust
fn metrics(&self) -> AlgorithmMetricsSnapshot
```

#### `algorithm_type()`

获取算法类型。

```rust
fn algorithm_type(&self) -> AlgorithmType
```

**返回：** `AlgorithmType` —— `Segment`、`Snowflake`、`UuidV8` 之一

#### `shutdown()`

优雅地关闭算法并释放资源。

```rust
async fn shutdown(&self) -> Result<()>
```

---

### IdGenerator Trait

高层 ID 生成器接口，支持工作区/分组/标签的组织方式。

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

> **注意：** `set_algorithm` 方法已被移除（`src/core/algorithm/traits.rs:58-99` 中不存在）。按算法路由改为通过 `generate_with_algorithm` / `batch_generate_with_algorithm` 处理，在调用时传入 `AlgorithmType` 参数。

---

## 🌐 协调器 API

### EtcdClusterHealthMonitor

监视 etcd 集群健康状况，并提供本地缓存兜底。

#### `EtcdClusterHealthMonitor::new(config: EtcdConfig, cache_file_path: String)`

创建新的健康监视器。

```rust
pub fn new(config: EtcdConfig, cache_file_path: String) -> Self
```

#### `get_status()`

获取当前集群状态。

```rust
pub fn get_status(&self) -> EtcdClusterStatus
```

**返回：** `EtcdClusterStatus` —— `Healthy`、`Degraded`、`Failed` 之一

#### `set_status(status: EtcdClusterStatus)`

手动设置集群状态。

```rust
pub fn set_status(&self, status: EtcdClusterStatus)
```

#### `record_success()`

记录一次成功操作。

```rust
pub async fn record_success(&self)
```

#### `record_failure()`

记录一次失败操作。

```rust
pub fn record_failure(&self)
```

#### `is_using_cache()`

检查当前是否正在使用本地缓存兜底。

```rust
pub fn is_using_cache(&self) -> bool
```

#### `load_local_cache()`

从本地文件加载缓存数据。

```rust
pub async fn load_local_cache(&self) -> Result<()>
```

#### `save_local_cache()`

将当前缓存数据保存到本地文件。

```rust
pub async fn save_local_cache(&self) -> Result<()>
```

---

### DcFailureDetector

检测并管理数据中心的健康状态。

#### `DcFailureDetector::new(failure_threshold: u64, recovery_timeout: Duration)`

创建新的故障检测器。

```rust
pub fn new(failure_threshold: u64, recovery_timeout: Duration) -> Self
```

**参数：**
- `failure_threshold`：判定为故障前允许的连续失败次数
- `recovery_timeout`：尝试恢复前等待的时长

#### `add_dc(dc_id: u8)`

添加要监视的数据中心。

```rust
pub fn add_dc(&self, dc_id: u8)
```

#### `get_dc_state(dc_id: u8)`

获取指定数据中心的健康状态。

```rust
pub fn get_dc_state(&self, dc_id: u8) -> Option<Arc<DcHealthState>>
```

#### `get_healthy_dcs()`

获取健康数据中心列表。

```rust
pub fn get_healthy_dcs(&self) -> Vec<u8>
```

#### `select_best_dc(preferred_dc: u8)`

选择最佳可用数据中心。

```rust
pub fn select_best_dc(&self, preferred_dc: u8) -> u8
```

#### `start_health_check(check_interval: Duration)`

启动后台健康检查循环。

```rust
pub async fn start_health_check(&self, check_interval: Duration)
```

---

### DcHealthState

表示一个数据中心的健康状态。

```rust
pub struct DcHealthState {
    pub dc_id: u8,
    pub status: AtomicU8,
    pub last_success: Arc<Mutex<Instant>>,
    pub failure_count: AtomicU64,
    pub consecutive_failures: AtomicU64,
}
```

**方法：**

```rust
pub fn new(dc_id: u8) -> Self
pub fn get_status(&self) -> DcStatus
pub fn set_status(&self, status: DcStatus)
pub fn record_success(&self)
pub fn record_failure(&self)
pub fn should_use_dc(&self) -> bool
```

---

## 📐 类型定义

### `Id`

Nebula ID 中的核心 ID 类型。

```rust
pub struct Id {
    // Internal representation (u128)
}
```

**方法：**

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

> `Id` 没有 `from_uuid_v7` / `from_uuid_v4` —— `uuid_v7` / `uuid_v4` 仅作为
> `AlgorithmType::from_str` 的输入别名存在（`src/core/types/id.rs:197-198`）。

### `IdBatch`

一批生成的 ID。

```rust
pub struct IdBatch {
    pub ids: Vec<Id>,
    pub algorithm: AlgorithmType,
    pub biz_tag: String,
    pub generated_at: chrono::DateTime<chrono::Utc>,
}
```

**方法：**

```rust
pub fn new(ids: Vec<Id>, algorithm: AlgorithmType, biz_tag: String) -> Self
pub fn from_u64s(values: &[u64]) -> Self
pub fn len(&self) -> usize
pub fn is_empty(&self) -> bool
```

### `AlgorithmType`

受支持算法类型的枚举。

```rust
pub enum AlgorithmType {
    #[default]
    Segment,
    Snowflake,
    UuidV8,
}
```

`AlgorithmType::from_str` 还接受旧拼写 `uuid_v7` / `uuid_v4`
（以及 `uuidv7` / `uuid7` / `uuidv4` / `uuid4`）作为 `UuidV8` 的**输入别名**
（`src/core/types/id.rs:195-201`）；`Display` 只会输出 `segment` / `snowflake` / `uuid_v8`。

### `DcStatus`

数据中心健康状态。

```rust
pub enum DcStatus {
    Healthy,
    Degraded,
    Failed,
}
```

### `EtcdClusterStatus`

etcd 集群健康状态。

```rust
pub enum EtcdClusterStatus {
    Healthy,
    Degraded,
    Failed,
}
```

### `HealthStatus`

算法健康状态。

```rust
pub enum HealthStatus {
    Healthy,
    Degraded(String),
    Unhealthy(String),
}
```

### `GenerateContext`

ID 生成请求的上下文。

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

性能指标快照。

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

> `p50/p99/p999_latency_us` 与 `clock_backwards` 由
> `AlgorithmRouter::metrics()` 依据路由层观测环形缓冲区填充；算法自身返回的是 `0`，
> 含义是「尚未被路由器观测到」，而非「延迟为 0」。
> `cache_hit_rate = None` 表示该算法没有缓存概念（Snowflake / UUID v8）——
> 使用 `Option` 可避免把「无缓存」当作 0% 命中率计入均值
> （`src/core/algorithm/traits.rs:144-166`）。

### `SegmentInfo`

数据库号段信息。

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

创建或轮换 API 密钥的返回结果 —— 明文凭证仅返回一次。

```rust
pub struct ApiKeyWithSecret {
    pub key: ApiKeyResponse,
    pub key_secret: String,
    pub grace_expires_at: Option<DateTime>,
}
```

> `grace_expires_at`（`src/core/database/api_key_entity.rs:189`）仅在轮换发生时
> `auth.key_rotation_grace_period_seconds > 0` 才不为 `None`：它是一个绝对的
> UTC 截止时间，在此之前**上一代**密钥的 secret 仍可用于认证
> （`prev_secret_hash` + `rotate_expires_at`，在 `validate_api_key` 内部惰性比较）。
> `None` 表示「无生效的宽限窗口」—— 默认值（`0`）以及所有新建密钥都是这种情况。
> 在 `pub` 结构体上新增该字段，对使用结构体字面量构造
> `ApiKeyWithSecret` 的外部代码而言是**破坏性变更**。
>
> 线上传输模型是 `ApiKeyWithSecretResponse`（`src/server/models.rs:749`），其中同一
> 字段为 RFC 3339 字符串或 `null`。如实说明这一边界：`ApiHandlers::rotate_api_key`
> （`src/server/handlers/api_key_handlers.rs:274`）**没有对应的 HTTP 路由** —— 与密钥相关的管理
> 路由是 `POST /api-keys`、`GET /api-keys`、`DELETE /api-keys/{id}` 与
> `POST /workspaces/{name}/regenerate-user-key`，而后两者走的是*创建* /
> *删除后重建*路径，因此 `grace_expires_at` 在那里始终为 `null`。并不存在
> 可供调用的 `POST /api-keys/{id}/rotate`。

---

## 🚨 错误处理

### HTTP API 错误响应格式

所有 HTTP API 端点（含 API 版本协商失败）都以**统一错误信封**返回错误
（`src/server/models.rs::ErrorResponse`）：

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

**字段：**
- `code`：HTTP 状态码（如 400、401、404、500）
- `business_code`：稳定的机器可读业务码（四位数字符串，如 `"3002"`）。
  客户端应**以本字段做语义分支**，不要解析 `message` 文案（文案随
  `Accept-Language` 本地化）。映射关系见下方 [`CoreError`](#coreerror) 表。
- `message`：人类可读的错误信息（按协商出的 `Locale` 翻译）
- `details`：可选的详细错误信息
- `request_id`：请求追踪 ID（响应装配处生成的 UUID v4，与审计日志关联）
- `timestamp`：错误发生时间（毫秒级 Unix 时间戳）

**常见 HTTP 状态码：**

| 状态码 | 说明 | 示例 |
|--------|------|------|
| 400 | 错误请求 | 参数无效、校验失败、不支持的 API 版本 |
| 401 | 未授权 | 缺失或无效的 API 密钥 |
| 403 | 禁止 | 权限不足（需 Admin）/ 跨 workspace 访问 |
| 404 | 未找到 | 资源不存在 |
| 429 | 请求过多 | 超出限流阈值 |
| 500 | 服务器内部错误 | 服务端错误 |
| 503 | 服务不可用 | 下游依赖超时 |

> **唯一例外**：全局限流中间件（`src/server/rate_limit/middleware.rs`）拒绝请求时
> 返回的 429 使用独立响应体 `{"code": 429, "message": "Rate limit exceeded", "retry_after": <n|null>}`
> （不含 `business_code` 等信封字段，`Retry-After` 语义见[限流响应头](#限流响应头)）。
> 核心层 `CoreError::RateLimitExceeded` 走 handler 装配路径时仍为统一信封
> （429 + `business_code "4001"`）。

### `CoreError`

`src/core/types/error.rs` 中 `CoreError` 的**全部**变体（T034 对照修正，
以代码为准），及其经 `core_error_classification`
（`src/server/handlers/helpers.rs`）单表映射出的 HTTP 状态码与
`business_code`：

| 变体 | 说明 | HTTP | `business_code` |
|------|------|------|-----------------|
| `InvalidIdFormat(String)` | ID 格式非法 | 400 | `3001` |
| `InvalidIdString(String)` | ID 字符串无法解析 | 400 | `3001` |
| `InvalidAlgorithmType(String)` | 算法类型非法 | 400 | `3001` |
| `InvalidInput(String)` | 输入非法（如批量大小为 0/超上限） | 400 | `3001` |
| `ParseError(String)` | 解析错误（如数字解析失败） | 400 | `3001` |
| `AuthenticationError(String)` | 认证失败 | 401 | `1001` |
| `InvalidApiKeySignature` | API Key 签名/格式非法 | 401 | `1003` |
| `ApiKeyExpired` | API Key 已过期 | 401 | `1004` |
| `ApiKeyDisabled` | API Key 已禁用 | 401 | `1005` |
| `WorkspaceDisabled(String)` | workspace 已停用 | 403 | `1002` |
| `NotFound(String)` | 资源未找到（默认按 workspace 404 处理） | 404 | `2001` |
| `BizTagNotFound(String)` | biz-tag 未找到 | 404 | `2003` |
| `RateLimitExceeded` | 触发限流 | 429 | `4001` |
| `TimeoutError` | 下游依赖超时 | 503 | `5004` |
| `DatabaseError(String)` | 数据库操作失败 | 500 | `5002` |
| `CacheError(String)` | 缓存服务失败 | 500 | `5003` |
| `InternalError(String)` | 附带描述的内部错误 | 500 | `5001` |
| `ConfigurationError(String)` | 配置错误 | 500 | `5001` |
| `EtcdError(String)` | etcd 协调层错误 | 500 | `5001` |
| `IoError(String)` | I/O 错误 | 500 | `5001` |
| `ClockMovedBackward { last_timestamp }` | 系统时钟回拨，可能产生重复 ID | 500 | `5001` |
| `SequenceOverflow { timestamp }` | 同一毫秒内序列号溢出 | 500 | `5001` |
| `SegmentExhausted { max_id }` | 号段已完全耗尽 | 500 | `5001` |
| `Unknown` | 未知错误 | 500 | `5001` |

> 算法类变体（`ClockMovedBackward` / `SequenceOverflow` / `SegmentExhausted`）
> 与其余未单列的 5xx 类变体统一收敛为 500 + `5001`（`InternalError`），
> 不会把内部细节泄露给客户端。

### 本地化错误响应（v0.2.0+）

自 v0.2.0 起，每个 HTTP 错误响应的 `message` 与 `details` 字段
都会依据请求协商出的 `Locale` 进行翻译（见
[Accept-Language 请求头](#accept-language-请求头)）。HTTP 状态码、
`business_code` 与 JSON 结构在各语言环境下保持不变；只有人类可读的文本不同。

**示例 - 400 错误请求（英语，默认）：**

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

**示例 - 400 错误请求（简体中文，`Accept-Language: zh-CN`）：**

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

**示例 - 500 服务器内部错误（英语）：**

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

**示例 - 500 服务器内部错误（简体中文）：**

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

> 5xx 响应经 `core_error_to_response` 消毒：`message` 收敛为通用文案，
> 原始错误细节只进服务端日志，避免向客户端泄露内部实现。

翻译键位于 `locales/en.yml` 与 `locales/zh-CN.yml` 中，归属于
`error.*` 命名空间。缺失的键会先回退到默认语言（`en`），
再回退到键本身（绝不会是空字符串）。

---

## 📨 HTTP 请求头

### `X-API-Version` 请求头

`X-API-Version` 请求头（`src/server/api_version.rs`）用于 API 版本协商，
由 `api_version_middleware` 在**所有 `/api/v1/*` 路由**上处理
（`/health`、`/ready`、`/metrics`、`/api-docs/openapi.json` 不经过该中间件）。

**协商机制（以代码为准）：**

1. 读取请求头 `X-API-Version`；**缺失或值非法时回退默认版本 `v1`**
   （fail-open，不会 4xx）。
2. 值大小写不敏感，且接受数字形式：`v1` / `1` → `ApiVersion::V1`，
   `v2` / `2` → `ApiVersion::V2`。
3. 仅 `v1` 受支持（`is_supported()`）：`v2` 虽可解析但尚未开放，
   请求会被拒绝。
4. 不支持的版本返回 `400`，响应体为统一错误信封
   （`business_code = "3001"`，`message = "Unsupported API version"`，
   `details` 列出请求版本与受支持版本）。
5. 请求放行时，响应会回写 `X-API-Version: v1` 响应头。

**示例：**

```bash
# 显式请求 v1（等价于不传）
curl -H "X-API-Version: v1" http://localhost:8080/api/v1/generate ...

# 不支持的版本 → 400 + business_code=3001
curl -i -H "X-API-Version: v2" http://localhost:8080/api/v1/config
# x-api-version 响应头缺失，响应体:
# {"code":400,"business_code":"3001","message":"Unsupported API version",
#  "details":"Requested version: 'v2'. Supported versions: v1",...}
```

### `Authorization` 请求头

除公开探针（`/health`、`/ready`、`/metrics`、`/api-docs/openapi.json`、
`GET /api/v1`）外，所有端点都要求 API Key 认证。凭证通过 `Authorization`
请求头传递（`src/server/middleware/api_key_auth.rs::parse_authorization_header_detailed`），
支持两种等价格式：

```text
Authorization: Basic base64(key_id:key_secret)
Authorization: ApiKey key_id:key_secret
```

- `key_id` 形如 `nino_...`；`key_secret` 仅在创建/重生成时明文返回一次。
- 角色由密钥本身决定（`admin` / `user`）：Admin 可访问 `api-keys` 管理面与
  `config` 变更面；User 仅可访问数据面（生成/解析/自身租户资源）。
- 密钥过期（`1004`）、禁用（`1005`）或签名不匹配（`1003`）分别映射到不同的
  `business_code`，见 [`CoreError`](#coreerror) 表。

### `Accept-Language` 请求头

Nebula ID 遵循 HTTP `Accept-Language` 请求头（参见
[RFC 7231 §5.3.5](https://www.rfc-editor.org/rfc/rfc7231#section-5.3.5)）
来协商错误响应消息的自然语言。该请求头由 `locale_middleware`
（见 `src/server/middleware/locale.rs`）在每个 `/api/v1/*` 请求上解析，
协商出的 `Locale` 会以 `Extension<Locale>` 的形式注入给下游 handler。

**支持的语言环境矩阵：**

| 语言标签 | 语言 | 语言文件 | 状态 |
|----------|------|----------|------|
| `en` | 英语（默认） | `locales/en.yml` | ✅ 完整 |
| `zh-CN` | 简体中文 | `locales/zh-CN.yml` | ✅ 完整 |

**协商规则：**

1. 逐条解析 `<language-tag>[;q=<weight>]`。依据 RFC 7231
   §5.3.1，`q=0`（明确不接受）或 q 值格式非法的条目会被丢弃。
2. 幸存的候选按 q 值降序排序，q 值相同时保持稳定排序，
   以维持请求头中的原始顺序。
3. 对每个候选先尝试精确匹配（`zh-CN` -> `ZhCn`），
   再尝试前缀匹配（`zh` -> `ZhCn`、`en-US` -> `En`）。通配符 `*`
   永远不会匹配到具体语言环境。
4. 若无候选匹配，则使用默认语言环境 `en`。
5. `Accept-Language` 请求头缺失或格式非法时，同样回退到 `en`。
6. 请求头上限为 4 KiB，以防利用超长值发起 DoS。

**curl 示例：**

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

> **安全提示**：`Locale` 源自用户输入（`Accept-Language`
> 请求头），可被伪造。它绝不能（MUST NOT）用于任何认证、
> 授权或安全决策，仅用于内容协商（翻译错误消息）。

locale 中间件仅作用于 `/api/v1/*` 路由。根路径下的
`/health`、`/ready`、`/metrics` 与 `/api-docs/openapi.json` 端点
不消费 `Extension<Locale>`，因此省去了 `Accept-Language` 的解析开销。

---

## 📤 HTTP 响应头

### 限流响应头

由 `RateLimitMiddleware` 写入其处理的**每一个**响应。头名称为
`src/server/rate_limit/middleware.rs:35-36` 中的规范小写常量：

| 响应头 | 放行时写入（2xx/4xx/5xx） | 拒绝时写入（429） |
|--------|--------------------------|-------------------|
| `x-ratelimit-limit` | 配额窗口大小（`result.limit`） | 配额窗口大小（`result.limit`） |
| `x-ratelimit-remaining` | 剩余配额（`result.remaining`） | 字面量 `0` |
| `Retry-After` | 不写入 | 秒数，`retry_after.unwrap_or(1)` |

429 响应体为 JSON：`{"code": 429, "message": "Rate limit exceeded", "retry_after": <n|null>}`。

**CORS 暴露范围很重要。** `src/server/config/cors.rs:36` 中的
`EXPOSED_HEADERS` 为 `["x-request-id", "x-ratelimit-remaining"]`。因此浏览器
只能读取 `x-ratelimit-remaining`；`x-ratelimit-limit` 与 `Retry-After` 虽然存在于
响应中，但**不会**暴露给脚本 —— 请在服务端读取。

**限流键优先级**（`middleware.rs:78-83`）：请求扩展中的
`workspace_id` → 客户端 IP → `"anonymous"`。只有 `with_trusted_proxies` 中列出的对端
才会把客户端 IP 回退到 `X-Forwarded-For`；未配置可信代理时，所有
未认证请求共享同一个 `"anonymous"` 桶。

**作用范围**：限流器仅挂载在 HTTP 栈上。gRPC 流量目前
不经过限流层（见 `src/server/grpc.rs` 的 T023 注记），因此 gRPC 客户端
没有对应 HTTP 429 的机制。

---

## 🔢 gRPC 状态码

认证由 `GrpcServer::authenticate`
（`src/server/grpc.rs:85`）对每个 RPC 执行一次。依据规范 R-auth-003 的映射如下：

| 情形 | 状态码 | 消息 |
|------|--------|------|
| 缺少 `authorization` 元数据 | `Unauthenticated` | `missing authorization metadata` |
| 不支持的授权格式 | `Unauthenticated` | `invalid authorization format` |
| 密钥存在但 `enabled = false` | `PermissionDenied` | `CoreError::ApiKeyDisabled` |
| 密钥存在但已过有效期 | `PermissionDenied` | `CoreError::ApiKeyExpired` |
| secret 不匹配，或密钥不存在 | `Unauthenticated` | `invalid or unknown api key` |
| `auth.enabled = false` | （直接放行） | 以 warn 级别记录 `auth_disabled_request` |

请求级校验与失败：

| 情形 | 状态码 |
|------|--------|
| 批量生成时 `count == 0` | `InvalidArgument` |
| `count > batch_generate.max_batch_size` | `InvalidArgument` |
| 对无法解析的 ID 执行 `parse_id` | `InvalidArgument` |
| 核心层返回的生成器/handler 错误 | `Internal` |

「secret 不匹配」与「密钥不存在」两种情形刻意共用同一条消息，
以免响应泄露哪些 `key_id` 值是有效的。

---

## 🌍 HTTP 端点

> T034 端点全录（与 `src/server/router.rs` 注册处一一对照，OpenAPI 侧见
> `/api-docs/openapi.json` 与 `src/server/openapi.rs` 的 `paths(...)`）。
> 认证标注：**公开** = 无需凭证；**认证** = 任意有效 API Key；
> **User** = 仅 User 角色；**Admin** = 仅 Admin 角色。
> 所有错误响应均为统一信封（见
> [HTTP API 错误响应格式](#http-api-错误响应格式)），受保护端点的
> 4xx/5xx 不再逐一列出。示例口径与 OpenAPI schema `example` 同源：
> workspace `demo`、group `order`、biz_tag `order`。

### 公开探针与文档端点

| 方法 | 路径 | 说明 |
|------|------|------|
| GET | `/health` | 存活探针（算法视角健康状态） |
| GET | `/ready` | 就绪探针（数据库/缓存依赖状态） |
| GET | `/metrics` | 聚合运行指标 |
| GET | `/api/v1/` | API 元信息：`name` / `version` / `description` / `endpoints` 端点清单 |
| GET | `/api-docs/openapi.json` | OpenAPI 3 规范（utoipa 生成） |
| GET | `/health/sdforge` | sdforge 插件健康检查（见下文） |

**`GET /health`：**

```json
{ "status": "healthy", "algorithm": "segment" }
```

**`GET /ready`：**

```json
{ "ready": true, "database": true, "cache": true, "message": "service is ready" }
```

**`GET /metrics`：**

```json
{
  "total_requests": 1024,
  "successful_generations": 1000,
  "failed_generations": 24,
  "total_ids_generated": 50000,
  "avg_latency_ms": 3,
  "uptime_seconds": 86400,
  "database": { "status": "healthy", "connection_pool": { "active_connections": 2, "idle_connections": 8, "max_connections": 10 }, "last_error": null },
  "cache": { "status": "healthy", "hit_rate": 0.92, "has_cache": true, "memory_usage_mb": 64, "key_count": 1200 },
  "algorithms": [
    { "algorithm": "segment", "status": "healthy", "total_generated": 50000, "total_failed": 24, "p50_latency_ms": 1.0, "p99_latency_ms": 5.0, "p999_latency_ms": 9.0, "clock_backwards": 0, "cache_hit_rate": 0.92 }
  ],
  "degradation_metrics": [
    { "algorithm": "segment", "total_requests": 1024, "total_successes": 1000, "total_failures": 24, "success_rate": 0.976, "consecutive_failures": 0, "consecutive_successes": 500, "circuit_breaker_state": "closed", "is_degraded": false }
  ]
}
```

### ID 生成与解析（`ids`）

**`POST /api/v1/generate`**（认证，User 角色）—— 生成单个 ID。

```bash
curl -X POST http://localhost:8080/api/v1/generate \
    -H "Content-Type: application/json" \
    -H "Authorization: ApiKey <key_id>:<key_secret>" \
    -d '{"workspace":"demo","group":"order","biz_tag":"order","algorithm":"segment"}'
```

```json
{ "id": "123456789012345", "algorithm": "segment", "timestamp": "2026-09-17T08:00:00+00:00" }
```

**`POST /api/v1/generate/batch`**（认证，User 角色）—— 批量生成。
`size` 缺省 10，上限为配置项 `batch_generate.max_batch_size`
（默认 `100`，可配，配置校验上限 10000，见[参数校验](#-参数校验)）。

```json
{
  "ids": ["123456789012346", "123456789012347", "123456789012348"],
  "size": 3,
  "algorithm": "segment",
  "timestamp": "2026-09-17T08:00:00+00:00"
}
```

**`POST /api/v1/parse`**（认证）—— 解析 ID 元数据。
`algorithm` 可空（自动按 ID 形态识别）。

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

### Workspace 管理（`workspaces`）

| 方法 | 路径 | 认证 | 说明 |
|------|------|------|------|
| GET | `/api/v1/workspaces` | 认证 | 列表：Admin 全量，User 仅自身 workspace |
| POST | `/api/v1/workspaces` | Admin | 创建 workspace（返回 User API Key 明文，仅此一次） |
| GET | `/api/v1/workspaces/{name}` | 认证 | 单查：User 仅可查自身，他人 → 403 |
| POST | `/api/v1/workspaces/{name}/regenerate-user-key` | Admin | 重生成该 workspace 的 User API Key |

**`POST /api/v1/workspaces` 请求/响应：**

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

**`GET /api/v1/workspaces/{name}` 响应：** 同上，但 `user_api_key` 为 `null`
（明文凭证不随查询下发）。

### Group 管理（`groups`）

| 方法 | 路径 | 认证 | 说明 |
|------|------|------|------|
| POST | `/api/v1/groups` | User | 创建 group（`workspace` 须为自身 workspace 名） |
| GET | `/api/v1/groups?workspace={name}&page=1&page_size=20` | User | 列出 workspace 下的 group（仅自身 workspace） |

**`POST /api/v1/groups` 请求/响应：**

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

**`GET /api/v1/groups` 响应：**

```json
{ "groups": [ { "id": "...", "workspace_name": "demo", "name": "order" } ], "total": 1 }
```

### BizTag 管理（`biz-tags`）

| 方法 | 路径 | 认证 | 说明 |
|------|------|------|------|
| POST | `/api/v1/biz-tags` | User | 创建（`workspace_id` 强制与调用者一致，IDOR 防护） |
| GET | `/api/v1/biz-tags?workspace_id={uuid}&page=1&page_size=20` | 认证 | 分页列表：User 强制自身 workspace；Admin 必须显式传 `workspace_id` |
| GET | `/api/v1/biz-tags/{id}` | User | 单查（跨 workspace → 403） |
| PUT | `/api/v1/biz-tags/{id}` | User | 更新（先校验归属再更新） |
| DELETE | `/api/v1/biz-tags/{id}` | User | 删除，成功返回 `204 No Content` |

**`POST /api/v1/biz-tags` 请求：**

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

**响应（`POST`/`GET {id}`/`PUT {id}` 同形）：**

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

**`GET /api/v1/biz-tags` 响应：**

```json
{ "biz_tags": [ { "id": "...", "name": "order" } ], "total": 1, "page": 1, "page_size": 20 }
```

### API Key 管理（`api-keys`，仅 Admin）

| 方法 | 路径 | 说明 |
|------|------|------|
| POST | `/api/v1/api-keys` | 创建 API Key（`role: "admin"` 为全局密钥；`"user"` 必须带 `workspace_id`）。明文 `key_secret` 仅此一次返回 |
| GET | `/api/v1/api-keys?workspace_id={uuid}&page=1&page_size=20` | 分页列表；`workspace_id` 缺省/非法时回退 nil UUID（跨全量语义） |
| DELETE | `/api/v1/api-keys/{id}` | 吊销密钥 |

**`POST /api/v1/api-keys` 请求/响应：**

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

**`DELETE /api/v1/api-keys/{id}` 响应：**

```json
{ "success": true, "message": "API key revoked" }
```

### 配置管理（`config`）

| 方法 | 路径 | 认证 | 说明 |
|------|------|------|------|
| GET | `/api/v1/config` | 认证 | 读取当前配置（脱敏：不含数据库连接信息） |
| POST | `/api/v1/config/rate-limit` | Admin | 调整全局限流（`default_rps` 1..=1000000，`burst_size` 1..=1000） |
| POST | `/api/v1/config/logging` | Admin | 调整日志级别（`level`，长度 1..=20） |
| POST | `/api/v1/config/reload` | Admin | 从磁盘热重载配置 |
| POST | `/api/v1/config/algorithm` | Admin | 为 biz_tag 绑定默认算法 |

**`POST /api/v1/config/rate-limit` 请求/响应（`config/logging`、`config/algorithm` 同形）：**

```json
{ "default_rps": 2000, "burst_size": 200 }
```

```json
{ "success": true, "message": "rate limit updated", "config": { "app": { }, "algorithm": { }, "monitoring": { }, "logging": { }, "rate_limit": { "enabled": true, "default_rps": 2000, "burst_size": 200 } } }
```

**`POST /api/v1/config/algorithm` 请求/响应：**

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

**说明：**

sdforge 集成健康检查。该端点通过
`#[forge]` 宏注册（见 `src/server/sdforge_adapter.rs::sdforge_health`），
并经 `sdforge` 0.5.0-rc.4 的 inventory 路由合并器对外暴露。它返回
Nebula ID crate 的版本号，调用方可据此确认该路由由
正在运行的二进制提供服务。

> 该端点在 v0.1.x 中加入，但直到 v0.2.0 才写入文档。它
> 不同于应用层的 `/health` 路由，主要用于
> 验证 `sdforge` 插件的防链接器剥离机制生效。

**认证：** 无（公开端点，与 `/health` 相同）。

**请求：** 无参数、无请求体。

**响应 - 200 OK：**

```json
{
  "status": "ok",
  "sdforge_version": "0.2.0"
}
```

**字段：**

| 字段 | 类型 | 说明 |
|------|------|------|
| `status` | string | 200 响应时恒为 `"ok"` |
| `sdforge_version` | string | Nebula ID crate 版本（编译期取自 `CARGO_PKG_VERSION`） |

**curl 示例：**

```bash
curl http://localhost:8080/health/sdforge
# {"status":"ok","sdforge_version":"0.2.0"}
```

**注意：**

- 该路由通过 `#[forge]` 宏注解注册：
  `#[forge(name = "sdforge_health", version = "v1", description = "sdforge integration health check", path = "/health/sdforge", method = "GET")]`。
  `name` 参数采用下划线分隔形式 `sdforge_health`
  （而非连字符形式 `sdforge-health`），因为 `sdforge-macros`
  0.5.0-rc.4 会校验 `name` 必须是合法的 Rust 标识符。HTTP 路由
  路径不受影响。
- 启动时必须调用一次 `init_sdforge()`（见 `src/main.rs`），
  以防 inventory 注册的路由被链接器剥离。

---

## ✅ 参数校验

### 校验策略

Nebula ID 实施严格的参数校验，以确保系统稳定与安全：

**校验原则：**
1. **快速失败**：以清晰的错误信息立即拒绝无效请求
2. **安全优先**：通过大小与速率限制防范 DoS 攻击
3. **对用户友好**：提供描述性的错误信息，便于调试

### 请求参数校验

| 参数 | 类型 | 校验规则 | 错误响应 |
|------|------|----------|----------|
| `workspace` | String | 1-64 个字符 | `400: "workspace length must be between 1 and 64"` |
| `group` | String | 1-64 个字符 | `400: "group length must be between 1 and 64"` |
| `biz_tag` | String | 1-64 个字符 | `400: "biz_tag length must be between 1 and 64"` |
| `size` | Integer | `1..=batch_generate.max_batch_size`（配置项，默认 **100**，可配，配置校验上限 10000） | `400: "Batch size {size} exceeds maximum allowed value of {max}"` |
| `id` | String | 合法的 ID 格式 | `400: "Invalid ID format"` |

> 静态 validator 只保留下界（`size >= 1`）；上限在 handler 层按
> `batch_generate.max_batch_size` 运行时校验（`src/server/handlers/id_handlers.rs`），
> 调整配置即可扩缩，无需改代码。gRPC 面同一配置生效
> （`count > batch_generate.max_batch_size` → `InvalidArgument`）。

### 安全校验

- **批量大小上限**：每批最多 `batch_generate.max_batch_size` 个 ID（默认 100，
  可配），以防 DoS 攻击
- **速率限制**：可按 API 密钥配置限流阈值
- **输入净化**：所有字符串输入都会做长度与格式校验
- **类型安全**：编译期强类型检查

---

## 💡 使用示例

### Segment 算法基础用法

```rust
use nebulaid::core::algorithm::SegmentAlgorithm;

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

### Snowflake 算法

```rust
use nebulaid::core::algorithm::SnowflakeAlgorithm;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let snowflake = SnowflakeAlgorithm::new(1, 1);
    
    let id = snowflake.generate_id()?;
    println!("Generated Snowflake ID: {}", id.to_u128());
    
    println!("Datacenter ID: {}", snowflake.get_datacenter_id());
    println!("Worker ID: {}", snowflake.get_worker_id());
    
    Ok(())
}
```

### UUID 生成

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

### 使用 IdAlgorithm Trait

```rust
use nebulaid::core::algorithm::traits::IdAlgorithm;
use nebulaid::core::algorithm::SnowflakeAlgorithm;
use nebulaid::core::algorithm::GenerateContext;

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

### 结合健康监测

```rust
use nebulaid::core::algorithm::segment::{SegmentAlgorithm, DcFailureDetector};
use nebulaid::core::coordinator::EtcdClusterHealthMonitor;
use nebulaid::core::config::EtcdConfig;
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

### 批量生成

```rust
use nebulaid::core::algorithm::SegmentAlgorithm;

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

**[📖 用户指南](../USER_GUIDE.md)** • **[🏠 首页](../README.md)** • **[🐛 报告问题](../../issues)**

由 Nebula ID 团队用 ❤️ 构建

</div>
