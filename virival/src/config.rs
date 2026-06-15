use std::env;

#[derive(Debug, Clone)]
pub struct Config {
    pub app_name: String,
    pub bind_address: String,
    pub database_url: String,
    pub allow_dev_auth_bypass: bool,
    pub google_workspace_domain: Option<String>,
    pub google_client_id: Option<String>,
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
        Self {
            app_name,
            bind_address,
            database_url,
            allow_dev_auth_bypass,
            google_workspace_domain,
            google_client_id,
        }
    }
}
