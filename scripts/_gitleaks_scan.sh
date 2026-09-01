#!/bin/bash
# lefthook pre-commit 私钥/密钥扫描（gitleaks）
# 注：不写成 lefthook.yml 的多行 `run: |` 内联脚本——Windows 下 lefthook
# 会破坏多行脚本的换行语义（fi 与前一命令被空格拼接导致 if 永不闭合）。

if ! command -v gitleaks >/dev/null 2>&1; then
  echo "::warning::gitleaks 未安装，跳过私钥扫描"
  echo "安装: brew install gitleaks | go install github.com/zricethezlev/gitleaks/v8@latest"
  exit 0
fi

gitleaks protect --staged --redact --config .gitleaks.toml || {
  echo "::error::检测到潜在密钥泄漏，请检查 staged 文件"
  exit 1
}
