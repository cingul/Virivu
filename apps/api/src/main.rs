mod auth;
mod config;
mod db;
mod models;
mod routes;

use anyhow::Context;
use tokio::net::TcpListener;
use tower_http::trace::TraceLayer;
use tracing::info;

use crate::{
    config::Config,
    db::Db,
    routes::{router, AppContext},
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    init_tracing();

    let config = Config::from_env();
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

    info!(
        app_name = %config.app_name,
        bind_address = %config.bind_address,
        database_url = %config.database_url,
        allowed_google_workspace_domain = %config.allowed_google_workspace_domain,
        allow_dev_auth_bypass = config.allow_dev_auth_bypass,
        "starting API server"
    );

    axum::serve(listener, app).await?;
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
