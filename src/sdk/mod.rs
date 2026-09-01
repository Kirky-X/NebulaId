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

//! 嵌入式 SDK（trait-kit Kit 范式装配，feature `sdk` 门控）。
//!
//! 规则 25：本 `mod.rs` 只做模块声明与 re-export，实现位于 [`kit`]（`client` 为
//! 旧 API，T008 移除）。

pub mod client;
pub mod kit;

pub use client::{NebulaIdClient, NebulaIdClientBuilder};
pub use kit::{NebulaIdKit, NebulaIdKitBuilder};
