// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

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
#[serde(deny_unknown_fields)]
pub struct NebulaIdConfig {
    /// Application name
    pub name: String,
    /// Server listen address
    pub host: String,
    /// HTTP server port
    pub http_port: u16,
    /// gRPC server port
    pub grpc_port: u16,
    /// Datacenter ID (0-7)
    ///
    /// 多实例约束：未配置 etcd（无 worker_id 运行时分配）时，本值与
    /// [`NebulaIdConfig::worker_id`] 共同构成 Snowflake 的机器标识；多实例部署
    /// 必须显式配置（环境变量 `DC_ID` 或配置文件），默认值 0 会使多实例
    /// 生成重复 ID（启动时进程会输出 warn 提醒，见 main.rs T018）。
    pub dc_id: u8,
    /// Worker ID (0-255)
    ///
    /// 多实例约束：配置了 etcd endpoints 时本值会被运行时分配的 worker_id
    /// 覆盖（T017）；未配置 etcd 时即为 Snowflake 的最终机器标识，多实例
    /// 部署必须显式配置（环境变量 `WORKER_ID` 或配置文件），默认值 0 会使
    /// 多实例生成重复 ID（启动时进程会输出 warn 提醒，见 main.rs T018）。
    pub worker_id: u8,
    /// Graceful shutdown timeout (seconds)
    #[serde(default = "default_shutdown_timeout_seconds")]
    pub shutdown_timeout_seconds: u64,
    /// Process default locale (T035/T013)。空串(auto,serde 默认)= 未显式
    /// 表达语言偏好,跟随系统语言检测链(NEBULA_LOCALE → 本字段 → LC_ALL/
    /// LC_MESSAGES/LANG → sys-locale → en,见 main.rs `resolve_locale`);
    /// 显式写入规范值 "en"/"zh-CN" 则钉死进程语言,**用户配置优先于系统
    /// 语言**。非法值在启动时告警并继续走链(链尾落 en)。
    #[serde(default = "default_locale")]
    pub locale: String,
}

fn default_shutdown_timeout_seconds() -> u64 {
    30
}

fn default_locale() -> String {
    // T013(unify-rust-i18n):默认空串 = auto(跟随系统语言检测链,链尾 en)。
    // 若默认 "en",config.app.locale 会在检测链中恒短路,统一基线要求的
    // 系统语言自动检测在默认部署下不可达(设计决策,见 T013 实施记录)。
    String::new()
}

impl Default for NebulaIdConfig {
    fn default() -> Self {
        Self {
            name: "nebula-id".to_string(),
            host: "0.0.0.0".to_string(),
            http_port: 8080,
            grpc_port: 9091,
            dc_id: 0,
            worker_id: 0,
            shutdown_timeout_seconds: default_shutdown_timeout_seconds(),
            locale: default_locale(),
        }
    }
}

impl NebulaIdConfig {
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
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
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
    /// 单语句执行超时（秒，T028）。仓储热查询（validate_api_key /
    /// allocate_segment 等）经 `tokio::time::timeout` 包裹，防止 DB 挂起
    /// 拖死生成与认证热路径。默认 5 秒。
    #[serde(default = "default_statement_timeout_secs")]
    pub statement_timeout_secs: u64,
}

/// T028 —— `statement_timeout_secs` 的 serde 默认值（与仓储内置默认一致）。
fn default_statement_timeout_secs() -> u64 {
    5
}

/// 手写 `Debug`：`password` 是明文口令，`url` 可能以
/// `postgresql://user:pass@host/db` 的形式内嵌口令，两者都不得进 `{:?}` 输出
/// （CWE-532）。`url` 复用连接层已有的 [`redact_db_url`]，不另写第二份脱敏逻辑；
/// 空串保持原样输出，避免丢掉"没有配置整串 URL"这一可读状态。
impl std::fmt::Debug for DatabaseConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DatabaseConfig")
            .field("engine", &self.engine)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username)
            .field("password", &"[REDACTED]")
            .field("database", &self.database)
            .field("url", &redact_optional_url(&self.url))
            .field("max_connections", &self.max_connections)
            .field("min_connections", &self.min_connections)
            .field("acquire_timeout_seconds", &self.acquire_timeout_seconds)
            .field("idle_timeout_seconds", &self.idle_timeout_seconds)
            .field("statement_timeout_secs", &self.statement_timeout_secs)
            .finish()
    }
}

/// 空串不参与脱敏（表示"未使用整串 URL"），其余交给连接层的 `redact_db_url`。
pub(crate) fn redact_optional_url(url: &str) -> String {
    if url.is_empty() {
        String::new()
    } else {
        crate::core::database::redact_db_url(url)
    }
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
                statement_timeout_secs: default_statement_timeout_secs(),
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
                statement_timeout_secs: default_statement_timeout_secs(),
            };
        }

        // SECURITY: 密码来自环境变量；缺失时置空而非 panic —— `Default` 不应因
        // 环境变量缺失而 panic（嵌入式 SDK 使用 `Config::default()` 且不连数据库）。
        // 服务端安全性不受弱化：`create_connection` 在建连时对空密码显性返回
        // `ConfigurationError`，忘设密码仍在启动时报错（时机从构造默认配置后移到建连）。
        let password = std::env::var("NEBULA_DATABASE_PASSWORD").unwrap_or_default();

        if password == "idgen123" || password.is_empty() {
            tracing::warn!(
                "{}",
                t!("log.core.config.app.weak_database_password_detected")
            );
        }

        Self {
            engine: DatabaseEngine::Postgresql,
            // url 必须留空：merge() 把"非默认字段"当作环境覆盖回写，预填的
            // localhost:5432 URL 会静默覆盖配置文件里 host/port 分离写法的
            // 端口。留空时 create_connection 按 host/port/username/password
            // 拼接 URL（显式 DATABASE_URL 仍走上方分支并被 merge 采纳）。
            url: String::new(),
            host: "localhost".to_string(),
            port: 5432,
            username: "idgen".to_string(),
            password,
            database: "idgen".to_string(),
            max_connections: 100,
            min_connections: 10,
            acquire_timeout_seconds: 30,
            idle_timeout_seconds: 300,
            statement_timeout_secs: default_statement_timeout_secs(),
        }
    }
}

/// etcd configuration for distributed coordination
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EtcdConfig {
    /// List of etcd endpoints
    pub endpoints: Vec<String>,
    /// Connection timeout (milliseconds)
    pub connect_timeout_ms: u64,
    /// Watch timeout (milliseconds)
    pub watch_timeout_ms: u64,
    /// 单次 etcd 操作超时（秒，T028）。`EtcdClientWrapper` 的各操作经
    /// `tokio::time::timeout` 包裹，防止 etcd 挂起阻塞协调路径。默认 3 秒。
    #[serde(default = "default_operation_timeout_secs")]
    pub operation_timeout_secs: u64,
}

/// T028 —— `operation_timeout_secs` 的 serde 默认值（与客户端内置默认一致）。
fn default_operation_timeout_secs() -> u64 {
    3
}

impl Default for EtcdConfig {
    fn default() -> Self {
        Self {
            endpoints: vec!["etcd:2379".to_string()],
            connect_timeout_ms: 5000,
            watch_timeout_ms: 5000,
            operation_timeout_secs: default_operation_timeout_secs(),
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

    // ----- NebulaIdConfig Default + http_addr / grpc_addr -----

    #[test]
    fn test_app_config_default_values() {
        let cfg = NebulaIdConfig::default();
        assert_eq!(cfg.name, "nebula-id");
        assert_eq!(cfg.host, "0.0.0.0");
        assert_eq!(cfg.http_port, 8080);
        assert_eq!(cfg.grpc_port, 9091);
        assert_eq!(cfg.dc_id, 0);
        assert_eq!(cfg.worker_id, 0);
    }

    #[test]
    fn test_app_config_http_addr_success() {
        let cfg = NebulaIdConfig::default();
        let addr = cfg.http_addr().expect("default http_addr should parse");
        assert_eq!(addr.port(), 8080);
        assert_eq!(addr.ip().to_string(), "0.0.0.0");
    }

    #[test]
    fn test_app_config_grpc_addr_success() {
        let cfg = NebulaIdConfig::default();
        let addr = cfg.grpc_addr().expect("default grpc_addr should parse");
        assert_eq!(addr.port(), 9091);
        assert_eq!(addr.ip().to_string(), "0.0.0.0");
    }

    #[test]
    fn test_app_config_http_addr_invalid_host_returns_error() {
        // 不合法的 host（带空格）→ 解析失败
        let cfg = NebulaIdConfig {
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
        let cfg = NebulaIdConfig {
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
        let cfg = NebulaIdConfig {
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
        let cfg = NebulaIdConfig {
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

    // ----- T035: NebulaIdConfig.locale -----

    #[test]
    fn test_app_config_locale_default_is_auto_empty() {
        let cfg = NebulaIdConfig::default();
        assert_eq!(
            cfg.locale, "",
            "默认 locale 必须为空串(auto:跟随系统检测链)"
        );
    }

    #[test]
    fn test_app_config_locale_missing_in_toml_defaults_to_auto() {
        // 既有部署的 [app] 段没有 locale 键 → serde default 兜底空串(auto)
        let cfg: NebulaIdConfig = toml::from_str(
            r#"
name = "nebula-id"
host = "0.0.0.0"
http_port = 8080
grpc_port = 9091
dc_id = 0
worker_id = 0
"#,
        )
        .expect("[app] 段缺 locale 键必须可解析");
        assert_eq!(cfg.locale, "");
    }

    #[test]
    fn test_app_config_locale_explicit_value_roundtrips() {
        let cfg: NebulaIdConfig = toml::from_str(
            r#"
name = "nebula-id"
host = "0.0.0.0"
http_port = 8080
grpc_port = 9091
dc_id = 0
worker_id = 0
locale = "zh-CN"
"#,
        )
        .expect("显式 locale 必须可解析");
        assert_eq!(cfg.locale, "zh-CN");
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

    #[test]
    fn test_redact_optional_url() {
        assert_eq!(redact_optional_url(""), "");
        assert_eq!(
            redact_optional_url("postgresql://u:p@localhost:5432/db"),
            "postgresql://u:***@localhost:5432/db"
        );
        assert_eq!(redact_optional_url("not a url"), "<redacted>");
    }
}
