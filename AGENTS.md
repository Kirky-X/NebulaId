# AGENTS.md

This file provides guidance for AI agents working in the Nebula ID repository.

## Project Overview

Nebula ID is an enterprise-grade distributed ID generation system written in Rust, supporting multiple ID generation algorithms (Segment, Snowflake, UUID v7/v4) with high availability, distributed coordination, and monitoring capabilities.

## Build Commands

```bash
# Development build
cargo build

# Production release build
cargo build --release

# Build specific crate
cargo build -p nebula-core
cargo build -p nebula-server
```

## Lint & Format Commands

```bash
# Check code formatting
cargo fmt --all -- --check

# Format code
cargo fmt --all

# Run Clippy lints
cargo clippy --all -- -D warnings -A clippy::derivable-clones -A clippy::redundant-pub-crate

# Run pre-commit checks (format, clippy, build, tests)
./scripts/pre-commit-check.sh
```

## Test Commands

```bash
# Run all tests with 4 threads
cargo test -- --test-threads=4

# Run all tests including slow tests
cargo test

# Run specific test by name
cargo test test_name
cargo test --package nebula-core test_segment

# Run tests in a specific module
cargo test -p nebula-core --lib algorithm::segment

# Run tests matching a pattern
cargo test -- segment_
cargo test -- degradation_

# Run integration tests
cargo test --test integration

# Skip slow tests
cargo test -- --skip slow

# Run with output capture disabled
cargo test -- --nocapture

# Test coverage
cargo tarpaulin --out Html
```

## CI/CD Workflows

GitHub Actions workflows are located in `.github/workflows/`:

| Workflow | Trigger | Purpose |
|----------|---------|---------|
| `ci.yml` | Push/PR | Main CI: lint, build, test, security audit |
| `code-review.yml` | Push/PR | AI-powered code review |
| `release.yml` | Tag `v*` | Multi-platform release builds |

### CI Pipeline (ci.yml)

```bash
# Jobs run in sequence: quick-check → build → test → security-audit
```

**Quick Check Job:**
- Format validation: `cargo fmt --all -- --check`
- Clippy lint (lib + bins only): `cargo clippy --lib --bins`

**Build Job:**
- Compile all workspace crates: `cargo build --workspace`

**Test Job:**
- Run all tests: `cargo test --workspace`
- Generate coverage for PRs (codecov)

**Security Audit Job:**
- Vulnerability scanning: `cargo deny check security`

### Pre-commit Check Script

Use the local CI script before committing:

```bash
./scripts/pre-commit-check.sh
```

This runs the same checks as the CI pipeline locally.

## Code Style Guidelines

### Imports

```rust
// 1. Standard library imports first
use std::sync::{Arc, Mutex};
use std::time::Duration;

// 2. External crate imports (alphabetically)
use async_trait::async_trait;
use dashmap::DashMap;
use tokio::sync::mpsc;

// 3. Local crate imports (use crate::)
use crate::algorithm::{IdAlgorithm, HealthStatus};
use crate::config::Config;
use crate::types::{CoreError, Result};

// Group by crate, then alphabetically within groups
```

### Formatting

- Use `rustfmt` default settings (4 spaces, snake_case file names)
- Keep lines under 120 characters
- Use blank lines to group related code (2-3 lines max)
- Place `#[cfg(test)]` modules at the end of files
- Add license header to new files:

```rust
// Copyright © 2026 Kirky.X
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
```

### Naming Conventions

| Item | Convention | Example |
|------|------------|---------|
| Modules | `snake_case` | `segment`, `cache` |
| Structs | `PascalCase` | `SegmentAlgorithm`, `CpuMonitor` |
| Enums | `PascalCase` | `CoreError`, `HealthStatus` |
| Variants | `PascalCase` | `ClockMovedBackward` |
| Functions | `snake_case` | `generate_id()`, `start_monitoring()` |
| Variables | `snake_case` | `current_usage`, `max_id` |
| Constants | `SCREAMING_SNAKE_CASE` | `DEFAULT_CPU_USAGE` |
| Type parameters | `PascalCase` | `T`, `Result<T, E>` |
| Config fields | `snake_case` | `max_connections`, `api_key` |

### Visibility Rules

```rust
// Public API (re-exported in lib.rs)
pub mod algorithm;
pub use algorithm::IdAlgorithm;

// Crate-internal only
pub(crate) mod config_management;
pub(crate) const INTERNAL_CONST: u64 = 1000;

// Module-private (default)
struct PrivateStruct;
fn helper_function() {}
```

### Error Handling

Use `thiserror` for enum errors with `#[derive_more::Display]`:

```rust
use derive_more::Display;
use thiserror::Error;

#[derive(Debug, Error, Display)]
pub enum CoreError {
    #[display("Database error: {}", _0)]
    DatabaseError(String),

    #[display("Segment exhausted, max_id: {}", max_id)]
    SegmentExhausted { max_id: u64 },

    #[display("Invalid ID format: {}", _0)]
    InvalidIdFormat(String),
}

// Use ? operator and From implementations
impl From<std::io::Error> for CoreError {
    fn from(e: std::io::Error) -> Self {
        CoreError::DatabaseError(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, CoreError>;
```

### Async Patterns

- Use `tokio` as the async runtime
- Prefer `async/.await` over blocking operations
- Use `Arc<Mutex<T>>` or `dashmap::DashMap` for shared state
- Use `tokio::sync` primitives (`mpsc`, `oneshot`, `watch`)
- Use `parking_lot` for faster mutex operations

### Documentation

Document public APIs with doc comments:

```rust
/// Generates unique IDs using the Segment algorithm.
///
/// This algorithm provides high-throughput, ordered ID generation
/// with database-backed segment allocation.
///
/// # Errors
///
/// Returns `CoreError::SegmentExhausted` when the current segment
/// is fully utilized and database refresh fails.
pub async fn generate_id(&self, ctx: &GenerateContext) -> Result<Id>;
```

### Module Organization

```
crates/core/src/
├── lib.rs              # Public API exports
├── algorithm/          # ID generation algorithms
│   ├── mod.rs
│   ├── segment.rs
│   ├── snowflake.rs
│   ├── traits.rs       # IdAlgorithm trait
│   └── router.rs
├── cache/              # Caching layer
├── config/             # Configuration
├── database/           # SeaORM entities & repositories
└── coordinator/        # Distributed coordination (etcd)
```

### Testing

- Place unit tests in `tests/` module at the end of source files
- Place integration tests in `crates/*/tests/` directory
- Use `#[cfg(test)]` to mark test modules
- Follow naming convention: `test_*` or `should_*`

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_segment_generation() {
        let segment = SegmentAlgorithm::new();
        let id = segment.generate_id(&ctx).await;
        assert!(id.is_ok());
    }
}
```

## Dependencies

All dependencies are defined in workspace `Cargo.toml` under `[workspace.dependencies]`. When adding new dependencies:

1. Add version to `[workspace.dependencies]`
2. Use the workspace dependency in crate `Cargo.toml`:

```toml
[dependencies]
serde = { workspace = true }
tokio = { workspace = true, features = ["full"] }
```

## Database Migrations

```bash
# Run migrations
cargo run --bin migrate -- crates/server

# Or via Makefile
make db-migrate
```

## Debugging

```bash
# Enable debug logging
RUST_LOG=debug cargo run

# Module-specific logging
RUST_LOG=nebula_core::algorithm=debug cargo run
```
