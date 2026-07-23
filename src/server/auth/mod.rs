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

//! garrison 集成模块 — 提供 `GarrisonDao` 的进程内实现。
//!
//! 仅在启用 `garrison-auth` 特性时编译。`MemoryGarrisonDao` 用
//! `parking_lot::RwLock<HashMap>` + per-entry TTL 模拟 garrison 的 KV 存储，
//! 供 `ApiKeyHandler` 进行 token 生成/校验/吊销/轮换。

pub mod memory_dao;

pub use memory_dao::MemoryGarrisonDao;
