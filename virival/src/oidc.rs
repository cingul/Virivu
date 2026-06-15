use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::Utc;
use serde::Deserialize;

use crate::error::ApiError;

#[derive(Debug, Clone)]
pub struct OidcIdentity {
    pub subject: String,
    pub email: String,
    pub display_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GoogleIdTokenClaims {
    iss: Option<String>,
    aud: Option<String>,
    exp: Option<i64>,
    sub: Option<String>,
    email: Option<String>,
    email_verified: Option<serde_json::Value>,
    hd: Option<String>,
    name: Option<String>,
}

pub async fn verify_google_id_token(
    id_token: &str,
    expected_client_id: Option<&str>,
    expected_workspace_domain: Option<&str>,
) -> Result<OidcIdentity, ApiError> {
    // NOTE: This validates token claims only. Full JWKS signature verification is the next hardening step.
    let payload_segment = id_token
        .split('.')
        .nth(1)
        .ok_or_else(|| ApiError::Unauthorized("OIDC token format is invalid".to_string()))?;
    let payload_bytes = URL_SAFE_NO_PAD.decode(payload_segment).map_err(|err| {
        ApiError::Unauthorized(format!("unable to decode OIDC token payload: {err}"))
    })?;
    let claims: GoogleIdTokenClaims = serde_json::from_slice(&payload_bytes)
        .map_err(|err| ApiError::Unauthorized(format!("invalid OIDC token JSON payload: {err}")))?;

    let issuer = claims
        .iss
        .as_deref()
        .ok_or_else(|| ApiError::Unauthorized("OIDC token missing issuer claim".to_string()))?;
    if issuer != "https://accounts.google.com" && issuer != "accounts.google.com" {
        return Err(ApiError::Unauthorized(
            "OIDC token issuer is not Google".to_string(),
        ));
    }

    let audience = claims
        .aud
        .as_deref()
        .ok_or_else(|| ApiError::Unauthorized("OIDC token missing audience claim".to_string()))?;
    if let Some(expected_client_id) = expected_client_id {
        if audience != expected_client_id {
            return Err(ApiError::Unauthorized(
                "OIDC token audience does not match configured GOOGLE_CLIENT_ID".to_string(),
            ));
        }
    }

    let exp = claims
        .exp
        .ok_or_else(|| ApiError::Unauthorized("OIDC token missing exp claim".to_string()))?;
    if exp <= Utc::now().timestamp() {
        return Err(ApiError::Unauthorized("OIDC token is expired".to_string()));
    }

    let subject = claims
        .sub
        .clone()
        .ok_or_else(|| ApiError::Unauthorized("OIDC token missing sub claim".to_string()))?;
    let email = claims
        .email
        .clone()
        .ok_or_else(|| ApiError::Unauthorized("OIDC token missing email claim".to_string()))?;

    let email_verified = claims
        .email_verified
        .as_ref()
        .and_then(verified_flag)
        .unwrap_or(false);
    if !email_verified {
        return Err(ApiError::Unauthorized(
            "OIDC token email is not verified".to_string(),
        ));
    }

    if let Some(expected_domain) = expected_workspace_domain {
        let expected_domain = expected_domain.to_lowercase();
        let hosted_domain = claims.hd.clone().unwrap_or_default().to_lowercase();
        if hosted_domain != expected_domain {
            return Err(ApiError::Unauthorized(format!(
                "OIDC hosted domain mismatch: expected {expected_domain}"
            )));
        }
        if !email
            .to_lowercase()
            .ends_with(&format!("@{expected_domain}"))
        {
            return Err(ApiError::Unauthorized(format!(
                "OIDC email domain mismatch for expected workspace {expected_domain}"
            )));
        }
    }

    Ok(OidcIdentity {
        subject,
        email,
        display_name: claims.name,
    })
}

fn verified_flag(value: &serde_json::Value) -> Option<bool> {
    match value {
        serde_json::Value::Bool(flag) => Some(*flag),
        serde_json::Value::String(text) => Some(text.eq_ignore_ascii_case("true")),
        _ => None,
    }
}
