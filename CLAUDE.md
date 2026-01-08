# CLAUDE.md

本文件为 Claude Code (claude.ai/code) 在本仓库中工作时提供指导。

## 项目概述

Nebula ID 是一个企业级分布式 ID 生成系统，使用 Rust 编写，支持多种 ID 生成算法（Segment、Snowflake、UUID v7/v4），具有高可用性、分布式协调和监控能力。

### 工作空间结构

这是一个 Cargo workspace 项目，包含以下 crates：

- `crates/core` - 核心库：算法实现、缓存、认证、配置管理等
- `crates/server` - HTTP/gRPC 服务端实现

## 常用命令

### 构建与测试

```bash
# 开发构建
make build-dev
# 或
cargo build

# 生产构建
make build
# 或
cargo build --release

# 运行所有测试
make test
# 或
cargo test --all -- --test-threads=4

# 运行特定测试
cargo test test_name

# 运行集成测试
cargo test --test integration

# 测试覆盖率
make test-coverage
# 或
cargo tarpaulin --out Html
```

### 代码质量

```bash
# 格式化检查
make lint
# 或
cargo fmt --all -- --check

# 格式化代码
cargo fmt --all

# Clippy 静态分析
make clippy
# 或
cargo clippy --all -- -D warnings -A clippy::derivable-clones -A clippy::redundant-pub-crate

# 预提交检查（格式化、静态分析、构建、测试等）
./scripts/pre-commit-check.sh
```

### 开发环境

```bash
# 启动开发环境（Docker Compose）
make dev-up

# 停止开发环境
make dev-down

# 查看日志
make dev-logs

# 进入应用容器 shell
make shell

# 连接 PostgreSQL
make db-shell

# 连接 Redis CLI
make redis-cli

# 运行数据库迁移
make db-migrate
```

### 文档

```bash
# 生成并打开文档
make doc
# 或
cargo doc --no-deps --open
```

### 清理

```bash
make clean
# 清理构建产物并停止 Docker 服务
```

## 架构概览

### 核心组件

#### 1. ID 生成算法 (`crates/core/src/algorithm/`)

- **Segment 算法**: 基于数据库号段，支持双缓冲和动态步长调整
  - 位置: `algorithm/segment/`
  - 特点: 高吞吐量、有序、数据库依赖
  - 关键组件: `SegmentAlgorithm`, `DoubleBuffer`, `DatabaseSegmentLoader`

- **Snowflake 算法**: Twitter Snowflake 变体
  - 位置: `algorithm/snowflake.rs`
  - 特点: 无需数据库、分布式唯一、时间排序
  - 时钟回拨处理: 三级处理（<5ms 等待、6-1000ms 逻辑时钟、>1000ms 降级到 UUID v7）

- **UUID 算法**: UUID v7/v4
  - 位置: `algorithm/uuid_v7.rs`
  - 特点: v7 时间排序、v4 随机

#### 2. 算法路由 (`crates/core/src/algorithm_router.rs`)

- `AlgorithmRouter`: 根据配置和健康状态路由 ID 生成请求
- `DegradationManager`: 管理算法降级链（Segment → Snowflake → UUID v7 → UUID v4）
- 自动故障检测和算法切换

#### 3. 缓存层 (`crates/core/src/cache/`)

- **RingBuffer**: 基于 `crossbeam::ArrayQueue` 的无锁环形缓冲区
- **DoubleBuffer**: 双缓冲号段机制，支持无缝切换
- **MultiLevelCache**: Redis + 内存两级缓存

#### 4. 分布式协调 (`crates/core/src/coordinator/`)

- **EtcdClusterHealthMonitor**: Etcd 集群健康监控
  - 支持本地缓存降级
  - 自动健康检查和状态恢复

- **DcFailureDetector**: 数据中心故障检测器
  - 多数据中心健康状态管理
  - 自动选择最佳数据中心

#### 5. 数据库层 (`crates/core/src/database/`)

- **Repository 模式**: 使用 SeaORM 的数据访问层
- **乐观锁**: 防止并发号段分配冲突
- **连接池**: 最大 1200 连接（可配置）

#### 6. 认证与安全 (`crates/core/src/auth.rs`, `crates/server/src/`)

- **API Key 认证**: 使用常量时间比较防止时序攻击
- **限流**: 可配置 QPS 限制，默认 1000
- **审计日志**: 完整的操作跟踪
- **TLS 支持**: HTTP/HTTPS 和 gRPC/gRPCS

#### 7. 监控 (`crates/core/src/monitoring.rs`)

- **指标收集**: QPS、延迟（P50/P99/P999）、缓存命中率
- **健康检查**: 组件级和服务级健康状态
- **Prometheus 集成**: 标准指标导出

### 服务端组件 (`crates/server/`)

- **HTTP API**: Axum 框架，RESTful 端点
  - 位置: `router.rs`, `handlers/`
  - 支持单个/批量 ID 生成、ID 解析

- **gRPC API**: 基于 Tonic 的高性能 RPC
  - 位置: `grpc.rs`, `proto/`
  - Protobuf 定义在 `protos/` 目录

- **中间件**:
  - `audit_middleware.rs`: 审计日志
  - `rate_limit_middleware.rs`: 请求限流
  - TLS 支持: `tls_server.rs`

- **配置管理**:
  - `config_management.rs`: 配置 CRUD 操作
  - `config_hot_reload.rs`: 配置热重载

## 代码组织原则

### 可见性规则

- **`pub`**: 导出的公共 API，在 `lib.rs` 中 re-export
- **`pub(crate)`**: 仅 crate 内部可见
- **私有**: 模块内部使用

在 `crates/core/src/lib.rs` 中：
- Public modules 重新导出公共 API（`algorithm`, `auth`, `cache`, `config`, `types`）
- Coordinator 是 `pub` 但不在公共 API 中重新导出（特殊情况）

在 `crates/server/src/lib.rs` 中：
- 某些内部模块是 `pub` 以便二进制目标访问，但不在公共 API 中重新导出

### 模块命名

- `algorithm/`: ID 生成算法相关
- `cache/`: 缓存实现
- `config/`: 配置结构和加载
- `database/`: 数据库访问层
- `coordinator/`: 分布式协调（etcd）

### 错误处理

- 使用 `thiserror` 和 `anyhow` 进行错误处理
- 核心错误类型: `CoreError` (在 `algorithm/` 模块中定义)
- 服务端错误: 通过 HTTP/gRPC 状态码传播

### 异步编程

- 使用 `tokio` 运行时
- 所有数据库/网络操作都是异步的
- 使用 `Arc` + `Mutex/RwLock` 或 `parking_lot` 进行并发控制

## 配置

### 配置文件位置

- `config/` 目录包含配置示例
- 主要配置: TOML 格式
- 支持环境变量覆盖（`NEBULA_*` 前缀）

### 关键配置项

```toml
[app]
name = "nebula-id"
host = "0.0.0.0"
port = 8080

[algorithm]
type = "segment"  # segment | snowflake | uuid_v7 | uuid_v4

[database]
url = "postgresql://user:pass@localhost/nebula"
max_connections = 1200

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

## 测试

### 测试结构

- 单元测试: 与源代码同目录的 `tests/` 模块
- 集成测试: `crates/*/tests/` 目录
- 性能测试: 使用 `criterion`（在 `benches/` 中）

### 运行测试

```bash
# 所有测试
cargo test --all

# 特定算法测试
cargo test --package nebula-core --test segment

# 忽略慢测试
cargo test --all -- --skip slow
```

### 测试数据准备

参考 `docs/test.md` 中的 SQL 准备测试数据。

## 性能基准

目标性能指标（根据 `docs/test.md`）：

- **单实例 QPS**: > 1,000,000
- **P50 延迟**: < 1ms
- **P99 延迟**: < 10ms
- **内存占用**: < 4GB

运行基准测试：

```bash
cargo bench
```

## 常见任务

### 添加新的 ID 算法

1. 在 `crates/core/src/algorithm/` 创建新模块
2. 实现 `IdAlgorithm` trait
3. 在 `algorithm_router.rs` 中注册
4. 添加单元测试
5. 更新文档

### 修改数据库 Schema

1. 在 `crates/core/src/database/` 更新实体模型
2. 创建 SeaORM 迁移文件
3. 运行 `make db-migrate`
4. 更新测试数据

### 添加新的 API 端点

1. 在 `crates/server/src/handlers/` 添加 handler
2. 在 `router.rs` 注册路由
3. 如果需要 gRPC，在 `protos/` 更新 `.proto` 文件
4. 重新生成 protobuf 代码（使用 `tonic-prost-build`）
5. 添加集成测试

### 添加监控指标

1. 在 `crates/core/src/monitoring.rs` 添加指标定义
2. 在代码中记录指标
3. 更新 Prometheus 配置
4. 在 Grafana 添加仪表板

## 数据库 Schema

主要表：

- `workspaces`: 工作空间
- `api_keys`: API 密钥
- `groups`: 分组
- `biz_tags`: 业务标签（算法配置）
- `segments`: 号段信息（按 DC 隔离）
- `audit_logs`: 审计日志

### 关键字段

- `segments.datacenter_id`: DC 隔离，每个 DC 有独立号段
- `segments.step`: 动态步长，根据 QPS 自动调整
- `segments.version`: 乐观锁版本号

## 依赖管理

主要依赖（在 `Cargo.toml` workspace dependencies 中定义）：

- **异步**: tokio 1.49, futures
- **Web**: axum 0.8.8, tower, hyper 1.8
- **gRPC**: tonic 0.14, prost
- **数据库**: sea-orm 1.1, sqlx 0.8
- **缓存**: redis 0.26, dashmap 6, lru 0.12
- **协调**: etcd-client 0.17
- **监控**: prometheus-client 0.24
- **序列化**: serde 1, serde_json 1

添加新依赖时，在 `Cargo.toml` 的 `[workspace.dependencies]` 中添加版本定义。

## 故障排查

### 常见问题

1. **数据库连接失败**: 检查 PostgreSQL 是否运行，连接 URL 是否正确
2. **Etcd 连接失败**: 检查 etcd 端点配置，系统会自动降级到本地缓存
3. **性能下降**: 检查连接池配置，是否触发算法降级
4. **内存泄漏**: 使用 `valgrind` 或 `heaptrack` 分析

### 日志位置

- 应用日志: 控制台输出（使用 `tracing`）
- 审计日志: 数据库 `audit_logs` 表

### 调试

```bash
# 使用环境变量启用调试日志
RUST_LOG=debug cargo run

# 使用特定模块日志
RUST_LOG=nebula_core::algorithm=debug cargo run
```

## 相关文档

- `README.md`: 项目概述和快速开始
- `README_zh.md`: 中文版 README
- `docs/API_REFERENCE.md`: 完整 API 参考
- `docs/test.md`: 测试计划和验收标准
- `docs/USER_GUIDE.md`: 用户指南
- `docs/FAQ.md`: 常见问题

## 版本信息

- 当前版本: 0.1.1
- Rust 版本: 查看 `rust-toolchain.toml`（如果存在）
- 许可证: Apache-2.0 或 MIT
