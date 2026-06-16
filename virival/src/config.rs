use std::env;

#[derive(Debug, Clone)]
pub struct Config {
    pub app_name: String,
    pub bind_address: String,
    pub database_url: String,
    pub allow_dev_auth_bypass: bool,
    pub google_workspace_domain: Option<String>,
    pub google_client_id: Option<String>,
    pub google_jwks_url: Option<String>,
    pub oidc_jwks_cache_seconds: u64,
    pub run_mode: String,
    pub worker_poll_seconds: u64,
    pub worker_batch_size: i64,
    pub media_storage_root: String,
    pub media_signing_secret: String,
    pub media_signed_url_ttl_seconds: u64,
}

impl Config {
    pub fn from_env() -> Self {
        let app_name = env::var("APP_NAME").unwrap_or_else(|_| "virival-api".to_string());
        let bind_address = env::var("BIND_ADDRESS").unwrap_or_else(|_| "0.0.0.0:8090".to_string());
        let database_url = env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/virival".to_string());
        let allow_dev_auth_bypass = env::var("ALLOW_DEV_AUTH_BYPASS")
            .ok()
            .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
            .unwrap_or(true);
        let google_workspace_domain = env::var("GOOGLE_WORKSPACE_DOMAIN")
            .ok()
            .map(|value| value.trim().to_lowercase())
            .filter(|value| !value.is_empty());
        let google_client_id = env::var("GOOGLE_CLIENT_ID")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let google_jwks_url = env::var("GOOGLE_JWKS_URL")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let oidc_jwks_cache_seconds = env::var("OIDC_JWKS_CACHE_SECONDS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(3600);
        let run_mode = env::var("RUN_MODE")
            .ok()
            .map(|value| value.trim().to_lowercase())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "api".to_string());
        let worker_poll_seconds = env::var("WORKER_POLL_SECONDS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(30);
        let worker_batch_size = env::var("WORKER_BATCH_SIZE")
            .ok()
            .and_then(|value| value.parse::<i64>().ok())
            .unwrap_or(50);
        let media_storage_root = env::var("MEDIA_STORAGE_ROOT")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "./data/media".to_string());
        let media_signing_secret = env::var("MEDIA_SIGNING_SECRET")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "dev-media-signing-secret-change-me".to_string());
        let media_signed_url_ttl_seconds = env::var("MEDIA_SIGNED_URL_TTL_SECONDS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(900);
        Self {
            app_name,
            bind_address,
            database_url,
            allow_dev_auth_bypass,
            google_workspace_domain,
            google_client_id,
            google_jwks_url,
            oidc_jwks_cache_seconds,
            run_mode,
            worker_poll_seconds,
            worker_batch_size,
            media_storage_root,
            media_signing_secret,
            media_signed_url_ttl_seconds,
        }
    }
}
