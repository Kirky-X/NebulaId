#!/usr/bin/env bash
# Nebula ID Docker 镜像构建（单包 nebulaid）。
#
# 依赖已全部走 crates.io，无 [patch.crates-io] 本地条目，直接构建即可。
#
# 用法：./docker/build.sh [额外的 docker build 参数，如 -t 标签]
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

cd "$ROOT"
docker build \
  -f docker/Dockerfile \
  -t nebulaid:local \
  "$@" \
  .
