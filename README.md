<div align="center">

# 🚀 Nebula ID

[![GitHub release](https://img.shields.io/github/v/release/Kirky-X/NebulaId)](https://github.com/Kirky-X/NebulaId/releases) [![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-green)](./LICENSE) [![CI](https://img.shields.io/github/actions/workflow/status/Kirky-X/NebulaId/ci.yml?branch=main)](https://github.com/Kirky-X/NebulaId/actions/workflows/ci.yml) [![Security](https://img.shields.io/github/actions/workflow/status/Kirky-X/NebulaId/codeql.yml?branch=main&label=security)](https://github.com/Kirky-X/NebulaId/actions/workflows/codeql.yml)

<p align="center"><a href="./README_zh.md">中文文档</a> | <b>English</b></p>

<p align="center">
  <strong>Enterprise-grade distributed ID generation system for high-performance applications</strong>
</p>

<p align="center">
  <a href="#-features">Features</a> •
  <a href="#-quick-start">Quick Start</a> •
  <a href="#-documentation">Documentation</a> •
  <a href="#-examples">Examples</a> •
  <a href="#-contributing">Contributing</a>
</p>

</div>

---

## 📋 Table of Contents

<details open>
<summary>Click to expand</summary>

- [✨ Features](#-features)
- [🎯 Use Cases](#-use-cases)
- [🚀 Quick Start](#-quick-start)
  - [Installation](#installation)
  - [Basic Usage](#basic-usage)
- [📚 Documentation](#-documentation)
- [🎨 Examples](#-examples)
- [🏗️ Architecture](#️-architecture)
- [⚙️ Configuration](#️-configuration)
- [🧪 Testing](#-testing)
- [📊 Performance](#-performance)
- [🔒 Security](#-security)
- [🌐 Internationalization](#-internationalization)
- [🛠️ scripts/run.sh Usage](#️-scriptsrunsh-usage)
- [🗺️ Roadmap](#️-roadmap)
- [🤝 Contributing](#-contributing)
- [📄 License](#-license)
- [🙏 Acknowledgments](#-acknowledgments)

</details>

---

## ✨ Features

<table>
<tr>
<td width="50%">

### 🎯 Core Features

- ✅ **Multiple ID Algorithms** - Segment, Snowflake, UUID v7, UUID v4
- ✅ **Distributed Coordination** - Etcd-based leader election and coordination
- ✅ **High Availability** - Datacenter health monitoring and automatic failover
- ✅ **Type-Safe Design** - Full Rust type safety with async/await patterns

</td>
<td width="50%">

### ⚡ Advanced Features

- 🚀 **High Performance** - Million+ IDs per second with concurrent access
- 🔐 **API Security** - API key authentication and rate limiting
- 📊 **Monitoring** - Built-in metrics, health checks, and alerting
- 🌐 **Multi-Protocol** - HTTP/HTTPS REST API and gRPC/gRPCS support

</td>
</tr>
</table>

<div align="center">

### 🎨 Feature Highlights

</div>

```mermaid
graph LR
    A[Client Applications] --> B[Nebula ID Service]
    B --> C[Algorithm Router]
    C --> D[Segment Algorithm]
    C --> E[Snowflake Algorithm]
    C --> F[UUID v7/v4]
    B --> G[Distributed Coordination]
    G --> H[Etcd]
    B --> I[Monitoring]
    I --> J[Health Checks]
    I --> K[Metrics]
```

---

## 🎯 Use Cases

<details>
<summary><b>💼 Distributed Systems</b></summary>

<br>

```rust
use nebula_core::algorithm::{SegmentAlgorithm, SnowflakeAlgorithm};

// Segment algorithm for ordered, high-throughput ID generation
let segment = SegmentAlgorithm::new(1);
let id = segment.generate_id()?;

// Snowflake algorithm for globally unique IDs
let snowflake = SnowflakeAlgorithm::new(1, 1);
let id = snowflake.generate_id()?;
```

Perfect for large-scale distributed systems requiring unique, ordered identifiers with high availability.

</details>

<details>
<summary><b>🔧 Microservices</b></summary>

<br>

```rust
use nebula_core::types::Id;
use uuid::Uuid;

// Generate UUID v7 for time-ordered identifiers
let uuid_v7 = Uuid::now_v7();
let id = Id::from_uuid_v7(uuid_v7);
let id_string = id.to_string();

// Generate UUID v4 for random identifiers
let uuid_v4 = Uuid::new_v4();
let id_v4 = Id::from_uuid_v4(uuid_v4);
```

Ideal for microservices requiring unique identifiers with different ordering guarantees.

</details>

<details>
<summary><b>🌐 High-Performance Applications</b></summary>

<br>

```rust
use nebula_core::algorithm::SegmentAlgorithm;

// Double buffering for maximum throughput
let segment = SegmentAlgorithm::new(1);
let id = segment.generate_id()?;
```

Great for high-performance applications requiring millions of IDs per second with low latency.

</details>

---

## 🚀 Quick Start

### Installation

<table>
<tr>
<td width="50%">

#### 🦀 Build from Source

```bash
# Clone the repository
git clone https://github.com/Kirky-X/NebulaId.git
cd NebulaId

# Build all features
cargo build --all-features --release

# Run the server
./target/release/nebula-id
```

</td>
<td width="50%">

#### 📦 Feature Flags

```toml
# Cargo.toml features
[features]
default = ["postgresql"]
postgresql = ["sea-orm/sqlx-postgres", "sqlx/postgres"]
sqlite    = ["sea-orm/sqlx-sqlite", "sqlx/sqlite"]
etcd      = ["dep:etcd-client"]
```

**Build with specific features:**
```bash
# Default (PostgreSQL)
cargo build --release

# With etcd distributed coordination
cargo build --release --features etcd

# With SQLite (no PostgreSQL)
cargo build --release --no-default-features --features sqlite

# All features
cargo build --all-features --release
```

</td>
</tr>
</table>

### Basic Usage

<div align="center">

#### 🎬 5-Minute Quick Start

</div>

<table>
<tr>
<td width="50%">

**Step 1: Create Configuration**

```toml
[algorithm]
type = "segment"

[database]
url = "postgresql://user:pass@localhost/nebula"
max_connections = 10

[redis]
url = "redis://localhost"
```

</td>
<td width="50%">

**Step 2: Initialize Service**

```rust
use nebula_core::Config;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::load_from_file("config.toml")?;
    
    let service = nebula_server::NebulaIdService::new(config).await?;
    service.start().await?;
    
    Ok(())
}
```

</td>
</tr>
</table>

<details>
<summary><b>📖 Complete Example</b></summary>

<br>

```rust
use nebula_core::algorithm::SegmentAlgorithm;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let segment = SegmentAlgorithm::new(1);
    let id = segment.generate_id()?;
    
    println!("Generated ID: {}", id);
    Ok(())
}
```

</details>

---

## 📚 Documentation

<div align="center">

<table>
<tr>
<td align="center" width="25%">
<a href="https://docs.rs/nebula-id">
<img src="https://img.icons8.com/fluency/96/000000/api.png" width="64" height="64"><br>
<b>API Reference</b>
</a><br>
Full API documentation
</td>
<td align="center" width="25%">
<a href="examples/">
<img src="https://img.icons8.com/fluency/96/000000/code.png" width="64" height="64"><br>
<b>Examples</b>
</a><br>
Code examples
</td>
<td align="center" width="25%">
<a href="https://github.com/nebula-id/nebula-id">
<img src="https://img.icons8.com/fluency/96/000000/github.png" width="64" height="64"><br>
<b>GitHub</b>
</a><br>
Source code
</td>
<td align="center" width="25%">
<a href="https://crates.io/crates/nebula-id">
<img src="https://img.icons8.com/fluency/96/000000/package.png" width="64" height="64"><br>
<b>Crates.io</b>
</a><br>
Package registry
</td>
</tr>
</table>

</div>

### 📖 Additional Resources

- 🎓 **Algorithm Selection** - Choosing the right ID generation algorithm
- 🔧 **Configuration Guide** - Complete configuration reference
- ❓ **FAQ** - Frequently asked questions about distributed ID generation

---

## 🎨 Examples

<div align="center">

### 💡 Real-world Examples

</div>

<table>
<tr>
<td width="50%">

#### 📝 Example 1: Segment Algorithm

```rust
use nebula_core::algorithm::SegmentAlgorithm;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize with local datacenter ID
    let segment = SegmentAlgorithm::new(1);
    
    // Generate IDs
    let id = segment.generate_id()?;
    
    println!("Generated ID: {}", id);
    Ok(())
}
```

<details>
<summary>View output</summary>

```
Segment ID generated: 1000001
```

</details>

</td>
<td width="50%">

#### 🔥 Example 2: Snowflake Algorithm

```rust
use nebula_core::algorithm::SnowflakeAlgorithm;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize with datacenter and worker IDs
    let snowflake = SnowflakeAlgorithm::new(1, 1);
    
    // Generate IDs
    let id = snowflake.generate_id()?;
    
    println!("Generated Snowflake ID: {}", id);
    Ok(())
}
```

<details>
<summary>View output</summary>

```
Datacenter: 1, Worker: 1
Snowflake ID generated: 4200000000000000001
```

</details>

</td>
</tr>
</table>

<div align="center">

**[📂 View All Examples →](examples/)**

</div>

---

## 🏗️ Architecture

<div align="center">

### System Overview

</div>

```mermaid
graph TB
    A[Client Applications] --> B[API Gateway]
    B --> C[HTTP REST API]
    B --> D[gRPC Service]
    C --> E[Algorithm Router]
    D --> E
    E --> F[Segment Algorithm]
    E --> G[Snowflake Algorithm]
    E --> H[UUID v7/v4]
    F --> I[(Database)]
    G --> J[Distributed Coordination]
    J --> K[Etcd]
    H --> L[(Cache)]
    E --> M[Monitoring]
    M --> N[Health Checks]
    M --> O[Metrics]
    
    style A fill:#e1f5ff
    style B fill:#b3e5fc
    style C fill:#81d4fa
    style D fill:#4fc3f7
    style E fill:#29b6f6
    style F fill:#03a9f4
    style G fill:#03a9f4
    style H fill:#03a9f4
```

<details>
<summary><b>📐 Component Details</b></summary>

<br>

| Component | Description | Status |
|-----------|-------------|--------|
| **Algorithm Router** | Routes ID generation requests to appropriate algorithm | ✅ Stable |
| **Segment Algorithm** | Database-based segment ID generation with double buffering | ✅ Stable |
| **Snowflake Algorithm** | Twitter Snowflake variant for distributed unique IDs | ✅ Stable |
| **UUID Generator** | UUID v7 and v4 implementation | ✅ Stable |
| **Distributed Coordination** | Etcd-based leader election and coordination | ✅ Stable |
| **Monitoring** | Health checks, metrics collection, and alerting | ✅ Stable |
| **API Gateway** | HTTP/HTTPS and gRPC/gRPCS endpoint management | ✅ Stable |

</details>

---

## ⚙️ Configuration

<div align="center">

### 🎛️ Configuration Options

</div>

<table>
<tr>
<td width="50%">

**Basic Configuration (config.toml)**

```toml
[app]
name = "nebula-id"
host = "0.0.0.0"
port = 8080

[algorithm]
type = "segment"

[database]
url = "postgresql://user:pass@localhost/nebula"
max_connections = 10

[redis]
url = "redis://localhost"

[etcd]
endpoints = ["http://localhost:2379"]

[auth]
api_key = "your-api-key-here"

[rate_limit]
requests_per_second = 1000

[tls]
enabled = false
```

</td>
<td width="50%">

**Environment Variables**

```bash
export NEBULA_APP_NAME="nebula-id"
export NEBULA_APP_PORT="8080"
export NEBULA_DATABASE_URL="postgresql://user:pass@localhost/nebula"
export NEBULA_REDIS_URL="redis://localhost"
export NEBULA_ETCD_ENDPOINTS="http://localhost:2379"
export NEBULA_AUTH_API_KEY="your-api-key-here"
```

</td>
</tr>
</table>

<details>
<summary><b>🔧 All Configuration Options</b></summary>

<br>

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `app.name` | String | "nebula-id" | Application name |
| `app.host` | String | "0.0.0.0" | Server bind address |
| `app.port` | u16 | 8080 | Server port |
| `algorithm.type` | String | "segment" | ID generation algorithm |
| `database.url` | String | - | Database connection URL |
| `database.max_connections` | u32 | 1200 | Connection pool size |
| `redis.url` | String | - | Redis connection URL |
| `etcd.endpoints` | Vec&lt;String&gt; | [] | Etcd server endpoints |
| `auth.api_key` | String | - | API key for authentication |
| `rate_limit.requests_per_second` | u32 | 1000 | Rate limit threshold |
| `tls.enabled` | Boolean | false | Enable TLS/SSL |
</td>
</tr>
</table>

### Algorithm Configuration

<table>
<tr>
<td width="50%">

**Segment Algorithm**

```toml
[algorithm.segment]
name = "default"
step = 1000
max_retry = 3
```

</td>
<td width="50%">

**Snowflake Algorithm**

```toml
[algorithm.snowflake]
datacenter_id = 1
worker_id = 1
sequence_bits = 12
```

</td>
</tr>
</table>

> **注意**: 详细配置说明请参考 [Configuration Guide](#-documentation)。

</details>

---

## 🧪 Testing

<div align="center">

### 🎯 Test Coverage

</div>

```bash
# Run all tests
cargo test --all-features

# Run with coverage
cargo tarpaulin --out Html

# Run specific test
cargo test test_name

# Run integration tests
cargo test --test integration

# Run pre-commit checks (format, lint, build, test, security, docs, coverage)
./scripts/run.sh pre-commit
```

<details>
<summary><b>📊 Test Statistics</b></summary>

<br>

| Category | Tests | Coverage |
|----------|-------|----------|
| Unit Tests | 453 | 95%+ |
| Integration Tests | 42 | 95%+ |
| **Total** | **495+** | **95%+** |

> Since v0.2.0, the CI coverage gate has been raised to `--fail-under-lines 95` (see `.github/workflows/ci.yml`).

</details>

---

## 📊 Performance

<div align="center">

### ⚡ Benchmark Results

</div>

<table>
<tr>
<td width="50%">

**ID Generation Throughput**

```
Segment: 100,000+ IDs/sec
Snowflake: 1,000,000+ IDs/sec
UUID v7: 500,000+ IDs/sec
UUID v4: 1,000,000+ IDs/sec
```

</td>
<td width="50%">

**Latency (P99)**

```
Segment: ~0.5ms
Snowflake: ~0.1ms
UUID v7: ~0.05ms
UUID v4: ~0.05ms
```

</td>
</tr>
</table>

<details>
<summary><b>📈 Detailed Benchmarks</b></summary>

<br>

```bash
# Run benchmarks
cargo bench

# Sample output:
test segment_next_id    ... bench: 500 ns/iter (+/- 50)
test snowflake_next_id  ... bench: 100 ns/iter (+/- 10)
test uuid_v7_next_id    ... bench: 50 ns/iter (+/- 5)
test uuid_v4_next_id    ... bench: 50 ns/iter (+/- 5)
```

</details>

---

## 🔒 Security

<div align="center">

### 🛡️ Security Features

</div>

<table>
<tr>
<td align="center" width="33%">
<img src="https://img.icons8.com/fluency/96/000000/lock.png" width="64" height="64"><br>
<b>API Authentication</b><br>
API key-based authentication with timing attack protection
</td>
<td align="center" width="33%">
<img src="https://img.icons8.com/fluency/96/000000/security-checked.png" width="64" height="64"><br>
<b>Rate Limiting</b><br>
Configurable rate limits to prevent abuse
</td>
<td align="center" width="33%">
<img src="https://img.icons8.com/fluency/96/000000/privacy.png" width="64" height="64"><br>
<b>Audit Logging</b><br>
Track all ID generation operations
</td>
</tr>
</table>

<details>
<summary><b>🔐 Security Details</b></summary>

<br>

### Security Measures

- ✅ **API Key Authentication** - Secure API access with API key authentication using constant-time comparison to prevent timing attacks
- ✅ **Rate Limiting** - Configurable rate limits to prevent abuse and DoS attacks (max batch size: 100)
- ✅ **Audit Logging** - Full operation tracking for compliance and monitoring with IP spoofing protection
- ✅ **TLS Support** - HTTPS and gRPCS for encrypted communication (TLS 1.2/1.3)
- ✅ **CORS Restrictions** - Strict cross-origin resource sharing policies
- ✅ **Security Headers** - X-Content-Type-Options, X-Frame-Options, CSP, HSTS, X-XSS-Protection, Referrer-Policy
- ✅ **IP Spoofing Protection** - Trusted proxy validation for X-Forwarded-For headers

### Feature Flags

```toml
[dependencies.nebula-id]
version = "0.2.0"
features = ["audit", "tls"]
```

</details>

---

## 🌐 Internationalization

<div align="center">

### 🌍 ICU i18n Support (new in v0.2.0)

</div>

Nebula ID ships with built-in ICU internationalization since v0.2.0, powered by [`rust-i18n`](https://crates.io/crates/rust-i18n) `3.1`. It covers runtime translation of error messages and log entries.

**Supported locale matrix:**

| Locale tag | Language | Locales file | Status |
|------------|----------|--------------|--------|
| `en` | English (default) | `locales/en.yml` | ✅ Complete |
| `zh-CN` | Simplified Chinese | `locales/zh-CN.yml` | ✅ Complete |

**Negotiation flow:**

1. The client declares preferred languages via the HTTP `Accept-Language` header (per [RFC 7231 5.3.5](https://www.rfc-editor.org/rfc/rfc7231#section-5.3.5)), e.g. `Accept-Language: zh-CN,zh;q=0.9,en;q=0.8`.
2. `locale_middleware` (`src/server/middleware/locale.rs`) parses the header, sorts candidates by descending q-value, and picks the first supported locale (exact match wins; otherwise prefix match such as `zh` → `zh-CN`).
3. On missing header or no match, the default locale `en` is used.
4. Business handlers read the negotiated result via `Extension<Locale>` and translate error response messages with `translate_with_locale_args`.

**curl examples:**

```bash
# Chinese error response
curl -H "Accept-Language: zh-CN" http://localhost:8080/api/v1/invalid
# {
#   "code": 404,
#   "message": "未找到路径",
#   "details": "..."
# }

# English error response (default)
curl http://localhost:8080/api/v1/invalid
# {
#   "code": 404,
#   "message": "Path not found",
#   "details": "..."
# }
```

> **Security note**: `Locale` is derived from user input (the `Accept-Language` header) and is forgeable. Do **not** use it for any authentication, authorization, or security decision it is intended solely for content negotiation.

See [API Reference  Accept-Language](docs/API_REFERENCE.md#accept-language-header) and [Architecture  i18n module](docs/ARCHITECTURE.md#8-i18n-module-position) for details.

---

## 🛠️ scripts/run.sh Usage

<div align="center">

### 📦 Unified Script Entry (new in v0.2.0)

</div>

Since v0.2.0 all development/deployment scripts are merged into a single entry point `scripts/run.sh`, replacing the scattered v0.1.x scripts (`deploy`, `pre-commit-check`, `redis_test`, `test_api`, `install-pre-commit-hooks`, etc.). The legacy scripts have been renamed to `_*_impl.sh` internal implementations and are no longer invoked directly.

**Subcommand overview:**

| Subcommand | Alias | Purpose | Internal impl |
|------------|-------|---------|---------------|
| `deploy` | — | Deploy Nebula ID via docker-compose | `_deploy_impl.sh` |
| `lint` | `pre-commit` | Run local CI pre-checks (fmt + clippy + test + security/docs/coverage) | `_pre_commit_impl.sh` |
| `redis-test` | — | Run Redis integration tests | `_redis_test_impl.sh` |
| `api-test` | — | Run API endpoint tests, optional `server_url` argument | `_api_test_impl.sh` |
| `install-hooks` | — | Install git pre-commit hooks | `_install_hooks_impl.sh` |
| `pre-commit` | `lint` | Same as `lint`, runs local CI pre-checks | `_pre_commit_impl.sh` |
| `help` | `--help`, `-h` | Show usage information | — |

**Examples:**

```bash
# Show help
./scripts/run.sh help

# Deploy (docker-compose full stack)
./scripts/run.sh deploy

# Local CI pre-checks (must run before commit)
./scripts/run.sh pre-commit
# Or the equivalent alias
./scripts/run.sh lint

# Redis integration tests (requires Redis listening on 6379)
./scripts/run.sh redis-test

# API endpoint tests (defaults to http://localhost:8080)
./scripts/run.sh api-test
# Specify server URL
./scripts/run.sh api-test http://localhost:8080

# Install git pre-commit hooks
./scripts/run.sh install-hooks
```

**GitHub Actions integration:**

CI calls go through the same entry point (see `.github/workflows/ci.yml`, `release.yml`, `health-check.yml`), keeping local and CI behavior identical.

See [Deployment Guide  scripts/run.sh Subcommands](docs/DEPLOYMENT.md#8-scriptsrunsh-subcommands) for details.

---

## 🗺️ Roadmap

<div align="center">

### 🎯 Development Timeline

</div>

<table>
<tr>
<td width="50%">

### ✅ Completed

- [x] Core ID generation algorithms
- [x] Segment algorithm with double buffering
- [x] Snowflake algorithm
- [x] UUID v7/v4 implementation
- [x] Distributed coordination with Etcd

</td>
<td width="50%">

### 🚧 In Progress

- [ ] Enhanced monitoring and alerting
- [ ] Multi-datacenter support
- [ ] Performance optimization
- [ ] Client SDK improvements

</td>
</tr>
<tr>
<td width="50%">

### 📋 Planned

- [ ] Automatic failover
- [ ] Dynamic algorithm switching
- [ ] Custom ID format support
- [ ] Cloud provider integrations

</td>
<td width="50%">

### 💡 Future Ideas

- [ ] Kubernetes operator
- [ ] Multi-region deployment
- [ ] GraphQL API
- [ ] ID namespace management

</td>
</tr>
</table>

---

## 🤝 Contributing

<div align="center">

### 💖 We Love Contributors!

</div>

<table>
<tr>
<td width="33%" align="center">

### 🐛 Report Bugs

Found a bug?<br>
[Create an Issue](https://github.com/nebula-id/nebula-id/issues)

</td>
<td width="33%" align="center">

### 💡 Request Features

Have an idea?<br>
[Start a Discussion](https://github.com/nebula-id/nebula-id/discussions)

</td>
<td width="33%" align="center">

### 🔧 Submit PRs

Want to contribute?<br>
[Fork & PR](https://github.com/nebula-id/nebula-id/pulls)

</td>
</tr>
</table>

<details>
<summary><b>📝 Contribution Guidelines</b></summary>

<br>

### How to Contribute

1. **Fork** the repository
2. **Clone** your fork: `git clone https://github.com/yourusername/nebula-id.git`
3. **Create** a branch: `git checkout -b feature/amazing-feature`
4. **Make** your changes
5. **Test** your changes: `cargo test --all-features`
6. **Commit** your changes: `git commit -m 'Add amazing feature'`
7. **Push** to branch: `git push origin feature/amazing-feature`
8. **Create** a Pull Request

### Code Style

- Follow Rust standard coding conventions
- Run `cargo fmt` and `cargo clippy` before committing
- Write comprehensive tests
- Update documentation

</details>

---

## 📄 License

<div align="center">

This project is licensed under dual license:

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE-MIT)
[![License: Apache 2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](LICENSE-APACHE)

You may choose either license for your use.

</div>

---

## 🙏 Acknowledgments

<div align="center">

### Built With Amazing Tools

</div>

<table>
<tr>
<td align="center" width="25%">
<a href="https://www.rust-lang.org/">
<img src="https://www.rust-lang.org/static/images/rust-logo-blk.svg" width="64" height="64"><br>
<b>Rust</b>
</a>
</td>
<td align="center" width="25%">
<a href="https://github.com/">
<img src="https://github.githubassets.com/images/modules/logos_page/GitHub-Mark.png" width="64" height="64"><br>
<b>GitHub</b>
</a>
</td>
<td align="center" width="25%">
<img src="https://img.icons8.com/fluency/96/000000/code.png" width="64" height="64"><br>
<b>Open Source</b>
</a>
</td>
<td align="center" width="25%">
<img src="https://img.icons8.com/fluency/96/000000/community.png" width="64" height="64"><br>
<b>Community</b>
</a>
</td>
</tr>
</table>

### Special Thanks

- 🌟 **Dependencies** - Built on these amazing projects:
  - [tokio](https://github.com/tokio-rs/tokio) - Async runtime
  - [axum](https://github.com/tokio-rs/axum) - HTTP framework
  - [tonic](https://github.com/hyperium/tonic) - gRPC framework
  - [sea-orm](https://github.com/SeaQL/sea-orm) - Database ORM
  - [etcd-client](https://github.com/etcd-rs/etcd-client) - Etcd client (optional, `etcd` feature)
  - [uuid](https://github.com/uuid-rs/uuid) - UUID generation
  - [confers](https://crates.io/crates/confers) - Configuration management
  - [oxcache](https://crates.io/crates/oxcache) - Multi-level cache
  - [dbnexus](https://crates.io/crates/dbnexus) - Database abstraction
  - [limiteron](https://crates.io/crates/limiteron) - Rate limiting
  - [sdforge](https://crates.io/crates/sdforge) - Service discovery
  - [prometheus-client](https://github.com/prometheus/client_rust) - Metrics

- 👥 **Contributors** - Thanks to all our amazing contributors!

---

## 📞 Contact & Support

<div align="center">

<table>
<tr>
<td align="center" width="50%">
<a href="https://github.com/nebula-id/nebula-id/issues">
<img src="https://img.icons8.com/fluency/96/000000/bug.png" width="48" height="48"><br>
<b>Issues</b>
</a><br>
Report bugs & issues
</td>
<td align="center" width="50%">
<a href="https://github.com/nebula-id/nebula-id/discussions">
<img src="https://img.icons8.com/fluency/96/000000/chat.png" width="48" height="48"><br>
<b>Discussions</b>
</a><br>
Ask questions & share ideas
</td>
</tr>
</table>

### Stay Connected

[![GitHub](https://img.shields.io/badge/GitHub-Follow-181717?style=for-the-badge&logo=github&logoColor=white)](https://github.com/nebula-id)
[![Crates.io](https://img.shields.io/badge/Crates.io-Version-DF5500?style=for-the-badge&logo=rust&logoColor=white)](https://crates.io/crates/nebula-id)

</div>

---

## ⭐ Star History

<div align="center">

[![Star History Chart](https://api.star-history.com/svg?repos=nebula-id/nebula-id&type=Date)](https://star-history.com/#nebula-id/nebula-id&Date)

</div>

---

<div align="center">

### 💝 Support This Project

If you find this project useful, please consider giving it a ⭐️!

**Built with ❤️ by the Nebula ID Team**

[⬆ Back to Top](#-nebula-id)

---

<sub>© 2025 Nebula ID. All rights reserved.</sub>
