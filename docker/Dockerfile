FROM rust:1.92 AS builder

WORKDIR /app

RUN apt-get update && apt-get install -y protobuf-compiler pkg-config libssl-dev ca-certificates && rm -rf /var/lib/apt/lists/*

COPY . .

RUN cargo fetch

RUN cargo build --release --locked -p nebula-server --bin nebula-id

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y libssl3 ca-certificates && rm -rf /var/lib/apt/lists/*

WORKDIR /app

RUN groupadd --gid 1000 appgroup && useradd --uid 1000 --gid appgroup --shell /bin/bash --create-home appuser

COPY --from=builder /app/target/release/nebula-id /app/nebula-id

RUN chown -R appuser:appgroup /app

USER appuser

EXPOSE 8080 9091

HEALTHCHECK --interval=30s --timeout=10s --start-period=5s --retries=3 \
    CMD wget --spider -q http://localhost:8080/health || exit 1

ENTRYPOINT ["./nebula-id"]
