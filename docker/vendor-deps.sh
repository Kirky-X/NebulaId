#!/usr/bin/env bash
# wiring T011: 为 Docker 构建准备本地 RC 依赖的瘦身副本。
#
# Cargo.toml 的 [patch.crates-io] 把 sdforge / inklog 钉在本地兄弟目录 ../，
# 而 Docker 构建上下文无法越过仓库边界。本脚本把这两个仓库（排除 target/、
# .git/、logs/ 等构建产物）打包进 .docker-vendor/，随后由 build.sh 通过
# BuildKit 命名上下文（--build-context）注入，使镜像内的相对路径
# ../sdforge、../inklog 可解析（Dockerfile 将它们放在 /sdforge、/inklog）。
#
# sdforge / inklog 正式版发布并移除 patch 条目后，本脚本可删除。
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
STAGE="$ROOT/.docker-vendor"

vendor_one() {
  local name="$1"
  local src="$ROOT/../$1"
  local dst="$STAGE/$name"
  if [[ ! -d "$src" ]]; then
    echo "[vendor-deps] ERROR: sibling repo not found: $src" >&2
    echo "[vendor-deps] it is required by [patch.crates-io] in Cargo.toml" >&2
    exit 1
  fi
  rm -rf "$dst"
  mkdir -p "$dst"
  tar -C "$src" \
    --exclude='./target' \
    --exclude='./.git' \
    --exclude='./logs' \
    -cf - . | tar -C "$dst" -xf -
  echo "[vendor-deps] vendored $name -> $dst"
}

vendor_one sdforge
vendor_one inklog
echo "[vendor-deps] done: $STAGE"
