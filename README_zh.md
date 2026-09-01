<div align="center">

# 🚀 Nebula ID

[![GitHub release](https://img.shields.io/github/v/release/Kirky-X/NebulaId)](https://github.com/Kirky-X/NebulaId/releases) [![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-green)](./LICENSE) [![CI](https://img.shields.io/github/actions/workflow/status/Kirky-X/NebulaId/ci.yml?branch=main)](https://github.com/Kirky-X/NebulaId/actions/workflows/ci.yml) [![Security](https://img.shields.io/github/actions/workflow/status/Kirky-X/NebulaId/codeql.yml?branch=main&label=security)](https://github.com/Kirky-X/NebulaId/actions/workflows/codeql.yml)

<p align="center"><b>中文</b> | <a href="./README.md">English</a></p>

<p align="center">
  <strong>企业级高性能应用分布式ID生成系统</strong>
</p>

<p align="center">
  <a href="#-features">功能特性</a> •
  <a href="#-quick-start">快速开始</a> •
  <a href="#-documentation">文档</a> •
  <a href="#-examples">示例</a> •
  <a href="#-contributing">贡献指南</a>
</p>

</div>

---

## 📋 目录

<details open>
<summary>点击展开</summary>

- [✨ 功能特性](#-功能特性)
- [🎯 使用场景](#-使用场景)
- [🚀 快速开始](#-快速开始)
  - [安装](#安装)
  - [基本用法](#基本用法)
- [📚 文档](#-文档)
- [🎨 示例](#-示例)
- [🏗️ 架构设计](#️-架构设计)
- [⚙️ 配置](#️-配置)
- [🧪 测试](#-测试)
- [📊 性能](#-性能)
- [🔒 安全](#-安全)
- [🌐 国际化](#-国际化)
- [🛠️ scripts/run.sh 用法](#️-scriptsrunsh-用法)
- [🗺️ 路线图](#️-路线图)
- [🤝 贡献指南](#-贡献指南)
- [📄 许可证](#-许可证)
- [🙏 致谢](#-致谢)

</details>

---

## ✨ 功能特性

<table>
<tr>
<td width="50%">

### 🎯 核心功能

- ✅ **多种ID算法** - Segment、Snowflake、UUID v8
- ✅ **分布式协调** - 基于Etcd的leader选举和协调
- ✅ **高可用性** - 数据中心健康监控和自动故障转移
- ✅ **类型安全设计** - 完整的Rust类型安全与async/await模式

</td>
<td width="50%">

### ⚡ 高级功能

- 🚀 **高性能** - 支持并发访问，每秒可生成百万级ID
- 🔐 **API安全** - API密钥认证和限流
- 📊 **监控** - 内置指标、健康检查和告警
- 🌐 **多协议支持** - HTTP/HTTPS REST API和gRPC/gRPCS支持

</td>
</tr>
<tr>
<td width="50%">

### 🌟 v0.2.0 新增特性

- 🌍 **ICU 国际化** - `rust-i18n 3.1` + `Accept-Language` 协商（RFC 7231 §5.3.5），支持 `en` + `zh-CN`，1989 处 `t!()` 调用
- 🔧 **Trait 抽象** - `EtcdClientOps` 与 `ConfigManagementService` trait 支持 mock 注入，业务逻辑可测试
- 🛡️ **SAST 加固** - `tiangang` SAST + `diting` 三维度审查，0 CRITICAL / 0 HIGH
- 📦 **统一脚本入口** - `scripts/run.sh` 统一调度 `deploy` / `lint` / `redis-test` / `api-test` / `install-hooks` / `help`

</td>
<td width="50%">

### 🎯 v0.2.0 质量门禁

- ✅ **0 警告**：`cargo build --package nebulaid --features etcd` 与 `cargo clippy --features etcd -D warnings` 均无告警
- ✅ **4000+ 测试**：行覆盖率 89.91%（CI 门禁 `--fail-under-lines 95`）
- ✅ **0 死代码**：`cargo udeps` + `cargo rustc -W dead_code` 双重审计
- ✅ **mod.rs 接口隔离**：强制执行规则 25（`mod.rs` 只暴露 trait + pub 类型）

</td>
</tr>
</table>

<div align="center">

### 🎨 功能亮点

</div>

```mermaid
graph LR
    A[客户端应用] --> B[Nebula ID服务]
    B --> C[算法路由]
    C --> D[Segment算法]
    C --> E[Snowflake算法]
    C --> F[UUID v8算法]
    B --> G[分布式协调]
    G --> H[Etcd]
    B --> I[监控]
    I --> J[健康检查]
    I --> K[指标]
```

---

## 🎯 使用场景

<details>
<summary><b>💼 分布式系统</b></summary>

<br>

```rust
use nebulaid::core::types::AlgorithmType;
use nebulaid::core::Config;
use nebulaid::sdk::NebulaIdKitBuilder; // feature `sdk`

#[tokio::main]
async fn main() -> nebulaid::core::Result<()> {
    // Segment 需要从数据库领取号段，必须先用
    // `NebulaIdKitBuilder::with_repository(..)` 注入仓储；纯算法不需要。
    let mut config = Config::default();
    config.algorithm.default = "snowflake".to_string();

    let kit = NebulaIdKitBuilder::new(config).build().await?;
    let generator = kit.id_generator()?;

    // 使用默认算法（`config.algorithm.default`）
    let id = generator.generate("prod", "core", "order").await?;

    // 或按次指定算法
    let uuid = generator
        .generate_with_algorithm(AlgorithmType::UuidV8, "prod", "core", "trace")
        .await?;

    println!("snowflake={id} uuid_v8={uuid}");
    kit.shutdown().await;
    Ok(())
}
```

适用于需要高可用性、有序唯一标识符的大规模分布式系统。

</details>

<details>
<summary><b>🔧 微服务</b></summary>

<br>

```rust
use nebulaid::core::types::Id;
use uuid::Uuid;

// 任意 Uuid 都用同一个构造函数包装为 Nebula `Id`（仅有 `from_uuid_v8`）
let id = Id::from_uuid_v8(Uuid::now_v7());
let id_string = id.to_string(); // 输出标准 36 字符 UUID 字符串

// 随机标识符使用同一构造函数
let id_v4 = Id::from_uuid_v8(Uuid::new_v4());

// 可无损转回 Uuid
let uuid = id_v4.to_uuid_v8();
```

适用于需要不同排序保证的唯一标识符的微服务。

</details>

<details>
<summary><b>🌐 高性能应用</b></summary>

<br>

```rust
use nebulaid::core::Config;
use nebulaid::sdk::NebulaIdKitBuilder;

#[tokio::main]
async fn main() -> nebulaid::core::Result<()> {
    let mut config = Config::default();
    config.algorithm.default = "snowflake".to_string();
    let kit = NebulaIdKitBuilder::new(config).build().await?;
    let generator = kit.id_generator()?;

    // 一次调用拿一批。双缓冲是 Segment 的内部机制 —— 对外只需
    // 一次申请 N 个 ID（`IdAlgorithm::batch_generate`）。
    let batch = generator.batch_generate("prod", "core", "order", 1000).await?;
    println!("{} ids via {:?}", batch.len(), batch.algorithm);

    kit.shutdown().await;
    Ok(())
}
```

适用于需要每秒生成数百万ID且低延迟的高性能应用。

</details>

---

## 🚀 快速开始

### 安装

<table>
<tr>
<td width="50%">

#### 🦀 从源码构建

```bash
# 克隆仓库
git clone https://github.com/Kirky-X/NebulaId.git
cd NebulaId

# 构建默认特性（postgresql + http + grpc + garrison-auth）
cargo build --release

# 运行服务
./target/release/nebula-id
```

</td>
<td width="50%">

#### 📦 功能标志

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

**按特性构建:**
```bash
# 默认（PostgreSQL + HTTP + gRPC + garrison 认证）
cargo build --release

# 最大可构建特性集
cargo build --release --features etcd

# 嵌入式 SDK（src/sdk + examples/{embedded,sdk_server}）
cargo build --release --features sdk

# 说明：sqlite 当前不可构建 —— default 特性集恒含 dbnexus/postgres，
# 而 dbnexus 禁止 sqlite 与 postgres 混用（compile_error）；
# 同一约束也使「全特性」构建无效。
```

</td>
</tr>
</table>

### 基本用法

<div align="center">

#### 🎬 5分钟快速开始

</div>

<table>
<tr>
<td width="50%">

**步骤1：创建配置**

```bash
# 以仓库示例为起点 —— 它是能被完整解析的最小配置。
cp config/config.toml my-config.toml

# 至少修改：[database].password（经 ${NEBULA_DATABASE_PASSWORD}）、
# [database].url / host / port，以及 [algorithm].default
```

</td>
<td width="50%">

**步骤2：启动服务**

```bash
# 二进制默认读取 config/config.toml，可用 --config 指定路径。
./target/release/nebula-id --config my-config.toml &

# 探活
curl -s http://localhost:8080/health
curl -s http://localhost:8080/metrics
```

若要把本 crate 作为库嵌入而不是起服务，请看下文 Complete Example
中的 `examples/embedded.rs` 片段。

</td>
</tr>
</table>

<details>
<summary><b>📖 完整示例</b></summary>

<br>

```rust
// 与 examples/embedded.rs 一致 —— 运行方式：
//   cargo run --package nebulaid --example embedded --features sdk
use nebulaid::core::Config;
use nebulaid::sdk::NebulaIdKitBuilder;

#[tokio::main]
async fn main() -> nebulaid::core::Result<()> {
    // 仅纯算法：`segment` 还需
    // NebulaIdKitBuilder::with_repository(..)，
    // 因为它要从数据库领取号段。
    let mut config = Config::default();
    config.algorithm.default = "snowflake".to_string();

    let kit = NebulaIdKitBuilder::new(config).build().await?;
    let generator = kit.id_generator()?;

    for _ in 0..5 {
        let id = generator.generate("embedded", "demo", "order").await?;
        println!("生成的ID: {id}");
    }

    kit.shutdown().await;
    Ok(())
}
```

</details>

---

## 📚 文档

<div align="center">

<table>
<tr>
<td align="center" width="25%">
<a href="https://docs.rs/nebula-id">
<img src="https://img.icons8.com/fluency/96/000000/api.png" width="64" height="64"><br>
<b>API参考</b>
</a><br>
完整API文档
</td>
<td align="center" width="25%">
<a href="examples/">
<img src="https://img.icons8.com/fluency/96/000000/code.png" width="64" height="64"><br>
<b>示例</b>
</a><br>
代码示例
</td>
<td align="center" width="25%">
<a href="https://github.com/nebula-id/nebula-id">
<img src="https://img.icons8.com/fluency/96/000000/github.png" width="64" height="64"><br>
<b>GitHub</b>
</a><br>
源代码
</td>
<td align="center" width="25%">
<a href="https://crates.io/crates/nebula-id">
<img src="https://img.icons8.com/fluency/96/000000/package.png" width="64" height="64"><br>
<b>Crates.io</b>
</a><br>
包注册表
</td>
</tr>
</table>

</div>

### 📖 额外资源

- 🎓 **算法选择** - 选择合适的ID生成算法
- 🔧 **配置指南** - 完整配置参考
- ❓ **常见问题** - 关于分布式ID生成的常见问题

---

## 🎨 示例

<div align="center">

### 💡 实际示例

</div>

<table>
<tr>
<td width="50%">

#### 📝 示例1：Segment算法

```rust
use nebulaid::core::algorithm::{AlgorithmBuilder, GenerateContext, IdAlgorithm};
use nebulaid::core::types::AlgorithmType;
use nebulaid::core::Config;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // `dc_id` 取自 [app]；具体的 SegmentAlgorithm 类型是 crate 内部的，
    // 只能通过公开的 AlgorithmBuilder 构建。
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

    println!("生成的ID: {}", id);
    Ok(())
}
```

<details>
<summary>查看输出</summary>

```
生成的ID: 17731488000000
```

未注入仓储时内置加载器把每个号段起点设为 `unix_seconds × 10000`；
注入仓储后号段由数据库分配。

</details>

</td>
<td width="50%">

#### 🔥 示例2：Snowflake算法

```rust
use nebulaid::core::algorithm::{AlgorithmBuilder, GenerateContext, IdAlgorithm};
use nebulaid::core::types::AlgorithmType;
use nebulaid::core::Config;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // dc/worker 取自 [app]；位布局取自 [algorithm.snowflake]
    let mut config = Config::default();
    config.app.dc_id = 1;
    config.app.worker_id = 1;

    let snowflake = AlgorithmBuilder::new(AlgorithmType::Snowflake)
        .build(&config)
        .await?;

    let id = snowflake.generate(&GenerateContext::default()).await?;
    println!("生成的Snowflake ID: {}", id);

    let s = &config.algorithm.snowflake;
    println!(
        "位布局: timestamp({}) | dc({}) | worker({}) | seq({})",
        s.timestamp_bits(),
        s.datacenter_id_bits,
        s.worker_id_bits,
        s.sequence_bits
    );
    Ok(())
}
```

<details>
<summary>查看输出</summary>

```
生成的Snowflake ID: <64 位数值，随时钟递增>
位布局: timestamp(43) | dc(3) | worker(8) | seq(10)
```

</details>

</td>
</tr>
</table>

<div align="center">

**[📂 查看所有示例 →](examples/)**

</div>

---

## 🏗️ 架构设计

<div align="center">

### 系统概览

</div>

```mermaid
graph TB
    A[客户端应用] --> B[API网关]
    B --> C[HTTP REST API]
    B --> D[gRPC服务]
    C --> E[算法路由]
    D --> E
    E --> F[Segment算法]
    E --> G[Snowflake算法]
    E --> H[UUID v8]
    F --> I[(数据库)]
    G --> J[分布式协调]
    J --> K[Etcd]
    H --> L[(缓存)]
    E --> M[监控]
    M --> N[健康检查]
    M --> O[指标]
    
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
<summary><b>📐 组件详情</b></summary>

<br>

| 组件 | 描述 | 状态 |
|-----------|-------------|--------|
| **算法路由** | 将ID生成请求路由到合适的算法 | ✅ 稳定 |
| **Segment算法** | 基于数据库的Segment ID生成，支持双缓冲 | ✅ 稳定 |
| **Snowflake算法** | Twitter Snowflake变体，用于分布式唯一ID | ✅ 稳定 |
| **UUID生成器** | UUID v8（RFC 9562 §5.8 自定义结构化）实现 | ✅ 稳定 |
| **分布式协调** | 基于Etcd的leader选举和协调 | ✅ 稳定 |
| **监控** | 健康检查、指标收集和告警 | ✅ 稳定 |
| **API网关** | HTTP/HTTPS和gRPC/gRPCS端点管理 | ✅ 稳定 |

</details>

---

## ⚙️ 配置

<div align="center">

### 🎛️ 配置选项

</div>

**能被完整解析的最小配置（`config.toml`）**

`Config` 对 `app`、`database`、`etcd`、`auth`、`algorithm`、`monitoring`、`logging`、
`rate_limit`、`tls`、`batch_generate` 都**没有**标注 `#[serde(default)]`
（`src/core/config/app_config.rs:37-64`）。缺任一必填字段会让**整份**文件解析失败，
**未知键**同样会被拒绝 —— 17 个配置结构体全部带 `deny_unknown_fields`。解析失败会让进程
以退出码 1 终止（`resolve_startup_config`，`src/main.rs:542`），不再退回
`Config::default()`；只有在既没给 `--config`、`config/config.toml` 也确实不存在时，才使用
内置默认配置，并额外输出一条 `warn`。所以请照抄，不要裁剪。
只有 `[redis]` 与 `[hot_reload]` 可以整体省略。

```toml
[app]
name = "nebula-id"
host = "0.0.0.0"
http_port = 8080                 # 不存在 `app.port`
grpc_port = 9091
dc_id = 0                        # 0..=31
worker_id = 0
# shutdown_timeout_seconds = 30  # 可选

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
# url = "postgresql://idgen:pw@localhost:5432/idgen"   # 可选的整串写法

[etcd]
endpoints = ["http://localhost:2379"]
connect_timeout_ms = 5000
watch_timeout_ms = 5000

[auth]
enabled = true
cache_ttl_seconds = 300
# api_keys = []                  # 可选；启动时只创建第一条；需 key_id/key_secret/workspace（UUID 或 "global"）/role/rate_limit/name
# api_key_salt = "..."           # 可选（回退到 $NEBULA_API_KEY_SALT）
# key_rotation_grace_period_seconds = 0      # 可选；0（默认）= 不启用宽限期

[algorithm]
default = "segment"              # segment | snowflake | uuid_v8（没有 `type` 键）

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
burst_size = 100                 # validate()：必须 <= 10 × default_rps

[tls]
enabled = false
cert_path = ""
key_path = ""
http_enabled = false
grpc_enabled = false
# ca_path = ""                   # 可选
# min_tls_version = "tls13"      # 可选：tls12 | tls13
# alpn_protocols = ["h2", "http/1.1"]   # 可选

[batch_generate]
max_batch_size = 100             # validate()：1..=10000

# 完全可选的段：
# [redis]
# url = "redis://localhost:6379"
# pool_size = 16                 # 可选
# key_prefix = "nebula:id:"      # 可选
# ttl_seconds = 600              # 可选
# [hot_reload]
# auto_watch_enabled = false
```

> ⚠️ **代码事实核对**：服务端启动时 `Config::merge()` 会用
> 「环境变量配置」的 `algorithm.segment` / `algorithm.snowflake` /
> `algorithm.uuid_v8` 覆盖文件值，而它们恒为默认值
> （`src/core/config/app_config.rs:393-395`，由 `src/main.rs:559` 调用）。
> 合并后只有 `algorithm.default` 保留；在该合并逻辑修正前，三个子表请在代码里调。

**环境变量**

并不存在 `NEBULA_APP_*` / `NEBULA_AUTH_API_KEY` 这一族变量。真实机制只有两种：

```bash
# 1. 启动时由 `Config::load_from_env()` 覆盖到文件配置之上
#    （只有与默认值不同的项才生效）：
export APP_HOST="0.0.0.0"
export APP_HTTP_PORT="8080"
export APP_GRPC_PORT="9091"
export DC_ID="0"
export WORKER_ID="0"
export DATABASE_URL="postgresql://idgen:pass@localhost:5432/idgen"
export ETCD_ENDPOINTS="http://localhost:2379,http://localhost:22379"
export RUST_LOG="info"

# 2. 在文件里以 ${VAR} 引用，解析前先展开
#    （`Config::expand_env_vars`）：
export NEBULA_DATABASE_PASSWORD="..."   # [database].password / url
export NEBULA_API_KEY_SALT="..."        # [auth].api_key_salt 回退值
```

<details>
<summary><b>🔧 所有配置选项</b></summary>

<br>

| 选项 | 类型 | `Config::default()` | 文件内必填 | 说明 |
|--------|------|---------------------|--------------|------|
| `app.name` | String | `"nebula-id"` | ✅ | 应用名称 |
| `app.host` | String | `"0.0.0.0"` | ✅ | 服务器绑定地址 |
| `app.http_port` | u16 | `8080` | ✅ | HTTP 端口，必须 > 0 |
| `app.grpc_port` | u16 | `9091` | ✅ | gRPC 端口，必须 > 0 |
| `app.dc_id` | u8 | `0` | ✅ | 数据中心 ID，必须 ≤ 31 |
| `app.worker_id` | u8 | `0` | ✅ | 工作节点 ID |
| `app.shutdown_timeout_seconds` | u64 | `30` | ➖ | 优雅停机超时，必须 > 0 |
| `database.engine` | String | `"postgresql"` | ✅ | `postgresql` / `postgres` / `mysql` / `sqlite` |
| `database.host` / `port` / `username` / `password` / `database` | — | `localhost` / `5432` / `idgen` / `$NEBULA_DATABASE_PASSWORD` / `idgen` | ✅ | 逐项连接参数 |
| `database.url` | String | `""` | ➖ | 上面各项的整串替代写法 |
| `database.max_connections` | u32 | `100` | ✅ | 连接池大小，必须 > 0 |
| `database.min_connections` | u32 | `10` | ✅ | 必须 ≤ `max_connections` |
| `database.acquire_timeout_seconds` | u64 | `30` | ✅ | 必须 > 0 |
| `database.idle_timeout_seconds` | u64 | `300` | ✅ | 空闲连接超时 |
| `redis` | 段 | — | ➖ | 整段可省略 |
| `redis.url` | String | `$REDIS_URL` 或 `redis://localhost:6379` | ✅（写了 `[redis]` 就必填） | Redis 连接 URL |
| `redis.pool_size` / `key_prefix` / `ttl_seconds` | u32 / String / u64 | `16` / `"nebula:id:"` / `600` | ➖ | 缓存调优 |
| `etcd.endpoints` | Vec&lt;String&gt; | `["etcd:2379"]` | ✅ | `[]` 时退回 `LocalDistributedLock` |
| `etcd.connect_timeout_ms` / `watch_timeout_ms` | u64 | `5000` / `5000` | ✅ | etcd 超时 |
| `auth.enabled` | bool | `true` | ✅ | API Key 中间件总开关 |
| `auth.cache_ttl_seconds` | u64 | `300` | ✅ | 认证缓存 TTL |
| `auth.api_keys` | 数组 | `[]` | ➖ | 条目字段：`key_id`、`key_secret`、`workspace`、`role`、`rate_limit`、`name`（全部必填）。启动时**只创建第一条**；`workspace` 必须是 UUID 字符串或 `global`；`role` 仅精确取 `admin` 时才建管理员 |
| `auth.api_key_salt` | String | `$NEBULA_API_KEY_SALT` 或 `""` | ➖ | 密钥哈希盐值 |
| `auth.key_rotation_grace_period_seconds` | u64 | `0`（关闭宽限期） | ➖ | 设为 `> 0` 才在轮换后保留上一代凭证该秒数；超过 30 天会被钳制到 30 天并告警；需库中存在宽限期两列，启动期迁移会自动补齐（仅数据库账号无 DDL 权限时需手工执行，见 `docs/CONFIG_MIGRATION_GUIDE.md`） |
| `algorithm.default` | String | `"segment"` | ✅ | `segment` / `snowflake` / `uuid_v8` |
| `algorithm.segment.base_step` / `min_step` / `max_step` / `switch_threshold` | u64 / u64 / u64 / f64 | `1000` / `500` / `100000` / `0.1` | ✅ | 动态步长（注意上文的 `merge` 说明） |
| `algorithm.snowflake.datacenter_id_bits` / `worker_id_bits` / `sequence_bits` / `clock_drift_threshold_ms` | u8 / u8 / u8 / u64 | `3` / `8` / `10` / `1000` | ✅ | 位布局；余量为时间戳位 |
| `algorithm.uuid_v8.enabled` | bool | `true` | ✅ | UUID v8 开关 |
| `monitoring.metrics_enabled` / `metrics_path` / `tracing_enabled` / `otlp_endpoint` | bool / String / bool / String | `true` / `"/metrics"` / `false` / `""` | ✅ | Prometheus + OTLP |
| `logging.level` / `format` / `include_location` | String / String / bool | `"info"` / `"json"` / `true` | ✅ | `level`: trace…error，`format`: json/pretty |
| `rate_limit.enabled` | bool | `true` | ✅ | 限流总开关 |
| `rate_limit.default_rps` | u32 | `10000` | ✅ | 每秒请求数，启用时必须 > 0 |
| `rate_limit.burst_size` | u32 | `100` | ✅ | 启用时必须 ≤ 10 × `default_rps` |
| `hot_reload` | 段 | `auto_watch_enabled = false` | ➖ | 整段可省略 |
| `tls.enabled` / `cert_path` / `key_path` / `http_enabled` / `grpc_enabled` | bool / String / String / bool / bool | `false` / `""` / `""` / `false` / `false` | ✅ | HTTP 与 gRPC 的 TLS |
| `tls.ca_path` | String? | `null` | ➖ | 可选 CA |
| `tls.min_tls_version` | String | `"tls13"` | ➖ | `tls12` / `tls13` |
| `tls.alpn_protocols` | Vec&lt;String&gt; | `["h2", "http/1.1"]` | ➖ | ALPN 列表 |
| `batch_generate.max_batch_size` | u32 | `100` | ✅ | 必须在 1..=10000 |

默认值即 `Config::default()` 的取值；「文件内必填」表示该字段
是否带 serde 默认值。17 个配置结构体全部带 `#[serde(deny_unknown_fields)]`，
未知键的严重后果与缺必填键完全一样：两者都让**整份**文件解析失败并终止启动，
段名拼错不再可能被静默丢弃。

### 校验规则

解析成功后立刻执行 `Config::validate()`（`src/core/config/app_config.rs:173-293`）；违反
即 `Config::load_from_file` 返回 `ConfigError::InvalidValue`，服务端启动会以退出码 1 终止，
并在消息里同时给出文件路径与违规项（例如
`failed to load configuration from 'config/config.toml': Invalid configuration value:
HTTP port must be between 1 and 65535`）：

| 约束 | 来源 |
|------|------|
| `http_port > 0`、`grpc_port > 0`、`shutdown_timeout_seconds > 0` | `Config::validate` |
| `dc_id <= 31` | `Config::validate` |
| `max_connections > 0`、`min_connections <= max_connections`、`acquire_timeout_seconds > 0` | `Config::validate` |
| `rate_limit.enabled` ⇒ `default_rps > 0`、`burst_size > 0`、`burst_size <= 10 × default_rps` | `Config::validate` |
| `algorithm.default ∈ {segment, snowflake, uuid_v8}` | `Config::validate` |
| `segment.min_step <= segment.max_step` 且 `min_step <= base_step <= max_step` | `Config::validate` |
| `0.0 <= segment.switch_threshold <= 1.0` | `Config::validate` |
| `snowflake.datacenter_id_bits + worker_id_bits + sequence_bits < 64`（默认 ⇒ 43 位时间戳） | `Config::validate` |
| `snowflake.clock_drift_threshold_ms > 0` | `Config::validate` |
| `1 <= batch_generate.max_batch_size <= 10000` | `Config::validate` |

> 完整参考：[`config/config.toml`](config/config.toml) 与
> [CONFIG_MIGRATION_GUIDE.md](docs/CONFIG_MIGRATION_GUIDE.md)。

</details>

---

## 🧪 测试

<div align="center">

### 🎯 测试覆盖率

</div>

```bash
# 运行所有测试
cargo test --features etcd

# 运行覆盖率测试
cargo tarpaulin --out Html

# 运行特定测试
cargo test test_name

# 运行集成测试
cargo test --test integration

# 运行预提交检查（格式化、静态分析、构建、测试、安全、文档、覆盖率）
./scripts/run.sh pre-commit
```

<details>
<summary><b>📊 测试统计</b></summary>

<br>

| 类别 | 测试数量 | 覆盖率 |
|----------|-------|----------|
| 单元测试 | 4000+ | 89.91% |
| 集成测试 | 42 | 89.91% |
| **总计** | **4000+** | **89.91%** |

> 自 v0.2.0 起，CI 覆盖率门禁已调高至 `--fail-under-lines 95`（见 `.github/workflows/ci.yml`）。v0.2.0 发布时实际行覆盖率为 89.91%；门禁值是下限，非当前值。

</details>

---

## 📊 性能

<div align="center">

### ⚡ 基准测试结果

</div>

<table>
<tr>
<td width="50%">

**ID生成吞吐量**

```
Segment: 100,000+ IDs/秒
Snowflake: 1,000,000+ IDs/秒
UUID v8: 500,000+ IDs/秒
```

</td>
<td width="50%">

**延迟 (P99)**

```
Segment: ~0.5ms
Snowflake: ~0.1ms
UUID v8: ~0.05ms
```

</td>
</tr>
</table>

<details>
<summary><b>📈 详细基准测试</b></summary>

<br>

```bash
# 运行本仓库实际提供的基准测试
cargo bench --bench i18n
```

`Cargo.toml` 中唯一声明的 Criterion 基准是 `i18n`（`benches/i18n.rs`）；
本仓库没有 ID 生成基准框架，因此这里不发布任何 `*_next_id` 数值。

</details>

---

## 🔒 安全

<div align="center">

### 🛡️ 安全特性

</div>

<table>
<tr>
<td align="center" width="33%">
<img src="https://img.icons8.com/fluency/96/000000/lock.png" width="64" height="64"><br>
<b>API认证</b><br>
基于API密钥的认证，具有时序攻击防护
</td>
<td align="center" width="33%">
<img src="https://img.icons8.com/fluency/96/000000/security-checked.png" width="64" height="64"><br>
<b>限流</b><br>
可配置限流防止滥用（最大批量大小：100）
</td>
<td align="center" width="33%">
<img src="https://img.icons8.com/fluency/96/000000/privacy.png" width="64" height="64"><br>
<b>审计日志</b><br>
跟踪所有ID生成操作，具有IP欺骗防护
</td>
</tr>
</table>

<details>
<summary><b>🔐 安全详情</b></summary>

<br>

### 安全措施

- ✅ **API密钥认证** - 使用API密钥认证保护API访问，采用常量时间比较防止时序攻击
- ✅ **限流** - 可配置限流防止滥用和DoS攻击（最大批量大小：100）
- ✅ **审计日志** - 完整的操作跟踪，满足合规和监控需求，具有IP欺骗防护
- ✅ **TLS支持** - HTTPS和gRPCS实现加密通信（TLS 1.2/1.3）
- ✅ **CORS限制** - 严格的跨域资源共享策略
- ✅ **安全响应头** - X-Content-Type-Options、X-Frame-Options、CSP、HSTS、X-XSS-Protection、Referrer-Policy
- ✅ **IP欺骗防护** - 对X-Forwarded-For头进行可信代理验证

### 功能标志

```toml
# Cargo 包名是 `nebulaid`；审计日志与 TLS 是运行时配置（[auth] / [tls]），
# 不是 Cargo feature。
[dependencies]
nebulaid = { version = "0.2", features = ["sdk"] }      # 嵌入式客户端 facade
# nebulaid = { version = "0.2", features = ["etcd"] }   # 分布式协调
# 可用 feature：postgresql（默认）、etcd、garrison-auth（默认）、
# sdk、http/grpc/openapi（sdforge 镜像，http+grpc 为默认）、integration-tests。
```

</details>

---

## 🌐 国际化

<div align="center">

### 🌍 ICU i18n 支持（v0.2.0 新增）

</div>

Nebula ID 自 v0.2.0 起内置 ICU 国际化支持，基于 [`rust-i18n`](https://crates.io/crates/rust-i18n) `3.1` 实现，覆盖错误消息与日志的运行时翻译。

**支持的语言（locale）矩阵：**

| Locale 标签 | 语言 | locales 文件 | 状态 |
|-------------|------|--------------|------|
| `en` | English（默认） | `locales/en.yml` | ✅ 完整 |
| `zh-CN` | 简体中文 | `locales/zh-CN.yml` | ✅ 完整 |

**协商机制：**

1. 客户端通过 HTTP `Accept-Language` 头声明偏好语言（遵循 [RFC 7231 §5.3.5](https://www.rfc-editor.org/rfc/rfc7231#section-5.3.5)），例如 `Accept-Language: zh-CN,zh;q=0.9,en;q=0.8`。
2. `locale_middleware`（`src/server/middleware/locale.rs`）解析头并按 q-value 降序排序，匹配首个受支持的 locale（精确匹配优先，次之 prefix 匹配如 `zh` → `zh-CN`）。
3. 匹配失败或头缺失时回退到默认 locale `en`。
4. 业务 handler 通过 `Extension<Locale>` 读取协商结果，用 `translate_with_locale_args` 翻译错误响应消息。

**curl 示例：**

```bash
# 中文错误响应
curl -H "Accept-Language: zh-CN" http://localhost:8080/api/v1/invalid
# {
#   "code": 404,
#   "message": "未找到路径",
#   "details": "..."
# }

# 英文错误响应（默认）
curl http://localhost:8080/api/v1/invalid
# {
#   "code": 404,
#   "message": "Path not found",
#   "details": "..."
# }
```

> **安全提示**：`Locale` 派生自用户输入（`Accept-Language` 头），可被伪造，**不得**用于任何认证、授权或安全决策，仅用于内容协商。

更多细节见 [API 参考 — Accept-Language](docs/API_REFERENCE.md#accept-language-header) 与 [架构文档 — i18n 模块](docs/ARCHITECTURE.md#8-i18n-模块位置)。

---

## 🛠️ scripts/run.sh 用法

<div align="center">

### 📦 统一脚本入口（v0.2.0 新增）

</div>

自 v0.2.0 起，所有开发/部署脚本合并为统一入口 `scripts/run.sh`，替代了 v0.1.x 的多个分散脚本（`deploy`、`pre-commit-check`、`redis_test`、`test_api`、`install-pre-commit-hooks` 等），旧脚本已重命名为 `_*_impl.sh` 内部实现，不再直接调用。

**子命令一览：**

| 子命令 | 别名 | 作用 | 对应内部实现 |
|--------|------|------|--------------|
| `deploy` | — | 通过 docker-compose 部署 Nebula ID | `_deploy_impl.sh` |
| `lint` | `pre-commit` | 运行本地 CI 预检（fmt + clippy + test + 安全/文档/覆盖率） | `_pre_commit_impl.sh` |
| `redis-test` | — | 运行 Redis 集成测试 | `_redis_test_impl.sh` |
| `api-test` | — | 运行 API 端点测试，可选参数 `server_url` | `tests/api_test.sh` |
| `install-hooks` | — | 安装 git pre-commit hooks | `_install_hooks_impl.sh` |
| `pre-commit` | `lint` | 同 `lint`，运行本地 CI 预检 | `_pre_commit_impl.sh` |
| `help` | `--help`、`-h` | 显示 Usage 信息 | — |

**使用示例：**

```bash
# 显示帮助
./scripts/run.sh help

# 部署（docker-compose 全栈启动）
./scripts/run.sh deploy

# 本地 CI 预检（提交前必跑）
./scripts/run.sh pre-commit
# 或等价的别名
./scripts/run.sh lint

# Redis 集成测试（需先启动 Redis 监听 6379）
./scripts/run.sh redis-test

# API 端点测试（默认 http://localhost:8080）
./scripts/run.sh api-test
# 指定服务器 URL
./scripts/run.sh api-test http://localhost:8080

# 安装 git pre-commit hooks
./scripts/run.sh install-hooks
```

**GitHub Actions 集成：**

CI 也通过同一入口调用（见 `.github/workflows/ci.yml`、`release.yml`、`health-check.yml`），确保本地与 CI 行为一致。

更多细节见 [部署指南 — scripts/run.sh 子命令](docs/DEPLOYMENT.md#8-scriptsrunsh-子命令)。

---

## 🗺️ 路线图

<div align="center">

### 🎯 开发计划

</div>

<table>
<tr>
<td width="50%">

### ✅ 已完成

- [x] 核心ID生成算法
- [x] 支持双缓冲的Segment算法
- [x] Snowflake算法
- [x] UUID v8实现
- [x] 基于Etcd的分布式协调

</td>
<td width="50%">

### 🚧 进行中

- [ ] 增强监控和告警
- [ ] 多数据中心支持
- [ ] 性能优化
- [ ] 客户端SDK改进

</td>
</tr>
<tr>
<td width="50%">

### 📋 计划中

- [ ] 自动故障转移
- [ ] 动态算法切换
- [ ] 自定义ID格式支持
- [ ] 云服务提供商集成

</td>
<td width="50%">

### 💡 未来规划

- [ ] Kubernetes operator
- [ ] 多区域部署
- [ ] GraphQL API
- [ ] ID命名空间管理

</td>
</tr>
</table>

---

## 🤝 贡献指南

<div align="center">

### 💖 我们热爱贡献者！

</div>

<table>
<tr>
<td width="33%" align="center">

### 🐛 报告Bug

发现Bug？<br>
[创建Issue](https://github.com/nebula-id/nebula-id/issues)

</td>
<td width="33%" align="center">

### 💡 功能建议

有想法？<br>
[发起讨论](https://github.com/nebula-id/nebula-id/discussions)

</td>
<td width="33%" align="center">

### 🔧 提交PR

想要贡献？<br>
[Fork并提交PR](https://github.com/nebula-id/nebula-id/pulls)

</td>
</tr>
</table>

<details>
<summary><b>📝 贡献指南</b></summary>

<br>

### 如何贡献

1. **Fork** 本仓库
2. **克隆** 你的fork: `git clone https://github.com/yourusername/nebula-id.git`
3. **创建** 分支: `git checkout -b feature/amazing-feature`
4. **进行** 你的修改
5. **测试** 你的修改: `cargo test --features etcd`
6. **提交** 你的修改: `git commit -m 'Add amazing feature'`
7. **推送** 到分支: `git push origin feature/amazing-feature`
8. **创建** Pull Request

### 代码规范

- 遵循Rust标准编码规范
- 提交前运行 `cargo fmt` 和 `cargo clippy`
- 编写全面的测试
- 更新文档

</details>

---

## 📄 许可证

<div align="center">

本项目采用双许可证：

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE-MIT)
[![License: Apache 2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](LICENSE-APACHE)

你可以选择任一许可证使用。

</div>

---

## 🙏 致谢

<div align="center">

### 🛠️ 基于优秀工具构建

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
<b>开源</b>
</td>
<td align="center" width="25%">
<img src="https://img.icons8.com/fluency/96/000000/community.png" width="64" height="64"><br>
<b>社区</b>
</td>
</tr>
</table>

### 特别感谢

- 🌟 **依赖库** - 基于以下优秀项目：
  - [tokio](https://github.com/tokio-rs/tokio) - 异步运行时
  - [axum](https://github.com/tokio-rs/axum) - HTTP框架
  - [tonic](https://github.com/hyperium/tonic) - gRPC框架
  - [sea-orm](https://github.com/SeaQL/sea-orm) - 数据库ORM
  - [etcd-client](https://github.com/etcd-rs/etcd-client) - Etcd客户端（可选，`etcd` 特性）
  - [uuid](https://github.com/uuid-rs/uuid) - UUID生成
  - [confers](https://crates.io/crates/confers) - 配置管理
  - [oxcache](https://crates.io/crates/oxcache) - 多级缓存
  - [dbnexus](https://crates.io/crates/dbnexus) - 数据库抽象
  - [limiteron](https://crates.io/crates/limiteron) - 限流
  - [sdforge](https://crates.io/crates/sdforge) - 服务发现
  - [prometheus-client](https://github.com/prometheus/client_rust) - 指标库

- 👥 **贡献者** - 感谢所有优秀的贡献者！

---

## 📞 联系我们

<div align="center">

<table>
<tr>
<td align="center" width="50%">
<a href="https://github.com/nebula-id/nebula-id/issues">
<img src="https://img.icons8.com/fluency/96/000000/bug.png" width="48" height="48"><br>
<b>Issues</b>
</a><br>
报告Bug和问题
</td>
<td align="center" width="50%">
<a href="https://github.com/nebula-id/nebula-id/discussions">
<img src="https://img.icons8.com/fluency/96/000000/chat.png" width="48" height="48"><br>
<b>Discussions</b>
</a><br>
提问和分享想法
</td>
</tr>
</table>

### 关注我们

[![GitHub](https://img.shields.io/badge/GitHub-Follow-181717?style=for-the-badge&logo=github&logoColor=white)](https://github.com/nebula-id)
[![Crates.io](https://img.shields.io/badge/Crates.io-Version-DF5500?style=for-the-badge&logo=rust&logoColor=white)](https://crates.io/crates/nebula-id)

</div>

---

## ⭐ Star历史

<div align="center">

[![Star History Chart](https://api.star-history.com/svg?repos=nebula-id/nebula-id&type=Date)](https://star-history.com/#nebula-id/nebula-id&Date)

</div>

---

<div align="center">

### 💝 支持本项目

如果你觉得这个项目有用，请考虑给它一个⭐️！

**由 ❤️ 构建，Nebula ID团队**

[⬆ 返回顶部](#-nebula-id)

---

<sub>© 2025 Nebula ID. 保留所有权利。</sub>
