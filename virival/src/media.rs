use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use ring::hmac;
use uuid::Uuid;

#[derive(Clone)]
pub struct MediaUrlSigner {
    key: hmac::Key,
}

impl MediaUrlSigner {
    pub fn new(secret: &str) -> Self {
        Self {
            key: hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes()),
        }
    }

    pub fn sign_upload(&self, asset_id: Uuid, expires_epoch: i64) -> String {
        self.sign("upload", asset_id, expires_epoch)
    }

    pub fn verify_upload(&self, asset_id: Uuid, expires_epoch: i64, signature: &str) -> bool {
        self.verify("upload", asset_id, expires_epoch, signature)
    }

    pub fn sign_download(&self, asset_id: Uuid, expires_epoch: i64) -> String {
        self.sign("download", asset_id, expires_epoch)
    }

    pub fn verify_download(&self, asset_id: Uuid, expires_epoch: i64, signature: &str) -> bool {
        self.verify("download", asset_id, expires_epoch, signature)
    }

    fn payload(scope: &str, asset_id: Uuid, expires_epoch: i64) -> String {
        format!("{scope}:{asset_id}:{expires_epoch}")
    }

    fn sign(&self, scope: &str, asset_id: Uuid, expires_epoch: i64) -> String {
        let payload = Self::payload(scope, asset_id, expires_epoch);
        let tag = hmac::sign(&self.key, payload.as_bytes());
        URL_SAFE_NO_PAD.encode(tag.as_ref())
    }

    fn verify(&self, scope: &str, asset_id: Uuid, expires_epoch: i64, signature: &str) -> bool {
        let payload = Self::payload(scope, asset_id, expires_epoch);
        let decoded = match URL_SAFE_NO_PAD.decode(signature.as_bytes()) {
            Ok(value) => value,
            Err(_) => return false,
        };
        hmac::verify(&self.key, payload.as_bytes(), &decoded).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::MediaUrlSigner;
    use uuid::Uuid;

    #[test]
    fn upload_signature_round_trip() {
        let signer = MediaUrlSigner::new("unit-test-secret");
        let asset_id = Uuid::new_v4();
        let expires = 1_781_600_000;
        let signature = signer.sign_upload(asset_id, expires);
        assert!(signer.verify_upload(asset_id, expires, &signature));
    }

    #[test]
    fn signature_rejects_tampered_scope_and_expiry() {
        let signer = MediaUrlSigner::new("unit-test-secret");
        let asset_id = Uuid::new_v4();
        let expires = 1_781_600_000;
        let signature = signer.sign_upload(asset_id, expires);
        assert!(!signer.verify_download(asset_id, expires, &signature));
        assert!(!signer.verify_upload(asset_id, expires + 1, &signature));
    }
}
