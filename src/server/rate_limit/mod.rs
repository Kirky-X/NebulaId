// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Rate limiting module.

pub mod limiter;
pub mod middleware;

// Re-exports
pub use limiter::{RateLimitResult, RateLimitStatus, RateLimiter};
pub use middleware::RateLimitMiddleware;
