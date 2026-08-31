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

/// API key 轮换宽限期默认值：`0` = 关闭（T011）。
///
/// `0` 表示轮换后上一代凭证立即失效：`rotate_api_key` 不写 `prev_secret_hash`
/// / `rotate_expires_at`，`validate_api_key` 只校验当代凭证。需要"轮换不掉请求"
/// 时由运维显式配置 `auth.key_rotation_grace_period_seconds`（上限 30 天）。
/// 默认值取关闭而非 7 天：宽限期在语义上是"让一个已知/可能泄露的旧凭证继续有效"，
/// 该取舍必须由显式配置承担，不能作为出厂默认。
///
/// 原定义于 `core/config/auth.rs`，因被 `core/config/auth.rs`（serde 默认值）与
/// `server/handlers/mod.rs`（未注入配置时的兜底）两域共同消费而迁入本注册表；
/// 全仓仅此一处定义。
pub const DEFAULT_KEY_ROTATION_GRACE_PERIOD_SECONDS: u64 = 0;
