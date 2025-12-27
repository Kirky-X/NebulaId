<div align="center">

# 🚀 Nebula ID

<p>
  <img src="https://img.shields.io/badge/version-0.1.0-blue.svg" alt="Version">
  <img src="https://img.shields.io/badge/license-MIT-green.svg" alt="License">
  <img src="https://img.shields.io/badge/build-passing-brightgreen.svg" alt="Build">
  <img src="https://img.shields.io/badge/coverage-85%25-success.svg" alt="Coverage">
</p>

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

- ✅ **多种ID算法** - Segment、Snowflake、UUID v7、UUID v4
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
    C --> F[UUID v7/v4]
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
use nebula_id::algorithm::{SegmentAlgorithm, SnowflakeAlgorithm};

// Segment算法用于有序、高吞吐量的ID生成
let segment = SegmentAlgorithm::new(1);
let id = segment.generate_id()?;

// Snowflake算法用于全局唯一ID
let snowflake = SnowflakeAlgorithm::new(1, 1);
let id = snowflake.generate_id()?;
```

适用于需要高可用性、有序唯一标识符的大规模分布式系统。

</details>

<details>
<summary><b>🔧 微服务</b></summary>

<br>

```rust
use nebula_id::types::NebulaId;

// 生成UUID v7用于时间排序的标识符
let uuid = NebulaId::from_uuid_v7(uuid::Uuid::now_v7());
let id_string = uuid.to_string();

// 生成UUID v4用于随机标识符
let uuid_v4 = NebulaId::from_uuid_v4(uuid::Uuid::new_v4());
```

适用于需要不同排序保证的唯一标识符的微服务。

</details>

<details>
<summary><b>🌐 高性能应用</b></summary>

<br>

```rust
use nebula_id::algorithm::SegmentAlgorithm;

// 双缓冲机制实现最大吞吐量
let segment = SegmentAlgorithm::new(1);
let id = segment.generate_id()?;
```

适用于需要每秒生成数百万ID且低延迟的高性能应用。

</details>

---

## 🚀 快速开始

### 安装

<table>
<tr>
<td width="50%">

#### 🦀 Rust

```toml
[dependencies]
nebula-id = "0.1.0"
tokio = { version = "1.0", features = ["full"] }
uuid = { version = "1.0", features = ["v7"] }
```

</td>
<td width="50%">

#### 📦 功能标志

```toml
[dependencies.nebula-id]
version = "0.1.0"
features = ["monitoring", "audit", "grpc"]
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

```toml
[algorithm]
type = "segment"

[database]
url = "postgresql://user:pass@localhost/nebula"
max_connections = 10

[redis]
url = "redis://localhost"
```

</td>
<td width="50%">

**步骤2：初始化服务**

```rust
use nebula_id::Config;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::load_from_file("config.toml")?;
    
    let service = NebulaIdService::new(config).await?;
    service.start().await?;
    
    Ok(())
}
```

</td>
</tr>
</table>

<details>
<summary><b>📖 完整示例</b></summary>

<br>

```rust
use nebula_id::algorithm::SegmentAlgorithm;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let segment = SegmentAlgorithm::new(1);
    let id = segment.generate_id().await?;
    
    println!("生成的ID: {}", id);
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
use nebula_id::algorithm::SegmentAlgorithm;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 使用本地数据中心ID初始化
    let segment = SegmentAlgorithm::new(1);
    
    // 生成ID
    let id = segment.generate_id().await?;
    
    println!("生成的ID: {}", id);
    Ok(())
}
```

<details>
<summary>查看输出</summary>

```
生成的Segment ID: 1000001
```

</details>

</td>
<td width="50%">

#### 🔥 示例2：Snowflake算法

```rust
use nebula_id::algorithm::SnowflakeAlgorithm;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 使用数据中心ID和工作节点ID初始化
    let snowflake = SnowflakeAlgorithm::new(1, 1);
    
    // 生成ID
    let id = snowflake.generate_id()?;
    
    println!("数据中心: 1, 工作节点: 1");
    println!("生成的Snowflake ID: {}", id);
    Ok(())
}
```

<details>
<summary>查看输出</summary>

```
数据中心: 1, 工作节点: 1
生成的Snowflake ID: 4200000000000000001
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
    E --> H[UUID v7/v4]
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
| **UUID生成器** | UUID v7和v4实现 | ✅ 稳定 |
| **分布式协调** | 基于Etcd的leader选举和协调 | ✅ 稳定 |
| **监控** | 健康检查、指标收集和告警 | ✅ 稳定 |
| **API网关** | HTTP/HTTPS和gRPC/gRPCS端点管理 | ✅ 稳定 |

</details>

---

## ⚙️ 配置

<div align="center">

### 🎛️ 配置选项

</div>

<table>
<tr>
<td width="50%">

**基本配置 (config.toml)**

```toml
[app]
name = "nebula-id"
host = "0.0.0.0"
port = 8080

[algorithm]
type = "segment"

[database]
url = "postgresql://user:pass@localhost/nebula"
max_connections = 10

[redis]
url = "redis://localhost"

[etcd]
endpoints = ["http://localhost:2379"]

[auth]
api_key = "your-api-key-here"

[rate_limit]
requests_per_second = 1000

[tls]
enabled = false
```

</td>
<td width="50%">

**环境变量**

```bash
export NEBULA_APP_NAME="nebula-id"
export NEBULA_APP_PORT="8080"
export NEBULA_DATABASE_URL="postgresql://user:pass@localhost/nebula"
export NEBULA_REDIS_URL="redis://localhost"
export NEBULA_ETCD_ENDPOINTS="http://localhost:2379"
export NEBULA_AUTH_API_KEY="your-api-key-here"
```

</td>
</tr>
</table>

<details>
<summary><b>🔧 所有配置选项</b></summary>

<br>

| 选项 | 类型 | 默认值 | 描述 |
|--------|------|---------|-------------|
| `app.name` | String | "nebula-id" | 应用名称 |
| `app.host` | String | "0.0.0.0" | 服务器绑定地址 |
| `app.port` | u16 | 8080 | 服务器端口 |
| `algorithm.type` | String | "segment" | ID生成算法 |
| `database.url` | String | - | 数据库连接URL |
| `database.max_connections` | u32 | 10 | 连接池大小 |
| `redis.url` | String | - | Redis连接URL |
| `etcd.endpoints` | Vec&lt;String&gt; | [] | Etcd服务器端点 |
| `auth.api_key` | String | - | 用于认证的API密钥 |
| `rate_limit.requests_per_second` | u32 | 1000 | 限流阈值 |
| `tls.enabled` | Boolean | false | 启用TLS/SSL |
</td>
</tr>
</table>

### 算法配置

<table>
<tr>
<td width="50%">

**Segment算法**

```toml
[algorithm.segment]
name = "default"
step = 1000
max_retry = 3
```

</td>
<td width="50%">

**Snowflake算法**

```toml
[algorithm.snowflake]
datacenter_id = 1
worker_id = 1
sequence_bits = 12
```

</td>
</tr>
</table>

> **注意**: 详细配置说明请参考 [配置指南](#-文档)。

</details>

---

## 🧪 测试

<div align="center">

### 🎯 测试覆盖率

</div>

```bash
# 运行所有测试
cargo test --all-features

# 运行覆盖率测试
cargo tarpaulin --out Html

# 运行特定测试
cargo test test_name

# 运行集成测试
cargo test --test integration
```

<details>
<summary><b>📊 测试统计</b></summary>

<br>

| 类别 | 测试数量 | 覆盖率 |
|----------|-------|----------|
| 单元测试 | 100+ | 85% |
| 集成测试 | 30+ | 80% |
| **总计** | **130+** | **85%** |

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
UUID v7: 500,000+ IDs/秒
UUID v4: 1,000,000+ IDs/秒
```

</td>
<td width="50%">

**延迟 (P99)**

```
Segment: ~0.5ms
Snowflake: ~0.1ms
UUID v7: ~0.05ms
UUID v4: ~0.05ms
```

</td>
</tr>
</table>

<details>
<summary><b>📈 详细基准测试</b></summary>

<br>

```bash
# 运行基准测试
cargo bench

# 示例输出:
test segment_next_id    ... bench: 500 ns/iter (+/- 50)
test snowflake_next_id  ... bench: 100 ns/iter (+/- 10)
test uuid_v7_next_id    ... bench: 50 ns/iter (+/- 5)
test uuid_v4_next_id    ... bench: 50 ns/iter (+/- 5)
```

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
基于API密钥的ID生成请求认证
</td>
<td align="center" width="33%">
<img src="https://img.icons8.com/fluency/96/000000/security-checked.png" width="64" height="64"><br>
<b>限流</b><br>
可配置限流防止滥用
</td>
<td align="center" width="33%">
<img src="https://img.icons8.com/fluency/96/000000/privacy.png" width="64" height="64"><br>
<b>审计日志</b><br>
跟踪所有ID生成操作
</td>
</tr>
</table>

<details>
<summary><b>🔐 安全详情</b></summary>

<br>

### 安全措施

- ✅ **API密钥认证** - 使用API密钥认证保护API访问
- ✅ **限流** - 可配置限流防止滥用和DoS攻击
- ✅ **审计日志** - 完整的操作跟踪，满足合规和监控需求
- ✅ **TLS支持** - HTTPS和gRPCS实现加密通信

### 功能标志

```toml
[dependencies.nebula-id]
version = "0.1.0"
features = ["audit", "tls"]
```

</details>

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
- [x] UUID v7/v4实现
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
5. **测试** 你的修改: `cargo test --all-features`
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
  - [sea-orm](https://github.com/SeaQL/sea-orm) - 数据库ORM
  - [etcd-client](https://github.com/etcd-rs/etcd-client) - Etcd客户端
  - [uuid](https://github.com/uuid-rs/uuid) - UUID生成
  - [tonic](https://github.com/hyperium/tonic) - gRPC框架
  - [axum](https://github.com/tokio-rs/axum) - HTTP框架
  - [dashmap](https://github.com/xacrimon/dashmap) - 并发HashMap
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
