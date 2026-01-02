# ============================================
# Builder Stage
# ============================================
FROM rust:1.92-slim AS builder

# 安装构建依赖
RUN apt-get update && apt-get install -y \
    protobuf-compiler \
    pkg-config \
    libssl-dev \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# 优化层缓存：先复制依赖配置文件
COPY Cargo.toml Cargo.lock ./
COPY crates/ ./crates/

# 下载依赖（利用 Docker 缓存）
RUN cargo fetch

# 构建应用
ARG BUILD_PROFILE=release
RUN cargo build --${BUILD_PROFILE} --locked -p nebula-server --bin nebula-id

# ============================================
# Runtime Stage
# ============================================
FROM debian:bookworm-slim

# 安装运行时依赖
RUN apt-get update && apt-get install -y \
    libssl3 \
    ca-certificates \
    curl \
    && rm -rf /var/lib/apt/lists/*

# 创建非 root 用户
RUN groupadd --gid 1000 appgroup && \
    useradd --uid 1000 --gid appgroup --shell /bin/bash --create-home appuser

WORKDIR /app

# 复制二进制文件
COPY --from=builder /app/target/release/nebula-id /app/nebula-id

# 设置权限
RUN chown -R appuser:appgroup /app && \
    chmod +x /app/nebula-id

# 切换到非 root 用户
USER appuser

# 暴露端口
EXPOSE 8080 9091

# 环境变量
ENV RUST_LOG=info \
    RUST_BACKTRACE=0 \
    SERVER_HTTP_HOST=0.0.0.0 \
    SERVER_HTTP_PORT=8080 \
    METRICS_ENABLED=true \
    METRICS_HOST=0.0.0.0 \
    METRICS_PORT=9091

# 健康检查
HEALTHCHECK --interval=30s --timeout=10s --start-period=10s --retries=3 \
    CMD curl -f http://localhost:8080/health || exit 1

# 入口点
ENTRYPOINT ["./nebula-id"]
