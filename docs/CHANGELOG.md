# 📋 Nebula ID 更新日志

本文件记录 Nebula ID 的全部重要变更。

格式基于 [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)，
版本号遵循 [语义化版本](https://semver.org/spec/v2.0.0.html)。

[🏠 主页](../README.md) • [🔒 安全文档](SECURITY.md) • [⚡ 性能指南](PERFORMANCE.md)

---

## 📋 目录

- [Unreleased](#unreleased)
- [0.2.0 - 2026-07-23](#020---2026-07-23)

---

## [Unreleased]

接线修复与 SDK 强化（specmark change `wiring-and-sdk-hardening`）＋ 密钥轮换宽限期与配置
fail-fast（change `key-rotation-and-config-failfast`）。**含多项行为变更，部署方需注意。**

### Breaking（破坏性 API 变更）

- **嵌入式 SDK 全面迁移到 trait-kit 装配（change `sdk-trait-kit-migration`）**：
  `NebulaIdClient` / `NebulaIdClientBuilder` 移除，新门面为 `NebulaIdKit` /
  `NebulaIdKitBuilder`（装配经 trait-kit AsyncKit 依赖图校验，缺失依赖/环在
  `build()` 期报错）。迁移对照：

  | 旧 API（已移除） | 新 API |
  |---|---|
  | `NebulaIdClientBuilder::new(config)` | `NebulaIdKitBuilder::new(config)` |
  | `.with_audit_logger(logger)` | `.with_audit_logger(logger)`（同名） |
  | `.with_repository(repo)` | `.with_repository(repo)`（同名） |
  | `.build().await?` → `NebulaIdClient` | `.build().await?` → `NebulaIdKit` |
  | `client.generate(ws, group, tag).await?` | `kit.id_generator()?.generate(ws, group, tag).await?` |
  | `client.batch_generate(ws, group, tag, n).await?` | `kit.id_generator()?.batch_generate(ws, group, tag, n).await?` |
  | `client.generate_with_algorithm(alg, ..).await?` | `kit.id_generator()?.generate_with_algorithm(alg, ..).await?` |
  | `client.health_check().await` | `kit.health_check().await`（算法级快照，语义不变；模块级报告用 `kit.health_report()`） |
  | `client.shutdown().await` | `kit.shutdown().await` |
  | `client.has_repository()` | `kit.has_repository()` |
  | `client.router()`（逃生舱） | **已移除**：`AlgorithmRouter` 不再出现在 SDK 公共面，Segment 无仓储的守卫不再可绕过 |

  生成语义不变：纯算法（snowflake/uuid_v8）零 DB 可用；Segment 需
  `.with_repository(..)`，未注入时返回 `CoreError::ConfigurationError`。
  `IdGenerator` handle 为 `Clone + Send + Sync`，可跨任务共享（比缓存整个 Kit 更轻）。

### Changed（行为变更）

- **Snowflake 毫秒轮转等待改为混合 spin-then-sleep**：`wait_for_next_ms` 此前
  固定 1ms 周期 `tokio::time::sleep`，定时器粒度过冲把持续发号吞吐压到位宽
  理论上限（1023 ID/ms ≈ 102 万 ID/s，默认 10 位 sequence）的一半左右；现先
  自旋（≤2ms 预算，覆盖毫秒轮转的亚毫秒等待），预算耗尽仍未越过目标毫秒
  （时钟回拨等待场景）才回落 1ms sleep，避免长等待空转。基准确认串行 / 批量
  / 超大批量三口径全部收敛到位宽上限（约 2.1×，见
  [PERFORMANCE.md · 已记录基线](PERFORMANCE.md#已记录基线)）；语义不变——
  仍严格等待真实时钟越过目标毫秒后才发号。同步新增
  `snowflake/batch_generate_10000` 基准用例（单次 `block_on` 摊薄到 1 万 ID，
  近似「纯 reserve 路径」下限口径）。

- **HTTP workspace/group 读接口租户隔离**：`GET /api/v1/groups` 此前未做任何
  角色校验，按客户端提供的 `?workspace=` 任意列取任意租户的 group；现收敛为
  User-only 端点（Admin 调用返回 403），且 User key 仅可列自身 workspace
  （他人 workspace 返回 403 `workspace_mismatch`）。`GET /api/v1/workspaces`
  对 User key 过滤为仅自身 workspace（`total` 随之收敛），`GET
  /api/v1/workspaces/{name}` 对 User key 仅放行自身 workspace（他人 → 403）；
  **Admin 保持全量与跨租户读（行为不变）**。归属判定复用与 gRPC 同源的
  `authorize_workspace_access` 单点。
- **统一错误信封追加 `business_code`（Minor，非 Breaking）**：所有 HTTP 错误
  响应（含限流、请求体超限、API 版本错误）统一为单一信封
  `ErrorResponse { code, business_code, message, details, request_id,
  timestamp }`。`business_code` 为四位数字串（如 `"4001"`），与 HTTP 状态码
  同源于 `CoreError → (状态码, 业务码)` 单张分类表，客户端应以它做语义分支
  而非解析 `message` 文案；`request_id`（UUID v4）与 `timestamp`（毫秒级
  Unix 时间）在响应装配处生成，可与审计日志关联。OpenAPI 错误码表已与实际
  映射对齐（`2002`/`2004` 标注为预留码，当前不返回）。定级理由：对既有
  客户端 JSON 字段为**纯增量**（未知字段容忍即不受影响），且被移除的
  `ApiErrorResponse` 从未出现在任何端点响应与 SDK 面上——仅影响直接引用
  server `models::ApiErrorResponse` 的 Rust 代码（源码级，已由单信封承接）。
- **HTTP TLS fail-fast**：`tls.enabled=true` 且证书缺失/解析失败时拒绝启动
  （不再静默降级明文）；仅 `enabled=false` 允许明文。
- **gRPC 启用 API key 认证**：NebulaIdService 全 RPC（含双向流）在各入口经
  单点 `authenticate()` 校验 `authorization`（Basic/ApiKey）。设计稿原定 tonic
  `with_interceptor`，但拦截器 `call` 是同步签名而凭证校验必须异步查库
  （Argon2id + DB），故改为各 RPC 入口一行调用同一助手 —— 单点实现不变。
  **未带凭证的 gRPC 客户端将收到 `Unauthenticated`**；`auth.enabled=false` 时放行。
- **gRPC 认证失败码区分**：key 被禁用或已过期 → `PermissionDenied`；凭证本身
  无效 / key 不存在 → `Unauthenticated`（落实 R-auth-003）。此前一律 401。
- **全局限流真实生效**：`RateLimitMiddleware` 从"仅 Extension 注入（死代码）"
  改为真实挂载到 HTTP 栈；`POST /config/rate-limit` 热更新作用于实际流量。
- **限流层序、开关与响应头（含破坏性变更）**：
  - 限流层移到 CORS / 安全头**内侧**：旧顺序下 429 由最外层短路生成，客户端拿不到
    CORS 头，且 OPTIONS 预检会消耗真实配额。
  - `[rate_limit].enabled = false`（顶层段，非 `auth.rate_limit.enabled`）现在真的不挂
    限流层（`src/main.rs` 读取 `config.rate_limit.enabled`；此前该配置无消费点）。
  - `POST /config/rate-limit` 复用启动期 `Config::validate`，`burst > 10×rps`
    这类运行期不一致组合被拒绝（此前只有单字段 range 校验）。
  - 限流响应头名单源统一：此前 CORS 暴露 `x-rate-limit-remaining`，而中间件写出的
    是 `x-ratelimit-remaining`（header 名连字符敏感），浏览器永远读不到；两处现共用
    `rate_limit::middleware` 常量，写出名为规范小写 `x-ratelimit-limit` /
    `x-ratelimit-remaining`，429 额外带 `x-ratelimit-remaining: 0`。
- **认证缓存（`auth.cache_ttl_seconds`）接线**：存储是本项目自研
  `MemoryGarrisonDao`（实现 garrison `GarrisonDao` 接口的进程内 HashMap），
  校验先查缓存、未命中回源 DB；缓存值仅含 workspace_id/role/过期时间（不含 secret）。
  吊销/轮换/禁用对**本进程**即时失效；多节点部署时其他节点最长滞后一个
  `cache_ttl`，`last_used_at` 同样滞后（运维口径见 docs/DEPLOYMENT.md 7.1）。
  `cache_ttl_seconds = 0` 时不再装配缓存实例。
- **认证缓存不再接受 glob 形态的 key_id**：`invalidate` 改为精确前缀匹配，
  含 `*`/`?` 的 key_id 不再能一次清掉其他主体的全部条目。
- **HTTP 认证头解析单源**：`Authorization` 的 Basic/ApiKey 解析合并为唯一实现
  （新增带原因的 `parse_authorization_header_detailed`）；此前 HTTP 与 gRPC 各
  一份，且空凭证判定不一致。审计 reason 与 i18n 文案保持不变。
- **biz-tags 租户隔离（IDOR 修复）**：`GET /api/v1/biz-tags` User 角色仅见本
  workspace（忽略 workspace 参数覆盖）；Admin 按参数过滤；无仓储时不再静默
  回退 nil 查询。
- **请求归属真实到 IP**：HTTP serve 注入连接对端地址（DualListener 用本 crate
  新类型 `PeerAddr` 承载 `ConnectInfo`）。此前 `get_client_ip` 在生产恒 `None`，
  限流键、认证失败计数与审计 `client_ip` 全部落入共享 `anonymous` 单桶 ——
  单个攻击者可把 `/health`、`/metrics` 等一并打成 429。
- **TLS 握手不再串行**：每连接独立任务 + 10s 握手超时，未认证客户端"建连后不发
  ClientHello"无法再冻结整个监听端口（未认证远程 DoS）；accept 错误增加退避。
- **metrics 端到端闭环**：延迟样本改由路由层观测（HTTP / gRPC / SDK 三条入口共用
  唯一记录点，按实际服务的算法归因），`/metrics` 逐算法暴露 p50/p99/p999 与
  `clock_backwards`；告警规则 `clock_backward` 改用真实回拨计数（此前"该算法有延迟
  样本"即触发，跑过请求就恒真）。**破坏性变更**：`AlgorithmMetrics` 的
  `p50/p99/p999` 公开原子字段被移除，改由 `latency_percentiles_ns()` /
  `get_p*_latency_ms()` 读取；`MetricsSnapshot` 新增 `clock_backwards` 字段。
- **`p50/p99` 语义**：由"历史最大值（只增不降）"改为最近 1024 样本环形缓冲的
  真实分位数（单次排序取三档）。
- **`DatabaseConfig::default` 不再 panic**：默认值不再 `.expect()` 环境变量
  `NEBULA_DATABASE_PASSWORD`（纯算法嵌入方只需 `Config::default()`）；建立连接时
  仍 fail-fast 返回 `ConfigurationError`。
- **`tls.min_tls_version` 现在真实强制**：此前该配置只写日志、不进 `ServerConfig`，
  实际恒为 rustls 默认的 TLS 1.2+1.3（旧注释已承认"延后到 v0.3.0"）。HTTP 侧改用
  `ServerConfig::builder_with_protocol_versions`，`tls13` 时低于 TLS 1.3 的
  ClientHello 被直接拒绝（**行为变更**：老客户端可能开始握手失败）。gRPC 侧的
  `ServerConfig` 由 tonic 装配、无法注入版本集合，启动时按 `warn` 如实标界。
- **`algorithm_type` ENUM 与代码对齐（需 DB 迁移）**：`scripts/init.sql` 此前仍建
  `('segment','snowflake','uuid_v7','uuid_v4')`，而代码侧只有
  `segment/snowflake/uuid_v8`，新库写入 `uuid_v8` 会被拒绝。存量库按
  `docs/CONFIG_MIGRATION_GUIDE.md` 的 ENUM 迁移章节执行（含 NULL 回填与回滚）。
- **坏配置不再静默降级为默认值**：此前 `main.rs` 对 `Config::load_from_file` 的
  `Err` 分支只打一条 `error!` 日志，然后用 `Config::default()` 继续启动，进程带着
  一份"想象中的配置"跑起来（例如 `[algorithm.uuid_v8]` 写错段名时实际跑的是默认
  算法）。现在文件存在但读失败或解析失败一律致命退出（exit 1），错误经
  `Termination` 打到 stderr 并点名失败字段。仅"未显式给 `--config` 且默认路径文件
  不存在"这一种情况仍回落内置默认值并 warn。
- **配置未知键一律拒绝**：新增 17 处 `#[serde(deny_unknown_fields)]`（分布在
  `src/core/config/` 的 10 个文件）。此前全仓**一处都没有**——顶层 `Config`
  也不例外，因此任何段名/键名拼错都会被 serde 静默丢弃，整段设置无声失效。
  **破坏性变更**：存量配置里任何误写的键现在都会阻止启动，
  升级前先用 `docs/CONFIG_MIGRATION_GUIDE.md` 的预检命令跑一遍。
- **密钥轮换宽限期真正生效且默认关闭**：此前配置值确实被读到并传进仓储，但
  `rotate_api_key` 的形参写作 `_grace_period_seconds`（下划线前缀、完全不读），
  轮换直接覆盖 `key_secret_hash` → 旧密钥当场失效，宽限期形同虚设。现在
  `>0` 时轮换会把上一代哈希和窗口截止时刻落库，旧密钥在宽限窗口内仍可验证、
  到期自动失效（惰性按 `rotate_expires_at` 判定），超过 30 天会被钳制；
  `key_rotation_grace_period_seconds` 的默认值由 7 天改为 `0`（宽限期关闭），
  因为旧行为等价于"没有宽限期"，默认开启会改变既有部署的安全窗口语义。
  `api_keys` 表新增 `prev_secret_hash` / `rotate_expires_at` 两列；两列由启动期
  `run_migrations` 自动幂等补齐（`ADD COLUMN IF NOT EXISTS`，失败即终止启动，
  `src/core/database/connection.rs:310-339`），**常规部署无需手工 DDL**。仅当数据库账号
  无 DDL 权限、或 schema 由外部工具管控时需手工执行，见
  `docs/CONFIG_MIGRATION_GUIDE.md` 的"密钥轮换宽限期"章节。
- **第二个 admin key 首次真正被拒**：创建 API key 与吊销 API key 的 admin 守卫
  此前用 `list_api_keys(workspace_id = NULL)` 分页查询 + 内存扫描，而全局 admin
  key 的 `workspace_id` 是 NULL，`NULL = nil_uuid` 在 SQL 中不成立 → 查询恒空 →
  守卫从未生效，持 admin 凭证者可以再开一个 admin key 作为持久化后门。改用
  行 id 精确查询与 SQL 侧 `role='admin' AND enabled=true` 计数（无 workspace
  过滤、无 1000 行分页上界）。**注意作用范围**：守卫只在 HTTP `POST /api-keys`
  上生效，启动期环境/配置引导的 admin key 走仓储层直插，不经过守卫。
- **特性模型解禁 `--all-features`**：dbnexus 的 `pub use sea_orm` 不再绑定驱动
  feature（`dbnexus` 40580df，sea-orm prelude 只依赖 macros/with-chrono/with-uuid，
  与驱动无关），无驱动构建不再报 187 个错误；`sqlite` feature 删除（此前
  default 恒含 dbnexus/postgres 使其必然触发引擎互斥 `compile_error!`，从未可
  构建），`--all-features`（= default + etcd + alerting + sdk + integration-tests
  + openapi）自此可构建。CI（clippy/test/coverage）、lefthook、预检脚本与全部
  文档统一收敛到 `--all-features` 口径；fmt-clippy job 新增
  `--no-default-features --lib --bins` 编译检查钉住 garrison 回退路径；
  test matrix 由 default/postgresql/etcd（postgresql 与 default 等价、冗余）
  改为 default/all，独立 sdk job 并入 all leg（只跑测试；覆盖率门禁仍以
  default leg ≥95% 为权威——sdk 代码测试密度低，进 all leg 门禁会挂）；
  本地覆盖率门禁补 proto 排除正则并改用全特性口径实测阈值 90%；
  删除以旧约束为前提的文本守卫测试
  `repo_docs_guards_tests.rs` 及 2 处 sqlite 死测试。运行时 `DatabaseEngine`
  枚举的 `Sqlite`/`Mysql` 变体保留，但构建不含对应驱动，连接会失败。
- **修复号段唯一约束的迁移漂移（真库 bug）**：`scripts/init.sql` 的
  `nebula_segments` 建表 DDL 带 `uq_nebula_segments_ws_tag_dc UNIQUE
  (workspace_id, biz_tag, dc_id)`，而 `run_migrations` 的同表 DDL 漏了它；
  生产代码 `allocate_segment` 的首查无行路径 `INSERT ... ON CONFLICT
  (workspace_id, biz_tag, dc_id) DO NOTHING` 依赖该约束，经自动迁移建表的
  库上首次号段分配必然报错。迁移现于建表 DDL 内联该约束，并对存量库补
  幂等回填（`DO $$ ... EXCEPTION WHEN duplicate_object`）；回填失败终止
  启动（缺约束 = 多实例首分配不可用，fail-closed）。此修复同时让 7 个
  因 mock 配额欠账（未随迁移语句增长更新）而长期失败的迁移测试转绿，
  **当前全量测试 0 失败**，本地覆盖率门禁随之移除 `--skip`。
- **许可表述统一为 Apache-2.0**：仓库唯一许可文本 `LICENSE` 自始为
  Apache-2.0，`LICENSE-MIT` 从不存在（FAQ 旧文引用的 `../LICENSE-MIT`
  为死链），双许可声明（Cargo.toml 元数据、README 徽章与尾注）是空承诺。
  Cargo.toml `license` 字段、README 双语徽章与尾注、FAQ 许可章节统一为
  Apache-2.0。
- **文档事实性修正**（遗留 WIP 收口）：`database.engine`/连接池默认值/
  `dc_id` 范围（0-7）等配置文档与实现对齐；`nebula_segments` 手工建表
  DDL 对齐 `init.sql`（VARCHAR(255)、`dc_id` 列、唯一约束）；部署架构图
  更正为指标经主端口 `/metrics` 暴露（`METRICS_PORT` 等环境变量当前无
  代码消费者，属遗留死配置）；FAQ 的 Rust 基础镜像、限流描述、许可
  章节与链接修正。

### Added

- **嵌入式 SDK（feature `sdk`）**：`NebulaIdClientBuilder` / `NebulaIdClient`，
  `build()` 收拢分布式锁注入 / 路由初始化 / 降级后台任务；`examples/embedded.rs`
  （零 DB 零网络）与 `examples/sdk_server.rs`（sdforge `#[forge]` 封装 +
  OpenAPI）。
- **CI/构建**：`ci.yml` clippy 与 test matrix 改单特性（`--features etcd`，
  因 dbnexus 禁止 sqlite+postgres 混用）；`docker/Dockerfile` 适配单包
  `nebulaid` 构建；新增 `openapi` 镜像特性。
- **配置错误分类**：`ConfigError::FileNotFound`（区分"文件不存在"与"文件存在但
  读/解析失败"），使启动期判定矩阵可以只在前者回落默认值。
- **启动期配置判定策略函数** `resolve_startup_config(path, explicit_path)` 与
  `StartupConfig`：把"显式给路径 / 默认路径 / 文件是否存在 / 是否解析成功"
  四种组合的取舍收敛成一处可单测的纯函数（`src/core/config/app_config.rs`），
  `main.rs` 只消费其返回值。
- **轮换宽限期可观测**：`ApiKeyWithSecret` 新增 `grace_expires_at` 字段（宽限期
  未启用或未处于宽限窗口时为 `None`）；新增宽限期与 admin 守卫相关 i18n 文案键。
  该结构体字段全 `pub` 且无 `#[non_exhaustive]`，因此对**外部构造方是源码级
  破坏性变更**（必须补上新字段），crate 内构造点已同步。

### Fixed

- **日志管道容量级阻塞导致的全文服务停顿**：inklog 主 async 通道为
  `bounded(channel_capacity)`（默认 10000），file sink 未显式配置路径时无
  消费者排水；服务累计 ~1 万条日志后每条日志经 `send_timeout(100ms)` 阻塞
  再降级。审计 `log_shared` 又把环满的 `warn!` 横跨互斥锁发出，全部请求的
  审计写入按 100ms/条串行化——实测运行 ~1 万请求后 HTTP 吞吐从 ~1.4 万 rps
  塌到 ~6 rps（每请求 ~11s，服务端 CPU 空闲）。修复两层：`init_observability`
  显式配置 inklog file sink（通道持续排水）；丢弃告警移到互斥锁外（日志后端
  再慢也不串行化请求）。回归测试 `tests/audit_ring_stall_repro.rs` 灌满通道
  后断言 log() 不阻塞。修复后 c=64 实测：/health 13,675 rps（p99 16ms）、
  /generate 4,974 rps（p99 39ms）、批量 100 ID 达 ~28.4 万 ID/s。
- **全新数据库迁移不再失败**：`nebula_segments` 的号段唯一约束回填 DO 块只
  接 `duplicate_object`（42710），而全新库路径下建表 DDL 的内联同名约束
  `uq_nebula_segments_ws_tag_dc` 已先行创建同名索引，`ADD CONSTRAINT` 报的是
  "relation already exists"（42P07 `duplicate_table`），未被吞掉 → 全新库
  首次启动迁移必失败退出。现同时接住两个条件，存量库升级与全新库初始化
  两条路径均幂等（钉子测试已同步）。
- **停机不再泄漏后台任务**：`rate_limit_cleanup` 与降级巡检任务此前只在
  `shutdown_signal` 分支回收；服务器先退出（正常停止或错误返回）时二者泄漏，
  且 tokio 运行时 drop 会一直等这个永不自退的循环任务。改为 select 各分支只产出
  结果、退出后统一回收（etcd / 非 etcd 两块同构）。
- **矛盾 TLS 配置不再静默明文**：`tls.enabled = false` 且
  `http_enabled`/`grpc_enabled = true` 时，per-port 开关被忽略、端口按明文启动，
  此前无任何提示；现按 `tls_config_conflict` 显式 warn。启动日志里
  "HTTPS is enabled but using HTTP fallback for now" 的误导文案改为如实描述
  （该端口确实做 TLS 终结）。
- **`docker/vendor-deps.sh` 不再打包凭据**：向 `.docker-vendor/` 打依赖副本时
  增加 `.env*` / `local_settings*` 排除（修复前实测确实带入了依赖仓库的
  `.env.test` / `.env.example`）。
- **SDK 示例不回显内部错误**：`to_api_error` 此前把 `e.to_string()` 直接发给客户端
  （sdforge 将 message 原样下发）；改为对外固定概要 + `error_id`，内部细节只进日志。
- **locale 键集对齐守卫**：新增测试断言 `locales/en.yml` 与 `locales/zh-CN.yml`
  顶层键完全一致（单侧缺键只会在该语言下静默回退）；同时删除 `min_tls_version`
  改造后成为孤儿的三个键（en + zh）。
- **snowflake 并发重复 ID 竞态**：串行化 `(last_timestamp, sequence)` 迁移，
  并发下不再产生重复。
- **降级链去重**：fallback 链 `[Snowflake, UuidV8]`（去除重复占位）。
- **main.rs 卫生**：删除非 etcd 分支重复 router 构造；hot_reload watcher 启动
  条件与 feature 解耦。
- **含非 ASCII 的配置文件不再 panic**：`Config::load_from_file` 在打 debug 日志前
  对 `[auth]` 段做 `&expanded[start..start+100]` 字节切片，配置文件含中文注释或
  中文值时该下标不落在字符边界上 → 进程在 TOML 解析之前 panic（与日志级别无关，
  实测 exit 101）。改为按字符截断。回归用例见 `app_config.rs` 的
  `load_from_file_with_non_ascii_comments_does_not_panic`。
- **删除无消费点的配置死键**：`config/config.toml` 与 `config/config_test.toml`
  中的 `[logging] backtrace = false` —— `LoggingConfig` 无该字段且全仓零引用，
  此前一直被静默忽略；未知键一律拒绝后它会让这两个配置无法启动，故先清掉。
- **`test_uuid_v8_counter_resets_on_millisecond_rollover` 不再 flaky**：该用例
  原先假设"连续两次生成必定跨毫秒"，在快速机器上隔离运行 8 次失败 2 次。改为
  从生成值内嵌的时间戳判断样本是否同毫秒，同毫秒才断言计数器 +1，并显式断言
  "4 次采样中至少有一对同毫秒"作为测试前提。

## [0.2.0] - 2026-07-23

v0.2.0 is the first release since v0.1.1, shipping 11 phases of hardening,
refactor, and developer-experience work plus subsequent security hardening.
Highlights: three strix security fixes, a dbnexus/sdforge/confers architecture
takeover, a 1829-test e2e suite at 95% coverage, garrison DAO infrastructure,
and redundant-comment cleanup.

### Added

- **garrison DAO infrastructure** (`src/server/auth/memory_dao.rs`): full
  in-memory `GarrisonDao` implementation (TTL, glob, atomic get_and_delete /
  incr / decr / CAS) for garrison `ApiKeyHandler`. Feature-gated under
  `garrison-auth`; not yet wired into the request path (migration design in
  `temp/garrison-migration-plan.md`, deferred to a later change).
- **e2e test suite** (commits 36c8bf8, fe10013, f7ac3be): 1829 end-to-end
  tests covering 76 functional scenarios (95% module coverage); fixed
  `snowflake.rs` `batch_generate(0)` boundary bug and `audit/logger.rs`
  per-event `sync_all` performance bottleneck (added `flush()` interface).
- **distributed analysis report** (`specmark/reports/distributed-analysis.md`):
  2.5/5-star assessment with P0/P1/P2 improvement roadmap (EtcdWorkerAllocator
  dead code, Dockerfile etcd feature, WORKER_ID env var).

### Changed

- **Architecture takeover** (commit bc4980c): database / HTTP / gRPC / config
  fully delegated to `dbnexus` / `sdforge` / `confers`; Cargo.lock now tracked
  in VCS.
- **strix security fixes** (commit 9a26926): IDOR in BizTag endpoints (added
  workspace verification), config mutation (moved endpoints to admin routes),
  metrics leak (replaced DB error strings); plus `inklog` EnvFilter fix and
  `trait-kit` DI.
- **Cargo.toml cleanup** (commits 4e022af, d573317): removed unused deps,
  updated all deps to latest, fixed 9 version-format violations (rule 25).
- **Redundant-comment cleanup** (`src/core/coordinator/etcd.rs`): removed 39
  decorative separators and code-restating comments; 63 etcd tests still pass.

### Fixed

- **TOCTOU race** in `DatabaseConfig::default` (commit 9b9b884).
- **worker_id start from 1** in etcd allocator + HangingPingClient coverage
  (commit 240b9eb).
- **i18n locale test isolation** via `LOCALE_LOCK` (commit 70bfc3e).

### Security

- **strix-0001 (IDOR)**: BizTag endpoints lacked workspace verification.
- **strix-0002 (config mutation)**: config endpoints exposed to non-admin.
- **strix-0003 (metrics leak)**: DB error strings leaked in metrics.

### Deprecated

- Nothing deprecated in v0.2.0.
