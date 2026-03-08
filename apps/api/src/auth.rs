use axum::{
    extract::FromRequestParts,
    http::{header, request::Parts, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use base64::{
    engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD},
    Engine as _,
};
use serde::{Deserialize, Serialize};
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
    let info = decode_claims_without_verification(id_token)?;

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

    // TODO: Enable cryptographic signature verification via Google's JWKS before production use.
    let display_name = info.name.unwrap_or_else(|| info.email.clone());
    let user = db
        .upsert_user(&info.email, &info.sub, &display_name)
        .await
        .map_err(|e| AuthError::Internal(format!("unable to upsert user: {e}")))?;

    let memberships = db
        .load_memberships_for_email(&user.email)
        .await
        .map_err(|e| AuthError::Internal(format!("unable to load memberships: {e}")))?;

    Ok(AuthenticatedUser {
        user_id: user.id,
        email: user.email,
        google_subject: user.google_subject,
        display_name: user.display_name,
        domain,
        memberships,
    })
}

fn decode_claims_without_verification(id_token: &str) -> Result<GoogleIdClaims, AuthError> {
    let mut parts = id_token.split('.');
    let _header = parts
        .next()
        .ok_or_else(|| AuthError::Unauthorized("malformed JWT".to_string()))?;
    let payload = parts
        .next()
        .ok_or_else(|| AuthError::Unauthorized("malformed JWT".to_string()))?;
    let _signature = parts
        .next()
        .ok_or_else(|| AuthError::Unauthorized("malformed JWT".to_string()))?;

    let decoded_payload = URL_SAFE_NO_PAD
        .decode(payload.as_bytes())
        .or_else(|_| URL_SAFE.decode(payload.as_bytes()))
        .map_err(|e| AuthError::Unauthorized(format!("unable to decode JWT payload: {e}")))?;
    serde_json::from_slice::<GoogleIdClaims>(&decoded_payload)
        .map_err(|e| AuthError::Unauthorized(format!("unable to parse JWT payload: {e}")))
}
