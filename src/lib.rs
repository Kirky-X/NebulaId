// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Nebula ID - Enterprise-grade distributed ID generation system
//!
//! 多算法（Segment/Snowflake/UUID v8/v7/v5/v4）经 `AlgorithmType` 选择，辅以分布式协调、缓存与监控。
//!
//! ```rust
//! use nebulaid::core::Config;
//! ```

// ============================================================================
// t! — 进程级全局翻译宏（unify-rust-i18n 统一基线）
// ============================================================================
//
// 文本作用域要求定义先于使用，故置于 pub mod 声明之前；#[macro_export] 挂到
// crate 根，bin 侧经 #[macro_use] extern crate nebulaid 引入。
// 运行时语义：按进程默认 locale 命中 FTL 键，未命中退 en 束并返回键本身，
// 参数经 Display 格式化后交 Fluent 变量解析。
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

// Core namespace：核心业务逻辑
pub mod core;

// Server namespace：HTTP/gRPC 服务
#[cfg(feature = "server")]
pub mod server;

// SDK namespace：嵌入式 SDK facade
#[cfg(feature = "sdk")]
pub mod sdk;
