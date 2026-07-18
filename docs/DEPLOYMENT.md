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

## 相关文档

- [架构文档](ARCHITECTURE.md)
- [API 参考](API_REFERENCE.md)
- [用户指南](USER_GUIDE.md)
- [配置迁移指南](CONFIG_MIGRATION_GUIDE.md)
