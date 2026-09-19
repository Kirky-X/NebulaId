# 🧪 Nebula ID 测试场景矩阵

> 适用版本：Nebula ID **0.2.x 工作区**（Rust edition 2021，单包 `nebulaid`）。
> 编写依据（只读核对）：`Cargo.toml [features]` 与 `[[bench]]`、`src/core/tests/mod.rs` 模块注册表、`tests/` 目录、`.github/workflows/ci.yml`、`lefthook.yml`、`scripts/_coverage_gate.sh`、`CHANGELOG.md`。
> 所有模块级测试计数均经 `grep -c '#\[test\]|#\[tokio::test'` 核实。

[🏠 主页](../README.md) • [架构文档](ARCHITECTURE.md) • [性能指南](PERFORMANCE.md) • **深度方案**：[测试总体方案](TEST_PLAN.md)（模块交互分析 · 场景穷举 · 特性组合 · 执行计划）

---

## 📋 目录

- [阅读约定](#阅读约定)
- [测试分层总览](#测试分层总览)
- [场景矩阵](#场景矩阵)
- [Shell 端到端脚本](#shell-端到端脚本)
- [运行命令与门禁](#运行命令与门禁)
- [统计汇总](#统计汇总)
- [相关文档](#相关文档)

---

## 阅读约定

- **编号**：`<域>-<序号>`（如 `ALG-01`），域前缀见场景矩阵各表。
- **覆盖位置**：指向实际存在的测试文件 / 模块；不虚构测试函数名，函数级细节以对应文件为准。
- **特性口径**：`--all-features` 可构建（sqlite feature 已删除，dbnexus 的 embedded/server 互斥不再触发），矩阵中的运行命令均采用该口径。

---

## 测试分层总览

| 层 | 位置 | 规模（截至 0.2.x 工作区） | 运行方式 |
|----|------|---------------------------|----------|
| 内联单元测试 | `src/` 各模块 `#[cfg(test)]` | 约 1560 个测试函数 | `cargo test --package nebulaid` |
| E2E 测试模块 | `src/core/tests/`（13 个文件，经 `mod.rs` 注册） | 221 个测试函数 | `cargo test --package nebulaid --features etcd` |
| i18n 端到端 | `tests/i18n_e2e.rs` | 1 个文件（middleware → Extension → 翻译响应全链） | 同上 |
| 审计管道阻塞回归 | `tests/audit_ring_stall_repro.rs` | 1 个测试（灌满 inklog async 通道后断言审计 log() 不阻塞） | 同上 |
| Shell 端到端 | `tests/*.sh` | 4 个脚本（见下节） | `./scripts/run.sh api-test` 等 |
| Criterion 基准（i18n 热路径） | `benches/i18n.rs` | 1 组（4 基准函数 / 12 用例） | `cargo bench --bench i18n` |
| Criterion 基准（发号热路径） | `benches/algorithms.rs` | 1 组（7 基准函数 / 7 用例） | `cargo bench --bench algorithms` |
| 仓库文本守卫 | `src/core/tests/repo_docs_guards_tests.rs` | 1 个测试 | 随 `cargo test` 运行 |

`src/core/tests/` 内的 E2E 模块按层组织，计数如下（`grep -c` 口径）：

| 模块 | 测试函数 | 覆盖域 |
|------|:--------:|--------|
| `server_layer_e2e_tests.rs` | 55 | HTTP 服务层端到端 |
| `auth_handlers_e2e_tests.rs` | 35 | 认证 handlers |
| `grpc_monitoring_e2e_tests.rs` | 34 | gRPC 与监控 |
| `supporting_layer_e2e_tests.rs` | 28 | 支撑层（配置 / 日志 / 缓存装配） |
| `infra_e2e_tests.rs` | 22 | 基础设施（数据库连接 / 迁移） |
| `degradation_tests.rs` | 12 | 降级链 |
| `remaining_e2e_tests.rs` | 12 | 收尾场景 |
| `algorithm_e2e_tests.rs` | 11 | 算法端到端 |
| `cache_tests.rs` | 5 | 缓存 |
| `integration_tests.rs` | 4 | 集成冒烟 |
| `segment_monitoring_e2e_tests.rs` | 2 | Segment 监控 |

---

## 场景矩阵

### 算法（ALG）

| 编号 | 场景 | 验证点 | 覆盖位置 |
|------|------|--------|----------|
| ALG-01 | Segment 正常发号 | 同 workspace/group/biz_tag 单调递增 | `src/core/tests/algorithm_e2e_tests.rs` |
| ALG-02 | Segment 无仓储守卫 | 未注入仓储返回 `CoreError::ConfigurationError` | 同上（配合 `sdk` 面测试） |
| ALG-03 | Segment 双缓冲切换 | 步长动态调节在 `min_step ≤ base_step ≤ max_step` 内生效 | `src/core/algorithm/segment.rs` 内联测试 |
| ALG-04 | Snowflake 并发唯一性 | `(last_timestamp, sequence)` 串行化迁移后并发无重复 | `src/core/algorithm/snowflake.rs` 内联测试 |
| ALG-05 | Snowflake 时钟回拨 | 超过 `clock_drift_threshold_ms` 触发 `ClockMovedBackward` | 同上 |
| ALG-06 | UUID v8 毫秒翻转 | `test_uuid_v8_counter_resets_on_millisecond_rollover`（非 flaky：按样本内嵌时间戳判定同毫秒） | `src/core/algorithm/uuid_v8.rs` 内联测试 |
| ALG-07 | 算法路由 | `generate_with_algorithm` 按次覆盖 `algorithm.default` | `src/core/algorithm/router.rs` 内联测试 |
| ALG-08 | 批量生成边界 | `batch_generate(0)` 边界与 `max_batch_size` 上限（1..=10000） | 算法模块内联 + `validate` 测试 |

### 认证与密钥（AUTH）

| 编号 | 场景 | 验证点 | 覆盖位置 |
|------|------|--------|----------|
| AUTH-01 | API key 全生命周期 | 创建 / 校验 / 吊销 / 启停 | `src/core/tests/auth_handlers_e2e_tests.rs` |
| AUTH-02 | Argon2id 哈希与常量时间比较 | 凭证不以明文落库；比较走 `subtle` | auth 模块内联测试 |
| AUTH-03 | 轮换宽限期 | `>0` 时旧密钥在窗口内可用、到期惰性失效；`>30 天`钳制并告警 | 同上（CHANGELOG「密钥轮换宽限期」） |
| AUTH-04 | 认证缓存 | 命中优先、未命中回源；缓存值不含 secret；`cache_ttl_seconds=0` 不装配 | `src/server/auth/memory_dao.rs` 内联测试 |
| AUTH-05 | 缓存失效精确匹配 | 含 `*`/`?` 的 key_id 不能批量清除他人条目 | 同上 |
| AUTH-06 | gRPC 认证失败码 | 禁用/过期 → `PermissionDenied`；无效/不存在 → `Unauthenticated` | `src/core/tests/grpc_monitoring_e2e_tests.rs` |
| AUTH-07 | HTTP 认证头单源解析 | Basic/ApiKey 解析唯一实现（`parse_authorization_header_detailed`） | server 层内联测试 |
| AUTH-08 | 单 admin key 守卫 | `POST /api-keys` 上第二个 admin key 被拒（SQL 侧 `role='admin' AND enabled=true` 计数） | `auth_handlers_e2e_tests.rs` |
| AUTH-09 | biz-tags 租户隔离 | User 仅见本 workspace；Admin 可按参数过滤（IDOR 回归） | 同上 |

### 限流与请求归属（RATE）

| 编号 | 场景 | 验证点 | 覆盖位置 |
|------|------|--------|----------|
| RATE-01 | 限流真实挂载 | 429 真实产生于 HTTP 栈（非死代码 Extension） | `src/server/` 限流中间件内联测试 |
| RATE-02 | 响应头规范 | `x-ratelimit-limit` / `x-ratelimit-remaining` 规范小写；429 带 `remaining: 0` | 同上 |
| RATE-03 | 层序 | 限流在 CORS / 安全头内侧，预检不消耗配额 | 同上 |
| RATE-04 | 热更新校验 | `POST /config/rate-limit` 复用 `Config::validate`（`burst > 10×rps` 被拒） | config 管理测试 |
| RATE-05 | 客户端 IP 归属 | `PeerAddr`/`ConnectInfo` 注入后限流键 / 审计 IP 按对端区分 | server 层内联测试 |

### 传输安全（TLS）

| 编号 | 场景 | 验证点 | 覆盖位置 |
|------|------|--------|----------|
| TLS-01 | 证书缺失 fail-fast | `tls.enabled=true` 且证书缺失/坏格式拒绝启动 | `src/server/config/tls.rs` 内联测试 |
| TLS-02 | min_tls_version 强制 | `tls13` 拒绝低于 1.3 的握手（HTTP 侧） | 同上 |
| TLS-03 | 矛盾配置显式 warn | `enabled=false` + per-port true 按 `tls_config_conflict` 告警 | 同上 |
| TLS-04 | 握手不可冻结监听 | 每连接独立任务 + 10s 握手超时（未认证 DoS 回归） | server 监听测试 |

### 配置（CFG）

| 编号 | 场景 | 验证点 | 覆盖位置 |
|------|------|--------|----------|
| CFG-01 | 未知键拒绝 | 17 个结构体 `deny_unknown_fields`，拼错段名启动失败 | `src/core/config/` 内联测试 |
| CFG-02 | 坏配置 fail-fast | 解析失败退出码 1，不再回退 `Config::default()`；仅「未给 `--config` 且默认路径不存在」回落默认 + warn | `app_config.rs`（`resolve_startup_config`） |
| CFG-03 | 校验规则全集 | `Config::validate` 的 10 条规则逐条触发（见[配置迁移指南 · 校验规则](CONFIG_MIGRATION_GUIDE.md#校验规则)） | `app_config.rs` 内联测试 |
| CFG-04 | 非 ASCII 配置不 panic | 含中文注释/值的配置文件解析不 panic（字节切片回归） | `load_from_file_with_non_ascii_comments_does_not_panic` |
| CFG-05 | 宽限期两列自动迁移 | 启动期 `ADD COLUMN IF NOT EXISTS` 幂等补齐；失败终止启动 | `src/core/database/connection.rs` 内联测试 |
| CFG-06 | 环境变量双机制 | `Config::load_from_env` 覆盖 + `${VAR}` 展开 | `app_config.rs` 内联测试 |

### i18n 与监控（OBS）

| 编号 | 场景 | 验证点 | 覆盖位置 |
|------|------|--------|----------|
| OBS-01 | Accept-Language 全链 | header → `locale_middleware` → `Extension<Locale>` → 翻译响应 | `tests/i18n_e2e.rs` |
| OBS-02 | locale 键集对齐 | `en.yml` 与 `zh-CN.yml` 顶层键完全一致（守卫测试） | i18n 模块内联测试 |
| OBS-03 | 逐算法分位数 | p50/p99/p999 取自 1024 样本环形缓冲（真实分位数口径） | `src/core/types/metrics.rs` 内联测试 |
| OBS-04 | 时钟回拨可观测 | `clock_backwards` 真实计数驱动告警（非「有延迟样本即真」） | `grpc_monitoring_e2e_tests.rs` |
| OBS-05 | 健康与指标端点 | `/health`、`/health/sdforge`、`/metrics` 语义 | `server_layer_e2e_tests.rs` |

### 分布式与降级（DST）

| 编号 | 场景 | 验证点 | 覆盖位置 |
|------|------|--------|----------|
| DST-01 | 降级链兜底 | `[Snowflake, UuidV8]` 去重后按序接管 | `src/core/tests/degradation_tests.rs` |
| DST-02 | etcd 协调（feature `etcd`） | 工作器分配 / leader 协调；`endpoints=[]` 退回 `LocalDistributedLock` | coordinator 内联测试 + `infra_e2e_tests.rs` |
| DST-03 | 停机不泄漏后台任务 | `rate_limit_cleanup` 与降级巡检在全部退出分支被回收 | server 层内联测试 |

### SDK 与示例（SDK）

| 编号 | 场景 | 验证点 | 覆盖位置 |
|------|------|--------|----------|
| SDK-01 | Kit 装配校验 | trait-kit AsyncKit 依赖图缺失依赖 / 环在 `build()` 期报错 | `src/sdk/kit.rs` 内联测试（feature `sdk`） |
| SDK-02 | 纯算法零 DB | `Config::default()` + snowflake/uuid_v8 可发号 | `src/sdk` 内联测试 |
| SDK-03 | 错误不外泄 | `to_api_error` 对外固定概要 + `error_id` | `src/sdk` 内联测试 |
| SDK-04 | 示例可构建 | `embedded` / `sdk_server` 以 `required-features` 门控编译 | CI test job（`--all-features` leg） |

---

## Shell 端到端脚本

| 脚本 | 行数 | 场景 | 前置条件 |
|------|-----:|------|----------|
| `tests/api_test.sh` | 1037 | API 端点回归（认证、发号、管理面） | 服务已启动；`./scripts/run.sh api-test [server_url]` |
| `tests/degradation_test.sh` | 313 | 依赖故障下的降级行为 | 按脚本头部说明准备依赖 |
| `tests/distributed_test.sh` | 459 | etcd 分布式协调场景 | etcd 可用 |
| `tests/db_concurrency_test.sh` | 289 | 数据库并发正确性 | PostgreSQL 可用 |
| `tests/lib.sh` | 544 | 上述脚本的共用断言/工具库 | — |

---

## 运行命令与门禁

```bash
# 全量测试（CI 矩阵按 default / postgresql / etcd 三档运行）
cargo test --package nebulaid --features etcd

# SDK 特性面（独立 CI job：clippy + test）
cargo clippy --package nebulaid --features sdk -- -D warnings
cargo test --package nebulaid --features sdk

# 覆盖率门禁：CI ≥ 95%（排除 server/proto/ 生成代码）
cargo llvm-cov --package nebulaid --features etcd \
  --fail-under-lines 95 --ignore-filename-regex "server/proto/"

# 本地门禁：pre-commit（fmt + clippy + gitleaks）与 pre-push（test + 覆盖率 ≥ 80%）
./scripts/run.sh pre-commit

# 基准
cargo bench --bench i18n
```

门禁归属：CI 五阶段（fmt-clippy → deny → audit → test 矩阵 → gate）见 `.github/workflows/ci.yml`；pre-commit / pre-push 钩子见 `lefthook.yml`；pre-push 覆盖率阈值为 ≥ 80%（`scripts/_coverage_gate.sh`），CI 为 ≥ 95%。

---

## 统计汇总

截至 0.2.x 工作区（口径：`grep -rc '#\[test\]\|#\[tokio::test'` 于 `src/` 与 `tests/`）：

| 类别 | 数量 |
|------|-----:|
| Rust 测试函数合计 | 约 1780 |
| ├─ `src/` 内联单元测试 | 约 1560 |
| ├─ `src/core/tests/` E2E 模块 | 221 |
| └─ `tests/i18n_e2e.rs` | 少量（全链 i18n） |
| Shell 端到端脚本 | 4（另有 1 个共用库） |
| Criterion 基准组 | 2（i18n：4 函数 / 12 用例；发号热路径：7 函数 / 7 用例） |
| 仓库文本守卫 | 1 |

覆盖率：CI 门禁为行覆盖率 ≥ 95%（`ci.yml`），pre-push 本地门禁 ≥ 80%（`_coverage_gate.sh`）；v0.2.0 发布时实际行覆盖率 89.91%（门禁值是下限，非当前值）。历史口径：v0.2.0 发布时 e2e 套件曾报 1829 条（见 [CHANGELOG](CHANGELOG.md)），后续演进以上表 grep 口径为准。

---

## 相关文档

- [性能指南](PERFORMANCE.md)：基准口径与热路径
- [安全文档](SECURITY.md)：供应链与门禁清单
- [贡献指南](CONTRIBUTING.md)：测试规范与提交要求
- [更新日志](CHANGELOG.md)：测试相关变更记录
