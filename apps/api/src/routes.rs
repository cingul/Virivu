use axum::{
    extract::{Form, Path, Query, Request, State},
    http::{header, HeaderValue, StatusCode},
    middleware::{from_fn_with_state, Next},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use uuid::Uuid;

use crate::{
    auth::{extract_bearer_token, verify_google_workspace_user, AuthError, AuthenticatedUser},
    config::Config,
    db::Db,
    models::{DataUseAgreement, DataUseAgreementSignature, OutboundEmail},
};

const ROLE_PLATFORM_ADMIN: &[&str] = &["platform_admin"];
const ROLE_ORG_MANAGERS: &[&str] = &["platform_admin", "org_admin"];
const ROLE_COORDINATOR_OR_BETTER: &[&str] = &[
    "platform_admin",
    "org_admin",
    "site_coordinator",
    "investigator",
];
const ROLE_ANALYTICS: &[&str] = &[
    "platform_admin",
    "org_admin",
    "investigator",
    "site_coordinator",
    "analyst",
];

#[derive(Clone)]
pub struct AppContext {
    pub config: Config,
    pub db: Db,
}

pub fn router(ctx: AppContext) -> Router {
    let public_router = Router::new()
        .route("/health", get(health))
        .route("/ui", get(redirect_ui_home))
        .route("/ui/app", get(render_app_dashboard))
        .route(
            "/ui/app/create-organization",
            post(submit_app_create_organization),
        )
        .route("/ui/app/create-project", post(submit_app_create_project))
        .route("/ui/app/create-site", post(submit_app_create_site))
        .route("/ui/app/create-patient", post(submit_app_create_patient))
        .route("/ui/app/create-provider", post(submit_app_create_provider))
        .route(
            "/ui/app/create-encounter",
            post(submit_app_create_encounter),
        )
        .route("/ui/app/send-invite", post(submit_app_send_invite))
        .route(
            "/ui/app/create-media-ticket",
            post(submit_app_create_media_ticket),
        )
        .route(
            "/v1/auth/google/token-introspect",
            post(google_token_introspect),
        )
        .route("/ui/dua", get(render_dua_admin_page))
        .route(
            "/ui/dua/create-organization",
            post(submit_create_organization_from_ui),
        )
        .route("/ui/dua/open-agreement", post(open_dua_agreement_workspace))
        .route("/ui/dua/draft", post(render_create_dua_from_form))
        .route(
            "/ui/dua/sign/{signing_token}",
            get(render_dua_hospital_sign_page),
        )
        .route(
            "/ui/dua/sign/{signing_token}",
            post(submit_dua_hospital_sign_form),
        )
        .route("/ui/dua/{agreement_id}", get(render_dua_agreement_page))
        .route(
            "/ui/dua/{agreement_id}/send-hospital-link",
            post(submit_dua_send_hospital_link_form),
        )
        .route(
            "/ui/dua/{agreement_id}/export.pdf",
            get(download_data_use_agreement_pdf_ui),
        )
        .route(
            "/ui/dua/{agreement_id}/sign-cingulum",
            post(submit_dua_cingulum_sign_form),
        )
        .route(
            "/v1/legal/data-use-agreements/sign-hospital",
            post(sign_data_use_agreement_hospital),
        );

    let protected_router = Router::new()
        .route("/v1/organizations", post(create_organization))
        .route("/v1/projects", post(create_project))
        .route("/v1/sites", post(create_site))
        .route("/v1/patients", post(create_patient))
        .route("/v1/providers", post(create_provider))
        .route("/v1/encounters", post(create_encounter))
        .route(
            "/v1/legal/data-use-agreements",
            post(create_data_use_agreement),
        )
        .route(
            "/v1/legal/organizations/{org_id}/data-use-agreements",
            get(list_data_use_agreements),
        )
        .route(
            "/v1/legal/data-use-agreements/{agreement_id}",
            get(get_data_use_agreement),
        )
        .route(
            "/v1/legal/data-use-agreements/{agreement_id}/send-hospital-sign-link",
            post(send_hospital_signing_link_email),
        )
        .route(
            "/v1/legal/data-use-agreements/{agreement_id}/emails",
            get(list_data_use_agreement_emails),
        )
        .route(
            "/v1/legal/data-use-agreements/{agreement_id}/export.pdf",
            get(download_data_use_agreement_pdf),
        )
        .route(
            "/v1/legal/data-use-agreements/{agreement_id}/sign-cingulum",
            post(sign_data_use_agreement_cingulum),
        )
        .route("/v1/forms/send-invite", post(send_form_invite))
        .route("/v1/media/presign-upload", post(presign_media_upload))
        .route(
            "/v1/analytics/organizations/{org_id}/summary",
            get(organization_summary),
        )
        .route(
            "/v1/reports/projects/{project_id}/progress",
            get(project_progress_report),
        )
        .route(
            "/v1/agents/doctor-patient-transcript",
            post(generate_doctor_patient_note),
        )
        .route_layer(from_fn_with_state(ctx.clone(), require_auth));

    public_router.merge(protected_router).with_state(ctx)
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
    service: String,
    timestamp_utc: String,
}

async fn health(State(ctx): State<AppContext>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        service: ctx.config.app_name,
        timestamp_utc: Utc::now().to_rfc3339(),
    })
}

async fn require_auth(State(ctx): State<AppContext>, mut request: Request, next: Next) -> Response {
    let maybe_bearer = extract_bearer_token(request.headers());
    let auth_result = if let Some(token) = maybe_bearer {
        verify_google_workspace_user(&ctx.config, &ctx.db, &token).await
    } else if ctx.config.allow_dev_auth_bypass {
        authenticate_from_dev_headers(&ctx, request.headers()).await
    } else {
        Err(AuthError::Unauthorized(
            "missing bearer token in Authorization header".to_string(),
        ))
    };

    match auth_result {
        Ok(user) => {
            request.extensions_mut().insert(user);
            next.run(request).await
        }
        Err(err) => err.into_response(),
    }
}

async fn authenticate_from_dev_headers(
    ctx: &AppContext,
    headers: &axum::http::HeaderMap,
) -> Result<AuthenticatedUser, AuthError> {
    let email = headers
        .get("x-dev-user-email")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| {
            AuthError::Unauthorized(
                "no bearer token supplied and x-dev-user-email is missing".to_string(),
            )
        })?;

    let display_name = headers
        .get("x-dev-user-name")
        .and_then(|v| v.to_str().ok())
        .unwrap_or(email);
    let google_subject = format!("dev-{}", email);

    let user = ctx
        .db
        .upsert_user(email, &google_subject, display_name)
        .await
        .map_err(|e| AuthError::Internal(format!("unable to create dev user: {e}")))?;

    let memberships = ctx
        .db
        .load_memberships_for_email(email)
        .await
        .map_err(|e| AuthError::Internal(format!("unable to load user memberships: {e}")))?;

    Ok(AuthenticatedUser {
        user_id: user.id,
        email: user.email,
        google_subject: user.google_subject,
        display_name: user.display_name,
        domain: ctx.config.allowed_google_workspace_domain.clone(),
        memberships,
    })
}

#[derive(Debug, Deserialize)]
struct GoogleIntrospectRequest {
    id_token: String,
}

async fn google_token_introspect(
    State(ctx): State<AppContext>,
    Json(payload): Json<GoogleIntrospectRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let user = verify_google_workspace_user(&ctx.config, &ctx.db, payload.id_token.trim())
        .await
        .map_err(ApiError::Auth)?;
    Ok((StatusCode::OK, Json(user)))
}

#[derive(Debug, Deserialize)]
struct CreateOrganizationRequest {
    name: String,
}

async fn create_organization(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
    Json(payload): Json<CreateOrganizationRequest>,
) -> Result<impl IntoResponse, ApiError> {
    require_platform_role(&user, ROLE_PLATFORM_ADMIN)?;

    if payload.name.trim().is_empty() {
        return Err(ApiError::Validation(
            "organization name is required".to_string(),
        ));
    }

    let org = ctx
        .db
        .create_organization(payload.name.trim())
        .await
        .map_err(ApiError::internal)?;
    Ok((StatusCode::CREATED, Json(org)))
}

#[derive(Debug, Deserialize)]
struct CreateProjectRequest {
    organization_id: Uuid,
    name: String,
    therapeutic_area: String,
}

async fn create_project(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
    Json(payload): Json<CreateProjectRequest>,
) -> Result<impl IntoResponse, ApiError> {
    require_org_role(&user, payload.organization_id, ROLE_ORG_MANAGERS)?;

    if payload.name.trim().is_empty() {
        return Err(ApiError::Validation("project name is required".to_string()));
    }

    let project = ctx
        .db
        .create_project(
            payload.organization_id,
            payload.name.trim(),
            payload.therapeutic_area.trim(),
        )
        .await
        .map_err(ApiError::internal)?;
    Ok((StatusCode::CREATED, Json(project)))
}

#[derive(Debug, Deserialize)]
struct CreateSiteRequest {
    project_id: Uuid,
    name: String,
    principal_investigator: String,
}

async fn create_site(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
    Json(payload): Json<CreateSiteRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let project = ctx
        .db
        .get_project(payload.project_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("project not found".to_string()))?;
    require_org_role(&user, project.organization_id, ROLE_ORG_MANAGERS)?;

    let site = ctx
        .db
        .create_site(
            payload.project_id,
            payload.name.trim(),
            payload.principal_investigator.trim(),
        )
        .await
        .map_err(ApiError::internal)?;

    Ok((StatusCode::CREATED, Json(site)))
}

#[derive(Debug, Deserialize)]
struct CreatePatientRequest {
    site_id: Uuid,
    external_subject_id: Option<String>,
    email: Option<String>,
    date_of_birth: Option<chrono::NaiveDate>,
}

async fn create_patient(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
    Json(payload): Json<CreatePatientRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let site = ctx
        .db
        .get_site(payload.site_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("site not found".to_string()))?;
    let project = ctx
        .db
        .get_project(site.project_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("project not found".to_string()))?;
    require_org_role(&user, project.organization_id, ROLE_COORDINATOR_OR_BETTER)?;

    let patient = ctx
        .db
        .create_patient(
            payload.site_id,
            payload.external_subject_id.as_deref().map(str::trim),
            payload.email.as_deref().map(str::trim),
            payload.date_of_birth,
        )
        .await
        .map_err(ApiError::internal)?;
    Ok((StatusCode::CREATED, Json(patient)))
}

#[derive(Debug, Deserialize)]
struct CreateProviderRequest {
    organization_id: Uuid,
    name: String,
    title: Option<String>,
    referral_source: Option<String>,
}

async fn create_provider(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
    Json(payload): Json<CreateProviderRequest>,
) -> Result<impl IntoResponse, ApiError> {
    require_org_role(&user, payload.organization_id, ROLE_ORG_MANAGERS)?;
    if payload.name.trim().is_empty() {
        return Err(ApiError::Validation(
            "provider name is required".to_string(),
        ));
    }
    let provider = ctx
        .db
        .create_provider(
            payload.organization_id,
            payload.name.trim(),
            payload.title.as_deref().unwrap_or("").trim(),
            payload.referral_source.as_deref().unwrap_or("").trim(),
        )
        .await
        .map_err(ApiError::internal)?;
    Ok((StatusCode::CREATED, Json(provider)))
}

#[derive(Debug, Deserialize)]
struct CreateEncounterRequest {
    patient_id: Uuid,
    encounter_type: String,
    provider_id: Option<Uuid>,
    notes: Option<String>,
}

async fn create_encounter(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
    Json(payload): Json<CreateEncounterRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let patient = ctx
        .db
        .get_patient(payload.patient_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("patient not found".to_string()))?;
    require_org_role(&user, patient.organization_id, ROLE_COORDINATOR_OR_BETTER)?;

    let encounter = ctx
        .db
        .create_encounter(
            payload.patient_id,
            payload.encounter_type.trim(),
            payload.provider_id,
            payload.notes.as_deref().unwrap_or("").trim(),
        )
        .await
        .map_err(ApiError::internal)?;
    Ok((StatusCode::CREATED, Json(encounter)))
}

#[derive(Debug, Deserialize)]
struct SendFormInviteRequest {
    organization_id: Uuid,
    project_id: Uuid,
    patient_email: String,
    form_type: String,
}

async fn send_form_invite(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
    Json(payload): Json<SendFormInviteRequest>,
) -> Result<impl IntoResponse, ApiError> {
    require_org_role(&user, payload.organization_id, ROLE_COORDINATOR_OR_BETTER)?;

    let invite = ctx
        .db
        .create_form_invite(
            payload.organization_id,
            payload.project_id,
            payload.patient_email.trim(),
            payload.form_type.trim(),
        )
        .await
        .map_err(ApiError::internal)?;
    Ok((StatusCode::CREATED, Json(invite)))
}

#[derive(Debug, Deserialize)]
struct PresignMediaUploadRequest {
    organization_id: Uuid,
    project_id: Uuid,
    patient_id: String,
    mime_type: String,
}

async fn presign_media_upload(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
    Json(payload): Json<PresignMediaUploadRequest>,
) -> Result<impl IntoResponse, ApiError> {
    require_org_role(&user, payload.organization_id, ROLE_COORDINATOR_OR_BETTER)?;

    let ticket = ctx
        .db
        .create_media_upload_ticket(
            payload.organization_id,
            payload.project_id,
            payload.patient_id.trim(),
            payload.mime_type.trim(),
        )
        .await
        .map_err(ApiError::internal)?;
    Ok((StatusCode::CREATED, Json(ticket)))
}

#[derive(Debug, Serialize)]
struct OrganizationSummary {
    organization_id: Uuid,
    projects: i64,
    sites: i64,
    sent_form_invites: i64,
    generated_media_upload_links: i64,
}

async fn organization_summary(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
    Path(org_id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    require_org_role(&user, org_id, ROLE_ANALYTICS)?;
    let summary = ctx
        .db
        .organization_summary(org_id)
        .await
        .map_err(ApiError::internal)?;

    Ok(Json(OrganizationSummary {
        organization_id: org_id,
        projects: summary.projects,
        sites: summary.sites,
        sent_form_invites: summary.sent_form_invites,
        generated_media_upload_links: summary.generated_media_upload_links,
    }))
}

#[derive(Debug, Serialize)]
struct ProjectProgressReport {
    project_id: Uuid,
    total_sites: i64,
    total_form_invites: i64,
    total_media_captures_requested: i64,
    report_generated_at: String,
}

async fn project_progress_report(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
    Path(project_id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let project = ctx
        .db
        .get_project(project_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("project not found".to_string()))?;
    require_org_role(&user, project.organization_id, ROLE_ANALYTICS)?;

    let report = ctx
        .db
        .project_progress_report(project_id)
        .await
        .map_err(ApiError::internal)?;

    Ok(Json(ProjectProgressReport {
        project_id,
        total_sites: report.total_sites,
        total_form_invites: report.total_form_invites,
        total_media_captures_requested: report.total_media_captures_requested,
        report_generated_at: Utc::now().to_rfc3339(),
    }))
}

#[derive(Debug, Default, Deserialize)]
struct AppDashboardQuery {
    admin_email: Option<String>,
    organization_id: Option<String>,
    project_id: Option<String>,
    patient_id: Option<String>,
    notice: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AppCreateOrganizationForm {
    admin_email: String,
    organization_name: String,
}

#[derive(Debug, Deserialize)]
struct AppCreateProjectForm {
    admin_email: String,
    organization_id: String,
    project_name: String,
    therapeutic_area: String,
}

#[derive(Debug, Deserialize)]
struct AppCreateSiteForm {
    admin_email: String,
    project_id: String,
    site_name: String,
    principal_investigator: String,
}

#[derive(Debug, Deserialize)]
struct AppCreatePatientForm {
    admin_email: String,
    site_id: String,
    external_subject_id: String,
    email: String,
    date_of_birth: String,
}

#[derive(Debug, Deserialize)]
struct AppCreateProviderForm {
    admin_email: String,
    organization_id: String,
    provider_name: String,
    provider_title: String,
    referral_source: String,
}

#[derive(Debug, Deserialize)]
struct AppCreateEncounterForm {
    admin_email: String,
    patient_id: String,
    encounter_type: String,
    provider_id: String,
    notes: String,
}

#[derive(Debug, Deserialize)]
struct AppSendInviteForm {
    admin_email: String,
    organization_id: String,
    project_id: String,
    patient_email: String,
    form_type: String,
}

#[derive(Debug, Deserialize)]
struct AppCreateMediaTicketForm {
    admin_email: String,
    organization_id: String,
    project_id: String,
    patient_id: String,
    mime_type: String,
}

async fn redirect_ui_home() -> Redirect {
    Redirect::to("/ui/app")
}

async fn render_app_dashboard(
    State(ctx): State<AppContext>,
    Query(query): Query<AppDashboardQuery>,
) -> Result<Html<String>, ApiError> {
    let admin_email = query
        .admin_email
        .unwrap_or_else(|| "arcot@cingulum.org".to_string());
    let admin_email_q = query_escape(admin_email.trim());

    let organizations = ctx
        .db
        .list_organizations_for_email(admin_email.trim())
        .await
        .map_err(ApiError::internal)?;

    let selected_org_id = query
        .organization_id
        .as_deref()
        .and_then(|raw| raw.parse::<Uuid>().ok())
        .or_else(|| organizations.first().map(|org| org.id));

    let projects = if let Some(org_id) = selected_org_id {
        ctx.db
            .list_projects_by_organization(org_id)
            .await
            .map_err(ApiError::internal)?
    } else {
        Vec::new()
    };

    let selected_project_id = query
        .project_id
        .as_deref()
        .and_then(|raw| raw.parse::<Uuid>().ok())
        .filter(|pid| projects.iter().any(|project| project.id == *pid))
        .or_else(|| projects.first().map(|project| project.id));

    let sites = if let Some(project_id) = selected_project_id {
        ctx.db
            .list_sites_by_project(project_id)
            .await
            .map_err(ApiError::internal)?
    } else {
        Vec::new()
    };

    let duas = if let Some(org_id) = selected_org_id {
        ctx.db
            .list_data_use_agreements(org_id)
            .await
            .map_err(ApiError::internal)?
    } else {
        Vec::new()
    };

    let org_summary = if let Some(org_id) = selected_org_id {
        Some(
            ctx.db
                .organization_summary(org_id)
                .await
                .map_err(ApiError::internal)?,
        )
    } else {
        None
    };

    let project_report = if let Some(project_id) = selected_project_id {
        Some(
            ctx.db
                .project_progress_report(project_id)
                .await
                .map_err(ApiError::internal)?,
        )
    } else {
        None
    };

    let patients = if let Some(project_id) = selected_project_id {
        ctx.db
            .list_patients_by_project(project_id)
            .await
            .map_err(ApiError::internal)?
    } else {
        Vec::new()
    };

    let providers = if let Some(org_id) = selected_org_id {
        ctx.db
            .list_providers_by_organization(org_id)
            .await
            .map_err(ApiError::internal)?
    } else {
        Vec::new()
    };

    let selected_patient_id = query
        .patient_id
        .as_deref()
        .and_then(|raw| raw.parse::<Uuid>().ok())
        .filter(|pid| patients.iter().any(|patient| patient.id == *pid))
        .or_else(|| patients.first().map(|patient| patient.id));

    let encounters = if let Some(patient_id) = selected_patient_id {
        ctx.db
            .list_encounters_by_patient(patient_id)
            .await
            .map_err(ApiError::internal)?
    } else {
        Vec::new()
    };

    let notice_html = query
        .notice
        .map(|notice| format!(r#"<p class="notice">{}</p>"#, html_escape(notice.trim())))
        .unwrap_or_default();

    let organizations_html = if organizations.is_empty() {
        "<li>No organizations available yet.</li>".to_string()
    } else {
        organizations
            .iter()
            .map(|org| {
                format!(
                    r#"<li><a href="/ui/app?admin_email={}&organization_id={}">{}</a> <small>(id: {} · hex: {})</small></li>"#,
                    admin_email_q,
                    org.id,
                    html_escape(&org.name),
                    org.id,
                    html_escape(org.hex_code.as_deref().unwrap_or("pending"))
                )
            })
            .collect::<Vec<_>>()
            .join("")
    };

    let projects_html = if projects.is_empty() {
        "<li>No projects yet for selected organization.</li>".to_string()
    } else {
        projects
            .iter()
            .map(|project| {
                let selected_org = selected_org_id
                    .map(|org_id| format!("&organization_id={}", org_id))
                    .unwrap_or_default();
                format!(
                    r#"<li><a href="/ui/app?admin_email={}{}&project_id={}">{}</a> <small>(area: {} · hex: {})</small></li>"#,
                    admin_email_q,
                    selected_org,
                    project.id,
                    html_escape(&project.name),
                    html_escape(&project.therapeutic_area),
                    html_escape(project.hex_code.as_deref().unwrap_or("pending"))
                )
            })
            .collect::<Vec<_>>()
            .join("")
    };

    let sites_html = if sites.is_empty() {
        "<li>No sites yet for selected project.</li>".to_string()
    } else {
        sites
            .iter()
            .map(|site| {
                format!(
                    "<li><strong>{}</strong> <small>(PI: {} · hex: {} · id: {})</small></li>",
                    html_escape(&site.name),
                    html_escape(&site.principal_investigator),
                    html_escape(site.hex_code.as_deref().unwrap_or("pending")),
                    site.id
                )
            })
            .collect::<Vec<_>>()
            .join("")
    };

    let dua_html = if duas.is_empty() {
        "<li>No DUAs yet for selected organization.</li>".to_string()
    } else {
        duas.iter()
            .take(8)
            .map(|dua| {
                format!(
                    r#"<li><a href="/ui/dua/{}">{}</a> <span class="status-chip">{}</span></li>"#,
                    dua.id,
                    html_escape(&dua.hospital_name),
                    html_escape(&dua.status)
                )
            })
            .collect::<Vec<_>>()
            .join("")
    };

    let selected_org_value = selected_org_id.map(|id| id.to_string()).unwrap_or_default();
    let selected_project_value = selected_project_id
        .map(|id| id.to_string())
        .unwrap_or_default();
    let selected_patient_value = selected_patient_id
        .map(|id| id.to_string())
        .unwrap_or_default();

    let org_summary_html = if let Some(summary) = org_summary {
        format!(
            "<p><strong>Projects:</strong> {} · <strong>Sites:</strong> {} · <strong>Form Invites:</strong> {} · <strong>Media Links:</strong> {}</p>",
            summary.projects, summary.sites, summary.sent_form_invites, summary.generated_media_upload_links
        )
    } else {
        "<p class=\"muted\">Select an organization to view summary.</p>".to_string()
    };

    let project_summary_html = if let Some(summary) = project_report {
        format!(
            "<p><strong>Sites:</strong> {} · <strong>Form Invites:</strong> {} · <strong>Media Capture Requests:</strong> {}</p>",
            summary.total_sites, summary.total_form_invites, summary.total_media_captures_requested
        )
    } else {
        "<p class=\"muted\">Select a project to view summary.</p>".to_string()
    };

    let patients_html = if patients.is_empty() {
        "<li>No patients yet for selected project.</li>".to_string()
    } else {
        patients
            .iter()
            .take(12)
            .map(|patient| {
                let selected_org = selected_org_id
                    .map(|org_id| format!("&organization_id={org_id}"))
                    .unwrap_or_default();
                let selected_project = selected_project_id
                    .map(|project_id| format!("&project_id={project_id}"))
                    .unwrap_or_default();
                format!(
                    r#"<li><a href="/ui/app?admin_email={}{}{}&patient_id={}">{}</a> <small>(id: {} · site: {})</small></li>"#,
                    admin_email_q,
                    selected_org,
                    selected_project,
                    patient.id,
                    html_escape(patient.hex_code.as_deref().unwrap_or("pending")),
                    patient.id,
                    patient
                        .site_id
                        .map(|id| id.to_string())
                        .unwrap_or_else(|| "none".to_string())
                )
            })
            .collect::<Vec<_>>()
            .join("")
    };

    let providers_html = if providers.is_empty() {
        "<li>No providers yet for selected organization.</li>".to_string()
    } else {
        providers
            .iter()
            .take(12)
            .map(|provider| {
                format!(
                    "<li><strong>{}</strong> <small>(hex: {} · title: {})</small></li>",
                    html_escape(&provider.name),
                    html_escape(&provider.hex_code),
                    html_escape(&provider.title)
                )
            })
            .collect::<Vec<_>>()
            .join("")
    };

    let encounters_html = if encounters.is_empty() {
        "<li>No encounters yet for selected patient.</li>".to_string()
    } else {
        encounters
            .iter()
            .take(16)
            .map(|encounter| {
                format!(
                    "<li><strong>{}</strong> <small>(type: {} · provider: {})</small></li>",
                    html_escape(&encounter.hex_code),
                    html_escape(&encounter.encounter_type),
                    encounter
                        .provider_id
                        .map(|id| id.to_string())
                        .unwrap_or_else(|| "none".to_string())
                )
            })
            .collect::<Vec<_>>()
            .join("")
    };

    let body = format!(
        r#"
<h1>Virivu Research Web App</h1>
<p class="muted">Unified operations workspace: institutions, trial setup, patient workflows, legal agreements, and analytics.</p>
{}
<section class="card">
  <h2>Workspace Context</h2>
  <p><strong>Admin:</strong> {}</p>
  <p><strong>Organization:</strong> {}</p>
  <p><strong>Project:</strong> {}</p>
  <p><a href="/ui/dua?admin_email={}">Open dedicated DUA console</a></p>
</section>

<section class="card">
  <h2>1) Organizations</h2>
  <form method="post" action="/ui/app/create-organization">
    <label>Admin email (platform admin)</label>
    <input name="admin_email" value="{}" required />
    <label>Organization legal name</label>
    <input name="organization_name" placeholder="Cingulum Foundation Inc." required />
    <button type="submit">Create Organization</button>
  </form>
  <h3 style="margin-top:1rem;">Available organizations</h3>
  <ul>{}</ul>
</section>

<section class="card">
  <h2>2) Project Setup</h2>
  <form method="post" action="/ui/app/create-project">
    <label>Admin email</label>
    <input name="admin_email" value="{}" required />
    <label>Organization ID</label>
    <input name="organization_id" value="{}" required />
    <label>Project name</label>
    <input name="project_name" placeholder="Stroke Registry 2026" required />
    <label>Therapeutic area</label>
    <input name="therapeutic_area" placeholder="Neurology" required />
    <button type="submit">Create Project</button>
  </form>
  <h3 style="margin-top:1rem;">Projects</h3>
  <ul>{}</ul>
</section>

<section class="card">
  <h2>3) Site Setup</h2>
  <form method="post" action="/ui/app/create-site">
    <label>Admin email</label>
    <input name="admin_email" value="{}" required />
    <label>Project ID</label>
    <input name="project_id" value="{}" required />
    <label>Site name</label>
    <input name="site_name" placeholder="North Campus Site A" required />
    <label>Principal investigator</label>
    <input name="principal_investigator" placeholder="Dr. Example" required />
    <button type="submit">Create Site</button>
  </form>
  <h3 style="margin-top:1rem;">Sites</h3>
  <ul>{}</ul>
</section>

<section class="card">
  <h2>4) Patient Workflow</h2>
  <form method="post" action="/ui/app/create-patient" style="margin-bottom:1rem;">
    <label>Admin email</label>
    <input name="admin_email" value="{}" required />
    <label>Site ID</label>
    <input name="site_id" placeholder="site-uuid" required />
    <label>External subject label (optional)</label>
    <input name="external_subject_id" placeholder="SUBJ-001" />
    <label>Patient email (optional)</label>
    <input type="email" name="email" placeholder="patient@example.org" />
    <label>Date of birth (optional, YYYY-MM-DD)</label>
    <input name="date_of_birth" placeholder="1980-01-01" />
    <button type="submit">Create Patient + Cascading Hex ID</button>
  </form>

  <form method="post" action="/ui/app/send-invite" style="margin-bottom:1rem;">
    <label>Admin email</label>
    <input name="admin_email" value="{}" required />
    <label>Organization ID</label>
    <input name="organization_id" value="{}" required />
    <label>Project ID</label>
    <input name="project_id" value="{}" required />
    <label>Patient email</label>
    <input type="email" name="patient_email" placeholder="patient@example.org" required />
    <label>Form type</label>
    <input name="form_type" placeholder="demographics-intake" required />
    <button type="submit">Send Patient Form Invite</button>
  </form>

  <form method="post" action="/ui/app/create-media-ticket">
    <label>Admin email</label>
    <input name="admin_email" value="{}" required />
    <label>Organization ID</label>
    <input name="organization_id" value="{}" required />
    <label>Project ID</label>
    <input name="project_id" value="{}" required />
    <label>Patient ID</label>
    <input name="patient_id" placeholder="subject-001" required />
    <label>MIME type</label>
    <input name="mime_type" placeholder="video/mp4" required />
    <button type="submit">Generate Media Upload Link</button>
  </form>

  <h3 style="margin-top:1rem;">Patients</h3>
  <ul>{}</ul>
</section>

<section class="card">
  <h2>5) Providers + Encounters</h2>
  <form method="post" action="/ui/app/create-provider" style="margin-bottom:1rem;">
    <label>Admin email</label>
    <input name="admin_email" value="{}" required />
    <label>Organization ID</label>
    <input name="organization_id" value="{}" required />
    <label>Provider name</label>
    <input name="provider_name" placeholder="Dr. Jane Doe" required />
    <label>Provider title</label>
    <input name="provider_title" placeholder="Cardiology" />
    <label>Referral source</label>
    <input name="referral_source" placeholder="External referral network" />
    <button type="submit">Register Provider + Hex Block</button>
  </form>

  <form method="post" action="/ui/app/create-encounter">
    <label>Admin email</label>
    <input name="admin_email" value="{}" required />
    <label>Patient ID</label>
    <input name="patient_id" value="{}" placeholder="patient-uuid" required />
    <label>Encounter type</label>
    <select name="encounter_type" required>
      <option value="outpatient">outpatient (000-2FF)</option>
      <option value="inpatient">inpatient (300-5FF)</option>
      <option value="labs">labs (600-8FF)</option>
      <option value="imaging">imaging (900-BFF)</option>
      <option value="procedures">procedures (C00-EFF)</option>
      <option value="misc">misc (F00-FFF)</option>
    </select>
    <label>Provider ID (optional)</label>
    <input name="provider_id" placeholder="provider-uuid" />
    <label>Notes (optional)</label>
    <input name="notes" placeholder="Encounter notes" />
    <button type="submit">Create Encounter + Range-Aware Hex</button>
  </form>

  <h3 style="margin-top:1rem;">Providers</h3>
  <ul>{}</ul>
  <h3 style="margin-top:1rem;">Encounters for selected patient</h3>
  <ul>{}</ul>
</section>

<section class="card">
  <h2>6) Analytics Summary</h2>
  {}
  {}
</section>

<section class="card">
  <h2>7) Legal / DUA</h2>
  <ul>{}</ul>
</section>
"#,
        notice_html,
        html_escape(admin_email.trim()),
        if selected_org_value.is_empty() {
            "<span class=\"muted\">none selected</span>".to_string()
        } else {
            selected_org_value.clone()
        },
        if selected_project_value.is_empty() {
            "<span class=\"muted\">none selected</span>".to_string()
        } else {
            selected_project_value.clone()
        },
        admin_email_q,
        html_escape(admin_email.trim()),
        organizations_html,
        html_escape(admin_email.trim()),
        selected_org_value.clone(),
        projects_html,
        html_escape(admin_email.trim()),
        selected_project_value.clone(),
        sites_html,
        html_escape(admin_email.trim()),
        html_escape(admin_email.trim()),
        selected_org_id.map(|v| v.to_string()).unwrap_or_default(),
        selected_project_id
            .map(|v| v.to_string())
            .unwrap_or_default(),
        html_escape(admin_email.trim()),
        selected_org_id.map(|v| v.to_string()).unwrap_or_default(),
        selected_project_id
            .map(|v| v.to_string())
            .unwrap_or_default(),
        patients_html,
        html_escape(admin_email.trim()),
        selected_org_id.map(|v| v.to_string()).unwrap_or_default(),
        html_escape(admin_email.trim()),
        selected_patient_value,
        providers_html,
        encounters_html,
        org_summary_html,
        project_summary_html,
        dua_html
    );

    Ok(Html(render_cingulum_page("Virivu Research Web App", body)))
}

async fn submit_app_create_organization(
    State(ctx): State<AppContext>,
    Form(form): Form<AppCreateOrganizationForm>,
) -> Result<Redirect, ApiError> {
    if form.organization_name.trim().is_empty() {
        return Err(ApiError::Validation(
            "organization_name is required".to_string(),
        ));
    }
    let is_platform_admin = ctx
        .db
        .email_has_platform_admin_role(form.admin_email.trim())
        .await
        .map_err(ApiError::internal)?;
    if !is_platform_admin {
        return Err(ApiError::Auth(AuthError::Forbidden(
            "admin_email is not a platform_admin".to_string(),
        )));
    }
    let organization = ctx
        .db
        .create_organization(form.organization_name.trim())
        .await
        .map_err(ApiError::internal)?;
    ctx.db
        .ensure_org_admin_membership(form.admin_email.trim(), organization.id)
        .await
        .map_err(ApiError::internal)?;
    Ok(Redirect::to(&format!(
        "/ui/app?admin_email={}&organization_id={}&notice={}",
        query_escape(form.admin_email.trim()),
        organization.id,
        query_escape("Organization created")
    )))
}

async fn submit_app_create_project(
    State(ctx): State<AppContext>,
    Form(form): Form<AppCreateProjectForm>,
) -> Result<Redirect, ApiError> {
    let organization_id = parse_uuid_field(&form.organization_id, "organization_id")?;
    if form.project_name.trim().is_empty() {
        return Err(ApiError::Validation("project_name is required".to_string()));
    }
    let allowed = ctx
        .db
        .email_has_org_manager_role(form.admin_email.trim(), organization_id)
        .await
        .map_err(ApiError::internal)?;
    if !allowed {
        return Err(ApiError::Auth(AuthError::Forbidden(
            "admin_email lacks organization manager access".to_string(),
        )));
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
        query_escape(form.admin_email.trim()),
        organization_id,
        project.id,
        query_escape("Project created")
    )))
}

async fn submit_app_create_site(
    State(ctx): State<AppContext>,
    Form(form): Form<AppCreateSiteForm>,
) -> Result<Redirect, ApiError> {
    let project_id = parse_uuid_field(&form.project_id, "project_id")?;
    let project = ctx
        .db
        .get_project(project_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("project not found".to_string()))?;
    let allowed = ctx
        .db
        .email_has_org_manager_role(form.admin_email.trim(), project.organization_id)
        .await
        .map_err(ApiError::internal)?;
    if !allowed {
        return Err(ApiError::Auth(AuthError::Forbidden(
            "admin_email lacks organization manager access".to_string(),
        )));
    }

    ctx.db
        .create_site(
            project_id,
            form.site_name.trim(),
            form.principal_investigator.trim(),
        )
        .await
        .map_err(ApiError::internal)?;

    Ok(Redirect::to(&format!(
        "/ui/app?admin_email={}&organization_id={}&project_id={}&notice={}",
        query_escape(form.admin_email.trim()),
        project.organization_id,
        project_id,
        query_escape("Site created")
    )))
}

async fn submit_app_create_patient(
    State(ctx): State<AppContext>,
    Form(form): Form<AppCreatePatientForm>,
) -> Result<Redirect, ApiError> {
    let site_id = parse_uuid_field(&form.site_id, "site_id")?;
    let site = ctx
        .db
        .get_site(site_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("site not found".to_string()))?;
    let project = ctx
        .db
        .get_project(site.project_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("project not found".to_string()))?;
    let allowed = ctx
        .db
        .email_has_org_manager_role(form.admin_email.trim(), project.organization_id)
        .await
        .map_err(ApiError::internal)?;
    if !allowed {
        return Err(ApiError::Auth(AuthError::Forbidden(
            "admin_email lacks organization manager access".to_string(),
        )));
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
        .create_patient(site_id, external_subject_id, patient_email, date_of_birth)
        .await
        .map_err(ApiError::internal)?;

    Ok(Redirect::to(&format!(
        "/ui/app?admin_email={}&organization_id={}&project_id={}&patient_id={}&notice={}",
        query_escape(form.admin_email.trim()),
        project.organization_id,
        project.id,
        patient.id,
        query_escape("Patient created with cascading hex identifier")
    )))
}

async fn submit_app_create_provider(
    State(ctx): State<AppContext>,
    Form(form): Form<AppCreateProviderForm>,
) -> Result<Redirect, ApiError> {
    let organization_id = parse_uuid_field(&form.organization_id, "organization_id")?;
    let allowed = ctx
        .db
        .email_has_org_manager_role(form.admin_email.trim(), organization_id)
        .await
        .map_err(ApiError::internal)?;
    if !allowed {
        return Err(ApiError::Auth(AuthError::Forbidden(
            "admin_email lacks organization manager access".to_string(),
        )));
    }
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
        )
        .await
        .map_err(ApiError::internal)?;
    Ok(Redirect::to(&format!(
        "/ui/app?admin_email={}&organization_id={}&notice={}",
        query_escape(form.admin_email.trim()),
        organization_id,
        query_escape("Provider created with reserved hex block")
    )))
}

async fn submit_app_create_encounter(
    State(ctx): State<AppContext>,
    Form(form): Form<AppCreateEncounterForm>,
) -> Result<Redirect, ApiError> {
    let patient_id = parse_uuid_field(&form.patient_id, "patient_id")?;
    let patient = ctx
        .db
        .get_patient(patient_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("patient not found".to_string()))?;
    let allowed = ctx
        .db
        .email_has_org_manager_role(form.admin_email.trim(), patient.organization_id)
        .await
        .map_err(ApiError::internal)?;
    if !allowed {
        return Err(ApiError::Auth(AuthError::Forbidden(
            "admin_email lacks organization manager access".to_string(),
        )));
    }
    let provider_id = if form.provider_id.trim().is_empty() {
        None
    } else {
        Some(parse_uuid_field(&form.provider_id, "provider_id")?)
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
        query_escape(form.admin_email.trim()),
        patient.organization_id,
        patient.project_id,
        patient.id,
        query_escape("Encounter created with range-based hex identifier")
    )))
}

async fn submit_app_send_invite(
    State(ctx): State<AppContext>,
    Form(form): Form<AppSendInviteForm>,
) -> Result<Redirect, ApiError> {
    let organization_id = parse_uuid_field(&form.organization_id, "organization_id")?;
    let project_id = parse_uuid_field(&form.project_id, "project_id")?;
    let allowed = ctx
        .db
        .email_has_org_manager_role(form.admin_email.trim(), organization_id)
        .await
        .map_err(ApiError::internal)?;
    if !allowed {
        return Err(ApiError::Auth(AuthError::Forbidden(
            "admin_email lacks organization manager access".to_string(),
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
        query_escape(form.admin_email.trim()),
        organization_id,
        project_id,
        query_escape("Form invite sent")
    )))
}

async fn submit_app_create_media_ticket(
    State(ctx): State<AppContext>,
    Form(form): Form<AppCreateMediaTicketForm>,
) -> Result<Html<String>, ApiError> {
    let organization_id = parse_uuid_field(&form.organization_id, "organization_id")?;
    let project_id = parse_uuid_field(&form.project_id, "project_id")?;
    let allowed = ctx
        .db
        .email_has_org_manager_role(form.admin_email.trim(), organization_id)
        .await
        .map_err(ApiError::internal)?;
    if !allowed {
        return Err(ApiError::Auth(AuthError::Forbidden(
            "admin_email lacks organization manager access".to_string(),
        )));
    }
    let ticket = ctx
        .db
        .create_media_upload_ticket(
            organization_id,
            project_id,
            form.patient_id.trim(),
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
        query_escape(form.admin_email.trim()),
        organization_id,
        project_id
    );
    Ok(Html(render_cingulum_page(
        "Media Upload Ticket Created",
        body,
    )))
}

#[derive(Debug, Deserialize)]
struct DuaDraftForm {
    admin_email: String,
    organization_id: String,
    hospital_name: String,
    hospital_contact_name: String,
    hospital_contact_email: String,
    agreement_version: String,
    effective_date: String,
    expiration_date: String,
    agreement_text: String,
}

#[derive(Debug, Deserialize)]
struct DuaCingulumSignForm {
    admin_email: String,
    signer_name: String,
    signer_email: String,
    signer_title: String,
    signature_text: String,
}

#[derive(Debug, Deserialize)]
struct DuaHospitalSignForm {
    signer_name: String,
    signer_email: String,
    signer_title: String,
    signer_organization: String,
    signature_text: String,
}

#[derive(Debug, Deserialize)]
struct DuaSendLinkForm {
    admin_email: String,
}

#[derive(Debug, Deserialize)]
struct DuaExportQuery {
    admin_email: String,
}

#[derive(Debug, Default, Deserialize)]
struct DuaAdminPageQuery {
    admin_email: Option<String>,
    organization_id: Option<String>,
    notice: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DuaCreateOrganizationForm {
    admin_email: String,
    organization_name: String,
}

#[derive(Debug, Deserialize)]
struct DuaOpenAgreementForm {
    agreement_id: String,
}

async fn render_dua_admin_page(
    State(ctx): State<AppContext>,
    Query(query): Query<DuaAdminPageQuery>,
) -> Result<Html<String>, ApiError> {
    let admin_email = query
        .admin_email
        .unwrap_or_else(|| "arcot@cingulum.org".to_string());
    let organizations = ctx
        .db
        .list_organizations_for_email(admin_email.trim())
        .await
        .map_err(ApiError::internal)?;

    let selected_organization_id = query
        .organization_id
        .or_else(|| organizations.first().map(|o| o.id.to_string()))
        .unwrap_or_default();

    let organization_options = organizations
        .iter()
        .map(|org| {
            format!(
                r#"<option value="{}">{}</option>"#,
                org.id,
                html_escape(&format!("{} ({})", org.name, org.id))
            )
        })
        .collect::<Vec<_>>()
        .join("");

    let managed_orgs_html = if organizations.is_empty() {
        "<li>No organizations found for this admin email yet.</li>".to_string()
    } else {
        organizations
            .iter()
            .map(|org| {
                format!(
                    "<li><strong>{}</strong> — {}</li>",
                    html_escape(&org.name),
                    org.id
                )
            })
            .collect::<Vec<_>>()
            .join("")
    };

    let notice_html = query
        .notice
        .map(|notice| format!(r#"<p class="notice">{}</p>"#, html_escape(notice.trim())))
        .unwrap_or_default();

    let body = format!(
        r#"
  <h1>Electronic Data Use Agreements</h1>
  <p class="muted">Create and manage DUA records between hospitals and Cingulum Foundation Inc.</p>
  {}
  <section class="card">
    <h2>Step 1: Create or choose organization</h2>
    <form method="post" action="/ui/dua/create-organization">
      <label>Admin email (platform admin required to create org)</label>
      <input name="admin_email" value="{}" required />

      <label>New organization legal name</label>
      <input name="organization_name" placeholder="Cingulum Foundation Inc." required />

      <button type="submit">Create Organization</button>
    </form>
    <h3 style="margin-top:1rem;">Organizations available for this admin</h3>
    <ul>{}</ul>
  </section>

  <section class="card">
    <h2>Return to existing agreement workspace</h2>
    <form method="post" action="/ui/dua/open-agreement">
      <label>Agreement ID (UUID)</label>
      <input name="agreement_id" placeholder="agreement-uuid" required />
      <button type="submit">Open Agreement Workspace</button>
    </form>
  </section>

  <section class="card">
    <h2>Step 2: Draft DUA</h2>
    <form method="post" action="/ui/dua/draft">
      <label>Admin email (must be org manager or platform admin)</label>
      <input name="admin_email" value="{}" required />

      <label>Organization ID (UUID)</label>
      <input name="organization_id" list="organization-options" value="{}" placeholder="organization-uuid" required />
      <datalist id="organization-options">
        {}
      </datalist>

      <label>Hospital legal name</label>
      <input name="hospital_name" placeholder="Hospital Name" required />

      <label>Hospital contact name</label>
      <input name="hospital_contact_name" placeholder="Contact Name" required />

      <label>Hospital contact email</label>
      <input type="email" name="hospital_contact_email" placeholder="legal@hospital.org" required />

      <label>Agreement version</label>
      <input name="agreement_version" value="1.0" required />

      <label>Effective date (YYYY-MM-DD)</label>
      <input name="effective_date" placeholder="2026-06-01" />

      <label>Expiration date (YYYY-MM-DD)</label>
      <input name="expiration_date" placeholder="2027-06-01" />

      <label>Agreement text</label>
      <textarea name="agreement_text" required>{}</textarea>

      <button type="submit">Create DUA + Queue Hospital Signing Link</button>
    </form>
  </section>
"#,
        notice_html,
        html_escape(&admin_email),
        managed_orgs_html,
        html_escape(&admin_email),
        html_escape(&selected_organization_id),
        organization_options,
        html_escape(default_dua_text())
    );

    Ok(Html(render_cingulum_page("Virivu DUA Console", body)))
}

async fn submit_create_organization_from_ui(
    State(ctx): State<AppContext>,
    Form(form): Form<DuaCreateOrganizationForm>,
) -> Result<Html<String>, ApiError> {
    if form.organization_name.trim().is_empty() {
        return Err(ApiError::Validation(
            "organization_name is required".to_string(),
        ));
    }

    let is_platform_admin = ctx
        .db
        .email_has_platform_admin_role(form.admin_email.trim())
        .await
        .map_err(ApiError::internal)?;
    if !is_platform_admin {
        return Err(ApiError::Auth(AuthError::Forbidden(
            "admin_email is not a platform_admin".to_string(),
        )));
    }

    let organization = ctx
        .db
        .create_organization(form.organization_name.trim())
        .await
        .map_err(ApiError::internal)?;
    ctx.db
        .ensure_org_admin_membership(form.admin_email.trim(), organization.id)
        .await
        .map_err(ApiError::internal)?;

    let body = format!(
        r#"
<section class="card">
  <h1>Organization Created</h1>
  <p><strong>Name:</strong> {}</p>
  <p><strong>Organization ID:</strong> {}</p>
  <p><a href="/ui/dua?admin_email={}&organization_id={}&notice=Organization+created+successfully">Continue to DUA drafting</a></p>
</section>
"#,
        html_escape(&organization.name),
        organization.id,
        form.admin_email.trim(),
        organization.id
    );
    Ok(Html(render_cingulum_page("Organization Created", body)))
}

async fn open_dua_agreement_workspace(
    Form(form): Form<DuaOpenAgreementForm>,
) -> Result<Redirect, ApiError> {
    let agreement_id = form
        .agreement_id
        .trim()
        .parse::<Uuid>()
        .map_err(|_| ApiError::Validation("agreement_id must be a valid UUID".to_string()))?;
    Ok(Redirect::to(&format!("/ui/dua/{}", agreement_id)))
}

async fn render_create_dua_from_form(
    State(ctx): State<AppContext>,
    Form(form): Form<DuaDraftForm>,
) -> Result<Html<String>, ApiError> {
    let organization_id =
        form.organization_id.trim().parse::<Uuid>().map_err(|_| {
            ApiError::Validation("organization_id must be a valid UUID".to_string())
        })?;

    let has_access = ctx
        .db
        .email_has_org_manager_role(form.admin_email.trim(), organization_id)
        .await
        .map_err(ApiError::internal)?;
    if !has_access {
        return Err(ApiError::Auth(AuthError::Forbidden(
            "admin_email lacks permission for this organization".to_string(),
        )));
    }

    let effective_date = parse_optional_date(&form.effective_date)?;
    let expiration_date = parse_optional_date(&form.expiration_date)?;
    let created_by_user_id = ctx
        .db
        .get_user_by_email(form.admin_email.trim())
        .await
        .map_err(ApiError::internal)?
        .map(|u| u.id);

    let agreement = ctx
        .db
        .create_data_use_agreement(
            organization_id,
            form.hospital_name.trim(),
            form.hospital_contact_name.trim(),
            form.hospital_contact_email.trim(),
            form.agreement_version.trim(),
            effective_date,
            expiration_date,
            form.agreement_text.trim(),
            created_by_user_id,
        )
        .await
        .map_err(ApiError::internal)?;

    ctx.db
        .queue_hospital_signing_email(agreement.id, created_by_user_id, &ctx.config.app_base_url)
        .await
        .map_err(ApiError::internal)?;

    let signing_url = format!(
        "{}/ui/dua/sign/{}",
        ctx.config.app_base_url.trim_end_matches('/'),
        agreement.hospital_signing_token
    );

    let body = format!(
        r#"
<section class="card">
  <h1>DUA Created</h1>
  <p><strong>Agreement ID:</strong> {}</p>
  <p><strong>Status:</strong> <span class="status-chip">{}</span></p>
  <p><strong>Hospital signing URL:</strong> <a href="{}">{}</a></p>
  <p><a href="/ui/dua/{}">Open agreement workspace</a></p>
  <p><a href="/ui/dua">Create another agreement</a></p>
</section>
"#,
        agreement.id,
        html_escape(&agreement.status),
        html_escape(&signing_url),
        html_escape(&signing_url),
        agreement.id
    );
    Ok(Html(render_cingulum_page("DUA Created", body)))
}

async fn render_dua_hospital_sign_page(
    State(ctx): State<AppContext>,
    Path(signing_token): Path<Uuid>,
) -> Result<Html<String>, ApiError> {
    let agreement = ctx
        .db
        .get_data_use_agreement_by_signing_token(signing_token)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("signing token is invalid or expired".to_string()))?;

    let body = format!(
        r#"
<section class="card">
  <h1>Sign Data Use Agreement</h1>
  <p><strong>Hospital:</strong> {}</p>
  <p><strong>Counterparty:</strong> {}</p>
  <p><strong>Agreement Version:</strong> {}</p>
  <p><a href="/ui/dua/{}">Open agreement workspace</a></p>
  <form method="post" action="/ui/dua/sign/{}">
    <label>Signer name</label>
    <input name="signer_name" required />
    <label>Signer email</label>
    <input type="email" name="signer_email" required />
    <label>Signer title</label>
    <input name="signer_title" required />
    <label>Signer organization</label>
    <input name="signer_organization" value="{}" required />
    <label>Electronic signature text</label>
    <input name="signature_text" placeholder="/s/ Your Name" required />
    <button type="submit">Submit Signature</button>
  </form>
</section>
"#,
        html_escape(&agreement.hospital_name),
        html_escape(&agreement.counterparty_name),
        html_escape(&agreement.agreement_version),
        agreement.id,
        agreement.hospital_signing_token,
        html_escape(&agreement.hospital_name),
    );
    Ok(Html(render_cingulum_page("Hospital DUA Signature", body)))
}

async fn submit_dua_hospital_sign_form(
    State(ctx): State<AppContext>,
    Path(signing_token): Path<Uuid>,
    Form(form): Form<DuaHospitalSignForm>,
) -> Result<Html<String>, ApiError> {
    let (agreement, _signature) = ctx
        .db
        .sign_data_use_agreement_by_token(
            signing_token,
            form.signer_name.trim(),
            form.signer_email.trim(),
            form.signer_title.trim(),
            form.signer_organization.trim(),
            "typed",
            form.signature_text.trim(),
            None,
        )
        .await
        .map_err(|error| {
            let message = error.to_string();
            if message.contains("not found for token") {
                ApiError::NotFound("signing token is invalid or expired".to_string())
            } else {
                ApiError::internal(message)
            }
        })?;

    let body = format!(
        r#"
<section class="card">
  <h1>Signature received</h1>
  <p>Thank you. Your hospital signature has been recorded for agreement <strong>{}</strong>.</p>
  <p>Current status: <span class="status-chip">{}</span></p>
  <p><a href="/ui/dua/{}">Open agreement workspace</a></p>
  <p><a href="/ui/dua">Back to DUA home</a></p>
</section>
"#,
        agreement.id,
        html_escape(&agreement.status),
        agreement.id
    );
    Ok(Html(render_cingulum_page("Signature Submitted", body)))
}

async fn render_dua_agreement_page(
    State(ctx): State<AppContext>,
    Path(agreement_id): Path<Uuid>,
) -> Result<Html<String>, ApiError> {
    let agreement = ctx
        .db
        .get_data_use_agreement(agreement_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("data use agreement not found".to_string()))?;
    let signatures = ctx
        .db
        .list_data_use_agreement_signatures(agreement_id)
        .await
        .map_err(ApiError::internal)?;
    let emails = ctx
        .db
        .list_outbound_emails_for_agreement(agreement_id)
        .await
        .map_err(ApiError::internal)?;

    let signatures_html = signatures
        .iter()
        .map(|s| {
            format!(
                "<li><strong>{}</strong> - {} ({}) at {}</li>",
                html_escape(&s.signer_role),
                html_escape(&s.signer_name),
                html_escape(&s.signer_email),
                s.signed_at
            )
        })
        .collect::<Vec<_>>()
        .join("");

    let email_html = emails
        .iter()
        .map(|e| {
            format!(
                "<li>{} | {} | {}</li>",
                e.created_at,
                html_escape(&e.recipient_email),
                html_escape(&e.status)
            )
        })
        .collect::<Vec<_>>()
        .join("");

    let signing_link = format!(
        "{}/ui/dua/sign/{}",
        ctx.config.app_base_url.trim_end_matches('/'),
        agreement.hospital_signing_token
    );

    let body = format!(
        r#"
<section class="card">
  <h1>DUA Workspace</h1>
  <p><strong>Agreement ID:</strong> {}</p>
  <p><strong>Hospital:</strong> {}</p>
  <p><strong>Status:</strong> <span class="status-chip">{}</span></p>
  <p><strong>Hospital signing link:</strong> <a href="{}">{}</a></p>
</section>

<section class="card">
  <h2>Actions</h2>
  <form method="post" action="/ui/dua/{}/send-hospital-link" style="margin-bottom:1rem;">
    <label>Admin email for send action</label>
    <input name="admin_email" placeholder="arcot@cingulum.org" required style="max-width:480px;" />
    <button type="submit">Queue Hospital Signing Email</button>
  </form>

  <form method="post" action="/ui/dua/{}/sign-cingulum" style="margin-bottom:1rem;">
    <input type="hidden" name="admin_email" value="arcot@cingulum.org" />
    <label>Cingulum signer name</label><input name="signer_name" required style="max-width:480px;" />
    <label>Cingulum signer email</label><input name="signer_email" required style="max-width:480px;" />
    <label>Cingulum signer title</label><input name="signer_title" required style="max-width:480px;" />
    <label>Signature text</label><input name="signature_text" placeholder="/s/ Name" required style="max-width:480px;" />
    <button type="submit">Apply Cingulum Signature</button>
  </form>

  <p><a href="/ui/dua/{}/export.pdf?admin_email=arcot@cingulum.org">Download PDF (requires admin_email query)</a></p>
</section>

<section class="card">
  <h2>Signatures</h2>
  <ul>{}</ul>

  <h2>Email Queue</h2>
  <ul>{}</ul>
</section>

<section class="card">
  <h2>Agreement Text</h2>
  <pre style="white-space: pre-wrap; border:1px solid #C5B7AB; padding:1rem; border-radius:10px; background:#fff;">{}</pre>
</section>
"#,
        agreement.id,
        html_escape(&agreement.hospital_name),
        html_escape(&agreement.status),
        signing_link,
        signing_link,
        agreement.id,
        agreement.id,
        agreement.id,
        if signatures_html.is_empty() {
            "<li>No signatures yet</li>".to_string()
        } else {
            signatures_html
        },
        if email_html.is_empty() {
            "<li>No queued/sent emails yet</li>".to_string()
        } else {
            email_html
        },
        html_escape(&agreement.agreement_text)
    );
    Ok(Html(render_cingulum_page("DUA Workspace", body)))
}

async fn submit_dua_send_hospital_link_form(
    State(ctx): State<AppContext>,
    Path(agreement_id): Path<Uuid>,
    Form(form): Form<DuaSendLinkForm>,
) -> Result<Html<String>, ApiError> {
    let agreement = ctx
        .db
        .get_data_use_agreement(agreement_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("data use agreement not found".to_string()))?;
    let allowed = ctx
        .db
        .email_has_org_manager_role(form.admin_email.trim(), agreement.organization_id)
        .await
        .map_err(ApiError::internal)?;
    if !allowed {
        return Err(ApiError::Auth(AuthError::Forbidden(
            "admin_email lacks permission for this organization".to_string(),
        )));
    }
    let requested_by = ctx
        .db
        .get_user_by_email(form.admin_email.trim())
        .await
        .map_err(ApiError::internal)?
        .map(|u| u.id);
    ctx.db
        .queue_hospital_signing_email(agreement_id, requested_by, &ctx.config.app_base_url)
        .await
        .map_err(ApiError::internal)?;
    let body = format!(
        r#"
<section class="card">
  <h1>Email queued</h1>
  <p>Hospital signing email has been queued for agreement <strong>{}</strong>.</p>
  <p><a href="/ui/dua/{}">Back to agreement</a></p>
</section>
"#,
        agreement_id, agreement_id
    );
    Ok(Html(render_cingulum_page("Email queued", body)))
}

async fn submit_dua_cingulum_sign_form(
    State(ctx): State<AppContext>,
    Path(agreement_id): Path<Uuid>,
    Form(form): Form<DuaCingulumSignForm>,
) -> Result<Html<String>, ApiError> {
    let agreement = ctx
        .db
        .get_data_use_agreement(agreement_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("data use agreement not found".to_string()))?;
    let allowed = ctx
        .db
        .email_has_org_manager_role(form.admin_email.trim(), agreement.organization_id)
        .await
        .map_err(ApiError::internal)?;
    if !allowed {
        return Err(ApiError::Auth(AuthError::Forbidden(
            "admin_email lacks permission for this organization".to_string(),
        )));
    }
    let user_id = ctx
        .db
        .get_user_by_email(form.admin_email.trim())
        .await
        .map_err(ApiError::internal)?
        .map(|u| u.id);

    ctx.db
        .sign_data_use_agreement_as_cingulum(
            agreement_id,
            form.signer_name.trim(),
            form.signer_email.trim(),
            form.signer_title.trim(),
            "typed",
            form.signature_text.trim(),
            None,
            user_id,
        )
        .await
        .map_err(ApiError::internal)?;

    let body = format!(
        r#"
<section class="card">
  <h1>Cingulum signature recorded</h1>
  <p><a href="/ui/dua/{}">Back to agreement</a></p>
</section>
"#,
        agreement_id
    );
    Ok(Html(render_cingulum_page(
        "Cingulum signature recorded",
        body,
    )))
}

async fn download_data_use_agreement_pdf_ui(
    State(ctx): State<AppContext>,
    Path(agreement_id): Path<Uuid>,
    Query(query): Query<DuaExportQuery>,
) -> Result<Response, ApiError> {
    let agreement = ctx
        .db
        .get_data_use_agreement(agreement_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("data use agreement not found".to_string()))?;
    let allowed = ctx
        .db
        .email_has_org_manager_role(query.admin_email.trim(), agreement.organization_id)
        .await
        .map_err(ApiError::internal)?;
    if !allowed {
        return Err(ApiError::Auth(AuthError::Forbidden(
            "admin_email lacks permission for this organization".to_string(),
        )));
    }

    let signatures = ctx
        .db
        .list_data_use_agreement_signatures(agreement_id)
        .await
        .map_err(ApiError::internal)?;
    let bytes = build_data_use_agreement_pdf(&agreement, &signatures);
    pdf_download_response(agreement_id, bytes)
}

#[derive(Debug, Deserialize)]
struct CreateDataUseAgreementRequest {
    organization_id: Uuid,
    hospital_name: String,
    hospital_contact_name: String,
    hospital_contact_email: String,
    agreement_version: Option<String>,
    effective_date: Option<chrono::NaiveDate>,
    expiration_date: Option<chrono::NaiveDate>,
    agreement_text: String,
}

#[derive(Debug, Serialize)]
struct DataUseAgreementDetailResponse {
    agreement: DataUseAgreement,
    signatures: Vec<DataUseAgreementSignature>,
    hospital_signature_endpoint: String,
    hospital_signing_url: String,
}

async fn create_data_use_agreement(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
    Json(payload): Json<CreateDataUseAgreementRequest>,
) -> Result<impl IntoResponse, ApiError> {
    require_org_role(&user, payload.organization_id, ROLE_ORG_MANAGERS)?;

    if payload.hospital_name.trim().is_empty() {
        return Err(ApiError::Validation(
            "hospital_name is required".to_string(),
        ));
    }
    if payload.hospital_contact_email.trim().is_empty() {
        return Err(ApiError::Validation(
            "hospital_contact_email is required".to_string(),
        ));
    }
    if payload.agreement_text.trim().is_empty() {
        return Err(ApiError::Validation(
            "agreement_text is required".to_string(),
        ));
    }

    let agreement_version = payload
        .agreement_version
        .unwrap_or_else(|| "1.0".to_string());
    let agreement = ctx
        .db
        .create_data_use_agreement(
            payload.organization_id,
            payload.hospital_name.trim(),
            payload.hospital_contact_name.trim(),
            payload.hospital_contact_email.trim(),
            agreement_version.trim(),
            payload.effective_date,
            payload.expiration_date,
            payload.agreement_text.trim(),
            Some(user.user_id),
        )
        .await
        .map_err(ApiError::internal)?;

    Ok((
        StatusCode::CREATED,
        Json(DataUseAgreementDetailResponse {
            hospital_signing_url: format!(
                "{}/ui/dua/sign/{}",
                ctx.config.app_base_url.trim_end_matches('/'),
                agreement.hospital_signing_token
            ),
            agreement,
            signatures: Vec::new(),
            hospital_signature_endpoint: "/v1/legal/data-use-agreements/sign-hospital".to_string(),
        }),
    ))
}

async fn list_data_use_agreements(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
    Path(org_id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    require_org_role(&user, org_id, ROLE_ORG_MANAGERS)?;
    let agreements = ctx
        .db
        .list_data_use_agreements(org_id)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(agreements))
}

async fn get_data_use_agreement(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
    Path(agreement_id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let agreement = ctx
        .db
        .get_data_use_agreement(agreement_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("data use agreement not found".to_string()))?;
    require_org_role(&user, agreement.organization_id, ROLE_ORG_MANAGERS)?;

    let signatures = ctx
        .db
        .list_data_use_agreement_signatures(agreement_id)
        .await
        .map_err(ApiError::internal)?;

    Ok(Json(DataUseAgreementDetailResponse {
        hospital_signing_url: format!(
            "{}/ui/dua/sign/{}",
            ctx.config.app_base_url.trim_end_matches('/'),
            agreement.hospital_signing_token
        ),
        agreement,
        signatures,
        hospital_signature_endpoint: "/v1/legal/data-use-agreements/sign-hospital".to_string(),
    }))
}

#[derive(Debug, Serialize)]
struct SendHospitalSigningEmailResponse {
    email: OutboundEmail,
    signing_url: String,
}

async fn send_hospital_signing_link_email(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
    Path(agreement_id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let agreement = ctx
        .db
        .get_data_use_agreement(agreement_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("data use agreement not found".to_string()))?;
    require_org_role(&user, agreement.organization_id, ROLE_ORG_MANAGERS)?;

    let queued_email = ctx
        .db
        .queue_hospital_signing_email(agreement_id, Some(user.user_id), &ctx.config.app_base_url)
        .await
        .map_err(ApiError::internal)?;

    let signing_url = format!(
        "{}/ui/dua/sign/{}",
        ctx.config.app_base_url.trim_end_matches('/'),
        agreement.hospital_signing_token
    );

    Ok((
        StatusCode::CREATED,
        Json(SendHospitalSigningEmailResponse {
            email: queued_email,
            signing_url,
        }),
    ))
}

async fn list_data_use_agreement_emails(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
    Path(agreement_id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let agreement = ctx
        .db
        .get_data_use_agreement(agreement_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("data use agreement not found".to_string()))?;
    require_org_role(&user, agreement.organization_id, ROLE_ORG_MANAGERS)?;

    let emails = ctx
        .db
        .list_outbound_emails_for_agreement(agreement_id)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(emails))
}

async fn download_data_use_agreement_pdf(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
    Path(agreement_id): Path<Uuid>,
) -> Result<Response, ApiError> {
    let agreement = ctx
        .db
        .get_data_use_agreement(agreement_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("data use agreement not found".to_string()))?;
    require_org_role(&user, agreement.organization_id, ROLE_ORG_MANAGERS)?;
    let signatures = ctx
        .db
        .list_data_use_agreement_signatures(agreement_id)
        .await
        .map_err(ApiError::internal)?;

    let bytes = build_data_use_agreement_pdf(&agreement, &signatures);
    pdf_download_response(agreement_id, bytes)
}

#[derive(Debug, Deserialize)]
struct SignCingulumAgreementRequest {
    signer_name: String,
    signer_email: String,
    signer_title: String,
    signature_method: Option<String>,
    signature_text: String,
    ip_address: Option<String>,
}

async fn sign_data_use_agreement_cingulum(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
    Path(agreement_id): Path<Uuid>,
    Json(payload): Json<SignCingulumAgreementRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let agreement = ctx
        .db
        .get_data_use_agreement(agreement_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("data use agreement not found".to_string()))?;
    require_org_role(&user, agreement.organization_id, ROLE_ORG_MANAGERS)?;

    if payload.signature_text.trim().is_empty() {
        return Err(ApiError::Validation(
            "signature_text is required".to_string(),
        ));
    }

    let signature_method = payload
        .signature_method
        .unwrap_or_else(|| "typed".to_string())
        .trim()
        .to_string();
    let ip_address = parse_ip_address(payload.ip_address)?;

    ctx.db
        .sign_data_use_agreement_as_cingulum(
            agreement_id,
            payload.signer_name.trim(),
            payload.signer_email.trim(),
            payload.signer_title.trim(),
            &signature_method,
            payload.signature_text.trim(),
            ip_address,
            Some(user.user_id),
        )
        .await
        .map_err(ApiError::internal)?;

    let updated_agreement = ctx
        .db
        .get_data_use_agreement(agreement_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("data use agreement not found".to_string()))?;
    let signatures = ctx
        .db
        .list_data_use_agreement_signatures(agreement_id)
        .await
        .map_err(ApiError::internal)?;

    Ok(Json(DataUseAgreementDetailResponse {
        hospital_signing_url: format!(
            "{}/ui/dua/sign/{}",
            ctx.config.app_base_url.trim_end_matches('/'),
            updated_agreement.hospital_signing_token
        ),
        agreement: updated_agreement,
        signatures,
        hospital_signature_endpoint: "/v1/legal/data-use-agreements/sign-hospital".to_string(),
    }))
}

#[derive(Debug, Deserialize)]
struct SignHospitalAgreementRequest {
    signing_token: Uuid,
    signer_name: String,
    signer_email: String,
    signer_title: String,
    signer_organization: String,
    signature_method: Option<String>,
    signature_text: String,
    ip_address: Option<String>,
}

async fn sign_data_use_agreement_hospital(
    State(ctx): State<AppContext>,
    Json(payload): Json<SignHospitalAgreementRequest>,
) -> Result<impl IntoResponse, ApiError> {
    if payload.signature_text.trim().is_empty() {
        return Err(ApiError::Validation(
            "signature_text is required".to_string(),
        ));
    }
    if payload.signer_organization.trim().is_empty() {
        return Err(ApiError::Validation(
            "signer_organization is required".to_string(),
        ));
    }

    let signature_method = payload
        .signature_method
        .unwrap_or_else(|| "typed".to_string())
        .trim()
        .to_string();
    let ip_address = parse_ip_address(payload.ip_address)?;

    let (agreement, _signature) = ctx
        .db
        .sign_data_use_agreement_by_token(
            payload.signing_token,
            payload.signer_name.trim(),
            payload.signer_email.trim(),
            payload.signer_title.trim(),
            payload.signer_organization.trim(),
            &signature_method,
            payload.signature_text.trim(),
            ip_address,
        )
        .await
        .map_err(|error| {
            let message = error.to_string();
            if message.contains("not found for token") {
                ApiError::NotFound("signing token is invalid or expired".to_string())
            } else {
                ApiError::internal(message)
            }
        })?;

    let signatures = ctx
        .db
        .list_data_use_agreement_signatures(agreement.id)
        .await
        .map_err(ApiError::internal)?;

    Ok(Json(DataUseAgreementDetailResponse {
        hospital_signing_url: format!(
            "{}/ui/dua/sign/{}",
            ctx.config.app_base_url.trim_end_matches('/'),
            agreement.hospital_signing_token
        ),
        agreement,
        signatures,
        hospital_signature_endpoint: "/v1/legal/data-use-agreements/sign-hospital".to_string(),
    }))
}

fn pdf_download_response(agreement_id: Uuid, bytes: Vec<u8>) -> Result<Response, ApiError> {
    let mut response = Response::new(bytes.into());
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/pdf"),
    );

    let filename = format!("data-use-agreement-{}.pdf", agreement_id);
    let content_disposition = format!("attachment; filename=\"{}\"", filename);
    let header_value = HeaderValue::from_str(&content_disposition)
        .map_err(|e| ApiError::internal(format!("invalid header value: {e}")))?;
    response
        .headers_mut()
        .insert(header::CONTENT_DISPOSITION, header_value);
    Ok(response)
}

fn parse_optional_date(raw: &str) -> Result<Option<chrono::NaiveDate>, ApiError> {
    if raw.trim().is_empty() {
        return Ok(None);
    }
    chrono::NaiveDate::parse_from_str(raw.trim(), "%Y-%m-%d")
        .map(Some)
        .map_err(|_| ApiError::Validation("date must use YYYY-MM-DD format".to_string()))
}

fn parse_uuid_field(raw: &str, field_name: &str) -> Result<Uuid, ApiError> {
    raw.trim()
        .parse::<Uuid>()
        .map_err(|_| ApiError::Validation(format!("{field_name} must be a valid UUID")))
}

fn query_escape(input: &str) -> String {
    input
        .replace('%', "%25")
        .replace(' ', "+")
        .replace('&', "%26")
        .replace('?', "%3F")
        .replace('#', "%23")
        .replace('=', "%3D")
}

fn default_dua_text() -> &'static str {
    "This Data Use Agreement is entered into between [HOSPITAL LEGAL NAME] and Cingulum Foundation Inc. for approved medical research data workflows. The parties agree to HIPAA-aligned safeguards, role-based access controls, minimum necessary use, and auditable electronic signatures."
}

fn html_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn build_data_use_agreement_pdf(
    agreement: &DataUseAgreement,
    signatures: &[DataUseAgreementSignature],
) -> Vec<u8> {
    let mut lines = vec![
        format!("Data Use Agreement {}", agreement.id),
        format!("Hospital: {}", agreement.hospital_name),
        format!("Counterparty: {}", agreement.counterparty_name),
        format!("Status: {}", agreement.status),
        format!("Version: {}", agreement.agreement_version),
        format!(
            "Effective: {}",
            agreement
                .effective_date
                .map(|d| d.to_string())
                .unwrap_or_else(|| "N/A".to_string())
        ),
        format!(
            "Expiration: {}",
            agreement
                .expiration_date
                .map(|d| d.to_string())
                .unwrap_or_else(|| "N/A".to_string())
        ),
        String::new(),
        "Signatures".to_string(),
    ];

    if signatures.is_empty() {
        lines.push("- none recorded".to_string());
    } else {
        for signature in signatures {
            lines.push(format!(
                "- {} | {} | {} | {}",
                signature.signer_role,
                signature.signer_name,
                signature.signer_email,
                signature.signed_at
            ));
        }
    }

    lines.push(String::new());
    lines.push("Agreement Text".to_string());
    for wrapped in wrap_text(&agreement.agreement_text, 92) {
        lines.push(wrapped);
    }

    let mut content = String::from("BT\n/F1 10 Tf\n50 790 Td\n12 TL\n");
    for line in lines.into_iter().take(280) {
        content.push_str(&format!("({}) Tj\nT*\n", pdf_escape(&line)));
    }
    content.push_str("ET\n");
    let content_bytes = content.as_bytes();

    let mut pdf = Vec::<u8>::new();
    let mut offsets = Vec::<usize>::new();

    pdf.extend_from_slice(b"%PDF-1.4\n");

    offsets.push(pdf.len());
    pdf.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");

    offsets.push(pdf.len());
    pdf.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");

    offsets.push(pdf.len());
    pdf.extend_from_slice(
        b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>\nendobj\n",
    );

    offsets.push(pdf.len());
    pdf.extend_from_slice(
        b"4 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n",
    );

    offsets.push(pdf.len());
    pdf.extend_from_slice(
        format!("5 0 obj\n<< /Length {} >>\nstream\n", content_bytes.len()).as_bytes(),
    );
    pdf.extend_from_slice(content_bytes);
    pdf.extend_from_slice(b"endstream\nendobj\n");

    let xref_offset = pdf.len();
    pdf.extend_from_slice(format!("xref\n0 {}\n", offsets.len() + 1).as_bytes());
    pdf.extend_from_slice(b"0000000000 65535 f \n");
    for offset in offsets {
        pdf.extend_from_slice(format!("{:010} 00000 n \n", offset).as_bytes());
    }

    pdf.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n",
            6, xref_offset
        )
        .as_bytes(),
    );
    pdf
}

fn cingulum_theme_css() -> &'static str {
    r#"
    :root {
      --cg-cream: #E7E5DA;
      --cg-sand: #C5B7AB;
      --cg-forest: #283E28;
      --cg-navy: #02182B;
      --cg-orange: #F05708;
    }
    * { box-sizing: border-box; }
    body {
      margin: 0;
      font-family: Inter, Arial, sans-serif;
      color: var(--cg-navy);
      background: linear-gradient(180deg, #f7f5ef 0%, var(--cg-cream) 100%);
    }
    .page {
      max-width: 1024px;
      margin: 1.8rem auto;
      padding: 0 1rem 2rem;
    }
    .brand {
      color: var(--cg-forest);
      font-size: 0.9rem;
      letter-spacing: 0.04em;
      text-transform: uppercase;
      font-weight: 700;
      margin-bottom: 0.4rem;
    }
    .card {
      background: #fffdfa;
      border: 1px solid var(--cg-sand);
      border-radius: 14px;
      box-shadow: 0 10px 24px rgba(2, 24, 43, 0.08);
      padding: 1rem 1.1rem;
      margin-bottom: 1rem;
    }
    h1, h2, h3 { margin-top: 0; color: var(--cg-navy); }
    p, li, label, small { color: #10263d; }
    a { color: var(--cg-forest); font-weight: 600; }
    a:hover { color: var(--cg-orange); }
    input, textarea, select {
      width: 100%;
      border: 1px solid var(--cg-sand);
      border-radius: 10px;
      padding: 0.62rem;
      background: #fff;
      color: var(--cg-navy);
    }
    input:focus, textarea:focus, select:focus {
      outline: 2px solid rgba(240, 87, 8, 0.25);
      border-color: var(--cg-orange);
    }
    textarea { min-height: 170px; }
    button {
      background: var(--cg-orange);
      color: #fff;
      border: none;
      border-radius: 10px;
      padding: 0.68rem 1rem;
      cursor: pointer;
      font-weight: 700;
    }
    button:hover { filter: brightness(0.95); }
    .muted { color: #41566d; }
    .status-chip {
      display: inline-block;
      padding: 0.18rem 0.6rem;
      border-radius: 999px;
      background: #efe6de;
      color: var(--cg-forest);
      font-weight: 700;
      font-size: 0.84rem;
    }
    "#
}

fn render_cingulum_page(title: &str, body_content: String) -> String {
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>{}</title>
  <style>{}</style>
</head>
<body>
  <main class="page">
    <div class="brand">Cingulum Foundation Inc.</div>
    {}
  </main>
</body>
</html>"#,
        html_escape(title),
        cingulum_theme_css(),
        body_content
    )
}

fn wrap_text(input: &str, max_chars: usize) -> Vec<String> {
    let mut wrapped = Vec::new();
    for paragraph in input.lines() {
        if paragraph.trim().is_empty() {
            wrapped.push(String::new());
            continue;
        }
        let mut current = String::new();
        for word in paragraph.split_whitespace() {
            if current.is_empty() {
                current.push_str(word);
            } else if current.len() + 1 + word.len() > max_chars {
                wrapped.push(current);
                current = word.to_string();
            } else {
                current.push(' ');
                current.push_str(word);
            }
        }
        if !current.is_empty() {
            wrapped.push(current);
        }
    }
    wrapped
}

fn pdf_escape(input: &str) -> String {
    input
        .replace('\\', "\\\\")
        .replace('(', "\\(")
        .replace(')', "\\)")
}

fn parse_ip_address(raw: Option<String>) -> Result<Option<IpAddr>, ApiError> {
    match raw {
        Some(text) if !text.trim().is_empty() => {
            text.trim().parse::<IpAddr>().map(Some).map_err(|_| {
                ApiError::Validation("ip_address must be a valid IP address".to_string())
            })
        }
        _ => Ok(None),
    }
}

#[derive(Debug, Deserialize)]
struct TranscriptRequest {
    organization_id: Uuid,
    project_id: Uuid,
    clinician_name: String,
    patient_name: String,
    transcript: String,
}

#[derive(Debug, Serialize)]
struct TranscriptResponse {
    organization_id: Uuid,
    project_id: Uuid,
    summary_note: String,
    reminder: &'static str,
}

async fn generate_doctor_patient_note(
    user: AuthenticatedUser,
    Json(payload): Json<TranscriptRequest>,
) -> Result<impl IntoResponse, ApiError> {
    require_org_role(&user, payload.organization_id, ROLE_COORDINATOR_OR_BETTER)?;
    if !user.has_project_role(payload.project_id, ROLE_COORDINATOR_OR_BETTER)
        && !user.has_org_role(payload.organization_id, ROLE_COORDINATOR_OR_BETTER)
    {
        return Err(ApiError::Auth(AuthError::Forbidden(
            "user lacks project or org permission".to_string(),
        )));
    }

    let transcript_excerpt: String = payload.transcript.chars().take(240).collect();
    let summary_note = format!(
        "Draft note for clinician {} and patient {}. Transcript excerpt: {}",
        payload.clinician_name, payload.patient_name, transcript_excerpt
    );

    Ok((
        StatusCode::OK,
        Json(TranscriptResponse {
            organization_id: payload.organization_id,
            project_id: payload.project_id,
            summary_note,
            reminder: "This is a non-diagnostic draft and requires clinician review.",
        }),
    ))
}

fn require_platform_role(user: &AuthenticatedUser, roles: &[&str]) -> Result<(), ApiError> {
    if user.has_platform_role(roles) {
        Ok(())
    } else {
        Err(ApiError::Auth(AuthError::Forbidden(
            "user lacks required platform role".to_string(),
        )))
    }
}

fn require_org_role(
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

#[derive(Debug)]
enum ApiError {
    Auth(AuthError),
    Validation(String),
    NotFound(String),
    Internal(String),
}

impl ApiError {
    fn internal<E: ToString>(error: E) -> Self {
        Self::Internal(error.to_string())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        match self {
            Self::Auth(error) => error.into_response(),
            Self::Validation(msg) => (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": msg })),
            )
                .into_response(),
            Self::NotFound(msg) => (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": msg })),
            )
                .into_response(),
            Self::Internal(msg) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": msg })),
            )
                .into_response(),
        }
    }
}
