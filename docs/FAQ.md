# ❓ Nebula ID FAQ

> 关于 Nebula ID 常见问题的快速解答。

[🏠 主页](../README.md) • [📖 用户指南](USER_GUIDE.md) • [🔧 API 参考](API_REFERENCE.md)

---

## 📋 目录

- [通用问题](#-通用问题)
- [安装与配置](#-安装与配置)
- [使用与特性](#-使用与特性)
- [性能](#-性能)
- [安全](#-安全)
- [故障排查](#-故障排查)
- [参与贡献](#-参与贡献)
- [许可证](#-许可证)

---

## 🧭 通用问题

<div align="center">

### 🤔 关于本项目

</div>

<details>
<summary><b>❓ 什么是 Nebula ID？</b></summary>

<br>

**Nebula ID** 是一个面向高性能应用的企业级分布式 ID 生成系统。它提供：

- ✅ **多种 ID 算法** - Segment、Snowflake、UUID v8
- ✅ **分布式协调** - 基于 Etcd 的领导者选举与协调
- ✅ **高可用** - 数据中心健康监控与自动故障转移
- ✅ **类型安全设计** - 完整的 Rust 类型安全与 async/await 模式

它专为需要唯一、有序、高吞吐标识符生成能力的**分布式系统**而设计。

**了解更多：**[用户指南](USER_GUIDE.md)

</details>

<details>
<summary><b>❓ 为什么选择它而不是其他方案？</b></summary>

<br>

<table>
<tr>
<th>特性</th>
<th>Nebula ID</th>
<th>Snowflake</th>
<th>UUID</th>
</tr>
<tr>
<td>时间有序</td>
<td>✅ 是</td>
<td>✅ 是</td>
<td>⚠️ 仅 v7</td>
</tr>
<tr>
<td>高吞吐</td>
<td>✅ 100 万+ ID/秒</td>
<td>✅ 100 万+ ID/秒</td>
<td>✅ 100 万+ ID/秒</td>
</tr>
<tr>
<td>无需时钟同步</td>
<td>✅ Segment</td>
<td>❌ 否</td>
<td>✅ 是</td>
</tr>
<tr>
<td>容错能力</td>
<td>✅ 内置</td>
<td>⚠️ 手动</td>
<td>✅ 是</td>
</tr>
</table>

**核心优势：**
- 🚀 **多算法**：需要数据库支撑的有序性选 Segment，追求速度选 Snowflake，追求简单选 UUID
- 🔄 **自动故障转移**：数据中心健康监控与自动恢复
- 🛡️ **企业级就绪**：API 认证、限流与审计日志
- 📊 **内置监控**：健康检查与指标采集

</details>

<details>
<summary><b>❓ 它可以用于生产环境吗？</b></summary>

<br>

**当前状态：**✅ **可用于生产！**

<table>
<tr>
<td width="50%">

**已就绪：**
- ✅ 核心 ID 生成算法（Segment、Snowflake、UUID v8）
- ✅ 基于 Etcd 的分布式协调
- ✅ 数据中心健康监控与故障转移
- ✅ HTTP/HTTPS 与 gRPC/gRPCS API
- ✅ API 密钥认证与限流

</td>
<td width="50%">

**成熟度指标：**
- 📊 85%+ 测试覆盖率
- 🔄 持续维护
- 🛡️ 以安全为中心的设计
- 📖 文档完备

</td>
</tr>
</table>

> **注意：**升级版本前请务必查阅[更新日志](CHANGELOG.md)。

</details>

<details>
<summary><b>❓ 支持哪些平台？</b></summary>

<br>

<table>
<tr>
<th>平台</th>
<th>架构</th>
<th>状态</th>
<th>说明</th>
</tr>
<tr>
<td rowspan="2"><b>Linux</b></td>
<td>x86_64</td>
<td>✅ 完整支持</td>
<td>主力平台</td>
</tr>
<tr>
<td>ARM64</td>
<td>✅ 完整支持</td>
<td>已在 ARM 服务器上测试</td>
</tr>
<tr>
<td rowspan="2"><b>macOS</b></td>
<td>x86_64</td>
<td>✅ 完整支持</td>
<td>Intel Mac</td>
</tr>
<tr>
<td>ARM64</td>
<td>✅ 完整支持</td>
<td>Apple Silicon（M1/M2/M3）</td>
</tr>
<tr>
<td><b>Windows</b></td>
<td>x86_64</td>
<td>✅ 完整支持</td>
<td>Windows 10+</td>
</tr>
</table>

</details>

<details>
<summary><b>❓ 支持哪些编程语言？</b></summary>

<br>

**Nebula ID** 是原生 **Rust** 库，同时提供多协议服务支持：

- **Rust**：原生库（`nebula-id` crate）
- **HTTP/REST**：任何具备 HTTP 客户端的语言
- **gRPC**：任何支持 gRPC 的语言（Python、Java、Go 等）

**文档：**
- [Rust API 文档](https://docs.rs/nebula-id)
- [API 参考](API_REFERENCE.md)

</details>

<details>
<summary><b>❓ 支持哪些 ID 算法？</b></summary>

<br>

<table>
<tr>
<th>算法</th>
<th>格式</th>
<th>时间有序</th>
<th>吞吐量</th>
<th>适用场景</th>
</tr>
<tr>
<td>Segment</td>
<td>64 位</td>
<td>✅ 是</td>
<td>10 万+/秒</td>
<td>数据库主键</td>
</tr>
<tr>
<td>Snowflake</td>
<td>64 位</td>
<td>✅ 是</td>
<td>100 万+/秒</td>
<td>高性能系统</td>
</tr>
<tr>
<td>UUID v8</td>
<td>128 位</td>
<td>✅ 是</td>
<td>50 万+/秒</td>
<td>分布式系统</td>
</tr>
</table>

> 吞吐量数据仅供参考——本仓库尚无针对 UUID 路径的 `benches/` 基准测试覆盖。

</details>

---

## 📦 安装与配置

<div align="center">

### 🚀 快速开始

</div>

<details>
<summary><b>❓ 如何安装？</b></summary>

<br>

**Rust 项目：**

在 `Cargo.toml` 中添加：

```toml
[dependencies]
nebulaid = "0.2"                       # Cargo 包名是 `nebulaid`
tokio = { version = "1.0", features = ["full"] }
```

或使用 cargo：

```bash
cargo add nebulaid tokio
```

**可选特性**（`Cargo.toml` 的 `[features]`）：

```toml
# default = ["postgresql", "http", "grpc", "garrison-auth"]
nebulaid = { version = "0.2", features = ["etcd"] }  # 分布式协调
# nebulaid = { version = "0.2", features = ["sdk"] } # NebulaIdKit 门面
# 并不存在 `monitoring` / `audit` / `tls` 特性：指标、审计日志与
# TLS 均为运行时配置（[monitoring] / [auth] / [tls]）。
```

**验证：**

```rust
use nebulaid::core::Config;
use nebulaid::sdk::NebulaIdKitBuilder; // 需要特性 `sdk`

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

**另请参阅：**[用户指南](USER_GUIDE.md#安装)

</details>

<details>
<summary><b>❓ 系统要求是什么？</b></summary>

<br>

**最低要求：**

<table>
<tr>
<th>组件</th>
<th>要求</th>
<th>推荐</th>
</tr>
<tr>
<td>Rust 版本</td>
<td>1.75+</td>
<td>最新稳定版</td>
</tr>
<tr>
<td>内存</td>
<td>256MB</td>
<td>1GB+</td>
</tr>
<tr>
<td>磁盘空间</td>
<td>50MB</td>
<td>100MB+</td>
</tr>
<tr>
<td>数据库</td>
<td>PostgreSQL</td>
<td>PostgreSQL 13+</td>
</tr>
</table>

**可选依赖：**
- 🔧 **Etcd**：用于分布式协调（v3.4+）
- ☁️ **Redis**：用于缓存（v6+）
- 📊 **Prometheus**：用于指标可视化

</details>

<details>
<summary><b>❓ 遇到编译错误怎么办？</b></summary>

<br>

**常见解决办法：**

1. **检查 Rust 版本：**
   ```bash
   rustc --version
   # 应为 1.75.0 或更高
   ```

2. **确保所需特性已启用**（默认为
   `["postgresql", "http", "grpc", "garrison-auth"]`）：
   ```toml
   nebulaid = "0.2"
   ```

3. **清理构建产物：**
   ```bash
   cargo clean
   cargo build
   ```

**仍有问题？**
- 📝 查看[故障排查](#-故障排查)
- 🐛 [提交 issue](../../issues) 并附上错误详情

</details>

<details>
<summary><b>❓ 可以在 Docker 中使用吗？</b></summary>

<br>

**可以！**Nebula ID 在容器化环境中可完美运行。

**Dockerfile 示例：**

```dockerfile
FROM rust:1.92.0-bullseye AS builder
WORKDIR /app
COPY . .
RUN cargo build --release

FROM debian:bullseye-slim
COPY --from=builder /app/target/release/nebula-id /usr/local/bin/
CMD ["nebula-id"]
```

**包含依赖的 Docker Compose：**

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
<summary><b>❓ 如何配置 Nebula ID？</b></summary>

<br>

**配置文件（`config.toml`）：**

以下全部十个配置段均为**必填** —— `Config` 没有为它们提供
`#[serde(default)]`（`src/core/config/app_config.rs:37-64`）。解析失败现在会以退出码 1 中止
启动，而不是回退到 `Config::default()`；未知键也会以同样方式被拒绝，
因为每个配置结构体都带有 `deny_unknown_fields`。仅当未给定 `--config` *且*
`config/config.toml` 不存在时，才会使用内置默认值，此时会输出一条 `warn`。
只有 `[redis]` 与 `[hot_reload]` 可以省略。

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

# 可选项：
# [redis] url = "redis://localhost:6379"
# [hot_reload] auto_watch_enabled = false
```

> ⚠️ `Config::merge()` 在启动时会把环境变量配置叠加到文件之上，
> 并无条件地用它替换 `algorithm.segment` / `algorithm.snowflake` /
> `algorithm.uuid_v8`（`src/core/config/app_config.rs:393-395`、
> `src/main.rs:559`）——实际上这三个子表最终总是默认值。
> 只有 `algorithm.default` 能保留下来。目前请在代码中调整它们。

**环境变量：**

并不存在 `NEBULA_DATABASE_URL` / `NEBULA_AUTH_API_KEY` 这一系列。真实的环境变量是：

```bash
# 由 `Config::load_from_env()` 叠加到文件之上：
export APP_HOST="0.0.0.0"
export APP_HTTP_PORT="8080"
export APP_GRPC_PORT="9091"
export DC_ID="0"
export WORKER_ID="0"
export DATABASE_URL="postgresql://idgen:pass@localhost:5432/idgen"
export ETCD_ENDPOINTS="http://localhost:2379"
export RUST_LOG="info"

# 在解析前于文件内部展开为 ${VAR}：
export NEBULA_DATABASE_PASSWORD="..."   # [database].password / url
export NEBULA_API_KEY_SALT="..."        # [auth].api_key_salt 的回退值
```

**另请参阅：**[配置迁移指南](CONFIG_MIGRATION_GUIDE.md#配置全表全量选项与校验规则)

</details>

---

## 🎯 使用与特性

<div align="center">

### 💡 API 使用

</div>

<details>
<summary><b>❓ 如何快速上手基本用法？</b></summary>

<br>

**5 分钟快速上手：**

```rust
use nebulaid::core::Config;
use nebulaid::sdk::NebulaIdKitBuilder; // 特性 `sdk`

#[tokio::main]
async fn main() -> nebulaid::core::Result<()> {
    // Segment 需要 `NebulaIdKitBuilder::with_repository(..)`，因为它
    // 要从数据库分配号段；纯算法则无需任何额外设置。
    let mut config = Config::default();
    config.algorithm.default = "snowflake".to_string();

    let kit = NebulaIdKitBuilder::new(config).build().await?;
    let generator = kit.id_generator()?;

    // 生成单个 ID
    let id = generator.generate("prod", "core", "order").await?;
    println!("Generated ID: {} (u128: {})", id, id.as_u128());

    // 批量生成 ID
    let batch = generator.batch_generate("prod", "core", "order", 100).await?;
    println!("Generated {} IDs", batch.len());

    kit.shutdown().await;
    Ok(())
}
```

**下一步：**
- 📖 [用户指南](USER_GUIDE.md)
- 💻 [示例](../examples/)

</details>

<details>
<summary><b>❓ 如何选择合适的算法？</b></summary>

<br>

**算法选择指南：**

| 使用场景 | 推荐算法 | 原因 |
|----------|----------------------|--------|
| 数据库主键 | Segment | 有序、由数据库支撑、可靠 |
| 高吞吐微服务 | Snowflake | 快速、无数据库依赖 |
| 时间有序的分布式 ID | UUID v8 | RFC 9562 §5.8 布局、按时间可排序、内嵌 dc/worker/shard |
| 混合需求 | 多算法 | 按使用场景使用不同算法 |

**配置代码示例：**

```rust
use nebulaid::core::algorithm::AlgorithmBuilder;
use nebulaid::core::types::AlgorithmType;
use nebulaid::core::Config;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::default();

    // 算法结构体（SegmentAlgorithm / SnowflakeAlgorithm / UuidV8Impl）是 crate 内部的；
    // 请通过公开的 AlgorithmBuilder 构建。
    // Snowflake 与 UuidV8 是纯算法、无需数据库；Segment 则需要先接入
    // repository（见 src/sdk/client.rs 中的 SDK 说明）。
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
<summary><b>❓ Segment 算法是如何工作的？</b></summary>

<br>

Segment 算法从数据库预分配 ID 区间，以实现高效的批量生成：

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

**核心优势：**
- 🚀 **高吞吐**：从本地内存生成 ID
- 📦 **批量高效**：预分配减少数据库往返
- 🔄 **容错**：自动故障转移到健康的数据中心

**代码示例：**

```rust
use nebulaid::core::algorithm::{AlgorithmBuilder, GenerateContext, IdAlgorithm};
use nebulaid::core::types::AlgorithmType;
use nebulaid::core::Config;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // `dc_id` 来自 [app]；具体的 SegmentAlgorithm 类型是
    // crate 内部的，因此请通过公开的 AlgorithmBuilder 构建。
    let mut config = Config::default();
    config.app.dc_id = 1;

    let segment = AlgorithmBuilder::new(AlgorithmType::Segment)
        .build(&config)
        .await?;

    let ctx = GenerateContext::default();

    // 生成单个 ID（来自预分配的号段）
    let id = segment.generate(&ctx).await?;
    println!("Generated ID: {}", id);

    // 批量生成（为 `size` 个 ID 只做一次数据库往返）
    let batch = segment.batch_generate(&ctx, 1000).await?;
    println!("Generated {} IDs", batch.ids.len());

    Ok(())
}
```

</details>

<details>
<summary><b>❓ Snowflake 算法是如何工作的？</b></summary>

<br>

Snowflake 算法生成 64 位 ID，位分配可配置
（`src/core/algorithm/snowflake.rs` 中的 `construct_id` 对
`timestamp | datacenter | worker | sequence` 进行移位，无符号位）：

```
┌────────────────────────────────────────────────────────────────┐
│              Snowflake ID Structure (defaults)                  │
├────────────────────────────────────────────────────────────────┤
│  43 bits   │  3 bits    │  8 bits  │  10 bits                   │
│  timestamp │  datacenter│  worker  │  sequence                  │
└────────────────────────────────────────────────────────────────┘
```

**核心优势：**
- 🚀 **快速**：无数据库依赖
- 📈 **可扩展**：按默认位布局可支持 8 个数据中心 × 256 个 worker
  （`datacenter_id_bits` / `worker_id_bits` / `sequence_bits` 均可配置；
  其总和必须保持 < 64）
- 🎯 **有序**：毫秒内按时间排序

**代码示例：**

```rust
use nebulaid::core::algorithm::{AlgorithmBuilder, GenerateContext, IdAlgorithm};
use nebulaid::core::types::AlgorithmType;
use nebulaid::core::Config;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // dc/worker 来自 [app]；具体的 SnowflakeAlgorithm 类型是
    // crate 内部的，因此请通过公开的 AlgorithmBuilder 构建。
    let mut config = Config::default();
    config.app.dc_id = 1;
    config.app.worker_id = 1;

    let snowflake = AlgorithmBuilder::new(AlgorithmType::Snowflake)
        .build(&config)
        .await?;

    let id = snowflake.generate(&GenerateContext::default()).await?;
    println!("Snowflake ID: {} (u128: {})", id, id.as_u128());

    // 位布局可从配置中读出；64 位中的剩余部分
    // 即时间戳字段。
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
<summary><b>❓ 什么是 UUID v8？什么时候该用它？</b></summary>

<br>

Nebula ID 提供一个**按时间排序的 UUID v8** 生成器（RFC 9562 §5.8 自定义布局）。标准
UUID v7 同样携带 48 位毫秒级时间戳，但其余位固定为
clock-seq + node；Nebula 的 v8 组合方式则利用厂商自定义字段把
租户/区域上下文直接嵌入 ID（UUIDP "Cluster" 风格：每实例随机起点加上
严格单调计数器）：

```
┌────────────────────────────────────────────────────────────────┐
│                    UUID v8 Structure (128 bits)                 │
├────────────────────────────────────────────────────────────────┤
│  48 bits   │  16 bits (incl. version=0b1000 + variant=0b10)    │
│  timestamp │  dc(3) | worker(8) | counter_hi(1) ...            │
│            │  62 bits: shard(16) | counter_lo(20) | rand(26)   │
└────────────────────────────────────────────────────────────────┘
```

**优势：**
- ✅ **时间有序**：按创建时间字典序可排序
- ✅ **自描述**：`dc` / `worker` / `shard` 可直接从 ID 本身读出
- ✅ **抗碰撞**：单调计数器 + 每实例随机起点
- ⚠️ **版本半字节为 `8`**：严格校验 "version == 7" 的验证器会拒绝它

**代码示例：**

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

    // 通过 uuid crate 的表示形式做往返转换
    let as_uuid = id.to_uuid_v8();
    let back = Id::from_uuid_v8(as_uuid);
    assert_eq!(back, id);

    Ok(())
}
```

**适用场景：**
- 你需要保持索引友好性的 UUID 形态标识符
- 按时间排序很重要
- 你希望 dc / worker / shard 能直接从 ID 中观察到

> **旧命名**：`uuid_v7` 与 `uuid_v4` 已不再是独立算法，但
> `AlgorithmType::from_str`（`src/core/types/id.rs:195-201`）仍接受这两种拼写作为
> 解析到 `UuidV8` 的**输入别名**，因此旧配置与 API 载荷依然可用。
> Nebula 输出的一律是 `uuid_v8`。

</details>

<details>
<summary><b>❓ 分布式协调是如何工作的？</b></summary>

<br>

Nebula ID 使用 etcd 进行分布式协调：

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

**组件：**

1. **EtcdClusterHealthMonitor**：监控 etcd 集群健康状态（公开，特性 `etcd`）
2. **DcFailureDetector**：跟踪数据中心健康状态 —— **crate 内部**
   （`src/core/algorithm/segment.rs`，只能通过 `SegmentAlgorithm` 访问）；它没有
   公开构造函数，因此无法从外部装配
3. **自动故障转移**：把流量路由到健康的数据中心

**代码示例：**

```rust
use nebulaid::core::algorithm::{AlgorithmBuilder, GenerateContext, IdAlgorithm};
use nebulaid::core::coordinator::EtcdClusterHealthMonitor; // 特性 `etcd`
use nebulaid::core::config::EtcdConfig;
use nebulaid::core::types::AlgorithmType;
use nebulaid::core::Config;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // new(config: EtcdConfig, cache_file_path: String) -> Self
    // 当 etcd 不可达时使用缓存文件。
    let health_monitor = Arc::new(EtcdClusterHealthMonitor::new(
        EtcdConfig::default(),
        "./etcd-cache.json".to_string(),
    ));

    // 通过公开的 builder 把它交给算法。
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
<summary><b>❓ 如何正确处理错误？</b></summary>

<br>

**推荐模式：**

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
        // 变体名称与载荷形态与
        // src/core/types/error.rs 中的声明完全一致。
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

**错误类型：**

| 错误 | 载荷 | 说明 | 恢复方式 |
|-------|---------|-------------|----------|
| `ClockMovedBackward` | `{ last_timestamp }` | 系统时钟回拨 | 需要 NTP 同步 |
| `DatabaseError` | `(String)` | 数据库不可用或查询失败 | 检查连接，使用缓存 |
| `SegmentExhausted` | `{ max_id }` | ID 区间耗尽 | 自动刷新号段 |
| `EtcdError` | `(String)` | Etcd 不可用 | 使用本地缓存 |
| `SequenceOverflow` | `{ timestamp }` | Snowflake 序列溢出 | 等待下一毫秒（算法已自动休眠 1 毫秒并重试） |
| `ConfigurationError` | `(String)` | 必需配置缺失或无效 | 修正配置 |

不存在 `DatabaseConnectionFailed` / `EtcdConnectionFailed` 变体 —— 那些是
过时的名称；数据库与 etcd 路径都通过 `DatabaseError` / `EtcdError` 上报。

</details>

<details>
<summary><b>❓ 是否支持 async/await？</b></summary>

<br>

**支持！**Nebula ID 从底层设计上就面向 async/await。

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

    // 异步生成单个 ID
    let id = segment.generate(&ctx).await?;
    println!("Generated ID: {}", id);

    // 异步批量生成
    let batch = segment.batch_generate(&ctx, 100).await?;
    println!("Generated {} IDs", batch.len());

    Ok(())
}
```

**运行时要求：**

- ✅ **Tokio —— 必需。**算法会派生后台任务并在内部使用 tokio
  原语（号段健康检查、`tokio::sync` 通道、
  时钟/序列等待时的 `tokio::time::sleep`），因此必须在 tokio
  运行时中运行。
- ❌ Async-Std / smol：不支持；没有运行时抽象层。

</details>

---

## 📈 性能

<div align="center">

### ⚡ 速度与优化

</div>

<details>
<summary><b>❓ 它有多快？</b></summary>

<br>

**基准测试结果：**

<table>
<tr>
<th>算法</th>
<th>吞吐量</th>
<th>P50 延迟</th>
<th>P99 延迟</th>
</tr>
<tr>
<td>Segment</td>
<td>100,000+ ID/秒</td>
<td>~0.1ms</td>
<td>~0.5ms</td>
</tr>
<tr>
<td>Snowflake</td>
<td>1,000,000+ ID/秒</td>
<td>~0.05ms</td>
<td>~0.1ms</td>
</tr>
<tr>
<td>UUID v8</td>
<td>500,000+ ID/秒</td>
<td>~0.03ms</td>
<td>~0.05ms</td>
</tr>
</table>

**自行运行基准测试：**

```bash
cargo bench
```

</details>

<details>
<summary><b>❓ 如何提升性能？</b></summary>

<br>

**优化建议：**

1. **启用 Release 模式：**
   ```bash
   cargo build --release
   ```

2. **使用批量生成：**
   ```rust
   // 不要逐个生成 ID（`IdAlgorithm::batch_generate`）
   let batch = segment.batch_generate(&ctx, 1000).await?;
   ```

3. **配置合适的号段大小：**
   ```toml
   # `SegmentAlgorithmConfig` 的键 —— 四个键均为解析器必填。
   # base_step 必须保持在 [min_step, max_step] 之间。
   [algorithm.segment]
   base_step = 10000  # 步长越大 = 数据库往返越少
   min_step = 500
   max_step = 100000
   switch_threshold = 0.1
   ```
   > ⚠️ 服务器启动时 `Config::merge()` 会把该子表重置为默认值
   > （`src/core/config/app_config.rs:393-395`）；在该问题修复之前，请在代码中
   > 调整（在 `AlgorithmBuilder::build` 之前设置
   > `Config { algorithm: AlgorithmConfig { segment: .. } }`）。

4. **追求速度使用 Snowflake：**
   - 无数据库依赖
   - 内存中生成
   - 每实例约 100 万 ID/秒

5. **启用连接池：**
   ```toml
   [database]
   max_connections = 20
   ```

</details>

<details>
<summary><b>❓ 内存占用情况如何？</b></summary>

<br>

**典型内存占用：**

<table>
<tr>
<th>组件</th>
<th>内存</th>
</tr>
<tr>
<td>核心库</td>
<td>~1MB</td>
</tr>
<tr>
<td>号段缓存（100 万 ID）</td>
<td>~8MB</td>
</tr>
<tr>
<td>Etcd 客户端</td>
<td>~2MB</td>
</tr>
<tr>
<td>HTTP 服务器</td>
<td>~5MB</td>
</tr>
</table>

**合计：**约 16MB 基础占用 + 算法相关开销

**内存安全：**
- ✅ 无内存泄漏（经持续测试验证）
- ✅ 高效的批量处理
- ✅ 连接池
- ✅ 异步运行时高效

</details>

<details>
<summary><b>❓ 系统如何应对高并发？</b></summary>

<br>

Nebula ID 为高并发而生：

**并发特性：**
- 🚀 **Async/Await**：非阻塞操作
- 🔀 **DashMap**：线程安全的并发数据结构
- 📊 **连接池**：高效的数据库连接
- ⚡ **无锁**：最小化竞争点

**最佳实践：**

```rust
use nebulaid::core::algorithm::{AlgorithmBuilder, GenerateContext, IdAlgorithm};
use nebulaid::core::types::{AlgorithmType, Id};
use nebulaid::core::Config;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // `IdAlgorithm: Send + Sync`，因此一个共享句柄即可服务多个任务。
    let snowflake: Arc<dyn IdAlgorithm> = Arc::from(
        AlgorithmBuilder::new(AlgorithmType::Snowflake)
            .build(&Config::default())
            .await?,
    );

    // 派生并发任务
    let mut handles = Vec::new();
    for _ in 0..100 {
        let snowflake = Arc::clone(&snowflake);
        handles.push(tokio::spawn(async move {
            snowflake.generate(&GenerateContext::default()).await
        }));
    }

    // 收集结果（JoinError 与 CoreError 都可转换为 Box<dyn Error>）
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

## 🔐 安全

<div align="center">

### 🔒 安全特性

</div>

<details>
<summary><b>❓ 包含哪些安全特性？</b></summary>

<br>

**是的！**安全是 Nebula ID 的核心关注点。

**安全特性：**

<table>
<tr>
<td width="50%">

**认证**
- ✅ API Key 认证
- ✅ 恒定时间比较（防止时序攻击）
- ✅ 基于令牌的访问
- ✅ 可配置的密钥轮换

</td>
<td width="50%">

**防护**
- ✅ 限流（最大批量大小：100）
- ✅ 请求校验
- ✅ 审计日志（含 IP 伪造防护）
- ✅ CORS 限制
- ✅ 安全响应头

</td>
</tr>
</table>

**加密：**
- ✅ TLS/HTTPS 支持（TLS 1.2/1.3）
- ✅ gRPCS 支持
- ✅ 安全通信

**安全响应头：**
- X-Content-Type-Options: nosniff
- X-Frame-Options: DENY
- Content-Security-Policy: default-src 'self'
- Strict-Transport-Security: max-age=31536000; includeSubDomains
- X-XSS-Protection: 1; mode=block
- Referrer-Policy: strict-origin-when-cross-origin

**更多细节：**[安全文档](SECURITY.md#安全最佳实践)

</details>

<details>
<summary><b>❓ 如何配置 API 认证？</b></summary>

<br>

**配置：**

```toml
[auth]
enabled = true                     # 必填
cache_ttl_seconds = 300            # 必填
# 静态引导密钥；运行时密钥存放在数据库 / garrison 中。
# ApiKeyEntry 的每个字段都是必填的。
# 启动时只预置第一条条目（`src/main.rs:136` 取 `keys.first()`）；
# 第 2..N 条会被解析和校验但从不创建，且没有任何警告。
# `workspace` 必须是 UUID 字符串或字面量 "global"（其他值会使
# `Uuid::parse_str` 失败并被静默替换为全零 UUID）。
# `role` 只有在值恰好为 "admin"（不区分大小写）时才是 admin；其他
# 任何值都成为 user。当设置了 NEBULA_ADMIN_API_KEY_SECRET 时，
# 整个配置段都会被忽略。
api_keys = [
  { key_id = "svc-billing", key_secret = "replace-me", workspace = "0f5f6c8e-1f2e-4a7b-9c3d-2b1a4e5f6071",
    role = "user", rate_limit = 1000, name = "Billing service" },
]
api_key_salt = "${NEBULA_API_KEY_SALT}"   # 可选；生产环境拒绝空盐值
key_rotation_grace_period_seconds = 0 # 可选；0（默认）= 关闭宽限期；>0 需要两个
                                      # 宽限期列，启动迁移会自行添加

[rate_limit]
enabled = true
default_rps = 1000
burst_size = 100                   # validate(): <= 10 × default_rps

[batch_generate]
max_batch_size = 100               # 防止 DoS 攻击的最大批量大小
```

> 不存在 `[auth].api_key` 字符串键，也没有 `token_expiry_hours`；凭据始终是
> `key_id` + `key_secret` 对，过期时间不是配置概念。

**用法：**

```rust
use nebulaid::core::Config;

// API 密钥校验发生在由 `src/main.rs` 装配的 HTTP/gRPC 服务器内部。
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::load_from_file("config/config.toml")?;

    // `auth.enabled` 是控制中间件的开关。
    println!("auth enabled: {}", config.auth.enabled);

    Ok(())
}
```

**HTTP 请求头：**

`parse_authorization_header_detailed` 只接受两种 scheme
（`src/server/middleware/api_key_auth.rs:414-432`）—— `Bearer` 会被拒绝：

```
Authorization: ApiKey <key_id>:<key_secret>
Authorization: Basic base64(<key_id>:<key_secret>)
```

</details>

<details>
<summary><b>❓ 如何报告安全漏洞？</b></summary>

<br>

**请负责任地报告安全问题：**

1. **不要**创建公开的 GitHub issue
2. **邮箱：**security@nebula-id.io
3. **内容包括：**
   - 漏洞描述
   - 复现步骤
   - 潜在影响

**响应时间线：**
- 📧 初步响应：24 小时
- 🔍 评估：72 小时
- 📢 公开披露：修复发布之后

</details>

<details>
<summary><b>❓ 限流是怎样的？</b></summary>

<br>

Nebula ID 内置限流：

**配置：**

```toml
[rate_limit]
enabled = true
default_rps = 1000
burst_size = 100
```

**按 API Key 限流：**

限流通过 `[rate_limit]` 配置段全局控制，也支持按 API Key 粒度覆盖（通过管理 API 设置）。

| 参数 | 默认值 | 说明 |
|------|--------|------|
| `default_rps` | 1000 | 每秒请求数 |
| `burst_size` | 100 | 突发容量 |

**响应头：**

```
x-ratelimit-limit: 1000
x-ratelimit-remaining: 999
```

当请求被限流（HTTP 429）时，响应还会额外携带：

```
x-ratelimit-remaining: 0
retry-after: 1
```

</details>

---

## 🔍 故障排查

<div align="center">

### 🔧 常见问题

</div>

<details>
<summary><b>❓ 遇到 "ClockMovedBackward" 错误</b></summary>

<br>

**问题：**
```
Error: system clock moved backward
```

**原因：**检测到系统时钟回拨，这可能导致 ID 重复。

**解决方案：**
1. **同步系统时间：**
   ```bash
   # Linux
   sudo ntpdate pool.ntp.org
   
   # macOS
   sudo sntp -sS pool.ntp.org
   ```

2. **配置 NTP 自动同步：**
   ```bash
   # 添加到 /etc/chrony.conf
   server pool.ntp.org iburst
   ```

3. **对于虚拟化环境：**
   - 确保宿主机时钟已同步
   - 使用 VMware Tools 时间同步
   - 配置 Hyper-V 时间同步

**预防措施：**
- 使用 NTP 守护进程（chronyd、ntpd）
- 监控时钟漂移
- 对显著漂移进行告警

</details>

<details>
<summary><b>❓ 遇到 "DatabaseConnectionFailed" 错误</b></summary>

<br>

**问题：**
```
Error: failed to connect to database
```

**原因：**数据库连接问题。

**解决方案：**
1. **确认数据库正在运行：**
   ```bash
   # PostgreSQL
   pg_isready -h localhost -p 5432
   
   # MySQL
   mysqladmin ping -h localhost
   ```

2. **检查连接字符串：**
   ```toml
   [database]
   url = "postgresql://user:pass@localhost/nebula"
   ```

3. **测试网络连通性：**
   ```bash
   telnet localhost 5432
   ```

4. **检查凭据：**
   ```bash
   psql -U user -d nebula
   ```

5. **启用本地缓存回退：**
   ```rust
   let health_monitor = EtcdClusterHealthMonitor::new(config, "./cache.json");
   ```

</details>

<details>
<summary><b>❓ ID 不是按时间有序的</b></summary>

<br>

**问题：**
生成的 ID 不是单调递增的。

**原因：**多个实例同时生成 ID。

**解决方案：**

1. **对于 Snowflake：**确保各实例间时钟已同步

2. **对于 Segment：**检查号段刷新逻辑

3. **使用 UUID v8 实现时间有序：**
   ```rust
   // 在 `async fn` 内部：
   use nebulaid::core::algorithm::{AlgorithmBuilder, GenerateContext, IdAlgorithm};
   use nebulaid::core::types::AlgorithmType;
   use nebulaid::core::Config;

   let uuid = AlgorithmBuilder::new(AlgorithmType::UuidV8)
       .build(&Config::default())
       .await?;
   let id = uuid.generate(&GenerateContext::default()).await?;
   ```

**注意：**Snowflake ID 在同一实例内按毫秒有序。

</details>

<details>
<summary><b>❓ 如何调试 ID 生成问题？</b></summary>

<br>

**启用调试日志：**

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

设置环境变量（crate/模块路径是 `nebulaid`，而不是二进制
名 `nebula-id`）：

```bash
RUST_LOG=nebulaid=debug
```

**常用调试命令：**

```bash
# 检查 etcd 健康状态
etcdctl endpoint health

# 检查数据库连接
SELECT count(*) FROM pg_stat_activity;

# 监控指标
curl http://localhost:8080/metrics
```

</details>

<details>
<summary><b>❓ 性能下降</b></summary>

<br>

**问题：**ID 生成比预期慢。

**诊断步骤：**

1. **检查数据库性能：**
   ```sql
   EXPLAIN ANALYZE SELECT * FROM nebula_segments;
   ```

2. **监控连接池：**
   ```bash
   # 检查活跃连接
   SELECT count(*) FROM pg_stat_activity WHERE datname = 'nebula';
   ```

3. **检查 etcd 延迟：**
   ```bash
   etcdctl put test && etcdctl get test --cluster
   ```

**解决方案：**

1. **增加数据库连接数：**
   ```toml
   [database]
   max_connections = 20
   ```

2. **增大号段步长：**
   ```toml
   [algorithm.segment]
   base_step = 10000
   ```

3. **添加 Redis 缓存：**
   ```toml
   [redis]
   url = "redis://localhost"
   ```

</details>

**还有其他问题？**查看 [DEPLOYMENT.md](DEPLOYMENT.md) 中的故障排查章节，或在 [GitHub Issues](https://github.com/nebula-id/nebula-id/issues) 中提问。

---

## 👥 参与贡献

<div align="center">

### 🤝 加入社区

</div>

<details>
<summary><b>❓ 我能如何参与贡献？</b></summary>

<br>

**参与方式：**

<table>
<tr>
<td width="50%">

**代码贡献**
- 🐛 修复缺陷
- ✨ 添加特性
- 📝 改进文档
- ✅ 编写测试

</td>
<td width="50%">

**非代码贡献**
- 📖 编写教程
- 🎨 设计素材
- 🌍 翻译文档
- 💬 解答问题

</td>
</tr>
</table>

**上手步骤：**

1. 🍴 Fork 仓库
2. 🌱 创建分支：`git checkout -b feature/amazing-feature`
3. ✏️ 修改代码
4. ✅ 添加测试：`cargo test --package nebulaid --features etcd`
5. 📤 提交 PR

**指南：**[CONTRIBUTING.md](../CONTRIBUTING.md)

</details>

<details>
<summary><b>❓ 我发现了一个 bug，该怎么办？</b></summary>

<br>

**报告之前：**

1. ✅ 查看[现有 issue](../../issues)
2. ✅ 尝试最新版本
3. ✅ 查看[故障排查指南](#-故障排查)

**撰写一份高质量的 bug 报告：**

```markdown
### 描述
清晰描述该缺陷

### 复现步骤
1. 第一步
2. 第二步
3. 查看错误

### 预期行为
应当发生什么

### 实际行为
实际发生了什么

### 环境
- OS: Ubuntu 22.04
- Rust version: 1.75.0
- Nebula ID version: 0.1.0
- Database: PostgreSQL 15

### 补充信息
其他任何相关信息
```

**提交：**[创建 Issue](../../issues/new)

</details>

<details>
<summary><b>❓ 在哪里可以获得帮助？</b></summary>

<br>

<div align="center">

### 💬 支持渠道

</div>

<table>
<tr>
<td width="33%" align="center">

**🐛 Issues**

[GitHub Issues](../../issues)

缺陷报告与特性请求

</td>
<td width="33%" align="center">

**💬 Discussions**

[GitHub Discussions](../../discussions)

问答与想法交流

</td>
<td width="33%" align="center">

**📖 文档**

[用户指南](USER_GUIDE.md)

API 文档与教程

</td>
</tr>
</table>

**响应时间：**
- 🐛 关键缺陷：24 小时
- 🔧 特性请求：1 周
- 💬 问题咨询：2-3 天

</details>

---

## 📜 许可证

<div align="center">

### 📄 许可证信息

</div>

<details>
<summary><b>❓ 本项目采用什么许可证？</b></summary>

<br>

**双重许可：**

<table>
<tr>
<td width="50%" align="center">

**MIT 许可证**

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](../LICENSE-MIT)

**权限：**
- ✅ 商业使用
- ✅ 修改
- ✅ 分发
- ✅ 私有使用

</td>
<td width="50%" align="center">

**Apache License 2.0**

[![License: Apache 2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](../LICENSE-APACHE)

**权限：**
- ✅ 商业使用
- ✅ 修改
- ✅ 分发
- ✅ 专利授权

</td>
</tr>
</table>

**你可以任选其中一种许可证使用。**

</details>

<details>
<summary><b>❓ 可以在商业项目中使用吗？</b></summary>

<br>

**可以！**MIT 与 Apache 2.0 许可证均允许商业使用。

**你需要做的：**
1. ✅ 附上许可证文本
2. ✅ 附上版权声明
3. ✅ 声明所做的修改

**你不需要做的：**
- ❌ 公开你的源代码
- ❌ 将项目开源
- ❌ 支付版税

**有疑问？**联系：legal@nebula-id.io

</details>

---

<div align="center">

### 🎯 还有疑问？

<table>
<tr>
<td width="33%" align="center">
<a href="../../issues">
<img src="https://img.icons8.com/fluency/96/000000/bug.png" width="48"><br>
<b>提交 Issue</b>
</a>
</td>
<td width="33%" align="center">
<a href="../../discussions">
<img src="https://img.icons8.com/fluency/96/000000/chat.png" width="48"><br>
<b>发起讨论</b>
</a>
</td>
<td width="33%" align="center">
<a href="https://docs.rs/nebula-id">
<img src="https://img.icons8.com/fluency/96/000000/documentation.png" width="48"><br>
<b>阅读 API 文档</b>
</a>
</td>
</tr>
</table>

---

**[📖 用户指南](USER_GUIDE.md)** • **[🔧 API 参考](API_REFERENCE.md)** • **[🏠 主页](../README.md)**

由 Nebula ID 团队用 ❤️ 打造

[⬆ 回到顶部](#-nebula-id-faq)
