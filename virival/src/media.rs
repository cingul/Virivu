use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use std::{path::PathBuf, sync::Arc};

use async_trait::async_trait;
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

#[derive(Debug, Clone)]
pub struct MediaUploadPolicy {
    pub max_upload_bytes: u64,
    pub allowed_content_types: Vec<String>,
}

impl MediaUploadPolicy {
    pub fn allows_content_type(&self, content_type: &str) -> bool {
        let normalized = content_type.trim().to_ascii_lowercase();
        self.allowed_content_types
            .iter()
            .any(|allowed| allowed == &normalized)
    }
}

#[async_trait]
pub trait MediaStorage: Send + Sync {
    async fn put_object(&self, object_key: &str, payload: &[u8]) -> Result<(), String>;
    async fn get_object(&self, object_key: &str) -> Result<Vec<u8>, String>;
}

#[derive(Clone)]
pub struct LocalMediaStorage {
    root: PathBuf,
}

impl LocalMediaStorage {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn object_path(&self, object_key: &str) -> PathBuf {
        object_key
            .split('/')
            .fold(self.root.clone(), |acc, segment| acc.join(segment))
    }
}

#[async_trait]
impl MediaStorage for LocalMediaStorage {
    async fn put_object(&self, object_key: &str, payload: &[u8]) -> Result<(), String> {
        let path = self.object_path(object_key);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|err| format!("failed creating storage directories: {err}"))?;
        }
        tokio::fs::write(path, payload)
            .await
            .map_err(|err| format!("failed writing storage object: {err}"))?;
        Ok(())
    }

    async fn get_object(&self, object_key: &str) -> Result<Vec<u8>, String> {
        let path = self.object_path(object_key);
        tokio::fs::read(path)
            .await
            .map_err(|err| format!("failed reading storage object: {err}"))
    }
}

pub trait MediaScanner: Send + Sync {
    fn scan(&self, filename: &str, content_type: &str, payload: &[u8]) -> Result<(), String>;
}

#[derive(Clone)]
pub struct NoopMediaScanner;

impl MediaScanner for NoopMediaScanner {
    fn scan(&self, _filename: &str, _content_type: &str, _payload: &[u8]) -> Result<(), String> {
        Ok(())
    }
}

#[derive(Clone)]
pub struct KeywordMediaScanner {
    blocked_keywords: Vec<String>,
}

impl KeywordMediaScanner {
    pub fn new(blocked_keywords: Vec<String>) -> Self {
        Self { blocked_keywords }
    }
}

impl MediaScanner for KeywordMediaScanner {
    fn scan(&self, _filename: &str, _content_type: &str, payload: &[u8]) -> Result<(), String> {
        if self.blocked_keywords.is_empty() {
            return Ok(());
        }
        let body = String::from_utf8_lossy(payload).to_ascii_lowercase();
        for keyword in &self.blocked_keywords {
            if body.contains(keyword) {
                return Err(format!(
                    "media scanner rejected payload due to blocked keyword: {keyword}"
                ));
            }
        }
        Ok(())
    }
}

pub fn build_media_storage(
    backend: &str,
    local_root: PathBuf,
    bucket: Option<&str>,
) -> Result<Arc<dyn MediaStorage>, String> {
    match backend {
        "local" => Ok(Arc::new(LocalMediaStorage::new(local_root))),
        "s3" => Err(format!(
            "media storage backend 's3' requires external adapter; bucket={}",
            bucket.unwrap_or("unset")
        )),
        "gcs" => Err(format!(
            "media storage backend 'gcs' requires external adapter; bucket={}",
            bucket.unwrap_or("unset")
        )),
        other => Err(format!(
            "unsupported MEDIA_STORAGE_BACKEND value: {other} (use local|s3|gcs)"
        )),
    }
}

pub fn build_media_scanner(mode: &str, blocked_keywords: &[String]) -> Arc<dyn MediaScanner> {
    match mode {
        "keyword" => Arc::new(KeywordMediaScanner::new(
            blocked_keywords
                .iter()
                .map(|value| value.trim().to_ascii_lowercase())
                .filter(|value| !value.is_empty())
                .collect(),
        )),
        _ => Arc::new(NoopMediaScanner),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        build_media_scanner, build_media_storage, KeywordMediaScanner, MediaScanner,
        MediaUploadPolicy, MediaUrlSigner,
    };
    use std::path::PathBuf;
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

    #[tokio::test]
    async fn local_storage_round_trip() {
        let root = std::env::temp_dir().join(format!("virival-media-test-{}", Uuid::new_v4()));
        let storage =
            build_media_storage("local", root.clone(), None).expect("local storage should build");
        storage
            .put_object("org/asset/test.bin", b"payload")
            .await
            .expect("put object");
        let loaded = storage
            .get_object("org/asset/test.bin")
            .await
            .expect("get object");
        assert_eq!(loaded, b"payload");
        let _ = tokio::fs::remove_dir_all(root).await;
    }

    #[test]
    fn upload_policy_content_type_allowlist() {
        let policy = MediaUploadPolicy {
            max_upload_bytes: 1_000_000,
            allowed_content_types: vec!["application/pdf".to_string(), "video/mp4".to_string()],
        };
        assert!(policy.allows_content_type("application/pdf"));
        assert!(policy.allows_content_type("video/mp4"));
        assert!(!policy.allows_content_type("text/plain"));
    }

    #[test]
    fn keyword_scanner_blocks_payload_keywords() {
        let scanner = KeywordMediaScanner::new(vec!["eicar".to_string()]);
        assert!(scanner
            .scan("test.txt", "text/plain", b"safe content")
            .is_ok());
        assert!(scanner
            .scan("test.txt", "text/plain", b"EICAR test signature")
            .is_err());
    }

    #[test]
    fn scanner_builder_keyword_mode() {
        let scanner = build_media_scanner("keyword", &["malware".to_string()]);
        assert!(scanner
            .scan("x.txt", "text/plain", b"contains malware")
            .is_err());
    }

    #[test]
    fn unsupported_backend_returns_error() {
        let result = build_media_storage("azure", PathBuf::from("/tmp"), None);
        assert!(result.is_err());
    }
}
