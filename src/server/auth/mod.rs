// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! garrison 集成模块 — 提供 `GarrisonDao` 的进程内实现与认证缓存门面。
//!
//! 仅在启用 `garrison-auth` 特性时编译。`MemoryGarrisonDao` 用
//! `parking_lot::RwLock<HashMap>` + per-entry TTL 模拟 garrison 的 KV 存储，
//! 供 `ApiKeyHandler` 进行 token 生成/校验/吊销/轮换。`AuthCache` 在其之上
//! 缓存认证决策，消除每请求 Argon2id + DB 校验。

pub mod cache;
pub mod memory_dao;

pub use cache::{AuthCache, CachedIdentity};

pub use memory_dao::MemoryGarrisonDao;
