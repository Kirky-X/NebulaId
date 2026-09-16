<div align="center">

# 🚀 Nebula ID

[![GitHub release](https://img.shields.io/github/v/release/Kirky-X/NebulaId)](https://github.com/Kirky-X/NebulaId/releases) [![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-green)](./LICENSE) [![CI](https://img.shields.io/github/actions/workflow/status/Kirky-X/NebulaId/ci.yml?branch=main)](https://github.com/Kirky-X/NebulaId/actions/workflows/ci.yml) [![Security](https://img.shields.io/github/actions/workflow/status/Kirky-X/NebulaId/codeql.yml?branch=main&label=security)](https://github.com/Kirky-X/NebulaId/actions/workflows/codeql.yml)

**中文** | [English](README_EN.md)

**企业级高性能分布式 ID 生成系统**

[✨ 功能特性](#-功能特性) • [🚀 快速开始](#-快速开始) • [📚 文档](#-文档) • [💻 示例](#-示例) • [🤝 参与贡献](#-参与贡献)

</div>

---

<div align="center">

### 🎯 一套服务，三种发号算法

号段双缓冲、位切片防回拨、UUID v8 自定义布局，HTTP 与 gRPC 同源直达：

<table style="width:100%; border-collapse: collapse">
<tr>
<td align="center" width="25%">🧮<br><b>Segment 号段</b><br><span style="color:#64748B">双缓冲 · 动态步长</span></td>
<td align="center" width="25%">❄️<br><b>Snowflake</b><br><span style="color:#64748B">位切片 · 抗时钟回拨</span></td>
<td align="center" width="25%">🧬<br><b>UUID v8</b><br><span style="color:#64748B">自定义位布局 · 趋势有序</span></td>
<td align="center" width="25%">🔐<br><b>API 安全</b><br><span style="color:#64748B">密钥认证 · 限流审计</span></td>
</tr>
</table>

</div>

---

## 📋 目录

- [✨ 功能特性](#-功能特性)
- [🎯 使用场景](#-使用场景)
- [🚀 快速开始](#-快速开始)
- [📚 文档](#-文档)
- [💻 示例](#-示例)
- [🏗️ 架构](#️-架构)
- [⚙️ 配置](#️-配置)
- [🌐 国际化](#-国际化)
- [🛠️ scripts/run.sh 用法](#️-scriptsrunsh-用法)
- [🧪 测试](#-测试)
- [📊 性能](#-性能)
- [🔒 安全](#-安全)
- [🗺️ 开发路线图](#️-开发路线图)
- [🤝 参与贡献](#-参与贡献)
- [📋 更新日志](#-更新日志)
- [📄 许可证](#-许可证)
- [🙏 致谢](#-致谢)
- [📞 联系与支持](#-联系与支持)
- [⭐ Star 历史](#-star-历史)

---

## ✨ 功能特性

<table style="width:100%; border-collapse: collapse">
<tr>
<td width="50%" style="vertical-align:top; padding: 12px">🔢 <b>多算法引擎</b><br><span style="color:#64748B">Segment 号段（双缓冲 + 动态步长）、Snowflake（位布局可配 + 时钟回拨防护）、UUID v8（RFC 9562 §5.8），运行时按 <code>workspace / group / biz_tag</code> 路由</span></td>
<td width="50%" style="vertical-align:top; padding: 12px">🌐 <b>双协议接入</b><br><span style="color:#64748B">HTTP/HTTPS REST 与 gRPC/gRPCS 共用同一算法路由，sdforge <code>#[forge]</code> 自动产出 OpenAPI 文档</span></td>
</tr>
<tr>
<td width="50%" style="vertical-align:top; padding: 12px">🧩 <b>嵌入式 SDK</b><br><span style="color:#64748B"><code>sdk</code> feature 提供 <code>NebulaIdKit</code>：trait-kit AsyncKit 依赖图装配，缺失依赖在 <code>build()</code> 期报错；<code>embedded</code> 示例零 DB 零网络运行</span></td>
<td width="50%" style="vertical-align:top; padding: 12px">🏗️ <b>分布式协调</b><br><span style="color:#64748B"><code>etcd</code> feature 提供工作器分配与协调，端点为空时自动退回进程内本地锁</span></td>
</tr>
<tr>
<td width="50%" style="vertical-align:top; padding: 12px">🔐 <b>garrison 认证</b><br><span style="color:#64748B"><code>garrison-auth</code> feature 接管 API key 验证：Argon2id 哈希、常量时间比较、进程内缓存、轮换宽限期</span></td>
<td width="50%" style="vertical-align:top; padding: 12px">🚦 <b>全局限流</b><br><span style="color:#64748B">令牌桶真实挂载 HTTP 栈，<code>burst ≤ 10 × rps</code> 启动校验，<code>x-ratelimit-*</code> 响应头，管理接口热更新</span></td>
</tr>
<tr>
<td width="50%" style="vertical-align:top; padding: 12px">📊 <b>可观测性</b><br><span style="color:#64748B">Prometheus <code>/metrics</code> 逐算法暴露 p50/p99/p999 与 <code>clock_backwards</code>，OTLP tracing，健康检查端点</span></td>
<td width="50%" style="vertical-align:top; padding: 12px">🌍 <b>ICU 国际化</b><br><span style="color:#64748B"><code>rust-i18n</code> + <code>Accept-Language</code> 协商（RFC 7231 §5.3.5），<code>en</code> 与 <code>zh-CN</code> 全量错误文案与日志</span></td>
</tr>
<tr>
<td width="50%" style="vertical-align:top; padding: 12px">🛡️ <b>传输与响应安全</b><br><span style="color:#64748B">rustls TLS 1.2/1.3（<code>min_tls_version</code> 强制）、安全响应头、严格 CORS、可信代理 IP 归属</span></td>
<td width="50%" style="vertical-align:top; padding: 12px">🧾 <b>审计日志</b><br><span style="color:#64748B">ID 生成与密钥操作全量审计，客户端 IP 解析到真实对端连接地址</span></td>
</tr>
<tr>
<td width="50%" style="vertical-align:top; padding: 12px">⚙️ <b>配置 fail-fast</b><br><span style="color:#64748B">17 个配置结构体全部 <code>deny_unknown_fields</code>，坏配置以退出码 1 终止启动；<code>${VAR}</code> 环境变量展开</span></td>
<td width="50%" style="vertical-align:top; padding: 12px">🧰 <b>统一脚本入口</b><br><span style="color:#64748B"><code>scripts/run.sh</code> 调度 deploy / lint / redis-test / api-test / install-hooks，本地与 CI 同源</span></td>
</tr>
</table>

除上述核心能力外，PostgreSQL 之外的引擎声明（`sqlite` 当前不可构建）、需要真实数据库的 `integration-tests` 门控、热重载监听与 Redis 缓存等也以配置或 feature 形式提供；逐项说明见 [⚙️ 配置](#️-配置) 一节与 [配置迁移指南](docs/CONFIG_MIGRATION_GUIDE.md)。

---

## 🎯 使用场景

### 💼 嵌入既有 Rust 服务

把发号能力作为库内嵌，无需独立部署（改编自 [`examples/embedded.rs`](examples/embedded.rs)）：

```rust
use nebulaid::core::Config;
use nebulaid::sdk::NebulaIdKitBuilder; // feature `sdk`

#[tokio::main]
async fn main() -> nebulaid::core::Result<()> {
    let mut config = Config::default();
    config.algorithm.default = "snowflake".to_string();

    let kit = NebulaIdKitBuilder::new(config).build().await?;
    let generator = kit.id_generator()?;

    // 默认算法（`config.algorithm.default`）
    let id = generator.generate("prod", "core", "order").await?;

    // 或按次指定算法
    let uuid = generator
        .generate_with_algorithm(nebulaid::core::types::AlgorithmType::UuidV8,
                                 "prod", "core", "trace")
        .await?;

    println!("snowflake={id} uuid_v8={uuid}");
    kit.shutdown().await;
    Ok(())
}
```

### 🔧 微服务唯一标识

`Id` 是对 UUID 的零成本包装，可无损互转，适合跨服务传递与存储：

```rust
use nebulaid::core::types::Id;
use uuid::Uuid;

// 任意 Uuid 都用同一构造函数包装为 Nebula `Id`（仅有 `from_uuid_v8`）
let id = Id::from_uuid_v8(Uuid::now_v7());
let id_string = id.to_string(); // 输出标准 36 字符 UUID 字符串

// 可无损转回 Uuid
let uuid = id.to_uuid_v8();
```

### ⚡ 高吞吐批量发号

Segment 的双缓冲是内部机制，对外一次调用申请 N 个 ID（`IdAlgorithm::batch_generate`）：

```rust
let batch = generator.batch_generate("prod", "core", "order", 1000).await?;
println!("{} ids via {:?}", batch.len(), batch.algorithm);
```

---

## 🚀 快速开始

### 📦 安装

```bash
# 克隆仓库并构建（default 特性：postgresql + http + grpc + garrison-auth）
git clone https://github.com/Kirky-X/NebulaId.git
cd NebulaId
cargo build --release

# 运行服务（默认读取 config/config.toml，--config 可指定路径）
./target/release/nebula-id
```

| feature | 默认 | 说明 |
|---------|:----:|------|
| `postgresql` | ✅ | dbnexus PostgreSQL 存储后端 |
| `http` / `grpc` | ✅ | REST 与 gRPC 接入（sdforge 镜像 feature） |
| `garrison-auth` | ✅ | garrison 接管 API key 验证 |
| `etcd` | ➖ | etcd 分布式协调（可构建的最大特性集为 default + etcd） |
| `sdk` | ➖ | 嵌入式 SDK facade（`NebulaIdKit`，蕴含 `openapi`） |
| `integration-tests` | ➖ | 门控需要真实数据库的 `#[ignore]` 测试 |
| `sqlite` | ➖ | 保留定义但**当前不可构建**：default 恒含 dbnexus/postgres，叠加 sqlite 会触发 dbnexus 的 compile_error |

> ⚠️ 本项目**不存在可用的「全特性」构建**（原因见上表 sqlite 行），CI 与文档均不推荐该开关。

### 💡 最小示例

以下示例改编自 [`examples/embedded.rs`](examples/embedded.rs)，零 DB 零网络即可生成 ID：

```rust
use nebulaid::core::Config;
use nebulaid::sdk::NebulaIdKitBuilder;

#[tokio::main]
async fn main() -> nebulaid::core::Result<()> {
    // 纯算法（snowflake/uuid_v8）用 Config::default() 即可；
    // segment 需 NebulaIdKitBuilder::with_repository(..) 注入仓储。
    let mut config = Config::default();
    config.algorithm.default = "snowflake".to_string();

    let kit = NebulaIdKitBuilder::new(config).build().await?;
    let generator = kit.id_generator()?;

    for _ in 0..5 {
        let id = generator.generate("embedded", "demo", "order").await?;
        println!("Generated ID: {id}");
    }

    kit.shutdown().await;
    Ok(())
}
```

```bash
cargo run --package nebulaid --example embedded --features sdk
```

### 🧭 核心概念

- **三元组命名空间**：`generate(workspace_id, group_id, biz_tag)`，workspace 即租户隔离边界（biz-tags 查询按角色过滤）。
- **算法选择**：`[algorithm].default` 全局默认，`generate_with_algorithm` 按次覆盖；纯算法（snowflake/uuid_v8）零 DB 可用，segment 必须注入仓储。
- **双协议同源**：HTTP 与 gRPC 共用同一算法路由与认证中间件；未带凭证的 gRPC 请求收到 `Unauthenticated`。
- **Kit 装配**：SDK 经 trait-kit AsyncKit 依赖图校验（缺失依赖/环在 `build()` 期报错），`IdGenerator` handle 可 `Clone` 后跨任务共享。
- **降级兜底**：主算法不可用时按 `[Snowflake, UuidV8]` 降级链继续发号。

---

## 📚 文档

| 文档 | 说明 |
|------|------|
| [📖 用户指南](docs/USER_GUIDE.md) | 从安装到进阶的完整使用教程 |
| [📘 API 参考](docs/API_REFERENCE.md) | HTTP / gRPC 端点、请求头、错误码与类型定义 |
| [🏗️ 架构文档](docs/ARCHITECTURE.md) | 模块依赖、外部库角色与算法优化设计 |
| [🔧 配置迁移指南](docs/CONFIG_MIGRATION_GUIDE.md) | 配置全表、校验规则与版本间迁移操作 |
| [🚀 部署指南](docs/DEPLOYMENT.md) | Docker 部署、环境变量、监控与脚本子命令 |
| [📈 性能指南](docs/PERFORMANCE.md) | 基准口径、热路径设计与优化建议 |
| [🔒 安全文档](docs/SECURITY.md) | 安全设计、供应链门禁与漏洞处理流程 |
| [🧪 测试场景矩阵](docs/TEST_SCENARIOS.md) | 分层测试策略与场景穷举 |
| [❓ FAQ](docs/FAQ.md) | 常见问题解答 |
| [📋 更新日志](docs/CHANGELOG.md) | 每个版本的变更记录 |
| [🤝 贡献指南](docs/CONTRIBUTING.md) | 如何参与项目开发 |

---

## 💻 示例

全部示例位于 [`examples/`](examples/) 目录，仅启用 `sdk` 特性时构建：

```bash
# 嵌入式 SDK：零 DB 零网络生成 ID
cargo run --package nebulaid --example embedded --features sdk

# SDK 服务器：sdforge #[forge] 封装 + OpenAPI 文档
cargo run --package nebulaid --example sdk_server --features sdk,http
```

### 🧩 embedded.rs

`NebulaIdKitBuilder` 装配 → `id_generator()` 取句柄 → `generate` / `batch_generate` 发号 → `shutdown` 优雅停机。适合把发号能力直接内嵌进既有 Rust 服务。

### 🚀 sdk_server.rs

同一个 Kit 经 sdforge `#[forge]` 暴露为 HTTP 服务并注册 OpenAPI 路由（`/api-docs/openapi.json`），适合以「库 → 服务」一行切换的方式验证双协议接入。

---

## 🏗️ 架构

Nebula ID 采用「server → core → 自研生态」三层设计：`src/server/` 承载 HTTP（axum）与 gRPC（tonic）端点、中间件（认证、限流、CORS、locale、安全头）与 handlers；`src/core/` 包含算法路由器与三种算法实现、数据库仓储、etcd 协调器、监控与 i18n；数据库、配置、日志、缓存、限流、服务发现与认证分别委托给同作者生态 `dbnexus` / `confers` / `inklog` / `oxcache` / `limiteron` / `sdforge` / `garrison` / `trait-kit`。Segment 以双缓冲 + 动态步长从数据库领取号段，Snowflake 串行化 `(last_timestamp, sequence)` 迁移防并发重复，主算法不可用时按降级链兜底。

架构图、模块依赖关系、外部库角色、`mod.rs` 接口隔离标准（规则 25）与 trait 关系图的完整说明见 [🏗️ 架构文档](docs/ARCHITECTURE.md)。

---

## ⚙️ 配置

`Config` 覆盖 `app`、`database`、`etcd`、`auth`、`algorithm`、`monitoring`、`logging`、`rate_limit`、`tls`、`batch_generate` 十个段，全部**必填**（仅 `[redis]` 与 `[hot_reload]` 可整体省略）；17 个配置结构体均带 `#[serde(deny_unknown_fields)]`，未知键与缺失必填键同样导致整份文件解析失败、进程以退出码 1 终止。环境变量有两种机制：`APP_HOST`、`DATABASE_URL`、`ETCD_ENDPOINTS` 等在启动时覆盖文件配置；`NEBULA_DATABASE_PASSWORD`、`NEBULA_API_KEY_SALT` 等以 `${VAR}` 形式在文件内引用、解析前展开。

能被完整解析的最小配置见 [`config/config.toml`](config/config.toml)，形如：

```toml
[app]
name = "nebula-id"
host = "0.0.0.0"
http_port = 8080            # 不存在 `app.port`
grpc_port = 9091
dc_id = 0                   # 0..=31
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
default = "segment"         # segment | snowflake | uuid_v8

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
burst_size = 100            # validate(): 必须 <= 10 × default_rps

[tls]
enabled = false
cert_path = ""
key_path = ""
http_enabled = false
grpc_enabled = false

[batch_generate]
max_batch_size = 100        # validate(): 1..=10000
```

> ⚠️ 注意：服务端启动时 `Config::merge()` 会用环境派生配置覆盖 `algorithm.segment` / `algorithm.snowflake` / `algorithm.uuid_v8` 三个子表（仅 `algorithm.default` 保留），该合并修正前请勿依赖子表配置。

**全量选项表（类型 / 默认值 / 是否必填）、`Config::validate()` 校验规则清单与各版本迁移操作，见 [🔧 配置迁移指南](docs/CONFIG_MIGRATION_GUIDE.md)。**

---

## 🌐 国际化

Nebula ID 自 v0.2.0 起内置 ICU 国际化（`rust-i18n`），覆盖错误消息与日志的运行时翻译：

| Locale 标签 | 语言 | locales 文件 | 状态 |
|-------------|------|--------------|------|
| `en` | English（默认） | `locales/en.yml` | ✅ 完整 |
| `zh-CN` | 简体中文 | `locales/zh-CN.yml` | ✅ 完整 |

协商机制：`locale_middleware` 解析 HTTP `Accept-Language` 头（RFC 7231 §5.3.5），按 q-value 降序匹配首个受支持 locale（精确优先、其次前缀匹配），缺失时回退 `en`；业务 handler 经 `Extension<Locale>` 读取并翻译错误响应。`Locale` 派生自用户输入、可被伪造，**不得**用于认证、授权或任何安全决策。

curl 示例、请求头语义与 i18n 模块在架构中的位置，见 [📘 API 参考 · Accept-Language](docs/API_REFERENCE.md#accept-language-header) 与 [🏗️ 架构文档 · i18n 模块](docs/ARCHITECTURE.md#8-i18n-模块位置)。

---

## 🛠️ scripts/run.sh 用法

自 v0.2.0 起所有开发/部署脚本合并为统一入口 `scripts/run.sh`（旧脚本已重命名为 `_*_impl.sh` 内部实现，不再直接调用）：

| 子命令 | 别名 | 作用 |
|--------|------|------|
| `deploy` | — | 通过 docker-compose 部署全栈（PostgreSQL + Redis + Etcd + App） |
| `lint` / `pre-commit` | 互为别名 | 本地 CI 预检（fmt + clippy + test + 安全/文档/覆盖率） |
| `redis-test` | — | Redis 集成测试（需 Redis 监听 6379） |
| `api-test` | — | API 端点测试，可选 `server_url` 参数 |
| `install-hooks` | — | 安装 git pre-commit hooks |
| `help` | `--help`、`-h` | 显示 Usage |

```bash
./scripts/run.sh pre-commit            # 提交前必跑
./scripts/run.sh api-test http://localhost:8080
```

CI（`ci.yml` / `release.yml` / `health-check.yml`）也通过同一入口调用，本地与 CI 行为一致。各子命令的内部实现与完整参数见 [🚀 部署指南 · scripts/run.sh 子命令](docs/DEPLOYMENT.md#8-scriptsrunsh-子命令)。

---

## 🧪 测试

### 🎯 测试策略

测试分层覆盖：`src/` 内联单元测试（`#[cfg(test)]`）、`src/core/tests/` 下按层组织的 E2E 模块（算法、认证、缓存、降级、gRPC 监控、基础设施、服务层等 13 个文件）、`tests/i18n_e2e.rs` 端到端 i18n 测试、`tests/*.sh` shell 端到端脚本（API、降级、分布式、数据库并发）与 Criterion 基准（`benches/i18n.rs`）。逐域场景矩阵与文件映射见 [🧪 测试场景文档](docs/TEST_SCENARIOS.md)。

### ▶️ 运行命令（与 CI 一致）

```bash
# 全量测试（CI 矩阵按 default / postgresql / etcd 三档运行）
cargo test --package nebulaid --features etcd

# Lint 与格式门禁
cargo fmt --package nebulaid -- --check
cargo clippy --package nebulaid --features etcd -- -D warnings

# 覆盖率门禁：行覆盖率 ≥ 95%（排除 server/proto/ 生成代码）
cargo llvm-cov --package nebulaid --features etcd \
  --fail-under-lines 95 --ignore-filename-regex "server/proto/"

# SDK 特性面（独立 CI job）
cargo clippy --package nebulaid --features sdk -- -D warnings
cargo test --package nebulaid --features sdk

# 基准测试
cargo bench --bench i18n

# shell 端到端脚本
./scripts/run.sh api-test
```

### 📊 测试规模

截至 v0.2.x 工作区：约 1780 个 Rust 测试函数（`src/` 内联 + `src/core/tests/` E2E 模块 + `tests/i18n_e2e.rs`）、4 个 shell 端到端脚本、1 组 Criterion 基准（i18n 热路径，4 个基准函数）。CI 覆盖率门禁为行覆盖率 ≥ 95%，pre-push 钩子另执行 ≥ 80% 的本地门禁；v0.2.0 发布时实际行覆盖率 89.91%。逐模块统计与统计口径见 [🧪 测试场景文档 · 统计汇总](docs/TEST_SCENARIOS.md#统计汇总)。

---

## 📊 性能

本仓库唯一声明的 Criterion 基准是 i18n 热路径（`benches/i18n.rs`，`cargo bench --bench i18n` 复现），覆盖翻译查表、带参翻译、错误本地化与 Accept-Language 解析；ID 生成吞吐暂无公开基准数字（尚无对应基准框架，发布前请以自身负载实测）。设计层面的热路径要点——Segment 双缓冲与动态步长、Snowflake 串行化时间戳迁移、p50/p99/p999 环形缓冲分位数、release profile（thin LTO + `panic = "abort"`）——见 [📈 性能指南](docs/PERFORMANCE.md)。

---

## 🔒 安全

### 🛡️ 安全设计

安全设计围绕「认证 → 流控 → 传输 → 审计」展开：garrison API key 认证（Argon2id 哈希 + 常量时间比较 + 缓存 + 轮换宽限期，超 30 天钳制）、令牌桶限流（真实挂载 HTTP 栈，429 携带规范化响应头）、rustls TLS（`tls13` 时拒绝低于 1.3 的握手）、安全响应头与严格 CORS、可信代理 X-Forwarded-For 校验、gRPC 认证失败码区分（`Unauthenticated` / `PermissionDenied`）、单 admin key 守卫与 biz-tags 租户隔离（IDOR 修复）。逐项机制的代码级细节见 [🔒 安全文档](docs/SECURITY.md)。

### ⛓️ 供应链与门禁

CI 五阶段门禁（fmt + clippy → cargo-deny → cargo-audit `--deny warnings` → 多 feature 矩阵测试 + 覆盖率 ≥ 95% → 聚合 gate）与 CodeQL 静态分析、pre-commit 的 gitleaks 私钥扫描、pre-push 的测试 + 覆盖率门禁共同构成供应链防线，完整清单见 [🔒 安全文档 · 供应链与门禁](docs/SECURITY.md#供应链与门禁)。

### 🚨 报告安全漏洞

请勿通过公开 issue 报告安全漏洞。请使用 GitHub [Security Advisories](https://github.com/Kirky-X/NebulaId/security/advisories/new) 私密披露通道提交报告。完整政策见 [SECURITY.md](docs/SECURITY.md)。

---

## 🗺️ 开发路线图

<table style="width:100%; border-collapse: collapse">
<tr><th style="text-align:center">状态</th><th style="text-align:left">方向</th><th style="text-align:left">条目</th></tr>
<tr><td align="center">✅</td><td>核心算法</td><td>Segment 双缓冲与动态步长、Snowflake、UUID v8、算法路由与降级链</td></tr>
<tr><td align="center">✅</td><td>服务与协议</td><td>HTTP + gRPC 双协议、OpenAPI 文档、TLS、限流、garrison API key 认证、审计日志</td></tr>
<tr><td align="center">✅</td><td>可观测与国际化</td><td>逐算法 p50/p99/p999 指标、健康检查、OTLP tracing、en / zh-CN i18n</td></tr>
<tr><td align="center">✅</td><td>质量门禁</td><td>CI 五阶段门禁、覆盖率 ≥ 95%、cargo-deny / cargo-audit / CodeQL、约 1780 个测试</td></tr>
<tr><td align="center">🚧</td><td>SDK 与分布式协调</td><td><code>sdk</code> facade（trait-kit 装配）持续强化；<code>etcd</code> 协调的生产化验证（feature 默认关闭）</td></tr>
<tr><td align="center">📋</td><td>性能工程</td><td>ID 生成吞吐基准框架（criterion）、发布基线记录、批量生成调优</td></tr>
<tr><td align="center">📋</td><td>云原生与容灾</td><td>Kubernetes Operator、多数据中心与自动故障转移、动态算法切换</td></tr>
</table>

---

## 🤝 参与贡献

详细的贡献流程与代码规范请参阅 [🤝 贡献指南](docs/CONTRIBUTING.md)。

### 🛠️ 开发环境

提交前运行 `./scripts/run.sh pre-commit`（或 lefthook 等价钩子：pre-commit 执行 rustfmt、clippy 与 gitleaks 私钥扫描，pre-push 执行 `cargo test --package nebulaid --features etcd` 与覆盖率门禁）；commit message 遵循 Conventional Commits。完整环境搭建步骤见 [🤝 贡献指南 · 快速开始](docs/CONTRIBUTING.md#快速开始)。

### 💖 贡献方式

<table style="width:100%; border-collapse: collapse">
<tr>
<td width="33%" align="center" style="padding: 16px">

### 🐛 报告 Bug

发现问题？<br>
<a href="https://github.com/Kirky-X/NebulaId/issues/new">创建 Issue</a>

</td>
<td width="33%" align="center" style="padding: 16px">

### 💡 功能建议

有好想法？<br>
<a href="https://github.com/Kirky-X/NebulaId/discussions">开始讨论</a>

</td>
<td width="33%" align="center" style="padding: 16px">

### 🔧 提交 PR

想贡献代码？<br>
<a href="https://github.com/Kirky-X/NebulaId/pulls">Fork 并提交 PR</a>

</td>
</tr>
</table>

---

## 📋 更新日志

完整版本历史见 [📋 更新日志](docs/CHANGELOG.md)（遵循 [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) 格式，语义化版本）。

| 版本 | 日期 | 要点 |
|------|------|------|
| Unreleased | — | SDK 迁移 trait-kit 装配（`NebulaIdKit`）、gRPC 全 RPC 认证、限流真实挂载、TLS fail-fast、配置 `deny_unknown_fields` + 退出码 1、密钥轮换宽限期 |
| 0.2.0 | 2026-07-23 | garrison DAO 基础设施、e2e 套件扩充（95% 模块覆盖）、dbnexus / sdforge / confers 架构接管、3 项 strix 安全修复 |

---

## 📄 许可证

本项目采用 `MIT OR Apache-2.0` 双许可证，您可任选其一使用。详见 [LICENSE](LICENSE)。

---

## 🙏 致谢

### 🌟 核心依赖

Nebula ID 站在以下优秀开源项目的肩膀上：

| 依赖 | 用途 |
|------|------|
| [tokio](https://github.com/tokio-rs/tokio) | 异步运行时 |
| [axum](https://github.com/tokio-rs/axum) | HTTP 框架 |
| [tonic](https://github.com/hyperium/tonic) | gRPC 框架 |
| [sea-orm](https://github.com/SeaQL/sea-orm) | 数据库 ORM |
| [uuid](https://github.com/uuid-rs/uuid) | UUID 生成 |
| [rustls](https://github.com/rustls/rustls) | TLS 实现 |
| [confers](https://crates.io/crates/confers) | 配置管理（Kirky.X 自研生态） |
| [dbnexus](https://crates.io/crates/dbnexus) | 数据库抽象（Kirky.X 自研生态） |
| [sdforge](https://crates.io/crates/sdforge) | 服务框架与 OpenAPI（Kirky.X 自研生态） |
| [garrison](https://crates.io/crates/garrison) | API key 认证（Kirky.X 自研生态） |
| [trait-kit](https://crates.io/crates/trait-kit) | 模块接口与 Kit 装配（Kirky.X 自研生态） |
| [limiteron](https://crates.io/crates/limiteron) | 限流原语（Kirky.X 自研生态） |
| [oxcache](https://crates.io/crates/oxcache) | 多级缓存（Kirky.X 自研生态） |
| [rust-i18n](https://crates.io/crates/rust-i18n) | ICU 国际化 |

### 💝 特别感谢

感谢 Rust 社区与所有 [贡献者](https://github.com/Kirky-X/NebulaId/graphs/contributors)。

---

## 📞 联系与支持

<table style="width:100%; max-width: 600px">
<tr>
<td align="center" width="33%">
<a href="https://github.com/Kirky-X/NebulaId/issues"><b style="color:#991B1B">Issues</b></a><br>
<span style="color:#64748B">报告问题和 Bug</span>
</td>
<td align="center" width="33%">
<a href="https://github.com/Kirky-X/NebulaId/discussions"><b style="color:#1E40AF">讨论区</b></a><br>
<span style="color:#64748B">提问和分享想法</span>
</td>
<td align="center" width="33%">
<a href="https://github.com/Kirky-X/NebulaId"><b style="color:#1E293B">GitHub</b></a><br>
<span style="color:#64748B">查看源代码</span>
</td>
</tr>
</table>

---

## ⭐ Star 历史

[![Star History Chart](https://api.star-history.com/svg?repos=Kirky-X/NebulaId&type=Date)](https://star-history.com/#Kirky-X/NebulaId&Date)

如果这个项目对您有帮助，请考虑给它一个 ⭐️！

**由 Kirky.X 构建**

---

<sub>© 2026 Kirky.X. 保留所有权利。</sub>
