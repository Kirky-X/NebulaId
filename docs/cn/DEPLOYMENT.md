# 🚀 Nebula ID 部署指南

> 本文档描述 Nebula ID 的 Docker 部署、配置、环境变量与监控方案。

## 📋 目录

- [1. 部署架构](#1-部署架构)
- [2. Docker 部署](#2-docker-部署)
- [3. 配置说明](#3-配置说明)
- [4. 环境变量](#4-环境变量)
- [5. 健康检查与监控](#5-健康检查与监控)
- [6. 数据库初始化](#6-数据库初始化)
- [7. 生产部署建议](#7-生产部署建议)
- [8. scripts/run.sh 子命令](#8-scriptsrunsh-子命令)
- [9. Kubernetes Operator 安装](#9-kubernetes-operator-安装)
- [10. 跨数据中心故障转移](#10-跨数据中心故障转移)
- [相关文档](#相关文档)

---

## 1. 部署架构

```mermaid
graph TB
    subgraph Docker Compose
        App[Nebula ID App<br/>nebula-app<br/>:8080 HTTP<br/>:8443 HTTPS（可选）<br/>/health /ready /metrics<br/>:9091 gRPC]
        PG[(PostgreSQL 16<br/>nebula-postgres<br/>:5432)]
        Redis[(Redis 7.2<br/>nebula-redis<br/>:6379)]
        Etcd[(Etcd 3.5<br/>nebula-etcd<br/>:2379)]
    end

    Client[客户端] --> App
    App --> PG
    App --> Etcd
    App -.->|预留：当前无消费点<br/>缓存为进程内 oxcache| Redis

    Prom[Prometheus] -->|抓取 /metrics（Prometheus 文本）| App

    style App fill:#bbdefb
    style PG fill:#a5d6a7
    style Redis fill:#ce93d8
    style Etcd fill:#ffcc80
```

> Redis 说明：`[redis]` 段与 `REDIS_URL` 是**预留配置面**，当前版本无生产
> 消费点——号段双缓冲（`segment` 的 DoubleBuffer）与认证决策缓存都在进程内
> （oxcache memory / garrison memory）。因此 Redis 故障不影响发号与认证，
> `docker compose` 仍启动该服务以保持部署形态与后续接线一致（口径见
> `docs/TEST_PLAN.md` FAULT-08）。

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
| `app` | 本地构建 nebula-id:latest | 8080, 9091 | 2 CPU / 2G RAM | `curl /health` |

### 2.4 单独构建镜像

```bash
# 构建镜像
docker build -f docker/Dockerfile -t nebula-id:latest .

# 运行（需要外部 PostgreSQL/Redis/Etcd）
docker run -d \
  --name nebula-id \
  -p 8080:8080 -p 9091:9091 \
  -e DATABASE_URL=postgresql://idgen:password@host:5432/idgen \
  -e REDIS_URL=redis://host:6379/0 \
  -e ETCD_ENDPOINTS=host:2379 \
  -e NEBULA_API_KEY_SALT=$(openssl rand -hex 32) \
  nebula-id:latest
```

> **说明：** `docker/Dockerfile` 已适配单包 `nebulaid` 构建 —— builder 阶段
> `COPY . .` 后 `cargo build --release --package nebulaid --features etcd`（镜像
> 必须含 etcd feature，compose 注入的 `ETCD_ENDPOINTS` 才能建立协调锁），
> runtime 阶段拷贝 `target/release/nebula-id` 与 `config/`、`locales/`（已不再引用已移除的
> `crates/` 目录与 `nebula-server` 包名）。

## 3. 配置说明

主配置文件位于 `config/config.toml`，支持环境变量展开（`${VAR}` 语法）。

### 3.1 关键配置项

| 配置段 | 关键字段 | 说明 | 默认值 |
|--------|----------|------|--------|
| `[app]` | `dc_id` | 数据中心 ID (0-7) | 0 |
| `[app]` | `worker_id` | 工作节点 ID (0-255) | 0 |
| `[app]` | `http_port` | HTTP 服务端口 | 8080 |
| `[app]` | `grpc_port` | gRPC 服务端口 | 9091 |
| `[database]` | `password` | 数据库密码（环境变量展开） | `${NEBULA_DATABASE_PASSWORD}` |
| `[algorithm]` | `default` | 默认算法 | snowflake（出厂 `config/config.toml` 值；代码 `Config::default()` 为 `segment`） |
| `[algorithm.segment]` | `base_step` | 号段基础步长 | 1000 |
| `[algorithm.snowflake]` | `sequence_bits` | 序列号位数 | 10 |
| `[tls]` | `enabled` | 启用 TLS | false |
| `[auth]` | `enabled` | 启用 API Key 认证 | true |
| `[rate_limit]` | `enabled` | 启用限流 | true |
| `[etcd]` | `endpoints` | Etcd 端点列表 | `[]`（compose 由 `ETCD_ENDPOINTS` 注入；需以 `--features etcd` 构建） |

> ⚠️ **全局限流默认启用**：限流开关是**顶层** `[rate_limit].enabled`
> （不是 `auth.rate_limit.enabled`），由 `src/main.rs` 读取 `config.rate_limit.enabled`
> 决定是否挂载 `RateLimitMiddleware`。`RateLimitConfig::default()` 的 `enabled` 是 `true`，
> 仓库自带的 `config/config.toml` 里 `[rate_limit].enabled = true`，与默认值一致。
> 注意 `[rate_limit]` 段三个字段（`enabled` / `default_rps` / `burst_size`）都没有
> `#[serde(default)]`，整段或单字段缺失都会导致配置解析失败。

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
| `APP_GRPC_PORT` | 9091 | gRPC 端口 |
| `DC_ID` | 0 | 数据中心 ID |
| `RUST_LOG` | info | 日志级别 (trace/debug/info/warn/error) |
| `RUST_BACKTRACE` | 0 | 错误堆栈 (0/1/full) |
| `DATABASE_URL` | - | 完整数据库 URL（覆盖其他配置） |
| `ETCD_ENDPOINTS` | 未设置为空列表（单进程） | etcd 端点（逗号分隔）。**设为空串或配置文件写 `endpoints = []` 表示单进程部署**：应用回退 `LocalDistributedLock` 并输出告警（多实例部署会重复 ID，仅单进程合法）。尾逗号等空白项按未配置处理；未设置且配置文件未写 `[etcd]` 时内置默认同样为空列表 |
| `TLS_ENABLED` / `TLS_HTTP_ENABLED` / `TLS_GRPC_ENABLED` | false | TLS 开关（env 只能开启：`false` 不反向关闭文件里已启用的 TLS）。`TLS_HTTP_ENABLED=true` 时 HTTP 端口按 HTTPS 终止 |
| `TLS_CERT_PATH` / `TLS_KEY_PATH` / `TLS_CA_PATH` | - | 证书链 / 私钥 / 可选自定义 CA 路径 |
| `NEBULA_LOCALE` | 取 `app.locale`（默认 `en`） | 进程默认 locale（原 `LOCALE` 为文档虚构变量）。可选值：`en`、`zh-CN`，非法值回退 `en`。仅用于设置 `fluent-bundle` 全局 locale 影响启动日志与未走 `Accept-Language` 中间件的路径；`/api/v1/*` 路由的运行时响应语言由请求 `Accept-Language` 头协商，不受此变量影响。 |

### 4.3 NEBULA_LOCALE 与 locale 配置详解

进程默认 locale 的解析优先级：**`NEBULA_LOCALE` 环境变量 > 配置文件 `app.locale` > 内置默认 `en`**；取值非法（不在 `en`/`zh-CN` 中）时回退 `en` 并输出告警。

生效的 locale 控制服务进程的全局默认语言，主要影响以下场景：

- **启动日志**：服务启动期间的 `tracing::{info,warn,error}!` 输出会按生效 locale 翻译（参见 `src/main.rs` 中 `i18n::init_i18n(&locale)`）。配置加载前的极早期日志固定为 `en`。
- **非 `/api/v1/*` 路径的日志**：`/health`、`/ready`、`/metrics` 等不经过 `locale_middleware` 的路径，其日志消息使用全局 locale。
- **未携带 `Accept-Language` 头的请求**：`locale_middleware` 协商失败时回退到 `Locale::DEFAULT`（即 `en`），**不**回退到全局 locale。这是有意设计 — 全局 locale 与请求级 locale 解耦，避免请求间互相污染。

**配置示例：**

```bash
# 方式一：环境变量覆盖（优先级最高）
export NEBULA_LOCALE=zh-CN
./target/release/nebula-id

# 方式二：配置文件 config/config.toml 的 [app] 段
#   [app]
#   locale = "zh-CN"

# 或在 docker-compose 中配置
# docker/.env:
#   NEBULA_LOCALE=zh-CN
```

> **注意**：全局 locale 仅影响日志输出语言，**不**影响 `/api/v1/*` 路由的 HTTP 错误响应语言。HTTP 响应语言由每个请求的 `Accept-Language` 头独立协商，详见 [API 参考 - Accept-Language](API_REFERENCE.md#accept-language-请求头)。

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
# 抓取指标（与 HTTP 服务同端口，无独立指标端口）
curl http://localhost:8080/metrics
```

`GET /metrics` 在 HTTP 主端口上暴露 Prometheus 文本格式指标
（`text/plain; version=0.0.4`），包含：
- `nebula_id_ids_generated_total{algorithm}` — 各算法累计生成 ID 数（counter）
- `nebula_id_generate_latency_seconds{algorithm}` — 生成延迟直方图（histogram，
  配套 `histogram_quantile()` 使用）；另有 p50/p99/p999 分位数 gauge
- `nebula_id_generate_failures_total{algorithm}` / `nebula_id_clock_backwards_total{algorithm}` — 失败与时钟回拨计数
- `nebula_id_algorithm_degraded{algorithm}` / `nebula_id_circuit_breaker_open{algorithm}` — 降级与熔断器状态 gauge
- `nebula_id_requests_total` / `nebula_id_uptime_seconds` — API 层请求计数与运行时长

Grafana 面板见 `grafana/dashboards/nebula_id.json`。

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
psql -U idgen -d idgen <<'EOF'
CREATE SCHEMA IF NOT EXISTS nebula_id;

CREATE TABLE IF NOT EXISTS nebula_id.nebula_segments (
    id BIGSERIAL PRIMARY KEY,
    workspace_id VARCHAR(255) NOT NULL,
    biz_tag VARCHAR(255) NOT NULL,
    current_id BIGINT NOT NULL,
    max_id BIGINT NOT NULL,
    step INT NOT NULL DEFAULT 1000,
    delta INT NOT NULL DEFAULT 1,
    dc_id INT NOT NULL DEFAULT 0,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT uq_nebula_segments_ws_tag_dc UNIQUE (workspace_id, biz_tag, dc_id)
);
EOF
```

## 7. 生产部署建议

### 7.0 TLS/HTTPS（docker compose）

TLS 终止在 HTTP 端口（同一个监听 socket 按 `tls.http_enabled` 选择 rustls acceptor，
不额外开端口）。compose 默认把宿主 `8043` 映射到容器 `8443`，启用步骤：

1. 证书放入 `docker/certs/`（`server.crt`/`server.key`，可选 `ca.crt`；
   自签生成见该目录 README）。compose 已挂载为容器内 `/app/certs:ro`。
2. `.env` 设置：`TLS_ENABLED=true`、`TLS_HTTP_ENABLED=true`、`APP_HTTP_PORT=8443`、
   `HEALTHCHECK_PROTO=https`（证书路径如非默认再加 `TLS_CERT_PATH`/`TLS_KEY_PATH`）。
3. `docker compose -f docker/docker-compose.yml up -d app`，
   验证：`curl -k https://localhost:8043/health`。

gRPC 侧同理：`TLS_GRPC_ENABLED=true` 后 gRPC 端口按 TLS 握手（证书同一份）。
生产环境（`NEBULA_ENV=production`）未启用 TLS 会 fail-fast 拒绝启动，
显式放行需 `NEBULA_ALLOW_INSECURE_TLS=1`（仅限内网/过渡期）。

### 7.1 安全

- **必须**设置 `NEBULA_API_KEY_SALT`、`POSTGRES_PASSWORD`、`NEBULA_DATABASE_PASSWORD`
- 启用 TLS（`[tls].enabled = true`），配置证书路径
- 限制 PostgreSQL/Redis/Etcd 端口仅对内网开放
- 定期轮换 API Key
- **认证缓存的失效窗口**（`[auth].cache_ttl_seconds`）：缓存是进程内的，
  吊销 / 轮换 / 禁用只对**处理该管理请求的节点**即时生效，其余节点最长滞后
  一个 `cache_ttl_seconds`（默认 300 秒）才会停止放行旧决策。多副本部署且要求
  吊销近实时时，把 `cache_ttl_seconds` 设为 `0` 关闭缓存（代价：每请求回源
  DB + Argon2id）。另注：命中缓存的请求不回写数据库，`last_used_at` 同样最多
  滞后一个 TTL，不能作为实时活跃度依据。

### 7.2 性能

- PostgreSQL: `shared_buffers` 设为内存的 25%，`effective_cache_size` 设为内存的 75%
- Redis（预留面，当前无消费点）: `maxmemory` 256MB、`maxmemory-policy allkeys-lru` 即可；后续接入缓存层时再按缓存需求调整
- 应用: `--release` 构建，`lto = "thin"` 已在 Cargo.toml 中配置

### 7.3 高可用

- 多数据中心：不同 `dc_id`，共享 Etcd 集群
- 数据库主从复制：PostgreSQL streaming replication
- Etcd 集群：3 或 5 节点奇数部署

## 8. scripts/run.sh 子命令

自 v0.2.0 起，所有开发与部署脚本合并为统一入口 `scripts/run.sh`，替代了早期的多个分散脚本（`deploy`、`pre-commit-check`、`redis_test`、`test_api`、`install-pre-commit-hooks` 等）。旧脚本已重命名为 `_*_impl.sh` 内部实现，不再直接调用。

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
| `api-test` | — | `tests/api_test.sh` | 运行 API 端点测试，可选参数 `server_url` |
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

1. **格式化检查**：`cargo fmt -- --check`
2. **静态分析**：`cargo clippy --lib --bins -- -D warnings`
3. **构建**：`cargo build`
4. **测试**：`cargo test --lib`
5. **安全审计**：`cargo deny check`（若已安装）
6. **文档构建**：`cargo doc --no-deps`
7. **覆盖率**：`cargo tarpaulin`（若已安装）

需要覆盖可选特性或集成测试时，直接使用全特性口径：
`cargo test --package nebulaid --all-features`。`--all-features` 可构建
（= default + etcd + alerting + sdk + integration-tests + openapi）。

GitHub Actions CI（`.github/workflows/ci.yml`）不经过 `scripts/run.sh`，直接执行 `cargo fmt` / `clippy` / `test` / `llvm-cov` / `deny` / `audit` 等工具链命令；`scripts/run.sh` 是本地开发入口，与 CI 使用同一套工具链但调用路径相互独立。

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

内部调用 `tests/api_test.sh`（综合测试套件），对 Nebula ID HTTP API 执行端到端测试：

- 健康检查端点：`GET /health`、`GET /ready`
- 指标端点：`GET /metrics`
- ID 生成端点：`POST /api/v1/generate`、`POST /api/v1/generate/batch`
- ID 解析端点：`POST /api/v1/parse`
- 鉴权测试：`Authorization` 头校验（`ApiKey <key_id>:<key_secret>` scheme 与 Basic salted hash，全程不使用 `X-API-Key`）
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

## 9. Kubernetes Operator 安装

`nebulaid-operator` 是独立 bin crate（`crates/nebulaid-operator`），把 `NebulaIdCluster`
CRD 声明期望拓扑 reconcile 成 per-DC 的 StatefulSet + Headless Service，并周期探测
`/health` 回写 `status`。**kube / kube-runtime / k8s-openapi 依赖仅存在于该 crate**，
主 `nebulaid` 依赖树零影响（`cargo tree -p nebulaid` 可复核）。

### 9.1 前置条件

- Kubernetes 1.30+（CRD 的 `apiextensions.k8s.io/v1` 与 SSA apply 需 1.22+，本清单按 1.30+ 验证口径）
- `kubectl` 已配置目标集群上下文
- 镜像：`nebulaid-operator:0.1.0`（本仓 `crates/nebulaid-operator` 构建；NebulaId 镜像另按 `spec.version` 拉取）

### 9.2 安装步骤

```bash
# 1. 命名空间与 CRD
kubectl create namespace nebulaid-system
kubectl apply -f deploy/crd/nebulaidcluster.yaml

# 2. RBAC（最小权限：nebulaidclusters + apps StatefulSets + core Services/Endpoints）
kubectl apply -f deploy/rbac/operator-clusterrole.yaml

# 3. Operator 本体
kubectl apply -f deploy/manifest/operator-deployment.yaml

# 4. 校验
kubectl -n nebulaid-system rollout status deploy/nebulaid-operator
kubectl get crd nebulaidclusters.nebulaid.kirky.x
```

### 9.3 创建 NebulaIdCluster

```bash
kubectl apply -f deploy/manifest/nebulaidcluster-sample.yaml   # 2 DC 示例
kubectl get nebulaidclusters prod -o wide
kubectl get sts,svc -l nebulaid.kirky.x/cluster=prod
```

`spec` 字段：`replicas`（每 DC 副本数）、`datacenters[]`（`id` = dc 位标识、`etcdEndpoints`）、
`version`（NebulaId 镜像 tag）。`status` 由 operator 回写：`phase`（Pending/Ready/Degraded）、
`activeDc`、`perDcHealth[]`、`endpoints[]`。

### 9.4 权限边界

ClusterRole 只授予 CR 本体/status、`apps/statefulsets`、`core/services`、`core/endpoints`（只读）
与 `events`（只读）；**不授予 secrets、configmaps、pods 等无关资源**。operator 创建的
StatefulSet/Service 带 ownerReferences，删除 CR 即连带回收。

### 9.5 卸载

```bash
kubectl delete nebulaidcluster prod          # 连带回收 STS/Svc
kubectl delete -f deploy/manifest/operator-deployment.yaml
kubectl delete -f deploy/rbac/operator-clusterrole.yaml
kubectl delete -f deploy/crd/nebulaidcluster.yaml
```

---

## 10. 跨数据中心故障转移

### 10.1 per-DC etcd 拓扑

每个 DC 拥有一套**独立** etcd 集群（`spec.datacenters[].etcdEndpoints`），NebulaId StatefulSet
按 DC 各自连接本集群的协调面。拓扑是「多活协调 + 分区命名空间」而非共享单集群：

- **dc 位隔离**：号段 ID 的前 8 位为 dc 前缀（`DcId` 值域即 `u8`），局部空间 2^56；
  `dc=0` 时前缀为零、ID 值与单 DC 部署完全一致（向后兼容）。identifier 空间按 DC 分区，
  跨 DC 天然不重叠——**无需跨 DC 数据复制即可保证 ID 不冲突**。
- **worker 命名空间**：etcd worker 键为 `/idgen/workers/{dc}/{worker_id}`，同 DC 内 CAS 抢占，
  跨 DC 不共享键空间。
- **健康三态**：每个 DC 的 etcd 集群独立判 Healthy / Degraded / Failed（连续 3 次失败降级、
  连续 5 次判死），失败态自动切本地缓存降级（调用方语义不变）。

### 10.2 切换语义

判定规则（刻意保守，两个层面同构：进程内 `DcFailoverController` 与 operator 的 `status.activeDc`）：

| 现场 | 决策 |
| --- | --- |
| active 判 `Failed` 且存在 `Healthy` 候选 | 切到 **dc 号最小**的健康候选（确定性，同一现场必得同一目标） |
| active `Healthy` 或 `Degraded` | 保持（降级期切走会在两个 DC 之间来回打摆） |
| active 状态未知（尚未探测） | 保持（判据缺失 ≠ 判死） |
| 无任何 `Healthy` 候选 | 保持原值（宁可带病服务，不切进不确定源） |

并发正确性：active 切换经原子 CAS，只有真正换掉 active 的那一次算切换（16 线程并发判定
实测仅一次 `SwitchTo`）。发号流量在 `AlgorithmRouter` 层感知 `active_dc()`：切换后新号段
落入新 DC 的位前缀带，旧号段自然耗尽，不需要数据迁移。

### 10.3 验证边界（⚠️ Deferred 声明）

- **代码级已验证**：切换谓词（单测三形态 + 边界）、并发单切换（16 线程）、dc 位分区
  （跨 DC 命名空间不重叠）、真实双 etcd 用例（`tests/etcd_failover.rs`，CI 以 `etcd-a`/`etcd-b`
  两个容器 + `--ignored` 触发）。
- **未验证（Deferred）**：真实多云/多集群环境的端到端部署与故障注入。本变更不宣称已在
  真实生产云完成验证；具备目标集群窗口时按本节步骤执行并回填验证记录（重开条件见
  change `cloud-native-dr` 的 Deferred 段）。

---

## 相关文档

- [架构文档](ARCHITECTURE.md)
- [API 参考](API_REFERENCE.md)
- [用户指南](USER_GUIDE.md)
- [配置迁移指南](CONFIG_MIGRATION_GUIDE.md)
