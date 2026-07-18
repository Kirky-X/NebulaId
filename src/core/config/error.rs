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

    #[error("Configuration file error: {}", _0)]
    FileError(String),
}

pub type ConfigResult<T> = std::result::Result<T, ConfigError>;
