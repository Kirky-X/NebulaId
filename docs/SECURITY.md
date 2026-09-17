# 🔒 Nebula ID 安全文档

> 本文档描述 Nebula ID 的安全设计、供应链门禁、漏洞报告流程与安全最佳实践。内容基于仓库代码、CI 配置与 Git 钩子的实际实现。

[🏠 主页](../README.md) • [🏗️ 架构文档](ARCHITECTURE.md) • [🔧 配置迁移指南](CONFIG_MIGRATION_GUIDE.md)

---

## 📋 目录

- [支持版本](#支持版本)
- [漏洞报告流程](#漏洞报告流程)
- [安全设计概览](#安全设计概览)
- [供应链与门禁](#供应链与门禁)
- [安全最佳实践](#安全最佳实践)
- [相关文档](#相关文档)

---

## 支持版本

我们建议所有用户始终使用最新发布版本，以获得完整的安全修复。各版本的安全修复情况查阅 [CHANGELOG](CHANGELOG.md)：

| 版本 | 支持状态 |
|------|----------|
| latest（0.2.x 及 Unreleased） | ✅ 接收安全修复 |
| 更早版本 | ❌ 不再接收修复 |

依赖安全由 `cargo-audit`（CI 与 release 前置均以 `--deny warnings` 执行）与 `cargo-deny`（许可证 + 安全 Advisories 检查，配置见 `deny.toml`）双重监控。

---

## 漏洞报告流程

**请勿通过公开 issue 报告安全漏洞。**

请使用 GitHub [Security Advisories](https://github.com/Kirky-X/NebulaId/security/advisories/new) 私密披露通道提交报告，报告中尽量包含：受影响版本、复现步骤或概念验证、影响评估。项目维护者会在确认收到后跟进评估与修复，修复经 [CHANGELOG](CHANGELOG.md) 的 `Security` 小节披露。

对报告者的期望与处理优先级：可被远程未认证利用的问题（认证绕过、注入、DoS）优先处理；需要已认证账户或本地访问的问题次之。

---

## 安全设计概览

### 认证（garrison-auth）

- API key 验证由 [garrison](https://crates.io/crates/garrison) 接管（`web-axum` 中间件 + `protocol-apikey` ApiKeyHandler + `cache-memory` KV 后端），不再使用手写方案。
- 密钥以 **Argon2id** 哈希落库（`argon2 0.6`，修复 CWE-916；salt 由 `NEBULA_API_KEY_SALT` 或 `[auth].api_key_salt` 提供），凭证比较使用常量时间实现（`subtle`）。
- **轮换宽限期**：`key_rotation_grace_period_seconds > 0` 时，轮换会把上一代哈希与窗口截止时刻落库（`prev_secret_hash` / `rotate_expires_at` 两列，启动期迁移自动补齐，见[配置迁移指南](CONFIG_MIGRATION_GUIDE.md#密钥轮换宽限期与配置-fail-fast未发布版本)），旧密钥在宽限窗口内仍可验证、到期惰性失效；超过 30 天会被钳制并告警。默认 `0`（关闭）。
- **认证缓存**：进程内 `MemoryGarrisonDao` 实现缓存命中优先、未命中回源 DB；缓存值仅含 workspace_id / role / 过期时间，**不含 secret**。吊销/轮换/禁用对本进程即时失效；多节点部署其他节点最长滞后一个 TTL。缓存失效为精确前缀匹配，glob 形态（含 `*` / `?`）的 key_id 不能批量清除他人条目。
- **gRPC 认证**：全部 RPC（含双向流）在各入口经单点 `authenticate()` 校验 `authorization`（Basic/ApiKey）。key 被禁用或过期返回 `PermissionDenied`，凭证无效或不存在返回 `Unauthenticated`；`auth.enabled = false` 时放行。
- **单 admin key 守卫**：创建/吊销 admin key 的守卫用 SQL 侧 `role='admin' AND enabled=true` 计数判定（行 id 精确查询，无分页上界），阻止「admin 再开 admin」持久化后门。注意作用范围：守卫只在 HTTP `POST /api-keys` 生效，启动期环境/配置引导的 admin key 走仓储直插。

### 流控（限流）

- 令牌桶限流（`limiteron` 原语）真实挂载 HTTP 栈，位于 CORS / 安全头**内侧**，429 响应携带规范小写的 `x-ratelimit-limit` / `x-ratelimit-remaining`（429 额外 `x-ratelimit-remaining: 0`）。
- `[rate_limit].enabled = false` 时不挂限流层；`POST /config/rate-limit` 热更新复用启动期 `Config::validate`（含 `burst ≤ 10 × default_rps` 组合校验）。
- 请求归属真实到对端连接地址（`PeerAddr` 承载 `ConnectInfo`），限流键、认证失败计数与审计 `client_ip` 不再落入共享匿名桶；`X-Forwarded-For` 仅在可信代理配置下采信（IP 欺骗防护）。

### 传输与响应安全

- rustls TLS，`tls.min_tls_version` 真实强制：`tls13` 时低于 TLS 1.3 的 ClientHello 被直接拒绝（HTTP 侧经 `ServerConfig::builder_with_protocol_versions`）；gRPC 侧的 ServerConfig 由 tonic 装配，无法注入版本集合，启动时按 `warn` 如实标界。
- `tls.enabled = true` 且证书缺失/解析失败时**拒绝启动**（fail-fast，不静默降级明文）；`enabled=false` 与 per-port 开关矛盾时显式 warn。
- TLS 握手每连接独立任务 + 10 秒握手超时，未认证客户端无法通过「建连不发 ClientHello」冻结监听端口；accept 错误带退避。
- 安全响应头：X-Content-Type-Options、X-Frame-Options、CSP、HSTS、X-XSS-Protection、Referrer-Policy；CORS 严格白名单。

### 审计与租户隔离

- ID 生成与密钥操作全量审计（`src/core/algorithm/audit_trait.rs`、`src/server/audit/`），`client_ip` 归属到真实对端地址；审计 reason 与 i18n 文案单源。
- biz-tags 查询按角色过滤 workspace（User 仅见本 workspace，Admin 可按参数过滤）——修复 IDOR（strix-0001，见 [CHANGELOG](CHANGELOG.md)）。
- `Locale` 派生自 `Accept-Language` 用户输入、可被伪造，仅用于内容协商，**不得**参与任何认证/授权决策。
- SDK 错误对外只回固定概要 + `error_id`，内部细节仅进日志（不回显内部错误）。

### 配置面安全

- 17 个配置结构体全部 `#[serde(deny_unknown_fields)]`；坏配置（读失败、解析失败、校验失败）一律退出码 1 终止启动，不再静默降级默认值。
- 敏感信息经 `${VAR}` 环境变量展开注入（`NEBULA_DATABASE_PASSWORD`、`NEBULA_API_KEY_SALT`）；`algorithm_type` 数据库 ENUM 与代码对齐的迁移操作见[配置迁移指南](CONFIG_MIGRATION_GUIDE.md#algorithm_type-enum-迁移uuid_v7--uuid_v4---uuid_v8)。

---

## 供应链与门禁

门禁在 CI（`.github/workflows/`）与本地 Git 钩子（`lefthook.yml` + `scripts/run.sh`）双重执行：

| 门禁 | CI 位置 | 本地等价 | 内容 |
|------|---------|----------|------|
| 格式 + Lint | `ci.yml` · fmt-clippy | lefthook pre-commit / `run.sh lint` | `cargo fmt --check`；`cargo clippy --all-features -- -D warnings`（0 警告） |
| 依赖许可证与 Advisories | `ci.yml` · deny | — | `cargo-deny check`（`deny.toml`） |
| 依赖漏洞审计 | `ci.yml` · audit；`release.yml` 发布前置 | — | `cargo audit --deny warnings` |
| 静态安全分析 | `codeql.yml` | — | CodeQL Rust 分析（README 的 Security 徽章） |
| 测试 + 覆盖率 | `ci.yml` · test（default / all 矩阵；覆盖率门禁仅在 default leg） | lefthook pre-push | `cargo llvm-cov --fail-under-lines 95`（CI default leg）/ `≥ 90%` 全特性口径（pre-push 本地门禁，`scripts/_coverage_gate.sh`），排除 `server/proto/` 生成代码 |
| 私钥扫描 | — | lefthook pre-commit | gitleaks（`scripts/_gitleaks_scan.sh`） |
| 聚合门禁 | `ci.yml` · gate | — | 任一前置 job 失败即阻断合并 |

补充事实：

- `--all-features` 可构建（sqlite feature 已删除，dbnexus 的 embedded/server 互斥不再触发），CI 与文档统一采用该口径；`--no-default-features` 另有轻量编译检查（garrison 回退路径等）。
- `docker/vendor-deps.sh` 向 `.docker-vendor/` 打包依赖副本时排除 `.env*` / `local_settings*`，不把凭据带进镜像上下文。
- `Cargo.lock` 提交入库，保证 CI 与发布构建的依赖可复现。

---

## 安全最佳实践

**部署方：**

1. **密钥**：`NEBULA_DATABASE_PASSWORD` 与 `NEBULA_API_KEY_SALT` 必经环境变量注入，禁止写入配置文件明文；盐值使用高强度随机值且部署后不外泄。
2. **TLS**：生产环境 `tls.enabled = true` 并尽量 `min_tls_version = "tls13"`；证书私钥文件权限最小化。
3. **认证**：保持 `auth.enabled = true` 与 `cache_ttl_seconds > 0`；理解吊销跨节点收敛时间为一个 TTL，紧急场景可滚动重启实例。
4. **admin key**：仅经 HTTP `POST /api-keys` 创建（受单 admin 守卫保护），避免依赖启动期引导路径绕过守卫；定期轮换并启用宽限期平滑过渡。
5. **限流**：保持 `rate_limit.enabled = true`，按业务容量设置 `default_rps` / `burst_size`，防止单租户打满共享配额。
6. **网络**：PostgreSQL / Redis / etcd 不暴露公网；`/metrics` 端口仅对监控网段开放。
7. **升级**：升级前运行[配置迁移指南](CONFIG_MIGRATION_GUIDE.md)的预检命令，确认无未知键与缺失必填键；关注 CHANGELOG 中的 Breaking 与 Security 小节。

**贡献者：**

1. 新增配置字段必须同步 `Config::validate()` 与文档（[配置迁移指南 · 配置全表](CONFIG_MIGRATION_GUIDE.md#配置全表全量选项与校验规则)）。
2. 涉及凭证的代码使用常量时间比较，禁止日志输出 secret；新增敏感键必须走 `${VAR}` 展开路径。
3. 新增依赖须通过 `cargo-deny` / `cargo-audit` 门禁，优先小而专注、活跃维护的 crate。
4. 提交前运行 `./scripts/run.sh pre-commit`（gitleaks 私钥扫描包含在内）。

---

## 相关文档

- [更新日志](CHANGELOG.md)：安全修复与行为变更记录
- [配置迁移指南](CONFIG_MIGRATION_GUIDE.md)：密钥轮换宽限期与配置 fail-fast
- [部署指南](DEPLOYMENT.md)：生产部署与监控
- [API 参考](API_REFERENCE.md)：认证头与错误码语义
