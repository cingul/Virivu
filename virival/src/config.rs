use std::env;

#[derive(Debug, Clone)]
pub struct Config {
    pub app_name: String,
    pub bind_address: String,
}

impl Config {
    pub fn from_env() -> Self {
        let app_name = env::var("APP_NAME").unwrap_or_else(|_| "virival-api".to_string());
        let bind_address = env::var("BIND_ADDRESS").unwrap_or_else(|_| "0.0.0.0:8090".to_string());
        Self {
            app_name,
            bind_address,
        }
    }
}
