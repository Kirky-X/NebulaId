// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Configuration error types.

use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ConfigError {
    /// TOML 语法错误、字段缺失或未知、`Config::validate` 不通过都落在本变体。
    #[error("Invalid configuration value: {}", _0)]
    InvalidValue(String),

    /// 配置文件不存在（`io::ErrorKind::NotFound`）；必须与 [`ConfigError::FileError`] 区分：
    /// 仅文件确实不存在才回落默认值，权限/磁盘等 IO 失败不得误判为缺失。
    #[error("Configuration file not found: {}", _0)]
    FileNotFound(String),

    /// 读取配置文件失败，但失败原因不是"文件不存在"（权限、IO 错误、路径不是文件等）。
    #[error("Configuration file error: {}", _0)]
    FileError(String),
}

pub type ConfigResult<T> = std::result::Result<T, ConfigError>;
