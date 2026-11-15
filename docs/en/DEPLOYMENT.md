# 🚀 Nebula ID Deployment Guide

> This document describes Nebula ID's Docker deployment, configuration, environment variables, and monitoring.

## 📋 Table of Contents

- [1. Deployment Architecture](#1-deployment-architecture)
- [2. Docker Deployment](#2-docker-deployment)
- [3. Configuration](#3-configuration)
- [4. Environment Variables](#4-environment-variables)
- [5. Health Checks and Monitoring](#5-health-checks-and-monitoring)
- [6. Database Initialization](#6-database-initialization)
- [7. Production Deployment Advice](#7-production-deployment-advice)
- [8. scripts/run.sh Subcommands](#8-scriptsrunsh-subcommands)
- [9. Kubernetes Operator Installation](#9-kubernetes-operator-installation)
- [10. Cross-Datacenter Failover](#10-cross-datacenter-failover)
- [Related Documents](#related-documents)

---

## 1. Deployment Architecture

```mermaid
graph TB
    subgraph Docker Compose
        App[Nebula ID App<br/>nebula-app<br/>:8080 HTTP<br/>:8443 HTTPS（可选）<br/>/health /ready /metrics<br/>:9091 gRPC]
        PG[(PostgreSQL 16<br/>nebula-postgres<br/>:5432)]
        Redis[(Redis 7.2<br/>nebula-redis<br/>:6379)]
        Etcd[(Etcd 3.5<br/>nebula-etcd<br/>:2379)]
    end

    Client[客户端] --> App
    App --> PG
    App --> Etcd
    App -.->|预留：当前无消费点<br/>缓存为进程内 oxcache| Redis

    Prom[Prometheus] -->|抓取 /metrics（Prometheus 文本）| App

    style App fill:#bbdefb
    style PG fill:#a5d6a7
    style Redis fill:#ce93d8
    style Etcd fill:#ffcc80
```

> Redis note: the `[redis]` section and `REDIS_URL` are a **reserved configuration surface** — the current version has no production consumer: segment double buffering (the `segment` DoubleBuffer) and the auth decision cache are both in-process (oxcache memory / garrison memory). A Redis failure therefore does not affect minting or authentication; `docker compose` still starts the service to keep the deployment shape aligned with future wiring (methodology: FAULT-08 in `docs/TEST_PLAN.md`).

## 2. Docker Deployment

### 2.1 Prerequisites

- Docker 20.10+
- Docker Compose v2
- 2GB+ available memory

### 2.2 Quick Start

```bash
# 1. Clone the repository
git clone https://github.com/Kirky-X/NebulaId.git
cd NebulaId

# 2. Configure environment variables
cp docker/.env.example docker/.env
# Edit docker/.env with the real passwords

# 3. Start all services
docker compose -f docker/docker-compose.yml --env-file docker/.env up -d

# 4. Check service status
docker compose -f docker/docker-compose.yml ps
docker compose -f docker/docker-compose.yml logs -f app
```

### 2.3 Docker Compose Services

| Service | Image | Ports | Resource limits | Health check |
|---------|-------|-------|-----------------|--------------|
| `postgres` | postgres:16-alpine | 5432 | 2 CPU / 2G RAM | `pg_isready` |
| `redis` | redis:7.2-alpine | 6379 | 0.5 CPU / 512M RAM | `redis-cli ping` |
| `etcd` | quay.io/coreos/etcd:v3.5.11 | 2379, 2380 | 0.5 CPU / 512M RAM | `etcdctl endpoint health` |
| `app` | locally built nebula-id:latest | 8080, 9091 | 2 CPU / 2G RAM | `curl /health` |

### 2.4 Building the Image Separately

```bash
# Build the image
docker build -f docker/Dockerfile -t nebula-id:latest .

# Run (needs external PostgreSQL/Redis/Etcd)
docker run -d \
  --name nebula-id \
  -p 8080:8080 -p 9091:9091 \
  -e DATABASE_URL=postgresql://idgen:password@host:5432/idgen \
  -e REDIS_URL=redis://host:6379/0 \
  -e ETCD_ENDPOINTS=host:2379 \
  -e NEBULA_API_KEY_SALT=$(openssl rand -hex 32) \
  nebula-id:latest
```

> **Note:** `docker/Dockerfile` is adapted to the single-package `nebulaid` build — the builder stage runs `cargo build --release --package nebulaid --features etcd` after `COPY . .` (the image must include the etcd feature so the `ETCD_ENDPOINTS` injected by compose can establish coordination locks); the runtime stage copies `target/release/nebula-id` plus `config/` and `locales/` (the removed `crates/` directory and the `nebula-server` package name are no longer referenced).

## 3. Configuration

The main config file is `config/config.toml`, with environment variable expansion (`${VAR}` syntax) supported.

### 3.1 Key Configuration Items

| Section | Key field | Description | Default |
|---------|-----------|-------------|---------|
| `[app]` | `dc_id` | Datacenter ID (0-7) | 0 |
| `[app]` | `worker_id` | Worker ID (0-255) | 0 |
| `[app]` | `http_port` | HTTP service port | 8080 |
| `[app]` | `grpc_port` | gRPC service port | 9091 |
| `[database]` | `password` | Database password (env expansion) | `${NEBULA_DATABASE_PASSWORD}` |
| `[algorithm]` | `default` | Default algorithm | snowflake (factory `config/config.toml` value; the code `Config::default()` is `segment`) |
| `[algorithm.segment]` | `base_step` | Segment base step | 1000 |
| `[algorithm.snowflake]` | `sequence_bits` | Sequence bit width | 10 |
| `[tls]` | `enabled` | Enable TLS | false |
| `[auth]` | `enabled` | Enable API key auth | true |
| `[rate_limit]` | `enabled` | Enable rate limiting | true |
| `[etcd]` | `endpoints` | Etcd endpoint list | `[]` (compose injects via `ETCD_ENDPOINTS`; build with `--features etcd`) |

> ⚠️ **Global rate limiting is enabled by default**: the rate-limit switch is the **top-level** `[rate_limit].enabled` (not `auth.rate_limit.enabled`), and `src/main.rs` reads `config.rate_limit.enabled` to decide whether to mount `RateLimitMiddleware`. `RateLimitConfig::default()` has `enabled = true`, and the repo's `config/config.toml` also sets `[rate_limit].enabled = true`, matching the default. Note that none of the three fields of the `[rate_limit]` section (`enabled` / `default_rps` / `burst_size`) carry `#[serde(default)]` — a missing section or single field fails the whole config parse.

### 3.2 Algorithm Bit Layout (Snowflake)

```text
| timestamp (43 bits) | datacenter_id (3 bits) | worker_id (8 bits) | sequence (10 bits) |
```

- Datacenter ID range: 0-7
- Worker ID range: 0-255
- Per-millisecond sequence range: 0-1023
- The timestamp uses 2024-01-01 as the epoch start

## 4. Environment Variables

See `docker/.env.example`:

### 4.1 Required in Production

| Variable | Description | How to generate |
|----------|-------------|-----------------|
| `POSTGRES_PASSWORD` | PostgreSQL password | `openssl rand -base64 32` |
| `NEBULA_DATABASE_PASSWORD` | Password expanded into the config file | `openssl rand -base64 32` |
| `NEBULA_API_KEY_SALT` | API key salt | `openssl rand -hex 32` |

### 4.2 Optional

| Variable | Default | Description |
|----------|---------|-------------|
| `APP_HTTP_PORT` | 8080 | HTTP port |
| `APP_GRPC_PORT` | 9091 | gRPC port |
| `DC_ID` | 0 | Datacenter ID |
| `RUST_LOG` | info | Log level (trace/debug/info/warn/error) |
| `RUST_BACKTRACE` | 0 | Error backtraces (0/1/full) |
| `DATABASE_URL` | - | Full database URL (overrides other config) |
| `ETCD_ENDPOINTS` | unset → empty list (single process) | etcd endpoints (comma-separated). **An empty string, or `endpoints = []` in the config file, means single-process deployment**: the app falls back to `LocalDistributedLock` and emits a warning (multi-instance deployments would mint duplicate ids; single-process only). Blank items such as trailing commas count as unconfigured; when unset and `[etcd]` is absent from the config file, the built-in default is likewise an empty list |
| `TLS_ENABLED` / `TLS_HTTP_ENABLED` / `TLS_GRPC_ENABLED` | false | TLS switches (env can only enable: `false` does not turn off TLS already enabled in the file). With `TLS_HTTP_ENABLED=true`, the HTTP port terminates as HTTPS |
| `TLS_CERT_PATH` / `TLS_KEY_PATH` / `TLS_CA_PATH` | - | Certificate chain / private key / optional custom CA paths |
| `NEBULA_LOCALE` | from `app.locale` (default `en`) | Process default locale (the earlier `LOCALE` was a doc-only fictional variable). Values: `en`, `zh-CN`; invalid values fall back to `en`. It only sets the `fluent-bundle` global locale, affecting startup logs and paths that bypass the `Accept-Language` middleware; the runtime response language of `/api/v1/*` routes is negotiated per request by the `Accept-Language` header and is unaffected by this variable. |

### 4.3 NEBULA_LOCALE and the locale Config in Detail

The process default locale resolves by priority: **the `NEBULA_LOCALE` environment variable > `app.locale` in the config file > built-in default `en`**; an invalid value (not in `en`/`zh-CN`) falls back to `en` with a warning.

The effective locale controls the service process's global default language, mainly affecting:

- **Startup logs**: during service startup, `tracing::{info,warn,error}!` output is translated per the effective locale (see `i18n::init_i18n(&locale)` in `src/main.rs`). Extremely early logs before config loading are fixed to `en`.
- **Logs for non-`/api/v1/*` paths**: paths such as `/health`, `/ready`, and `/metrics` do not pass through `locale_middleware`; their log messages use the global locale.
- **Requests without an `Accept-Language` header**: when `locale_middleware` negotiation fails it falls back to `Locale::DEFAULT` (i.e. `en`), **not** to the global locale. This is intentional — the global locale and the per-request locale are decoupled to avoid cross-request pollution.

**Configuration example:**

```bash
# Option 1: environment variable override (highest priority)
export NEBULA_LOCALE=zh-CN
./target/release/nebula-id

# Option 2: the [app] section of config/config.toml
#   [app]
#   locale = "zh-CN"

# Or configure it in docker-compose
# docker/.env:
#   NEBULA_LOCALE=zh-CN
```

> **Note**: the global locale affects log output language only, **not** the language of HTTP error responses on `/api/v1/*` routes. HTTP response language is negotiated independently per request by the `Accept-Language` header — see the [API Reference · Accept-Language](API_REFERENCE.md#accept-language-header).

## 5. Health Checks and Monitoring

### 5.1 Health Check Endpoints

```bash
# Application health check
curl http://localhost:8080/health

# A 200 OK response means the service is healthy
```

Docker Compose configures a `HEALTHCHECK` that runs every 30 seconds.

### 5.2 Prometheus Metrics

```bash
# Scrape metrics (same port as the HTTP service; no separate metrics port)
curl http://localhost:8080/metrics
```

`GET /metrics` exposes Prometheus text-format metrics on the main HTTP port
(`text/plain; version=0.0.4`), including:
- `nebula_id_ids_generated_total{algorithm}` — cumulative ids generated per algorithm (counter)
- `nebula_id_generate_latency_seconds{algorithm}` — generation latency histogram (for use with `histogram_quantile()`); p50/p99/p999 percentile gauges are also provided
- `nebula_id_generate_failures_total{algorithm}` / `nebula_id_clock_backwards_total{algorithm}` — failure and clock-rollback counters
- `nebula_id_algorithm_degraded{algorithm}` / `nebula_id_circuit_breaker_open{algorithm}` — degradation and circuit-breaker state gauges
- `nebula_id_requests_total` / `nebula_id_uptime_seconds` — API-layer request counts and uptime

Grafana dashboard: `grafana/dashboards/nebula_id.json`.

### 5.3 Logs

Logs are emitted as JSON to stdout and collected by the Docker log driver:

```bash
# Follow logs
docker compose -f docker/docker-compose.yml logs -f app

# Last 100 lines
docker compose -f docker/docker-compose.yml logs --tail 100 app
```

The log level is controlled by the `RUST_LOG` environment variable, with per-module configuration supported:
```bash
RUST_LOG=nebulaid::core::algorithm=debug,info
```

## 6. Database Initialization

```bash
# Initialize with init.sql (Docker Compose runs this automatically)
psql -U idgen -d idgen -f scripts/init.sql

# Or create the table manually
psql -U idgen -d idgen <<'EOF'
CREATE SCHEMA IF NOT EXISTS nebula_id;

CREATE TABLE IF NOT EXISTS nebula_id.nebula_segments (
    id BIGSERIAL PRIMARY KEY,
    workspace_id VARCHAR(255) NOT NULL,
    biz_tag VARCHAR(255) NOT NULL,
    current_id BIGINT NOT NULL,
    max_id BIGINT NOT NULL,
    step INT NOT NULL DEFAULT 1000,
    delta INT NOT NULL DEFAULT 1,
    dc_id INT NOT NULL DEFAULT 0,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT uq_nebula_segments_ws_tag_dc UNIQUE (workspace_id, biz_tag, dc_id)
);
EOF
```

## 7. Production Deployment Advice

### 7.0 TLS/HTTPS (docker compose)

TLS terminates on the HTTP port (the same listening socket selects the rustls acceptor by `tls.http_enabled`; no extra port). Compose maps host `8043` to container `8443` by default. To enable:

1. Put certificates in `docker/certs/` (`server.crt`/`server.key`, optionally `ca.crt`;
   for self-signed generation see that directory's README). Compose mounts them at `/app/certs:ro` in the container.
2. Set in `.env`: `TLS_ENABLED=true`, `TLS_HTTP_ENABLED=true`, `APP_HTTP_PORT=8443`,
   `HEALTHCHECK_PROTO=https` (add `TLS_CERT_PATH`/`TLS_KEY_PATH` if the certificate paths differ from the defaults).
3. `docker compose -f docker/docker-compose.yml up -d app`;
   verify with `curl -k https://localhost:8043/health`.

gRPC works the same way: with `TLS_GRPC_ENABLED=true` the gRPC port performs a TLS handshake (same certificate pair). In production (`NEBULA_ENV=production`), starting without TLS fail-fasts and refuses startup; explicit opt-out requires `NEBULA_ALLOW_INSECURE_TLS=1` (internal networks / transition periods only).

### 7.1 Security

- **Always** set `NEBULA_API_KEY_SALT`, `POSTGRES_PASSWORD`, and `NEBULA_DATABASE_PASSWORD`
- Enable TLS (`[tls].enabled = true`) and configure certificate paths
- Restrict the PostgreSQL/Redis/Etcd ports to the internal network
- Rotate API keys periodically
- **Auth cache invalidation window** (`[auth].cache_ttl_seconds`): the cache is in-process; revocation / rotation / disable takes effect immediately only on **the node handling that management request**, while other nodes keep accepting the old decision for up to one `cache_ttl_seconds` (default 300 seconds). For multi-replica deployments requiring near-real-time revocation, set `cache_ttl_seconds` to `0` to disable the cache (cost: a DB round trip + Argon2id per request). Note also: cache-hit requests do not write back to the database, so `last_used_at` likewise lags by up to one TTL and cannot serve as a real-time activity signal.

### 7.2 Performance

- PostgreSQL: set `shared_buffers` to 25% of memory and `effective_cache_size` to 75%
- Redis (reserved surface, currently no consumer): `maxmemory` 256MB and `maxmemory-policy allkeys-lru` suffice; retune when a cache layer is actually wired in
- Application: build with `--release`; `lto = "thin"` is already configured in Cargo.toml

### 7.3 High Availability

- Multiple datacenters: distinct `dc_id` values sharing one etcd cluster
- Database primary-replica replication: PostgreSQL streaming replication
- etcd cluster: 3 or 5 nodes (odd counts)

## 8. scripts/run.sh Subcommands

Since v0.2.0, all development and deployment scripts merge into the single entry point `scripts/run.sh`, replacing the earlier scattered scripts (`deploy`, `pre-commit-check`, `redis_test`, `test_api`, `install-pre-commit-hooks`, etc.). Legacy scripts were renamed to `_*_impl.sh` internals and are no longer invoked directly.

```bash
# Generic invocation
scripts/run.sh <subcommand> [args...]
# Equivalent to
./scripts/run.sh <subcommand> [args...]
```

### 8.1 Subcommand Overview

| Subcommand | Alias | Internal implementation | Purpose |
|------------|-------|-------------------------|---------|
| `deploy` | — | `_deploy_impl.sh` | Deploy the Nebula ID full stack via docker-compose |
| `lint` | `pre-commit` | `_pre_commit_impl.sh` | Run local CI pre-checks (fmt + clippy + test + security/docs/coverage) |
| `redis-test` | — | `_redis_test_impl.sh` | Run Redis integration tests (SET/GET/EXISTS/TTL/DELETE) |
| `api-test` | — | `tests/api_test.sh` | Run API endpoint tests; optional `server_url` argument |
| `install-hooks` | — | `_install_hooks_impl.sh` | Install git pre-commit hooks (Python `pre-commit` + the Rust toolchain) |
| `pre-commit` | `lint` | `_pre_commit_impl.sh` | Same as `lint`, runs local CI pre-checks |
| `help` | `--help`, `-h` | — | Show usage |

Unknown subcommands exit non-zero and print usage.

### 8.2 deploy — Deployment

```bash
# Deploy the Nebula ID full stack (PostgreSQL + Redis + Etcd + App)
./scripts/run.sh deploy
```

Internally calls `_deploy_impl.sh`, which:

1. Checks that Docker 20.10+ / Docker Compose v2 are installed
2. Reads `docker/.env` (passwords, ports, and other sensitive config)
3. Starts all services with `docker compose -f docker/docker-compose.yml up -d`
4. Waits for the PostgreSQL / Redis / Etcd health checks to pass (up to 30 retries, 2s apart)
5. Waits for the App's `/health` endpoint to return 200 OK
6. Prints each service's status and log entry points

Prerequisite: `docker/.env` is configured (see `docker/.env.example`).

### 8.3 lint / pre-commit — Local CI Pre-checks

```bash
# Run before every commit
./scripts/run.sh pre-commit
# Or the equivalent alias
./scripts/run.sh lint
```

Internally calls `_pre_commit_impl.sh`, running checks in this order (stops on the first failure):

1. **Format check**: `cargo fmt -- --check`
2. **Static analysis**: `cargo clippy --lib --bins -- -D warnings`
3. **Build**: `cargo build`
4. **Tests**: `cargo test --lib`
5. **Security audit**: `cargo deny check` (if installed)
6. **Docs build**: `cargo doc --no-deps`
7. **Coverage**: `cargo tarpaulin` (if installed)

To cover optional features or integration tests, use the all-features switch directly:
`cargo test --package nebulaid --all-features`. `--all-features` is buildable
(= default + etcd + alerting + sdk + integration-tests + openapi).

GitHub Actions CI (`.github/workflows/ci.yml`) does not go through `scripts/run.sh`; it invokes the toolchain directly (`cargo fmt` / `clippy` / `test` / `llvm-cov` / `deny` / `audit`). `scripts/run.sh` is the local development entry point — it shares the toolchain with CI but the call paths are independent.

### 8.4 redis-test — Redis Integration Tests

```bash
# Run the Redis operation integration tests (Redis must be listening on 6379 first)
./scripts/run.sh redis-test
```

Internally calls `_redis_test_impl.sh`, which verifies 7 Redis operations in sequence:

1. `PING` connectivity
2. `SET` write
3. `GET` read
4. `EXISTS` presence
5. `TTL` remaining time
6. `DEL` delete
7. Post-delete `EXISTS` verification

Prerequisite: Redis on `localhost:6379` (no password, default db 0). Start it with `docker compose -f docker/docker-compose.yml up -d redis`.

### 8.5 api-test — API Endpoint Tests

```bash
# Tests http://localhost:8080 by default
./scripts/run.sh api-test

# With an explicit server URL
./scripts/run.sh api-test http://localhost:8080
```

Internally calls `tests/api_test.sh` (a comprehensive suite) and runs end-to-end tests against the Nebula ID HTTP API:

- Health endpoints: `GET /health`, `GET /ready`
- Metrics endpoint: `GET /metrics`
- ID generation endpoints: `POST /api/v1/generate`, `POST /api/v1/generate/batch`
- ID parsing endpoint: `POST /api/v1/parse`
- Auth tests: `Authorization` header verification (the `ApiKey <key_id>:<key_secret>` scheme and Basic salted hash; `X-API-Key` is never used)
- Config endpoint: `GET /api/v1/config`

Each test case prints PASS/FAIL, with a final `PASSED`/`FAILED` tally.

Prerequisite: the Nebula ID service is running and listening on `server_url`.

### 8.6 install-hooks — Install git pre-commit hooks

```bash
# Install git pre-commit hooks (run once after cloning the repository)
./scripts/run.sh install-hooks
```

Internally calls `_install_hooks_impl.sh`, which:

1. Checks Python 3.8+ and pip, printing installation instructions if missing
2. Checks the Rust toolchain (`cargo`, `rustfmt`, `clippy`), installing via `rustup component add` if missing
3. Installs the `pre-commit` tool via `pip3 install --user pre-commit`
4. Runs `pre-commit install` to place the hook in `.git/hooks/pre-commit`
5. Runs `pre-commit run --all-files` to verify the hooks work

After installation, every `git commit` automatically runs the `pre-commit` hooks (see `.pre-commit-config.yaml`), including formatting, clippy, the security audit, and other checks.

### 8.7 help — Show Usage

```bash
./scripts/run.sh help
# Or
./scripts/run.sh --help
./scripts/run.sh -h
# With no arguments it also behaves as help
./scripts/run.sh
```

Prints usage and examples for every subcommand.

## 9. Kubernetes Operator Installation

`nebulaid-operator` is a standalone bin crate (`crates/nebulaid-operator`) that reconciles the desired topology declared by the `NebulaIdCluster` CRD into per-DC StatefulSets + Headless Services, and periodically probes `/health` to write back `status`. **The kube / kube-runtime / k8s-openapi dependencies exist only in that crate** — the main `nebulaid` dependency tree is untouched (`cargo tree -p nebulaid` verifies this).

### 9.1 Prerequisites

- Kubernetes 1.30+ (the CRD's `apiextensions.k8s.io/v1` and SSA apply need 1.22+; this checklist is validated against 1.30+)
- `kubectl` configured with the target cluster context
- Image: `nebulaid-operator:0.1.0` (built from this repo's `crates/nebulaid-operator`; the NebulaId image is pulled separately per `spec.version`)

### 9.2 Installation Steps

```bash
# 1. Namespace and CRD
kubectl create namespace nebulaid-system
kubectl apply -f deploy/crd/nebulaidcluster.yaml

# 2. RBAC (least privilege: nebulaidclusters + apps StatefulSets + core Services/Endpoints)
kubectl apply -f deploy/rbac/operator-clusterrole.yaml

# 3. The operator itself
kubectl apply -f deploy/manifest/operator-deployment.yaml

# 4. Verify
kubectl -n nebulaid-system rollout status deploy/nebulaid-operator
kubectl get crd nebulaidclusters.nebulaid.kirky.x
```

### 9.3 Creating a NebulaIdCluster

```bash
kubectl apply -f deploy/manifest/nebulaidcluster-sample.yaml   # 2-DC example
kubectl get nebulaidclusters prod -o wide
kubectl get sts,svc -l nebulaid.kirky.x/cluster=prod
```

`spec` fields: `replicas` (per-DC replica count), `datacenters[]` (`id` = dc bit identifier, `etcdEndpoints`),
`version` (NebulaId image tag). `status` is written back by the operator: `phase` (Pending/Ready/Degraded),
`activeDc`, `perDcHealth[]`, `endpoints[]`.

### 9.4 Permission Boundary

The ClusterRole grants only the CR itself/status, `apps/statefulsets`, `core/services`, `core/endpoints` (read-only),
and `events` (read-only); **it grants no secrets, configmaps, pods, or other unrelated resources**. StatefulSets/Services
created by the operator carry ownerReferences, so deleting the CR reclaims them together.

### 9.5 Uninstall

```bash
kubectl delete nebulaidcluster prod          # Reclaims the STS/Svc along with it
kubectl delete -f deploy/manifest/operator-deployment.yaml
kubectl delete -f deploy/rbac/operator-clusterrole.yaml
kubectl delete -f deploy/crd/nebulaidcluster.yaml
```

---

## 10. Cross-Datacenter Failover

### 10.1 Per-DC etcd Topology

Each DC owns an **independent** etcd cluster (`spec.datacenters[].etcdEndpoints`), and the NebulaId StatefulSets connect to their own DC's coordination plane. The topology is "active-active coordination + partitioned namespaces", not a shared single cluster:

- **dc-bit isolation**: the top 8 bits of a segment ID are the dc prefix (the `DcId` value domain is `u8`), leaving a local space of 2^56; with `dc=0` the prefix is zero and ID values are exactly identical to a single-DC deployment (backward compatible). The identifier space is partitioned per DC, so IDs never collide across DCs — **no cross-DC data replication is needed to guarantee uniqueness**.
- **Worker namespace**: etcd worker keys are `/idgen/workers/{dc}/{worker_id}`; CAS preemption within a DC, and namespaces are not shared across DCs.
- **Tri-state health**: each DC's etcd cluster is judged Healthy / Degraded / Failed independently (3 consecutive failures degrade, 5 consecutive failures mark dead); on failure the DC automatically switches to local-cache degradation (caller semantics unchanged).

### 10.2 Switching Semantics

Decision rules (deliberately conservative; the same shape at both layers: the in-process `DcFailoverController` and the operator's `status.activeDc`):

| Situation | Decision |
| --- | --- |
| active judged `Failed` and a `Healthy` candidate exists | Switch to the healthy candidate with the **smallest dc number** (deterministic: the same situation always yields the same target) |
| active `Healthy` or `Degraded` | Hold (switching during degradation would oscillate between the two DCs) |
| active state unknown (not yet probed) | Hold (missing evidence ≠ dead) |
| No `Healthy` candidate at all | Hold the current value (better to serve degraded than switch into an uncertain source) |

Concurrency correctness: the active switch goes through an atomic CAS, and only the switch that truly replaces active counts (a 16-thread concurrent decision measured exactly one `SwitchTo`). Minting traffic observes `active_dc()` at the `AlgorithmRouter` layer: after a switch, new segments fall into the new DC's bit-prefix band while old segments drain naturally — no data migration needed.

### 10.3 Verification Boundary (⚠️ Deferred Statement)

- **Verified at code level**: the switch predicates (unit tests: three shapes + boundaries), concurrent single-switch (16 threads), dc-bit partitioning (cross-DC namespaces never overlap), and real two-etcd cases (`tests/etcd_failover.rs`, triggered in CI by the `etcd-a`/`etcd-b` containers + `--ignored`).
- **Not verified (Deferred)**: end-to-end deployment and fault injection in a real multi-cloud/multi-cluster environment. This change does not claim verification on a real production cloud; once a target-cluster window is available, follow the steps in this section and backfill the verification record (reopen conditions in the Deferred section of change `cloud-native-dr`).

---

## Related Documents

- [Architecture](ARCHITECTURE.md)
- [API Reference](API_REFERENCE.md)
- [User Guide](USER_GUIDE.md)
- [Config Migration Guide](CONFIG_MIGRATION_GUIDE.md)
