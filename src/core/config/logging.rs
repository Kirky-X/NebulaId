// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! Logging configuration.

use serde::{Deserialize, Serialize};

/// Log level for the application
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    /// Trace level - most verbose
    Trace,
    /// Debug level
    Debug,
    /// Info level - default
    Info,
    /// Warning level
    Warn,
    /// Error level
    Error,
}

impl std::fmt::Display for LogLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LogLevel::Trace => write!(f, "trace"),
            LogLevel::Debug => write!(f, "debug"),
            LogLevel::Info => write!(f, "info"),
            LogLevel::Warn => write!(f, "warn"),
            LogLevel::Error => write!(f, "error"),
        }
    }
}

impl From<&str> for LogLevel {
    fn from(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "trace" => LogLevel::Trace,
            "debug" => LogLevel::Debug,
            "info" => LogLevel::Info,
            "warn" => LogLevel::Warn,
            "error" => LogLevel::Error,
            _ => LogLevel::Info,
        }
    }
}

impl From<String> for LogLevel {
    fn from(s: String) -> Self {
        s.as_str().into()
    }
}

/// Log format for the application
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    /// JSON format - structured logging
    Json,
    /// Pretty format - human readable
    Pretty,
}

impl std::fmt::Display for LogFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LogFormat::Json => write!(f, "json"),
            LogFormat::Pretty => write!(f, "pretty"),
        }
    }
}

impl From<&str> for LogFormat {
    fn from(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "json" => LogFormat::Json,
            "pretty" => LogFormat::Pretty,
            _ => LogFormat::Json,
        }
    }
}

impl From<String> for LogFormat {
    fn from(s: String) -> Self {
        s.as_str().into()
    }
}

/// Logging configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LoggingConfig {
    /// Log level
    pub level: LogLevel,
    /// Log format (json/pretty)
    pub format: LogFormat,
    /// Include source location in logs
    pub include_location: bool,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: LogLevel::Info,
            format: LogFormat::Json,
            include_location: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_log_level_display_all_variants() {
        assert_eq!(LogLevel::Trace.to_string(), "trace");
        assert_eq!(LogLevel::Debug.to_string(), "debug");
        assert_eq!(LogLevel::Info.to_string(), "info");
        assert_eq!(LogLevel::Warn.to_string(), "warn");
        assert_eq!(LogLevel::Error.to_string(), "error");
    }

    #[test]
    fn test_log_level_from_str_all_branches() {
        assert_eq!(LogLevel::from("trace"), LogLevel::Trace);
        assert_eq!(LogLevel::from("debug"), LogLevel::Debug);
        assert_eq!(LogLevel::from("info"), LogLevel::Info);
        assert_eq!(LogLevel::from("warn"), LogLevel::Warn);
        assert_eq!(LogLevel::from("error"), LogLevel::Error);
        assert_eq!(LogLevel::from("TRACE"), LogLevel::Trace);
        assert_eq!(LogLevel::from("unknown"), LogLevel::Info);
        assert_eq!(LogLevel::from("".to_string()), LogLevel::Info);
    }

    #[test]
    fn test_log_format_display_and_from() {
        assert_eq!(LogFormat::Json.to_string(), "json");
        assert_eq!(LogFormat::Pretty.to_string(), "pretty");
        assert_eq!(LogFormat::from("json"), LogFormat::Json);
        assert_eq!(LogFormat::from("pretty"), LogFormat::Pretty);
        assert_eq!(LogFormat::from("JSON"), LogFormat::Json);
        assert_eq!(LogFormat::from("xml"), LogFormat::Json);
        assert_eq!(LogFormat::from("pretty".to_string()), LogFormat::Pretty);
    }
}
