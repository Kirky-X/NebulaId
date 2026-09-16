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

//! Application runtime environment.

use rust_i18n::t;
use std::sync::Once;

/// warn 一次：`NEBULA_ENV` 缺失（`from_env` 可能在启动路径被多处调用）。
static MISSING_ENV_WARN: Once = Once::new();

/// warn 一次：`NEBULA_ENV` 取值无法识别。
static UNKNOWN_ENV_WARN: Once = Once::new();

/// Application runtime environment
///
/// 反向默认（fail-closed）：[`Environment::Production`] 是 `Default`。
/// 只有显式设置 `NEBULA_ENV=development|dev` 才进入开发模式；
/// 缺失或无法识别的取值一律按生产校验执行（见 [`Environment::from_env`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Environment {
    /// Development environment（显式 `NEBULA_ENV=development|dev` 才生效）
    Development,
    /// Production environment（fail-closed 默认）
    #[default]
    Production,
}

impl std::fmt::Display for Environment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Environment::Development => write!(f, "development"),
            Environment::Production => write!(f, "production"),
        }
    }
}

impl std::str::FromStr for Environment {
    type Err = super::ConfigError;

    fn from_str(s: &str) -> super::ConfigResult<Self> {
        match s.to_lowercase().as_str() {
            "development" | "dev" => Ok(Environment::Development),
            "production" | "prod" => Ok(Environment::Production),
            _ => Err(super::ConfigError::InvalidValue(format!(
                "Invalid environment '{}'. Expected 'development' or 'production'",
                s
            ))),
        }
    }
}

impl From<String> for Environment {
    fn from(s: String) -> Self {
        s.as_str().into()
    }
}

impl From<&str> for Environment {
    /// 反向默认（fail-closed）：只有显式 `development|dev` 是开发，
    /// `production|prod` 与一切无法识别的取值一律按生产处理。
    fn from(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "development" | "dev" => Environment::Development,
            _ => Environment::Production,
        }
    }
}

impl Environment {
    /// Check if this is the production environment
    pub fn is_production(&self) -> bool {
        matches!(self, Environment::Production)
    }

    /// Check if this is the development environment
    pub fn is_development(&self) -> bool {
        matches!(self, Environment::Development)
    }

    /// Get the current environment from NEBULA_ENV environment variable
    ///
    /// 反向默认（fail-closed）：
    /// - `NEBULA_ENV` 缺失 → [`Environment::Production`]，启动时 warn 一次
    ///   （提示未显式设置、按生产校验执行；显式 `NEBULA_ENV=development` 可回到开发模式）
    /// - 无法识别的取值 → [`Environment::Production`] + warn 一次
    /// - 显式 `development` / `dev` → [`Environment::Development`]
    /// - 显式 `production` / `prod` → [`Environment::Production`]
    pub fn from_env() -> Self {
        match std::env::var("NEBULA_ENV") {
            Err(_) => {
                MISSING_ENV_WARN.call_once(|| {
                    tracing::warn!(
                        "{}",
                        t!("log.core.config.environment.missing_nebula_env_treated_as_production")
                    );
                });
                Environment::Production
            }
            Ok(value) => match value.to_lowercase().as_str() {
                "development" | "dev" => Environment::Development,
                "production" | "prod" => Environment::Production,
                _ => {
                    UNKNOWN_ENV_WARN.call_once(|| {
                        tracing::warn!(
                            "{}",
                            t!(
                                "log.core.config.environment.unknown_nebula_env_treated_as_production",
                                value = value
                            )
                        );
                    });
                    Environment::Production
                }
            },
        }
    }
}

/// Check if the application is running in production environment
pub fn is_production() -> bool {
    Environment::from_env().is_production()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn test_environment_display() {
        assert_eq!(Environment::Development.to_string(), "development");
        assert_eq!(Environment::Production.to_string(), "production");
    }

    #[test]
    fn test_environment_from_str() {
        assert_eq!(
            Environment::from_str("development").unwrap(),
            Environment::Development
        );
        assert_eq!(
            Environment::from_str("dev").unwrap(),
            Environment::Development
        );
        assert_eq!(
            Environment::from_str("PROD").unwrap(),
            Environment::Production
        );
        assert_eq!(
            Environment::from_str("production").unwrap(),
            Environment::Production
        );
        let err = Environment::from_str("staging").unwrap_err();
        assert!(err.to_string().contains("staging"));
    }

    #[test]
    fn test_environment_from_string_and_str() {
        assert_eq!(
            Environment::from("production".to_string()),
            Environment::Production
        );
        // 反向默认（fail-closed）：无法识别的取值一律按生产处理
        assert_eq!(
            Environment::from("anything-else".to_string()),
            Environment::Production
        );
        assert_eq!(Environment::from("prod"), Environment::Production);
        assert_eq!(Environment::from("dev"), Environment::Development);
        // 反向默认（fail-closed）：Default 是 Production
        assert_eq!(Environment::default(), Environment::Production);
    }

    #[test]
    fn test_environment_predicates() {
        assert!(Environment::Production.is_production());
        assert!(!Environment::Development.is_production());
        assert!(Environment::Development.is_development());
        assert!(!Environment::Production.is_development());
    }

    /// 串行化 `NEBULA_ENV` 环境变量的读写：进程级 env 是全局状态，
    /// 并行操作会互相踩踏（与 core/tests 的 E2E_ENV_LOCK 同一口径）。
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// 守卫：测试结束时无条件移除 NEBULA_ENV，避免污染同进程其他测试。
    struct EnvGuard;

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            std::env::remove_var("NEBULA_ENV");
        }
    }

    #[test]
    fn test_environment_from_env_unset_defaults_to_production() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _guard = EnvGuard;
        std::env::remove_var("NEBULA_ENV");

        // 反向默认（fail-closed）：NEBULA_ENV 缺失时按生产校验执行
        assert_eq!(Environment::from_env(), Environment::Production);
        assert!(is_production());
    }

    #[test]
    fn test_environment_from_env_explicit_development() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _guard = EnvGuard;
        std::env::set_var("NEBULA_ENV", "development");

        // 显式 development 才回到开发模式
        assert_eq!(Environment::from_env(), Environment::Development);
        assert!(!is_production());
    }

    #[test]
    fn test_environment_from_env_explicit_production() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _guard = EnvGuard;
        std::env::set_var("NEBULA_ENV", "production");

        assert_eq!(Environment::from_env(), Environment::Production);
        assert!(is_production());
    }

    #[test]
    fn test_environment_from_env_unknown_value_defaults_to_production() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _guard = EnvGuard;
        std::env::set_var("NEBULA_ENV", "staging");

        // 无法识别的取值按生产处理（fail-closed），不静默降级为开发模式
        assert_eq!(Environment::from_env(), Environment::Production);
        assert!(is_production());
    }

    #[test]
    fn test_environment_from_env_explicit_dev_alias() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _guard = EnvGuard;
        std::env::set_var("NEBULA_ENV", "dev");

        assert_eq!(Environment::from_env(), Environment::Development);
        assert!(!is_production());
    }
}
