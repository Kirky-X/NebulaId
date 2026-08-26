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

//! 跨模块共享的默认值常量统一注册表（code-hygiene-cleanup T009）。
//!
//! 收录规则：仅迁移被 **≥2 个文件**引用的 `DEFAULT_*` 常量；单文件使用的
//! 常量保留在原定义处，不为搬而搬。历史教训见
//! [`crate::core::config::auth`] 中 grace period 曾三处重复的自述。

/// API key 轮换宽限期默认值：7 天（秒）。
///
/// 原定义于 `core/config/auth.rs`，因被 infrastructure/config_adapter 与
/// server/handlers 两域共同消费而迁入本注册表；原位置经 `pub use` 保持
/// 导入路径兼容。
pub const DEFAULT_KEY_ROTATION_GRACE_PERIOD_SECONDS: u64 = 7 * 24 * 60 * 60;
