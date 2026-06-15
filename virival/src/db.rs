use deadpool_postgres::{Manager, ManagerConfig, Pool, RecyclingMethod};
use tokio_postgres::NoTls;

use crate::error::ApiError;

pub type DbPool = Pool;

pub fn create_pool(database_url: &str) -> Result<DbPool, ApiError> {
    let config: tokio_postgres::Config = database_url.parse().map_err(|err| {
        ApiError::Internal(format!(
            "invalid DATABASE_URL value for PostgreSQL connection: {err}"
        ))
    })?;
    let manager_config = ManagerConfig {
        recycling_method: RecyclingMethod::Fast,
    };
    let manager = Manager::from_config(config, NoTls, manager_config);
    Pool::builder(manager)
        .max_size(16)
        .build()
        .map_err(|err| ApiError::Internal(format!("failed to build PostgreSQL pool: {err}")))
}

pub async fn run_migrations(pool: &DbPool) -> Result<(), ApiError> {
    let client = pool
        .get()
        .await
        .map_err(|err| ApiError::Internal(format!("unable to acquire DB connection: {err}")))?;
    client
        .batch_execute(
            "CREATE TABLE IF NOT EXISTS virival_schema_migrations (
                version TEXT PRIMARY KEY,
                applied_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            )",
        )
        .await
        .map_err(|err| {
            ApiError::Internal(format!("failed to initialize migration table: {err}"))
        })?;

    apply_migration(
        &client,
        "0001_init",
        include_str!("../migrations/0001_init.sql"),
    )
    .await?;
    Ok(())
}

async fn apply_migration(
    client: &tokio_postgres::Client,
    version: &str,
    sql: &str,
) -> Result<(), ApiError> {
    let row = client
        .query_opt(
            "SELECT version FROM virival_schema_migrations WHERE version = $1",
            &[&version],
        )
        .await
        .map_err(|err| ApiError::Internal(format!("failed checking migration state: {err}")))?;
    if row.is_none() {
        client.batch_execute(sql).await.map_err(|err| {
            ApiError::Internal(format!("failed applying migration {version}: {err}"))
        })?;
        client
            .execute(
                "INSERT INTO virival_schema_migrations(version, applied_at) VALUES ($1, NOW())",
                &[&version],
            )
            .await
            .map_err(|err| {
                ApiError::Internal(format!("failed recording migration {version}: {err}"))
            })?;
    }
    Ok(())
}
