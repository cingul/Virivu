use std::sync::Arc;

use crate::oidc::OidcVerifier;
use crate::repository::Repository;

#[derive(Clone)]
pub struct AppState {
    pub repository: Arc<dyn Repository>,
    pub oidc_verifier: Arc<OidcVerifier>,
    pub allow_dev_auth_bypass: bool,
}
