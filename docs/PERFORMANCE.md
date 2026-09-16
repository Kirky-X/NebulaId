# ⚡ Nebula ID 性能指南

> 本文档描述 Nebula ID 的基准口径、热路径设计要点与性能优化建议。所有内容以仓库内实际存在的基准与代码为准；文档不发布未经基准框架产出的吞吐数字。

[🏠 主页](../README.md) • [🏗️ 架构文档](ARCHITECTURE.md) • [🧪 测试场景矩阵](TEST_SCENARIOS.md)

---

## 📋 目录

- [基准口径与诚实声明](#基准口径与诚实声明)
- [基准套件（i18n 热路径）](#基准套件i18n-热路径)
- [如何运行](#如何运行)
- [热路径设计要点](#热路径设计要点)
- [构建与运行时性能配置](#构建与运行时性能配置)
- [基线记录约定](#基线记录约定)
- [优化建议](#优化建议)
- [相关文档](#相关文档)

---

## 基准口径与诚实声明

截至 v0.2.x，本仓库 `Cargo.toml` 中声明的 Criterion 基准**只有一项**：`[[bench]] name = "i18n"`（`benches/i18n.rs`，`harness = false`）。它覆盖 i18n 热路径，用于钉住 Phase 8 T041（LOW L4 perf fix）的基线，保证后续 i18n 改动可量化。

**本仓库没有 ID 生成吞吐基准框架，因此不发布任何 `*_next_id` 吞吐 / P99 延迟数字。** 任何来源的「Segment/Snowflake/UUID 每秒 XXX 万」类数字都不是本仓库基准产出；评估吞吐请以自身负载实测（见[优化建议](#优化建议)）。补齐 ID 生成基准框架已列入路线图（见主 README [🗺️ 路线图](../README.md#️-路线图)）。

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

## 如何运行

```bash
# 运行 i18n 基准（结果写入 target/criterion/，可用 criterion 归纳报告查看）
cargo bench --bench i18n
```

`[profile.bench]` 为 `opt-level = 3`、`lto = true`，与 release 接近，可视为发号热路径的参考编译口径。

---

## 热路径设计要点

以下要点来自代码与变更记录，是影响发号吞吐/延迟的实际设计：

| 设计 | 位置 | 说明 |
|------|------|------|
| Segment 双缓冲 + 动态步长 | `src/core/algorithm/segment.rs` | 号段预领取与 `[base_step, min_step, max_step]` 动态调节（`switch_threshold` 触发），把数据库往返摊薄到号段边界 |
| Snowflake 串行化迁移 | `src/core/algorithm/snowflake.rs` | `(last_timestamp, sequence)` 迁移串行化，修复并发下重复 ID 竞态（见 [CHANGELOG](CHANGELOG.md) Fixed） |
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
3. 在基线建立前，本文件**不填数字**——避免把一次性测量固化成文档承诺。

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
