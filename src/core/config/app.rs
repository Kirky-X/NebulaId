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
        // If DATABASE_URL is set, password is embedded in URL, not required separately
        if std::env::var("DATABASE_URL").is_ok() {
            return Self {
                engine: DatabaseEngine::Postgresql,
                url: std::env::var("DATABASE_URL").unwrap(),
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
