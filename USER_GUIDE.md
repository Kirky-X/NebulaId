<div align="center">

# 📖 User Guide

### 完整的 Nebula ID 使用指南

[🏠 首页](README.md) • [📚 文档](README.md) • [🎯 示例](../examples/) • [❓ 常见问题](FAQ.md)

---

</div>

## 📋 目录

- [简介](#简介)
- [快速入门](#快速入门)
  - [先决条件](#先决条件)
  - [安装](#安装)
  - [第一步](#第一步)
- [核心概念](#核心概念)
- [基础用法](#基础用法)
  - [Segment 算法](#segment-算法)
  - [Snowflake 算法](#snowflake-算法)
  - [UUID 生成](#uuid-生成)
- [高级用法](#高级用法)
  - [分布式协调](#分布式协调)
  - [健康监控](#健康监控)
  - [性能优化](#性能优化)
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

**Nebula ID** 是一个功能强大的企业级分布式 ID 生成系统，提供多种高性能、高可用的 ID 生成算法，包括 Segment（号段）、Snowflake（雪花）以及标准 UUID v7/v4 实现。它专为分布式系统设计，支持数据中心健康监控、故障自动转移和毫秒级延迟。

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
- 🔧 PostgreSQL/MySQL (用于 Segment 算法持久化)

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

在你的 `Cargo.toml` 中添加 `nebula-id`：

```toml
[dependencies]
nebula-id = "0.1"

# 如果需要 Segment 算法持久化
nebula-id = { version = "0.1", features = ["database-postgres"] }

# 如果需要 HTTP/gRPC 服务
nebula-id = { version = "0.1", features = ["server"] }

# 完整功能
nebula-id = { version = "0.1", features = ["full"] }
```

或者使用命令行：

```bash
cargo add nebula-id
```

### 第一步

让我们通过一个简单的例子来验证安装。我们将使用 Segment 算法生成分布式 ID：

```rust
use nebula_id::algorithm::SegmentAlgorithm;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 使用本地数据中心 ID 初始化
    let segment = SegmentAlgorithm::new(1);
    
    // 生成 ID
    let id = segment.generate_id().await?;
    
    println!("Generated ID: {}", id);
    Ok(())
}
```

---

## 核心概念

理解这些核心概念将帮助你更有效地使用 `Nebula ID`。

### 1️⃣ ID 生成算法

`Nebula ID` 提供三种核心算法：
- **Segment (号段)**: 基于数据库的号段分配，支持高并发批量获取
- **Snowflake (雪花)**: Twitter 风格的分布式 ID，时间有序、无需协调
- **UUID (通用唯一标识符)**: 标准 UUID v7/v4 实现，符合 RFC 4122

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

## 基础用法

### Segment 算法

Segment 算法通过预分配号段的方式实现高性能 ID 生成：

```rust
use nebula_id::algorithm::SegmentAlgorithm;
use nebula_id::config::SegmentAlgorithmConfig;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 自定义配置
    let config = SegmentAlgorithmConfig {
        segment_size: 1000,        // 每个号段的大小
        preload_count: 5,          // 预加载号段数量
        ..Default::default()
    };
    
    // 初始化（使用本地 DCID）
    let segment = SegmentAlgorithm::with_config(1, config);
    
    // 生成 ID
    let id = segment.generate_id().await?;
    println!("Generated ID: {}", id);
    
    // 批量生成（更高效）
    let ids = segment.generate_batch(100).await?;
    println!("Generated {} IDs", ids.len());
    
    Ok(())
}
```

### Snowflake 算法

Snowflake 算法生成 64 位有序 ID，无需数据库协调：

```rust
use nebula_id::algorithm::SnowflakeAlgorithm;
use nebula_id::config::SnowflakeAlgorithmConfig;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 自定义配置
    let config = SnowflakeAlgorithmConfig {
        worker_id: 1,
        datacenter_id: 1,
        ..Default::default()
    };
    
    // 初始化
    let snowflake = SnowflakeAlgorithm::with_config(config);
    
    // 生成 ID
    let id = snowflake.generate_id()?;
    println!("Generated ID: {}", id);
    
    // 批量生成
    let ids = snowflake.generate_batch(100)?;
    println!("Generated {} IDs", ids.len());
    
    Ok(())
}
```

### UUID 生成

支持标准 UUID v7（时间有序）和 v4（完全随机）：

```rust
use nebula_id::algorithm::UuidV7Algorithm;
use nebula_id::algorithm::UuidV4Algorithm;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // UUID v7 - 时间有序，适合数据库主键
    let v7 = UuidV7Algorithm::new();
    let uuid_v7 = v7.generate_id().await?;
    println!("UUID v7: {}", uuid_v7);
    
    // UUID v4 - 完全随机，适合分布式标识
    let v4 = UuidV4Algorithm::new();
    let uuid_v4 = v4.generate_id().await?;
    println!("UUID v4: {}", uuid_v4);
    
    // 批量生成
    let batch = v7.generate_batch(100).await?;
    println!("Generated {} UUIDs", batch.len());
    
    Ok(())
}
```

---

## 高级用法

### 分布式协调

`Nebula ID` 支持多数据中心部署，实现负载均衡和故障转移：

```rust
use nebula_id::algorithm::SegmentAlgorithm;
use nebula_id::coordinator::DatacenterCoordinator;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 创建数据中心协调器
    let coordinator = Arc::new(DatacenterCoordinator::new());
    
    // 注册多个数据中心
    coordinator.register_dc(1, "primary".to_string()).await?;
    coordinator.register_dc(2, "secondary".to_string()).await?;
    coordinator.register_dc(3, "tertiary".to_string()).await?;
    
    // 创建 Segment 算法实例
    let segment = SegmentAlgorithm::with_coordinator(1, coordinator);
    
    // 生成 ID（自动负载均衡）
    let id = segment.generate_id().await?;
    
    Ok(())
}
```

### 健康监控

实时监控数据中心健康状态：

```rust
use nebula_id::algorithm::SegmentAlgorithm;
use nebula_id::health::DcHealthMonitor;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 创建健康监控器
    let monitor = DcHealthMonitor::new();
    
    // 添加数据中心
    monitor.add_dc(1);
    monitor.add_dc(2);
    monitor.add_dc(3);
    
    // 定期检查健康状态
    loop {
        let health_status = monitor.get_all_health_status().await;
        
        for (dc_id, status) in health_status {
            println!("DC {}: {:?}", dc_id, status);
        }
        
        tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
    }
}
```

### 性能优化

针对高并发场景的性能优化配置：

```rust
use nebula_id::algorithm::SegmentAlgorithm;
use nebula_id::config::SegmentAlgorithmConfig;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 优化配置
    let config = SegmentAlgorithmConfig {
        segment_size: 10000,      // 增大号段大小，减少数据库访问
        preload_count: 10,        // 增加预加载数量
        buffer_capacity: 1000,    // 增加缓冲区容量
        async_preload: true,      // 启用异步预加载
        ..Default::default()
    };
    
    // 初始化
    let segment = SegmentAlgorithm::with_config(1, config);
    
    // 使用通道进行异步生成
    let (tx, rx) = tokio::sync::mpsc::channel(1000);
    
    // 在后台任务中生成 ID
    let handle = tokio::spawn(async move {
        for _ in 0..10000 {
            let id = segment.generate_id().await?;
            tx.send(id).await?;
        }
        Ok::<_, Box<dyn std::error::Error>>(())
    });
    
    // 收集结果
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

## 最佳实践

<div align="center">

### 🌟 推荐的设计模式

</div>

### ✅ 推荐做法

- **选择合适的算法**: 根据需求选择算法 - Segment 适合高并发、Snowflake 适合低延迟、UUID 适合分布式标识。
- **预配置号段大小**: 根据业务增长预估设置合理的 `segment_size`，避免频繁数据库访问。
- **健康监控**: 生产环境务必启用健康监控，实现故障自动转移。
- **批量生成**: 对于批量操作（如数据导入），使用 `generate_batch` 提高性能。
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
1. 检查 `segment_size` 是否过小，增加号段大小减少数据库访问。
2. 确认 `preload_count` 设置是否合理，增加预加载数量。
3. 检查数据库连接池配置，确保有足够的连接数。
4. 考虑使用本地缓存减少网络开销。

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
2. 减少 `segment_size` 以降低并发访问压力。
3. 使用连接池复用技术，避免频繁创建连接。
4. 考虑使用读写分离，将 Segment 加载指向从库。

</details>

<div align="center">

**💬 仍然需要帮助？** [提交 Issue](../../issues) 或 [访问文档中心](https://github.com/nebula-id/nebula-id)

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

**[📖 API 文档](https://docs.rs/nebula-id)** • **[❓ 常见问题](FAQ.md)** • **[🐛 报告问题](../../issues)**

由 Nebula ID Team 用 ❤️ 制作

[⬆ 回到顶部](#-用户指南)

</div>
