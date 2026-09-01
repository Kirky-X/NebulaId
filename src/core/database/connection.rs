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

use dbnexus::sea_orm::{
    ConnectOptions, ConnectionTrait, Database, DatabaseConnection, DbBackend, DbErr, Statement,
};
use tracing::{info, warn};

use crate::core::config::DatabaseConfig;
use crate::core::types::CoreError;

/// Schema name for Nebula ID tables
pub const NEBULA_SCHEMA: &str = "nebula_id";

/// Redact the password component of a database connection URL before
/// the URL is written to logs or other observability surfaces.
///
/// Phase 9 T043 (CRITICAL C1 / tiangang HIGH-2) — `final_url` carries
/// the plaintext database password (e.g. `postgresql://user:pass@host/db`)
/// and must never be recorded verbatim. Only `scheme://user@host:port/db`
/// is emitted; the password is replaced by `***`. If the URL cannot be
/// parsed by `url::Url`, the entire string is replaced by `<redacted>`
/// to guarantee no password can leak.
pub(crate) fn redact_db_url(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(mut parsed) => {
            let _ = parsed.set_password(Some("***"));
            parsed.to_string()
        }
        Err(_) => "<redacted>".to_string(),
    }
}

impl From<DbErr> for CoreError {
    fn from(e: DbErr) -> Self {
        CoreError::DatabaseError(e.to_string())
    }
}

pub async fn create_connection(config: &DatabaseConfig) -> Result<DatabaseConnection, CoreError> {
    // Check if we are using a complete connection string URL
    let use_full_url = config.url.starts_with("postgresql://")
        || config.url.starts_with("mysql://")
        || config.url.starts_with("sqlite://")
        || config.url.starts_with("postgres://");

    // Validate database password only if not using full URL and not Sqlite
    if !use_full_url && config.engine != crate::core::config::DatabaseEngine::Sqlite {
        if config.password.is_empty() || config.password.contains("${") {
            return Err(CoreError::ConfigurationError(
                "Database password not configured. Set NEBULA_DATABASE_PASSWORD environment variable".to_string()
            ));
        }
        if config.username.is_empty() {
            return Err(CoreError::ConfigurationError(
                "Database username not configured".to_string(),
            ));
        }
    }

    // Use URL if it's a complete connection string, otherwise construct from parts
    let final_url = if use_full_url {
        config.url.clone()
    } else {
        match config.engine {
            crate::core::config::DatabaseEngine::Postgresql
            | crate::core::config::DatabaseEngine::Postgres => {
                format!(
                    "postgresql://{}:{}@{}:{}/{}",
                    config.username, config.password, config.host, config.port, config.database
                )
            }
            crate::core::config::DatabaseEngine::Mysql => {
                format!(
                    "mysql://{}:{}@{}:{}/{}",
                    config.username, config.password, config.host, config.port, config.database
                )
            }
            crate::core::config::DatabaseEngine::Sqlite => config.database.clone(),
        }
    };

    let mut connect_options = ConnectOptions::new(final_url.clone());

    if config.engine != crate::core::config::DatabaseEngine::Sqlite {
        connect_options
            .max_connections(config.max_connections)
            .min_connections(config.min_connections)
            .connect_timeout(std::time::Duration::from_secs(
                config.acquire_timeout_seconds,
            ))
            .idle_timeout(std::time::Duration::from_secs(config.idle_timeout_seconds));
    }

    info!(
        "{}",
        t!(
            "log.core.database.connection.connecting",
            engine = config.engine,
            url = redact_db_url(&final_url)
        )
    );

    let db = Database::connect(connect_options)
        .await
        .map_err(CoreError::from)?;

    info!(
        "{}",
        t!("log.core.database.connection.established_successfully")
    );

    Ok(db)
}

/// 幂等地给 `nebula_id.api_keys` 补齐宽限期两列（存量库升级路径）。
///
/// 新库由 `run_migrations` 里的 `CREATE TABLE` 直接带列，本语句对它们是 no-op
/// （`ADD COLUMN IF NOT EXISTS`）。两列都可空，因此不需要回填存量行。
///
/// 抽成独立函数是测试接缝：sea-orm 2.0 的 `MockDatabase` 不记录
/// `execute_unprepared` 的 SQL，只有把语句构造分离出来，才能对列名与幂等子句断言。
pub(crate) fn api_keys_grace_period_alter_sql() -> String {
    format!(
        r#"ALTER TABLE {}.api_keys ADD COLUMN IF NOT EXISTS prev_secret_hash VARCHAR(128), ADD COLUMN IF NOT EXISTS rotate_expires_at TIMESTAMP"#,
        NEBULA_SCHEMA
    )
}

/// Auto-create schema and tables for Nebula ID
pub async fn run_migrations(db: &DatabaseConnection) -> Result<(), CoreError> {
    info!("{}", t!("log.core.database.connection.running_migrations"));

    // Create schema if not exists (only for PostgreSQL)
    let create_schema_sql = format!(r#"CREATE SCHEMA IF NOT EXISTS {}"#, NEBULA_SCHEMA);
    // sea-orm 2.0: execute() 要求 StatementBuilder trait，原始 SQL 用 execute_unprepared
    match db.execute_unprepared(&create_schema_sql).await {
        Ok(_) => info!(
            "{}",
            t!(
                "log.core.database.connection.schema_created_verified",
                schema = NEBULA_SCHEMA
            )
        ),
        Err(e) => {
            warn!(
                "{}",
                t!(
                    "log.core.database.connection.schema_create_failed",
                    error = e
                )
            );
        }
    }

    // Define all tables with their CREATE statements
    // All tables are created in the nebula_id schema
    let tables = vec![
        // API Keys table
        format!(
            r#"
        CREATE TABLE IF NOT EXISTS {}.api_keys (
            id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
            key_id VARCHAR(64) NOT NULL UNIQUE,
            key_secret_hash VARCHAR(128) NOT NULL,
            prev_secret_hash VARCHAR(128),
            rotate_expires_at TIMESTAMP,
            key_prefix VARCHAR(16) NOT NULL,
            role VARCHAR(20) NOT NULL DEFAULT 'user',
            workspace_id UUID,  -- 允许 NULL，用于全局 admin key
            name VARCHAR(255) NOT NULL,
            description TEXT,
            rate_limit INT DEFAULT 1000,
            enabled BOOLEAN DEFAULT true,
            expires_at TIMESTAMP,
            last_used_at TIMESTAMP,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            CONSTRAINT check_admin_key CHECK (
                (workspace_id IS NULL AND role = 'admin')
                OR (workspace_id IS NOT NULL AND role != 'admin')
            )
        )
        "#,
            NEBULA_SCHEMA
        ),
        // Workspaces table
        format!(
            r#"
        CREATE TABLE IF NOT EXISTS {}.workspaces (
            id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
            name VARCHAR(255) NOT NULL UNIQUE,
            description TEXT,
            status VARCHAR(20) DEFAULT 'active',
            max_groups INT DEFAULT 100,
            max_biz_tags INT DEFAULT 1000,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        )
        "#,
            NEBULA_SCHEMA
        ),
        // Groups table
        format!(
            r#"
        CREATE TABLE IF NOT EXISTS {}.groups (
            id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
            workspace_id UUID NOT NULL REFERENCES {}.workspaces(id) ON DELETE CASCADE,
            name VARCHAR(255) NOT NULL,
            description TEXT,
            max_biz_tags INT DEFAULT 100,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            UNIQUE(workspace_id, name)
        )
        "#,
            NEBULA_SCHEMA, NEBULA_SCHEMA
        ),
        // BizTags table
        format!(
            r#"
        CREATE TABLE IF NOT EXISTS {}.biz_tags (
            id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
            workspace_id UUID NOT NULL REFERENCES {}.workspaces(id) ON DELETE CASCADE,
            group_id UUID NOT NULL REFERENCES {}.groups(id) ON DELETE CASCADE,
            name VARCHAR(255) NOT NULL,
            description TEXT,
            algorithm VARCHAR(20) DEFAULT 'segment',
            format VARCHAR(20) DEFAULT 'numeric',
            prefix VARCHAR(50) DEFAULT '',
            base_step INT DEFAULT 1000,
            max_step INT DEFAULT 100000,
            datacenter_ids TEXT DEFAULT '[]',
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            UNIQUE(workspace_id, group_id, name)
        )
        "#,
            NEBULA_SCHEMA, NEBULA_SCHEMA, NEBULA_SCHEMA
        ),
        // Nebula segments table
        format!(
            r#"
        CREATE TABLE IF NOT EXISTS {}.nebula_segments (
            id BIGSERIAL PRIMARY KEY,
            workspace_id VARCHAR(255) NOT NULL,
            biz_tag VARCHAR(255) NOT NULL,
            current_id BIGINT NOT NULL,
            max_id BIGINT NOT NULL,
            step INT NOT NULL DEFAULT 1000,
            delta INT NOT NULL DEFAULT 1,
            dc_id INT NOT NULL DEFAULT 0,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        )
        "#,
            NEBULA_SCHEMA
        ),
    ];

    for sql in tables {
        // sea-orm 2.0: execute() 要求 StatementBuilder trait，原始 SQL 用 execute_unprepared
        match db.execute_unprepared(&sql).await {
            Ok(_) => {
                let table_name = sql.split_whitespace().nth(4).unwrap_or("").replace('(', "");
                info!(
                    "{}",
                    t!(
                        "log.core.database.connection.table_created_verified",
                        table_name = table_name
                    )
                );
            }
            Err(e) => {
                let error_msg = e.to_string();
                if error_msg.contains("already exists") || error_msg.contains("duplicate") {
                    info!(
                        "{}",
                        t!("log.core.database.connection.table_already_exists")
                    );
                } else {
                    // MEDIUM-2 修复（CWE-209）：不将 SeaORM 原始错误消息嵌入
                    // CoreError::DatabaseError（可能含 schema/表名/字段名/SQL 片段）。
                    // 完整错误通过 tracing::error! 记录到服务端日志，
                    // 返回给上层的是通用消息（helpers.rs 仍会进一步净化）。
                    tracing::error!(
                        event = "db_create_table_failed",
                        error = %error_msg,
                        "database table creation failed"
                    );
                    return Err(CoreError::DatabaseError(
                        "Failed to create table (see server logs for details)".to_string(),
                    ));
                }
            }
        }
    }

    // 宽限期两列的存量库迁移：新库的 CREATE TABLE 已带这两列，本语句对它们是 no-op。
    // 失败必须终止启动 —— 缺列会让 validate_api_key 读不到宽限期而静默放行旧凭证。
    match db
        .execute_unprepared(&api_keys_grace_period_alter_sql())
        .await
    {
        Ok(_) => tracing::info!(
            event = "api_keys_grace_columns_ensured",
            table = "api_keys",
            "rotated-credential grace period columns ensured"
        ),
        Err(e) => {
            let error_msg = e.to_string();
            if error_msg.contains("already exists") || error_msg.contains("duplicate") {
                info!(
                    "{}",
                    t!("log.core.database.connection.table_already_exists")
                );
            } else {
                tracing::error!(
                    event = "db_alter_table_failed",
                    error = %error_msg,
                    "database table alteration failed"
                );
                return Err(CoreError::DatabaseError(
                    "Failed to alter table (see server logs for details)".to_string(),
                ));
            }
        }
    }

    info!(
        "{}",
        t!("log.core.database.connection.migrations_completed")
    );
    Ok(())
}

#[allow(dead_code)]
pub struct DatabaseManager {
    db: DatabaseConnection,
}

#[allow(dead_code)]
impl DatabaseManager {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    pub fn get_connection(&self) -> &DatabaseConnection {
        &self.db
    }

    pub fn into_connection(self) -> DatabaseConnection {
        self.db
    }

    pub async fn health_check(&self) -> Result<(), CoreError> {
        self.db.ping().await.map_err(CoreError::from)?;
        Ok(())
    }

    pub async fn close(self) -> Result<(), CoreError> {
        drop(self.db);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::{DatabaseConfig, DatabaseEngine};
    use dbnexus::sea_orm::{DatabaseBackend, DbErr, MockDatabase, MockExecResult, RuntimeErr};

    #[cfg(feature = "sqlite")]
    #[tokio::test]
    async fn test_sqlite_connection() {
        let config = DatabaseConfig {
            engine: DatabaseEngine::Sqlite,
            url: "sqlite::memory:".to_string(),
            host: "".to_string(),
            port: 0,
            username: "".to_string(),
            password: "".to_string(),
            database: "sqlite::memory:".to_string(),
            max_connections: 10,
            min_connections: 1,
            acquire_timeout_seconds: 30,
            idle_timeout_seconds: 300,
        };

        let conn = create_connection(&config).await;
        assert!(conn.is_ok());
    }

    // ===== redact_db_url (private helper) =====

    #[test]
    fn test_redact_db_url_masks_password_with_stars() {
        let url = "postgresql://user:secret@localhost:5432/db";
        let redacted = redact_db_url(url);
        assert!(
            !redacted.contains("secret"),
            "redacted URL must not contain original password: {redacted}"
        );
        assert!(
            redacted.contains("***"),
            "redacted URL should contain placeholder, got: {redacted}"
        );
    }

    #[test]
    fn test_redact_db_url_returns_redacted_for_unparseable_input() {
        let url = "not_a_valid_url";
        let redacted = redact_db_url(url);
        assert_eq!(
            redacted, "<redacted>",
            "unparseable URL should be fully redacted"
        );
    }

    #[test]
    fn test_redact_db_url_handles_url_without_password() {
        // set_password(Some("***")) is called even when no password was set,
        // so the result becomes user:***@host. Verify host is preserved and
        // no panic occurs.
        let url = "postgresql://user@localhost:5432/db";
        let redacted = redact_db_url(url);
        assert!(
            redacted.contains("localhost"),
            "redacted URL should preserve host, got: {redacted}"
        );
        assert!(
            redacted.contains("user"),
            "redacted URL should preserve username, got: {redacted}"
        );
    }

    // ===== From<DbErr> for CoreError =====

    #[test]
    fn test_from_dberr_produces_database_error_with_original_message() {
        let db_err = DbErr::Query(RuntimeErr::Internal("query blew up".to_string()));
        let core_err: CoreError = db_err.into();
        match core_err {
            CoreError::DatabaseError(msg) => {
                assert!(
                    msg.contains("query blew up"),
                    "DatabaseError should embed original DbErr text, got: {msg}"
                );
            }
            other => panic!("expected CoreError::DatabaseError, got {other:?}"),
        }
    }

    // ===== create_connection validation paths =====

    fn make_pg_config(password: &str, username: &str) -> DatabaseConfig {
        DatabaseConfig {
            engine: DatabaseEngine::Postgresql,
            url: String::new(),
            host: "localhost".to_string(),
            port: 5432,
            username: username.to_string(),
            password: password.to_string(),
            database: "test_db".to_string(),
            max_connections: 10,
            min_connections: 1,
            acquire_timeout_seconds: 5,
            idle_timeout_seconds: 300,
        }
    }

    #[tokio::test]
    async fn test_create_connection_rejects_empty_password_for_postgres() {
        let config = make_pg_config("", "user");
        let result = create_connection(&config).await;
        match result {
            Err(CoreError::ConfigurationError(msg)) => {
                assert!(
                    msg.contains("password"),
                    "should mention password, got: {msg}"
                );
                assert!(
                    msg.contains("NEBULA_DATABASE_PASSWORD"),
                    "should hint env var, got: {msg}"
                );
            }
            other => panic!("expected ConfigurationError for empty password, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_create_connection_rejects_unsubstituted_placeholder_password() {
        let config = make_pg_config("${DB_PASSWORD}", "user");
        let result = create_connection(&config).await;
        match result {
            Err(CoreError::ConfigurationError(msg)) => {
                assert!(
                    msg.contains("password"),
                    "should mention password, got: {msg}"
                );
            }
            other => panic!("expected ConfigurationError for ${{...}} password, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_create_connection_rejects_empty_username_for_postgres() {
        let config = make_pg_config("real_password", "");
        let result = create_connection(&config).await;
        match result {
            Err(CoreError::ConfigurationError(msg)) => {
                assert!(
                    msg.contains("username"),
                    "should mention username, got: {msg}"
                );
            }
            other => panic!("expected ConfigurationError for empty username, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_create_connection_skips_validation_when_full_url_provided() {
        // Full URL bypasses empty-password/username checks. With
        // min_connections=0, sqlx-postgres creates a lazy pool without
        // actually establishing a connection, so Database::connect returns
        // Ok immediately. Either Ok or DatabaseError is acceptable; the
        // meaningful property is that ConfigurationError is NOT returned.
        let config = DatabaseConfig {
            engine: DatabaseEngine::Postgresql,
            url: "postgresql://user:pass@127.0.0.1:1/db".to_string(),
            host: String::new(),
            port: 0,
            username: String::new(),
            password: String::new(),
            database: String::new(),
            max_connections: 1,
            min_connections: 0,
            acquire_timeout_seconds: 1,
            idle_timeout_seconds: 1,
        };
        let result = create_connection(&config).await;
        match result {
            Ok(_db) => { /* lazy pool created, success path covered */ }
            Err(CoreError::DatabaseError(_msg)) => { /* connect failed, error path covered */ }
            Err(other) => panic!(
                "expected Ok or DatabaseError (validation should be skipped for full URL), got {other:?}"
            ),
        }
    }

    // ===== DatabaseManager with MockDatabase =====

    #[tokio::test]
    async fn test_database_manager_new_stores_connection() {
        let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();
        let manager = DatabaseManager::new(db);
        let _conn = manager.get_connection();
    }

    #[tokio::test]
    async fn test_database_manager_into_connection_consumes_self() {
        let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();
        let manager = DatabaseManager::new(db);
        let _conn = manager.into_connection();
    }

    #[tokio::test]
    async fn test_database_manager_health_check_returns_ok_with_mock() {
        // ConnectionTrait::ping() issues a SELECT 1 query, which consumes
        // one exec result from the mock queue.
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_exec_results(vec![MockExecResult {
                last_insert_id: 0,
                rows_affected: 0,
            }])
            .into_connection();
        let manager = DatabaseManager::new(db);
        let result = manager.health_check().await;
        assert!(
            result.is_ok(),
            "health_check should succeed with mock db: {result:?}"
        );
    }

    #[tokio::test]
    async fn test_database_manager_close_returns_ok_and_consumes_self() {
        let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();
        let manager = DatabaseManager::new(db);
        let result = manager.close().await;
        assert!(result.is_ok(), "close should return Ok: {result:?}");
    }

    // ===== run_migrations with MockDatabase =====

    /// n 个"执行成功"桩结果（每个 execute 消费一个）。
    fn ok_exec_results(n: usize) -> Vec<MockExecResult> {
        (0..n)
            .map(|_| MockExecResult {
                last_insert_id: 0,
                rows_affected: 0,
            })
            .collect()
    }

    fn mock_db_with_n_ok(n: usize) -> dbnexus::sea_orm::DatabaseConnection {
        MockDatabase::new(DatabaseBackend::Postgres)
            .append_exec_results(ok_exec_results(n))
            .into_connection()
    }

    #[tokio::test]
    async fn test_run_migrations_succeeds_when_all_executes_succeed() {
        // 1 schema + 5 tables + 1 grace-column ALTER = 7 successful executes.
        let db = mock_db_with_n_ok(7);
        let result = run_migrations(&db).await;
        assert!(
            result.is_ok(),
            "migrations should succeed when all execs succeed: {result:?}"
        );
    }

    #[tokio::test]
    async fn test_run_migrations_logs_warn_but_continues_when_schema_create_fails() {
        // Schema create fails (logged as warn), tables + grace-column ALTER still succeed.
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_exec_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "schema creation permission denied".to_string(),
            ))])
            .append_exec_results(ok_exec_results(6))
            .into_connection();
        let result = run_migrations(&db).await;
        assert!(
            result.is_ok(),
            "schema create failure should be logged but not propagate Err: {result:?}"
        );
    }

    #[tokio::test]
    async fn test_run_migrations_returns_database_error_when_table_create_fails() {
        // Schema + first table succeed, second table fails with non-"already exists".
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_exec_results(vec![
                MockExecResult {
                    last_insert_id: 0,
                    rows_affected: 0,
                },
                MockExecResult {
                    last_insert_id: 0,
                    rows_affected: 0,
                },
            ])
            .append_exec_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "syntax error near 'FOO'".to_string(),
            ))])
            .into_connection();
        let result = run_migrations(&db).await;
        match result {
            Err(CoreError::DatabaseError(msg)) => {
                assert!(
                    msg.contains("Failed to create table"),
                    "should return generic message, got: {msg}"
                );
                assert!(
                    !msg.contains("syntax error"),
                    "should NOT leak SQL error details (CWE-209), got: {msg}"
                );
            }
            other => panic!("expected DatabaseError for table create failure, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_run_migrations_treats_already_exists_as_info_not_error() {
        // Schema + first table succeed, second table fails with "already exists",
        // remaining tables + the grace-column ALTER succeed.
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_exec_results(ok_exec_results(2))
            .append_exec_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "relation already exists".to_string(),
            ))])
            .append_exec_results(ok_exec_results(4))
            .into_connection();
        let result = run_migrations(&db).await;
        assert!(
            result.is_ok(),
            "'already exists' should be logged as info, not propagate Err: {result:?}"
        );
    }

    #[tokio::test]
    async fn test_run_migrations_treats_duplicate_as_info_not_error() {
        // Schema + 2 tables succeed, third table fails with "duplicate",
        // remaining table + the grace-column ALTER succeed.
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_exec_results(ok_exec_results(3))
            .append_exec_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "duplicate table name".to_string(),
            ))])
            .append_exec_results(ok_exec_results(3))
            .into_connection();
        let result = run_migrations(&db).await;
        assert!(
            result.is_ok(),
            "'duplicate' should be logged as info, not propagate Err: {result:?}"
        );
    }

    /// T006：宽限期两列的存量库迁移必须是幂等的 `ADD COLUMN IF NOT EXISTS`，
    /// 且两列可空（存量行无需回填）。
    #[test]
    fn test_run_migrations_emits_alter_table_for_grace_columns() {
        let sql = api_keys_grace_period_alter_sql();

        assert!(
            sql.contains(&format!("ALTER TABLE {}.api_keys", NEBULA_SCHEMA)),
            "ALTER must target the nebula_id api_keys table, got: {sql}"
        );
        assert!(
            sql.contains("ADD COLUMN IF NOT EXISTS prev_secret_hash VARCHAR(128)"),
            "prev_secret_hash must be added idempotently, got: {sql}"
        );
        assert!(
            sql.contains("ADD COLUMN IF NOT EXISTS rotate_expires_at TIMESTAMP"),
            "rotate_expires_at must be added idempotently, got: {sql}"
        );
        assert!(
            !sql.contains("NOT NULL"),
            "grace columns must stay nullable so existing rows need no backfill, got: {sql}"
        );
    }

    /// T006：ALTER 硬失败必须终止迁移 —— 缺列会让 `validate_api_key` 读不到宽限期，
    /// 从而静默按"无旧凭证"处理，属于必须响而不能兜底的故障。
    #[tokio::test]
    async fn test_run_migrations_returns_error_when_grace_column_alter_fails() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_exec_results(ok_exec_results(6))
            .append_exec_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "permission denied for table api_keys".to_string(),
            ))])
            .into_connection();

        let err = run_migrations(&db).await.unwrap_err();
        match err {
            CoreError::DatabaseError(msg) => {
                assert!(
                    msg.contains("Failed to alter table"),
                    "should report the alteration step, got: {msg}"
                );
                assert!(
                    !msg.contains("permission denied"),
                    "should NOT leak raw DB error details (CWE-209), got: {msg}"
                );
            }
            other => panic!("expected DatabaseError for ALTER failure, got {other:?}"),
        }
    }
}
