use sea_orm::{ConnectOptions, Database, DatabaseConnection, DbErr};
use tracing::info;

use crate::config::DatabaseConfig;
use crate::types::CoreError;

impl From<DbErr> for CoreError {
    fn from(e: DbErr) -> Self {
        CoreError::DatabaseError(e.to_string())
    }
}

pub async fn create_connection(config: &DatabaseConfig) -> Result<DatabaseConnection, CoreError> {
    let connection_string = match config.engine.as_str() {
        "postgresql" => {
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
    };

    let mut connect_options = ConnectOptions::new(connection_string);

    if config.engine != "sqlite" {
        connect_options
            .max_connections(config.max_connections as u32)
            .min_connections(config.min_connections as u32)
            .connect_timeout(std::time::Duration::from_secs(
                config.acquire_timeout_seconds,
            ))
            .idle_timeout(std::time::Duration::from_secs(config.idle_timeout_seconds));
    }

    info!(
        "Connecting to {} database at {}:{}",
        config.engine, config.host, config.port
    );

    let db = Database::connect(connect_options)
        .await
        .map_err(CoreError::from)?;

    info!("Database connection established successfully");

    Ok(db)
}

pub async fn run_migrations(_db: &DatabaseConnection) -> Result<(), CoreError> {
    info!("Database migrations - migration system ready");
    Ok(())
}

pub struct DatabaseManager {
    db: DatabaseConnection,
}

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
