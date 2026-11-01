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
    /// [`NebulaIdConfig::worker_id`] 共同构成 Snowflake 机器标识，多实例必须显式配置
    /// （`DC_ID` 或配置文件），否则默认值 0 会使多实例生成重复 ID。
    pub dc_id: u8,
    /// Worker ID (0-255)
    ///
    /// 多实例约束：配置 etcd 时本值被运行时分配的 worker_id 覆盖；未配置 etcd 时为
    /// Snowflake 最终机器标识，多实例必须显式配置（`WORKER_ID` 或配置文件），否则
    /// 默认值 0 会使多实例生成重复 ID。
    pub worker_id: u8,
    /// Graceful shutdown timeout (seconds)
    #[serde(default = "default_shutdown_timeout_seconds")]
    pub shutdown_timeout_seconds: u64,
    /// Process default locale。空串(auto,serde 默认) = 未显式表达语言偏好，跟随系统
    /// 语言检测链(NEBULA_LOCALE → 本字段 → LC_ALL/LC_MESSAGES/LANG → sys-locale → en)；
    /// 显式写入 "en"/"zh-CN" 则钉死进程语言，**用户配置优先于系统语言**。
    /// 非法值在启动时告警并继续走链(链尾落 en)。
    #[serde(default = "default_locale")]
    pub locale: String,
}

fn default_shutdown_timeout_seconds() -> u64 {
    30
}

fn default_locale() -> String {
    // 默认空串 = auto：若默认 "en"，config.app.locale 会在检测链中恒短路，
    // 系统语言自动检测在默认部署下不可达。
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
    /// 单语句执行超时（秒）。仓储热查询（validate_api_key /
    /// allocate_segment 等）经 `tokio::time::timeout` 包裹，防止 DB 挂起拖死
    /// 生成与认证热路径。默认 5 秒。
    #[serde(default = "default_statement_timeout_secs")]
    pub statement_timeout_secs: u64,
}

/// `statement_timeout_secs` 的 serde 默认值（与仓储内置默认一致）。
fn default_statement_timeout_secs() -> u64 {
    5
}

/// 手写 `Debug`：`password` 是明文口令，`url` 可能以
/// `postgresql://user:pass@host/db` 形式内嵌口令，两者都不得进 `{:?}`（CWE-532）；
/// `url` 经本模块 [`redact_db_url`] 脱敏（`core::database` 复用同一实现），
/// 空串原样输出以保留"未配置整串 URL"的状态。
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

/// 空串不参与脱敏（表示"未使用整串 URL"），其余经本地 `redact_db_url`。
/// `redact_db_url` 定义在本模块（纯 url 解析，零 DB 依赖），保证
/// `--no-default-features` 下配置层仍可编译；`core::database` 复用本实现。
pub(crate) fn redact_db_url(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(mut parsed) => {
            let _ = parsed.set_password(Some("***"));
            parsed.to_string()
        }
        Err(_) => "<redacted>".to_string(),
    }
}

/// 空串不参与脱敏（表示"未使用整串 URL"），其余交给本地 `redact_db_url`。
pub(crate) fn redact_optional_url(url: &str) -> String {
    if url.is_empty() {
        String::new()
    } else {
        redact_db_url(url)
    }
}

/// endpoint 列表脱敏：逐项经 `redact_db_url` 剥离 userinfo。
/// etcd 端点可携带 `scheme://user:pass@host:port`，错误消息/日志直接打印
/// 会把凭据外泄；调用方（SDK 构建、主服务启动）统一走此入口。
///
/// cfg 门控跟随调用方：主服务侧仅 `not(etcd)` 的 fail-closed 分支引用它，
/// SDK 侧引用随 `sdk` 特性；两臂都不在（如 `--features etcd` 且无 sdk）时
/// 本函数编译即死代码——按使用面裁剪，而不是用 `allow(dead_code)` 掩盖。
/// `pub`（非 `pub(crate)`）：bin 侧 main.rs 的 fail-closed 分支同为本仓消费面，
/// lib/bin 是两个 crate，`pub(crate)` 对 bin 不可见（E0603）；cfg 门控保持使用面裁剪。
#[cfg(any(not(feature = "etcd"), feature = "sdk"))]
pub fn redact_endpoints(endpoints: &[String]) -> Vec<String> {
    endpoints.iter().map(|e| redact_db_url(e)).collect()
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        // DATABASE_URL 存在时口令内嵌于 URL，无需单独 password；用 `if let Ok(url)`
        // 只读取一次 env，避免原 `.is_ok`+`.unwrap` 之间的 TOCTOU race。
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

        // SECURITY: 该兜底仅用于测试（cfg!(test) 或 NEBULA_TEST_MODE），生产不适用。
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

        // SECURITY: 密码来自环境变量，缺失时置空而非 panic（`Default` 不应因环境
        // 缺失而 panic）；空密码在 `create_connection` 建连时显性返回
        // `ConfigurationError`，忘设密码仍在启动时报错。
        let password = std::env::var("NEBULA_DATABASE_PASSWORD").unwrap_or_default();

        if password == "idgen123" || password.is_empty() {
            tracing::warn!(
                "{}",
                t!("log.core.config.app.weak_database_password_detected")
            );
        }

        Self {
            engine: DatabaseEngine::Postgresql,
            // url 必须留空：merge 把非默认字段当环境覆盖回写，预填 localhost:5432 URL
            // 会静默覆盖配置文件的 host/port 分离写法的端口；留空时 create_connection
            // 按 host/port/username/password 拼 URL（显式 DATABASE_URL 走上分支）。
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
    /// 单次 etcd 操作超时（秒）。`EtcdClientWrapper` 各操作经
    /// `tokio::time::timeout` 包裹，防止 etcd 挂起阻塞协调路径。默认 3 秒。
    #[serde(default = "default_operation_timeout_secs")]
    pub operation_timeout_secs: u64,
}

/// `operation_timeout_secs` 的 serde 默认值（与客户端内置默认一致）。
fn default_operation_timeout_secs() -> u64 {
    3
}

impl EtcdConfig {
    /// 是否存在可用 endpoint：`ETCD_ENDPOINTS=""` 与尾逗号写法会解析出空白项，
    /// 空白项等同「未配置」，装配层据此退回 `LocalDistributedLock`（单进程），
    /// 而不是拿空串去建连后 fail-closed 拒绝启动。
    pub fn has_configured_endpoints(&self) -> bool {
        self.endpoints.iter().any(|e| !e.trim().is_empty())
    }

    /// 建连用 endpoint 列表（去除空白项，保留其余取值原样）。
    pub fn configured_endpoints(&self) -> Vec<String> {
        self.endpoints
            .iter()
            .filter(|e| !e.trim().is_empty())
            .cloned()
            .collect()
    }
}

impl Default for EtcdConfig {
    fn default() -> Self {
        Self {
            // 内置默认必须能在裸机（无 etcd、无编排内网 DNS）启动：空列表 = 单进程
            // 本地锁（装配层 warn）。容器/多实例部署由 compose 显式注入
            // `ETCD_ENDPOINTS=etcd:2379`，仓库配置文件见 `config/config.toml`。
            endpoints: Vec::new(),
            connect_timeout_ms: 5000,
            watch_timeout_ms: 5000,
            operation_timeout_secs: default_operation_timeout_secs(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // DatabaseEngine Display

    #[test]
    fn test_database_engine_display_postgresql() {
        assert_eq!(DatabaseEngine::Postgresql.to_string(), "postgresql");
    }

    #[test]
    fn test_database_engine_display_postgres_alias() {
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

    // DatabaseEngine From<&str>

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
        let e: DatabaseEngine = "redis".into();
        assert_eq!(e, DatabaseEngine::Postgresql);
    }

    #[test]
    fn test_database_engine_from_str_case_insensitive() {
        let e: DatabaseEngine = "MySQL".into();
        assert_eq!(e, DatabaseEngine::Mysql);

        let e: DatabaseEngine = "POSTGRESQL".into();
        assert_eq!(e, DatabaseEngine::Postgresql);

        let e: DatabaseEngine = "SQLite".into();
        assert_eq!(e, DatabaseEngine::Sqlite);
    }

    // DatabaseEngine From<String>

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

    // NebulaIdConfig Default + http_addr / grpc_addr

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

    // EtcdConfig Default

    #[test]
    fn test_etcd_config_default_values() {
        let cfg = EtcdConfig::default();
        // 内置默认空列表 = 单进程本地锁：裸机（无 etcd/编排 DNS）必须能启动，
        // compose 部署由 ETCD_ENDPOINTS=etcd:2379 显式注入。
        assert!(cfg.endpoints.is_empty());
        assert_eq!(cfg.connect_timeout_ms, 5000);
        assert_eq!(cfg.watch_timeout_ms, 5000);
    }

    /// 空白项（`ETCD_ENDPOINTS=""`、尾逗号）不算已配置；建连列表同时剔除空白项。
    #[test]
    fn test_etcd_config_blank_endpoints_not_configured() {
        let blank = EtcdConfig {
            endpoints: vec![String::new()],
            ..Default::default()
        };
        assert!(!blank.has_configured_endpoints());
        assert!(blank.configured_endpoints().is_empty());

        let whitespace = EtcdConfig {
            endpoints: vec!["   ".to_string(), "\t".to_string()],
            ..Default::default()
        };
        assert!(!whitespace.has_configured_endpoints());
        assert!(whitespace.configured_endpoints().is_empty());

        let mixed = EtcdConfig {
            endpoints: vec!["http://127.0.0.1:2379".to_string(), String::new()],
            ..Default::default()
        };
        assert!(mixed.has_configured_endpoints());
        assert_eq!(
            mixed.configured_endpoints(),
            vec!["http://127.0.0.1:2379".to_string()]
        );
    }

    // NebulaIdConfig.locale

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

    // DatabaseConfig::default 在测试模式下应进入 sqlite 分支

    #[test]
    fn test_database_config_default_in_test_mode_uses_sqlite() {
        let cfg = DatabaseConfig::default();
        assert_eq!(cfg.engine, DatabaseEngine::Sqlite);
        assert_eq!(cfg.url, "sqlite::memory:");
        assert_eq!(cfg.max_connections, 10);
        assert_eq!(cfg.min_connections, 1);
        assert_eq!(cfg.acquire_timeout_seconds, 30);
        assert_eq!(cfg.idle_timeout_seconds, 300);
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
