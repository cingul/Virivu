use std::sync::Arc;

use crate::media::MediaUrlSigner;
use crate::oidc::OidcVerifier;
use crate::repository::Repository;

#[derive(Clone)]
pub struct AppState {
    pub repository: Arc<dyn Repository>,
    pub oidc_verifier: Arc<OidcVerifier>,
    pub allow_dev_auth_bypass: bool,
    pub media_signer: Arc<MediaUrlSigner>,
    pub media_storage_root: Arc<std::path::PathBuf>,
    pub media_signed_url_ttl_seconds: u64,
}
