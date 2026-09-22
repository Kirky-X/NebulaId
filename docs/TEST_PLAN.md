# 📐 NebulaId 测试总体方案：模块分析 · 场景穷举 · 特性组合 · 执行计划

> 适用版本：NebulaId **0.2.x 工作区**（Rust edition 2021，单包 `nebulaid`）。
> 编写依据（只读核对）：`src/` 全部模块源码、`src/core/tests/` 13 个 E2E 文件、`tests/` 脚本、`docs/` 11 篇文档、`docker/`、`scripts/`、`.github/workflows/ci.yml`、`protos/nebula_id.proto`。
> 与 [TEST_SCENARIOS.md](TEST_SCENARIOS.md) 的关系：该文档是**场景矩阵概览**（已覆盖什么）；本文档是**深度方案**——模块交互分析、正常+异常场景穷举、特性组合行为差异、以及每个场景的前置条件/步骤/预期/验证方法。两者编号体系独立。

[🏠 主页](../README.md) • [架构文档](ARCHITECTURE.md) • [场景矩阵](TEST_SCENARIOS.md) • [API 参考](API_REFERENCE.md)

---

## 📋 目录

- [阅读约定](#阅读约定)
- [1. 功能模块分析与交互关系](#1-功能模块分析与交互关系)
- [2. 特性组合分析](#2-特性组合分析)
- [3. 场景穷举与测试用例](#3-场景穷举与测试用例)
- [4. 跨域故障注入专项](#4-跨域故障注入专项)
- [5. 测试执行计划](#5-测试执行计划)
- [6. 缺口汇总与新增测试优先级](#6-缺口汇总与新增测试优先级)

---

## 阅读约定

- **场景编号**：`<域>-<序号>`。域前缀：CFG 配置 / ALG 算法 / DEG 降级熔断 / AUTH 认证授权 / IDGEN 发号 API / RES 资源管理 / RATE 限流 / GRPC / OBS 观测审计 / TLS 传输安全 / COORD 分布式协调 / DB 数据库 / I18N 国际化 / SEC 安全防护 / SDK 嵌入式 / LIFE 生命周期 / FAULT 故障注入。
- **现状标记**：✅ 已有自动化覆盖（附位置）；⚠️ 部分覆盖或覆盖方式有缺陷；❌ 缺口（第 6 节给出新增建议）。
- **特性口径**：`--all-features` **可构建**（sqlite feature 已删除，dbnexus 的 embedded/server 互斥不再触发；= default + etcd + alerting + sdk + integration-tests + openapi）。CI、hooks 与本文档统一采用该口径；`--no-default-features` 另作轻量编译检查（仅编译，不跑测试，运行时需自备 PostgreSQL）。
- ~~**基线豁免**~~（已失效，2026-09-18 修复）：干净基线（2026-09-17，c3bc165 起）曾有 **7 个失败测试**——6 个 `core::database::connection::tests::test_run_migrations_*` + 1 个 `infra_e2e_tests::e2e_database_run_migrations_creates_tables`，根因是 mock 配额未随迁移语句列表的增长（枚举类型 +2）同步更新。随号段唯一约束回填语句的加入已一并修正配额，**当前全套测试 0 失败**。

---

## 1. 功能模块分析与交互关系

### 1.1 分层总览

```
┌─────────────────────────── 表现层 src/server/ ───────────────────────────┐
│ router.rs（路由装配+Prometheus桥+中间件序）                                │
│ handlers/（system·workspace·biz_tag·id·api_key）  grpc.rs（5 RPC）        │
│ middleware/（request_id→size_limit→locale→api_key_auth）                  │
│ rate_limit/  audit/  auth/（garrison缓存）  config/（cors·hot_reload·tls）│
│ api_version.rs  models.rs（错误信封）  openapi.rs  sdforge_adapter.rs     │
└──────────────────────────────────┬───────────────────────────────────────┘
┌──────────────────────────────────▼───────────────────────────────────────┐
│                          核心层 src/core/                                 │
│ algorithm/（router·segment·snowflake·uuid_v8·degradation_manager·traits） │
│ coordinator/（etcd│local：worker分配·分布式锁·集群健康监控）               │
│ database/（connection·迁移·4域仓储）  config/（13节聚合+22条校验+热重载）  │
│ auth/（KeyHasher→Argon2id）  monitoring/（告警子系统，门控）  i18n.rs      │
│ types/（CoreError 24变体·Id·Metrics）                                     │
└──────────────────────────────────┬───────────────────────────────────────┘
┌──────────────────────────────────▼───────────────────────────────────────┐
│ 基础设施（自研生态 ../base、../garrison 路径依赖）：                       │
│ dbnexus(PostgreSQL) · oxcache(缓存) · confers(配置/插值/watch)             │
│ sdforge(HTTP/gRPC/OpenAPI宏) · inklog(日志/脱敏) · limiteron(令牌桶)       │
│ garrison(认证缓存) · trait-kit(仅sdk) · etcd-client(仅etcd)               │
└───────────────────────────────────────────────────────────────────────────┘
```

### 1.2 核心模块清单与职责

| 模块 | 关键文件（行数） | 职责 | 高价值内部行为 |
|---|---|---|---|
| 算法路由 | `core/algorithm/router.rs`（2262） | 算法注册表、per-biz_tag 路由、按次覆盖、fallback 链、观测 | 主算法失败→链上回退→全败返回**主算法原始错误**；initialize 单算法 build 失败仅 warn 不中止；健康聚合 Unhealthy 优先于 Degraded |
| Segment | `core/algorithm/segment.rs`（1468） | 号段双缓冲、CPU 监控、DC 故障检测器（半成品）、三种 Loader | 3 次重试耗尽→`SegmentExhausted`；batch size=0 也报 `SegmentExhausted`（与另两算法语义相反）；生产默认 Loader=Unconfigured（显性报错防静默） |
| Snowflake | `core/algorithm/snowflake.rs`（1077） | 无锁 CAS 状态字 `(ts<<seq)|seq`、时钟回拨+drift 衰减 | 单毫秒容量=mask（不发号 mask 本身）；回拨>阈值→`ClockMovedBackward{状态字ts}`；drift 超 60s 衰减清零；batch 重试 100 次耗尽→`InternalError`（不返回短批） |
| UUID v8 | `core/algorithm/uuid_v8.rs`（453） | RFC 9562 v8 时间有序、21bit 计数器+26bit 随机熵 | 复用 Snowflake 的 `clock_drift_threshold_ms`（耦合点）；回拨载荷=**drift 差值**（与 Snowflake 语义不同）；dc/worker 静默截断 |
| 降级熔断 | `core/algorithm/degradation_manager.rs`（2097） | 熔断状态机 Closed→Open→HalfOpen、降级/恢复、审计联动 | 巡检模式下 `failure_threshold` 计数按**巡检轮次**而非请求；已降级算法跳过健康检查；`auto_recovery`/`recovery_check_interval_ms` 为死配置 |
| 协调器 | `core/coordinator/etcd.rs`（3164，门控）/`local.rs`（594） | worker_id 租约分配、分布式锁、etcd 集群健康监控+文件缓存 | etcd：fail-stop（续期连续 3 失败→触发停机，防双主重复 ID）；release 归属校验防误删；local：无租约、恒返回配置 worker_id（**多实例必冲突**）、锁 TTL 被忽略 |
| 配置 | `core/config/app_config.rs` + 13 节文件 | 聚合根 `Config`（13 节全 `deny_unknown_fields`）、22 条 validate、env 插值/覆盖、merge、启动判定矩阵 | env 覆盖后 merge 会把 `algorithm.segment/snowflake/uuid_v8` 三个子表**重置为默认**，仅 `default` 保留；`key_rotation_grace_period_seconds` 刻意不参与 merge |
| 数据库 | `core/database/`（connection+4 域仓储） | 连接池、幂等迁移（schema `nebula_id`、2 枚举、5 表）、宽限期两列自动补齐 | 号段分配=单语句 `UPDATE…RETURNING`+`ON CONFLICT` 重试（PG 行锁保证并发）；语句超时映射为 `TimeoutError`（非 DatabaseError） |
| 认证核心 | `core/auth/key_hasher.rs` | `KeyHasher` trait + Argon2id（m=19456,t=2,p=1） | 密码材料=`{salt}|{key_id}:{key_secret}` 单一来源；verify 对 malformed 哈希返回 false 而非 Err |
| 监控 | `core/monitoring/core.rs`（门控）+ `server/router.rs` Prometheus 桥 | 告警规则/状态机/通知通道（SSRF 防护）/broadcast 事件流；/metrics 采样渲染桥 | 生产零引用、**test 构建恒编译**（告警 e2e 的 import 所需）；webhook SSRF 已知残余：`https://[::1]` 可绕过 |
| HTTP 服务 | `server/router.rs`（3946）+ handlers | 路由装配、中间件栈、错误信封、限流挂载、安全头 | 中间件序（外→内）：size_limit(main 层)→request_id→sdforge context→审计→CORS→安全头×6→限流→路由→locale→api_version→auth→角色守卫 |
| gRPC | `server/grpc.rs` + `proto/` | 5 RPC（Generate/BatchGenerate/BatchGenerateStream/Parse/HealthCheck） | 与 HTTP 共用 `ApiKeyAuth` 与失败桶；gRPC 侧独有 `classify_miss`（禁用/过期→PermissionDenied，其余→Unauthenticated 同文案防探测）；count 校验先于流消费 |
| 限流 | `server/rate_limit/` | limiteron 令牌桶，16 分片，5min 空闲回收 | 键优先级 workspace_id→IP→"anonymous"，但生产装配**从不注入 workspace_id 扩展**（实际全按 IP）；限流器内部错误 fail-open |
| 审计 | `server/audit/` | 内存环+可选文件落盘（独立 writer task）、PII 脱敏 | 审计失败**不阻塞请求**；文件打开失败回退内存环；路径拒绝 `..` 遍历 |
| 热重载 | `server/config/hot_reload.rs` | `ArcSwap<Config>` 原子换配 + confers FsWatcher（去抖=interval） | 解析/读失败→`Ok(false)` **保留旧配置**；变更 diff 仅 5 字段写审计 |
| SDK | `sdk/kit.rs`（1211，门控） | AsyncKit 四模块装配（锁→仓储→路由→发号）、依赖图校验、健康聚合 | etcd 连接失败 build 直接 Err（fail-closed，不回退本地锁）；Segment 无仓储显性报错 |
| i18n | `core/i18n.rs` + `server/middleware/locale.rs` | 编译期内嵌 en/zh-CN，RFC 7231 协商 | 缺键→en→仍缺→返回键名（永不为空）；locale 不可用于鉴权决策 |

### 1.3 HTTP 请求生命周期（测试必须理解的主干）

```
TCP(TLS?) → size_limit(1MiB→413) → request_id(UUIDv7透传/生成) → sdforge context
→ 审计中间件(响应后按状态码记 Success/Failure/Partial) → CORS(ALLOWED_ORIGINS env)
→ 安全头×6(nosniff/DENY/CSP/HSTS/XSS/Referrer) → 限流[可选](429特例体+x-ratelimit-*)
→ 路由匹配 ┬ 公开: /health /ready /metrics /api-docs/openapi.json /health/sdforge /swagger-ui /api/v1(info)
           └ /api/v1 → locale(Accept-Language) → api_version(v2→400)
              ├ admin组: auth_middleware → admin_required(非Admin→403)
              └ authenticated组: auth_middleware → anonymous_block(Anonymous→401 fail-closed)
                 → handler → core_error_classification 单表映射错误信封
信封: {code, business_code(4位串), message(i18n+5xx消毒), details, request_id, timestamp(ms)}
```

关键不变量（每条都应有测试钉住）：

1. 认证豁免路由（health/ready/metrics/openapi/info）**仍经过**限流、审计、安全头、request_id。
2. OPTIONS 预检在 CORS 层短路，**不消耗限流配额**，且 429 响应必带 CORS 头（限流在 CORS 内侧）。
3. 认证中间件 401/429 是**裸 JSON**（非信封）；handler 层错误是信封。
4. 5xx 消息经消毒收敛为固定 i18n 文案，原始错误只进 tracing 日志（CWE-209）。
5. `Authorization` 头解析全仓唯一实现 `parse_authorization_header_detailed`（HTTP/gRPC 共用），`Bearer` 一律拒绝。

### 1.4 gRPC 链路

每次 RPC：`authenticate`（共享失败桶，10 次/5min/IP→ResourceExhausted）→ `authorize_namespace`（Admin→拒绝发号；User 跨租户→PermissionDenied；未知 ns→NotFound）→ 业务 → `core_error_grpc_code` 映射。`BatchGenerateStream` 在流消费前先校验请求初始化，流内逐项拒绝以 Status **终止整个流**。

### 1.5 启动/停机生命周期（main.rs）

```
init_observability → init_sdforge(防inventory剥离) → load_config(fail-fast矩阵)
→ 生产强制TLS校验(exit 1, 逃生门NEBULA_ALLOW_INSECURE_TLS=1)
→ init_repository(DB连接失败exit1→迁移失败exit1→[etcd]fail-closed装配→盐缺失panic)
→ init_auth_stack(garrison缓存[ttl>0]→ApiKeyAuth[无DB exit1]→key装载→审计→热重载)
→ run_servers: [etcd]健康巡检+worker租约(失败exit1) → 降级巡检后台 → spawn HTTP+gRPC
→ select! 四臂: HTTP结束/GRPC结束/shutdown信号(SIGINT·SIGTERM)/[etcd]租约续期失败(→fail-stop)
→ 收尾: 停巡检→abort限流清理→[etcd]停keepalive+释放worker_id
```

---

## 2. 特性组合分析

### 2.1 feature 门控全景

| Feature | 门控内容 | 默认 | 备注 |
|---|---|---|---|
| `postgresql` | `dbnexus/postgres` | ✅ | default 成员；`--no-default-features` 可关（仅保证编译，运行时需自备 PostgreSQL） |
| `http` / `grpc` / `openapi` | sdforge 宏展开的镜像 feature（空） | http/grpc ✅ | 缺 `openapi` 时 `#[forge]` 的 OpenAPI 路由信息不注册 |
| `garrison-auth` | garrison 接管验证 + `AuthCache` 接线（`server/auth/cache.rs`、`api_key_auth.rs` 缓存路径、main.rs 装配） | ✅ | 关闭后走手写 Argon2id 直查路径（无缓存） |
| `etcd` | `core/coordinator/etcd.rs` 全部、router 的 etcd 健康监控接线、main.rs 协调运行时与 fail-stop select 臂 | ❌ | 开启后 worker_id 租约分配、分布式锁、健康巡检+文件缓存 |
| `alerting` | `core/monitoring/core.rs` 告警子系统（AlertManager/规则状态机/通知通道/事件流） | ❌ | **test 构建恒包含**（`cfg(any(test, feature))`）；生产零引用 |
| `integration-tests` | 门控需要真实数据库的 `#[ignore]` 测试（8 个仓储 + 1 个 etcd） | ❌ | 需真实 PostgreSQL / etcd |
| `sdk` | `src/sdk/`（kit.rs 1211 行）+ trait-kit 依赖，蕴含 `openapi` | ❌ | 不开时依赖树零影响 |

### 2.2 可构建组合矩阵与行为差异

| 组合 | 可构建 | CI 覆盖 | 关键行为差异 |
|---|---|---|---|
| `default` | ✅ | ✅ test job | LocalDistributedLock + 静态 worker_id + 启动 warn；garrison 缓存生效 |
| `--all-features` | ✅ | ✅ test job (all leg) | 覆盖 etcd 租约分配 worker_id、锁带 TTL、集群健康监控+缓存降级、租约失败触发停机；同时编译 sdk 与两个 example、alerting、integration-tests 门控测试 |
| `--no-default-features` | ✅ | ✅ fmt-clippy job（仅编译） | postgres 驱动可关；garrison 手写 Argon2id 回退路径与镜像 feature 关闭路径参与编译；不跑测试 |
| `etcd,integration-tests` | ✅ | ❌（手工） | 8+1 个真库 `#[ignore]` 测试可执行 |
| `default,alerting` | ✅ | ❌（all leg 已覆盖） | 生产二进制体积增大，运行行为与 default 相同（生产路径零引用） |

### 2.3 同一代码路径在不同特性下的差异点（测试须分别验证）

| 差异点 | etcd 关 | etcd 开 |
|---|---|---|
| worker_id 来源 | `Config.app.worker_id` 静态值 | etcd 租约分配（1..=255，跳过 0 哨兵） |
| 分布式锁 | `LocalDistributedLock`（进程内 HashMap，TTL 被忽略） | `EtcdDistributedLock`（CAS+lease，Drop 后台释放） |
| 启动 fail-closed | 无（未配 endpoints 仅 warn） | endpoints 配置但连不通 → exit 1 |
| 健康降级 | `EtcdClusterHealthMonitor` stub（恒 Failed、set_status no-op） | 真实 ping：3 连败 Degraded / 5 连败 Failed+缓存模式 |
| 停机 select 臂 | 租约失败臂恒 pending | 续期连续 3 失败 → **主动停机**（fail-stop 防重复 ID） |

| 差异点 | garrison-auth 关 | garrison-auth 开（默认） |
|---|---|---|
| 验证路径 | 每次 Argon2id 直查 DB | 先查 AuthCache（命中跳过 DB+Argon2），miss 回源 |
| 宽限期凭证 | 每次验证 prev_hash | **永不写缓存**（防 TTL 变相延长轮换窗口） |
| 吊销传播 | 立即（直查） | 最长一个 TTL（默认 300s）；handler 吊销/轮换时主动失效缓存 |

---

## 3. 场景穷举与测试用例

> 每张表列：编号 | 场景（前置条件） | 操作步骤 | 预期结果 | 验证方法 | 现状。
> 「验证方法」中的 cargo 命令均遵守特性约束（CI 口径为 `--all-features`）。

### 3.1 CFG — 配置管理域

**正常流**

| 编号 | 场景（前置） | 步骤 | 预期 | 验证 | 现状 |
|---|---|---|---|---|---|
| CFG-01 | 无配置文件且未显式指定路径 | 启动服务 | 回落 `Config::default()` + warn（DefaultsBecauseMissing），启动成功 | 删文件启动，观察日志与 `/health` | ✅ app_config 单测 |
| CFG-02 | 合法 `config/config.toml` | `--config` 启动 | 13 节全部加载，validate 通过 | 启动 + `/api/v1/config` 抽查 | ✅ supporting_layer_e2e |
| CFG-03 | 文件含 `${VAR}` 与 `${VAR:default}` | 设置/不设置 VAR 加载 | 插值生效；缺失保留字面量 | 单测 `expand_env_vars` | ✅ app_config |
| CFG-04 | 环境变量覆盖 | 设 `APP_HTTP_PORT/DC_ID/DATABASE_URL/ETCD_ENDPOINTS`（逗号拆分）等后启动 | 值覆盖文件；非法值→`InvalidValue` 指明变量名 | 单测 `load_from_env` | ✅ app_config |
| CFG-05 | env+文件合并（热重载路径） | `merge` 后检查 algorithm 子表 | `segment/snowflake/uuid_v8` 子表**被重置为默认**，仅 `default` 保留；宽限期字段不参与 merge | 单测 merge 优先级 | ✅（FAQ 已记录该行为） |
| CFG-06 | `hot_reload.auto_watch_enabled=true` | 启动后修改配置文件 | 去抖 2s 后重载成功，回调触发，diff 写审计 | e2e：临时文件 + watch | ✅ hot_reload 内联 |
| CFG-07 | Admin 已认证，限流启用 | `POST /api/v1/config/rate-limit` 合法值 | 200 success:true；**运行中限流器同步生效**（旧桶清空） | HTTP 调用后立即压测观察新阈值 | ✅ server_layer_e2e |
| CFG-08 | Admin 已认证 | `POST /api/v1/config/logging` level 合法 | 200；实时调整 inklog 级别（立即观察日志变化） | HTTP + 日志断言 | ✅ |
| CFG-09 | Admin 已认证 | `POST /api/v1/config/reload` | 从 `config/config.toml` 重读成功 | HTTP | ⚠️ 仅 smoke（router.rs:2376） |
| CFG-10 | 存在 biz_tag | `POST /api/v1/config/algorithm` 绑定 `biz_tag→snowflake` | success:true；该 tag 后续发号走 snowflake | HTTP + 发号验证 ID 形态 | ✅ api_test.sh / e2e |
| CFG-11 | 任意已认证用户 | `GET /api/v1/config` | 返回脱敏配置：**无 database 段**、TLS 仅布尔 | HTTP + JSON 断言 | ⚠️ 无角色分支测试 |
| CFG-12 | 启动判定矩阵四格 | 分别测：显式路径缺失 / 默认路径缺失 / 文件权限拒绝 / 坏 TOML | Err(FileNotFound) / 默认回落 / Err(FileError) / Err(InvalidValue)，除第二格外全部退出 | 单测 `resolve_startup_config` | ✅ 8 格全覆盖 |

**校验拒绝分支（22 条，统一方法：构造 TOML→加载→断言拒绝消息与退出码 1；`Config::validate` 单测已全覆盖 ✅）**

| # | 分支 | # | 分支 |
|---|---|---|---|
| 1 | `http_port=0` | 12 | `algorithm.default` ∉ {segment,snowflake,uuid_v8} |
| 2 | `grpc_port=0` | 13 | `segment.min_step > max_step` |
| 3 | `shutdown_timeout_seconds=0` | 14 | `base_step` 越出 [min,max] |
| 4 | `dc_id > 31` | 15 | `switch_threshold` 越出 [0,1] |
| 5 | `max_connections=0` | 16 | snowflake 三位宽 u32 求和 ≥64（防回绕） |
| 6 | `min > max_connections` | 17 | `clock_drift_threshold_ms=0` |
| 7 | `acquire_timeout_seconds=0` | 18 | `batch_generate.max_batch_size=0` |
| 8 | `idle_timeout_seconds=0` | 19 | `max_batch_size > 10000` |
| 9 | 限流启用且 `default_rps=0` | 20 | 宽限期 > 30 天（fail-fast 非 clamp） |
| 10 | 限流启用且 `burst_size=0` | 21 | `api_key_salt`/`key_secret` 含未展开 `${` 字面量 |
| 11 | `burst_size > 10×rps`（saturating 防回绕放行） | 22 | 未知键/段（deny_unknown_fields）、重复段、TOML 语法错、缺必填字段 |

**异常流**

| 编号 | 场景（前置） | 步骤 | 预期 | 验证 | 现状 |
|---|---|---|---|---|---|
| CFG-20 | 热重载遇坏 TOML | watch 生效后写入语法错误配置 | 重载返回 `Ok(false)`，**运行态保留旧配置**，error 日志 | e2e：写坏文件→断言旧配置仍生效 | ✅ hot_reload |
| CFG-21 | watch 建立失败 | 监视器初始化异常（如目录不可读） | 按间隔重试重建，不 panic | 单测 | ✅ |
| CFG-22 | 热更 API 服务层校验失败 | `POST /config/rate-limit` 传 `burst>10×rps` | handler 层 validate→400/3002；**服务层**校验失败→**200 包裹** `success:false`（两层行为不同，须分别钉住） | HTTP 两种非法载荷分别断言 | ⚠️ 服务层 200 包裹行为无显式测试 |
| CFG-23 | 非 Admin 调热更 API | User key 调 `POST /config/*` | 403 admin_required | HTTP | ✅ admin 中间件测试 |
| CFG-24 | 宽限期热更 | 运行中尝试变更 `key_rotation_grace_period_seconds` | 该字段不参与 merge/热更（部署期决策），重启才生效 | 单测钉桩 | ✅ merge 单测 |
| CFG-25 | `NEBULA_ENV` 缺失或非法 | 不设该变量启动 | 按**生产**处理（fail-closed）：强制 TLS 校验、盐校验、CORS 收紧；`FromStr` 未知值→Err | 单测 environment | ✅ |

### 3.2 ALG — 算法核心域

**Segment**

| 编号 | 场景（前置） | 步骤 | 预期 | 验证 | 现状 |
|---|---|---|---|---|---|
| ALG-01 | 注入 DbSegmentLoader（测试 Loader） | 同 workspace/group/biz_tag 连续发号 | 单调递增，区间不重叠 | `algorithm_e2e_tests` + 内联 | ✅ |
| ALG-02 | 消耗至剩余 < `switch_threshold` | 持续发号触发换段 | 后台预载下一段，发号不中断 | 内联 DoubleBuffer 测试 | ✅ |
| ALG-03 | 段耗尽且 3 次重试失败 | 模拟 loader 持续失败 | `SegmentExhausted{max_id}` | 内联 | ⚠️ 耗尽路径依赖并发注入，spin-wait 分支未直测 |
| ALG-04 | 未注入仓储 | `generate` | `ConfigurationError`（提示装配 DbSegmentLoader），**不 panic 不静默** | sdk 面 e2e + 内联 | ✅ |
| ALG-05 | `batch_generate(size=0)` | 调用 | **`SegmentExhausted`**（注意：与 Snowflake/UUID 返回空批的语义相反——既有钉桩行为） | 内联 segment.rs:1040 | ✅ |
| ALG-06 | DB 返回负数 current_id（脏数据） | load_segment | 防御性收敛为 0，不按位回绕 | 内联 DbSegmentLoader | ✅ |
| ALG-07 | loader 抛 `DatabaseError` | generate/batch | 错误原样透传（不吞不改写） | 内联 | ✅ |
| ALG-08 | 无任何 buffer / 最近一次加载失败 | `health_check` | Degraded("No active buffers") / Degraded("Last segment load failed")；失败后成功即恢复 Healthy | 内联 4 分支 | ✅ |
| ALG-09 | 无请求发生 | `metrics().cache_hit_rate` | 默认 1.0（total=0 特判） | 内联 | ✅ |
| ALG-10 | 并发单发（多任务同 key） | 并发 generate | CAS loading 标记保证单次 load，spin 方等待 | ⚠️ 需并发压力测试（shell db_concurrency 有黑盒版） | ⚠️ |
| ALG-11 | 并发批量 | 并发 batch_generate | 行为正确（现状：batch 路径**无 loading CAS**，可能重复 load_segment——浪费号段/DB 压力） | 并发测试断言 DB 加载次数 | ❌ 缺口 |

**Snowflake**

| 编号 | 场景（前置） | 步骤 | 预期 | 验证 | 现状 |
|---|---|---|---|---|---|
| ALG-20 | 正常构建 | 连续/并发发号 | 单调、无重复（16 任务并发） | 内联 + e2e | ✅ |
| ALG-21 | 单毫秒打满 | 同毫秒消耗至 mask | 单毫秒容量=sequence_mask（1023，mask 本身保留不发号） | 内联序列绕回测试 | ✅ |
| ALG-22 | 回拨 > 阈值（默认 1000ms） | 注入回拨 | `ClockMovedBackward{last_timestamp=状态字ts}` + drift 记录 + `clock_backwards` 指标 +1 | 内联 | ✅ |
| ALG-23 | 回拨 ≤ 阈值 | 注入小回拨 | 等待追平后成功发号 | 内联 | ✅ |
| ALG-24 | `sequence_bits=0` | 构建后发号 | `SequenceOverflow{timestamp}` 短路返回（防 CAS 空转） | 单测 | ❌ 缺口（分支无直接测试） |
| ALG-25 | batch 高压 | batch 大批量 | 重试 100 次耗尽→`InternalError`（含 generated/requested 明细），**不返回短批** | 内联 | ✅ |
| ALG-26 | `batch_generate(0)` | 调用 | 空 `IdBatch` Ok（与 Segment 相反） | e2e + 内联 | ✅ |
| ALG-27 | 回拨后静置 >60s（drift_decay） | 注入回拨→推进单调钟→发号 | drift 衰减清零，health 恢复；新回拨事件阻止衰减 | 内联三用例 | ✅ |
| ALG-28 | drift == 阈值（边界） | 精确注入 | health **仍 Healthy**（严格大于才 Unhealthy） | 内联边界 | ✅ |
| ALG-29 | dc/worker 超出位宽 | `worker_id=300`（8bit 布局） | 现状：**无校验，位污染高位**（validate 也不查 worker_id 对位宽） | 钉桩测试暴露该行为 | ❌ 缺口 |

**UUID v8**

| 编号 | 场景（前置） | 步骤 | 预期 | 验证 | 现状 |
|---|---|---|---|---|---|
| ALG-30 | 多节点发号 | 4 节点 ×1000 | 全局唯一 + 时间有序 + version 半字节=8 + 嵌入时间戳 ±1s | 内联 | ✅ |
| ALG-31 | 回拨 > 阈值 | 注入回拨 | `ClockMovedBackward{last_timestamp=**drift差值**}`（载荷语义与 Snowflake 不同，断言须分别处理） | 内联 | ✅ |
| ALG-32 | 毫秒翻转 | 注入 last_ts=now-10 | counter 重置为新随机起点 | 内联（非 flaky：按样本内嵌时间戳判定） | ✅ |
| ALG-33 | dc/worker 超位宽 | dc=8/worker=300 | **静默截断**（`& MAX`）——行为钉桩 | 单测 | ❌ 缺口 |
| ALG-34 | batch size=0 / 500 | 调用 | 空 Ok 批 / 全部唯一 | 内联 | ✅ |

**算法路由与工厂**

| 编号 | 场景（前置） | 步骤 | 预期 | 验证 | 现状 |
|---|---|---|---|---|---|
| ALG-40 | 默认 Segment | 构建 Router | fallback 链=[Snowflake,UuidV8]；默认 Snowflake→[UuidV8]；默认 UuidV8→空链 | 内联去重/构建测试 | ✅ |
| ALG-41 | biz_tag 绑定算法（CFG-10） | generate(ctx 带 tag) | per-biz_tag 覆盖优先于 config 默认 | e2e ALG 路由测试 | ✅ |
| ALG-42 | `generate_with_algorithm` | 按次指定算法 | 覆盖默认，仅本次生效 | 内联 + e2e | ✅ |
| ALG-43 | 主算法失败，链上第一个可用 | 注入主算法故障 | fallback 成功 + 降级计数/审计联动 | router 内联 6 类组合 + degradation_tests | ✅ |
| ALG-44 | 主算法失败且全链失败 | 全部注入故障 | 返回**主算法原始错误**（非 "All algorithms failed"） | 内联 | ✅ |
| ALG-45 | 指定算法未注册 | `generate_with_algorithm(未build的算法)` | 走 fallback；链空→`InternalError("All algorithms failed")` | 内联 | ✅ |
| ALG-46 | initialize 单算法 build 失败 | 工厂注入失败 | **仅 warn 收集 errors，其余照常可用**；全部失败→`InternalError("No algorithms available")` | 工厂失败注入 | ❌ 部分失败分支无测试 |
| ALG-47 | 健康聚合 | 各算法混合健康态 | Unhealthy > Degraded > Healthy 优先级；全空→Unhealthy | 内联 4 分支 | ✅ |
| ALG-48 | shutdown | 任一算法 shutdown 失败 | error 日志但继续其他算法，不返回 Err | 内联 3 分支 | ✅ |
| ALG-49 | 生产自举 Segment Loader | 默认算法=Segment、无显式仓储（生产构建） | 进程级 OnceCell 自举（成功/失败都只试一次，失败仅 warn，发号期显性报错） | ⚠️ 需真库（自举失败分支无测试） | ⚠️ |

**类型与错误（types/）**

| 编号 | 场景 | 预期 | 验证 | 现状 |
|---|---|---|---|---|
| ALG-50 | `Id::from_string` 6 分支（UUID/数值/溢出 2^128/含空白/36 字符非 UUID 回退） | 各分支正确或 `InvalidIdString(原文)` | 内联 | ✅ |
| ALG-51 | `AlgorithmType::from_str` 7 个 UUID 别名（uuid_v7/uuid_v4 等全部映射 UuidV8）+ 非法值 | 别名生效；非法→`InvalidAlgorithmType` | e2e + 内联 | ✅ |
| ALG-52 | `Id::Display` 版本位嗅探 | bits76-79==4/7/8 渲染为 UUID，否则数值——**数值型 ID 高位恰好命中版本位会被误渲染**（固有歧义） | 钉桩测试 | ❌ 缺口 |
| ALG-53 | `CoreError` 24 变体 → HTTP 码/business_code/i18n（en+zh-CN） | 单表映射全对（见 3.14 SEC-30 消毒） | helpers 全变体矩阵 + error.rs Display 测试 | ✅ |
| ALG-54 | `ErrorSource` 保链 | `DatabaseError`/`EtcdError`/`IoError` 底层 cause 经 source() 保留，顶层文案与 i18n 同源 | 保链测试 | ✅ |
| ALG-55 | 延迟环 1024 槽、分位、8 线程并发写 | 环绕正确、p50/p99/p999 nearest-rank、clock_backwards 计数 | metrics 内联 | ✅ |
| ALG-56 | QpsWindow 秒切换/时钟回拨 | 秒切换双缓冲正确；**时钟早于 epoch 时 `expect` panic**（与 snowflake 容错策略不一致） | 回拨分支钉桩 | ❌ 缺口（回拨分支） |

### 3.3 DEG — 降级与熔断域

| 编号 | 场景（前置） | 步骤 | 预期 | 验证 | 现状 |
|---|---|---|---|---|---|
| DEG-01 | 连续失败达 `failure_threshold`(5) | record_failure | trigger_degradation + 审计 `algorithm_degradation` | 内联全分支 | ✅ |
| DEG-02 | 降级后连续成功达 `recovery_threshold`(10) | record_success | 仅 **primary** 恢复会重算 current_state；非 primary 恢复不动状态（钉桩行为） | 内联 | ✅ |
| DEG-03 | 未知算法事件 | record_generation_result | 静默 no-op | 内联 | ✅ |
| DEG-04 | 熔断全生命周期 | Closed→(巡检 Unhealthy 达阈值)Open→超时(60s)HalfOpen→连续成功≥2 Closed / 失败重开 Open | 状态机转换与 `is_circuit_open(timeout)` 三态正确；timeout=0 立即可半开 | 内联 10+ 用例 | ✅ |
| DEG-05 | 已降级算法巡检 | check_all_health | **完全跳过**其 health_check | 内联 | ✅ |
| DEG-06 | `enable_circuit_breaker=false` | 巡检 Unhealthy 达阈值 | 仅 trigger_degradation，不开闸 | 内联 | ✅ |
| DEG-07 | `enabled=false` / 后台 task | start/stop/double-start | enabled=false 巡检空转；CAS 防重复启动；stop 优雅 join | 内联 3 用例 | ✅ |
| DEG-08 | `determine_effective_algorithm` | primary 健康/链上首个可用/全降级 | Normal / Degraded(fallback) / Critical | 内联+e2e（critical 场景） | ✅ |
| DEG-09 | 审计 previous_state | 降级事件 | 现状**恒为 "Normal"**（硬编码，非 Normal 起点的状态链失真） | 钉桩测试暴露 | ❌ 缺口 |
| DEG-10 | 计数器长期运行 | 连续失败/成功 >255 | AtomicU8 **回绕**，`should_degrade` 可能误判 | 钉桩/修复 | ❌ 缺口 |
| DEG-11 | 死配置钉桩 | 设置 `auto_recovery=false`、`recovery_check_interval_ms`、`min_step/max_step` | 现状无消费点（配置合法但无效果）——钉住防止误以为生效 | 钉桩测试 | ❌ 缺口 |
| DEG-12 | e2e 降级链 | 12 个 degradation_tests（含 4 个 etcd 门控状态机测试） | 降级→fallback 切换→恢复全链 | `--features etcd` | ✅ |

### 3.4 AUTH — 认证授权域

**凭证解析与验证**

| 编号 | 场景（前置） | 步骤 | 预期 | 验证 | 现状 |
|---|---|---|---|---|---|
| AUTH-01 | 合法 key | `Authorization: Basic base64(id:secret)` / `ApiKey id:secret`（secret 可含冒号，按首个冒号切分） | 200，角色/workspace 注入扩展 | auth_handlers_e2e（35 测试） | ✅ |
| AUTH-02 | 头畸形 6 类 | Bearer 前缀/坏 base64/非 UTF-8/Basic 无冒号/ApiKey 格式错/空凭证 | 全部 401 裸 JSON `{"code":401,"message":"Invalid or missing API key"}`（不区分原因防枚举）+ 记失败 | e2e 六连 | ✅ |
| AUTH-03 | `auth.enabled=false` | 无头请求 | 注入 Anonymous 放行至 `anonymous_block` → **401 fail-closed**（角色扩展缺失同样 401） | e2e 四态 | ✅ |
| AUTH-04 | 同 IP 连续认证失败 | 10 次/5min 窗口 | 第 11 次→429（独立于业务限流桶）；窗口过期解封；桶容量 10000 按最旧淘汰 | e2e + 内联 | ✅ |
| AUTH-05 | garrison 缓存命中 | 同 key 二次请求 | 第二次跳过 DB+Argon2（可观测：DB 调用计数）；**错误密钥恒落库**（不缓存负结果） | auth_cache_wiring 7 测试 | ✅ |
| AUTH-06 | 宽限期凭证 | 旧 secret 在窗口内验证成功 | `used_previous_credential=true`→**不写缓存** | e2e | ✅ |
| AUTH-07 | 吊销后缓存残留 | 吊销已缓存 key 立即请求 | 401（handler 吊销/轮换主动失效缓存） | e2e | ✅ |
| AUTH-08 | `cache_ttl_seconds=0` | 启动 | 不装配 AuthCache，全直查 | e2e zero-TTL | ✅ |
| AUTH-09 | 缓存 TTL 钳制 | key 剩余寿命 < cache_ttl | TTL 被钳制到剩余寿命（±10% 抖动） | cache 内联 | ✅（single-flight 并发 100 miss 已测） |

**角色与租户**

| 编号 | 场景 | 预期 | 验证 | 现状 |
|---|---|---|---|---|
| AUTH-20 | Admin 访问 User-only 端点（generate/biz-tags 等） | 403 `admin_cannot_perform`/1002 | e2e 矩阵 | ✅ |
| AUTH-21 | User 访问 Admin-only（api-keys、workspaces 创建、config 写） | 403 `Admin access required` | e2e + 内联 mini router | ⚠️ 真实 router 穿透无测试（见 GAP） |
| AUTH-22 | Anonymous 访问认证端点（5 方法） | 401 fail-closed | e2e | ✅ |
| AUTH-23 | User 访问他人 workspace 资源 | biz-tag 增删改查 4 方法全部 403（IDOR 防护：先查后比 workspace） | e2e 4 例 + 隔离 | ✅ |
| AUTH-24 | Admin `GET /biz-tags` 不带 workspace_id | **400/拒绝**（必须显式指定，CWE-639 修复）；User 强制绑定自身 | e2e | ✅ |
| AUTH-25 | 认证禁用时 workspace 归属 | key 无 workspace 绑定 → 放行任意 workspace（既有语义） | 内联 verify_* 矩阵 | ✅ |

**密钥生命周期（仓储层）**

| 编号 | 场景（前置） | 步骤 | 预期 | 验证 | 现状 |
|---|---|---|---|---|---|
| AUTH-30 | Admin 创建 key | `POST /api-keys` | 前缀 niad_/nino_ 按角色；secret 明文**仅返回一次**；默认 30 天过期、rate_limit=10000 | e2e + 仓储内联 | ✅ |
| AUTH-31 | secret 长度越界 | <8 或 >128 | `InvalidInput`（DoS 防护）；`Anonymous` 角色 fail-fast 拒绝持久化 | 仓储内联 | ✅ |
| AUTH-32 | 第二个 admin key | 再建 admin | 拒绝（SQL `role='admin' AND enabled=true` 全表计数守卫）；**吊销最后一个启用 admin 也被拒** | e2e 双向 | ✅ |
| AUTH-33 | 轮换宽限期 >0 | 轮换后旧 secret 验证 | 窗口内可用（prev_hash+rotate_expires_at 惰性到期）；到期即失效；宽限期=0 立即失效；grace_expires_at 仅「轮换且>0」非空 | 仓储内联窗口边界 + e2e | ✅ |
| AUTH-34 | last_used 节流 | 高频验证 | 60s 窗口按 key 节流写 DB，10000 容量最旧逐出 | 仓储内联 | ✅ |
| AUTH-35 | Argon2id 参数稳定 | hash→PHC 串 | v=19,m=19456,t=2,p=1；材料=`salt|key_id:secret`；malformed 哈希 verify→false（不 Err） | 仓储向量测试 | ⚠️ key_hasher 本体零内联测试（间接覆盖） |
| AUTH-36 | 配置预置 api_keys | 启动带多条 `[auth].api_keys` | **仅第一条生效**（keys.first()），其余静默忽略；`NEBULA_ADMIN_API_KEY_SECRET` 设置时整段被覆盖；workspace 非法值静默替换为 nil UUID；role 仅精确 `admin` 生效 | main.rs key 装载（⚠️ 建议补集成钉桩） | ⚠️ FAQ 记录，缺自动化 |
| AUTH-37 | 生产盐校验 | 生产环境盐空/弱默认 | 启动 panic（fail-fast）；`NEBULA_DATABASE_PASSWORD` 弱口令仅 warn | main.rs 测试 | ✅ |

### 3.5 IDGEN — 发号 API 域（HTTP）

| 编号 | 场景（前置） | 步骤 | 预期 | 验证 | 现状 |
|---|---|---|---|---|---|
| IDGEN-01 | User key + 已建 workspace/biz_tag | `POST /api/v1/generate` 各算法 | 200 + ID；snowflake/uuid 形态校验（parse 位拆解） | api_test.sh + e2e | ✅ |
| IDGEN-02 | 请求带 `algorithm` 覆盖 | 指定 uuid_v7（别名） | 按次覆盖生效 | api_test.sh | ✅ |
| IDGEN-03 | Admin 调 generate | 同上 | **403**（generate/batch 仅 User） | e2e | ✅ |
| IDGEN-04 | 跨租户 namespace | User B 发 A 的 workspace | 403 workspace_mismatch | e2e + grpc | ✅ |
| IDGEN-05 | workspace/biz_tag 不存在 | 发号 | 404（2001/2003 业务码） | e2e | ✅ |
| IDGEN-06 | batch 边界 | size=1 / size=max(默认100) / size=0 / size>max | 200 / 200 / 400 / 400（3001） | e2e（1..=100 边界）+ grpc | ✅ |
| IDGEN-07 | 请求体缺字段/类型错 | 空体、错型 | 400 + validator 结构化 field+rule（不泄约束值） | e2e 请求验证边界 | ✅ |
| IDGEN-08 | `POST /api/v1/parse` | 各算法 ID/空串/非 ID | 200 元数据 / 400 InvalidIdString | e2e 组合边界 | ✅ |
| IDGEN-09 | 请求体 >1MiB | 2MB POST | 413 PayloadTooLarge（3001） | ⚠️ IntoResponse 单测有；真实 router 穿透无 | ⚠️ |
| IDGEN-10 | Content-Type 错误 / 不存在的路径 / 不存在的方法 | 415/404/405 | 信封或 axum 默认体按现状断言 | api_test.sh | ✅（黑盒） |
| IDGEN-11 | `X-API-Version: v2` | 任意 /api/v1 请求 | 400 + 3001；非法值/缺失→fail-open 按 v1；响应回写 `X-API-Version: v1` | server_layer 8 例 | ✅ |
| IDGEN-12 | `format`/`prefix` 请求字段 | 传 prefixed/自定义前缀 | 现状**无消费点**（未实现，字段被忽略）——钉桩防误解 | 钉桩 | ❌ 缺口 |

### 3.6 RES — 资源管理域

| 编号 | 场景（前置） | 步骤 | 预期 | 验证 | 现状 |
|---|---|---|---|---|---|
| RES-01 | Admin 创建 workspace | `POST /workspaces` | 200 + **User key 明文仅此一次**；默认 max_groups=10/max_biz_tags=100/status=Active | e2e + api_test.sh | ✅ |
| RES-02 | 列表租户视图 | User list / Admin list | User 逐条过滤只见自身；Admin 全量 | e2e | ✅ |
| RES-03 | 按名查询 | `GET /workspaces/{name}` | User 仅自身（先反查行再比对）；他人→403；不存在→404 | e2e | ✅ |
| RES-04 | 重置用户密钥 | `POST /workspaces/{name}/regenerate-user-key` | 旧 user key 删除 + 缓存失效 + 新 secret 返回一次 | e2e/api_test.sh | ✅ |
| RES-05 | group 创建/列表 | User + 归属校验 | 200/分页；跨租户 403 | e2e | ✅ |
| RES-06 | biz_tag 全 CRUD | 创建/查/改/删 | DELETE 返回 204；IDOR 4 方法防护（AUTH-23）；默认 segment/Numeric/base_step=100 | e2e | ✅ |
| RES-07 | 级联删除 | 删 group/删除带 tags 的 workspace | group 事务级联删 biz_tags；workspace CASCADE | 仓储内联（mock）+ ignore 真库 | ⚠️ 真库仅 ignore 覆盖 |
| RES-08 | api-key 管理 | list（非法 UUID 参数→warn 回退 nil）/revoke | 响应体正确；守卫见 AUTH-32 | e2e | ⚠️ handler 内联仅 5 测 |
| RES-09 | 仓储缺行/错参数 | update/delete 不存在行 | `NotFound`；非法入参 `InvalidInput` | 仓储内联 | ✅ |
| RES-10 | 分页边界 | limit/offset 0、超大 | 不 panic、返回空页 | 仓储内联 | ✅ |

### 3.7 RATE — 限流域

| 编号 | 场景（前置） | 步骤 | 预期 | 验证 | 现状 |
|---|---|---|---|---|---|
| RATE-01 | 429 真实产生 | 压穿 burst | 429 特例体 `{"code":429,"message":"Rate limit exceeded","retry_after":N}`（**非信封**） | server_layer 12 例 | ✅ |
| RATE-02 | 响应头规范 | 放行/拒绝 | 恒有 `x-ratelimit-limit`；拒绝时 `remaining:0` + `Retry-After`（=桶补满秒数，非硬编码） | 内联 | ✅ |
| RATE-03 | 层序 | OPTIONS 预检 | 预检不消耗配额；429 响应带 CORS 头（限流在 CORS 内侧） | router 层序契约测试 | ✅ |
| RATE-04 | 键隔离 | 不同 IP / 同 IP | 分桶互不影响；16 分片均匀 | 内联 per-key | ✅ |
| RATE-05 | workspace 键 | — | **生产装配从不注入 workspace_id 扩展**→实际全按 IP 分桶（测试钉住的是与生产不一致的键策略，属测试-生产漂移） | 漂移说明 + 建议钉桩 | ⚠️ |
| RATE-06 | 热更新 | `POST /config/rate-limit` 后压测 | 换表清桶，新阈值立即生效 | e2e 热更生效 | ✅ |
| RATE-07 | 限流器内部错误 | 注入 limiter 错误 | **fail-open 放行**（可用性优先） | 内联 | ✅ |
| RATE-08 | 空闲桶回收 | 5min 无访问 | 后台 60s 周期回收 | e2e | ✅ |
| RATE-09 | XFF 伪造 | 非可信代理发 XFF | 仅 `NEBULA_TRUSTED_PROXIES` 内代理采信；直连按 PeerAddr（DualListener 注入） | infra_e2e 5 例 | ✅ |
| RATE-10 | gRPC | — | **gRPC 无业务限流**（仅认证失败桶）——文档与行为一致性钉桩 | 钉桩 | ⚠️ |
| RATE-11 | token 回补 | 等待后重试 | 按速率回补 | 内联 | ✅ |

### 3.8 GRPC — gRPC 域

| 编号 | 场景（前置） | 步骤 | 预期 | 验证 | 现状 |
|---|---|---|---|---|---|
| GRPC-01 | 合法 key | Generate / Parse / HealthCheck | 正常返回；**HealthCheck 也纳入认证**（探针应走 HTTP /health） | grpc_e2e（真实 tonic transport） | ✅ |
| GRPC-02 | count 边界 | 0 / max+1 / 1 / max | 端点两侧 `InvalidArgument`，两侧通过 | e2e | ✅ |
| GRPC-03 | 认证失败分类 | 缺 metadata/坏格式→Unauthenticated；禁用/过期→**PermissionDenied**（classify_miss 回查行）；未知/错密钥→Unauthenticated（与未知共用文案防探测） | e2e 全分支 | ✅ |
| GRPC-04 | 失败桶共享 | HTTP+gRPC 混合失败 | 同一桶累计，跨协议 429/ResourceExhausted | ⚠️ 单协议已测，跨协议组合缺 | ⚠️ |
| GRPC-05 | authorize_namespace | admin 发号→403；跨租户→403；未知 ns→404；仓储故障→Internal 固定文案 | e2e 5 例 | ✅ |
| GRPC-06 | BatchGenerateStream | 请求初始化非法 | 流消费前即拒绝；流内某项跨租户→Status **终止整个流** | e2e 流测试 | ✅ |
| GRPC-07 | 错误映射全表 | CoreError 24 变体 → gRPC 码 | 4xx 保留本地化消息（200 字节截断）；5xx→Internal("internal error") 固定文案（消毒） | helpers 矩阵 | ✅ |
| GRPC-08 | proto 契约 | 字段名 `namespace`/`tag` 对应 HTTP workspace/biz_tag；GenerateResponse 无 datacenter 字段 | 契约测试防 proto 漂移 | ⚠️ 生成物手工维护（build.rs 已移除），无契约守卫 | ⚠️ |
| GRPC-09 | gRPC TLS/mTLS | ca_path 配置 | tonic ServerTlsConfig 生效 | ⚠️ TLS 装配单测有，传输级 mTLS 握手缺 | ⚠️ |

### 3.9 OBS — 观测与审计域

| 编号 | 场景（前置） | 步骤 | 预期 | 验证 | 现状 |
|---|---|---|---|---|---|
| OBS-01 | 发号后抓取 | `GET /metrics` | Prometheus 文本 0.0.4；指标全集：ids_generated_total/failures/clock_backwards（algorithm 标签）、latency p50/p99/p999 gauge + histogram（50µs–5s 桶）、cache_hit_rate、algorithm_degraded、circuit_breaker_open/half_open、requests_total、uptime_seconds | 渲染桥内联 + api_test.sh | ✅ |
| OBS-02 | 零流量抓取 | 未发号即抓 | 分位全 0 时 histogram 跳过（防伪造观测） | 内联 | ✅ |
| OBS-03 | 探针 | /health /ready /health/sdforge | healthy/ready 组件态/sdforge 版本号 | api_test.sh + e2e | ✅ |
| OBS-04 | OpenAPI 守卫 | `GET /api-docs/openapi.json` | paths 与路由注册**双向一致**（EXPECTED_V1_PATHS 守卫）；缺 openapi feature 时 paths 为空 | openapi 内联 | ✅ |
| OBS-05 | 审计中间件 | 2xx/4xx/5xx 请求 | 事件 result=Success/Failure/Partial；含 duration、client_ip、user_agent、request_id/trace_id | e2e 4 例 | ✅ |
| OBS-06 | 审计文件落盘 | 生产+file_logging | writer task 异步写、50ms flush、sync_all 确认；client_ip 前 3/4 段保留、UA 脱敏 | logger 内联 15 例 | ✅ |
| OBS-07 | 审计异常 | 环满（10000）/文件打不开/写失败/路径含 `..` | 淘汰最旧+warn+计数 / 回退内存环 / broken 计数 / 拒绝路径（CWE-22） | 内联 | ✅ |
| OBS-08 | 告警状态机 | 规则连续满足 | Pending→(for_duration)Firing→解除 Resolved；promotions u8 saturating；history 1000 FIFO | 内联 130+ 测试（alerting/test 恒编译） | ✅ |
| OBS-09 | 告警表达式 | 9 种 DSL（qps/latency/error_rate/segment_exhausted 等） | 解析失败/未知→warn+不触发 | 内联 | ✅ |
| OBS-10 | webhook SSRF | 内网/环回/CGNAT/组播目标 | 拒绝（仅 http(s)、禁 userinfo/私网；重定向禁用）；**已知残余 `https://[::1]` 绕过**（钉桩留档） | 内联全分支 | ✅（残余已知） |
| OBS-11 | 通知通道热替换 | update_channels | ArcSwap 原子生效 | 内联 | ✅ |
| OBS-12 | 日志脱敏 | 凭证/密钥字段入日志 | inklog fast-masking 自动脱敏 | ⚠️ 依赖库能力，无本仓断言（README_TEST_CONFIG 有黑盒检查思路） | ⚠️ |

### 3.10 TLS — 传输安全域

| 编号 | 场景（前置） | 步骤 | 预期 | 验证 | 现状 |
|---|---|---|---|---|---|
| TLS-01 | enabled=true 缺证书/缺私钥 | 启动 | init fail-fast 拒绝启动 | infra_e2e 3 例 | ✅ |
| TLS-02 | min_tls_version=tls13 | 低版本 ClientHello | HTTP 侧握手拒绝；**gRPC 侧例外**（tonic 默认 1.2+，启动 warn 标界） | rcgen 自签实测 | ✅ |
| TLS-03 | DualListener | 同端口明文/TLS 自选 | HTTPS 端到端 + 无 TLS 纯 HTTP 均可用 | infra_e2e | ✅ |
| TLS-04 | 慢握手 | ClientHello 拖延 | 握手 10s 超时+128 深度队列，**不冻结 accept** | infra_e2e | ✅ |
| TLS-05 | PeerAddr 注入 | TLS 连接发请求 | 限流/审计拿到真实对端地址 | infra_e2e | ✅ |
| TLS-06 | 生产强制 | `NEBULA_ENV=production`+tls disabled | exit 1；`NEBULA_ALLOW_INSECURE_TLS=1` 逃生门（warn） | main.rs 测试 | ✅ |
| TLS-07 | ALPN | 默认 ["h2","http/1.1"] | 协商正确 | 单测 | ✅ |
| TLS-08 | 冲突配置 | 全局 disabled 但 per-port enabled | warn tls_config_conflict，整体关闭 | 单测 | ✅ |
| TLS-09 | CORS 生产未配置 | 无 ALLOWED_ORIGINS 启动 | `CorsLayer::new()`（拒绝所有跨源）+ 3 条 error 日志；开发缺省仅 localhost:3000 | server_layer CORS 9 例 | ✅ |
| TLS-10 | CORS 暴露面 | 预检/响应头 | expose 仅 x-request-id/x-ratelimit-remaining（浏览器读不到 limit/Retry-After） | 单测 | ✅ |

### 3.11 COORD — 分布式协调域（etcd 门控）

| 编号 | 场景（前置） | 步骤 | 预期 | 验证 | 现状 |
|---|---|---|---|---|---|
| COORD-01 | 正常 etcd | allocate | 从 1..=255 分配（跳过 0 哨兵），写 `/idgen/workers/{dc}/{worker}`+lease | mock 12 例 + 真库 ignore | ✅ |
| COORD-02 | 全占 | 255 个全被占 | `NoAvailableId` | mock | ✅ |
| COORD-03 | CAS 冲突/预检失败 | key 已存在/kv_get Err | 跳过该 id 继续（预检失败容错为可分配） | mock | ✅ |
| COORD-04 | release 语义 | 本人释放/他人 key/不存在/删除失败 | 成功并清状态 / **拒绝删除（归属校验）** / 幂等成功 / Err | mock | ✅ |
| COORD-05 | keepalive | 周期=TTL/3(10s) | 连续 3 失败→fail-stop 上报+退出；间歇成功重置计数；stop 信号优雅退出 | mock 4 例 | ✅ |
| COORD-06 | 健康状态机 | ping 连败 3/5 次 | Degraded→Failed+缓存模式；成功恢复退出缓存；空 endpoints **不改状态** | mock（degradation_tests 4 例 + etcd.rs 内联） | ✅ |
| COORD-07 | 缓存文件 | 状态落盘/跨实例读 | JSON roundtrip；坏 JSON→InternalError；目录不存在→写失败错误 | mock 8 例 | ✅ |
| COORD-08 | 分布式锁 | acquire 冲突 | 重试 3 次×100ms→AcquireFailed；TTL 下限钳 1s；CAS 失败撤销已授 lease 防泄漏；Drop spawn 释放/无 runtime 仅 warn | mock 12 例 | ✅ |
| COORD-09 | lease 泄漏窗口 | grant 成功后 txn 网络错误 | 现状：lease 不撤销靠 TTL 兜底——**已知窗口** | 钉桩/修复 | ❌ 缺口 |
| COORD-10 | 超时 | 操作超 `operation_timeout_secs`(3s) | `EtcdError::Network("timed out")` | mock 超时映射 | ✅ |
| COORD-11 | 启动 fail-closed | endpoints 配置但不可达 | exit 1（不静默回退本地锁） | main.rs 测试 | ✅ |
| COORD-12 | local 模式（无 etcd） | allocate×N | 恒返回配置 worker_id（多实例冲突——语义钉桩）；锁 TTL 被忽略、遗忘 release 永久死锁 | local 内联 | ⚠️ TOCTOU 并发缺口 |
| COORD-13 | stub 监控（无 etcd） | set_status(Healthy) | 恒 Failed（no-op 钉桩） | local 内联 8 例 | ✅ |
| COORD-14 | clone 语义 | EtcdWorkerAllocator.clone | 原子快照复制但 allocated_value 是 Arc 共享（与 monitor 全 Arc 不一致）——行为钉桩 | 钉桩 | ❌ 缺口 |

### 3.12 DB — 数据库域

| 编号 | 场景（前置） | 步骤 | 预期 | 验证 | 现状 |
|---|---|---|---|---|---|
| DB-01 | 密码缺失/含 `${` 未展开 | 非 URL 且非 SQLite 连接 | `ConfigurationError`（提示 NEBULA_DATABASE_PASSWORD） | 内联 + infra_e2e | ✅ |
| DB-02 | PG URL 无 options | create_connection | 自动注入 `search_path=nebula_id,public`（sea-orm 枚举 CAST 依赖） | 内联 | ✅ |
| DB-03 | 迁移幂等 | 重复 run_migrations | 枚举/表/约束 already-exists 吞掉继续；**ALTER 补列失败→终止启动**；失败消息通用化（CWE-209） | 内联 10 语句 mock | ✅（mock 全绿；真库路径仍为 ignore 集成测试） |
| DB-04 | 并发首分配 | 多实例同 tag 首次 allocate | `ON CONFLICT DO NOTHING`+重试保证单次插入，无重复号段 | **真库并发测试** | ❌ 自动化缺口（shell 脚本部分覆盖） |
| DB-05 | 原子分配 | allocate_segment | 单语句 `UPDATE…RETURNING` 起止区间；行锁串行化 | SQL 形状 mock | ✅ |
| DB-06 | 语句超时 | 慢查询 >statement_timeout_secs | `CoreError::TimeoutError`（语义区分"慢"与"坏"） | mock 注入 | ✅ |
| DB-07 | DbErr 映射 | 任意 DB 错误 | `DatabaseError(首层文案, ErrorSource 保链)`；redact_db_url 密码→`***` | 内联 | ✅ |
| DB-08 | update_segment 0 行 | 更新不存在段 | `NotFound` | 内联 | ✅ |
| DB-09 | 实体枚举 roundtrip | AlgorithmTypeDb/IdFormatDb | DB 枚举与域类型互转 | 内联 | ✅ |
| DB-10 | Debug 脱敏 | api_key Model Debug | hash 字段脱敏显示 | 内联 | ✅ |

### 3.13 I18N — 国际化域

| 编号 | 场景（前置） | 步骤 | 预期 | 验证 | 现状 |
|---|---|---|---|---|---|
| I18N-01 | Accept-Language 协商 | zh-CN / en / 无头 / fr,ja（不支持）/ q=0 / `*` / >4KiB | 对应语言 / 默认 en / 回退 en / q=0 丢弃 / `*` 永不匹配 / 整头拒绝→en | locale 内联 32 例 + i18n_e2e 7 例 | ✅ |
| I18N-02 | 错误信封翻译 | 同一错误双头请求 | en/zh-CN 双语正确；`%{name}` 参数替换 | i18n_e2e + error.rs | ✅ |
| I18N-03 | fallback 链 | 缺键 | en→仍缺→返回键名本身（永不为空） | 内联 | ✅ |
| I18N-04 | 键集对齐 | `locales/{en,zh}/messages.ftl` | 键集一致守卫（防漏翻） | 内联守卫 | ✅ |
| I18N-05 | locale 不参与鉴权 | 伪造 Accept-Language | 不影响认证/授权决策（安全边界） | ⚠️ 设计注释在，无显式负例测试 | ⚠️ |

### 3.14 SEC — 恶意输入与防护域

| 编号 | 场景（前置） | 步骤 | 预期 | 验证 | 现状 |
|---|---|---|---|---|---|
| SEC-01 | 5xx 信息泄漏 | DatabaseError 携带含密码连接串 | 响应体不含 `postgres://`/secret/host:5432；固定文案+全量只进日志（CWE-209） | i18n_e2e 密码泄漏测试 | ✅ |
| SEC-02 | IDOR | User 操作他人 biz_tag（4 方法） | 403 | e2e | ✅ |
| SEC-03 | 认证探测 | 存在/不存在 key、错 secret | 统一 401 文案（HTTP）/"invalid or unknown api key"（gRPC） | e2e | ✅ |
| SEC-04 | 路径穿越 | 审计日志路径 `../x` | 拒绝空路径与 `..` | logger 内联 | ✅ |
| SEC-05 | SSRF | webhook 指向内网 | 拒绝（残余 `https://[::1]` 已知，OBS-10） | 内联 | ✅ |
| SEC-06 | 时序侧信道 | secret 比较 | Argon2 内置常时比较（verify 路径） | ⚠️ 依赖库保证 | ⚠️ |
| SEC-07 | 大请求 | >1MiB | 413（IDGEN-09） | ⚠️ | ⚠️ |
| SEC-08 | 头注入 | request_id 头传非法值 | 合法 UUIDv7 才透传，否则生成新的 | request_id 内联 | ✅ |
| SEC-09 | Bearer 拒绝 | `Authorization: Bearer x` | 401（AUTH-02 一部分） | ✅ | ✅ |
| SEC-10 | 日志 PII | 审计行 | IP 截断、UA 脱敏（OBS-06） | ✅ | ✅ |
| SEC-11 | 未知字段 | JSON body 多传字段 | 配置层 deny_unknown_fields 拒绝；**HTTP handler 层 serde 默认忽略未知字段**（口径不同，钉桩说明） | ⚠️ | ⚠️ |

### 3.15 SDK — 嵌入式域（feature `sdk`）

| 编号 | 场景（前置） | 步骤 | 预期 | 验证 | 现状 |
|---|---|---|---|---|---|
| SDK-01 | 完整装配 | NebulaIdKitBuilder.build | 四模块拓扑序（lock→repository→router→idgen）；依赖图缺失/环→build Err | kit 内联 24 测试 | ✅ |
| SDK-02 | 未注入仓储 | 默认算法=Segment | repository 模块缺席；发号**显性报错**（require_repository_for_segment） | 内联 | ✅ |
| SDK-03 | etcd fail-closed | endpoints 不可达 | build 直接 Err（不回退本地锁） | 内联 | ✅ |
| SDK-04 | 健康聚合 | health_summary | worst-of 聚合 JSON（Healthy/Degraded/Unhealthy/无算法） | 内联 | ✅ |
| SDK-05 | 生命周期 | on_ready/on_shutdown | 降级巡检启停、逆拓扑 drain | 内联 | ✅ |
| SDK-06 | 示例可运行 | `--example embedded --features sdk` / `--example sdk_server --features sdk,http` | 零 DB 发号 / SDK HTTP 服务两个端点 | 手工/脚本 | ⚠️ 无自动化 |

### 3.16 LIFE — 生命周期域

| 编号 | 场景（前置） | 步骤 | 预期 | 验证 | 现状 |
|---|---|---|---|---|---|
| LIFE-01 | DB 不可达启动 | 停 PG 启动 | 连接失败 exit 1；迁移失败 exit 1 | 手工/脚本 | ⚠️ 无自动化 |
| LIFE-02 | 优雅停机 | SIGINT/SIGTERM | select 臂触发：停巡检→abort 限流清理→[etcd]停 keepalive+释放 worker；HTTP graceful shutdown | main.rs 测试（shrink 场景）+ docker compose stop 观察 | ⚠️ |
| LIFE-03 | etcd 租约失败停机 | 断 etcd ≥3 续期周期 | **进程主动退出**（fail-stop 防双主重复 ID）——本系统最关键的安全属性之一 | 真实 etcd 故障注入 | ❌ 自动化缺口（mock 有单步，端到端无） |
| LIFE-04 | HTTP/gRPC 监听失败 | 端口占用 | select 臂结束进程并收尾 | 手工 | ❌ |
| LIFE-05 | 后台任务清理 | 停机后 | 限流清理 task abort、降级巡检停止、审计 writer flush 退出 | 内联各自有；组合序无 | ⚠️ |
| LIFE-06 | 双实例 worker 冲突（无 etcd） | 两进程同配置 | 恒同 worker_id → Snowflake 理论可撞——文档已声明 etcd 为多实例前提 | 文档口径钉桩 | ⚠️ |

---

## 4. 跨域故障注入专项

> 以下场景横切多个域，需要「故障注入 → 行为断言 → 恢复断言」三段式验证。Rust 层单步已有 mock 覆盖；**端到端恢复链是主要缺口**。

| 编号 | 故障 | 注入方法 | 期望行为（降级） | 期望行为（恢复） | 现状 |
|---|---|---|---|---|---|
| FAULT-01 | PostgreSQL 断连 | docker stop nebula-postgres | 发号（segment）报 5xx 消毒文案；snowflake/uuid_v8 **不受影响**（零 DB）；`/ready` database=false | 重启后自动恢复，无需重启应用 | ⚠️ mock 单步有；shell degradation 脚本不注入真故障 |
| FAULT-02 | PostgreSQL 慢查询 | `pg_sleep` 后台会话 / 池耗尽 | 语句超时→`TimeoutError`/503-5004；acquire 超时错误 | 恢复后正常 | ❌ |
| FAULT-03 | etcd 断连（--features etcd） | docker stop nebula-etcd | 健康监控 3 连败 Degraded→5 连败 Failed+**缓存模式**（读本地 cache 文件）；锁操作失败 | 恢复 ping 成功退出缓存模式 | ⚠️ mock 全有、真机链路无 |
| FAULT-04 | etcd 长时间断连 | 持续断 >3 续期周期 | worker keepalive fail-stop → **进程退出**（宁可停机不可重复发号） | —（编排层重启，重获租约） | ❌ 关门场景，建议真机自动化 |
| FAULT-05 | 时钟回拨 | 系统级（需特权）或算法注入 | >阈值：报错+指标+降级链可能接管；≤阈值：静默等待 | drift 60s 衰减恢复 health | ✅ 算法级注入；系统级无 |
| FAULT-06 | 审计文件系统满/只读 | 挂载只读目录 | 文件打不开→**回退内存环**，请求不受影响 | — | ✅ 内联 |
| FAULT-07 | 限流器内部错误 | 注入 limiter 错误 | fail-open 放行（可用性优先） | — | ✅ 内联 |
| FAULT-08 | Redis 故障 | docker stop nebula-redis | 现状 Redis 后端**无生产消费点**（缓存走进程内 oxcache/garrison memory）——验证服务完全不受影响 + 文档口径一致 | — | ⚠️ 口径确认即测试 |
| FAULT-09 | 磁盘满（号段持久化） | tmpfs 塞满 | allocate 报 DatabaseError（5xx 消毒）；不 panic | 恢复后正常 | ❌ |

---

## 5. 测试执行计划

### 5.1 分层执行策略

| 层 | 内容 | 规模（实测 2026-09-17） | 外部依赖 | 频率 |
|---|---|---|---|---|
| L0 内联单元 | src 各模块 `#[cfg(test)]` | core 892 + server 717 + sdk 24 + main 18 ≈ 1651 | 零（MockDatabase/rcgen 自签/tempfile） | 每次提交（pre-commit `cargo test --lib`） |
| L1 crate 内 E2E | `src/core/tests/` 12 模块 + `tests/i18n_e2e.rs` | 221 + 7 | 零 | 每次提交 |
| L2 真实基建 | `#[ignore]` ×9（8 仓储 + 1 etcd） | 9 | PostgreSQL / etcd | 合并前手工 / 夜间 |
| L3 shell 黑盒 | `tests/*.sh` 4 脚本（对运行中服务） | 4 | docker compose 全栈 | 发版前 |
| L4 基准 | `benches/i18n.rs`（唯一）、`benches/algorithms.rs` | 2 文件 | 零 | 性能回归时 |

### 5.2 执行命令手册

```bash
# L0+L1 全量（默认特性，零外部依赖）
cargo test --package nebulaid --no-fail-fast
# 对照基线：恰有 7 个已知失败（6× test_run_migrations_* + 1× e2e_database_run_migrations_creates_tables）

# 全特性全集（pre-push / 覆盖率门禁 / CI all leg 同款）
cargo test --package nebulaid --all-features --no-fail-fast

# 轻量编译检查（CI fmt-clippy job 同款；仅编译不跑测试）
cargo check --package nebulaid --no-default-features --lib --bins

# L2 真实 PostgreSQL（docker compose 先起 postgres）
docker compose -f docker/docker-compose.yml up -d postgres
DATABASE_URL=postgresql://idgen:idgen123@localhost:5432/idgen \
  cargo test --package nebulaid --features integration-tests -- --ignored --no-fail-fast

# L2 真实 etcd
ETCD_TEST_ENDPOINTS=http://127.0.0.1:2379 \
  cargo test --package nebulaid --features etcd,integration-tests -- --ignored

# L3 全栈黑盒
docker compose -f docker/docker-compose.yml --env-file docker/.env up -d
export ADMIN_API_KEY_ID=... ADMIN_API_KEY_SECRET=...
./scripts/run.sh api-test http://localhost:8080        # 主链路（健康/发号/parse/校验/性能）
./tests/db_concurrency_test.sh                          # 并发与唯一性
./tests/degradation_test.sh                             # 健康路径观察（注意：不注入真故障）
./tests/distributed_test.sh                             # 分布式唯一/时序/位结构

# 覆盖率门禁（pre-push 90%；CI 95% 且排除 server/proto/）
cargo llvm-cov --package nebulaid --fail-under-lines 90

# 基准
cargo bench --bench i18n
```

> ⚠️ 已知脚本缺陷（使用 L3 结果时须人工复核）：`db_concurrency_test.sh` 高并发阶段不收集 ID（唯一性仅末段真校验）且总结无条件打印通过；`degradation_test.sh` 不注入真实故障、失败仅告警不改退出码；`distributed_test.sh` 时序断言在响应无 timestamp 字段时空真通过；`api_test.sh` 默认指向外部生产 URL（务必显式传 localhost）、`GET /api/v1/metrics` 用例因路由不存在预期失败、创建的 workspace 不清理。

### 5.3 CI 门禁对照（.github/workflows/ci.yml）

| 阶段 | 命令要点 | 阈值 |
|---|---|---|
| fmt+clippy | `--all-features` `-D warnings`（另有 `--no-default-features` 编译检查） | 零告警 |
| deny+audit | cargo-deny / cargo-audit | deny warnings |
| 多特性矩阵测试 | matrix = default / all（`--all-features`，覆盖 sdk+etcd+alerting+integration-tests） | 全绿 |
| 覆盖率 | `llvm-cov --fail-under-lines 95 --ignore-filename-regex "server/proto/"`（default leg 权威门禁；all leg 只跑测试不进门禁——sdk 代码测试密度低，与旧独立 sdk job 策略一致） | 95% 行覆盖 |
| lefthook pre-push | `--all-features` test + coverage `--fail-under-lines 90`（本地实测约 93.6%：sdk 代码稀释） | 90% |

### 5.4 新增测试的落位约定

- 纯逻辑分支/边界 → 对应模块 `#[cfg(test)]` 内联（mock 注入沿 `testing.rs` 既有替身：`DummyDistributedLock`/`FailingDistributedLock`/`MockDatabase`）。
- 跨中间件 HTTP 行为 → `src/core/tests/server_layer_e2e_tests.rs`（mini router 模式）。
- 真实基建语义（并发分配、fail-stop）→ `#[ignore]` + `integration-tests` 门控，复用生产 `run_migrations` 作 schema 事实源。
- 黑盒回归 → 扩展 `tests/*.sh`（优先修复 5.2 所列脚本缺陷而非新写脚本）。

---

## 6. 缺口汇总与新增测试优先级

### P0 — 正确性/资损风险（建议立即补）

| # | 缺口 | 对应场景 | 建议落位 |
|---|---|---|---|
| 1 | 真实 PG 并发首分配防重（ON CONFLICT 路径）无自动化 | DB-04 | `#[ignore]` 并发 allocate + 唯一性断言（repository.rs integration 模块） |
| 2 | etcd 租约 fail-stop 端到端（断 etcd→进程退出） | LIFE-03 / FAULT-04 | ignore + 真实 etcd 容器 |
| 3 | HTTP 全中间件叠加链（限流→CORS→审计→locale→auth 组合序）仅一个点测 | §1.3 不变量 1-4 | server_layer_e2e 组合 router |
| 4 | `size_limit` 413 与 admin 403 在**真实 router** 的穿透 | IDGEN-09 / AUTH-21 | server_layer_e2e |
| 5 | batch size=0 三算法语义分裂 + Segment batch 无 CAS 防重 | ALG-05/11 | 行为钉桩（先钉现状再议修复） |

### P1 — 行为漂移防护（钉桩防回归）

| # | 缺口 | 场景 |
|---|---|---|
| 6 | 死配置钉桩：`min_step/max_step`、`auto_recovery`、`recovery_check_interval_ms`、`GenerateContext.format/prefix`、Redis 后端 | DEG-11 / IDGEN-12 / FAULT-08 |
| 7 | 载荷语义分裂钉桩：`ClockMovedBackward.last_timestamp`（Snowflake=状态字 ts vs UUIDv8=drift 差值） | ALG-22/31 |
| 8 | `SequenceOverflow`（seq_bits=0）、`Id::Display` 版本位误判、dc/worker 超位宽截断 | ALG-24/29/33/52 |
| 9 | 降级审计 `previous_state` 恒 "Normal"、AtomicU8 回绕 | DEG-09/10 |
| 10 | `QpsWindow` 时钟回拨 panic 风险、LocalDistributedLock TOCTOU、etcd lease 泄漏窗口 | ALG-56 / COORD-09/12 |
| 11 | config 服务层校验失败 200 包裹行为、`GET /config` 角色分支 | CFG-22/11 |
| 12 | key_hasher 直接单测（材料编码/空盐边界） | AUTH-35 |
| 13 | 跨协议共享认证失败桶（HTTP+gRPC 混合计数） | GRPC-04 |

### P2 — 工程与文档一致性

| # | 缺口 | 说明 |
|---|---|---|
| 14 | docker 构建链断裂：`vendor-deps.sh` 仍找 `../sdforge`、`../inklog` 旧路径（现为 `../base/*` + `../garrison`），`docker/build.sh` 与 compose build 必然失败 | DEPLOYMENT 快速启动不可用；修复后补 L3 compose 冒烟 |
| 15 | metrics 端口三处矛盾（compose 9092 映射 / Dockerfile EXPOSE+死 ENV / 文档"无独立指标端口"） | 清理 compose 与 Dockerfile |
| 16 | 文档漂移：`to_u128()` 示例（实为 `as_u128`）、DEPLOYMENT §8.5 旧端点清单、FAQ 吞吐数字无基准支撑、漏洞报告渠道两说、dc_id 0-7 vs ≤31、USER_GUIDE 仍列 sqlite/MySQL | 逐项修正并纳入 FAQ 已知问题 |
| 17 | `api_test.sh` 的 `/api/v1/metrics` 用例与默认生产 URL、workspace 不清理 | 脚本修复 |
| 18 | proto 生成物手工维护无契约守卫；examples 无自动化运行 | 契约测试 + sdk example 冒烟 |

---

## 统计汇总

- 场景总数：**16 域约 190 个**（正常流 + 异常流 + 边界 + 故障注入），其中已有自动化覆盖约 8 成（L0+L1 ≈1879 个测试函数），明确缺口 18 项（P0×5 / P1×8 / P2×5）。
- 特性组合：全部组合可构建（`--all-features` 已解禁，`--no-default-features` 仅作编译检查），行为差异点 11 处（§2.3）。
- 覆盖率基线：本地 `llvm-cov` 全特性口径约 93.6%（sdk 代码测试密度低）；CI default leg ≥95% 权威门禁。
