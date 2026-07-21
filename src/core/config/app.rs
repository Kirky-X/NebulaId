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

//! Application, database, and etcd configuration.

use serde::{Deserialize, Serialize};
use std::net::SocketAddr;

/// Database engine types supported by Nebula ID
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum DatabaseEngine {
    /// PostgreSQL database
    Postgresql,
    /// PostgreSQL (alias)
    Postgres,
    /// MySQL database
    Mysql,
    /// SQLite database
    Sqlite,
}

impl std::fmt::Display for DatabaseEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DatabaseEngine::Postgresql | DatabaseEngine::Postgres => write!(f, "postgresql"),
            DatabaseEngine::Mysql => write!(f, "mysql"),
            DatabaseEngine::Sqlite => write!(f, "sqlite"),
        }
    }
}

impl From<&str> for DatabaseEngine {
    fn from(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "postgresql" | "postgres" => DatabaseEngine::Postgresql,
            "mysql" => DatabaseEngine::Mysql,
            "sqlite" => DatabaseEngine::Sqlite,
            _ => DatabaseEngine::Postgresql,
        }
    }
}

impl From<String> for DatabaseEngine {
    fn from(s: String) -> Self {
        s.as_str().into()
    }
}

/// Application configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AppConfig {
    /// Application name
    pub name: String,
    /// Server listen address
    pub host: String,
    /// HTTP server port
    pub http_port: u16,
    /// gRPC server port
    pub grpc_port: u16,
    /// Datacenter ID (0-31)
    pub dc_id: u8,
    /// Worker ID (0-255)
    pub worker_id: u8,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            name: "nebula-id".to_string(),
            host: "0.0.0.0".to_string(),
            http_port: 8080,
            grpc_port: 9091,
            dc_id: 0,
            worker_id: 0,
        }
    }
}

impl AppConfig {
    pub fn http_addr(&self) -> Result<SocketAddr, Box<dyn std::error::Error + Send + Sync>> {
        format!("{}:{}", self.host, self.http_port)
            .parse()
            .map_err(|e| {
                format!(
                    "Invalid HTTP address '{}:{}': {}",
                    self.host, self.http_port, e
                )
                .into()
            })
    }

    pub fn grpc_addr(&self) -> Result<SocketAddr, Box<dyn std::error::Error + Send + Sync>> {
        format!("{}:{}", self.host, self.grpc_port)
            .parse()
            .map_err(|e| {
                format!(
                    "Invalid gRPC address '{}:{}': {}",
                    self.host, self.grpc_port, e
                )
                .into()
            })
    }
}

/// Database configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DatabaseConfig {
    /// Database engine type
    pub engine: DatabaseEngine,
    /// Database host address
    pub host: String,
    /// Database port
    pub port: u16,
    /// Database username
    pub username: String,
    /// Database password
    pub password: String,
    /// Database name
    pub database: String,
    /// Full database URL (alternative to individual settings)
    #[serde(default)]
    pub url: String,
    /// Maximum number of connections in pool
    pub max_connections: u32,
    /// Minimum number of connections in pool
    pub min_connections: u32,
    /// Connection acquisition timeout (seconds)
    pub acquire_timeout_seconds: u64,
    /// Idle connection timeout (seconds)
    pub idle_timeout_seconds: u64,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        // If DATABASE_URL is set, password is embedded in URL, not required separately.
        // 用 `if let Ok(url) = ...` 模式只读取一次 env var，避免原写法中
        // `.is_ok()` 检查后 `.unwrap()` 读取之间的 TOCTOU race（与
        // `load_from_env_database_url` 等设置 DATABASE_URL 的测试并行时
        // 会 panic）。
        if let Ok(url) = std::env::var("DATABASE_URL") {
            return Self {
                engine: DatabaseEngine::Postgresql,
                url,
                host: "localhost".to_string(),
                port: 5432,
                username: "idgen".to_string(),
                password: String::new(),
                database: "idgen".to_string(),
                max_connections: 100,
                min_connections: 10,
                acquire_timeout_seconds: 30,
                idle_timeout_seconds: 300,
            };
        }

        // For tests, allow fallback to in-memory database
        // SECURITY: This fallback is ONLY for testing, not production
        if cfg!(test) || std::env::var("NEBULA_TEST_MODE").is_ok() {
            return Self {
                engine: DatabaseEngine::Sqlite,
                url: "sqlite::memory:".to_string(),
                host: String::new(),
                port: 0,
                username: String::new(),
                password: String::new(),
                database: String::new(),
                max_connections: 10,
                min_connections: 1,
                acquire_timeout_seconds: 30,
                idle_timeout_seconds: 300,
            };
        }

        // SECURITY: Require environment variable for password in production
        let password = std::env::var("NEBULA_DATABASE_PASSWORD")
            .expect("NEBULA_DATABASE_PASSWORD environment variable must be set in production. For development, set this variable or use DATABASE_URL.");

        if password == "idgen123" || password.is_empty() {
            tracing::warn!(
                "{}",
                t!("log.core.config.app.weak_database_password_detected")
            );
        }

        Self {
            engine: DatabaseEngine::Postgresql,
            url: format!("postgresql://idgen:{}@localhost:5432/idgen", password),
            host: "localhost".to_string(),
            port: 5432,
            username: "idgen".to_string(),
            password,
            database: "idgen".to_string(),
            max_connections: 100,
            min_connections: 10,
            acquire_timeout_seconds: 30,
            idle_timeout_seconds: 300,
        }
    }
}

/// etcd configuration for distributed coordination
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EtcdConfig {
    /// List of etcd endpoints
    pub endpoints: Vec<String>,
    /// Connection timeout (milliseconds)
    pub connect_timeout_ms: u64,
    /// Watch timeout (milliseconds)
    pub watch_timeout_ms: u64,
}

impl Default for EtcdConfig {
    fn default() -> Self {
        Self {
            endpoints: vec!["etcd:2379".to_string()],
            connect_timeout_ms: 5000,
            watch_timeout_ms: 5000,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ----- DatabaseEngine Display -----

    #[test]
    fn test_database_engine_display_postgresql() {
        assert_eq!(DatabaseEngine::Postgresql.to_string(), "postgresql");
    }

    #[test]
    fn test_database_engine_display_postgres_alias() {
        // Postgres 是 PostgreSQL 的别名，Display 输出应与 Postgresql 一致
        assert_eq!(DatabaseEngine::Postgres.to_string(), "postgresql");
    }

    #[test]
    fn test_database_engine_display_mysql() {
        assert_eq!(DatabaseEngine::Mysql.to_string(), "mysql");
    }

    #[test]
    fn test_database_engine_display_sqlite() {
        assert_eq!(DatabaseEngine::Sqlite.to_string(), "sqlite");
    }

    // ----- DatabaseEngine From<&str> -----

    #[test]
    fn test_database_engine_from_str_postgresql() {
        let e: DatabaseEngine = "postgresql".into();
        assert_eq!(e, DatabaseEngine::Postgresql);
    }

    #[test]
    fn test_database_engine_from_str_postgres() {
        let e: DatabaseEngine = "postgres".into();
        assert_eq!(e, DatabaseEngine::Postgresql);
    }

    #[test]
    fn test_database_engine_from_str_mysql() {
        let e: DatabaseEngine = "mysql".into();
        assert_eq!(e, DatabaseEngine::Mysql);
    }

    #[test]
    fn test_database_engine_from_str_sqlite() {
        let e: DatabaseEngine = "sqlite".into();
        assert_eq!(e, DatabaseEngine::Sqlite);
    }

    #[test]
    fn test_database_engine_from_str_unknown_falls_back_to_postgresql() {
        // 未知字符串应回退到 PostgreSQL（默认值，不报错）
        let e: DatabaseEngine = "redis".into();
        assert_eq!(e, DatabaseEngine::Postgresql);
    }

    #[test]
    fn test_database_engine_from_str_case_insensitive() {
        // 大小写不敏感：MySQL/MYSQL/POSTGRESQL 都应被识别
        let e: DatabaseEngine = "MySQL".into();
        assert_eq!(e, DatabaseEngine::Mysql);

        let e: DatabaseEngine = "POSTGRESQL".into();
        assert_eq!(e, DatabaseEngine::Postgresql);

        let e: DatabaseEngine = "SQLite".into();
        assert_eq!(e, DatabaseEngine::Sqlite);
    }

    // ----- DatabaseEngine From<String> -----

    #[test]
    fn test_database_engine_from_string_postgresql() {
        let e: DatabaseEngine = String::from("postgresql").into();
        assert_eq!(e, DatabaseEngine::Postgresql);
    }

    #[test]
    fn test_database_engine_from_string_mysql() {
        let e: DatabaseEngine = String::from("mysql").into();
        assert_eq!(e, DatabaseEngine::Mysql);
    }

    #[test]
    fn test_database_engine_from_string_unknown_falls_back() {
        let e: DatabaseEngine = String::from("unknown-db").into();
        assert_eq!(e, DatabaseEngine::Postgresql);
    }

    // ----- AppConfig Default + http_addr / grpc_addr -----

    #[test]
    fn test_app_config_default_values() {
        let cfg = AppConfig::default();
        assert_eq!(cfg.name, "nebula-id");
        assert_eq!(cfg.host, "0.0.0.0");
        assert_eq!(cfg.http_port, 8080);
        assert_eq!(cfg.grpc_port, 9091);
        assert_eq!(cfg.dc_id, 0);
        assert_eq!(cfg.worker_id, 0);
    }

    #[test]
    fn test_app_config_http_addr_success() {
        let cfg = AppConfig::default();
        let addr = cfg.http_addr().expect("default http_addr should parse");
        assert_eq!(addr.port(), 8080);
        assert_eq!(addr.ip().to_string(), "0.0.0.0");
    }

    #[test]
    fn test_app_config_grpc_addr_success() {
        let cfg = AppConfig::default();
        let addr = cfg.grpc_addr().expect("default grpc_addr should parse");
        assert_eq!(addr.port(), 9091);
        assert_eq!(addr.ip().to_string(), "0.0.0.0");
    }

    #[test]
    fn test_app_config_http_addr_invalid_host_returns_error() {
        // 不合法的 host（带空格）→ 解析失败
        let cfg = AppConfig {
            host: "not a valid host".to_string(),
            ..Default::default()
        };
        let result = cfg.http_addr();
        let err = result.expect_err("invalid host should yield parse error");
        let msg = err.to_string();
        assert!(
            msg.contains("Invalid HTTP address"),
            "error message should mention HTTP address, got: {msg}"
        );
        assert!(msg.contains("not a valid host"));
        assert!(msg.contains("8080"));
    }

    #[test]
    fn test_app_config_grpc_addr_invalid_host_returns_error() {
        let cfg = AppConfig {
            host: "not a valid host".to_string(),
            ..Default::default()
        };
        let result = cfg.grpc_addr();
        let err = result.expect_err("invalid host should yield parse error");
        let msg = err.to_string();
        assert!(
            msg.contains("Invalid gRPC address"),
            "error message should mention gRPC address, got: {msg}"
        );
        assert!(msg.contains("not a valid host"));
        assert!(msg.contains("9091"));
    }

    #[test]
    fn test_app_config_http_addr_custom_port() {
        // 自定义端口应正确解析
        let cfg = AppConfig {
            host: "127.0.0.1".to_string(),
            http_port: 12345,
            ..Default::default()
        };
        let addr = cfg.http_addr().unwrap();
        assert_eq!(addr.port(), 12345);
        assert_eq!(addr.ip().to_string(), "127.0.0.1");
    }

    #[test]
    fn test_app_config_grpc_addr_custom_port() {
        let cfg = AppConfig {
            host: "127.0.0.1".to_string(),
            grpc_port: 54321,
            ..Default::default()
        };
        let addr = cfg.grpc_addr().unwrap();
        assert_eq!(addr.port(), 54321);
        assert_eq!(addr.ip().to_string(), "127.0.0.1");
    }

    // ----- EtcdConfig Default -----

    #[test]
    fn test_etcd_config_default_values() {
        let cfg = EtcdConfig::default();
        assert_eq!(cfg.endpoints, vec!["etcd:2379".to_string()]);
        assert_eq!(cfg.connect_timeout_ms, 5000);
        assert_eq!(cfg.watch_timeout_ms, 5000);
    }

    // ----- DatabaseConfig::default 在测试模式下应进入 sqlite 分支 -----

    #[test]
    fn test_database_config_default_in_test_mode_uses_sqlite() {
        // cfg!(test) 为 true 时，应进入 in-memory sqlite 分支
        let cfg = DatabaseConfig::default();
        assert_eq!(cfg.engine, DatabaseEngine::Sqlite);
        assert_eq!(cfg.url, "sqlite::memory:");
        assert_eq!(cfg.max_connections, 10);
        assert_eq!(cfg.min_connections, 1);
        assert_eq!(cfg.acquire_timeout_seconds, 30);
        assert_eq!(cfg.idle_timeout_seconds, 300);
        // 测试模式下 host/port/username/password/database 均为空
        assert!(cfg.host.is_empty());
        assert_eq!(cfg.port, 0);
        assert!(cfg.username.is_empty());
        assert!(cfg.password.is_empty());
        assert!(cfg.database.is_empty());
    }
}
