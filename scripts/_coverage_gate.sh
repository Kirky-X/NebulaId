#!/bin/bash
# lefthook pre-push 覆盖率门禁（≥80%）
# 注：不写成 lefthook.yml 的多行 `run: |` 内联脚本——Windows 下 lefthook
# 会破坏多行脚本的换行语义（fi 与前一命令被空格拼接导致 if 永不闭合）。

if ! command -v cargo-llvm-cov >/dev/null 2>&1; then
  echo "::warning::cargo-llvm-cov 未安装，跳过覆盖率门禁"
  echo "安装: cargo install cargo-llvm-cov --locked"
  exit 0
fi

cargo llvm-cov --package nebulaid --features etcd --fail-under-lines 80
