use std::env;

#[derive(Debug, Clone)]
pub struct Config {
    pub app_name: String,
    pub bind_address: String,
    pub database_url: String,
    pub allowed_google_workspace_domain: String,
    pub google_client_id: Option<String>,
    /// WARNING: Only enable this in local development.
    /// Even then, it should ideally be combined with localhost-only checks.
    pub allow_dev_auth_bypass: bool,
    /// When true, the dev auth bypass is allowed even from non-localhost connections.
    /// This is extremely dangerous and should almost never be true in any shared environment.
    pub allow_unsafe_dev_bypass: bool,
    pub app_base_url: String,
}

impl Config {
    pub fn from_env() -> Self {
        let app_name = env::var("APP_NAME").unwrap_or_else(|_| "virivu-api".to_string());
        let bind_address = env::var("BIND_ADDRESS").unwrap_or_else(|_| "0.0.0.0:8080".to_string());
        let database_url = env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/virivu".to_string());
        let allowed_google_workspace_domain =
            env::var("GOOGLE_WORKSPACE_DOMAIN").unwrap_or_else(|_| "cingulum.org".to_string());
        let google_client_id = env::var("GOOGLE_CLIENT_ID").ok();
        let allow_dev_auth_bypass = env::var("ALLOW_DEV_AUTH_BYPASS")
            .map(|v| matches!(v.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
            .unwrap_or(false);

        let allow_unsafe_dev_bypass = env::var("ALLOW_UNSAFE_DEV_BYPASS")
            .map(|v| matches!(v.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
            .unwrap_or(false);

        let app_base_url =
            env::var("APP_BASE_URL").unwrap_or_else(|_| "http://localhost:8080".to_string());

        Self {
            app_name,
            bind_address,
            database_url,
            allowed_google_workspace_domain,
            google_client_id,
            allow_dev_auth_bypass,
            allow_unsafe_dev_bypass,
            app_base_url,
        }
    }
}
