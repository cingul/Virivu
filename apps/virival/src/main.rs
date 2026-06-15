mod config;
mod error;
mod models;
mod routes;
mod state;
mod workflow;

use config::Config;
use state::AppState;
use tower_http::trace::TraceLayer;
use tracing::info;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG")
                .unwrap_or_else(|_| "virival_api=info,tower_http=info".to_string()),
        )
        .init();

    let config = Config::from_env();
    let state = AppState::default();
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
        "starting Virival API server"
    );

    if let Err(err) = axum::serve(listener, app).await {
        panic!("server error: {err}");
    }
}
