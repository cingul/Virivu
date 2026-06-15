mod auth;
mod config;
mod db;
mod error;
mod models;
mod oidc;
mod repository;
mod routes;
mod state;
mod workflow;

use config::Config;
use db::{create_pool, run_migrations};
use oidc::OidcVerifier;
use repository::PgRepository;
use state::AppState;
use std::sync::Arc;
use tower_http::trace::TraceLayer;
use tracing::{info, warn};

#[tokio::main]
async fn main() {
    let _ = dotenvy::dotenv();
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG")
                .unwrap_or_else(|_| "virival_api=info,tower_http=info".to_string()),
        )
        .init();

    let config = Config::from_env();
    let pool = create_pool(&config.database_url).unwrap_or_else(|err| {
        panic!("unable to initialize database pool: {err}");
    });
    if let Err(err) = run_migrations(&pool).await {
        panic!("failed running migrations: {err}");
    }
    let repository = Arc::new(PgRepository::new(pool));
    let oidc_verifier = Arc::new(OidcVerifier::new(
        config.google_client_id.clone(),
        config.google_workspace_domain.clone(),
        config.google_jwks_url.clone(),
        config.oidc_jwks_cache_seconds,
    ));
    let state = AppState {
        repository,
        oidc_verifier,
        allow_dev_auth_bypass: config.allow_dev_auth_bypass,
    };
    let app = routes::router(state, config.app_name.clone()).layer(TraceLayer::new_for_http());

    let listener = match tokio::net::TcpListener::bind(&config.bind_address).await {
        Ok(listener) => listener,
        Err(err) => {
            panic!(
                "failed to bind {} on {}: {}",
                config.app_name, config.bind_address, err
            );
        }
    };

    info!(
        app_name = %config.app_name,
        bind_address = %config.bind_address,
        database_url = %config.database_url,
        google_workspace_domain = ?config.google_workspace_domain,
        google_client_id_configured = config.google_client_id.is_some(),
        google_jwks_url = ?config.google_jwks_url,
        oidc_jwks_cache_seconds = config.oidc_jwks_cache_seconds,
        "starting Virival API server"
    );
    if config.allow_dev_auth_bypass {
        warn!(
            "dev auth bypass is enabled; provide x-virival-user and x-virival-role in production"
        );
    }

    if let Err(err) = axum::serve(listener, app).await {
        panic!("server error: {err}");
    }
}
