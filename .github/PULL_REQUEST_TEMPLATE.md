## 变更类型

<!-- 勾选适用项，未勾选 PR 描述视为不完整 -->

- [ ] **feat**: 新功能（feature）
- [ ] **fix**: 缺陷修复（bug fix）
- [ ] **refactor**: 重构（无行为变更）
- [ ] **perf**: 性能优化
- [ ] **docs**: 文档变更
- [ ] **test**: 测试补充/修复
- [ ] **chore**: 构建/CI/工具链变更
- [ ] **breaking**: 破坏性变更（需同步更新版本号）

## 关联 Issue

<!-- 关联 issue 编号，例如 closes #123, refs #456 -->

-

## 变更说明

<!-- 描述本 PR 做了什么，为什么这么做。包含设计决策和权衡 -->

## 自检清单

- [ ] 代码已通过 `cargo fmt --package nebulaid -- --check`
- [ ] 代码已通过 `cargo clippy --package nebulaid --all-features -- -D warnings`
- [ ] 已运行 `cargo test --package nebulaid --all-features`
- [ ] 已运行 `cargo llvm-cov --package nebulaid --all-features --fail-under-lines 80`（覆盖率 ≥80%）
- [ ] 已通过 `cargo deny check`（许可证 + 安全）
- [ ] 已通过 `cargo audit --deny warnings`
- [ ] 新增公共 API 已更新 `docs/` 或 CHANGELOG
- [ ] 新增配置项已更新配置文档与示例
- [ ] 无 hardcode 密钥/凭证/连接字符串
- [ ] 数据库变更已配套迁移脚本（如适用）

## 测试策略

<!-- 描述如何验证本变更 -->
<!-- 如果是性能变更，附上 benchmark 结果对比 -->

```
# 验证命令
cargo test --package nebulaid --all-features
```

## 破坏性变更说明

<!-- 如果勾选了 breaking，请描述迁移路径 -->
<!-- 如果是 API 变更，列出 before/after 接口签名 -->

无破坏性变更。

## 截图 / 日志（如适用）

<!-- 如果是 UI 或行为变更，附上 before/after 截图或日志 -->
