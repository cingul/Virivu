use std::str::FromStr;

use axum::{
    extract::{Request, State},
    middleware::Next,
    response::{IntoResponse, Response},
};

use crate::{
    error::ApiError,
    models::{AppRole, AuthenticatedUser},
    state::AppState,
};

pub async fn require_auth(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Response {
    match authenticate_request(&state, request.headers()) {
        Ok(user) => {
            request.extensions_mut().insert(user);
            next.run(request).await
        }
        Err(err) => err.into_response(),
    }
}

fn authenticate_request(
    state: &AppState,
    headers: &axum::http::HeaderMap,
) -> Result<AuthenticatedUser, ApiError> {
    let subject = headers
        .get("x-virival-user")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);

    let role = headers
        .get("x-virival-role")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(AppRole::from_str)
        .transpose()
        .map_err(ApiError::BadRequest)?;

    match (subject, role) {
        (Some(subject), Some(role)) => Ok(AuthenticatedUser { subject, role }),
        (Some(subject), None) => Ok(AuthenticatedUser {
            subject,
            role: AppRole::Investigator,
        }),
        (None, _) if state.allow_dev_auth_bypass => Ok(AuthenticatedUser {
            subject: "dev@virival.local".to_string(),
            role: AppRole::PlatformAdmin,
        }),
        _ => Err(ApiError::Unauthorized(
            "missing auth headers: x-virival-user (+ optional x-virival-role)".to_string(),
        )),
    }
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
