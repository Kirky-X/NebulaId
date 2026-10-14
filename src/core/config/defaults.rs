// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! 跨模块共享的默认值常量统一注册表。
//!
//! 仅收录被 ≥2 个文件引用的 `DEFAULT_*` 常量；单文件使用的保留在原定义处，不为搬而搬。

/// API key 轮换宽限期默认值：`0` = 关闭（不写 `prev_secret_hash`，上一代凭证立即失效）。
///
/// 关闭而非 7 天：宽限期在语义上是"让已知/可能泄露的旧凭证继续有效"，该取舍必须由
/// 显式配置承担。需"轮换不掉请求"时由运维显式配置 `auth.key_rotation_grace_period_seconds`。
pub const DEFAULT_KEY_ROTATION_GRACE_PERIOD_SECONDS: u64 = 0;

/// 宽限期上限：30 天（秒）。超限值在配置校验阶段即被拒绝（fail-fast），
/// 避免静默传递到 handler 层才被 clamp。
pub const MAX_KEY_ROTATION_GRACE_PERIOD_SECONDS: u64 = 30 * 24 * 60 * 60;
