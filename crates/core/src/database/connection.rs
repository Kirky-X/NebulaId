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

impl From<DbErr> for CoreError {
    fn from(e: DbErr) -> Self {
        CoreError::DatabaseError(e.to_string())
    }
}

pub async fn create_connection(config: &DatabaseConfig) -> Result<DatabaseConnection, CoreError> {
    // Use URL if it's a complete connection string, otherwise construct from parts
    let final_url = if config.url.starts_with("postgresql://")
        || config.url.starts_with("mysql://")
        || config.url.starts_with("sqlite://")
        || config.url.starts_with("postgres://")
    // Also support postgres://
    {
        config.url.clone()
    } else {
        match config.engine.as_str() {
            "postgresql" | "postgres" => {
                format!(
                    "postgresql://{}:{}@{}:{}/{}",
                    config.username, config.password, config.host, config.port, config.database
                )
            }
            "mysql" => {
                format!(
                    "mysql://{}:{}@{}:{}/{}",
                    config.username, config.password, config.host, config.port, config.database
                )
            }
            "sqlite" => config.database.clone(),
            _ => {
                return Err(CoreError::DatabaseError(format!(
                    "Unsupported database engine: {}",
                    config.engine
                )));
            }
        }
    };

    let mut connect_options = ConnectOptions::new(final_url.clone());

    if config.engine != "sqlite" {
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

/// Auto-create all tables based on SeaORM entities
pub async fn run_migrations(db: &DatabaseConnection) -> Result<(), CoreError> {
    info!("Running database migrations...");

    // Define all tables with their CREATE statements
    let tables = vec![
        // Workspaces table
        r#"
        CREATE TABLE IF NOT EXISTS workspaces (
            id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
            name VARCHAR(255) NOT NULL UNIQUE,
            description TEXT,
            status VARCHAR(20) DEFAULT 'active',
            max_groups INT DEFAULT 100,
            max_biz_tags INT DEFAULT 1000,
            created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
            updated_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
        )
        "#,
        // Groups table
        r#"
        CREATE TABLE IF NOT EXISTS groups (
            id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
            workspace_id UUID NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
            name VARCHAR(255) NOT NULL,
            description TEXT,
            max_biz_tags INT DEFAULT 100,
            created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
            updated_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
            UNIQUE(workspace_id, name)
        )
        "#,
        // BizTags table
        r#"
        CREATE TABLE IF NOT EXISTS biz_tags (
            id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
            workspace_id UUID NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
            group_id UUID NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
            name VARCHAR(255) NOT NULL,
            description TEXT,
            algorithm VARCHAR(20) DEFAULT 'segment',
            format VARCHAR(20) DEFAULT 'numeric',
            prefix VARCHAR(50) DEFAULT '',
            base_step INT DEFAULT 1000,
            max_step INT DEFAULT 100000,
            datacenter_ids TEXT DEFAULT '[]',
            created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
            updated_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
            UNIQUE(workspace_id, group_id, name)
        )
        "#,
        // Nebula segments table
        r#"
        CREATE TABLE IF NOT EXISTS nebula_segments (
            id BIGSERIAL PRIMARY KEY,
            workspace_id VARCHAR(255) NOT NULL,
            biz_tag VARCHAR(255) NOT NULL,
            current_id BIGINT NOT NULL,
            max_id BIGINT NOT NULL,
            step INT NOT NULL DEFAULT 1000,
            delta INT NOT NULL DEFAULT 1,
            dc_id INT NOT NULL DEFAULT 0,
            created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
            updated_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
        )
        "#,
    ];

    for sql in tables {
        let stmt = Statement::from_string(DbBackend::Postgres, sql);
        match db.execute(stmt).await {
            Ok(_) => info!("Table created/verified successfully"),
            Err(e) => {
                let error_msg = e.to_string();
                if error_msg.contains("already exists") || error_msg.contains("duplicate") {
                    info!("Table already exists, skipping");
                } else {
                    warn!("Migration warning: {}", error_msg);
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
    use crate::config::DatabaseConfig;

    #[cfg(feature = "sqlite")]
    #[tokio::test]
    async fn test_sqlite_connection() {
        let config = DatabaseConfig {
            engine: "sqlite".to_string(),
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
