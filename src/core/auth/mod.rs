// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! 认证支撑模块：API Key 凭证哈希抽象与默认实现。

pub mod key_hasher;

pub use key_hasher::{Argon2KeyHasher, KeyHasher};
