#!/bin/bash
# lefthook pre-push 覆盖率门禁（≥90%）
# 注：不写成 lefthook.yml 的多行 `run: |` 内联脚本——Windows 下 lefthook
# 会破坏多行脚本的换行语义（fi 与前一命令被空格拼接导致 if 永不闭合）。
#
# 口径说明：与 CI 的差异是有意为之——
# - 权威门禁在 ci.yml default leg（≥95%，真库环境下 7 个基线失败测试通过，
#   且不含 sdk 稀释）；本脚本只是 pre-push 前置拦网。
# - 本地全特性口径实测约 93.2%（sdk 代码测试密度低 + 7 个基线失败测试缺
#   覆盖），故阈值取 90%：低于实测值保证可用，仍能拦住大幅回退。
# - `--skip` 排除 7 个已知基线失败（清单见 docs/TEST_PLAN.md「基线豁免」），
#   否则 llvm-cov 在测试失败时中止且不出报告。

if ! command -v cargo-llvm-cov >/dev/null 2>&1; then
  echo "::warning::cargo-llvm-cov 未安装，跳过覆盖率门禁"
  echo "安装: cargo install cargo-llvm-cov --locked"
  exit 0
fi

cargo llvm-cov --package nebulaid --all-features --ignore-filename-regex "server/proto/" \
  --fail-under-lines 90 \
  -- --skip test_run_migrations --skip e2e_database_run_migrations_creates_tables
