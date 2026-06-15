use std::{
    collections::HashMap,
    process::Command,
    sync::Arc,
    time::{Duration, Instant},
};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use ring::signature::{self, RsaPublicKeyComponents};
use serde::Deserialize;
use tokio::sync::RwLock;

use crate::error::ApiError;

#[derive(Debug, Clone)]
pub struct OidcIdentity {
    pub subject: String,
    pub email: String,
    pub display_name: Option<String>,
}

#[derive(Clone)]
pub struct OidcVerifier {
    expected_client_id: Option<String>,
    expected_workspace_domain: Option<String>,
    jwks_url: String,
    cache_ttl: Duration,
    cache: Arc<RwLock<JwksCache>>,
}

#[derive(Default)]
struct JwksCache {
    loaded_at: Option<Instant>,
    keys: HashMap<String, CachedRsaKey>,
}

#[derive(Clone)]
struct CachedRsaKey {
    modulus: Vec<u8>,
    exponent: Vec<u8>,
}

#[derive(Debug, Deserialize)]
struct JwtHeader {
    alg: String,
    kid: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GoogleClaims {
    iss: String,
    aud: serde_json::Value,
    exp: i64,
    sub: String,
    email: Option<String>,
    email_verified: Option<serde_json::Value>,
    hd: Option<String>,
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GoogleJwksResponse {
    keys: Vec<GoogleJwkKey>,
}

#[derive(Debug, Deserialize)]
struct GoogleJwkKey {
    kid: Option<String>,
    kty: Option<String>,
    n: Option<String>,
    e: Option<String>,
}

impl OidcVerifier {
    pub fn new(
        expected_client_id: Option<String>,
        expected_workspace_domain: Option<String>,
        jwks_url: Option<String>,
        cache_ttl_seconds: u64,
    ) -> Self {
        Self {
            expected_client_id,
            expected_workspace_domain: expected_workspace_domain.map(|value| value.to_lowercase()),
            jwks_url: jwks_url
                .unwrap_or_else(|| "https://www.googleapis.com/oauth2/v3/certs".to_string()),
            cache_ttl: Duration::from_secs(cache_ttl_seconds.max(60)),
            cache: Arc::new(RwLock::new(JwksCache::default())),
        }
    }

    pub async fn verify_google_id_token(&self, id_token: &str) -> Result<OidcIdentity, ApiError> {
        let segments = id_token.split('.').collect::<Vec<_>>();
        if segments.len() != 3 {
            return Err(ApiError::Unauthorized(
                "OIDC token format is invalid".to_string(),
            ));
        }
        let header: JwtHeader = decode_json_segment(segments[0])?;
        if header.alg != "RS256" {
            return Err(ApiError::Unauthorized(
                "Google OIDC token must use RS256".to_string(),
            ));
        }
        let kid = header
            .kid
            .clone()
            .ok_or_else(|| ApiError::Unauthorized("OIDC token header missing kid".to_string()))?;
        let jwk = self.rsa_key_for_kid(&kid).await?;

        let signing_input = format!("{}.{}", segments[0], segments[1]);
        let signature_bytes = URL_SAFE_NO_PAD.decode(segments[2]).map_err(|err| {
            ApiError::Unauthorized(format!("invalid JWT signature encoding: {err}"))
        })?;
        let public_key = RsaPublicKeyComponents {
            n: &jwk.modulus,
            e: &jwk.exponent,
        };
        public_key
            .verify(
                &signature::RSA_PKCS1_2048_8192_SHA256,
                signing_input.as_bytes(),
                &signature_bytes,
            )
            .map_err(|_| {
                ApiError::Unauthorized("OIDC token signature verification failed".to_string())
            })?;

        let claims: GoogleClaims = decode_json_segment(segments[1])?;
        self.validate_claims(&claims)?;

        let email = claims
            .email
            .clone()
            .ok_or_else(|| ApiError::Unauthorized("OIDC token missing email claim".to_string()))?;

        Ok(OidcIdentity {
            subject: claims.sub,
            email,
            display_name: claims.name,
        })
    }

    fn validate_claims(&self, claims: &GoogleClaims) -> Result<(), ApiError> {
        if claims.iss != "https://accounts.google.com" && claims.iss != "accounts.google.com" {
            return Err(ApiError::Unauthorized(
                "OIDC issuer claim is not Google".to_string(),
            ));
        }
        if claims.exp <= chrono::Utc::now().timestamp() {
            return Err(ApiError::Unauthorized("OIDC token is expired".to_string()));
        }
        if let Some(expected_client_id) = self.expected_client_id.as_deref() {
            let audience_ok = match &claims.aud {
                serde_json::Value::String(value) => value == expected_client_id,
                serde_json::Value::Array(values) => values
                    .iter()
                    .filter_map(|value| value.as_str())
                    .any(|value| value == expected_client_id),
                _ => false,
            };
            if !audience_ok {
                return Err(ApiError::Unauthorized(
                    "OIDC token audience does not match configured GOOGLE_CLIENT_ID".to_string(),
                ));
            }
        }

        let email = claims
            .email
            .as_deref()
            .ok_or_else(|| ApiError::Unauthorized("OIDC token missing email claim".to_string()))?;
        let email_verified = claims
            .email_verified
            .as_ref()
            .and_then(parse_bool_like)
            .unwrap_or(false);
        if !email_verified {
            return Err(ApiError::Unauthorized(
                "OIDC token email is not verified".to_string(),
            ));
        }

        if let Some(expected_domain) = self.expected_workspace_domain.as_deref() {
            let hosted_domain = claims.hd.clone().unwrap_or_default().to_lowercase();
            if hosted_domain != expected_domain {
                return Err(ApiError::Unauthorized(format!(
                    "OIDC hosted domain mismatch: expected {expected_domain}"
                )));
            }
            if !email
                .to_lowercase()
                .ends_with(&format!("@{}", expected_domain))
            {
                return Err(ApiError::Unauthorized(format!(
                    "OIDC email domain mismatch for expected workspace {expected_domain}"
                )));
            }
        }

        Ok(())
    }

    async fn rsa_key_for_kid(&self, kid: &str) -> Result<CachedRsaKey, ApiError> {
        {
            let cache = self.cache.read().await;
            if let Some(loaded_at) = cache.loaded_at {
                if loaded_at.elapsed() < self.cache_ttl {
                    if let Some(key) = cache.keys.get(kid) {
                        return Ok(key.clone());
                    }
                }
            }
        }

        self.refresh_keys().await?;
        let cache = self.cache.read().await;
        cache
            .keys
            .get(kid)
            .cloned()
            .ok_or_else(|| ApiError::Unauthorized(format!("OIDC key id {kid} not found in JWKS")))
    }

    async fn refresh_keys(&self) -> Result<(), ApiError> {
        let jwks_url = self.jwks_url.clone();
        let output = tokio::task::spawn_blocking(move || {
            Command::new("curl").arg("-fsSL").arg(jwks_url).output()
        })
        .await
        .map_err(|err| ApiError::Internal(format!("failed joining JWKS fetch task: {err}")))?
        .map_err(|err| ApiError::Internal(format!("failed running curl for JWKS fetch: {err}")))?;

        if !output.status.success() {
            return Err(ApiError::Unauthorized(format!(
                "unable to fetch Google JWKS (curl exit: {})",
                output.status
            )));
        }

        let payload = String::from_utf8(output.stdout)
            .map_err(|err| ApiError::Internal(format!("JWKS payload is not UTF-8: {err}")))?;
        let jwks: GoogleJwksResponse = serde_json::from_str(&payload)
            .map_err(|err| ApiError::Internal(format!("invalid JWKS JSON payload: {err}")))?;

        let mut keys = HashMap::new();
        for key in jwks.keys {
            if key.kty.as_deref() != Some("RSA") {
                continue;
            }
            let kid = match key.kid {
                Some(kid) if !kid.trim().is_empty() => kid,
                _ => continue,
            };
            let modulus = match key.n {
                Some(value) if !value.trim().is_empty() => {
                    URL_SAFE_NO_PAD.decode(value).map_err(|err| {
                        ApiError::Internal(format!(
                            "unable to decode JWKS modulus for kid {kid}: {err}"
                        ))
                    })?
                }
                _ => continue,
            };
            let exponent = match key.e {
                Some(value) if !value.trim().is_empty() => {
                    URL_SAFE_NO_PAD.decode(value).map_err(|err| {
                        ApiError::Internal(format!(
                            "unable to decode JWKS exponent for kid {kid}: {err}"
                        ))
                    })?
                }
                _ => continue,
            };
            keys.insert(kid, CachedRsaKey { modulus, exponent });
        }

        if keys.is_empty() {
            return Err(ApiError::Unauthorized(
                "no usable RSA keys found in JWKS payload".to_string(),
            ));
        }

        let mut cache = self.cache.write().await;
        cache.keys = keys;
        cache.loaded_at = Some(Instant::now());
        Ok(())
    }
}

fn decode_json_segment<T: for<'de> Deserialize<'de>>(segment: &str) -> Result<T, ApiError> {
    let bytes = URL_SAFE_NO_PAD
        .decode(segment)
        .map_err(|err| ApiError::Unauthorized(format!("unable to decode JWT segment: {err}")))?;
    serde_json::from_slice::<T>(&bytes)
        .map_err(|err| ApiError::Unauthorized(format!("unable to parse JWT segment JSON: {err}")))
}

fn parse_bool_like(value: &serde_json::Value) -> Option<bool> {
    match value {
        serde_json::Value::Bool(flag) => Some(*flag),
        serde_json::Value::String(text) => Some(text.eq_ignore_ascii_case("true")),
        _ => None,
    }
}
