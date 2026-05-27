use axum::{
    extract::FromRequestParts,
    http::{header, request::Parts, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::{config::Config, db::Db, models::UserMembership};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthenticatedUser {
    pub user_id: Uuid,
    pub email: String,
    pub google_subject: String,
    pub display_name: String,
    pub domain: String,
    pub memberships: Vec<UserMembership>,
}

impl AuthenticatedUser {
    pub fn has_platform_role(&self, allowed_roles: &[&str]) -> bool {
        self.memberships.iter().any(|m| {
            m.organization_id.is_none()
                && m.project_id.is_none()
                && allowed_roles.contains(&m.role.as_str())
        })
    }

    pub fn has_org_role(&self, organization_id: Uuid, allowed_roles: &[&str]) -> bool {
        self.has_platform_role(allowed_roles)
            || self.memberships.iter().any(|m| {
                m.organization_id == Some(organization_id)
                    && allowed_roles.contains(&m.role.as_str())
            })
    }

    pub fn has_project_role(&self, project_id: Uuid, allowed_roles: &[&str]) -> bool {
        self.has_platform_role(allowed_roles)
            || self.memberships.iter().any(|m| {
                m.project_id == Some(project_id) && allowed_roles.contains(&m.role.as_str())
            })
    }
}

impl<S> FromRequestParts<S> for AuthenticatedUser
where
    S: Send + Sync,
{
    type Rejection = AuthError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<AuthenticatedUser>()
            .cloned()
            .ok_or_else(|| AuthError::Unauthorized("authentication context missing".to_string()))
    }
}

#[derive(Debug, Deserialize)]
struct GoogleIdClaims {
    sub: String,
    email: String,
    email_verified: Option<bool>,
    name: Option<String>,
    hd: Option<String>,
    iss: Option<String>,
    aud: Option<String>,
    exp: Option<i64>,
}

/// Google's JWKS key representation (we only need the RSA components for RS256).
#[derive(Debug, Deserialize)]
struct JwksKey {
    kty: String,
    alg: Option<String>,
    kid: Option<String>,
    n: Option<String>,
    e: Option<String>,
}

#[derive(Debug, Deserialize)]
struct JwksResponse {
    keys: Vec<JwksKey>,
}

/// In-memory cache for Google's public keys.
/// Refreshes on unknown `kid` or after a safety window.
struct JwksCache {
    keys: HashMap<String, DecodingKey>,
    last_refresh: Instant,
}

/// Shared JWKS cache (lazy initialized on first real token verification).
static JWKS_CACHE: RwLock<Option<Arc<JwksCache>>> = RwLock::const_new(None);

/// Shared HTTP client for fetching JWKS (created once).
static HTTP_CLIENT: std::sync::OnceLock<Client> = std::sync::OnceLock::new();

fn get_http_client() -> &'static Client {
    HTTP_CLIENT.get_or_init(|| {
        Client::builder()
            .timeout(Duration::from_secs(10))
            .user_agent("virivu-api/1.0 (Google ID token verification)")
            .build()
            .expect("failed to build reqwest client for JWKS")
    })
}

/// Fetches Google's current JWKS and returns a map of kid -> DecodingKey.
/// Only RSA keys with n/e components are retained.
async fn fetch_google_jwks() -> Result<HashMap<String, DecodingKey>, AuthError> {
    let client = get_http_client();
    let resp = client
        .get("https://www.googleapis.com/oauth2/v3/certs")
        .send()
        .await
        .map_err(|e| AuthError::Internal(format!("failed to fetch Google JWKS: {e}")))?;

    if !resp.status().is_success() {
        return Err(AuthError::Internal(format!(
            "Google JWKS endpoint returned status {}",
            resp.status()
        )));
    }

    let jwks: JwksResponse = resp
        .json()
        .await
        .map_err(|e| AuthError::Internal(format!("failed to parse Google JWKS JSON: {e}")))?;

    let mut keys = HashMap::new();
    for key in jwks.keys {
        if key.kty != "RSA" {
            continue;
        }
        let kid = match key.kid {
            Some(k) if !k.is_empty() => k,
            _ => continue,
        };
        let n = match &key.n {
            Some(n) if !n.is_empty() => n,
            _ => continue,
        };
        let e = match &key.e {
            Some(e) if !e.is_empty() => e,
            _ => continue,
        };

        match DecodingKey::from_rsa_components(n, e) {
            Ok(decoding_key) => {
                keys.insert(kid, decoding_key);
            }
            Err(err) => {
                tracing::warn!("skipping JWKS key {}: failed to build decoding key: {}", kid, err);
            }
        }
    }

    if keys.is_empty() {
        return Err(AuthError::Internal(
            "Google JWKS response contained no usable RSA keys".to_string(),
        ));
    }

    Ok(keys)
}

/// Returns a snapshot of the currently cached Google public keys.
/// Triggers a refresh if the cache is cold or we see an unknown key id during verification.
async fn get_google_public_keys() -> Result<Arc<HashMap<String, DecodingKey>>, AuthError> {
    {
        let guard = JWKS_CACHE.read().await;
        if let Some(cache) = guard.as_ref() {
            // Simple time-based refresh (every 6 hours is plenty; Google rotates slowly)
            if cache.last_refresh.elapsed() < Duration::from_secs(6 * 3600) {
                return Ok(Arc::new(cache.keys.clone()));
            }
        }
    }

    // Need to refresh
    let new_keys = fetch_google_jwks().await?;
    let new_cache = Arc::new(JwksCache {
        keys: new_keys,
        last_refresh: Instant::now(),
    });

    {
        let mut guard = JWKS_CACHE.write().await;
        *guard = Some(new_cache.clone());
    }

    Ok(Arc::new(new_cache.keys.clone()))
}

#[derive(Debug)]
pub enum AuthError {
    Unauthorized(String),
    Forbidden(String),
    Internal(String),
}

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            Self::Unauthorized(m) => (StatusCode::UNAUTHORIZED, m),
            Self::Forbidden(m) => (StatusCode::FORBIDDEN, m),
            Self::Internal(m) => (StatusCode::INTERNAL_SERVER_ERROR, m),
        };
        (
            status,
            Json(serde_json::json!({
                "error": message
            })),
        )
            .into_response()
    }
}

pub fn extract_bearer_token(headers: &HeaderMap) -> Option<String> {
    let header_value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    if !header_value.starts_with("Bearer ") {
        return None;
    }
    Some(
        header_value
            .trim_start_matches("Bearer ")
            .trim()
            .to_string(),
    )
}

pub async fn verify_google_workspace_user(
    config: &Config,
    db: &Db,
    id_token: &str,
) -> Result<AuthenticatedUser, AuthError> {
    // 1. Parse the JWT header to extract the key id (kid) and algorithm.
    let header = decode_header(id_token).map_err(|e| {
        AuthError::Unauthorized(format!("failed to decode JWT header: {e}"))
    })?;

    if header.alg != Algorithm::RS256 {
        return Err(AuthError::Unauthorized(
            "only RS256 tokens are supported for Google ID tokens".to_string(),
        ));
    }

    let kid = header.kid.ok_or_else(|| {
        AuthError::Unauthorized("JWT header missing 'kid' (key id)".to_string())
    })?;

    // 2. Get (or refresh) Google's public keys.
    let public_keys = get_google_public_keys().await?;
    let decoding_key = public_keys.get(&kid).ok_or_else(|| {
        // Unknown kid (almost always means Google rotated the signing key).
        // Clear the cache so the next verification attempt will refetch JWKS.
        tokio::spawn(async {
            let mut guard = JWKS_CACHE.write().await;
            *guard = None;
        });
        AuthError::Unauthorized(format!(
            "unknown signing key id '{}' — token may be from a rotated key",
            kid
        ))
    })?;

    // 3. Cryptographically verify the signature and decode the claims.
    let mut validation = Validation::new(Algorithm::RS256);
    // We perform our own issuer / audience / expiry checks below for clearer error messages
    // and to keep behavior identical to the previous implementation.
    validation.validate_exp = false;
    validation.validate_aud = false;
    validation.validate_nbf = false;

    let decoded = decode::<GoogleIdClaims>(id_token, decoding_key, &validation)
        .map_err(|e| AuthError::Unauthorized(format!("JWT signature verification failed: {e}")))?;

    let info = decoded.claims;

    // 4. Run the existing business claim validations (kept almost identical for behavior continuity).
    if info.email.trim().is_empty() || info.sub.trim().is_empty() {
        return Err(AuthError::Unauthorized(
            "token missing required identity claims".to_string(),
        ));
    }

    if info.email_verified != Some(true) {
        return Err(AuthError::Unauthorized(
            "email is not verified by Google".to_string(),
        ));
    }

    let issuer = info.iss.unwrap_or_default();
    if issuer != "https://accounts.google.com" && issuer != "accounts.google.com" {
        return Err(AuthError::Unauthorized(
            "unexpected token issuer".to_string(),
        ));
    }

    if let Some(expected_client_id) = &config.google_client_id {
        let audience = info.aud.unwrap_or_default();
        if audience != *expected_client_id {
            return Err(AuthError::Unauthorized(
                "token audience does not match configured Google client".to_string(),
            ));
        }
    }

    let now = chrono::Utc::now().timestamp();
    if info.exp.unwrap_or_default() <= now {
        return Err(AuthError::Unauthorized("token has expired".to_string()));
    }

    let domain = info.hd.unwrap_or_default();
    if domain != config.allowed_google_workspace_domain {
        return Err(AuthError::Forbidden(format!(
            "workspace domain {} is not allowed",
            domain
        )));
    }

    // 5. Everything checks out — proceed with user upsert + membership loading (unchanged).
    let display_name = info.name.unwrap_or_else(|| info.email.clone());
    let user = db
        .upsert_user(&info.email, &info.sub, &display_name)
        .await
        .map_err(|e| AuthError::Internal(format!("unable to upsert user: {e}")))?;

    let memberships = db
        .load_memberships_for_email(&user.email)
        .await
        .map_err(|e| AuthError::Internal(format!("unable to load memberships: {e}")))?;

    tracing::info!(
        email = %user.email,
        user_id = %user.id,
        domain = %domain,
        method = "google_jwt",
        "user authenticated successfully via Google ID token"
    );

    // Light audit trail for authentication events
    let _ = db
        .insert_audit_log(
            "users",
            user.id,
            "auth.login",
            Some(user.id),
            None,
            None,
            Some("google_jwt"),
        )
        .await;

    Ok(AuthenticatedUser {
        user_id: user.id,
        email: user.email,
        google_subject: user.google_subject,
        display_name: user.display_name,
        domain,
        memberships,
    })
}
