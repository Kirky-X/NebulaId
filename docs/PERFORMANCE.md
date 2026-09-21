# ⚡ Nebula ID 性能指南

> 本文档描述 Nebula ID 的基准口径、热路径设计要点与性能优化建议。所有内容以仓库内实际存在的基准与代码为准；文档不发布未经基准框架产出的吞吐数字。

[🏠 主页](../README.md) • [🏗️ 架构文档](ARCHITECTURE.md) • [🧪 测试场景矩阵](TEST_SCENARIOS.md)

---

## 📋 目录

- [基准口径与诚实声明](#基准口径与诚实声明)
- [基准套件（i18n 热路径）](#基准套件i18n-热路径)
- [基准套件（发号热路径）](#基准套件发号热路径)
- [如何运行](#如何运行)
- [热路径设计要点](#热路径设计要点)
- [构建与运行时性能配置](#构建与运行时性能配置)
- [基线记录约定](#基线记录约定)
- [已记录基线](#已记录基线)
- [优化建议](#优化建议)
- [相关文档](#相关文档)

---

## 基准口径与诚实声明

本仓库 `Cargo.toml` 中声明**两项** Criterion 基准（均为 `harness = false`）：

- `[[bench]] name = "i18n"`（`benches/i18n.rs`）：i18n 热路径，钉住性能修复的基线，保证后续 i18n 改动可量化；
- `[[bench]] name = "algorithms"`（`benches/algorithms.rs`）：发号 / 限流 / 认证缓存热路径，覆盖 Snowflake 单条与批量、路由链路、令牌桶与认证缓存命中。

**口径边界：两项均为算法 / 中间件层微基准，不覆盖 HTTP 全链路（认证中间件、TLS、序列化、网络），基线数字亦非 SLA 承诺。** Segment 算法未纳入基准（`SegmentAlgorithm` / `SegmentLoader` 为 `pub(crate)`，外部 bench 无法注入内存 stub 号段装载器）；端到端吞吐请以自身负载实测（见[优化建议](#优化建议)）。

---

## 基准套件（i18n 热路径）

`benches/i18n.rs` 定义 4 个基准函数、共 12 个基准用例：

| 基准函数 | 用例 | 覆盖路径 |
|----------|------|----------|
| `bench_translate_with_locale` | `translate_with_locale/en/no_args`、`/zh-CN/no_args`、`/fr_fallback/no_args` | 无参查表（含不支持 locale 的回退路径） |
| `bench_translate_with_locale_args` | `translate_with_locale_args/en/1_arg`、`/zh-CN/1_arg` | 带参翻译（`error.invalid_id_format`） |
| `bench_to_localized_string` | `to_localized_string/InvalidIdFormat/{en,zh-CN}`、`/RateLimitExceeded/en`、`/ClockMovedBackward/en` | `CoreError::to_localized_string`（`i18n_key()` + `i18n_args()` + 带参翻译的完整链） |
| `bench_parse_accept_language` | `parse_accept_language/typical_2`、`/longer_5`、`/unsupported_only` | `negotiate_locale_str` 的 Accept-Language 协商（2 候选 / 5 候选带 q 值 / 仅不支持的 locale） |

说明：

- 基准刻意避开 `set_locale`（全局状态），只测并发安全的 per-call `translate_with_locale*` API。
- criterion 0.8 弃用了自身的 `black_box`，基准直接使用标准库的稳定实现（`std::hint::black_box`）。

---

## 基准套件（发号热路径）

`benches/algorithms.rs` 定义 7 个基准函数（含后续补充的 batch_10000）：

| 基准函数 | 用例 | 覆盖路径 |
|----------|------|----------|
| `bench_snowflake_generate` | `snowflake/generate` | Snowflake 单条生成——经公开 `SnowflakeFactory` 构建的 `Box<dyn IdAlgorithm>`，与生产 `AlgorithmBuilder::build` 完全同一路径 |
| `bench_snowflake_batch_generate` | `snowflake/batch_generate_100` | 批量生成（单批 100 个 ID） |
| `bench_snowflake_batch_generate_10000` | `snowflake/batch_generate_10000` | 超大批量（单批 10000 = `[batch_generate].max_batch_size` 上限）；单次 `block_on` 摊薄到 10k 个 ID，近似「纯 reserve 路径」下限口径 |
| `bench_router_generate` | `router/generate_snowflake_primary` | 路由链路：默认算法 snowflake + fallback 链 `[UuidV8]`，含查表、观测面记录与分发的完整开销 |
| `bench_router_generate_uuid` | `router/generate_with_algorithm_uuid_v8` | 路由直选 UUID v8 |
| `bench_rate_limiter` | `rate_limiter/check_rate_limit` | 令牌桶命中路径 |
| `bench_auth_cache_hit` | `auth_cache/get_hit` | 认证缓存命中（sha256 键 + oxcache L1 命中 + 反序列化） |

说明：

- 每次迭代经 `rt.block_on` 驱动 async 入口（与生产调用形态一致），因此数字含每迭代的异步调度开销，高于纯 CAS 成本属预期。
- Segment 路由未纳入：`SegmentAlgorithm` / `SegmentLoader` 均为 `pub(crate)`，外部 bench 无法注入内存 stub 号段装载器；待装配处接入 `DbSegmentLoader` 后再评估是否补 Segment 路由基准。

---

## 如何运行

```bash
# 运行 i18n 基准（结果写入 target/criterion/，可用 criterion 归纳报告查看）
cargo bench --bench i18n

# 运行发号 / 限流 / 认证缓存基准
cargo bench --bench algorithms
```

`[profile.bench]` 为 `opt-level = 3`、`lto = true`，与 release 接近，可视为发号热路径的参考编译口径。

---

## 热路径设计要点

以下要点来自代码与变更记录，是影响发号吞吐/延迟的实际设计：

| 设计 | 位置 | 说明 |
|------|------|------|
| Segment 双缓冲 + 动态步长 | `src/core/algorithm/segment.rs` | 号段预领取与 `[base_step, min_step, max_step]` 动态调节（`switch_threshold` 触发），把数据库往返摊薄到号段边界 |
| Snowflake 无锁 CAS 状态字 | `src/core/algorithm/snowflake.rs` | 生成状态压缩为单原子字 `(timestamp << seq_bits) \| sequence`，批量按区间一次 CAS 预留，取代历史 `gen_lock` 全局互斥（并发重复 ID 竞态的修复沿革见 [CHANGELOG](CHANGELOG.md) Fixed） |
| Snowflake 混合等待（spin-then-sleep） | `src/core/algorithm/snowflake.rs` | 毫秒轮转等待先自旋（≤2ms 预算）后回落 1ms sleep：自旋把亚毫秒轮转收紧到真实毫秒边界，消除定时器粒度过冲；预算耗尽（时钟回拨等待）才 sleep，避免长等待空转。持续吞吐约 2.1×，实测命中 1023 ID/ms 位宽上限（见[已记录基线](#已记录基线)） |
| 批量生成 | `IdAlgorithm::batch_generate` | 一次调用申请 N 个 ID（上限 `[batch_generate].max_batch_size`，1..=10000），摊薄认证/限流/路由的每请求开销 |
| 降级链 | `src/core/algorithm/degradation_manager.rs` | 主算法不可用时按 `[Snowflake, UuidV8]` 兜底，避免号段故障放大为全量失败 |
| 环形缓冲分位数 | `src/core/types/metrics.rs` | p50/p99/p999 由最近 1024 样本环形缓冲单次排序取三档，替代「历史最大值」口径，指标开销有界 |
| 纯算法零 DB | `src/core/algorithm/{snowflake,uuid_v8}.rs` | snowflake / uuid_v8 不依赖数据库，嵌入场景以 `Config::default()` 即可发号 |
| 认证缓存 | `[auth].cache_ttl_seconds` | garrison ApiKeyHandler 校验先查进程内缓存、未命中回源 DB；`cache_ttl_seconds = 0` 时不装配缓存实例 |
| 无锁/细粒度并发原语 | `arc-swap`、`parking_lot` | 配置热更新快照与内部锁采用低开销原语 |

---

## 构建与运行时性能配置

`Cargo.toml` 中的发布口径：

```toml
[profile.release]
opt-level = 3
lto = "thin"          # thin LTO
codegen-units = 4
strip = true
panic = "abort"       # 服务型二进制：更小体积、无 unwinding 开销

[profile.bench]
opt-level = 3
lto = true
```

运行时与性能相关的配置项（全表见 [配置迁移指南 · 配置全表](CONFIG_MIGRATION_GUIDE.md#配置全表全量选项与校验规则)）：

- `[database].max_connections` / `min_connections`：连接池大小，Segment 号段领取走数据库，池过小会在号段边界排队。
- `[rate_limit].default_rps` / `burst_size`：令牌桶容量，`burst_size ≤ 10 × default_rps`。
- `[auth].cache_ttl_seconds`：认证缓存 TTL，影响每请求认证成本与吊销收敛速度的折衷。
- `[batch_generate].max_batch_size`：批量发号上限（1..=10000）。

---

## 基线记录约定

1. 每次新增/修改基准后，在同一台开发机、同一编译口径下采集 criterion 中位数。
2. 记录进本文件时注明：日期、commit、机器规格、criterion 版本。
3. 吞吐 / 延迟数字**只能**出现在下方「已记录基线」小节，且须为 criterion 产出的中位数（换算值须标注）；其余章节不得引用未入册的数字。

---

## 已记录基线

### 发号热路径（`benches/algorithms.rs`）

| 条目 | 值 |
|------|-----|
| 日期 | 2026-09-18 |
| commit | `e6acb2c`（e6acb2c771ab43cb6960087bd49bbe319268f27c） |
| 机器 | AMD Ryzen 9 9950X（16C32T）；WSL2（kernel 6.6.87.2-microsoft-standard-WSL2）分配 12 逻辑 CPU、70.7 GiB RAM |
| 工具链 | rustc / cargo 1.97.1 |
| criterion | 0.8.2，默认口径（100 样本 / 3s 预热 / 5s 测量） |
| 编译口径 | `[profile.bench]`（opt-level = 3、lto = true），default features |
| 环境 | 开发机近空载（load average ≈ 1，仅常驻服务，无并行编译）；WSL2 无 CPU 频率调节控制 |

中位数为 criterion `target/criterion/**/new/estimates.json` 的 median point estimate；「换算吞吐」列为按中位数换算的参考值（非 criterion 直接产出）：

| 基准用例 | 中位数 | 换算吞吐 |
|----------|-------:|---------:|
| `snowflake/generate` | 2.083 µs | ≈ 48.0 万次/s |
| `snowflake/batch_generate_100` | 209.7 µs | ≈ 47.7 万 ID/s（约 2.10 µs/ID） |
| `router/generate_snowflake_primary` | 2.151 µs | ≈ 46.5 万次/s |
| `router/generate_with_algorithm_uuid_v8` | 419.3 ns | ≈ 238 万次/s |
| `rate_limiter/check_rate_limit` | 198.6 ns | ≈ 503 万次/s |
| `auth_cache/get_hit` | 313.2 ns | ≈ 319 万次/s |

解读：

- Snowflake 单实例理论上限为每毫秒 1023 个 ID（默认 10 位 sequence，约 102 万 ID/s）。上表为单调用者串行数字，含每迭代 `block_on` 调度开销，未达理论上限属预期；批量 + 多调用者并发可逼近上限。
- 路由链路（2.151 µs）相对直连算法（2.083 µs）增量约 3%，路由查表与观测面不构成瓶颈。
- 限流判定与认证缓存命中均在百纳秒量级，每请求固定开销可控。

### 发号热路径（`benches/algorithms.rs`，2026-09-18，混合等待优化后）

环境与口径同首个发号基线（同日、同机、同 commit `e6acb2c` 工作区、criterion 默认口径）；差异为 `wait_for_next_ms` 引入 spin-then-sleep 混合等待（见[热路径设计要点](#热路径设计要点)）：

| 基准用例 | 中位数 | 换算吞吐 | 相对优化前 |
|----------|-------:|---------:|-----------:|
| `snowflake/generate` | 977.5 ns | ≈ 102.3 万次/s | **2.13×** |
| `snowflake/batch_generate_100` | 97.76 µs | ≈ 102.3 万 ID/s | **2.14×** |
| `snowflake/batch_generate_10000` | 9.778 ms | ≈ 102.3 万 ID/s | **2.12×**（优化前快照 20.72 ms） |
| `router/generate_snowflake_primary` | 986.8 ns | ≈ 101.3 万次/s | **2.18×** |
| `router/generate_with_algorithm_uuid_v8` | 382.1 ns | ≈ 261 万次/s | ≈ 持平 |
| `rate_limiter/check_rate_limit` | 183.2 ns | ≈ 546 万次/s | ≈ 持平 |
| `auth_cache/get_hit` | 277.7 ns | ≈ 360 万次/s | ≈ 持平 |

解读：优化前三组 Snowflake 用例均卡在「毫秒轮转 1ms sleep 粒度过冲」上（实测约为理论上限的 47%）；混合等待把轮转等待收紧到真实毫秒边界后，串行/批量/超大批量全部收敛到 1023 ID/ms 位宽上限（每 ID 977.5 ns = 1ms ÷ 1023，即基准此时测量的是位宽上限本身，而非等待开销）。UUID / 限流 / 认证缓存路径不含毫秒轮转，数字与优化前持平（差异在噪声范围内）。

### i18n 热路径（`benches/i18n.rs`）

环境与口径同上表（同日、同 commit、同机采集）：

| 基准用例 | 中位数 |
|----------|-------:|
| `translate_with_locale/en/no_args` | 41.0 ns |
| `translate_with_locale/zh-CN/no_args` | 41.0 ns |
| `translate_with_locale/fr_fallback/no_args` | 54.1 ns |
| `translate_with_locale_args/en/1_arg` | 89.1 ns |
| `translate_with_locale_args/zh-CN/1_arg` | 91.3 ns |
| `to_localized_string/InvalidIdFormat/en` | 88.4 ns |
| `to_localized_string/InvalidIdFormat/zh-CN` | 86.5 ns |
| `to_localized_string/RateLimitExceeded/en` | 42.5 ns |
| `to_localized_string/ClockMovedBackward/en` | 104.1 ns |
| `parse_accept_language/typical_2` | 66.0 ns |
| `parse_accept_language/longer_5` | 191.0 ns |
| `parse_accept_language/unsupported_only` | 74.3 ns |

### HTTP 端到端（单实例，2026-09-18）

环境与口径同上表（同日、同机、同 commit `e6acb2c` 工作区）；服务以 `--release` 构建运行，PostgreSQL 16（docker，本机）；复现配置见 `config/bench_a.toml`（无限流）与 `config/bench_b.toml`（认证 + 缓存 + 限流判定开启、桶不节流），压测客户端 oha 1.16、keep-alive、c=64。

| 场景 | 吞吐 | p50 | p99 |
|------|-----:|----:|----:|
| `GET /health` | 13,675 rps | 4.0 ms | 16.2 ms |
| `POST /api/v1/generate`（认证缓存命中 + 限流判定 + 租户校验 + 发号全链） | 4,974 rps | 10.8 ms | 38.9 ms |
| `POST /api/v1/generate/batch`（单批 100 ID） | 2,842 rps ≈ **28.4 万 ID/s** | 21.9 ms | 46.7 ms |

每请求固定开销的构成（微基准口径）：认证缓存命中 ~0.28 µs + 限流判定 ~0.18 µs + 路由分发 ~0.99 µs，合计 ≈ 1.5 µs——毫秒级端到端延迟的主体是 HTTP 栈与每请求租户 workspace 校验查库（~0.3–3 ms，docker PG 本机 RTT），而非发号路径本身。对照参照系：美团 Leaf 官方口径为 4C8G 单机 ~5 万 rps（TP999 1ms）。

#### 已解决的容量级阻塞（历史事故记录）

本节数字曾在一次事故后一度无法采集：inklog 主 async 通道为
`bounded(channel_capacity)`（默认 10000），file sink 未显式配置路径时该通道
没有消费者；服务累计发出 ~1 万条日志后通道积压满，此后**每条日志经
`send_timeout(100ms)` 阻塞再降级**。审计 `log_shared` 又把环满的 `warn!`
横跨互斥锁发出，于是全部请求的审计写入按 100ms/条串行化——实测吞吐塌到
~6 rps、每请求固定 ~11s（c=64），且服务端 CPU 空闲、连接池全部空闲。
"运行 ~1 万请求后性能断崖"与当日早些时候观察到的"每请求 201ms/402ms"异常
同源（generate 每请求两条审计事件即 2×~200ms）。

修复（两层，回归测试 `tests/audit_ring_stall_repro.rs` 钉住）：

1. `init_observability` 显式配置 inklog file sink（`logs/nebula.log`），async
   通道有消费者持续排水；
2. `log_shared` 的丢弃告警移到互斥锁外——日志后端再慢也不串行化请求。

已知残留（不阻塞，记录在案）：inklog「file_sink 未配置（None）等价启用」的
默认语义与"无路径即无 worker"的事实组合存在隐患，属于 `../base/inklog` 的
上游设计问题，建议上游将 None 语义改为显式禁用或为无路径情形使用即刻断连
的哑通道；当前由宿主侧显式配置规避。

---

## 优化建议

- **批量优先**：高吞吐调用方使用 `batch_generate` 一次取一批，而不是循环单发。
- **算法选型**：强有序选 Snowflake（纯内存、零 DB）；需要数据库审计语义或步长控制选 Segment；需要标准 UUID 互操作选 UUID v8。
- **认证缓存**：生产环境保持 `auth.cache_ttl_seconds > 0`，并理解吊销在本进程即时生效、跨节点最长滞后一个 TTL。
- **连接池**：Segment 为主的负载适当调高 `max_connections`，观察 `/metrics` 与数据库侧等待。
- **发布构建**：始终以 `cargo build --release` 部署（dev profile 未做优化）；基准数据以 `[profile.bench]` 口径为准。

---

## 相关文档

- [架构文档](ARCHITECTURE.md)：算法优化设计与模块依赖
- [配置迁移指南](CONFIG_MIGRATION_GUIDE.md)：配置全表与校验规则
- [测试场景矩阵](TEST_SCENARIOS.md)：基准与测试的分层关系
- [更新日志](CHANGELOG.md)：性能相关变更记录
