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

//! Configuration error types.

use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ConfigError {
    #[error("Missing required configuration: {}", _0)]
    MissingRequired(String),

    #[error("Invalid configuration value: {}", _0)]
    InvalidValue(String),

    /// 配置文件不存在（`io::ErrorKind::NotFound`）。
    ///
    /// 必须与 [`ConfigError::FileError`] 区分：启动期配置解析只允许在"文件确实不存
    /// 在"时回落到内置默认值，而权限、磁盘、路径类型等 IO 失败不得被误判为缺失，
    /// 否则坏配置会静默降级成默认配置。
    #[error("Configuration file not found: {}", _0)]
    FileNotFound(String),

    /// 读取配置文件失败，但失败原因不是"文件不存在"（权限、IO 错误、路径不是文件等）。
    #[error("Configuration file error: {}", _0)]
    FileError(String),
}

pub type ConfigResult<T> = std::result::Result<T, ConfigError>;
