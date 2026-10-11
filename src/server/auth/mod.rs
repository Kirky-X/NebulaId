// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! garrison 集成模块 — 提供 `GarrisonDao` 的进程内实现与认证缓存门面。
//!
//! 仅在启用 `garrison-auth` 特性时编译。进程内 DAO 使用 garrison 库
//! `dao::InMemoryDao`（本模块 re-export 为 [`InMemoryDao`]），供给
//! `ApiKeyHandler` 进行 token 生成/校验/吊销/轮换。`AuthCache` 在其之上
//! 缓存认证决策，消除每请求 Argon2id + DB 校验。

pub mod cache;
pub mod memory_dao;

pub use cache::{AuthCache, CachedIdentity};

pub use memory_dao::{BoundedDao, InMemoryDao};
