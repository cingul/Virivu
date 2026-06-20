use std::sync::Arc;

use crate::media::{MediaScanner, MediaStorage, MediaUploadPolicy, MediaUrlSigner};
use crate::oidc::OidcVerifier;
use crate::rate_limit::SimpleRateLimiter;
use crate::repository::Repository;

#[derive(Clone)]
pub struct AppState {
    pub repository: Arc<dyn Repository>,
    pub oidc_verifier: Arc<OidcVerifier>,
    pub allow_dev_auth_bypass: bool,
    pub media_signer: Arc<MediaUrlSigner>,
    pub media_storage: Arc<dyn MediaStorage>,
    pub media_scanner: Arc<dyn MediaScanner>,
    pub media_upload_policy: Arc<MediaUploadPolicy>,
    pub media_rate_limiter: Arc<SimpleRateLimiter>,
    pub media_signed_url_ttl_seconds: u64,
}
