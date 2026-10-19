# 📖 Nebula ID 用户指南

> 完整的 Nebula ID 使用指南：从安装入门到进阶用法与最佳实践。

[🏠 首页](../README.md) • [🎯 示例](../examples/) • [❓ 常见问题](FAQ.md)

---

## 📋 目录

- [简介](#简介)
- [快速入门](#快速入门)
  - [先决条件](#先决条件)
  - [安装](#安装)
  - [第一步](#第一步)
- [核心概念](#核心概念)
- [特性组合](#特性组合)
- [基础用法](#基础用法)
  - [Segment 算法](#segment-算法)
  - [Snowflake 算法](#snowflake-算法)
  - [UUID 生成](#uuid-生成)
- [高级用法](#高级用法)
  - [分布式协调](#分布式协调)
  - [健康监控](#健康监控)
  - [性能优化](#性能优化)
  - [SDK 嵌入](#sdk-嵌入)
- [最佳实践](#最佳实践)
- [故障排除](#故障排除)
- [后续步骤](#后续步骤)

---

## 简介

<div align="center">

### 🎯 你将学到什么

</div>

<table>
<tr>
<td width="25%" align="center">
<img src="https://img.icons8.com/fluency/96/000000/rocket.png" width="64"><br>
<b>快速入门</b><br>
5 分钟内完成 ID 生成集成
</td>
<td width="25%" align="center">
<img src="https://img.icons8.com/fluency/96/000000/settings.png" width="64"><br>
<b>多种算法</b><br>
Segment、Snowflake、UUID
</td>
<td width="25%" align="center">
<img src="https://img.icons8.com/fluency/96/000000/code.png" width="64"><br>
<b>最佳实践</b><br>
学习分布式 ID 生成
</td>
<td width="25%" align="center">
<img src="https://img.icons8.com/fluency/96/000000/rocket-take-off.png" width="64"><br>
<b>高级特性</b><br>
掌握分布式协调与监控
</td>
</tr>
</table>

**Nebula ID** 是一个功能强大的企业级分布式 ID 生成系统，提供多种高性能、高可用的 ID 生成算法，包括 Segment（号段）、Snowflake（雪花）以及时间有序的 UUID v8 实现（RFC 9562 §5.8 自定义布局）。它专为分布式系统设计，支持数据中心健康监控、故障自动转移和毫秒级延迟。

> 💡 **提示**: 本指南假设你具备基本的 Rust 知识。如果你是 Rust 新手，建议先阅读 [Rust 官方教程](https://doc.rust-lang.org/book/)。

---

## 快速入门

### 先决条件

在开始之前，请确保你已安装以下工具：

<table>
<tr>
<td width="50%">

**必选**
- ✅ Rust 1.75+ (stable)
- ✅ Cargo (随 Rust 一起安装)
- ✅ Git

</td>
<td width="50%">

**可选**
- 🔧 支持 Rust 的 IDE (如 VS Code + rust-analyzer)
- 🔧 Docker (用于容器化部署)
- 🔧 PostgreSQL (用于 Segment 算法持久化)

</td>
</tr>
</table>

<details>
<summary><b>🔍 验证安装</b></summary>

```bash
# 检查 Rust 版本
rustc --version
# 预期: rustc 1.75.0 (或更高)

# 检查 Cargo 版本
cargo --version
# 预期: cargo 1.75.0 (或更高)
```

</details>

### 安装

在你的 `Cargo.toml` 中添加 `nebulaid`：

```toml
[dependencies]
nebulaid = { version = "0.3", features = ["postgresql"] }

# 如需 etcd 分布式协调（推荐生产环境）
# nebulaid = { version = "0.3", features = ["postgresql", "etcd"] }
```

或者使用命令行（自动添加最新版本到 Cargo.toml）：

```bash
cargo add nebulaid
```

> 💡 **可用 features**：`postgresql`（默认）、`etcd`、`alerting`、`garrison-auth`（默认）、`sdk`、`integration-tests`、`http` / `grpc`（默认，sdforge 镜像）。`--all-features` 可构建。项目为单包结构（无 `crates/` 子目录），不要使用 `path = "./crates/core"`。

### 第一步

让我们通过一个简单的例子来验证安装。我们将使用 Segment 算法生成分布式 ID（通过统一的 `AlgorithmBuilder` 入口）：

```rust
use nebulaid::core::algorithm::{AlgorithmBuilder, GenerateContext, IdAlgorithm};
use nebulaid::core::types::AlgorithmType;
use nebulaid::core::Config;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 加载配置（datacenter_id 等在 [app] 段配置，详见 config/config.toml）
    let config = Config::default();

    // 通过统一的 AlgorithmBuilder 构建算法实例（返回 Box<dyn IdAlgorithm>）
    let segment = AlgorithmBuilder::new(AlgorithmType::Segment)
        .build(&config)
        .await?;

    // 生成 ID（需传入 GenerateContext）
    let ctx = GenerateContext::default();
    let id = segment.generate(&ctx).await?;

    println!("Generated ID: {}", id);
    Ok(())
}
```

---

## 核心概念

理解这些核心概念将帮助你更有效地使用 `Nebula ID`。

### 1️⃣ ID 生成算法

`Nebula ID` 提供五种核心算法：
- **Segment (号段)**: 基于数据库的号段分配，支持高并发批量获取
- **Snowflake (雪花)**: Twitter 风格的分布式 ID，时间有序、无需协调
- **UUID v8 (通用唯一标识符)**: 时间有序的私有位布局（RFC 9562 §5.8，内嵌 dc/worker/shard，租户上下文独占）
- **UUID v7**: RFC 9562 §5.7 标准时间有序——时间戳可被任何标准工具直取，零 dc/worker 配置（add-uuid-v4-v7 起为真算法，不再是别名）
- **UUID v4**: 122 位随机 + 完全零元数据（不泄漏创建时间与部署结构，add-uuid-v4-v7 起为真算法）

### 2️⃣ 数据中心 (Datacenter)

Segment 算法支持多数据中心部署，每个数据中心分配唯一的 DCID，用于：
- 隔离不同数据中心的 ID 区间
- 实现跨数据中心的负载均衡
- 支持数据中心故障自动转移

### 3️⃣ 分布式协调

`Nebula ID` 内置分布式协调机制：
- **健康监控**: 实时监控各数据中心状态
- **故障检测**: 自动检测并标记失效的数据中心
- **智能调度**: 自动将流量转移到健康的数据中心

---

## 特性组合

`Nebula ID` 采用模块级特性门控，按消费形态裁剪依赖树（数字为 `cargo tree -e normal` 实测唯一 crate 数）：

| 消费形态 | 推荐组合 | 依赖树规模 |
|---|---|---|
| 部署完整服务 | default（`server` + 五算法 + `postgresql` + `garrison-auth`） | 519 |
| 嵌入式 SDK | `sdk`（蕴含 `openapi`；不蕴含 `database`，仓储面另显式启用） | 197（`sdk,database` 组合为 283） |
| 仅号段算法 | `segment`（蕴含 `database`） | 282 |
| 仅 Snowflake / UUID v8 | `snowflake` 或 `uuid-v8` | 196 |
| 仅 UUID v7 / v4 | `uuid-v7` 或 `uuid-v4` | 196 |
| 仅 UUID v5 确定性派生 | `uuid-v5` | 196（kit 不参与：直调 `derive_uuid_v5`） |

依赖树规模口径：`cargo tree -e normal --prefix none | sort -u | wc -l`（唯一 name+version 包数，2026-10-11 实测）。

```toml
# Cargo.toml —— 只用 UUID v8 的剪裁示例
[dependencies]
nebulaid = { version = "0.3", default-features = false, features = ["uuid-v8"] }
```

要点：

- 请求未启用的算法时返回 `InvalidAlgorithmType` 错误，不 panic、不静默回退到其他算法。
- `nebula-id` 二进制仅随 `server` 特性构建（`required-features`），纯算法剪裁组合自动跳过二进制目标。
- `alerting` 特性同时门控告警子系统、webhook 分发与 `reqwest` 依赖；本地运行告警单测需 `cargo test --features alerting`（CI 经 `--all-features` 覆盖）。
- 剪裁组合由 CI `Check feature trim` 步骤钉住，防止越界引用使剪裁能力静默腐烂。

---

## 基础用法

### Segment 算法

Segment 算法通过预分配号段的方式实现高性能 ID 生成：

```rust
use nebulaid::core::algorithm::{AlgorithmBuilder, GenerateContext, IdAlgorithm};
use nebulaid::core::types::AlgorithmType;
use nebulaid::core::Config;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // datacenter_id 等在 Config.app 中配置（见 config/config.toml）
    let config = Config::default();

    // 通过 AlgorithmBuilder 构建 Segment 算法
    let segment = AlgorithmBuilder::new(AlgorithmType::Segment)
        .build(&config)
        .await?;

    // 生成单个 ID
    let ctx = GenerateContext::default();
    let id = segment.generate(&ctx).await?;
    println!("Generated ID: {}", id);

    // 批量生成（更高效，单次数据库交互）
    let batch = segment.batch_generate(&ctx, 100).await?;
    println!("Generated {} IDs", batch.ids.len());

    Ok(())
}
```

### Snowflake 算法

Snowflake 算法生成 64 位有序 ID，无需数据库协调：

```rust
use nebulaid::core::algorithm::{AlgorithmBuilder, GenerateContext, IdAlgorithm};
use nebulaid::core::types::AlgorithmType;
use nebulaid::core::Config;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // worker_id、datacenter_id 在 Config.app 中配置
    let config = Config::default();

    // 构建 Snowflake 算法（无需数据库协调，依赖时钟与 worker_id 唯一性）
    let snowflake = AlgorithmBuilder::new(AlgorithmType::Snowflake)
        .build(&config)
        .await?;

    // 生成 ID（Snowflake 与 Segment 共用 IdAlgorithm trait 接口）
    let ctx = GenerateContext::default();
    let id = snowflake.generate(&ctx).await?;
    println!("Generated ID: {}", id);

    // 批量生成
    let batch = snowflake.batch_generate(&ctx, 100).await?;
    println!("Generated {} IDs", batch.ids.len());

    Ok(())
}
```

### UUID 生成

支持时间有序的 UUID v8（RFC 9562 §5.8 自定义布局，内嵌 dc/worker/shard，严格单调递增）：

```rust
use nebulaid::core::algorithm::{AlgorithmBuilder, GenerateContext, IdAlgorithm};
use nebulaid::core::types::AlgorithmType;
use nebulaid::core::Config;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::default();

    // UUID v8 - 时间有序，适合数据库主键
    let v8 = AlgorithmBuilder::new(AlgorithmType::UuidV8)
        .build(&config)
        .await?;

    let ctx = GenerateContext::default();
    let uuid_v8 = v8.generate(&ctx).await?;
    println!("UUID v8: {}", uuid_v8);

    // 批量生成
    let batch = v8.batch_generate(&ctx, 100).await?;
    println!("Generated {} UUIDs", batch.ids.len());

    Ok(())
}
```

> 💡 **关于 uuid_v7 / uuid_v4**（add-uuid-v4-v7，2026-10）：两者曾是 `uuid_v8`
> 的输入别名（配置 `uuid_v7` 实际产出 v8）；现已反转为**真算法**——
> `"uuid_v7"` 解析为 `AlgorithmType::UuidV7`（标准时间有序 UUID）、
> `"uuid_v4"` 解析为 `AlgorithmType::UuidV4`（随机 UUID）。历史配置中的
> 这两个串的输出布局随之改变（存量串使用面实测为零，CHANGELOG 有破坏性
> 声明）。v8 的别名（`uuidv8` / `uuid8`）保持折叠。

#### UUID v5 确定性派生（非发号）

与 v8 的发号语义不同，v5 是 **name-based 确定性派生**（RFC 9562 §5.5）：
同一 `(namespace, name)` 恒得同一 ID，适用于按业务身份稳定派生的场景——
如词典实体按 `(lemma, language)` 派生（etyma 340,566 个存量 ID 即标准 v5
输出，可经 `derive_uuid_v5` 逐位复现，见仓内对拍测试）。

直调路径（`uuid-v5` 特性，default 开启）：

```rust
use nebulaid::core::algorithm::derive_uuid_v5;
use nebulaid::core::types::IdFormat;
use uuid::Uuid;

let namespace = Uuid::from_bytes([/* 调用方业务命名空间（16 字节） */]);
let id = derive_uuid_v5(namespace, "ecdict|abandon|en")?;
// canonical 连字符字符串必须经 render（Id 的 Display 版本白名单为
// {v4, v7, v8}，直接 to_string() 对 v5 会输出十进制数值）
println!("{}", id.render(&IdFormat::Uuid, None, None)?);
```

嵌入式 SDK 用户经统一入口调用（`sdk` 特性，薄包装同一实现）：

```rust
let id = kit
    .id_generator()?
    .derive_uuid_v5(namespace, "ecdict|abandon|en")?;
```

> 💡 v5 不经 `/generate` HTTP 端点与 `AlgorithmRouter`——派生失败没有
> 「换一个发号算法」的可动作语义（fallback 会产出数值错误的非 v5 ID）；
> 亦无配额/审计面（纯计算，无资源消耗决策）。
>
> 💡 **v5-only 嵌入走直调路径**：kit `build()` 要求 ≥1 个发号算法在场
> （`AlgorithmRouter::initialize` 在空注册表时显性报错），v5 不是路由
> 算法、不计数——只消费 v5 的嵌入方（如 `features = ["uuid-v5"]`）用
> `nebulaid::core::algorithm::derive_uuid_v5` 直调，无需装配 kit。
>
> 💡 **命名空间约定**：调用方应使用自己的业务命名空间（勿用 nil
> namespace——不同业务若共用 nil 且 name 空间相近，派生结果会互撞）；
> name 空间需自行保证全局唯一（v5 是身份派生，不是唯一性保证）。

#### UUID v4 / v7：发号器的另两个版本（add-uuid-v4-v7）

`AlgorithmType` 现在有**五个真算法**。UUID 四个版本的分工——按场景选，不按版本号新旧的选：

| 版本 | 独占能力 | 选它的场景 |
|---|---|---|
| `uuid_v4` | 122 位随机、完全零元数据（不泄漏创建时间与部署结构） | request id、能力令牌——「不可从 ID 反推任何信息」 |
| `uuid_v7` | 标准时间有序（RFC 9562 §5.7，时间戳可被任何标准工具直取）+ 零配置（无 dc/worker） | 数据库主键要索引局部性、又不想绑 NebulaId 私有布局 |
| `uuid_v8` | 私有布局内嵌 dc/worker/shard（租户上下文） | 多租户观测、分片路由、需要 `/parse` 解析拓扑 |
| v5 派生 | 同输入恒同输出 | 按业务身份（如 `(lemma, language)`）稳定派生 |

两个版本的用法与 v8 同构（工厂构建 / 配置 `algorithm.default` / HTTP `algorithm` 字段均可选）：

```rust
use nebulaid::core::algorithm::{AlgorithmBuilder, GenerateContext, IdAlgorithm};
use nebulaid::core::types::AlgorithmType;
use nebulaid::core::Config;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::default();

    // UUID v7 —— 标准时间有序，进程内严格保序
    let v7 = AlgorithmBuilder::new(AlgorithmType::UuidV7).build(&config).await?;
    // UUID v4 —— 随机零元数据
    let v4 = AlgorithmBuilder::new(AlgorithmType::UuidV4).build(&config).await?;

    let ctx = GenerateContext::default();
    println!("v7: {}", v7.generate(&ctx).await?);
    println!("v4: {}", v4.generate(&ctx).await?);
    Ok(())
}
```

> ⚠️ **破坏性变更提示（2026-10）**：历史配置/API 请求里的字符串
> `"uuid_v7"` / `"uuid_v4"` 此前是 `uuid_v8` 的输入别名（配 v7 得 v8），
> 现已反转为真算法——同名串的输出布局随之改变。v8 自身的别名
> （`uuidv8` / `uuid8`）不受影响。详见 CHANGELOG 破坏性条目。

---

## 高级用法

### 分布式协调

`Nebula ID` 支持多数据中心部署，实现负载均衡和故障转移：

```rust
use nebulaid::core::coordinator::{EtcdClientOps, EtcdClientWrapper, EtcdWorkerAllocator, WorkerIdAllocator};
use nebulaid::core::config::EtcdConfig;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. 创建 etcd 客户端（生产环境用 EtcdClientWrapper）
    let client: Arc<dyn EtcdClientOps> = Arc::new(
        EtcdClientWrapper::new(vec!["http://localhost:2379".into()]).await?
    );

    // 2. 通过 EtcdWorkerAllocator 在分布式环境中分配唯一 worker_id
    //    签名：new(client, datacenter_id, etcd_config) -> Result<Self, WorkerAllocatorError>
    let allocator = EtcdWorkerAllocator::new(
        client,
        1,                       // datacenter_id
        EtcdConfig::default(),
    )
    .await?;

    let worker_id = allocator.allocate().await?;
    println!("Allocated worker_id: {}", worker_id);

    // 3. worker_id 通常写入 Config.app.worker_id，由 main.rs 启动流程注入 AlgorithmBuilder。
    //    业务代码一般不直接调用 EtcdWorkerAllocator，而是消费 main.rs 启动后的 IdGenerator 服务。

    Ok(())
}
```

### 健康监控

实时监控数据中心健康状态：

```rust
use nebulaid::core::coordinator::{EtcdClusterHealthMonitor, EtcdClusterStatus};
use nebulaid::core::config::EtcdConfig;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 创建集群健康监控器
    // 签名：new(config: EtcdConfig, cache_file_path: String) -> Self
    // （etcd 不可用时降级到 cache_file_path 指向的本地缓存文件）
    let monitor = EtcdClusterHealthMonitor::new(
        EtcdConfig::default(),
        "etcd-cache.json".to_string(),
    );

    // 获取集群状态（同步方法，返回 EtcdClusterStatus 枚举）
    let status: EtcdClusterStatus = monitor.get_status();
    println!("Cluster status: {:?}", status);

    // 检查是否在 etcd 不可用时降级到了本地缓存
    if monitor.is_using_cache() {
        println!("⚠️ etcd 不可用，已降级到本地缓存");
    }

    Ok(())
}
```

### 性能优化

针对高并发场景的性能优化配置：

```rust
use nebulaid::core::algorithm::{AlgorithmBuilder, GenerateContext, IdAlgorithm};
use nebulaid::core::config::SegmentAlgorithmConfig;
use nebulaid::core::types::AlgorithmType;
use nebulaid::core::Config;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 通过 Config.algorithm.segment 调优 Segment 算法
    // 实际字段：base_step / min_step / max_step / switch_threshold
    // （动态步长算法根据 CPU 使用率在 [min_step, max_step] 之间自动切换）
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

    // 使用通道进行异步批量生成（batch_generate 一次拿一批，减少数据库交互）
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

### SDK 嵌入

Nebula ID 提供一等公民的嵌入式 SDK（feature `sdk`），无需自行拼装
分布式锁 / 路由 / 降级后台任务等装配细节，`NebulaIdKitBuilder::build()`
内部经 trait-kit 依赖图校验统一收拢。

启用特性：

```toml
[dependencies]
nebulaid = { version = "0.3", features = ["sdk"] }
```

**纯算法嵌入（零 DB 零网络）** —— 对照 `examples/embedded.rs`：

```rust
use nebulaid::core::Config;
use nebulaid::sdk::NebulaIdKitBuilder;

#[tokio::main]
async fn main() -> nebulaid::core::Result<()> {
    // 纯算法嵌入：默认算法设为 snowflake，避开需要数据库的 segment
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

> **注意**：Segment 算法需要数据库做号段分配。未通过
> `with_repository(...)` 注入仓储时，Segment 请求返回
> `CoreError::ConfigurationError`；Snowflake / UuidV8 为纯算法，零 DB 可用。

**封装为 Web 服务（sdforge `#[forge]`）** —— 对照 `examples/sdk_server.rs`：
用 `#[forge]` 把 `generate` / `batch_generate` 声明为 `POST /generate`、
`POST /generate/batch`，并自动产出 OpenAPI（`GET /api-docs/openapi.json`）。

```bash
cargo run --package nebulaid --example embedded   --features sdk
cargo run --package nebulaid --example sdk_server --features sdk,http
```

---

## 最佳实践

<div align="center">

### 🌟 推荐的设计模式

</div>

### ✅ 推荐做法

- **选择合适的算法**: 根据需求选择算法 - Segment 适合高并发、Snowflake 适合低延迟、UUID 适合分布式标识。
- **预配置号段大小**: 根据业务增长预估设置合理的 `base_step`（`SegmentAlgorithmConfig`），避免频繁数据库访问。
- **健康监控**: 生产环境务必启用健康监控，实现故障自动转移。
- **批量生成**: 对于批量操作（如数据导入），使用 `IdAlgorithm::batch_generate(&ctx, size)` 提高性能。
- **异步处理**: 高并发场景使用异步通道处理 ID 生成请求。

### ❌ 避免做法

- **单数据中心部署**: 生产环境应部署多个数据中心，避免单点故障。
- **忽略时钟回拨**: Snowflake 算法需要注意时钟同步，避免时钟回拨导致 ID 重复。
- **过小号段**: 号段过小会导致频繁的数据库访问，影响性能。
- **缺少监控**: 生产环境应监控 ID 生成延迟、错误率等指标。

---

## 故障排除

<details>
<summary><b>❓ 问题：ID 生成延迟过高</b></summary>

**解决方案：**
1. 检查 `base_step`（`SegmentAlgorithmConfig`）是否过小，增加号段步长以减少数据库访问。
2. 确认 `max_step` / `switch_threshold` 配置是否合理，让动态步长算法在高 CPU 使用率下自动放大步长。
3. 检查数据库连接池配置，确保有足够的连接数。
4. 考虑使用 `batch_generate(&ctx, size)` 一次性获取多个 ID，减少网络开销。

</details>

<details>
<summary><b>❓ 问题：数据中心故障转移不生效</b></summary>

**解决方案：**
1. 检查健康监控配置，确保正确启用了心跳检测。
2. 确认所有数据中心都已正确注册到协调器。
3. 检查故障检测阈值设置，确保在合理范围内。
4. 验证网络连通性，确保跨数据中心通信正常。

</details>

<details>
<summary><b>❓ 问题：Snowflake ID 重复</b></summary>

**解决方案：**
1. 检查系统时钟是否发生回拨，使用 `ntpdate` 同步时间。
2. 确认 `datacenter_id` 和 `worker_id` 在同一集群内唯一。
3. 检查时间戳获取逻辑，确保单调递增。
4. 启用时钟回拨保护机制。

</details>

<details>
<summary><b>❓ 问题：数据库连接池耗尽</b></summary>

**解决方案：**
1. 增加数据库连接池大小配置。
2. 适当放大 `base_step` / `max_step`，让单次号段加载支持更多 ID 请求，降低并发访问压力。
3. 使用连接池复用技术，避免频繁创建连接。
4. 考虑使用读写分离，将 Segment 加载指向从库。

</details>

<div align="center">

**💬 仍然需要帮助？** [提交 Issue](https://github.com/Kirky-X/NebulaId/issues) 或 [访问文档中心](https://github.com/Kirky-X/NebulaId)

</div>

---

## 后续步骤

<div align="center">

### 🎯 继续探索

</div>

<table>
<tr>
<td width="33%" align="center">
<a href="API_REFERENCE.md">
<img src="https://img.icons8.com/fluency/96/000000/graduation-cap.png" width="64"><br>
<b>📚 API 参考</b>
</a><br>
详细的接口说明
</td>
<td width="33%" align="center">
<a href="ARCHITECTURE.md">
<img src="https://img.icons8.com/fluency/96/000000/settings.png" width="64"><br>
<b>🔧 架构设计</b>
</a><br>
深入了解内部机制
</td>
<td width="33%" align="center">
<a href="../examples/">
<img src="https://img.icons8.com/fluency/96/000000/code.png" width="64"><br>
<b>💻 示例代码</b>
</a><br>
真实场景的代码样例
</td>
</tr>
</table>

---

<div align="center">

**[📖 API 文档](https://docs.rs/nebulaid)** • **[❓ 常见问题](FAQ.md)** • **[🐛 报告问题](https://github.com/Kirky-X/NebulaId/issues)**

由 Kirky.X 用 ❤️ 制作

[⬆ 回到顶部](#-用户指南)

</div>
