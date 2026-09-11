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

/// Application runtime environment
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Environment {
    /// Development environment (default)
    #[default]
    Development,
    /// Production environment
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
    fn from(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "production" | "prod" => Environment::Production,
            _ => Environment::Development,
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
    pub fn from_env() -> Self {
        std::env::var("NEBULA_ENV")
            .map(|s| s.as_str().into())
            .unwrap_or_default()
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
        assert_eq!(
            Environment::from("anything-else".to_string()),
            Environment::Development
        );
        assert_eq!(Environment::from("prod"), Environment::Production);
        assert_eq!(Environment::from("dev"), Environment::Development);
        assert_eq!(Environment::default(), Environment::Development);
    }

    #[test]
    fn test_environment_predicates() {
        assert!(Environment::Production.is_production());
        assert!(!Environment::Development.is_production());
        assert!(Environment::Development.is_development());
        assert!(!Environment::Production.is_development());
    }
}
