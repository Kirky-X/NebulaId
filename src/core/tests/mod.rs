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
pub mod coordinator_auth_e2e_tests;
pub mod degradation_tests;
#[cfg(test)]
pub mod dynamic_step_tests;
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
