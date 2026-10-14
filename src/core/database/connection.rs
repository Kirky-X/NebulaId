// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

use dbnexus::sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, DbErr, Statement};
use tracing::{info, warn};

use crate::core::config::DatabaseConfig;
use crate::core::types::CoreError;

/// Nebula ID 表的 schema 名。
pub const NEBULA_SCHEMA: &str = "nebula_id";

/// 脱敏逻辑唯一定义在 `crate::core::config::app::redact_db_url`，本模块复用，
/// 避免第二份实现（config 层不能反向依赖 database）。
pub(crate) use crate::core::config::app::redact_db_url;

impl From<DbErr> for CoreError {
    fn from(e: DbErr) -> Self {
        // 第一层文案保持原样，底层 DbErr 经 ErrorSource 保链（错误链仅进服务端日志）。
        CoreError::DatabaseError(e.to_string(), Some(crate::core::types::ErrorSource::new(e)))
    }
}

/// 为 PostgreSQL URL 注入 `search_path = nebula_id, public`：
/// sea-orm 为枚举列生成的 `CAST(... AS algorithm_type)` 不带 schema 限定，
/// 默认 search_path 解析不到该类型；URL 已显式配置 `options` 时不覆盖。
pub(crate) fn ensure_pg_search_path(url: &str) -> String {
    let parsed = match url::Url::parse(url) {
        Ok(parsed) => parsed,
        Err(_) => return url.to_string(),
    };
    let is_postgres = matches!(parsed.scheme(), "postgres" | "postgresql");
    if !is_postgres {
        return url.to_string();
    }
    if parsed.query_pairs().any(|(key, _)| key == "options") {
        return url.to_string();
    }
    let mut rebuilt = parsed;
    rebuilt
        .query_pairs_mut()
        .append_pair("options", "-c search_path=nebula_id,public");
    rebuilt.to_string()
}

pub async fn create_connection(config: &DatabaseConfig) -> Result<DatabaseConnection, CoreError> {
    // 完整连接串 URL 判定
    let use_full_url = config.url.starts_with("postgresql://")
        || config.url.starts_with("mysql://")
        || config.url.starts_with("sqlite://")
        || config.url.starts_with("postgres://");

    // 非完整 URL 且非 Sqlite 时校验账号密码
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

    // 优先使用完整 URL，否则按字段拼接
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

    let final_url = ensure_pg_search_path(&final_url);

    // 连接经 dbnexus DbPool 建立（统一连接池 + 重试/故障转移/健康检查），返回底层
    // sea-orm 连接；PoolConfig 单位：idle_timeout 秒、acquire_timeout 毫秒。
    let defaults = dbnexus::PoolConfig::default();
    let (max_conn, min_conn, acquire_ms) =
        if config.engine == crate::core::config::DatabaseEngine::Sqlite {
            // Sqlite：不显式设置池参数，沿用 sea-orm 默认
            (
                defaults.max_connections,
                defaults.min_connections,
                defaults.acquire_timeout,
            )
        } else {
            (
                config.max_connections,
                config.min_connections,
                config.acquire_timeout_seconds * 1000,
            )
        };
    let db_config = dbnexus::DbConfig {
        url: final_url.clone(),
        pool_config: dbnexus::PoolConfig {
            max_connections: max_conn,
            min_connections: min_conn,
            idle_timeout: config.idle_timeout_seconds,
            acquire_timeout: acquire_ms,
        },
        ..dbnexus::DbConfig::default()
    };

    info!(
        "{}",
        t!(
            "log.core.database.connection.connecting",
            engine = config.engine,
            url = redact_db_url(&final_url)
        )
    );

    let pool = dbnexus::DbPool::with_config(db_config)
        .await
        .map_err(|e| CoreError::DatabaseError(e.to_string(), None))?;
    // DatabaseConnection 是 sqlx 池句柄，clone 后与会话共享同一连接池
    let session = dbnexus::ConnectionPool::get_session(&pool, "admin")
        .await
        .map_err(|e| CoreError::DatabaseError(e.to_string(), None))?;
    let db = session
        .connection()
        .map_err(|e| CoreError::DatabaseError(e.to_string(), None))?
        .clone();
    drop(session);

    info!(
        "{}",
        t!("log.core.database.connection.established_successfully")
    );

    Ok(db)
}

/// 幂等地给 `nebula_id.api_keys` 补齐宽限期两列（存量库升级路径）：
/// 新库 CREATE TABLE 已带列，本语句为 no-op；两列可空，无需回填存量行。
/// 抽成独立函数是测试接缝：MockDatabase 不记录 `execute_unprepared` 的 SQL。
pub(crate) fn api_keys_grace_period_alter_sql() -> String {
    format!(
        r#"ALTER TABLE {}.api_keys ADD COLUMN IF NOT EXISTS prev_secret_hash VARCHAR(128), ADD COLUMN IF NOT EXISTS rotate_expires_at TIMESTAMP"#,
        NEBULA_SCHEMA
    )
}

/// 幂等地给 `nebula_id.nebula_segments` 补齐号段唯一约束（存量库升级路径）。
/// `INSERT ... ON CONFLICT (workspace_id, biz_tag, dc_id)` 依赖该约束；PG 无
/// `ADD CONSTRAINT IF NOT EXISTS`，用 EXCEPTION 同时吞 42710 与 42P07。
pub(crate) fn nebula_segments_unique_constraint_sql() -> String {
    format!(
        r#"DO $$ BEGIN
            ALTER TABLE {}.nebula_segments
                ADD CONSTRAINT uq_nebula_segments_ws_tag_dc UNIQUE (workspace_id, biz_tag, dc_id);
        EXCEPTION
            WHEN duplicate_object OR duplicate_table THEN null;
        END $$"#,
        NEBULA_SCHEMA
    )
}

/// 自动创建 Nebula ID 的 schema 与数据表。
pub async fn run_migrations(db: &DatabaseConnection) -> Result<(), CoreError> {
    info!("{}", t!("log.core.database.connection.running_migrations"));

    // 创建 schema（仅 PostgreSQL）
    let create_schema_sql = format!(r#"CREATE SCHEMA IF NOT EXISTS {}"#, NEBULA_SCHEMA);
    // sea-orm 2.0：原始 SQL 须用 execute_unprepared（execute() 要求 StatementBuilder）
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

    // 实体枚举类型（sea-orm 对 `db_type = "Enum"` 生成不带 schema 限定的 CAST）必须真实存在。
    let enum_types = vec![
        (
            NEBULA_SCHEMA,
            r#"
        DO $$ BEGIN
            CREATE TYPE {}.algorithm_type AS ENUM ('segment', 'snowflake', 'uuid_v8');
        EXCEPTION
            WHEN duplicate_object THEN null;
        END $$"#,
        ),
        // algorithm_type 枚举扩容：每个 ADD VALUE 独立 DO 块，
        // 避免异常处理整块回卷导致 CREATE 成功后随 ADD 失败一并回滚。
        (
            NEBULA_SCHEMA,
            r#"
        DO $$ BEGIN
            ALTER TYPE {}.algorithm_type ADD VALUE 'uuid_v4';
        EXCEPTION
            WHEN duplicate_object THEN null;
        END $$"#,
        ),
        (
            NEBULA_SCHEMA,
            r#"
        DO $$ BEGIN
            ALTER TYPE {}.algorithm_type ADD VALUE 'uuid_v7';
        EXCEPTION
            WHEN duplicate_object THEN null;
        END $$"#,
        ),
        (
            NEBULA_SCHEMA,
            r#"
        DO $$ BEGIN
            CREATE TYPE {}.id_format AS ENUM ('numeric', 'prefixed', 'uuid', 'obfuscated');
        EXCEPTION
            WHEN duplicate_object THEN null;
        END $$"#,
        ),
    ];
    for (schema, sql) in &enum_types {
        let sql = sql.replace("{}", schema);
        if let Err(e) = db.execute_unprepared(&sql).await {
            tracing::error!(event = "db_create_enum_type_failed", error = %e, "enum type creation failed");
            return Err(CoreError::DatabaseError(
                "Failed to create enum type (see server logs for details)".to_string(),
                Some(crate::core::types::ErrorSource::new(e)),
            ));
        }
    }

    // 各表 CREATE 语句（全部建于 nebula_id schema）
    let tables = vec![
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
        format!(
            r#"
        CREATE TABLE IF NOT EXISTS {}.biz_tags (
            id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
            workspace_id UUID NOT NULL REFERENCES {}.workspaces(id) ON DELETE CASCADE,
            group_id UUID NOT NULL REFERENCES {}.groups(id) ON DELETE CASCADE,
            name VARCHAR(255) NOT NULL,
            description TEXT,
            algorithm {}.algorithm_type NOT NULL DEFAULT 'segment',
            format {}.id_format DEFAULT 'numeric',
            prefix VARCHAR(50) DEFAULT '',
            base_step INT DEFAULT 1000,
            max_step INT DEFAULT 100000,
            datacenter_ids JSONB DEFAULT '[]',
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            UNIQUE(workspace_id, group_id, name)
        )
        "#,
            NEBULA_SCHEMA, NEBULA_SCHEMA, NEBULA_SCHEMA, NEBULA_SCHEMA, NEBULA_SCHEMA
        ),
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
            -- 发号封顶（NULL = 不封顶）：与 scripts/init.sql 保持一致——
            -- 分配热路径 UPDATE 引用该列，缺列则号段分配开箱即坏。
            ceiling BIGINT,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            -- 号段原子分配（单语句 UPDATE ... RETURNING）依赖此约束：
            -- 1) INSERT ... ON CONFLICT (workspace_id, biz_tag, dc_id) DO NOTHING
            --    需要它匹配冲突目标；
            -- 2) 杜绝并发首分配插入重复行（否则 UPDATE ... RETURNING 会命中多行，
            --    区间可能交叠）。非 dc 变体按 dc_id = 0 读写，dc 变体按 dc_id 读写。
            CONSTRAINT uq_nebula_segments_ws_tag_dc UNIQUE (workspace_id, biz_tag, dc_id)
        )
        "#,
            NEBULA_SCHEMA
        ),
    ];

    for sql in tables {
        // sea-orm 2.0：原始 SQL 须用 execute_unprepared（execute() 要求 StatementBuilder）
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
                    // （CWE-209）不把 SeaORM 原始错误嵌入返回消息（可能含
                    // schema/表名/SQL 片段）；完整错误只记入服务端日志。
                    tracing::error!(
                        event = "db_create_table_failed",
                        error = %error_msg,
                        "database table creation failed"
                    );
                    return Err(CoreError::DatabaseError(
                        "Failed to create table (see server logs for details)".to_string(),
                        Some(crate::core::types::ErrorSource::new(e)),
                    ));
                }
            }
        }
    }

    // 号段唯一约束的存量库迁移：CREATE TABLE IF NOT EXISTS 对已存在表不生效，
    // 由本语句补齐；失败必须终止启动（缺约束则首分配 INSERT 直接报错）。
    match db
        .execute_unprepared(&nebula_segments_unique_constraint_sql())
        .await
    {
        Ok(_) => tracing::info!(
            event = "nebula_segments_unique_constraint_ensured",
            table = "nebula_segments",
            "segment first-allocation unique constraint ensured"
        ),
        Err(e) => {
            let error_msg = e.to_string();
            tracing::error!(
                event = "db_alter_table_failed",
                error = %error_msg,
                "nebula_segments unique constraint creation failed"
            );
            return Err(CoreError::DatabaseError(
                "Failed to alter table (see server logs for details)".to_string(),
                Some(crate::core::types::ErrorSource::new(e)),
            ));
        }
    }

    // 存量库幂等升级（与 scripts/migrations/001 对齐），必须在 CREATE TABLE 之后：
    // ALTER 的列级 IF NOT EXISTS 不豁免"表不存在"；含 id_format 'obfuscated' 与 ceiling 列补齐。
    let upgrade_sqls = [
        format!("ALTER TYPE {NEBULA_SCHEMA}.id_format ADD VALUE IF NOT EXISTS 'obfuscated'"),
        format!(
            "ALTER TABLE {NEBULA_SCHEMA}.nebula_segments ADD COLUMN IF NOT EXISTS ceiling BIGINT"
        ),
    ];
    for sql in &upgrade_sqls {
        if let Err(e) = db.execute_unprepared(sql).await {
            tracing::error!(
                event = "db_upgrade_statement_failed",
                error = %e,
                "{}",
                t!("log.core.database.connection.schema_upgrade_statement_failed")
            );
            return Err(CoreError::DatabaseError(
                t!("error.core.database.connection.schema_upgrade_failed"),
                Some(crate::core::types::ErrorSource::new(e)),
            ));
        }
    }

    // 宽限期两列迁移：新库已带列，本语句为 no-op；失败必须终止启动（缺列会静默放行旧凭证）。
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
                    Some(crate::core::types::ErrorSource::new(e)),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::{DatabaseConfig, DatabaseEngine};
    use dbnexus::sea_orm::{DatabaseBackend, DbErr, MockDatabase, MockExecResult, RuntimeErr};

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
        // 未设密码时 set_password(Some("***")) 仍会改写为 user:***@host；验证 host 保留且不 panic。
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
            CoreError::DatabaseError(msg, source) => {
                assert!(
                    msg.contains("query blew up"),
                    "DatabaseError should embed original DbErr text, got: {msg}"
                );
                // 底层 DbErr 必须经 source() 保链(错误链仅进服务端日志)
                let source = source.expect("From<DbErr> must preserve the source error");
                assert!(
                    source.to_string().contains("query blew up"),
                    "source chain must carry the DbErr text, got: {source}"
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
            statement_timeout_secs: 5,
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
        // 完整 URL 跳过账号密码校验；min_connections=0 时 sqlx-postgres 建懒池，
        // 立即返回 Ok。允许 Ok 或 DatabaseError，关键是不得返回 ConfigurationError。
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
            statement_timeout_secs: 5,
        };
        let result = create_connection(&config).await;
        match result {
            Ok(_db) => { /* lazy pool created, success path covered */ }
            Err(CoreError::DatabaseError(_msg, _)) => { /* connect failed, error path covered */ }
            Err(other) => panic!(
                "expected Ok or DatabaseError (validation should be skipped for full URL), got {other:?}"
            ),
        }
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
        // 1 schema + 4 enum types（2 CREATE + 2 ADD VALUE）+ 2 upgrade ALTERs
        //   + 5 tables + 1 constraint backfill + 1 grace-column ALTER = 14 次成功 execute。
        let db = mock_db_with_n_ok(14);
        let result = run_migrations(&db).await;
        assert!(
            result.is_ok(),
            "migrations should succeed when all execs succeed: {result:?}"
        );
    }

    #[tokio::test]
    async fn test_run_migrations_logs_warn_but_continues_when_schema_create_fails() {
        // schema 建失败仅 warn；其余 enum/ALTER/建表/回填/宽限期 ALTER 仍继续并成功。
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_exec_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "schema creation permission denied".to_string(),
            ))])
            .append_exec_results(ok_exec_results(13))
            .into_connection();
        let result = run_migrations(&db).await;
        assert!(
            result.is_ok(),
            "schema create failure should be logged but not propagate Err: {result:?}"
        );
    }

    #[tokio::test]
    async fn test_run_migrations_returns_database_error_when_table_create_fails() {
        // schema + 4 enum + 2 upgrade ALTER 成功，第一个建表以非 "already exists" 失败。
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_exec_results(ok_exec_results(7))
            .append_exec_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "syntax error near 'FOO'".to_string(),
            ))])
            .into_connection();
        let result = run_migrations(&db).await;
        match result {
            Err(CoreError::DatabaseError(msg, _)) => {
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
        // 第一表成功，第二表 "already exists"；其余表/回填/宽限期 ALTER 继续成功。
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_exec_results(ok_exec_results(8))
            .append_exec_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "relation already exists".to_string(),
            ))])
            .append_exec_results(ok_exec_results(8))
            .into_connection();
        let result = run_migrations(&db).await;
        assert!(
            result.is_ok(),
            "'already exists' should be logged as info, not propagate Err: {result:?}"
        );
    }

    #[tokio::test]
    async fn test_run_migrations_treats_duplicate_as_info_not_error() {
        // 前两表成功，第三表 "duplicate"；其余 2 表/回填/宽限期 ALTER 继续成功。
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_exec_results(ok_exec_results(9))
            .append_exec_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "duplicate table name".to_string(),
            ))])
            .append_exec_results(ok_exec_results(6))
            .into_connection();
        let result = run_migrations(&db).await;
        assert!(
            result.is_ok(),
            "'duplicate' should be logged as info, not propagate Err: {result:?}"
        );
    }

    /// 宽限期两列迁移必须是幂等 `ADD COLUMN IF NOT EXISTS`，且两列可空（存量行无需回填）。
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

    /// 号段唯一约束回填必须是幂等 DO 块（吞 `duplicate_object`/`duplicate_table`），
    /// 目标表与约束名必须正确 —— `INSERT ... ON CONFLICT` 依赖它。
    #[test]
    fn test_run_migrations_emits_unique_constraint_for_segments() {
        let sql = nebula_segments_unique_constraint_sql();

        assert!(
            sql.contains(&format!("ALTER TABLE {}.nebula_segments", NEBULA_SCHEMA)),
            "ALTER must target the nebula_id nebula_segments table, got: {sql}"
        );
        assert!(
            sql.contains(
                "ADD CONSTRAINT uq_nebula_segments_ws_tag_dc UNIQUE (workspace_id, biz_tag, dc_id)"
            ),
            "constraint name and columns must match init.sql / ON CONFLICT target, got: {sql}"
        );
        assert!(
            sql.contains("WHEN duplicate_object OR duplicate_table THEN null"),
            "backfill must be idempotent: 42710 duplicate_object（存量库重复约束）与 \
             42P07 duplicate_table（全新库建表内联约束的同名索引冲突）都要吞掉, got: {sql}"
        );
    }

    /// ALTER 硬失败必须终止迁移：缺列会让 `validate_api_key` 静默放行旧凭证。
    #[tokio::test]
    async fn test_run_migrations_returns_error_when_grace_column_alter_fails() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_exec_results(ok_exec_results(13))
            .append_exec_errors(vec![DbErr::Query(RuntimeErr::Internal(
                "permission denied for table api_keys".to_string(),
            ))])
            .into_connection();

        let err = run_migrations(&db).await.unwrap_err();
        match err {
            CoreError::DatabaseError(msg, _) => {
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

    #[tokio::test]
    async fn test_create_connection_mysql_url_branch_fails_to_connect() {
        let mut config = make_pg_config("secret", "user");
        config.engine = DatabaseEngine::Mysql;
        // 无 MySQL 服务：断言只能是连接失败（URL 拼装分支已执行）。
        assert!(create_connection(&config).await.is_err());
    }

    #[tokio::test]
    async fn test_create_connection_postgres_alias_url_branch() {
        let mut config = make_pg_config("secret", "user");
        config.engine = DatabaseEngine::Postgres;
        assert!(create_connection(&config).await.is_err());
    }

    #[tokio::test]
    async fn test_create_connection_full_url_skips_validation() {
        let mut config = make_pg_config("", "");
        config.url = "postgresql://user:pass@localhost:1/db".to_string();
        // 完整 URL 跳过账号密码校验，直接进入连接（端口 1 必定失败）。
        assert!(create_connection(&config).await.is_err());
    }
}
