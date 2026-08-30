#!/usr/bin/env bash
# wiring T011: Nebula ID Docker 镜像构建（单包 nebulaid）。
#
# RC 期依赖约束：Cargo.toml 的 [patch.crates-io] 把 sdforge / inklog 钉在
# 本地兄弟目录 ../，构建上下文拿不到，需先用 vendor-deps.sh 生成瘦身副本，
# 再经 BuildKit 命名上下文（--build-context）注入。sdforge / inklog 正式版
# 发布并移除 patch 条目后，可退回普通 `docker build -f docker/Dockerfile .`。
#
# 用法：./docker/build.sh [额外的 docker build 参数，如 -t 标签]
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

"$SCRIPT_DIR/vendor-deps.sh"

cd "$ROOT"
docker build \
  -f docker/Dockerfile \
  --build-context sdforge=.docker-vendor/sdforge \
  --build-context inklog=.docker-vendor/inklog \
  -t nebulaid:local \
  "$@" \
  .
