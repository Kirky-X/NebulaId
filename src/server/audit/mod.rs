// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Audit module for logging and middleware.

pub mod logger;
pub mod middleware;

// Re-exports
pub use logger::{AuditEvent, AuditEventType, AuditLogger, AuditResult};
pub use middleware::{audit_middleware_fn, AuditMiddleware};
