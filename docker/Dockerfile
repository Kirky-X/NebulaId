# ============================================
# Nebula ID Docker Build
# ============================================
# Multi-stage build for minimal image size
# Build arguments:
#   BUILD_PROFILE=release|dev
#   RUST_VERSION=1.92.0
# ============================================

# ---------- Builder Stage ----------
FROM rust:1.92.0-bullseye AS builder

# Build arguments
ARG BUILD_PROFILE=release
ARG RUSTFLAGS="-C target-cpu=skylake"

# Install build dependencies
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

WORKDIR /app

# Pre-build: Copy dependency files first (Docker layer caching optimization)
COPY Cargo.toml Cargo.lock ./
COPY crates/ ./crates/
COPY protos/ ./protos/

# Download and cache dependencies
RUN cargo fetch && \
    mkdir -p target && \
    echo '{}' > target/.cargo-lock

# Copy source code
COPY . .

# Build the application
RUN cargo build --${BUILD_PROFILE} --locked \
    -p nebula-server --bin nebula-id \
    --jobs $(nproc)

# ---------- Runtime Stage ----------
FROM debian:bullseye-slim AS runtime

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    libssl1.1 \
    ca-certificates \
    curl \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 1000 appgroup \
    && useradd --uid 1000 --gid appgroup --shell /bin/bash --create-home appuser

WORKDIR /app

# Copy binary from builder
COPY --from=builder /app/target/${BUILD_PROFILE}/nebula-id /app/nebula-id

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
