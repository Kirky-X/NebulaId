// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! garrison 集成模块 — `GarrisonDao` 的进程内实现与认证缓存门面。
//!
//! 仅 `garrison-auth` 特性下编译；`AuthCache` 缓存认证决策，避免每请求 Argon2id + DB 校验。

pub mod cache;
pub mod memory_dao;

pub use cache::{AuthCache, CachedIdentity};

pub use memory_dao::{BoundedDao, InMemoryDao};
