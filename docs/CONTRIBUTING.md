<div align="center">

# 🤝 贡献指南

### 加入我们，共同打造卓越的分布式 ID 生成系统！

[🏠 主页](README.md) • [📖 文档](../USER_GUIDE.md) • [💬 讨论](../../discussions)

---

</div>

## 🎯 欢迎贡献者！

感谢您对 **Nebula ID** 的关注！我们非常期待您的加入。无论是修复 Bug、添加新功能、改进文档还是帮助他人，您的贡献都是无价的。

<div align="center">

### 🌟 贡献方式

<table>
<tr>
<td width="25%" align="center">
<img src="https://img.icons8.com/fluency/96/000000/code.png" width="64"><br>
<b>代码</b><br>
修复 Bug & 添加功能
</td>
<td width="25%" align="center">
<img src="https://img.icons8.com/fluency/96/000000/documentation.png" width="64"><br>
<b>文档</b><br>
完善文档 & 指南
</td>
<td width="25%" align="center">
<img src="https://img.icons8.com/fluency/96/000000/test-tube.png" width="64"><br>
<b>测试</b><br>
编写测试 & 发现 Bug
</td>
<td width="25%" align="center">
<img src="https://img.icons8.com/fluency/96/000000/chat.png" width="64"><br>
<b>社区</b><br>
提供帮助 & 支持
</td>
</tr>
</table>

</div>

---

## 📋 目录

- [行为准则](#行为准则)
- [快速开始](#快速开始)
- [开发工作流](#开发工作流)
- [编码标准](#编码标准)
- [测试指南](#测试指南)
- [文档规范](#文档规范)
- [提交变更](#提交变更)
- [评审流程](#评审流程)
- [社区互动](#社区互动)

---

## 行为准则

<div align="center">

### 🤗 友善与尊重

</div>

我们致力于提供一个包容和友好的环境。参与本项目即表示您同意：

<table>
<tr>
<td width="50%">

**✅ 推荐行为**
- 保持尊重和体谅
- 欢迎新手
- 接受建设性批评
- 专注于对社区最有利的事情
- 对他人表现出同理心

</td>
<td width="50%">

**❌ 禁止行为**
- 使用攻击性语言
- 骚扰或侮辱他人
- 公布私人信息
- 进行人身攻击
- 干扰讨论

</td>
</tr>
</table>

> 📜 **完整行为准则:** [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md)

---

## 快速开始

### 前提条件

在开始之前，请确保您已安装：

- ✅ **Git** - 版本控制工具
- ✅ **Rust 1.75+** - 编程语言
- ✅ **Cargo** - Rust 包管理器
- ✅ **IDE** - VS Code, IntelliJ 或类似工具

<details>
<summary><b>🔧 环境搭建步骤</b></summary>

**1. 安装 Rust:**
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

**2. 安装辅助工具:**
```bash
# 代码格式化工具
rustup component add rustfmt

# 静态分析工具
rustup component add clippy

# 代码覆盖率工具 (可选)
cargo install cargo-tarpaulin
```

**3. 验证安装:**
```bash
rustc --version
cargo --version
```

</details>

### Fork 与 Clone

<table>
<tr>
<td width="50%">

**1. Fork 仓库**

点击 GitHub 上的 "Fork" 按钮

</td>
<td width="50%">

**2. Clone 您的 Fork**

```bash
git clone https://github.com/YOUR_USERNAME/nebula-id
cd nebula-id
```

</td>
</tr>
<tr>
<td width="50%">

**3. 添加上游远程仓库**

```bash
git remote add upstream \
  https://github.com/nebula-id/nebula-id
```

</td>
<td width="50%">

**4. 验证远程仓库**

```bash
git remote -v
# origin    your-fork
# upstream  nebula-id/nebula-id
```

</td>
</tr>
</table>

### 构建与测试

```bash
# 构建项目
cargo build

# 运行测试
cargo test

# 运行示例
cargo run --example basic

# 构建文档
cargo doc --no-deps
```

---

## 开发工作流

<div align="center">

### 🔄 标准贡献流程

</div>

```mermaid
graph LR
    A[Fork 仓库] --> B[创建分支]
    B --> C[修改代码]
    C --> D[编写测试]
    D --> E[运行测试]
    E --> F{测试通过?}
    F -->|否| C
    F -->|是| G[提交代码]
    G --> H[推送到 Fork]
    H --> I[创建 PR]
    I --> J[代码评审]
    J --> K{通过评审?}
    K -->|需要修改| C
    K -->|是| L[合并!]
    
    style A fill:#e1f5ff
    style L fill:#4caf50
```

### 详细步骤

#### 1️⃣ 创建分支

分支应基于 `develop` 分支创建。

```bash
# 同步上游 develop 分支
git fetch upstream
git checkout develop
git merge upstream/develop

# 创建功能分支
git checkout -b feature/ALGORITHM-id-generation

# 或创建 Bug 修复分支
git checkout -b bugfix/DC-health-monitor

# 或创建文档更新分支
git checkout -b docs/update-readme
```

**分支命名规范:**
- `feature/*` - 新功能开发 (如 `feature/segment-algorithm`, `feature/uuid-v7`)
- `bugfix/*` - 缺陷修复
- `hotfix/*` - 紧急修复
- `release/*` - 版本发布
- `refactor/*` - 代码重构
- `docs/*` - 文档更新
- `perf/*` - 性能优化

#### 2️⃣ 修改代码

请遵循项目的编码标准和最佳实践。参考 [crates/core/src/algorithm/](crates/core/src/algorithm/) 中的实现模式。

#### 3️⃣ 运行静态检查与测试

在提交前，请确保代码通过所有本地检查。

```bash
# 运行预提交检查脚本（推荐）
./scripts/pre-commit-check.sh

# 或手动执行以下步骤：

# 格式化代码
cargo fmt

# 运行 Clippy 静态分析 (必须无警告)
cargo clippy -- -D warnings

# 运行所有测试
cargo test --all-features

# 运行特定 crate 的测试
cargo test -p nebula-id-core
cargo test -p nebula-id-server
```

**预提交检查脚本**会自动执行以下检查：
1. 代码格式化
2. Clippy 静态分析
3. 构建验证
4. 测试执行
5. 安全审计（如果配置了 deny.toml）
6. 文档生成
7. 代码覆盖率

#### 4️⃣ 提交代码

我们遵循 [Conventional Commits](https://www.conventionalcommits.org/) 规范。

**提交格式:**
`<type>(<scope>): <subject>`

**常见类型:**
- `feat`: 新功能 (如 `feat(algorithm): add UUID v7 implementation`)
- `fix`: 修复 Bug (如 `fix(segment): resolve buffer overflow issue`)
- `docs`: 文档变更
- `style`: 代码格式调整 (不影响功能)
- `refactor`: 代码重构
- `perf`: 性能优化
- `test`: 测试相关
- `chore`: 构建工具、依赖更新等
- `ci`: CI/CD 变更

**Nebula ID 特定的 Scope:**
- `algorithm` - ID 生成算法 (segment, snowflake, uuid)
- `coordinator` - 分布式协调模块
- `health` - 健康监控模块
- `server` - HTTP/gRPC 服务
- `config` - 配置管理
- `database` - 数据库集成

**示例:**
```bash
git commit -m "feat(algorithm): add batch generation support for Segment"
git commit -m "fix(coordinator): resolve DC failover race condition"
git commit -m "docs: update USER_GUIDE with new examples"
```

#### 5️⃣ 推送到 Fork 仓库

```bash
git push origin feature/ALGORITHM-id-generation
```

#### 6️⃣ 创建 Pull Request (PR)

1. 前往 GitHub 上的 Fork 仓库
2. 点击 "Compare & pull request"
3. 按照 [PR 模板](#提交变更) 填写信息
4. 关联相关的 Issue
5. 提交 PR！

---

## 编码标准

### Rust 最佳实践

#### 所有权与借用
- 优先使用借用而非所有权转移
- 使用 `&` 进行不可变借用，`&mut` 进行可变借用
- 避免不必要的 `.clone()`；优先使用引用
- 保持生命周期标注尽可能简单

#### 类型系统
- 利用类型系统表达业务约束 (NewType 模式)
- 优先使用 `Option<T>` 而非空值
- 使用 `Result<T, E>` 进行错误处理；库代码中严禁使用 `panic!`
- 为自定义错误实现 `std::error::Error` trait

#### 并发与异步
- 使用 `Arc<T>` 共享不可变数据，使用 `Arc<Mutex<T>>` 或 `Arc<RwLock<T>>` 共享可变数据
- 优先使用通道 (Channel) 进行线程间通信
- 异步代码使用 `async/await`，避免阻塞运行时

#### 性能优化
- 使用 `Vec::with_capacity()` 预分配内存
- 优先使用迭代器链而非显式循环
- 函数参数优先使用 `&str` 而非 `String`
- 避免不必要的堆分配

### 命名规范
- 模块、函数、变量: `snake_case`
- 类型、Trait: `PascalCase`
- 常量、静态变量: `SCREAMING_SNAKE_CASE`

### 代码质量要求
- **零警告状态**: 严禁忽略编译器警告。所有警告必须在提交前修复。
- **Clippy**: 必须通过 `cargo clippy` 检查，不引入新的 lint 警告。
- **文档**: 所有公共 API 必须有文档注释。

---

## 测试指南

### 测试原则
- **TDD**: 鼓励遵循 "红-绿-重构" 循环。
- **严禁禁用测试**: 测试失败时应修复代码或测试，严禁直接禁用。
- **真实性**: 除非涉及外部硬件或无法控制的服务，否则严禁使用 Mock/Stub，优先使用真实实现。

### 测试金字塔
1. **单元测试**: 快速、独立，验证核心逻辑。位于 `src/` 目录下，使用 `#[cfg(test)]` 标注。
2. **集成测试**: 验证模块间交互。位于 `tests/` 目录。
3. **E2E 测试**: 验证关键业务流程。

### 覆盖率要求
- **新代码覆盖率**: ≥ 80%。
- **核心业务逻辑**: 100% 覆盖。

```bash
# 生成覆盖率报告
cargo tarpaulin --out Html

# 查看特定 crate 的覆盖率
cargo tarpaulin --package nebula-id-core --out Html
```

---

## 文档规范

- **公共 API**: 所有 `pub` 项目必须包含文档注释 `///`。
- **示例代码**: 文档注释中应包含可运行的示例代码。
- **更新同步**: 代码变更时必须同步更新相关的 README 和 API 文档。
- **算法文档**: 新算法实现需要在 `docs/ALGORITHM.md` 中添加设计文档。

---

## 提交变更

### PR 提交标准
- **原子性**: 每个 commit/PR 应只包含一个逻辑变更。
- **规模限制**: PR 变更行数建议控制在 400 行以内。
- **关联 Issue**: 必须在 PR 描述中关联相关 Issue。

### PR 模板

```markdown
## 变更类型
- [ ] 新功能 (feat)
- [ ] 缺陷修复 (fix)
- [ ] 重构 (refactor)
- [ ] 文档更新 (docs)
- [ ] 性能优化 (perf)
- [ ] 测试更新 (test)
- [ ] 其他

## 描述
简要说明本次变更的目的和内容。

## 测试情况
- [ ] 单元测试通过
- [ ] 集成测试通过
- [ ] 手动测试完成
- [ ] 性能测试通过 (如适用)

## 检查清单
- [ ] 代码遵循项目编码规范
- [ ] 已添加必要的测试
- [ ] 文档已更新
- [ ] 未引入新的警告 (Zero Warning)
- [ ] 变更已通过 cargo clippy
- [ ] 变更已通过 cargo fmt
- [ ] 变更已通过 ./scripts/pre-commit-check.sh (包括格式化、静态分析、构建、测试、安全审计、文档、覆盖率)

## 相关 Issue
Closes #123

## 附加信息
(如有必要，提供截图、日志或其他补充信息)
```

---

## 评审流程

### 评审要点
- **功能性**: 满足需求，逻辑正确，ID 生成算法符合规范。
- **代码质量**: 遵循 SOLID 原则，可读性好，无重复代码。
- **安全性**: 无硬编码敏感信息，有输入验证，考虑并发安全。
- **性能**: 无明显的性能隐患，注意原子操作和锁的使用。
- **测试**: 测试覆盖率和质量符合要求。

### 响应评审反馈
- 及时回复评审者的评论。
- 根据反馈修改代码并推送，PR 会自动更新。

---

## 社区互动

- **GitHub Discussions**: 提问、分享想法、讨论设计决策。
- **Issue Tracker**: 报告 Bug、提交功能请求。
- **算法讨论**: 有关新 ID 算法的设计讨论，请在 PR 中详细说明。

感谢您为 **Nebula ID** 贡献力量！🚀

---

<div align="center">

**[📖 文档](../USER_GUIDE.md)** • **[🐛 报告问题](../../issues)** • **[💬 讨论](../../discussions)**

由 Nebula ID Team 用 ❤️ 制作

</div>
