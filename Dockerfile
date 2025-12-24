FROM rust:1.75 AS builder

WORKDIR /app

RUN apt-get update && apt-get install -y protobuf-compiler pkg-config libssl-dev

COPY . .

RUN cargo build --release --bin nebula-id

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y libssl2t64 ca-certificates && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /app/target/release/nebula-id /app/nebula-id
COPY --from=builder /app/config.yaml /app/config.yaml

EXPOSE 8080 9091

ENTRYPOINT ["./nebula-id"]
