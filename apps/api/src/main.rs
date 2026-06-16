mod auth;
mod config;
mod db;
mod error;
mod models;
mod routes;
mod study_queries_ui;
mod workflow;

use anyhow::Context;
use diesel::{pg::PgConnection, prelude::*, sql_query};
use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};
use tokio::net::TcpListener;
use tower_http::trace::TraceLayer;
use tracing::info;

use crate::{
    config::Config,
    db::Db,
    routes::{router, AppContext},
};

const MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations");
const MIGRATION_INIT_VARIANTS: &[&str] = &["2026-05-17-000001", "2026-05-17-000001_init"];
const MIGRATION_DUA_VARIANTS: &[&str] =
    &["2026-05-17-000003", "2026-05-17-000003_data_use_agreements"];

#[derive(diesel::deserialize::QueryableByName)]
struct ExistsRow {
    #[diesel(sql_type = diesel::sql_types::Bool)]
    exists: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    init_tracing();

    let config = Config::from_env();
    run_migrations(&config.database_url)
        .with_context(|| "failed to run startup migrations before API launch")?;
    let db = Db::connect(&config.database_url)
        .await
        .with_context(|| "failed to connect to Postgres database")?;

    let app = router(AppContext {
        config: config.clone(),
        db,
    })
    .layer(TraceLayer::new_for_http());

    let listener = TcpListener::bind(&config.bind_address)
        .await
        .with_context(|| format!("failed to bind to {}", config.bind_address))?;

    if config.allow_dev_auth_bypass {
        if config.allow_unsafe_dev_bypass {
            tracing::error!(
                "!!! DANGER: ALLOW_UNSAFE_DEV_BYPASS is enabled. \
                 Dev authentication bypass is allowed from any IP. \
                 This should NEVER be true in production or shared environments. !!!"
            );
        } else {
            tracing::warn!(
                "Dev auth bypass is enabled. It will only work from localhost \
                 unless ALLOW_UNSAFE_DEV_BYPASS is also set to true."
            );
        }
    }

    info!(
        app_name = %config.app_name,
        bind_address = %config.bind_address,
        app_base_url = %config.app_base_url,
        database_url = %config.database_url,
        allowed_google_workspace_domain = %config.allowed_google_workspace_domain,
        allow_dev_auth_bypass = config.allow_dev_auth_bypass,
        allow_unsafe_dev_bypass = config.allow_unsafe_dev_bypass,
        "starting API server"
    );

    // Build marker for cache debugging - if you see this in logs, the new binary is running
    info!("VIRIVU-BUILD-2026-05-28-SITE-FIX-4 - create-site defensive fallback + logging active");

    axum::serve(listener, app).await?;
    Ok(())
}

fn run_migrations(database_url: &str) -> anyhow::Result<()> {
    let mut connection = PgConnection::establish(database_url)
        .with_context(|| "failed to connect for startup migrations")?;
    bootstrap_legacy_schema_if_needed(&mut connection)?;
    let applied = connection
        .run_pending_migrations(MIGRATIONS)
        .map_err(|error| anyhow::anyhow!("failed running startup Diesel migrations: {error}"))?;
    if applied.is_empty() {
        info!("startup migrations: no pending migrations");
    } else {
        info!(
            applied_count = applied.len(),
            "startup migrations applied successfully"
        );
    }
    Ok(())
}

fn bootstrap_legacy_schema_if_needed(connection: &mut PgConnection) -> anyhow::Result<()> {
    sql_query(
        "CREATE TABLE IF NOT EXISTS __diesel_schema_migrations (
            version VARCHAR(50) PRIMARY KEY,
            run_on TIMESTAMP NOT NULL DEFAULT NOW()
        )",
    )
    .execute(connection)
    .context("failed creating Diesel migration tracking table")?;

    let has_organizations =
        sql_query("SELECT to_regclass('public.organizations') IS NOT NULL AS exists")
            .get_result::<ExistsRow>(connection)
            .context("failed checking organizations table existence")?
            .exists;
    if has_organizations {
        for version in MIGRATION_INIT_VARIANTS {
            sql_query(format!(
                "INSERT INTO __diesel_schema_migrations(version, run_on)
                 VALUES ('{}', NOW())
                 ON CONFLICT (version) DO NOTHING",
                version
            ))
            .execute(connection)
            .context("failed baselining init migration")?;
        }
    }

    let has_dua_tables =
        sql_query("SELECT to_regclass('public.data_use_agreements') IS NOT NULL AS exists")
            .get_result::<ExistsRow>(connection)
            .context("failed checking DUA table existence")?
            .exists;
    if has_dua_tables {
        for version in MIGRATION_DUA_VARIANTS {
            sql_query(format!(
                "INSERT INTO __diesel_schema_migrations(version, run_on)
                 VALUES ('{}', NOW())
                 ON CONFLICT (version) DO NOTHING",
                version
            ))
            .execute(connection)
            .context("failed baselining DUA migration")?;
        }
    }

    Ok(())
}

fn init_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "virivu_api=debug,tower_http=info,axum=info".into()),
        )
        .init();
}
