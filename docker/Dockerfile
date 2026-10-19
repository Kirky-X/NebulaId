# ============================================
# Nebula ID Docker Build（单包 nebulaid）
# ============================================
# Multi-stage build for minimal image size
#
# 直接构建（依赖已全部走 crates.io，无本地 patch/命名上下文）：
#   docker build -f docker/Dockerfile .
#
# Build arguments:
#   BUILD_PROFILE=release|dev
#   RUST_VERSION=1.97.1（confers 0.6.0-rc.10 要求 rustc ≥ 1.97.1 / edition 2024）
# ============================================

# ---------- Builder Stage ----------
ARG RUST_VERSION=1.97.1
FROM rust:${RUST_VERSION}-bookworm AS builder

# Build arguments
ARG BUILD_PROFILE=release
ARG RUSTFLAGS="-C target-cpu=skylake"

# Install build dependencies（sdforge 的 build.rs 生成 protobuf 需要 protoc）
RUN apt-get update && apt-get install -y \
    protobuf-compiler \
    pkg-config \
    libssl-dev \
    ca-certificates \
    cmake \
    && rm -rf /var/lib/apt/lists/*

# Set build environment
ENV CARGO_NET_GIT_FETCH_WITH_CLI=false \
    CARGO_INCREMENTAL=1 \
    RUSTFLAGS="${RUSTFLAGS}"

WORKDIR /app

# Copy source code（.dockerignore 已排除 target/、.git 等）
COPY . .

# Build the single-package application
# etcd feature 必须随镜像编入：docker-compose 会注入 ETCD_ENDPOINTS，
# 未编译该 feature 时服务无法建立协调锁（已由启动 fail-closed 守卫拒绝）。
RUN cargo build --${BUILD_PROFILE} --locked \
    --package nebulaid \
    --features etcd \
    --jobs $(nproc)

# ---------- Runtime Stage ----------
# bookworm 与 builder 的 glibc 对齐（bullseye runtime 低于 builder 会报 GLIBC 版本缺失）。
FROM debian:bookworm-slim AS runtime

ARG BUILD_PROFILE=release

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    libssl3 \
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
# 8443 = TLS/HTTPS 监听端口（TLS 终止在 HTTP 端口，经 TLS_ENABLED/TLS_HTTP_ENABLED
# 与 APP_HTTP_PORT=8443 启用；compose 以 8043→8443 映射对外暴露）。
EXPOSE 8080 8443 9091 9092

# Environment variables
ENV RUST_LOG=info \
    RUST_BACKTRACE=0 \
    SERVER_HTTP_HOST=0.0.0.0 \
    SERVER_HTTP_PORT=8080 \
    METRICS_ENABLED=true \
    METRICS_HOST=0.0.0.0 \
    METRICS_PORT=9092
# TLS（默认关闭）：证书需挂载到镜像内（compose 默认 -v docker/certs:/app/certs:ro）；
# 开启示例：-e TLS_ENABLED=true -e TLS_HTTP_ENABLED=true -e APP_HTTP_PORT=8443
#          -e TLS_CERT_PATH=/app/certs/server.crt -e TLS_KEY_PATH=/app/certs/server.key
ENV TLS_ENABLED=false \
    TLS_HTTP_ENABLED=false \
    TLS_GRPC_ENABLED=false

# Health check
HEALTHCHECK --interval=30s --timeout=10s --start-period=10s --retries=3 \
    CMD curl -f http://localhost:8080/health || exit 1

# Entrypoint
ENTRYPOINT ["./nebula-id"]
