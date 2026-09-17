#!/bin/bash
# lefthook pre-push 覆盖率门禁（≥90%）
# 注：不写成 lefthook.yml 的多行 `run: |` 内联脚本——Windows 下 lefthook
# 会破坏多行脚本的换行语义（fi 与前一命令被空格拼接导致 if 永不闭合）。
#
# 口径说明：与 CI 的差异是有意为之——
# - 权威门禁在 ci.yml default leg（≥95%，不含 sdk 稀释）；本脚本只是
#   pre-push 前置拦网，采用全特性口径顺带验证 sdk 面的测试执行。
# - 本地全特性口径实测约 93%+（sdk 代码测试密度低稀释覆盖率），阈值取
#   90%：低于实测值保证可用，仍能拦住大幅回退。

if ! command -v cargo-llvm-cov >/dev/null 2>&1; then
  echo "::warning::cargo-llvm-cov 未安装，跳过覆盖率门禁"
  echo "安装: cargo install cargo-llvm-cov --locked"
  exit 0
fi

cargo llvm-cov --package nebulaid --all-features --ignore-filename-regex "server/proto/" \
  --fail-under-lines 90
