// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

// 用例与导入面依赖三算法特性并集；按并集门控整模块，防剪裁组合下 unused 残留。
#[cfg(all(test, feature = "segment", feature = "snowflake", feature = "uuid-v8"))]
pub mod algorithm_e2e_tests;
#[cfg(all(test, feature = "server"))]
pub mod cache_tests;
// 本 crate 的 lib 测试模块须经 mod.rs 注册才能被 `cargo test --lib` 编译（segment 为 pub(crate)）。
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
