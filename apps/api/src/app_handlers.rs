//! App dashboard CRUD submit handlers (organizations, projects, sites, patients,
//! providers, encounters, media tickets, invites).
//! Extracted from routes.rs to keep each module to a manageable size.

use axum::{
    extract::{Path, State},
    response::{Html, IntoResponse, Redirect, Response},
    Form,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::{
    auth::{AuthError, AuthenticatedUser},
    authz::{
        require_org_role, require_platform_role, ROLE_COORDINATOR_OR_BETTER, ROLE_ORG_MANAGERS,
        ROLE_PLATFORM_ADMIN,
    },
    error::ApiError,
    routes::{
        authenticate_from_dev_headers, html_escape, optional_non_empty, parse_optional_date,
        parse_uuid_field, query_escape, render_cingulum_page, AppContext,
    },
};

// ── Form structs ─────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct AppCreateOrganizationForm {
    pub admin_email: String,
    pub organization_name: String,
    pub parent_organization_id: String,
    pub organization_kind: String,
}

#[derive(Debug, Deserialize)]
pub struct AppCreateProjectForm {
    pub admin_email: String,
    pub organization_id: String,
    pub project_name: String,
    pub therapeutic_area: String,
}

#[derive(Debug, Deserialize)]
pub struct AppCreateSiteForm {
    pub admin_email: String,
    pub organization_id: String,
    pub project_id: String,
    pub site_name: String,
    pub principal_investigator: String,
    pub co_principal_investigator: Option<String>,
    pub sub_investigator: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AppDeleteSiteForm {
    pub admin_email: String,
}

#[derive(Debug, Deserialize)]
pub struct AppToggleDormancyForm {
    pub admin_email: String,
}

#[derive(Debug, Deserialize)]
pub struct AttachSiteToProjectForm {
    pub admin_email: String,
    pub project_id: Uuid,
}

#[derive(Debug, Deserialize)]
pub struct AppCreatePatientForm {
    pub admin_email: String,
    pub project_id: String,
    pub site_id: String,
    pub external_subject_id: String,
    pub email: String,
    pub date_of_birth: String,
}

#[derive(Debug, Deserialize)]
pub struct AppCreateProviderForm {
    pub admin_email: String,
    pub organization_id: String,
    pub provider_name: String,
    pub provider_title: String,
    pub referral_source: String,
    pub email: Option<String>,
    pub phone_number: Option<String>,
    pub npi_number: Option<String>,
    pub address: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AppCreateEncounterForm {
    pub admin_email: String,
    pub patient_id: String,
    pub encounter_type: String,
    pub provider_id: String,
    pub notes: String,
}

#[derive(Debug, Deserialize)]
pub struct AppSendInviteForm {
    pub admin_email: String,
    pub organization_id: String,
    pub project_id: String,
    pub patient_email: String,
    pub form_type: String,
}

#[derive(Debug, Deserialize)]
pub struct AppCreateMediaTicketForm {
    pub admin_email: String,
    pub organization_id: String,
    pub project_id: String,
    pub patient_id: String,
    pub mime_type: String,
}

pub(crate) async fn submit_app_create_organization(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
    Form(form): Form<AppCreateOrganizationForm>,
) -> Result<Redirect, ApiError> {
    require_platform_role(&user, ROLE_PLATFORM_ADMIN)?;

    if form.organization_name.trim().is_empty() {
        return Err(ApiError::Validation(
            "organization_name is required".to_string(),
        ));
    }

    let parent_organization_id = if form.parent_organization_id.trim().is_empty() {
        None
    } else {
        Some(parse_uuid_field(
            form.parent_organization_id.trim(),
            "parent_organization_id",
        )?)
    };
    let organization_kind = optional_non_empty(form.organization_kind.trim());
    let organization = ctx
        .db
        .create_organization(
            form.organization_name.trim(),
            parent_organization_id,
            organization_kind,
        )
        .await
        .map_err(ApiError::internal)?;

    // Keep the membership helper for now (it creates the initial org admin link)
    ctx.db
        .ensure_org_admin_membership(user.email.as_str(), organization.id)
        .await
        .map_err(ApiError::internal)?;

    Ok(Redirect::to(&format!(
        "/ui/app?admin_email={}&organization_id={}&notice={}",
        query_escape(user.email.as_str()),
        organization.id,
        query_escape("Organization created")
    )))
}

pub(crate) async fn submit_app_create_project(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
    Form(form): Form<AppCreateProjectForm>,
) -> Result<Redirect, ApiError> {
    let organization_id = parse_uuid_field(&form.organization_id, "organization_id")?;
    require_org_role(&user, organization_id, ROLE_ORG_MANAGERS)?;

    if form.project_name.trim().is_empty() {
        return Err(ApiError::Validation("project_name is required".to_string()));
    }

    let project = ctx
        .db
        .create_project(
            organization_id,
            form.project_name.trim(),
            form.therapeutic_area.trim(),
        )
        .await
        .map_err(ApiError::internal)?;

    Ok(Redirect::to(&format!(
        "/ui/app?admin_email={}&organization_id={}&project_id={}&notice={}",
        query_escape(user.email.as_str()),
        organization_id,
        project.id,
        query_escape("Project created")
    )))
}

pub(crate) async fn submit_app_create_site(
    State(ctx): State<AppContext>,
    Form(form): Form<AppCreateSiteForm>,
) -> Result<Redirect, ApiError> {
    tracing::warn!(
        admin_email = %form.admin_email,
        organization_id_raw = %form.organization_id,
        project_id_raw = %form.project_id,
        "submit_app_create_site received form submission"
    );

    // Resolve the acting user for authorization.
    // For dev bypass (the common browser testing path), we reconstruct using the admin_email
    // that the form always includes. This prevents "authentication context missing" when
    // these UI form routes are hit directly from the browser without the x-dev header being
    // present on the raw HTTP request.
    let user = if ctx.config.allow_dev_auth_bypass {
        let mut dev_headers = axum::http::HeaderMap::new();
        if let Ok(val) = form.admin_email.parse::<axum::http::HeaderValue>() {
            dev_headers.insert("x-dev-user-email", val);
        }
        match authenticate_from_dev_headers(&ctx, &dev_headers).await {
            Ok(u) => u,
            Err(e) => {
                return Err(ApiError::Auth(AuthError::Unauthorized(format!(
                    "dev auth failed for site creation form: {e:?}"
                ))));
            }
        }
    } else {
        // In a real-auth environment the route would be protected and the extractor would have run.
        // For now this path is not exercised in the user's dev setup.
        return Err(ApiError::Auth(AuthError::Unauthorized(
            "authenticated user required (real auth path not fully wired for this UI form yet)"
                .to_string(),
        )));
    };

    // organization_id is now required for site creation (sites live under orgs)
    if form.organization_id.trim().is_empty() {
        return Err(ApiError::Validation(
            "organization_id is required to create a site.".to_string(),
        ));
    }

    let organization_id = parse_uuid_field(&form.organization_id, "organization_id")?;
    require_org_role(&user, organization_id, ROLE_ORG_MANAGERS)?;

    // project_id is now optional (you can create org-level sites without a study).
    // Be extremely defensive: any non-empty value that is not a perfect UUID is treated
    // as "no project" so that old cached forms, short hex values, or bad context never
    // produce "project_id must be a valid UUID" when the user is just trying to create a site.
    let project_id = if form.project_id.trim().is_empty() {
        None
    } else {
        match form.project_id.trim().parse::<Uuid>() {
            Ok(id) => Some(id),
            Err(_) => {
                tracing::warn!(
                    project_id_raw = %form.project_id,
                    "non-UUID project_id received on site creation — forcing org-level site (no project attachment)"
                );
                None
            }
        }
    };

    ctx.db
        .create_site(
            organization_id,
            project_id,
            form.site_name.trim(),
            form.principal_investigator.trim(),
            form.co_principal_investigator
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty()),
            form.sub_investigator
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty()),
        )
        .await
        .map_err(ApiError::internal)?;

    // Redirect back to the org sites view; include project only if one was chosen
    let redirect_qs = if let Some(pid) = project_id {
        format!("&project_id={}", pid)
    } else {
        String::new()
    };
    Ok(Redirect::to(&format!(
        "/ui/app?admin_email={}&organization_id={}{}&notice={}",
        query_escape(user.email.as_str()),
        organization_id,
        redirect_qs,
        query_escape("Site created")
    )))
}

pub(crate) async fn submit_app_delete_site(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
    Path(site_id): Path<Uuid>,
    Form(form): Form<AppDeleteSiteForm>,
) -> Result<Redirect, ApiError> {
    let site = ctx
        .db
        .get_site(site_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("site not found".to_string()))?;

    // For actions that need the project, fall back gracefully if the site is org-level
    if let Some(pid) = site.project_id {
        if let Ok(Some(proj)) = ctx.db.get_project(pid).await {
            require_org_role(&user, proj.organization_id, ROLE_ORG_MANAGERS)?;
        }
    } else {
        require_org_role(&user, site.organization_id, ROLE_ORG_MANAGERS)?;
    }

    ctx.db
        .delete_site(site_id)
        .await
        .map_err(ApiError::internal)?;

    let redirect_project = site.project_id.unwrap_or_default();
    Ok(Redirect::to(&format!(
        "/ui/app?admin_email={}&organization_id={}&project_id={}&notice={}",
        query_escape(user.email.as_str()),
        site.organization_id,
        redirect_project,
        query_escape("Site deleted successfully")
    )))
}

pub(crate) async fn submit_app_site_toggle_dormancy(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
    Path(site_id): Path<Uuid>,
    Form(form): Form<AppToggleDormancyForm>,
) -> Result<Redirect, ApiError> {
    let site = ctx
        .db
        .get_site(site_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("site not found".to_string()))?;

    require_org_role(&user, site.organization_id, ROLE_ORG_MANAGERS)?;

    let next_status = if site.status == "dormant" {
        "active"
    } else {
        "dormant"
    };
    ctx.db
        .set_site_status(site_id, next_status)
        .await
        .map_err(ApiError::internal)?;

    let notice = format!("Site status updated to {}", next_status);
    Ok(Redirect::to(&format!(
        "/ui/app?admin_email={}&organization_id={}&project_id={}&view=sites&notice={}",
        query_escape(user.email.as_str()),
        site.organization_id,
        site.project_id.unwrap_or_default(),
        query_escape(&notice)
    )))
}

pub(crate) async fn submit_attach_site_to_project(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
    Path(site_id): Path<Uuid>,
    Form(form): Form<AttachSiteToProjectForm>,
) -> Result<Redirect, ApiError> {
    let site = ctx
        .db
        .get_site(site_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("site not found".to_string()))?;

    require_org_role(&user, site.organization_id, ROLE_ORG_MANAGERS)?;

    ctx.db
        .attach_site_to_project(site_id, form.project_id)
        .await
        .map_err(ApiError::internal)?;

    Ok(Redirect::to(&format!(
        "/ui/app?admin_email={}&organization_id={}&project_id={}&view=sites&notice={}",
        query_escape(user.email.as_str()),
        site.organization_id,
        form.project_id,
        query_escape("Site attached to study")
    )))
}

pub(crate) async fn submit_detach_site_from_project(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
    Path(site_id): Path<Uuid>,
    Form(form): Form<AttachSiteToProjectForm>, // reuse for admin_email
) -> Result<Redirect, ApiError> {
    let site = ctx
        .db
        .get_site(site_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("site not found".to_string()))?;

    require_org_role(&user, site.organization_id, ROLE_ORG_MANAGERS)?;

    ctx.db
        .detach_site_from_project(site_id)
        .await
        .map_err(ApiError::internal)?;

    Ok(Redirect::to(&format!(
        "/ui/app?admin_email={}&organization_id={}&view=sites&notice={}",
        query_escape(user.email.as_str()),
        site.organization_id,
        query_escape("Site detached from study (now org-level)")
    )))
}

pub(crate) async fn submit_reset_site_checklist(
    State(ctx): State<AppContext>,
    Path(site_id): Path<Uuid>,
    Form(form): Form<AppDeleteSiteForm>, // reuse for admin_email
) -> Result<Redirect, ApiError> {
    // Support dev bypass via the form's admin_email (same pattern as create-site forms)
    let user = if ctx.config.allow_dev_auth_bypass {
        let mut dev_headers = axum::http::HeaderMap::new();
        if let Ok(val) = form.admin_email.parse::<axum::http::HeaderValue>() {
            dev_headers.insert("x-dev-user-email", val);
        }
        match authenticate_from_dev_headers(&ctx, &dev_headers).await {
            Ok(u) => u,
            Err(e) => return Err(ApiError::Auth(e)),
        }
    } else {
        return Err(ApiError::Auth(AuthError::Unauthorized(
            "reset requires dev auth bypass or authenticated user".to_string(),
        )));
    };

    let site = ctx
        .db
        .get_site(site_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("site not found".to_string()))?;

    require_org_role(&user, site.organization_id, ROLE_ORG_MANAGERS)?;

    ctx.db
        .reset_site_startup_checklist_items(site_id)
        .await
        .map_err(ApiError::internal)?;

    Ok(Redirect::to(&format!(
        "/ui/app?admin_email={}&organization_id={}&view=sites&notice={}",
        query_escape(user.email.as_str()),
        site.organization_id,
        query_escape("Checklist reset to pending for this site")
    )))
}

pub(crate) async fn submit_app_project_toggle_dormancy(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
    Path(project_id): Path<Uuid>,
    Form(form): Form<AppToggleDormancyForm>,
) -> Result<Redirect, ApiError> {
    let project = ctx
        .db
        .get_project(project_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("project not found".to_string()))?;
    require_org_role(&user, project.organization_id, ROLE_ORG_MANAGERS)?;

    let next_status = if project.status == "dormant" {
        "active"
    } else {
        "dormant"
    };
    ctx.db
        .set_project_status(project_id, next_status)
        .await
        .map_err(ApiError::internal)?;

    let notice = format!("Study status updated to {}", next_status);
    Ok(Redirect::to(&format!(
        "/ui/app?admin_email={}&organization_id={}&project_id={}&view=projects&notice={}",
        query_escape(user.email.as_str()),
        project.organization_id,
        project.id,
        query_escape(&notice)
    )))
}

pub(crate) async fn submit_app_auto_archive(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
    Form(form): Form<AppToggleDormancyForm>,
) -> Result<Redirect, ApiError> {
    let days_threshold = 1095.0;
    let (sites_archived, projects_archived) = ctx
        .db
        .auto_archive_dormant_entities(days_threshold)
        .await
        .map_err(ApiError::internal)?;

    let notice = format!(
        "Inactivity scan completed: archived {} sites and {} studies with no activity for 3+ years.",
        sites_archived, projects_archived
    );
    Ok(Redirect::to(&format!(
        "/ui/app?admin_email={}&notice={}",
        query_escape(form.admin_email.trim()),
        query_escape(&notice)
    )))
}

pub(crate) async fn submit_app_create_patient(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
    Form(form): Form<AppCreatePatientForm>,
) -> Result<Redirect, ApiError> {
    let project_id = if form.project_id.trim().is_empty() {
        return Ok(Redirect::to(&format!(
            "/ui/app?admin_email={}&view=patients&notice={}",
            query_escape(user.email.as_str()),
            query_escape("Select an active study first, then create a patient.")
        )));
    } else {
        match parse_uuid_field(&form.project_id, "project_id") {
            Ok(pid) => pid,
            Err(_) => {
                return Ok(Redirect::to(&format!(
                    "/ui/app?admin_email={}&view=patients&notice={}",
                    query_escape(user.email.as_str()),
                    query_escape(
                        "Study context is invalid. Re-select the study and try patient creation again."
                    )
                )));
            }
        }
    };
    let project = ctx
        .db
        .get_project(project_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("project not found".to_string()))?;

    require_org_role(&user, project.organization_id, ROLE_COORDINATOR_OR_BETTER)?;

    // Site is now optional
    let site_id = if form.site_id.trim().is_empty() {
        None
    } else {
        match parse_uuid_field(&form.site_id, "site_id") {
            Ok(id) => Some(id),
            Err(_) => {
                return Ok(Redirect::to(&format!(
                    "/ui/app?admin_email={}&organization_id={}&project_id={}&view=patients&notice={}",
                    query_escape(user.email.as_str()),
                    project.organization_id,
                    project.id,
                    query_escape(
                        "Invalid site selection. Choose a site from the list, then retry patient enrollment."
                    )
                )));
            }
        }
    };
    if let Some(sid) = site_id {
        let site = ctx
            .db
            .get_site(sid)
            .await
            .map_err(ApiError::internal)?
            .ok_or_else(|| ApiError::NotFound("site not found".to_string()))?;
        if site.organization_id != project.organization_id {
            return Ok(Redirect::to(&format!(
                "/ui/app?admin_email={}&organization_id={}&project_id={}&view=patients&notice={}",
                query_escape(user.email.as_str()),
                project.organization_id,
                project.id,
                query_escape(
                    "Selected site belongs to a different organization. Please choose a site in this workspace."
                )
            )));
        }
        if let Some(site_project_id) = site.project_id {
            if site_project_id != project.id {
                return Ok(Redirect::to(&format!(
                    "/ui/app?admin_email={}&organization_id={}&project_id={}&view=sites&notice={}",
                    query_escape(user.email.as_str()),
                    project.organization_id,
                    project.id,
                    query_escape(
                        "Selected site is attached to another study. Attach it to this study first in Sites."
                    )
                )));
            }
        }
    }

    let external_subject_id = if form.external_subject_id.trim().is_empty() {
        None
    } else {
        Some(form.external_subject_id.trim())
    };
    let patient_email = if form.email.trim().is_empty() {
        None
    } else {
        Some(form.email.trim())
    };
    let date_of_birth = parse_optional_date(&form.date_of_birth)?;

    let patient = ctx
        .db
        .create_patient(
            project_id,
            site_id,
            external_subject_id,
            patient_email,
            date_of_birth,
        )
        .await
        .map_err(ApiError::internal)?;

    Ok(Redirect::to(&format!(
        "/ui/app?admin_email={}&organization_id={}&project_id={}&patient_id={}&notice={}",
        query_escape(user.email.as_str()),
        project.organization_id,
        project.id,
        patient.id,
        query_escape("Patient created successfully")
    )))
}

pub(crate) async fn submit_app_create_provider(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
    Form(form): Form<AppCreateProviderForm>,
) -> Result<Redirect, ApiError> {
    let organization_id = parse_uuid_field(&form.organization_id, "organization_id")?;
    require_org_role(&user, organization_id, ROLE_COORDINATOR_OR_BETTER)?;

    if form.provider_name.trim().is_empty() {
        return Err(ApiError::Validation(
            "provider_name is required".to_string(),
        ));
    }

    ctx.db
        .create_provider(
            organization_id,
            form.provider_name.trim(),
            form.provider_title.trim(),
            form.referral_source.trim(),
            form.email
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty()),
            form.phone_number
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty()),
            form.npi_number
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty()),
            form.address
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty()),
            form.notes
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty()),
        )
        .await
        .map_err(ApiError::internal)?;

    Ok(Redirect::to(&format!(
        "/ui/app?admin_email={}&organization_id={}&notice={}",
        query_escape(user.email.as_str()),
        organization_id,
        query_escape("Provider created successfully")
    )))
}

pub(crate) async fn submit_app_create_encounter(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
    Form(form): Form<AppCreateEncounterForm>,
) -> Result<Redirect, ApiError> {
    let patient_id = parse_uuid_field(&form.patient_id, "patient_id")?;
    let patient = ctx
        .db
        .get_patient(patient_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("patient not found".to_string()))?;

    require_org_role(&user, patient.organization_id, ROLE_COORDINATOR_OR_BETTER)?;
    let provider_id = if form.provider_id.trim().is_empty() {
        None
    } else {
        let p_id = parse_uuid_field(&form.provider_id, "provider_id")?;
        let provider = ctx
            .db
            .get_provider(p_id)
            .await
            .map_err(ApiError::internal)?
            .ok_or_else(|| ApiError::NotFound("provider not found".to_string()))?;
        if provider.organization_id != patient.organization_id {
            return Err(ApiError::Auth(AuthError::Forbidden(
                "provider does not belong to the patient's organization".to_string(),
            )));
        }
        Some(p_id)
    };
    ctx.db
        .create_encounter(
            patient_id,
            form.encounter_type.trim(),
            provider_id,
            form.notes.trim(),
        )
        .await
        .map_err(ApiError::internal)?;
    Ok(Redirect::to(&format!(
        "/ui/app?admin_email={}&organization_id={}&project_id={}&patient_id={}&notice={}",
        query_escape(user.email.as_str()),
        patient.organization_id,
        patient.project_id,
        patient.id,
        query_escape("Encounter created successfully")
    )))
}

pub(crate) async fn submit_app_send_invite(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
    Form(form): Form<AppSendInviteForm>,
) -> Result<Redirect, ApiError> {
    let organization_id = parse_uuid_field(&form.organization_id, "organization_id")?;
    let project_id = if form.project_id.trim().is_empty() {
        return Ok(Redirect::to(&format!(
            "/ui/app?admin_email={}&organization_id={}&view=patients&notice={}",
            query_escape(user.email.as_str()),
            organization_id,
            query_escape("Select an active study before sending patient form invites.")
        )));
    } else {
        match parse_uuid_field(&form.project_id, "project_id") {
            Ok(pid) => pid,
            Err(_) => {
                return Ok(Redirect::to(&format!(
                    "/ui/app?admin_email={}&organization_id={}&view=patients&notice={}",
                    query_escape(user.email.as_str()),
                    organization_id,
                    query_escape("Study context is invalid. Re-select the study and retry invite.")
                )));
            }
        }
    };

    require_org_role(&user, organization_id, ROLE_ORG_MANAGERS)?;
    let project = ctx
        .db
        .get_project(project_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("project not found".to_string()))?;
    if project.organization_id != organization_id {
        return Ok(Redirect::to(&format!(
            "/ui/app?admin_email={}&organization_id={}&view=patients&notice={}",
            query_escape(user.email.as_str()),
            organization_id,
            query_escape(
                "Selected study does not belong to the current organization. Re-select context and retry invite."
            )
        )));
    }

    ctx.db
        .create_form_invite(
            organization_id,
            project_id,
            form.patient_email.trim(),
            form.form_type.trim(),
        )
        .await
        .map_err(ApiError::internal)?;

    Ok(Redirect::to(&format!(
        "/ui/app?admin_email={}&organization_id={}&project_id={}&notice={}",
        query_escape(user.email.as_str()),
        organization_id,
        project_id,
        query_escape("Form invite sent")
    )))
}

pub(crate) async fn submit_app_create_media_ticket(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
    Form(form): Form<AppCreateMediaTicketForm>,
) -> Result<Response, ApiError> {
    let organization_id = parse_uuid_field(&form.organization_id, "organization_id")?;
    let project_id = if form.project_id.trim().is_empty() {
        return Ok(Redirect::to(&format!(
            "/ui/app?admin_email={}&organization_id={}&view=patients&notice={}",
            query_escape(user.email.as_str()),
            organization_id,
            query_escape("Select an active study before generating media upload links.")
        ))
        .into_response());
    } else {
        match parse_uuid_field(&form.project_id, "project_id") {
            Ok(pid) => pid,
            Err(_) => {
                return Ok(Redirect::to(&format!(
                    "/ui/app?admin_email={}&organization_id={}&view=patients&notice={}",
                    query_escape(user.email.as_str()),
                    organization_id,
                    query_escape(
                        "Study context is invalid. Re-select the study and retry media link generation."
                    )
                ))
                .into_response());
            }
        }
    };

    require_org_role(&user, organization_id, ROLE_COORDINATOR_OR_BETTER)?;
    let project = ctx
        .db
        .get_project(project_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("project not found".to_string()))?;
    if project.organization_id != organization_id {
        return Ok(Redirect::to(&format!(
            "/ui/app?admin_email={}&organization_id={}&view=patients&notice={}",
            query_escape(user.email.as_str()),
            organization_id,
            query_escape(
                "Selected study does not belong to this organization. Re-select study context first."
            )
        ))
        .into_response());
    }
    let patient_uuid = match parse_uuid_field(&form.patient_id, "patient_id") {
        Ok(pid) => pid,
        Err(_) => {
            return Ok(Redirect::to(&format!(
                "/ui/app?admin_email={}&organization_id={}&project_id={}&view=patients&notice={}",
                query_escape(user.email.as_str()),
                organization_id,
                project_id,
                query_escape("Select a valid patient from the dropdown before generating media upload links.")
            ))
            .into_response());
        }
    };
    let patient = ctx
        .db
        .get_patient(patient_uuid)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("patient not found".to_string()))?;
    if patient.organization_id != organization_id || patient.project_id != project_id {
        return Ok(Redirect::to(&format!(
            "/ui/app?admin_email={}&organization_id={}&project_id={}&view=patients&notice={}",
            query_escape(user.email.as_str()),
            organization_id,
            project_id,
            query_escape(
                "Patient is outside the selected study context. Re-select patient after choosing the active study."
            )
        ))
        .into_response());
    }
    let ticket = ctx
        .db
        .create_media_upload_ticket(
            organization_id,
            project_id,
            &patient.id.to_string(),
            form.mime_type.trim(),
        )
        .await
        .map_err(ApiError::internal)?;

    let body = format!(
        r#"<section class="card">
  <h1>Media Upload Ticket Created</h1>
  <p><strong>Ticket ID:</strong> {}</p>
  <p><strong>Upload URL:</strong> <a href="{}">{}</a></p>
  <p><strong>Expires At:</strong> {}</p>
  <p><a href="/ui/app?admin_email={}&organization_id={}&project_id={}">Back to app dashboard</a></p>
</section>"#,
        ticket.id,
        html_escape(&ticket.upload_url),
        html_escape(&ticket.upload_url),
        ticket.expires_at,
        query_escape(user.email.as_str()),
        organization_id,
        project_id
    );
    Ok(Html(render_cingulum_page("Media Upload Ticket Created", body)).into_response())
}

