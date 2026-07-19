# Nebula ID 部署指南

> 本文档描述 Nebula ID 的 Docker 部署、配置、环境变量与监控方案。

## 1. 部署架构

```mermaid
graph TB
    subgraph Docker Compose
        App[Nebula ID App<br/>nebula-app<br/>:8080 HTTP<br/>:50051 gRPC<br/>:9091 Metrics]
        PG[(PostgreSQL 16<br/>nebula-postgres<br/>:5432)]
        Redis[(Redis 7.2<br/>nebula-redis<br/>:6379)]
        Etcd[(Etcd 3.5<br/>nebula-etcd<br/>:2379)]
    end

    Client[客户端] --> App
    App --> PG
    App --> Redis
    App --> Etcd

    Prom[Prometheus] -->|抓取| App

    style App fill:#bbdefb
    style PG fill:#a5d6a7
    style Redis fill:#ce93d8
    style Etcd fill:#ffcc80
```

## 2. Docker 部署

### 2.1 前置条件

- Docker 20.10+
- Docker Compose v2
- 2GB+ 可用内存

### 2.2 快速启动

```bash
# 1. 克隆仓库
git clone https://github.com/Kirky-X/NebulaId.git
cd NebulaId

# 2. 配置环境变量
cp docker/.env.example docker/.env
# 编辑 docker/.env 填入实际密码

# 3. 启动全部服务
docker compose -f docker/docker-compose.yml --env-file docker/.env up -d

# 4. 检查服务状态
docker compose -f docker/docker-compose.yml ps
docker compose -f docker/docker-compose.yml logs -f app
```

### 2.3 Docker Compose 服务

| 服务 | 镜像 | 端口 | 资源限制 | 健康检查 |
|------|------|------|----------|----------|
| `postgres` | postgres:16-alpine | 5432 | 2 CPU / 2G RAM | `pg_isready` |
| `redis` | redis:7.2-alpine | 6379 | 0.5 CPU / 512M RAM | `redis-cli ping` |
| `etcd` | quay.io/coreos/etcd:v3.5.11 | 2379, 2380 | 0.5 CPU / 512M RAM | `etcdctl endpoint health` |
| `app` | 本地构建 nebula-id:latest | 8080, 50051, 9091 | 2 CPU / 2G RAM | `curl /health` |

### 2.4 单独构建镜像

```bash
# 构建镜像
docker build -f docker/Dockerfile -t nebula-id:latest .

# 运行（需要外部 PostgreSQL/Redis/Etcd）
docker run -d \
  --name nebula-id \
  -p 8080:8080 -p 50051:50051 -p 9091:9091 \
  -e DATABASE_URL=postgresql://idgen:password@host:5432/idgen \
  -e REDIS_URL=redis://host:6379/0 \
  -e ETCD_ENDPOINTS=host:2379 \
  -e NEBULA_API_KEY_SALT=$(openssl rand -hex 32) \
  nebula-id:latest
```

> **注意：** `docker/Dockerfile` 当前引用了 `crates/` 目录与 `nebula-server` 包名，与项目实际结构（单包 `nebulaid`，源码在 `src/`）存在历史遗留不一致。构建镜像前可能需要手动调整 Dockerfile 中的 `COPY` 与 `cargo build -p` 参数。后续将修复此问题。

## 3. 配置说明

主配置文件位于 `config/config.toml`，支持环境变量展开（`${VAR}` 语法）。

### 3.1 关键配置项

| 配置段 | 关键字段 | 说明 | 默认值 |
|--------|----------|------|--------|
| `[app]` | `dc_id` | 数据中心 ID (0-7) | 0 |
| `[app]` | `worker_id` | 工作节点 ID (0-255) | 0 |
| `[app]` | `http_port` | HTTP 服务端口 | 8080 |
| `[app]` | `grpc_port` | gRPC 服务端口 | 50051 |
| `[database]` | `password` | 数据库密码（环境变量展开） | `${NEBULA_DATABASE_PASSWORD}` |
| `[algorithm]` | `default` | 默认算法 | snowflake |
| `[algorithm.segment]` | `base_step` | 号段基础步长 | 1000 |
| `[algorithm.snowflake]` | `sequence_bits` | 序列号位数 | 10 |
| `[tls]` | `enabled` | 启用 TLS | false |
| `[auth]` | `enabled` | 启用 API Key 认证 | true |
| `[rate_limit]` | `enabled` | 启用限流 | false |
| `[etcd]` | `endpoints` | Etcd 端点列表 | `["http://localhost:2379"]` |

### 3.2 算法位分配（Snowflake）

```text
| timestamp (43 bits) | datacenter_id (3 bits) | worker_id (8 bits) | sequence (10 bits) |
```

- 数据中心 ID 范围：0-7
- 工作节点 ID 范围：0-255
- 单毫秒序列号范围：0-1023
- 时间戳使用 2024-01-01 作为 epoch 起点

## 4. 环境变量

参考 `docker/.env.example`：

### 4.1 必须配置（生产环境）

| 变量 | 说明 | 生成方法 |
|------|------|----------|
| `POSTGRES_PASSWORD` | PostgreSQL 密码 | `openssl rand -base64 32` |
| `NEBULA_DATABASE_PASSWORD` | 配置文件展开用密码 | `openssl rand -base64 32` |
| `NEBULA_API_KEY_SALT` | API Key 盐值 | `openssl rand -hex 32` |

### 4.2 可选配置

| 变量 | 默认值 | 说明 |
|------|--------|------|
| `APP_HTTP_PORT` | 8080 | HTTP 端口 |
| `APP_GRPC_PORT` | 50051 | gRPC 端口 |
| `APP_METRICS_PORT` | 9091 | 指标端口 |
| `DC_ID` | 0 | 数据中心 ID |
| `RUST_LOG` | info | 日志级别 (trace/debug/info/warn/error) |
| `RUST_BACKTRACE` | 0 | 错误堆栈 (0/1/full) |
| `DATABASE_URL` | - | 完整数据库 URL（覆盖其他配置） |
| `LOCALE` | `en` | 进程默认 locale（v0.2.0 新增）。可选值：`en`、`zh-CN`。仅用于设置 `rust-i18n` 全局 locale 影响启动日志与未走 `Accept-Language` 中间件的路径；`/api/v1/*` 路由的运行时响应语言由请求 `Accept-Language` 头协商，不受此变量影响。 |

### 4.3 LOCALE 环境变量详解（v0.2.0 新增）

`LOCALE` 控制服务进程的全局默认 locale，主要影响以下场景：

- **启动日志**：服务启动期间的 `tracing::{info,warn,error}!` 输出会按 `LOCALE` 翻译（参见 `src/main.rs` 中 `i18n::init_i18n(&locale)`）。
- **非 `/api/v1/*` 路径的日志**：`/health`、`/ready`、`/metrics` 等不经过 `locale_middleware` 的路径，其日志消息使用 `LOCALE` 设置的全局 locale。
- **未携带 `Accept-Language` 头的请求**：`locale_middleware` 协商失败时回退到 `Locale::DEFAULT`（即 `en`），**不**回退到 `LOCALE`。这是有意设计 — 全局 locale 与请求级 locale 解耦，避免请求间互相污染。

**配置示例：**

```bash
# 启动时使用中文 locale（启动日志将为中文）
export LOCALE=zh-CN
./target/release/nebula-id

# 或在 docker-compose 中配置
# docker/.env:
#   LOCALE=zh-CN
```

> **注意**：`LOCALE` 仅影响日志输出语言，**不**影响 `/api/v1/*` 路由的 HTTP 错误响应语言。HTTP 响应语言由每个请求的 `Accept-Language` 头独立协商，详见 [API 参考 - Accept-Language](API_REFERENCE.md#accept-language-header)。

## 5. 健康检查与监控

### 5.1 健康检查端点

```bash
# 应用健康检查
curl http://localhost:8080/health

# 返回 200 OK 表示服务正常
```

Docker Compose 配置了 `HEALTHCHECK`，每 30 秒检查一次。

### 5.2 Prometheus 指标

```bash
# 抓取指标
curl http://localhost:9091/metrics
```

指标端口（9091）暴露 Prometheus 格式指标，包含：
- ID 生成总数（按算法分类）
- 缓存命中率
- 请求延迟分布
- 熔断器状态
- 数据中心健康状态

### 5.3 日志

日志采用 JSON 格式输出到 stdout，由 Docker 日志驱动收集：

```bash
# 查看实时日志
docker compose -f docker/docker-compose.yml logs -f app

# 查看最近 100 行
docker compose -f docker/docker-compose.yml logs --tail 100 app
```

日志级别通过 `RUST_LOG` 环境变量控制，支持模块级配置：
```bash
RUST_LOG=nebulaid::core::algorithm=debug,info
```

## 6. 数据库初始化

```bash
# 使用 init.sql 初始化（Docker Compose 自动执行）
psql -U idgen -d idgen -f scripts/init.sql

# 或手动创建表
psql -U idgen -d idgen <<EOF
CREATE TABLE IF NOT EXISTS segment (
    id BIGSERIAL PRIMARY KEY,
    workspace_id VARCHAR(64) NOT NULL,
    biz_tag VARCHAR(128) NOT NULL,
    current_id BIGINT NOT NULL DEFAULT 0,
    max_id BIGINT NOT NULL,
    step INTEGER NOT NULL,
    delta INTEGER NOT NULL DEFAULT 1,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
EOF
```

## 7. 生产部署建议

### 7.1 安全

- **必须**设置 `NEBULA_API_KEY_SALT`、`POSTGRES_PASSWORD`、`NEBULA_DATABASE_PASSWORD`
- 启用 TLS（`[tls].enabled = true`），配置证书路径
- 限制 PostgreSQL/Redis/Etcd 端口仅对内网开放
- 定期轮换 API Key

### 7.2 性能

- PostgreSQL: `shared_buffers` 设为内存的 25%，`effective_cache_size` 设为内存的 75%
- Redis: `maxmemory` 根据缓存需求调整，`maxmemory-policy allkeys-lru`
- 应用: `--release` 构建，`lto = "thin"` 已在 Cargo.toml 中配置

### 7.3 高可用

- 多数据中心：不同 `dc_id`，共享 Etcd 集群
- 数据库主从复制：PostgreSQL streaming replication
- Etcd 集群：3 或 5 节点奇数部署

## 8. scripts/run.sh 子命令

自 v0.2.0 起，所有开发与部署脚本合并为统一入口 `scripts/run.sh`，替代了 v0.1.x 的多个分散脚本（`deploy`、`pre-commit-check`、`redis_test`、`test_api`、`install-pre-commit-hooks` 等）。旧脚本已重命名为 `_*_impl.sh` 内部实现，不再直接调用。

```bash
# 通用调用格式
scripts/run.sh <subcommand> [args...]
# 等价于
./scripts/run.sh <subcommand> [args...]
```

### 8.1 子命令总览

| 子命令 | 别名 | 调用的内部实现 | 用途 |
|--------|------|----------------|------|
| `deploy` | — | `_deploy_impl.sh` | 通过 docker-compose 部署 Nebula ID 全栈 |
| `lint` | `pre-commit` | `_pre_commit_impl.sh` | 运行本地 CI 预检（fmt + clippy + test + 安全/文档/覆盖率） |
| `redis-test` | — | `_redis_test_impl.sh` | 运行 Redis 集成测试（SET/GET/EXISTS/TTL/DELETE） |
| `api-test` | — | `_api_test_impl.sh` | 运行 API 端点测试，可选参数 `server_url` |
| `install-hooks` | — | `_install_hooks_impl.sh` | 安装 git pre-commit hooks（基于 Python `pre-commit` + Rust 工具链） |
| `pre-commit` | `lint` | `_pre_commit_impl.sh` | 同 `lint`，运行本地 CI 预检 |
| `help` | `--help`、`-h` | — | 显示 Usage 信息 |

未识别的子命令会以非零退出码退出并打印 Usage。

### 8.2 deploy — 部署

```bash
# 部署 Nebula ID 全栈（PostgreSQL + Redis + Etcd + App）
./scripts/run.sh deploy
```

内部调用 `_deploy_impl.sh`，执行流程：

1. 检查 Docker 20.10+ / Docker Compose v2 是否安装
2. 读取 `docker/.env` 配置（密码、端口等敏感配置）
3. `docker compose -f docker/docker-compose.yml up -d` 启动全部服务
4. 等待 PostgreSQL / Redis / Etcd 健康检查通过（最多 30 次重试，间隔 2 秒）
5. 等待 App 的 `/health` 端点返回 200 OK
6. 打印各服务状态与日志入口

前置条件：`docker/.env` 已配置（参考 `docker/.env.example`）。

### 8.3 lint / pre-commit — 本地 CI 预检

```bash
# 提交前必跑
./scripts/run.sh pre-commit
# 或等价别名
./scripts/run.sh lint
```

内部调用 `_pre_commit_impl.sh`，按以下顺序运行检查（任一失败即停止）：

1. **格式化检查**：`cargo fmt --check`
2. **静态分析**：`cargo clippy --all-features --all-targets -- -D warnings`
3. **构建**：`cargo build --all-features`
4. **测试**：`cargo test --all-features`
5. **安全审计**：`cargo audit`（若已安装）
6. **文档构建**：`cargo doc --no-deps --all-features`
7. **覆盖率**：`cargo llvm-cov --all-features --fail-under-lines 95`

GitHub Actions CI（`.github/workflows/ci.yml`）通过同一入口调用此子命令，确保本地与 CI 行为一致。

### 8.4 redis-test — Redis 集成测试

```bash
# 运行 Redis 操作集成测试（需先启动 Redis 监听 6379）
./scripts/run.sh redis-test
```

内部调用 `_redis_test_impl.sh`，依次执行 7 项 Redis 操作验证：

1. `PING` 连通性
2. `SET` 写入
3. `GET` 读取
4. `EXISTS` 存在性
5. `TTL` 剩余时间
6. `DEL` 删除
7. 删除后 `EXISTS` 验证

前置条件：Redis 监听 `localhost:6379`（无密码、默认 db 0）。可通过 `docker compose -f docker/docker-compose.yml up -d redis` 启动。

### 8.5 api-test — API 端点测试

```bash
# 默认测试 http://localhost:8080
./scripts/run.sh api-test

# 指定服务器 URL
./scripts/run.sh api-test http://localhost:8080
```

内部调用 `_api_test_impl.sh`，对 Nebula ID HTTP API 执行端到端测试：

- 健康检查端点：`GET /health`、`GET /ready`
- 指标端点：`GET /metrics`
- ID 生成端点：`POST /api/v1/id/generate`、`POST /api/v1/id/batch-generate`
- ID 解析端点：`POST /api/v1/id/parse`
- 鉴权测试：`X-API-Key` 头校验
- 配置端点：`GET /api/v1/config`

每个测试用例输出 PASS/FAIL，最终汇总 `PASSED`/`FAILED` 计数。

前置条件：Nebula ID 服务已启动并监听 `server_url`。

### 8.6 install-hooks — 安装 git pre-commit hooks

```bash
# 安装 git pre-commit hooks（首次克隆仓库后执行一次）
./scripts/run.sh install-hooks
```

内部调用 `_install_hooks_impl.sh`，执行流程：

1. 检查 Python 3.8+ 与 pip，缺失则提示安装方法
2. 检查 Rust 工具链（`cargo`、`rustfmt`、`clippy`），缺失则通过 `rustup component add` 安装
3. 通过 `pip3 install --user pre-commit` 安装 `pre-commit` 工具
4. `pre-commit install` 将 hook 装入 `.git/hooks/pre-commit`
5. `pre-commit run --all-files` 验证 hooks 工作正常

安装后每次 `git commit` 都会自动运行 `pre-commit` 钩子（参考 `.pre-commit-config.yaml`），包含格式化、clippy、安全审计等检查。

### 8.7 help — 显示 Usage

```bash
./scripts/run.sh help
# 或
./scripts/run.sh --help
./scripts/run.sh -h
# 未带参数时也等价于 help
./scripts/run.sh
```

输出全部子命令的 Usage 与示例。

## 相关文档

- [架构文档](ARCHITECTURE.md)
- [API 参考](API_REFERENCE.md)
- [用户指南](USER_GUIDE.md)
- [配置迁移指南](CONFIG_MIGRATION_GUIDE.md)
