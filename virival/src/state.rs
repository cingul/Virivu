use std::sync::Arc;

use crate::repository::Repository;

#[derive(Clone)]
pub struct AppState {
    pub repository: Arc<dyn Repository>,
    pub allow_dev_auth_bypass: bool,
    pub google_workspace_domain: Option<String>,
    pub google_client_id: Option<String>,
}
