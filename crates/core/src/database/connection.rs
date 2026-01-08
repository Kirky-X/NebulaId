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

use sea_orm::{
    ConnectOptions, ConnectionTrait, Database, DatabaseConnection, DbBackend, DbErr, Statement,
};
use tracing::{info, warn};

use crate::config::DatabaseConfig;
use crate::types::CoreError;

/// Schema name for Nebula ID tables
pub const NEBULA_SCHEMA: &str = "nebula_id";

impl From<DbErr> for CoreError {
    fn from(e: DbErr) -> Self {
        CoreError::DatabaseError(e.to_string())
    }
}

pub async fn create_connection(config: &DatabaseConfig) -> Result<DatabaseConnection, CoreError> {
    // Validate database password
    if config.engine != crate::config::DatabaseEngine::Sqlite {
        if config.password.is_empty() || config.password.contains("${") {
            return Err(CoreError::ConfigurationError(
                "Database password not configured. Set NEBULA_DATABASE_PASSWORD environment variable".to_string()
            ));
        }
        if config.username.is_empty() {
            return Err(CoreError::ConfigurationError(
                "Database username not configured".to_string()
            ));
        }
    }

    // Use URL if it's a complete connection string, otherwise construct from parts
    let final_url = if config.url.starts_with("postgresql://")
        || config.url.starts_with("mysql://")
        || config.url.starts_with("sqlite://")
        || config.url.starts_with("postgres://")
    {
        config.url.clone()
    } else {
        match config.engine {
            crate::config::DatabaseEngine::Postgresql | crate::config::DatabaseEngine::Postgres => {
                format!(
                    "postgresql://{}:{}@{}:{}/{}",
                    config.username, config.password, config.host, config.port, config.database
                )
            }
            crate::config::DatabaseEngine::Mysql => {
                format!(
                    "mysql://{}:{}@{}:{}/{}",
                    config.username, config.password, config.host, config.port, config.database
                )
            }
            crate::config::DatabaseEngine::Sqlite => config.database.clone(),
        }
    };

    let mut connect_options = ConnectOptions::new(final_url.clone());

    if config.engine != crate::config::DatabaseEngine::Sqlite {
        connect_options
            .max_connections(config.max_connections)
            .min_connections(config.min_connections)
            .connect_timeout(std::time::Duration::from_secs(
                config.acquire_timeout_seconds,
            ))
            .idle_timeout(std::time::Duration::from_secs(config.idle_timeout_seconds));
    }

    info!(
        "Connecting to {} database, URL: {}",
        config.engine, final_url
    );

    let db = Database::connect(connect_options)
        .await
        .map_err(CoreError::from)?;

    info!("Database connection established successfully");

    Ok(db)
}

/// Auto-create schema and tables for Nebula ID
pub async fn run_migrations(db: &DatabaseConnection) -> Result<(), CoreError> {
    info!("Running database migrations...");

    // Create schema if not exists (only for PostgreSQL)
    let create_schema_sql = format!(r#"CREATE SCHEMA IF NOT EXISTS {}"#, NEBULA_SCHEMA);
    let stmt = Statement::from_string(DbBackend::Postgres, &create_schema_sql);
    match db.execute(stmt).await {
        Ok(_) => info!("Schema '{}' created/verified", NEBULA_SCHEMA),
        Err(e) => {
            warn!("Could not create schema (may not be PostgreSQL): {}", e);
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
            CONSTRAINT check_admin_key CHECK (workspace_id IS NULL AND role = 'admin' OR workspace_id IS NOT NULL)
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
        let stmt = Statement::from_string(DbBackend::Postgres, &sql);
        match db.execute(stmt).await {
            Ok(_) => {
                let table_name = sql.split_whitespace().nth(4).unwrap_or("").replace('(', "");
                info!("Table created/verified: {}", table_name);
            }
            Err(e) => {
                let error_msg = e.to_string();
                if error_msg.contains("already exists") || error_msg.contains("duplicate") {
                    info!("Table already exists, skipping");
                } else {
                    return Err(CoreError::DatabaseError(format!(
                        "Failed to create table: {}",
                        error_msg
                    )));
                }
            }
        }
    }

    info!("Database migrations completed successfully");
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
    use crate::config::{DatabaseConfig, DatabaseEngine};

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
}
