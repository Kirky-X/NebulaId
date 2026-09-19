// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! 嵌入式 SDK（trait-kit Kit 范式装配，feature `sdk` 门控）。
//!
//! 规则 25：本 `mod.rs` 只做模块声明与 re-export，实现位于 [`kit`]。

pub mod kit;

pub use kit::{IdGenerator, NebulaIdKit, NebulaIdKitBuilder};
