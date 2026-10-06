// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

// 文件内用例的并集依赖三算法特性（导入面亦然），按并集门控整模块，
// 防止剪裁组合下的 unused 残留。
#[cfg(all(test, feature = "segment", feature = "snowflake", feature = "uuid-v8"))]
pub mod algorithm_e2e_tests;
#[cfg(all(test, feature = "server"))]
pub mod cache_tests;
// 注：任务原文称「不需要注册到 mod.rs」，但本 crate 所有 lib 测试模块均通过
// mod.rs 注册才能被 `cargo test --lib` 编译执行（segment 模块为 pub(crate)，
// 需作为 crate 内 module 挂载）。为满足验证步骤并遵循既有惯例，仍在此注册。
#[cfg(all(test, feature = "server"))]
pub mod auth_handlers_e2e_tests;
#[cfg(test)]
pub mod degradation_tests;
#[cfg(all(test, feature = "server", feature = "alerting"))]
pub mod grpc_monitoring_e2e_tests;
#[cfg(all(test, feature = "server"))]
pub mod infra_e2e_tests;
#[cfg(all(test, feature = "segment"))]
pub mod integration_tests;
#[cfg(all(test, feature = "server", feature = "alerting"))]
pub mod remaining_e2e_tests;
#[cfg(all(test, feature = "segment"))]
pub mod segment_monitoring_e2e_tests;
#[cfg(all(test, feature = "server"))]
pub mod server_layer_e2e_tests;
#[cfg(all(test, feature = "server"))]
pub mod supporting_layer_e2e_tests;
