use uuid::Uuid;

use crate::auth::{AuthError, AuthenticatedUser};
use crate::error::ApiError;

pub const ROLE_PLATFORM_ADMIN: &[&str] = &["platform_admin"];
pub const ROLE_ORG_MANAGERS: &[&str] = &["platform_admin", "org_admin"];
pub const ROLE_COORDINATOR_OR_BETTER: &[&str] = &[
    "platform_admin",
    "org_admin",
    "site_coordinator",
    "investigator",
];
pub const ROLE_ANALYTICS: &[&str] = &[
    "platform_admin",
    "org_admin",
    "investigator",
    "site_coordinator",
    "analyst",
    "monitor",
];

pub fn require_platform_role(user: &AuthenticatedUser, roles: &[&str]) -> Result<(), ApiError> {
    if user.has_platform_role(roles) {
        Ok(())
    } else {
        Err(ApiError::Auth(AuthError::Forbidden(
            "user lacks required platform role".to_string(),
        )))
    }
}

pub fn require_org_role(
    user: &AuthenticatedUser,
    organization_id: Uuid,
    roles: &[&str],
) -> Result<(), ApiError> {
    if user.has_org_role(organization_id, roles) {
        Ok(())
    } else {
        Err(ApiError::Auth(AuthError::Forbidden(
            "user lacks required organization role".to_string(),
        )))
    }
}
