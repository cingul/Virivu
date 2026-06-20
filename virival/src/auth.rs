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
    state::AppState,
};

pub async fn require_auth(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Response {
    let method = request.method().as_str().to_string();
    let path = request.uri().path().to_string();
    let (action, resource_type, resource_id) = infer_audit_context(&method, &path);

    match authenticate_request(&state, request.headers()).await {
        Ok(user) => {
            request.extensions_mut().insert(user.clone());
            let response = next.run(request).await;
            let metadata_json = serde_json::json!({
                "auth_source": user.auth_source,
                "organization_scope": user.organization_scope.map(|id| id.to_string()),
            });
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
                    action,
                    resource_type,
                    resource_id,
                    metadata_json: Some(metadata_json),
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
        let identity = state.oidc_verifier.verify_google_id_token(&token).await?;
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

fn infer_audit_context(method: &str, path: &str) -> (Option<String>, Option<String>, Option<Uuid>) {
    let segments = path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    let resource_id = segments
        .iter()
        .find_map(|segment| Uuid::parse_str(segment).ok());
    let resource_type = if segments.len() >= 3 && segments[0] == "api" && segments[1] == "v1" {
        if segments[2] == "admin" {
            segments.get(3).map(|segment| (*segment).to_string())
        } else {
            Some(segments[2].to_string())
        }
    } else {
        None
    };
    let action = if method.eq_ignore_ascii_case("GET") {
        Some("read".to_string())
    } else if path.ends_with("/phase") {
        Some("phase_transition".to_string())
    } else if path.ends_with("/publish") {
        Some("publish".to_string())
    } else if path.ends_with("/lock") {
        Some("lock".to_string())
    } else if path.ends_with("/close") {
        Some("close".to_string())
    } else if path.ends_with("/activate") {
        Some("activate".to_string())
    } else if method.eq_ignore_ascii_case("POST") {
        Some("create_or_mutate".to_string())
    } else {
        Some(method.to_ascii_lowercase())
    };

    (action, resource_type, resource_id)
}
