mod auth;
mod config;
mod db;
mod error;
mod media;
mod models;
mod oidc;
mod repository;
mod routes;
mod state;
mod workflow;

use config::Config;
use db::{create_pool, run_migrations};
use media::MediaUrlSigner;
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
    let media_signer = Arc::new(MediaUrlSigner::new(&config.media_signing_secret));
    let media_storage_root = Arc::new(std::path::PathBuf::from(&config.media_storage_root));
    if let Err(err) = std::fs::create_dir_all(media_storage_root.as_ref()) {
        panic!(
            "failed to create media storage root {}: {}",
            media_storage_root.display(),
            err
        );
    }
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
        media_signer,
        media_storage_root,
        media_signed_url_ttl_seconds: config.media_signed_url_ttl_seconds,
    };

    if config.run_mode == "reminder_worker" {
        info!(
            worker_poll_seconds = config.worker_poll_seconds,
            worker_batch_size = config.worker_batch_size,
            "starting Virival reminder worker mode"
        );
        run_reminder_worker(
            state.repository.clone(),
            config.worker_poll_seconds,
            config.worker_batch_size,
        )
        .await;
        return;
    }

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
        run_mode = %config.run_mode,
        media_storage_root = %config.media_storage_root,
        media_signed_url_ttl_seconds = config.media_signed_url_ttl_seconds,
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

async fn run_reminder_worker(
    repository: Arc<dyn repository::Repository>,
    poll_seconds: u64,
    batch_size: i64,
) {
    let sleep_duration = std::time::Duration::from_secs(poll_seconds.max(1));
    loop {
        match repository.process_due_reminder_jobs(batch_size).await {
            Ok(jobs) => {
                if jobs.is_empty() {
                    info!("reminder worker tick: no due jobs");
                } else {
                    info!(
                        processed_count = jobs.len(),
                        "reminder worker processed due jobs"
                    );
                }
            }
            Err(err) => {
                warn!(error = %err, "reminder worker failed processing due jobs");
            }
        }
        tokio::time::sleep(sleep_duration).await;
    }
}
