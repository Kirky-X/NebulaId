# ============================================
# Nebula ID Docker Build（单包 nebulaid）
# ============================================
# Multi-stage build for minimal image size
#
# ⚠ RC 期依赖约束：Cargo.toml 的 [patch.crates-io] 把 sdforge / inklog 钉在
#   本地兄弟目录 ../，构建上下文拿不到。请用 ./docker/build.sh 构建 ——
#   它先经 vendor-deps.sh 生成瘦身副本，再以 BuildKit 命名上下文注入：
#     ./docker/build.sh
#   sdforge / inklog 正式版发布并移除 patch 条目后，可退回普通
#     docker build -f docker/Dockerfile .
#
# Build arguments:
#   BUILD_PROFILE=release|dev
#   RUST_VERSION=1.92.0
# ============================================

# ---------- Builder Stage ----------
FROM rust:1.92.0-bullseye AS builder

# Build arguments
ARG BUILD_PROFILE=release
ARG RUSTFLAGS="-C target-cpu=skylake"

# Install build dependencies（sdforge 的 build.rs 生成 protobuf 需要 protoc）
RUN apt-get update && apt-get install -y \
    protobuf-compiler \
    pkg-config \
    libssl-dev \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Set build environment
ENV CARGO_NET_GIT_FETCH_WITH_CLI=false \
    CARGO_INCREMENTAL=1 \
    RUSTFLAGS="${RUSTFLAGS}"

# 本地 RC 依赖（命名构建上下文注入，见文件头说明）。置于根目录，使
# [patch.crates-io] 相对 /app 的 ../sdforge、../inklog 可解析。
COPY --from=sdforge / /sdforge
COPY --from=inklog / /inklog

WORKDIR /app

# Copy source code（.dockerignore 已排除 target/、.git 等）
COPY . .

# Build the single-package application
RUN cargo build --${BUILD_PROFILE} --locked \
    --package nebulaid \
    --jobs $(nproc)

# ---------- Runtime Stage ----------
FROM debian:bullseye-slim AS runtime

ARG BUILD_PROFILE=release

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    libssl1.1 \
    ca-certificates \
    curl \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 1000 appgroup \
    && useradd --uid 1000 --gid appgroup --shell /bin/bash --create-home appuser

WORKDIR /app

# Copy binary and runtime assets from builder
# （locales 已在编译期经 i18n! 宏嵌入二进制，随包拷贝仅作冗余兜底；
#   config/ 为运行时默认配置路径 config/config.toml 所需）
COPY --from=builder /app/target/${BUILD_PROFILE}/nebula-id /app/nebula-id
COPY --from=builder /app/config /app/config
COPY --from=builder /app/locales /app/locales

# Set permissions
RUN chown -R appuser:appgroup /app && \
    chmod +x /app/nebula-id

# Switch to non-root user
USER appuser

# Expose ports
EXPOSE 8080 50051 9091

# Environment variables
ENV RUST_LOG=info \
    RUST_BACKTRACE=0 \
    SERVER_HTTP_HOST=0.0.0.0 \
    SERVER_HTTP_PORT=8080 \
    METRICS_ENABLED=true \
    METRICS_HOST=0.0.0.0 \
    METRICS_PORT=9091

# Health check
HEALTHCHECK --interval=30s --timeout=10s --start-period=10s --retries=3 \
    CMD curl -f http://localhost:8080/health || exit 1

# Entrypoint
ENTRYPOINT ["./nebula-id"]
