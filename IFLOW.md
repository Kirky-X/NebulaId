# Nebula ID - 项目上下文文档

## 项目概述

**Nebula ID** 是一个企业级分布式 ID 生成系统，使用 Rust 语言开发，为高并发分布式应用提供高性能、高可用的全局唯一 ID 生成服务。

### 核心特性
- **多算法支持**: Segment 号段模式、Snowflake 雪花算法、UUID v7/v4
- **高性能**: 单实例支持百万级 QPS，三级缓存架构
- **高可用**: 多级降级策略、自动故障转移
- **分布式协调**: 基于 etcd 的 Worker ID 自动分配和跨数据中心支持
- **安全认证**: API Key 认证、速率限制、审计日志
- **监控告警**: Prometheus 指标、Grafana 仪表盘、告警系统

### 技术栈
- **语言**: Rust 1.75+
- **Web 框架**: axum (HTTP)、tonic (gRPC)
- **数据库**: PostgreSQL (SeaORM)、Redis
- **配置中心**: etcd
- **监控**: Prometheus、Grafana
- **容器化**: Docker、Kubernetes

---

## 项目结构

```
nebulaid/
├── crates/
│   ├── core/           # 核心库（算法、缓存、配置等）
│   ├── server/         # 服务器实现（HTTP/gRPC）
│   └── client/         # 客户端库
├── docker/             # Docker 配置
├── docs/               # 项目文档
├── grafana/            # Grafana 仪表盘配置
├── protos/             # gRPC 协议定义
├── scripts/            # 脚本工具
├── tests/              # 集成测试
└── target/             # 构建输出
```

---

## 构建和运行

### 前置依赖
- Rust 1.75+
- Docker & Docker Compose（用于开发环境）
- PostgreSQL 16+
- Redis 7.2+
- etcd 3.5+

### 开发环境启动

```bash
# 启动开发环境（PostgreSQL、Redis、etcd）
make dev-up

# 查看日志
make dev-logs

# 停止开发环境
make dev-down
```

### 构建命令

```bash
# 开发构建
make build-dev

# 生产构建（优化）
make build

# 清理构建产物
make clean
```

### 测试命令

```bash
# 运行所有测试
make test

# 运行测试并生成覆盖率报告
make test-coverage

# 运行特定测试
cargo test test_name
```

### 代码质量检查

```bash
# 检查代码格式
make lint

# 运行 Clippy 静态分析
make clippy

# 生成文档
make doc
```

### 数据库操作

```bash
# 运行数据库迁移
make db-migrate

# 进入应用容器
make shell

# 连接到 PostgreSQL
make db-shell

# 连接到 Redis
make redis-cli
```

### 运行服务

```bash
# 直接运行（开发模式）
cargo run --bin nebula-id

# 运行生产版本
./target/release/nebula-id
```

---

## 开发规范

### 代码风格
- 遵循 Rust 官方代码风格指南
- 使用 `cargo fmt` 格式化代码
- 使用 `cargo clippy` 进行静态分析
- 所有公开 API 必须有文档注释

### 测试规范
- 核心算法必须有单元测试
- 集成测试覆盖关键业务流程
- 测试覆盖率目标：85%+
- 使用 `tokio::test` 进行异步测试

### 提交规范
- 提交前运行 `make lint` 和 `make clippy`
- 提交前运行 `make test` 确保测试通过
- 使用清晰的提交信息格式

### 配置管理
- 配置文件使用 TOML 格式
- 支持环境变量覆盖
- 支持热重载（无需重启服务）

---

## 核心模块说明

### 1. 算法引擎 (`crates/core/src/algorithm/`)

#### Segment 算法 (`segment.rs`)
- 号段模式 ID 生成
- 双缓冲机制（DoubleBuffer）
- 动态步长调整
- 异步预加载

#### Snowflake 算法 (`snowflake.rs`)
- 64位分布式唯一 ID
- 时钟回拨处理（三级策略）
- Worker ID 自动分配
- 逻辑时钟抗回拨

#### UUID 算法 (`uuid_v7.rs`)
- UUID v7（时间排序）
- UUID v4（随机）

#### 算法路由器 (`router.rs`)
- 根据配置选择算法
- 支持按业务标签（biz_tag）路由

#### 降级管理器 (`degradation_manager.rs`)
- 多级降级策略
- 自动故障检测和恢复
- 熔断器模式

### 2. 缓存层 (`crates/core/src/cache/`)

#### 多级缓存 (`multi_level_cache.rs`)
- **L1**: RingBuffer（内存队列）
- **L2**: DoubleBuffer（双缓冲）
- **L3**: Redis（分布式缓存）

#### 环形缓冲区 (`ring_buffer.rs`)
- 无锁并发队列
- 水位线控制
- 批量填充

### 3. 认证与授权 (`crates/core/src/auth.rs`)
- API Key 认证
- 双级缓存（本地 + Redis）
- SHA256 哈希存储

### 4. 配置管理 (`crates/core/src/config/`)
- 配置结构定义
- 热重载机制
- 环境变量支持

### 5. 数据库层 (`crates/core/src/database/`)
- SeaORM 实体定义
- Repository 模式
- 连接池管理

### 6. 分布式协调 (`crates/core/src/coordinator/`)
- Etcd 集群健康监控
- Worker ID 自动分配
- 本地缓存故障转移

### 7. 监控告警 (`crates/core/src/monitoring.rs`)
- Prometheus 指标收集
- 告警规则引擎
- 多渠道通知

### 8. 服务器实现 (`crates/server/src/`)

#### HTTP 服务器 (`router.rs`, `handlers/`)
- RESTful API 端点
- 中间件（认证、限流、审计）
- 请求处理逻辑

#### gRPC 服务器 (`grpc.rs`)
- Protocol Buffers 定义
- 流式通信支持
- TLS 加密支持

#### 配置热重载 (`config_hot_reload.rs`)
- 文件监控
- 配置变更通知

---

## API 端点

### RESTful API

| 方法 | 路径 | 描述 | 认证 |
|------|------|------|------|
| POST | `/api/v1/generate` | 生成单个 ID | ✅ |
| POST | `/api/v1/generate/batch` | 批量生成 ID | ✅ |
| POST | `/api/v1/parse` | 解析 ID 信息 | ✅ |
| GET | `/health` | 健康检查 | ❌ |
| GET | `/metrics` | Prometheus 指标 | ❌ |
| GET | `/ready` | 就绪检查 | ❌ |

### gRPC API

```protobuf
service IdGenerator {
  rpc Generate(GenerateRequest) returns (GenerateResponse);
  rpc BatchGenerate(BatchGenerateRequest) returns (BatchGenerateResponse);
  rpc Parse(ParseRequest) returns (IdInfo);
  rpc HealthCheck(HealthCheckRequest) returns (HealthCheckResponse);
  rpc BatchGenerateStream(BatchGenerateStreamRequest) returns (stream BatchGenerateStreamResponse);
}
```

---

## 配置说明

### 主配置文件 (`config.toml`)

```toml
[app]
name = "nebula-id"
host = "0.0.0.0"
http_port = 8080
grpc_port = 50051
dc_id = 0  # 数据中心 ID (0-7)

[algorithm]
type = "segment"  # segment | snowflake | uuid_v7

[database]
url = "postgresql://user:pass@localhost/nebula"
max_connections = 10

[redis]
url = "redis://localhost"
pool_size = 10

[etcd]
endpoints = ["http://localhost:2379"]

[auth]
api_key = "your-api-key-here"

[rate_limit]
default_rps = 1000
burst_size = 1000

[tls]
enabled = false
cert_path = ""
key_path = ""
```

### 环境变量

```bash
export NEBULA_APP_NAME="nebula-id"
export NEBULA_HTTP_PORT="8080"
export NEBULA_DATABASE_URL="postgresql://user:pass@localhost/nebula"
export NEBULA_REDIS_URL="redis://localhost"
export NEBULA_ETCD_ENDPOINTS="http://localhost:2379"
export NEBULA_AUTH_API_KEY="your-api-key-here"
```

---

## 性能指标

### 目标性能
- **单实例 QPS**: > 1,000,000
- **P50 延迟**: < 1ms
- **P99 延迟**: < 10ms
- **P999 延迟**: < 50ms
- **并发连接数**: > 50,000
- **内存占用**: < 4GB

### 算法性能参考
- **Segment**: 100,000+ IDs/sec
- **Snowflake**: 1,000,000+ IDs/sec
- **UUID v7**: 500,000+ IDs/sec
- **UUID v4**: 1,000,000+ IDs/sec

---

## 部署建议

### Kubernetes 部署
- 副本数：每个 DC 至少 3 个
- 资源限制：CPU 4核，内存 4GB
- 健康检查：存活探针 + 就绪探针
- 自动扩容：基于 CPU 和 QPS 指标

### 高可用架构
- 多数据中心部署（支持 8 个 DC）
- etcd 集群用于配置协调
- PostgreSQL 主从复制
- Redis 集群模式

---

## 故障排查

### 常见问题

1. **数据库连接失败**
   - 检查数据库连接配置
   - 验证连接池大小
   - 查看数据库日志

2. **etcd 连接失败**
   - 检查 etcd 端点配置
   - 验证网络连通性
   - 查看本地缓存是否生效

3. **ID 生成失败**
   - 检查降级管理器状态
   - 验证算法配置
   - 查看错误日志

4. **性能未达标**
   - 检查缓存命中率
   - 验证连接池配置
   - 查看监控指标

### 日志查看

```bash
# 查看应用日志
make dev-logs

# 查看数据库日志
make db-shell
\l
\dt

# 查看 Redis 日志
make redis-cli
INFO
```

---

## 项目文档

- **产品需求文档**: `docs/prd.md`
- **技术设计文档**: `docs/tdd.md`
- **任务列表**: `docs/task.md`
- **测试文档**: `docs/test.md`
- **用户验收测试**: `docs/uat.md`
- **API 参考**: `docs/API_REFERENCE.md`
- **贡献指南**: `docs/CONTRIBUTING.md`
- **常见问题**: `docs/FAQ.md`
- **用户指南**: `USER_GUIDE.md`

---

## 开发工作流

### 1. 功能开发
1. 创建功能分支
2. 编写代码和测试
3. 运行 `make lint` 和 `make clippy`
4. 运行 `make test` 确保测试通过
5. 提交代码并创建 PR

### 2. Bug 修复
1. 定位问题并编写复现测试
2. 修复代码
3. 验证测试通过
4. 更新相关文档

### 3. 性能优化
1. 使用 `cargo bench` 进行基准测试
2. 识别性能瓶颈
3. 实施优化
4. 验证性能提升

---

## 贡献指南

1. Fork 项目仓库
2. 创建功能分支
3. 提交更改
4. 推送到分支
5. 创建 Pull Request

参考 `docs/CONTRIBUTING.md` 了解详细信息。

---

## 许可证

MIT License / Apache 2.0 License（双许可）

---

## 联系方式

- GitHub: https://github.com/Kirky-X/NebulaId
- Issues: https://github.com/Kirky-X/NebulaId/issues
- Discussions: https://github.com/Kirky-X/NebulaId/discussions

---

## 版本信息

- **当前版本**: 0.1.0
- **Rust 版本**: 1.75+
- **最后更新**: 2025-12-31