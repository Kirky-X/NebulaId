# 📦 自研依赖生态治理 ADR

> 本文档是 Nebula ID 自研依赖（Kirky.X 生态）的治理决策记录（ADR）：以**现行路径依赖策略**为基线，登记八项自研依赖的版本波次、切回 registry 的触发条件与步骤、以及每项的退出策略与回退点。基线提炼自 `Cargo.toml` 的 `[dependencies]` 头注与 `[patch.crates-io]` 注释，二者与本文件的口径一致时以 `Cargo.toml` 为准。

## 📋 目录

- [1. 基线：现行路径依赖策略](#1-基线现行路径依赖策略)
- [2. 八项自研依赖总览](#2-八项自研依赖总览)
- [3. 切回 registry：触发条件与通用步骤](#3-切回-registry触发条件与通用步骤)
- [4. 逐项退出策略与回退点](#4-逐项退出策略与回退点)
- [5. garrison 双轨处置决策记录](#5-garrison-双轨处置决策记录)
- [相关文档](#相关文档)

---

## 1. 基线：现行路径依赖策略

1. **本地工作副本路径依赖**：自研依赖一律以兄弟目录工作副本为源——`../base/`（confers、oxcache、dbnexus、sdforge、inklog、limiteron、trait-kit）与 `../garrison/`，随 base 仓库波次实时联动，本仓库不引用 rc 期的 registry 快照。
2. **path + version 双声明**：`[dependencies]` 中每项自研依赖同时写 `path` 与 `version`。`version` 字段平时用于声明意图与一致性校验，正式版发布后可按需保留并切回 registry。
3. **`[patch.crates-io]` 统一传递源**：路径依赖只作用于本 manifest 的**直接**声明；garrison、sdforge 等自研 crate 之间的**传递** registry 需求必须由 `[patch.crates-io]` 强制统一重定向到本地工作副本，否则 pre-release 语义下 path 与 registry 候选不自动合并，会出现双源并存（API 漂移时表现为下游 crate 编译失败）。
4. **构建需兄弟目录存在**：与路径依赖的代价相同，`[patch.crates-io]` 使构建依赖 `../base`、`../garrison` 两个工作副本存在；缺失时 `cargo` 解析直接失败。
5. **Docker 侧配套**：Docker 构建上下文无法越过仓库边界，`docker/vendor-deps.sh` 把被 patch 的仓库打包进 `.docker-vendor/`，再经 BuildKit 命名上下文（`docker/build.sh`、compose `additional_contexts`）注入。该 vendor 流是路径依赖切换的必要配套，与本文档的基线同步演进。
6. **可构建特性集上限**：dbnexus 禁止混用 embedded（sqlite/duckdb）与 server 端（postgres/mysql）feature；本项目 default 恒含 postgresql，故 `sqlite` feature 与 `--all-features` 不可构建，可构建最大集合为 `default + etcd`（详见 `Cargo.toml` feature 注释）。

## 2. 八项自研依赖总览

| crate | 当前版本波次 | 路径 | 本项目消费面 | 接线位置 |
|-------|-------------|------|-------------|---------|
| confers | 0.6.0-rc.4 | `../base/confers` | feature `interpolation`/`watch`；配置管理与 `${VAR}` 展开 | `src/core/config/` |
| oxcache | 0.5.0-rc.4 | `../base/oxcache` | 多级缓存抽象 | 缓存层（`src/core/`、`src/server/auth/cache.rs`） |
| dbnexus | 0.6.0-rc.4 | `../base/dbnexus` | `default-features = false` + `runtime-tokio-rustls`/`with-chrono`/`with-uuid`，`postgresql` feature 经其开 `dbnexus/postgres` | `src/core/database/` |
| sdforge | 0.5.0-rc.4 | `../base/sdforge` | `default-features = false` + `http`/`grpc`/`openapi`/`context`/`docs`；`#[forge]` 服务装配与服务发现 | `src/server/`、`src/sdk/` |
| inklog | 0.3.0-rc.4 | `../base/inklog` | `default-features = false` + `fast-masking`（日志自动脱敏）；日志初始化由其接管 | `src/main.rs` |
| garrison | 0.9.0-rc.1 | `../garrison` | optional（`garrison-auth` feature）+ `web-axum`/`protocol-apikey`/`cache-memory`；**代码实际仅用 `dao` trait 与 error 类型**（见第 5 节） | `src/server/auth/` |
| trait-kit | 0.5.0-rc.5 | `../base/trait-kit` | optional（`sdk` feature）+ `async`/`lifecycle`/`health`；SDK 的 Kit 装配 | `src/sdk/` |
| limiteron | 0.3.0-rc.4 | `../base/limiteron` | `default-features = false`，仅消费 core 原语（`TokenBucketLimiter`/`ConcurrencyLimiter`/`Limiter`） | `src/server/rate_limit/` |

> 版本波次说明：八项同处一个 RC 生态波次（garrison 0.9.0-rc.1 内部亦消费 confers 0.6.0-rc.4 / oxcache 0.5.0-rc.4 / sdforge 0.5.0-rc.4 / inklog 0.3.0-rc.4；trait-kit 锁 0.5.0-rc.5 与之同生态）。升级须整波联动，禁止单点跨波。

## 3. 切回 registry：触发条件与通用步骤

**触发条件**：对应自研 crate 发布正式版（非 `-rc.*` 的 semver 版本）。rc 期禁止切回——registry 上的 rc 候选与工作副本并存正是 `[patch.crates-io]` 要消灭的双源状态。

**通用步骤**（按 crate 逐项执行，全部完成后才可下线兄弟目录）：

1. 确认该 crate 已发正式版，且本仓库消费的 API 在正式版中无破坏性变更（rc → 正式若改 API，先在工作副本完成迁移再发版）。
2. `[dependencies]` 中移除该项的 `path` 字段，`version` 上浮到正式版（按 semver 上浮，如 `0.6.0-rc.4` → `0.6`）。
3. 删除 `[patch.crates-io]` 中对应条目；若八项条目全部移除，整节删除。
4. `cargo update -p <crate>` 刷新 `Cargo.lock`，并用 `cargo tree -i <crate>` 确认全图单源（不再出现 path/registry 双条目）。
5. Docker 侧同步收窄：`docker/vendor-deps.sh` 移除对应 `vendor_one` 调用，compose `additional_contexts` 与 `docker/build.sh` 的 `--build-context` 移除对应行；`Dockerfile` 中对应 `COPY --from=<ctx>` 行移除。
6. 该 crate 不再被任何 patch 条目与 vendor 脚本引用后，兄弟目录方可下线；`docker/vendor-deps.sh` 在全部条目切回后整体删除。

**回退点**：任一步骤失败（正式版 API 不兼容、双源解析冲突），恢复 `path` 声明与 patch 条目即回到基线；`Cargo.lock` 以 `git checkout Cargo.lock` 复原。

## 4. 逐项退出策略与回退点

> 「退出」指彻底移除该自研依赖、改用社区等价物或直连底层库。优先级标注：**优先评估** = 替换成本低、收益明确，应在下一次依赖治理窗口优先盘点；**合理保留** = 当前形态已是合理稳态，无迁移必要。

### confers —— 优先评估

- **回退点**：`config` crate（toml 反序列化 + 手写 `${VAR}` 展开）。
- **评估要点**：本项目消费面集中在配置加载、`interpolation`（`${VAR}`/`${VAR:default}` 展开与嵌套深度防护）与 `watch`。`interpolation` 已可由社区 crate（如 shellexpand 类）或手写替代；`watch` 若热加载被配置管理服务取代则自然消失。消费面小、无深度耦合，替换成本可控。

### oxcache —— 优先评估

- **回退点**：`moka`（社区标准同步/异步缓存）。
- **评估要点**：本项目以其多级缓存抽象承载 Redis/内存两级；`moka` 只覆盖进程内缓存，切换需自建「本地 LRU + Redis」两级编排。garrison 的 `GarrisonDaoOxcache` 适配面（`src/server/auth/cache.rs`）是主要迁移点。替换收益取决于是否真的用到多级拓扑——若实际只用内存层，`moka` 直替成本很低。

### inklog —— 优先评估

- **回退点**：`tracing-subscriber`（`tracing` 已是直接依赖， subscriber 层可直连）。
- **评估要点**：消费面为日志初始化接管与 `fast-masking`（Aho-Corasick 多模式凭证脱敏）。脱敏是合规刚需，切换前必须确认 `tracing-subscriber` 侧有等价 layer（自写 `Layer` 做字段级掩码）或放弃自动脱敏——后者不可接受。评估结论为「有等价脱敏 layer 方案则迁，否则保留」。

### dbnexus —— 合理保留（替换需整仓评估）

- **回退点**：直连 `sea-orm`（其本就是直接依赖，dbnexus 是其上的抽象/约束层）。
- **评估要点**：消费面在 `src/core/database/`（连接管理、迁移、仓储）。退出的实际工作是把 feature 联合（`runtime-tokio-rustls`/`with-chrono`/`with-uuid`/`postgres`）与迁移编排搬回本仓库。收益不明确，除非 dbnexus 波次长期停滞，否则不动。

### sdforge —— 合理保留（替换需整仓评估）

- **回退点**：直连 `axum` + `tonic`/`tonic-prost`（两者已因类型一致性以直接依赖存在）+ 手写 OpenAPI 注册。
- **评估要点**：`#[forge]` 宏与服务发现贯穿 `src/server/` 与 `src/sdk/`，且宏展开依赖镜像 feature 清单（见 `Cargo.toml` lints 与 feature 注释）。替换是全 server 层重构，仅在 sdforge 停止维护时启动。

### limiteron —— 合理保留

- **保留理由**：本项目只消费其 core 原语（`TokenBucketLimiter`/`ConcurrencyLimiter`/`Limiter`），且已以 `default-features = false` 把 `postgres`/`quota-control`/`ban-manager` 等全仓零使用 feature 关闭，顺带解除了对 dbnexus ^0.4 的传递约束——依赖面已收敛到最小。令牌桶/并发限流原语自写易错（并发正确性、时钟处理），保留质量收益大于引入成本。

### trait-kit —— 合理保留

- **保留理由**：仅 `sdk` feature 拉入（`dep:trait-kit`），不开 sdk 的构建依赖树零影响；它是自研生态的 Kit 能力管理中心（garrison/limiteron/dbnexus/sdforge 均提供其集成面），SDK 的 AsyncKit 模块化装配与依赖图校验（`kit.build()`）即建立在它之上。退出 trait-kit 等于放弃 SDK 的 Kit 范式，与 SDK 的设计前提冲突。

### garrison —— 见第 5 节（双轨处置）

## 5. garrison 双轨处置决策记录

- **日期**：2026-09-17（Lane W1-A 依赖治理 ADR 首版）
- **现状**：`garrison` 经 `garrison-auth` feature 启用，声明 `web-axum` + `protocol-apikey` + `cache-memory` 三个 feature；但代码的实际消费面只有 `garrison::dao::{GarrisonDao, GarrisonDaoOxcache}`（`src/server/auth/cache.rs`）与 `garrison::dao::GarrisonDao` + `garrison::error::*`（`src/server/auth/memory_dao.rs`）——即 **web-axum/protocol-apikey/cache-memory 三个 feature 已启用，代码仅用 dao trait**，中间件与 ApiKeyHandler 面未被接线。功能上 API key 验证仍走本仓实现，garrison 以「KV 存储抽象」身份被消费。
- **决策**：**暂维持现状**。feature 面与实际消费面不对齐虽是债务，但收敛它有两个方向（向上迁移：把认证主路径切到 garrison 的 web-axum/protocol-apikey；或向下降级：关掉未用 feature、仅留 dao 所需最小集），两者都改变认证行为面，需要独立评审与 e2e 回归，不属于文档治理窗口的改动范围。
- **后续**：迁移或降级留待专门变更；届时按第 3 节通用步骤同步处理 `Cargo.toml`、Docker vendor 流与本节决策记录。

## 相关文档

- [`Cargo.toml`](../Cargo.toml) —— `[dependencies]` 头注与 `[patch.crates-io]` 注释（本文件口径的源头）
- [`docs/ARCHITECTURE.md`](ARCHITECTURE.md) —— 外部库角色与模块依赖关系
- [`docker/vendor-deps.sh`](../docker/vendor-deps.sh) —— Docker 构建的路径依赖 vendor 配套
