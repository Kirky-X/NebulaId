// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Nebula ID - Enterprise-grade distributed ID generation system
//!
//! This crate provides a unified API for distributed ID generation with support for
//! multiple algorithms (Segment, Snowflake, UUID v8 — `uuid_v7` / `uuid_v4` are
//! accepted only as input aliases of `AlgorithmType::from_str`) and features like
//! distributed coordination, caching, and monitoring.
//!
//! # Architecture
//!
//! - [`core`] - Core business logic for ID generation algorithms
//! - [`server`] - HTTP/gRPC server implementations
//!
//! # Usage
//!
//! ```rust
//! use nebulaid::core::Config;
//! ```

// ============================================================================
// t! — 进程级全局翻译宏（unify-rust-i18n 统一基线，替换 rust-i18n 的 t!）
// ============================================================================
//
// 定义在 crate 根且必须位于 `pub mod core;` / `pub mod server;` 之前,经宏的
// 文本作用域覆盖全 crate 调用点(等价于原先 `#[macro_use] extern crate
// rust_i18n;` 的作用域效果);`#[macro_export]` 同时把宏挂到 crate 根,
// bin 侧(src/main.rs)经 `#[macro_use] extern crate nebulaid;` 引入。
//
// 运行时语义:进程默认 locale(经 `core::i18n::init_i18n` 设置)→ en 束 →
// 键本身;参数按 Display 格式化为字符串后交给 Fluent 变量解析。
#[macro_export]
macro_rules! t {
    ($key:expr $(,)?) => {
        $crate::core::i18n::global_translate($key, &[])
    };
    ($key:expr, $($name:ident = $value:expr),+ $(,)?) => {
        $crate::core::i18n::global_translate(
            $key,
            &[$((::std::stringify!($name), $value.to_string())),+],
        )
    };
}

// Core namespace - 核心业务逻辑
pub mod core;

// Server namespace - HTTP/gRPC 服务
pub mod server;

// SDK namespace - 嵌入式一等公民入口（feature `sdk` 门控）
#[cfg(feature = "sdk")]
pub mod sdk;
