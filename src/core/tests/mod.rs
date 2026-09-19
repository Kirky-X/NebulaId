// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

#[cfg(test)]
pub mod algorithm_e2e_tests;
#[cfg(test)]
pub mod cache_tests;
// 注：任务原文称「不需要注册到 mod.rs」，但本 crate 所有 lib 测试模块均通过
// mod.rs 注册才能被 `cargo test --lib` 编译执行（segment 模块为 pub(crate)，
// 需作为 crate 内 module 挂载）。为满足验证步骤并遵循既有惯例，仍在此注册。
#[cfg(test)]
pub mod auth_handlers_e2e_tests;
#[cfg(test)]
pub mod degradation_tests;
#[cfg(test)]
pub mod grpc_monitoring_e2e_tests;
#[cfg(test)]
pub mod infra_e2e_tests;
#[cfg(test)]
pub mod integration_tests;
#[cfg(test)]
pub mod remaining_e2e_tests;
#[cfg(test)]
pub mod segment_monitoring_e2e_tests;
#[cfg(test)]
pub mod server_layer_e2e_tests;
#[cfg(test)]
pub mod supporting_layer_e2e_tests;
