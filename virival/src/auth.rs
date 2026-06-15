use std::str::FromStr;

use axum::{
    extract::{Request, State},
    middleware::Next,
    response::{IntoResponse, Response},
};
use tracing::warn;
use uuid::Uuid;

use crate::{
    error::ApiError,
    models::{AppRole, AuthenticatedUser, NewAuditLogEntry},
    oidc::verify_google_id_token,
    state::AppState,
};

pub async fn require_auth(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Response {
    let method = request.method().as_str().to_string();
    let path = request.uri().path().to_string();

    match authenticate_request(&state, request.headers()).await {
        Ok(user) => {
            request.extensions_mut().insert(user.clone());
            let response = next.run(request).await;
            if let Err(err) = state
                .repository
                .insert_audit_log(NewAuditLogEntry {
                    actor_user_id: Some(user.user_id),
                    actor_subject: user.subject.clone(),
                    actor_role: Some(user.role.clone()),
                    actor_email: user.email.clone(),
                    auth_source: Some(user.auth_source.clone()),
                    method,
                    path,
                    status_code: i32::from(response.status().as_u16()),
                })
                .await
            {
                warn!(error = %err, "failed to persist audit log");
            }
            response
        }
        Err(err) => err.into_response(),
    }
}

async fn authenticate_request(
    state: &AppState,
    headers: &axum::http::HeaderMap,
) -> Result<AuthenticatedUser, ApiError> {
    let organization_scope = parse_org_scope(headers)?;

    if let Some(token) = extract_bearer_token(headers) {
        let identity = verify_google_id_token(
            &token,
            state.google_client_id.as_deref(),
            state.google_workspace_domain.as_deref(),
        )
        .await?;
        let user = state
            .repository
            .upsert_user(
                &identity.subject,
                Some(&identity.email),
                identity.display_name.as_deref(),
                AppRole::Investigator,
            )
            .await?;
        let role = resolve_effective_role(
            state,
            user.id,
            organization_scope,
            user.platform_role.clone(),
        )
        .await?;
        return Ok(AuthenticatedUser {
            user_id: user.id,
            subject: user.subject,
            email: user.email,
            role,
            organization_scope,
            auth_source: "google_oidc".to_string(),
        });
    }

    let header_subject = headers
        .get("x-virival-user")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let header_role = headers
        .get("x-virival-role")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(AppRole::from_str)
        .transpose()
        .map_err(ApiError::BadRequest)?;

    match (header_subject, header_role) {
        (Some(subject), role_hint) => {
            let role_hint = role_hint.unwrap_or(AppRole::Investigator);
            let email_hint = if subject.contains('@') {
                Some(subject.as_str())
            } else {
                None
            };
            let user = state
                .repository
                .upsert_user(&subject, email_hint, None, role_hint)
                .await?;
            let role = resolve_effective_role(
                state,
                user.id,
                organization_scope,
                user.platform_role.clone(),
            )
            .await?;
            Ok(AuthenticatedUser {
                user_id: user.id,
                subject: user.subject,
                email: user.email,
                role,
                organization_scope,
                auth_source: "dev_headers".to_string(),
            })
        }
        (None, _) if state.allow_dev_auth_bypass => {
            let user = state
                .repository
                .upsert_user(
                    "dev@virival.local",
                    Some("dev@virival.local"),
                    Some("Virival Dev User"),
                    AppRole::PlatformAdmin,
                )
                .await?;
            let role = resolve_effective_role(
                state,
                user.id,
                organization_scope,
                user.platform_role.clone(),
            )
            .await?;
            Ok(AuthenticatedUser {
                user_id: user.id,
                subject: user.subject,
                email: user.email,
                role,
                organization_scope,
                auth_source: "dev_bypass".to_string(),
            })
        }
        _ => Err(ApiError::Unauthorized(
            "missing auth credentials: provide Authorization bearer token or x-virival-user"
                .to_string(),
        )),
    }
}

async fn resolve_effective_role(
    state: &AppState,
    user_id: Uuid,
    organization_scope: Option<Uuid>,
    fallback_role: AppRole,
) -> Result<AppRole, ApiError> {
    if let Some(organization_id) = organization_scope {
        if let Some(role) = state
            .repository
            .get_membership_role(user_id, organization_id)
            .await?
        {
            return Ok(role);
        }
    }
    Ok(fallback_role)
}

fn parse_org_scope(headers: &axum::http::HeaderMap) -> Result<Option<Uuid>, ApiError> {
    headers
        .get("x-virival-organization-id")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            Uuid::parse_str(value).map_err(|err| {
                ApiError::BadRequest(format!("invalid organization scope header: {err}"))
            })
        })
        .transpose()
}

fn extract_bearer_token(headers: &axum::http::HeaderMap) -> Option<String> {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub fn require_any_role(user: &AuthenticatedUser, allowed: &[AppRole]) -> Result<(), ApiError> {
    if allowed.iter().any(|role| role == &user.role) {
        Ok(())
    } else {
        Err(ApiError::Forbidden(format!(
            "role {} is not allowed for this operation",
            user.role.as_str()
        )))
    }
}
