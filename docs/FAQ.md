<div align="center">

# ❓ Frequently Asked Questions (FAQ)

### Quick Answers to Common Questions about Nebula ID

[🏠 Home](../README.md) • [📖 User Guide](USER_GUIDE.md) • [🔧 API Reference](API_REFERENCE.md)

---

</div>

## 📋 Table of Contents

- [General Questions](#general-questions)
- [Installation & Setup](#installation--setup)
- [Usage & Features](#usage--features)
- [Performance](#performance)
- [Security](#security)
- [Troubleshooting](#troubleshooting)
- [Contributing](#contributing)
- [Licensing](#licensing)

---

## General Questions

<div align="center">

### 🤔 About the Project

</div>

<details>
<summary><b>❓ What is Nebula ID?</b></summary>

<br>

**Nebula ID** is an enterprise-grade distributed ID generation system for high-performance applications. It provides:

- ✅ **Multiple ID Algorithms** - Segment, Snowflake, UUID v7, UUID v4
- ✅ **Distributed Coordination** - Etcd-based leader election and coordination
- ✅ **High Availability** - Datacenter health monitoring and automatic failover
- ✅ **Type-Safe Design** - Full Rust type safety with async/await patterns

It's designed for **distributed systems** that require unique, ordered, and high-throughput identifier generation.

**Learn more:** [User Guide](USER_GUIDE.md)

</details>

<details>
<summary><b>❓ Why should I use this instead of alternatives?</b></summary>

<br>

<table>
<tr>
<th>Feature</th>
<th>Nebula ID</th>
<th>Snowflake</th>
<th>UUID</th>
</tr>
<tr>
<td>Time Ordering</td>
<td>✅ Yes</td>
<td>✅ Yes</td>
<td>⚠️ v7 only</td>
</tr>
<tr>
<td>High Throughput</td>
<td>✅ 1M+ IDs/sec</td>
<td>✅ 1M+ IDs/sec</td>
<td>✅ 1M+ IDs/sec</td>
</tr>
<tr>
<td>No Clock Sync</td>
<td>✅ Segment</td>
<td>❌ No</td>
<td>✅ Yes</td>
</tr>
<tr>
<td>Fault Tolerance</td>
<td>✅ Built-in</td>
<td>⚠️ Manual</td>
<td>✅ Yes</td>
</tr>
</table>

**Key Advantages:**
- 🚀 **Multiple Algorithms**: Choose Segment for database-backed ordering, Snowflake for speed, or UUID for simplicity
- 🔄 **Automatic Failover**: Datacenter health monitoring with automatic recovery
- 🛡️ **Enterprise Ready**: API authentication, rate limiting, and audit logging
- 📊 **Built-in Monitoring**: Health checks and metrics collection

</details>

<details>
<summary><b>❓ Is this production-ready?</b></summary>

<br>

**Current Status:** ✅ **Production-ready!**

<table>
<tr>
<td width="50%">

**What's Ready:**
- ✅ Core ID generation algorithms (Segment, Snowflake, UUID v7/v4)
- ✅ Distributed coordination with Etcd
- ✅ Datacenter health monitoring and failover
- ✅ HTTP/HTTPS and gRPC/gRPCS APIs
- ✅ API key authentication and rate limiting

</td>
<td width="50%">

**Maturity Indicators:**
- 📊 85%+ test coverage
- 🔄 Regular maintenance
- 🛡️ Security-focused design
- 📖 Comprehensive documentation

</td>
</tr>
</table>

> **Note:** Always review the [CHANGELOG](../CHANGELOG.md) before upgrading versions.

</details>

<details>
<summary><b>❓ What platforms are supported?</b></summary>

<br>

<table>
<tr>
<th>Platform</th>
<th>Architecture</th>
<th>Status</th>
<th>Notes</th>
</tr>
<tr>
<td rowspan="2"><b>Linux</b></td>
<td>x86_64</td>
<td>✅ Fully Supported</td>
<td>Primary platform</td>
</tr>
<tr>
<td>ARM64</td>
<td>✅ Fully Supported</td>
<td>Tested on ARM servers</td>
</tr>
<tr>
<td rowspan="2"><b>macOS</b></td>
<td>x86_64</td>
<td>✅ Fully Supported</td>
<td>Intel Macs</td>
</tr>
<tr>
<td>ARM64</td>
<td>✅ Fully Supported</td>
<td>Apple Silicon (M1/M2/M3)</td>
</tr>
<tr>
<td><b>Windows</b></td>
<td>x86_64</td>
<td>✅ Fully Supported</td>
<td>Windows 10+</td>
</tr>
</table>

</details>

<details>
<summary><b>❓ What programming languages are supported?</b></summary>

<br>

**Nebula ID** is a native **Rust** library with multi-protocol service support:

- **Rust**: Native library (`nebula-id` crate)
- **HTTP/REST**: Any language with HTTP client
- **gRPC**: Any language with gRPC support (Python, Java, Go, etc.)

**Documentation:**
- [Rust API Docs](https://docs.rs/nebula-id)
- [API Reference](API_REFERENCE.md)

</details>

<details>
<summary><b>❓ What ID algorithms are supported?</b></summary>

<br>

<table>
<tr>
<th>Algorithm</th>
<th>Format</th>
<th>Time Ordered</th>
<th>Throughput</th>
<th>Best For</th>
</tr>
<tr>
<td>Segment</td>
<td>64-bit</td>
<td>✅ Yes</td>
<td>100K+/sec</td>
<td>Database primary keys</td>
</tr>
<tr>
<td>Snowflake</td>
<td>64-bit</td>
<td>✅ Yes</td>
<td>1M+/sec</td>
<td>High-performance systems</td>
</tr>
<tr>
<td>UUID v7</td>
<td>128-bit</td>
<td>✅ Yes</td>
<td>500K+/sec</td>
<td>Distributed systems</td>
</tr>
<tr>
<td>UUID v4</td>
<td>128-bit</td>
<td>❌ No</td>
<td>1M+/sec</td>
<td>Unique identifiers</td>
</tr>
</table>

</details>

---

## Installation & Setup

<div align="center">

### 🚀 Getting Started

</div>

<details>
<summary><b>❓ How do I install this?</b></summary>

<br>

**For Rust Projects:**

Add the following to your `Cargo.toml`:

```toml
[dependencies]
nebula-id = "0.1.0"
tokio = { version = "1.0", features = ["full"] }
uuid = { version = "1.0", features = ["v7"] }
```

Or using cargo:

```bash
cargo add nebula-id tokio uuid
```

**Optional Features:**

```toml
nebula-id = { version = "0.1.0", features = ["monitoring", "audit", "grpc"] }
```

**Verification:**

```rust
use nebula_id::algorithm::SegmentAlgorithm;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let segment = SegmentAlgorithm::new(1);
    let id = segment.generate_id().await?;
    println!("✅ Generated ID: {}", id.to_u128());
    Ok(())
}
```

**See also:** [User Guide](USER_GUIDE.md#installation)

</details>

<details>
<summary><b>❓ What are the system requirements?</b></summary>

<br>

**Minimum Requirements:**

<table>
<tr>
<th>Component</th>
<th>Requirement</th>
<th>Recommended</th>
</tr>
<tr>
<td>Rust Version</td>
<td>1.75+</td>
<td>Latest stable</td>
</tr>
<tr>
<td>Memory</td>
<td>256MB</td>
<td>1GB+</td>
</tr>
<tr>
<td>Disk Space</td>
<td>50MB</td>
<td>100MB+</td>
</tr>
<tr>
<td>Database</td>
<td>PostgreSQL/MySQL</td>
<td>PostgreSQL 13+</td>
</tr>
</table>

**Optional Dependencies:**
- 🔧 **Etcd**: For distributed coordination (v3.4+)
- ☁️ **Redis**: For caching (v6+)
- 📊 **Prometheus**: For metrics visualization

</details>

<details>
<summary><b>❓ I'm getting compilation errors, what should I do?</b></summary>

<br>

**Common Solutions:**

1. **Check Rust version:**
   ```bash
   rustc --version
   # Should be 1.75.0 or higher
   ```

2. **Ensure required features are enabled:**
   ```toml
   nebula-id = "0.1.0"
   ```

3. **Clean build artifacts:**
   ```bash
   cargo clean
   cargo build
   ```

**Still having issues?**
- 📝 Check [Troubleshooting](#troubleshooting)
- 🐛 [Open an issue](../../issues) with error details

</details>

<details>
<summary><b>❓ Can I use this with Docker?</b></summary>

<br>

**Yes!** Nebula ID works perfectly in containerized environments.

**Sample Dockerfile:**

```dockerfile
FROM rust:1.75 as builder
WORKDIR /app
COPY . .
RUN cargo build --release

FROM debian:bookworm-slim
COPY --from=builder /app/target/release/nebula-id /usr/local/bin/
CMD ["nebula-id"]
```

**Docker Compose with Dependencies:**

```yaml
services:
  nebula-id:
    image: nebula-id:latest
    ports:
      - "8080:8080"
    depends_on:
      - postgres
      - etcd
    environment:
      - DATABASE_URL=postgresql://user:pass@postgres/nebula
      - ETCD_ENDPOINTS=http://etcd:2379

  postgres:
    image: postgres:15
    environment:
      POSTGRES_DB: nebula

  etcd:
    image: etcd:v3.5
```

</details>

<details>
<summary><b>❓ How do I configure Nebula ID?</b></summary>

<br>

**Configuration File (config.toml):**

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
```

**Environment Variables:**

```bash
export NEBULA_DATABASE_URL="postgresql://user:pass@localhost/nebula"
export NEBULA_ETCD_ENDPOINTS="http://localhost:2379"
export NEBULA_AUTH_API_KEY="your-api-key-here"
```

**See also:** [Configuration Guide](USER_GUIDE.md#configuration)

</details>

---

## Usage & Features

<div align="center">

### 💡 Working with the API

</div>

<details>
<summary><b>❓ How do I get started with basic usage?</b></summary>

<br>

**5-Minute Quick Start:**

```rust
use nebula_id::algorithm::SegmentAlgorithm;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create a segment algorithm instance with datacenter ID
    let segment = SegmentAlgorithm::new(1);
    
    // Generate a single ID
    let id = segment.generate_id().await?;
    println!("Generated ID: {}", id.to_u128());
    
    // Generate a batch of IDs
    let batch = segment.generate_batch(100).await?;
    println!("Generated {} IDs", batch.len());
    
    Ok(())
}
```

**Next Steps:**
- 📖 [User Guide](USER_GUIDE.md)
- 💻 [Examples](../examples/)

</details>

<details>
<summary><b>❓ How do I choose the right algorithm?</b></summary>

<br>

**Algorithm Selection Guide:**

| Use Case | Recommended Algorithm | Reason |
|----------|----------------------|--------|
| Database primary keys | Segment | Ordered, database-backed, reliable |
| High-throughput microservices | Snowflake | Fast, no database dependency |
| Time-ordered distributed IDs | UUID v7 | Standard, time-sortable |
| General unique identifiers | UUID v4 | Simple, collision-resistant |
| Mixed requirements | Multi-algorithm | Use different algorithms per use case |

**Configuration Example:**

```rust
use nebula_id::algorithm::{SegmentAlgorithm, SnowflakeAlgorithm, UuidV7Impl};

// For database primary keys
let segment = SegmentAlgorithm::new(1);

// For high-performance services
let snowflake = SnowflakeAlgorithm::new(1, 1);

// For time-ordered UUIDs
let uuid_v7 = UuidV7Impl::new();
```

</details>

<details>
<summary><b>❓ How does the Segment algorithm work?</b></summary>

<br>

The Segment algorithm pre-allocates ID ranges from the database for efficient batch generation:

```
┌─────────────────────────────────────────────────────────────┐
│                    Segment Algorithm                         │
├─────────────────────────────────────────────────────────────┤
│  1. Request ID range from database                          │
│  2. Pre-allocate range (e.g., 1-10000)                      │
│  3. Generate IDs from local cache                           │
│  4. When approaching limit, pre-fetch next range            │
└─────────────────────────────────────────────────────────────┘
```

**Key Benefits:**
- 🚀 **High Throughput**: Generate IDs from local memory
- 📦 **Batch Efficiency**: Pre-allocation reduces database round-trips
- 🔄 **Fault Tolerance**: Automatic failover to healthy datacenters

**Code Example:**

```rust
use nebula_id::algorithm::SegmentAlgorithm;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let segment = SegmentAlgorithm::new(1);
    
    // Generate single ID (from pre-allocated segment)
    let id = segment.generate_id().await?;
    
    // Generate batch (optimized for throughput)
    let batch = segment.generate_batch(1000).await?;
    
    Ok(())
}
```

</details>

<details>
<summary><b>❓ How does the Snowflake algorithm work?</b></summary>

<br>

The Snowflake algorithm generates 64-bit IDs with configurable bit allocation:

```
┌────────────────────────────────────────────────────────────────┐
│                    Snowflake ID Structure                       │
├────────────────────────────────────────────────────────────────┤
│  1 bit   │  41 bits    │  5 bits  │  5 bits  │  12 bits      │
│  (sign)  │  timestamp  │  datacenter │  worker │  sequence    │
└────────────────────────────────────────────────────────────────┘
```

**Key Benefits:**
- 🚀 **Fast**: No database dependency
- 📈 **Scalable**: Supports 32 datacenters × 32 workers
- 🎯 **Ordered**: Time-based ordering within millisecond

**Code Example:**

```rust
use nebula_id::algorithm::SnowflakeAlgorithm;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let snowflake = SnowflakeAlgorithm::new(1, 1); // datacenter, worker
    
    let id = snowflake.generate_id()?;
    println!("Snowflake ID: {}", id.to_u128());
    
    // Access components
    println!("Datacenter: {}", snowflake.get_datacenter_id());
    println!("Worker: {}", snowflake.get_worker_id());
    
    Ok(())
}
```

</details>

<details>
<summary><b>❓ What is UUID v7 and when should I use it?</b></summary>

<br>

UUID v7 is a time-ordered UUID format defined by RFC 9562:

```
┌────────────────────────────────────────────────────────────────┐
│                    UUID v7 Structure                            │
├────────────────────────────────────────────────────────────────┤
│  48 bits  │  4 bits  │  3 bits  │  13 bits  │  62 bits        │
│  timestamp│ version  │  variant │  clock-seq│  node ID        │
└────────────────────────────────────────────────────────────────┘
```

**Benefits:**
- ✅ **Time-Ordered**: Lexicographically sortable by creation time
- ✅ **Standard**: RFC 9562 compliant
- 🔄 **Compatible**: Works with existing UUID infrastructure

**Code Example:**

```rust
use nebula_id::algorithm::UuidV7Impl;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let v7 = UuidV7Impl::new();
    
    let uuid = UuidV7Impl::generate()?;
    println!("UUID v7: {}", uuid);
    
    // Convert to Nebula ID
    let nebula_id = nebula_id::types::Id::from_uuid_v7(uuid);
    
    Ok(())
}
```

**Use When:**
- You need standard-compliant identifiers
- Time-based sorting is important
- You want UUID compatibility

</details>

<details>
<summary><b>❓ How does distributed coordination work?</b></summary>

<br>

Nebula ID uses etcd for distributed coordination:

```
┌─────────────────────────────────────────────────────────────┐
│              Distributed Coordination                         │
├─────────────────────────────────────────────────────────────┤
│  1. Leader Election (etcd)                                  │
│  2. Datacenter Health Monitoring                            │
│  3. Automatic Failover                                      │
│  4. Segment Range Locking                                   │
└─────────────────────────────────────────────────────────────┘
```

**Components:**

1. **EtcdClusterHealthMonitor**: Monitors etcd cluster health
2. **DcFailureDetector**: Tracks datacenter health status
3. **Automatic Failover**: Routes traffic to healthy datacenters

**Code Example:**

```rust
use nebula_id::algorithm::segment::{SegmentAlgorithm, DcFailureDetector};
use nebula_id::coordinator::EtcdClusterHealthMonitor;
use nebula_id::config::EtcdConfig;
use std::sync::Arc;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create failure detector
    let dc_failure_detector = Arc::new(DcFailureDetector::new(
        5,                              // failure_threshold
        Duration::from_secs(300),       // recovery_timeout
    ));
    dc_failure_detector.add_dc(1);
    
    // Create health monitor
    let etcd_config = EtcdConfig::default();
    let health_monitor = Arc::new(EtcdClusterHealthMonitor::new(
        etcd_config,
        "./cache.json".to_string(),
    ));
    
    // Create algorithm with coordination
    let segment = SegmentAlgorithm::new(1);
    
    Ok(())
}
```

</details>

<details>
<summary><b>❓ How do I handle errors properly?</b></summary>

<br>

**Recommended Pattern:**

```rust
use nebula_id::error::CoreError;

#[tokio::main]
async fn main() {
    match run().await {
        Ok(id) => println!("Generated ID: {}", id.to_u128()),
        Err(e) => match e {
            CoreError::ClockMovedBackward { .. } => {
                eprintln!("❌ System clock issue detected");
            }
            CoreError::DatabaseConnectionFailed { .. } => {
                eprintln!("❌ Database connection failed");
            }
            CoreError::SegmentExhausted { .. } => {
                eprintln!("❌ ID segment exhausted, refreshing...");
            }
            CoreError::EtcdConnectionFailed { .. } => {
                eprintln!("❌ Etcd connection failed, using cache");
            }
            _ => eprintln!("❌ Error: {}", e),
        },
    }
}
```

**Error Types:**

| Error | Description | Recovery |
|-------|-------------|----------|
| `ClockMovedBackward` | System clock regression | NTP sync required |
| `DatabaseConnectionFailed` | Database unavailable | Check connection, use cache |
| `SegmentExhausted` | ID range depleted | Auto-refresh segment |
| `EtcdConnectionFailed` | Etcd unavailable | Use local cache |
| `SequenceOverflow` | Snowflake sequence overflow | Wait for next ms |

</details>

<details>
<summary><b>❓ Is there async/await support?</b></summary>

<br>

**Yes!** Nebula ID is designed for async/await from the ground up.

```rust
use nebula_id::algorithm::SegmentAlgorithm;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let segment = SegmentAlgorithm::new(1);
    
    // Async ID generation
    let id = segment.generate_id().await?;
    
    // Async batch generation
    let batch = segment.generate_batch(100).await?;
    
    Ok(())
}
```

**Supported Runtimes:**
- ✅ Tokio (recommended)
- ✅ Async-Std
- ✅ smol

</details>

---

## Performance

<div align="center">

### ⚡ Speed and Optimization

</div>

<details>
<summary><b>❓ How fast is it?</b></summary>

<br>

**Benchmark Results:**

<table>
<tr>
<th>Algorithm</th>
<th>Throughput</th>
<th>P50 Latency</th>
<th>P99 Latency</th>
</tr>
<tr>
<td>Segment</td>
<td>100,000+ IDs/sec</td>
<td>~0.1ms</td>
<td>~0.5ms</td>
</tr>
<tr>
<td>Snowflake</td>
<td>1,000,000+ IDs/sec</td>
<td>~0.05ms</td>
<td>~0.1ms</td>
</tr>
<tr>
<td>UUID v7</td>
<td>500,000+ IDs/sec</td>
<td>~0.03ms</td>
<td>~0.05ms</td>
</tr>
<tr>
<td>UUID v4</td>
<td>1,000,000+ IDs/sec</td>
<td>~0.03ms</td>
<td>~0.05ms</td>
</tr>
</table>

**Run benchmarks yourself:**

```bash
cargo bench
```

</details>

<details>
<summary><b>❓ How can I improve performance?</b></summary>

<br>

**Optimization Tips:**

1. **Enable Release Mode:**
   ```bash
   cargo build --release
   ```

2. **Use Batch Generation:**
   ```rust
   // Instead of generating IDs one by one
   let batch = segment.generate_batch(1000).await?;
   ```

3. **Configure Appropriate Segment Size:**
   ```toml
   [algorithm.segment]
   step = 10000  # Larger step = fewer database round-trips
   ```

4. **Use Snowflake for Speed:**
   - No database dependency
   - In-memory generation
   - ~1M IDs/sec per instance

5. **Enable Connection Pooling:**
   ```toml
   [database]
   max_connections = 20
   ```

</details>

<details>
<summary><b>❓ What's the memory usage like?</b></summary>

<br>

**Typical Memory Usage:**

<table>
<tr>
<th>Component</th>
<th>Memory</th>
</tr>
<tr>
<td>Core Library</td>
<td>~1MB</td>
</tr>
<tr>
<td>Segment Cache (1M IDs)</td>
<td>~8MB</td>
</tr>
<tr>
<td>Etcd Client</td>
<td>~2MB</td>
</tr>
<tr>
<td>HTTP Server</td>
<td>~5MB</td>
</tr>
</table>

**Total:** ~16MB base + algorithm-specific overhead

**Memory Safety:**
- ✅ No memory leaks (verified with continuous testing)
- ✅ Efficient batch processing
- ✅ Connection pooling
- ✅ Async runtime efficiency

</details>

<details>
<summary><b>❓ How does the system handle high concurrency?</b></summary>

<br>

Nebula ID is designed for high concurrency:

**Concurrency Features:**
- 🚀 **Async/Await**: Non-blocking operations
- 🔀 **DashMap**: Thread-safe concurrent data structures
- 📊 **Connection Pooling**: Efficient database connections
- ⚡ **Lock-Free**: Minimal contention points

**Best Practices:**

```rust
use nebula_id::algorithm::SnowflakeAlgorithm;
use std::sync::Arc;
use tokio::task;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let snowflake = Arc::new(SnowflakeAlgorithm::new(1, 1));
    
    // Spawn concurrent tasks
    let mut handles = Vec::new();
    for _ in 0..100 {
        let snowflake = snowflake.clone();
        handles.push(task::spawn(async move {
            snowflake.generate_id()
        }));
    }
    
    // Collect results
    let results: Vec<Result<_, _>> = futures::future::join_all(handles).await;
    
    Ok(())
}
```

</details>

---

## Security

<div align="center">

### 🔒 Security Features

</div>

<details>
<summary><b>❓ What security features are included?</b></summary>

<br>

**Yes!** Security is a core focus of Nebula ID.

**Security Features:**

<table>
<tr>
<td width="50%">

**Authentication**
- ✅ API Key authentication
- ✅ Constant-time comparison (timing attack prevention)
- ✅ Token-based access
- ✅ Configurable key rotation

</td>
<td width="50%">

**Protection**
- ✅ Rate limiting (max batch size: 100)
- ✅ Request validation
- ✅ Audit logging with IP spoofing protection
- ✅ CORS restrictions
- ✅ Security headers

</td>
</tr>
</table>

**Encryption:**
- ✅ TLS/HTTPS support (TLS 1.2/1.3)
- ✅ gRPCS support
- ✅ Secure communication

**Security Headers:**
- X-Content-Type-Options: nosniff
- X-Frame-Options: DENY
- Content-Security-Policy: default-src 'self'
- Strict-Transport-Security: max-age=31536000; includeSubDomains
- X-XSS-Protection: 1; mode=block
- Referrer-Policy: strict-origin-when-cross-origin

**More details:** [Security Guide](USER_GUIDE.md#security)

</details>

<details>
<summary><b>❓ How do I configure API authentication?</b></summary>

<br>

**Configuration:**

```toml
[auth]
api_key = "your-secure-api-key-here"
token_expiry_hours = 24

[rate_limit]
requests_per_second = 1000
burst_size = 100
max_batch_size = 100  # Maximum batch size to prevent DoS attacks
```

**Usage:**

```rust
use nebula_id::server::NebulaIdServer;
use nebula_id::Config;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::load_from_file("config.toml")?;
    
    let server = NebulaIdServer::new(config);
    
    // Server validates API key on each request
    server.start().await?;
    
    Ok(())
}
```

**HTTP Header:**

```
Authorization: Bearer your-api-key-here
```

</details>

<details>
<summary><b>❓ How do I report security vulnerabilities?</b></summary>

<br>

**Please report security issues responsibly:**

1. **DO NOT** create public GitHub issues
2. **Email:** security@nebula-id.io
3. **Include:**
   - Description of the vulnerability
   - Steps to reproduce
   - Potential impact

**Response Timeline:**
- 📧 Initial response: 24 hours
- 🔍 Assessment: 72 hours
- 📢 Public disclosure: After fix is released

</details>

<details>
<summary><b>❓ What about rate limiting?</b></summary>

<br>

Nebula ID includes built-in rate limiting:

**Configuration:**

```toml
[rate_limit]
requests_per_second = 1000
burst_size = 100
```

**Rate Limits by Plan:**

| Plan | Requests/Second | Burst |
|------|-----------------|-------|
| Free | 100 | 10 |
| Pro | 1,000 | 100 |
| Enterprise | 10,000 | 1,000 |

**Response Headers:**

```
X-RateLimit-Limit: 1000
X-RateLimit-Remaining: 999
X-RateLimit-Reset: 1640995200
```

</details>

---

## Troubleshooting

<div align="center">

### 🔧 Common Issues

</div>

<details>
<summary><b>❓ I'm getting "ClockMovedBackward" error</b></summary>

<br>

**Problem:**
```
Error: system clock moved backward
```

**Cause:** System clock regression detected, which could cause duplicate IDs.

**Solution:**
1. **Sync system time:**
   ```bash
   # Linux
   sudo ntpdate pool.ntp.org
   
   # macOS
   sudo sntp -sS pool.ntp.org
   ```

2. **Configure NTP auto-sync:**
   ```bash
   # Add to /etc/chrony.conf
   server pool.ntp.org iburst
   ```

3. **For virtualized environments:**
   - Ensure host clock is synced
   - Use VMware Tools time synchronization
   - Configure Hyper-V time synchronization

**Prevention:**
- Use NTP daemon (chronyd, ntpd)
- Monitor clock drift
- Alert on significant drift

</details>

<details>
<summary><b>❓ I'm getting "DatabaseConnectionFailed" error</b></summary>

<br>

**Problem:**
```
Error: failed to connect to database
```

**Cause:** Database connection issues.

**Solution:**
1. **Verify database is running:**
   ```bash
   # PostgreSQL
   pg_isready -h localhost -p 5432
   
   # MySQL
   mysqladmin ping -h localhost
   ```

2. **Check connection string:**
   ```toml
   [database]
   url = "postgresql://user:pass@localhost/nebula"
   ```

3. **Test network connectivity:**
   ```bash
   telnet localhost 5432
   ```

4. **Check credentials:**
   ```bash
   psql -U user -d nebula
   ```

5. **Enable local cache fallback:**
   ```rust
   let health_monitor = EtcdClusterHealthMonitor::new(config, "./cache.json");
   ```

</details>

<details>
<summary><b>❓ IDs are not time-ordered</b></summary>

<br>

**Problem:**
Generated IDs are not monotonically increasing.

**Cause:** Multiple instances generating IDs simultaneously.

**Solution:**

1. **For Snowflake:** Ensure clock is synchronized across instances

2. **For Segment:** Verify segment refresh logic

3. **Use UUID v7 for time-ordering:**
   ```rust
   use nebula_id::algorithm::UuidV7Impl;
   use uuid::Uuid;
   
   let uuid = Uuid::now_v7();
   ```

**Note:** Snowflake IDs are ordered within the same millisecond per instance.

</details>

<details>
<summary><b>❓ How do I debug ID generation issues?</b></summary>

<br>

**Enable Debug Logging:**

```rust
fn main() {
    tracing_subscriber::fmt::init();
    
    let segment = SegmentAlgorithm::new(1);
    let id = segment.generate_id().unwrap();
}
```

Set environment variable:
```bash
RUST_LOG=nebula_id=debug
```

**Common Debug Commands:**

```bash
# Check etcd health
etcdctl endpoint health

# Check database connections
SELECT count(*) FROM pg_stat_activity;

# Monitor metrics
curl http://localhost:8080/metrics
```

</details>

<details>
<summary><b>❓ Performance is degraded</b></summary>

<br>

**Problem:** ID generation is slower than expected.

**Diagnosis Steps:**

1. **Check database performance:**
   ```sql
   EXPLAIN ANALYZE SELECT * FROM nebula_segments;
   ```

2. **Monitor connection pool:**
   ```bash
   # Check active connections
   SELECT count(*) FROM pg_stat_activity WHERE datname = 'nebula';
   ```

3. **Check etcd latency:**
   ```bash
   etcdctl put test && etcdctl get test --cluster
   ```

**Solutions:**

1. **Increase database connections:**
   ```toml
   [database]
   max_connections = 20
   ```

2. **Increase segment step:**
   ```toml
   [algorithm.segment]
   step = 10000
   ```

3. **Add Redis caching:**
   ```toml
   [redis]
   url = "redis://localhost"
   ```

</details>

**More issues?** Check [Troubleshooting Guide](TROUBLESHOOTING.md)

---

## Contributing

<div align="center">

### 🤝 Join the Community

</div>

<details>
<summary><b>❓ How can I contribute?</b></summary>

<br>

**Ways to Contribute:**

<table>
<tr>
<td width="50%">

**Code Contributions**
- 🐛 Fix bugs
- ✨ Add features
- 📝 Improve documentation
- ✅ Write tests

</td>
<td width="50%">

**Non-Code Contributions**
- 📖 Write tutorials
- 🎨 Design assets
- 🌍 Translate docs
- 💬 Answer questions

</td>
</tr>
</table>

**Getting Started:**

1. 🍴 Fork the repository
2. 🌱 Create a branch: `git checkout -b feature/amazing-feature`
3. ✏️ Make changes
4. ✅ Add tests: `cargo test --all-features`
5. 📤 Submit PR

**Guidelines:** [CONTRIBUTING.md](../CONTRIBUTING.md)

</details>

<details>
<summary><b>❓ I found a bug, what should I do?</b></summary>

<br>

**Before Reporting:**

1. ✅ Check [existing issues](../../issues)
2. ✅ Try the latest version
3. ✅ Check [troubleshooting guide](#troubleshooting)

**Creating a Good Bug Report:**

```markdown
### Description
Clear description of the bug

### Steps to Reproduce
1. Step one
2. Step two
3. See error

### Expected Behavior
What should happen

### Actual Behavior
What actually happens

### Environment
- OS: Ubuntu 22.04
- Rust version: 1.75.0
- Nebula ID version: 0.1.0
- Database: PostgreSQL 15

### Additional Context
Any other relevant information
```

**Submit:** [Create Issue](../../issues/new)

</details>

<details>
<summary><b>❓ Where can I get help?</b></summary>

<br>

<div align="center">

### 💬 Support Channels

</div>

<table>
<tr>
<td width="33%" align="center">

**🐛 Issues**

[GitHub Issues](../../issues)

Bug reports & features

</td>
<td width="33%" align="center">

**💬 Discussions**

[GitHub Discussions](../../discussions)

Q&A and ideas

</td>
<td width="33%" align="center">

**📖 Documentation**

[User Guide](USER_GUIDE.md)

API docs & tutorials

</td>
</tr>
</table>

**Response Times:**
- 🐛 Critical bugs: 24 hours
- 🔧 Feature requests: 1 week
- 💬 Questions: 2-3 days

</details>

---

## Licensing

<div align="center">

### 📄 License Information

</div>

<details>
<summary><b>❓ What license is this under?</b></summary>

<br>

**Dual License:**

<table>
<tr>
<td width="50%" align="center">

**MIT License**

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](../LICENSE-MIT)

**Permissions:**
- ✅ Commercial use
- ✅ Modification
- ✅ Distribution
- ✅ Private use

</td>
<td width="50%" align="center">

**Apache License 2.0**

[![License: Apache 2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](../LICENSE-APACHE)

**Permissions:**
- ✅ Commercial use
- ✅ Modification
- ✅ Distribution
- ✅ Patent grant

</td>
</tr>
</table>

**You can choose either license for your use.**

</details>

<details>
<summary><b>❓ Can I use this in commercial projects?</b></summary>

<br>

**Yes!** Both MIT and Apache 2.0 licenses allow commercial use.

**What you need to do:**
1. ✅ Include the license text
2. ✅ Include copyright notice
3. ✅ State any modifications

**What you DON'T need to do:**
- ❌ Share your source code
- ❌ Open source your project
- ❌ Pay royalties

**Questions?** Contact: legal@nebula-id.io

</details>

---

<div align="center">

### 🎯 Still Have Questions?

<table>
<tr>
<td width="33%" align="center">
<a href="../../issues">
<img src="https://img.icons8.com/fluency/96/000000/bug.png" width="48"><br>
<b>Open an Issue</b>
</a>
</td>
<td width="33%" align="center">
<a href="../../discussions">
<img src="https://img.icons8.com/fluency/96/000000/chat.png" width="48"><br>
<b>Start a Discussion</b>
</a>
</td>
<td width="33%" align="center">
<a href="https://docs.rs/nebula-id">
<img src="https://img.icons8.com/fluency/96/000000/documentation.png" width="48"><br>
<b>Read API Docs</b>
</a>
</td>
</tr>
</table>

---

**[📖 User Guide](USER_GUIDE.md)** • **[🔧 API Reference](API_REFERENCE.md)** • **[🏠 Home](../README.md)**

Made with ❤️ by the Nebula ID Team

[⬆ Back to Top](#-frequently-asked-questions-faq)
