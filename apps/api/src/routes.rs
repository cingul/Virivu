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
        .route("/ui/studies", get(render_study_workbench))
        .route("/ui/studies/create", post(submit_create_study_from_ui))
        .route(
            "/ui/studies/{project_id}/phase",
            post(submit_study_phase_transition),
        )
        .route(
            "/ui/studies/{project_id}/crf-template",
            post(submit_create_study_crf_template),
        )
        .route(
            "/ui/studies/templates/{template_id}/field",
            post(submit_add_study_crf_field),
        )
        .route(
            "/ui/studies/templates/{template_id}/publish",
            post(submit_publish_study_crf_template),
        )
        .route(
            "/ui/studies/{project_id}/visit-template",
            post(submit_create_study_visit_template),
        )
        .route(
            "/ui/studies/{project_id}/schedule-visit",
            post(submit_schedule_patient_visit),
        )
        .route(
            "/ui/studies/{project_id}/crf-submission",
            post(submit_create_study_crf_submission),
        )
        .route(
            "/ui/studies/submissions/{submission_id}/submit",
            post(submit_mark_study_crf_submission_submitted),
        )
        .route(
            "/ui/studies/submissions/{submission_id}/lock",
            post(submit_lock_study_crf_submission),
        )
        .route(
            "/ui/studies/{project_id}/query",
            post(submit_create_study_data_query),
        )
        .route(
            "/ui/studies/queries/{query_id}/respond",
            post(submit_respond_study_data_query),
        )
        .route(
            "/ui/studies/queries/{query_id}/close",
            post(submit_close_study_data_query),
        )
        .route(
            "/ui/studies/{project_id}/close-checklist",
            post(submit_set_study_close_checklist_item),
        )
        .route(
            "/ui/studies/{project_id}/startup-checklist",
            post(submit_set_study_startup_checklist_item),
        )
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
        .route("/v1/studies", post(create_study_project))
        .route(
            "/v1/studies/{project_id}/phase",
            post(transition_study_phase),
        )
        .route(
            "/v1/studies/{project_id}/readiness",
            get(get_study_readiness),
        )
        .route(
            "/v1/studies/{project_id}/crf-templates",
            get(list_study_crf_templates).post(create_study_crf_template),
        )
        .route(
            "/v1/studies/crf-templates/{template_id}/fields",
            get(list_study_crf_fields).post(add_study_crf_field),
        )
        .route(
            "/v1/studies/crf-templates/{template_id}/publish",
            post(publish_study_crf_template),
        )
        .route(
            "/v1/studies/{project_id}/visit-templates",
            get(list_study_visit_templates).post(create_study_visit_template),
        )
        .route(
            "/v1/studies/{project_id}/patient-visits",
            get(list_patient_study_visits).post(schedule_patient_study_visit),
        )
        .route(
            "/v1/studies/{project_id}/crf-submissions",
            get(list_study_crf_submissions).post(create_study_crf_submission),
        )
        .route(
            "/v1/studies/crf-submissions/{submission_id}/submit",
            post(mark_study_crf_submission_submitted),
        )
        .route(
            "/v1/studies/crf-submissions/{submission_id}/lock",
            post(lock_study_crf_submission),
        )
        .route(
            "/v1/studies/{project_id}/data-queries",
            get(list_study_data_queries).post(create_study_data_query),
        )
        .route(
            "/v1/studies/data-queries/{query_id}/respond",
            post(respond_study_data_query),
        )
        .route(
            "/v1/studies/data-queries/{query_id}/close",
            post(close_study_data_query),
        )
        .route(
            "/v1/studies/{project_id}/close-checklist",
            get(list_study_close_checklist_items).post(set_study_close_checklist_item),
        )
        .route(
            "/v1/studies/{project_id}/startup-checklist",
            get(list_study_startup_checklist_items).post(set_study_startup_checklist_item),
        )
        .route(
            "/v1/studies/{project_id}/operational-summary",
            get(get_study_operational_summary),
        )
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
struct CreateStudyProjectRequest {
    organization_id: Uuid,
    name: String,
    therapeutic_area: String,
    protocol_code: Option<String>,
    planned_enrollment: Option<i32>,
    clinicaltrials_gov_id: Option<String>,
    study_summary: Option<String>,
}

async fn create_study_project(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
    Json(payload): Json<CreateStudyProjectRequest>,
) -> Result<impl IntoResponse, ApiError> {
    require_org_role(&user, payload.organization_id, ROLE_ORG_MANAGERS)?;
    if payload.name.trim().is_empty() {
        return Err(ApiError::Validation("study name is required".to_string()));
    }
    let planned_enrollment = payload.planned_enrollment.unwrap_or(0).max(0);
    let project = ctx
        .db
        .create_study_project(
            payload.organization_id,
            payload.name.trim(),
            payload.therapeutic_area.trim(),
            payload.protocol_code.as_deref().map(str::trim),
            planned_enrollment,
            payload.clinicaltrials_gov_id.as_deref().map(str::trim),
            payload.study_summary.as_deref().unwrap_or("").trim(),
        )
        .await
        .map_err(ApiError::internal)?;
    Ok((StatusCode::CREATED, Json(project)))
}

#[derive(Debug, Deserialize)]
struct StudyPhaseTransitionRequest {
    next_phase: String,
    notes: Option<String>,
}

async fn transition_study_phase(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
    Path(project_id): Path<Uuid>,
    Json(payload): Json<StudyPhaseTransitionRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let project = ctx
        .db
        .get_project(project_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("project not found".to_string()))?;
    require_org_role(&user, project.organization_id, ROLE_ORG_MANAGERS)?;
    let updated = ctx
        .db
        .transition_study_phase(
            project_id,
            payload.next_phase.trim(),
            Some(user.user_id),
            payload.notes.as_deref().unwrap_or("").trim(),
        )
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(updated))
}

async fn get_study_readiness(
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
    let readiness = ctx
        .db
        .study_readiness(project_id)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(readiness))
}

#[derive(Debug, Deserialize)]
struct CreateStudyCrfTemplateRequest {
    name: String,
    description: Option<String>,
    applicable_phase: Option<String>,
}

async fn create_study_crf_template(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
    Path(project_id): Path<Uuid>,
    Json(payload): Json<CreateStudyCrfTemplateRequest>,
) -> Result<impl IntoResponse, ApiError> {
    if payload.name.trim().is_empty() {
        return Err(ApiError::Validation("name is required".to_string()));
    }
    let project = ctx
        .db
        .get_project(project_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("project not found".to_string()))?;
    require_org_role(&user, project.organization_id, ROLE_ORG_MANAGERS)?;
    let template = ctx
        .db
        .create_study_crf_template(
            project_id,
            payload.name.trim(),
            payload.description.as_deref().unwrap_or("").trim(),
            payload
                .applicable_phase
                .as_deref()
                .unwrap_or("active")
                .trim(),
            Some(user.user_id),
        )
        .await
        .map_err(ApiError::internal)?;
    Ok((StatusCode::CREATED, Json(template)))
}

async fn list_study_crf_templates(
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
    let templates = ctx
        .db
        .list_study_crf_templates(project_id)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(templates))
}

#[derive(Debug, Deserialize)]
struct AddStudyCrfFieldRequest {
    field_key: String,
    field_label: String,
    field_type: String,
    required: Option<bool>,
    options_json: Option<String>,
    display_order: Option<i32>,
}

async fn add_study_crf_field(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
    Path(template_id): Path<Uuid>,
    Json(payload): Json<AddStudyCrfFieldRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let template = ctx
        .db
        .get_study_crf_template(template_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("template not found".to_string()))?;
    let project = ctx
        .db
        .get_project(template.project_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("project not found".to_string()))?;
    require_org_role(&user, project.organization_id, ROLE_ORG_MANAGERS)?;
    if payload.field_key.trim().is_empty() || payload.field_label.trim().is_empty() {
        return Err(ApiError::Validation(
            "field_key and field_label are required".to_string(),
        ));
    }
    let field = ctx
        .db
        .add_study_crf_field(
            template_id,
            payload.field_key.trim(),
            payload.field_label.trim(),
            payload.field_type.trim(),
            payload.required.unwrap_or(false),
            payload.options_json.as_deref().unwrap_or("[]").trim(),
            payload.display_order.unwrap_or(0),
        )
        .await
        .map_err(ApiError::internal)?;
    Ok((StatusCode::CREATED, Json(field)))
}

async fn list_study_crf_fields(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
    Path(template_id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let template = ctx
        .db
        .get_study_crf_template(template_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("template not found".to_string()))?;
    let project = ctx
        .db
        .get_project(template.project_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("project not found".to_string()))?;
    require_org_role(&user, project.organization_id, ROLE_ANALYTICS)?;
    let fields = ctx
        .db
        .list_study_crf_fields(template_id)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(fields))
}

async fn publish_study_crf_template(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
    Path(template_id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let template = ctx
        .db
        .get_study_crf_template(template_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("template not found".to_string()))?;
    let project = ctx
        .db
        .get_project(template.project_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("project not found".to_string()))?;
    require_org_role(&user, project.organization_id, ROLE_ORG_MANAGERS)?;
    let updated = ctx
        .db
        .publish_study_crf_template(template_id)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(updated))
}

#[derive(Debug, Deserialize)]
struct CreateStudyVisitTemplateRequest {
    visit_code: String,
    visit_name: String,
    target_day: Option<i32>,
    window_before_days: Option<i32>,
    window_after_days: Option<i32>,
    required: Option<bool>,
}

async fn create_study_visit_template(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
    Path(project_id): Path<Uuid>,
    Json(payload): Json<CreateStudyVisitTemplateRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let project = ctx
        .db
        .get_project(project_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("project not found".to_string()))?;
    require_org_role(&user, project.organization_id, ROLE_ORG_MANAGERS)?;
    if payload.visit_code.trim().is_empty() || payload.visit_name.trim().is_empty() {
        return Err(ApiError::Validation(
            "visit_code and visit_name are required".to_string(),
        ));
    }
    let visit = ctx
        .db
        .create_study_visit_template(
            project_id,
            payload.visit_code.trim(),
            payload.visit_name.trim(),
            payload.target_day.unwrap_or(0),
            payload.window_before_days.unwrap_or(0).max(0),
            payload.window_after_days.unwrap_or(0).max(0),
            payload.required.unwrap_or(true),
        )
        .await
        .map_err(ApiError::internal)?;
    Ok((StatusCode::CREATED, Json(visit)))
}

async fn list_study_visit_templates(
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
    let templates = ctx
        .db
        .list_study_visit_templates(project_id)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(templates))
}

#[derive(Debug, Deserialize)]
struct SchedulePatientStudyVisitRequest {
    patient_id: Uuid,
    visit_template_id: Uuid,
    scheduled_for: Option<chrono::NaiveDate>,
}

async fn schedule_patient_study_visit(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
    Path(project_id): Path<Uuid>,
    Json(payload): Json<SchedulePatientStudyVisitRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let project = ctx
        .db
        .get_project(project_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("project not found".to_string()))?;
    require_org_role(&user, project.organization_id, ROLE_COORDINATOR_OR_BETTER)?;
    let visit = ctx
        .db
        .schedule_patient_study_visit(
            project_id,
            payload.patient_id,
            payload.visit_template_id,
            payload.scheduled_for,
        )
        .await
        .map_err(ApiError::internal)?;
    Ok((StatusCode::CREATED, Json(visit)))
}

async fn list_patient_study_visits(
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
    let visits = ctx
        .db
        .list_patient_study_visits(project_id)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(visits))
}

#[derive(Debug, Deserialize)]
struct CreateStudyCrfSubmissionRequest {
    template_id: Uuid,
    patient_id: Uuid,
    patient_visit_id: Option<Uuid>,
    answers_json: Option<String>,
}

async fn create_study_crf_submission(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
    Path(project_id): Path<Uuid>,
    Json(payload): Json<CreateStudyCrfSubmissionRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let project = ctx
        .db
        .get_project(project_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("project not found".to_string()))?;
    require_org_role(&user, project.organization_id, ROLE_COORDINATOR_OR_BETTER)?;
    let submission = ctx
        .db
        .create_study_crf_submission(
            project_id,
            payload.template_id,
            payload.patient_id,
            payload.patient_visit_id,
            payload.answers_json.as_deref().unwrap_or("{}").trim(),
            Some(user.user_id),
        )
        .await
        .map_err(ApiError::internal)?;
    Ok((StatusCode::CREATED, Json(submission)))
}

async fn list_study_crf_submissions(
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
    let submissions = ctx
        .db
        .list_study_crf_submissions(project_id)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(submissions))
}

async fn mark_study_crf_submission_submitted(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
    Path(submission_id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let submission = ctx
        .db
        .get_study_crf_submission(submission_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("submission not found".to_string()))?;
    let project = ctx
        .db
        .get_project(submission.project_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("project not found".to_string()))?;
    require_org_role(&user, project.organization_id, ROLE_COORDINATOR_OR_BETTER)?;
    let updated = ctx
        .db
        .submit_study_crf_submission(submission_id)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(updated))
}

async fn lock_study_crf_submission(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
    Path(submission_id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let submission = ctx
        .db
        .get_study_crf_submission(submission_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("submission not found".to_string()))?;
    let project = ctx
        .db
        .get_project(submission.project_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("project not found".to_string()))?;
    require_org_role(&user, project.organization_id, ROLE_ORG_MANAGERS)?;
    let updated = ctx
        .db
        .lock_study_crf_submission(submission_id)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(updated))
}

#[derive(Debug, Deserialize)]
struct CreateStudyDataQueryRequest {
    submission_id: Uuid,
    field_key: String,
    query_text: String,
}

async fn create_study_data_query(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
    Path(project_id): Path<Uuid>,
    Json(payload): Json<CreateStudyDataQueryRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let project = ctx
        .db
        .get_project(project_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("project not found".to_string()))?;
    require_org_role(&user, project.organization_id, ROLE_COORDINATOR_OR_BETTER)?;
    if payload.field_key.trim().is_empty() || payload.query_text.trim().is_empty() {
        return Err(ApiError::Validation(
            "field_key and query_text are required".to_string(),
        ));
    }
    let submission = ctx
        .db
        .get_study_crf_submission(payload.submission_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("submission not found".to_string()))?;
    if submission.project_id != project_id {
        return Err(ApiError::Validation(
            "submission does not belong to project".to_string(),
        ));
    }
    let query = ctx
        .db
        .create_study_data_query(
            project_id,
            payload.submission_id,
            payload.field_key.trim(),
            payload.query_text.trim(),
            Some(user.user_id),
        )
        .await
        .map_err(ApiError::internal)?;
    Ok((StatusCode::CREATED, Json(query)))
}

async fn list_study_data_queries(
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
    let queries = ctx
        .db
        .list_study_data_queries(project_id)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(queries))
}

#[derive(Debug, Deserialize)]
struct RespondStudyDataQueryRequest {
    response_text: String,
}

async fn respond_study_data_query(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
    Path(query_id): Path<Uuid>,
    Json(payload): Json<RespondStudyDataQueryRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let query = ctx
        .db
        .get_study_data_query(query_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("data query not found".to_string()))?;
    let project = ctx
        .db
        .get_project(query.project_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("project not found".to_string()))?;
    require_org_role(&user, project.organization_id, ROLE_COORDINATOR_OR_BETTER)?;
    let updated = ctx
        .db
        .respond_study_data_query(query_id, payload.response_text.trim())
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(updated))
}

async fn close_study_data_query(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
    Path(query_id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let query = ctx
        .db
        .get_study_data_query(query_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("data query not found".to_string()))?;
    let project = ctx
        .db
        .get_project(query.project_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("project not found".to_string()))?;
    require_org_role(&user, project.organization_id, ROLE_ORG_MANAGERS)?;
    let updated = ctx
        .db
        .close_study_data_query(query_id, Some(user.user_id))
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(updated))
}

#[derive(Debug, Deserialize)]
struct SetStudyCloseChecklistItemRequest {
    item_code: String,
    item_label: String,
    completed: bool,
    notes: Option<String>,
}

async fn list_study_close_checklist_items(
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
    let items = ctx
        .db
        .list_study_close_checklist_items(project_id)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(items))
}

async fn set_study_close_checklist_item(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
    Path(project_id): Path<Uuid>,
    Json(payload): Json<SetStudyCloseChecklistItemRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let project = ctx
        .db
        .get_project(project_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("project not found".to_string()))?;
    require_org_role(&user, project.organization_id, ROLE_ORG_MANAGERS)?;
    if payload.item_code.trim().is_empty() || payload.item_label.trim().is_empty() {
        return Err(ApiError::Validation(
            "item_code and item_label are required".to_string(),
        ));
    }
    let item = ctx
        .db
        .set_study_close_checklist_item(
            project_id,
            payload.item_code.trim(),
            payload.item_label.trim(),
            payload.completed,
            if payload.completed {
                Some(user.user_id)
            } else {
                None
            },
            payload.notes.as_deref().unwrap_or("").trim(),
        )
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(item))
}

#[derive(Debug, Deserialize)]
struct SetStudyStartupChecklistItemRequest {
    item_code: String,
    item_label: String,
    completed: bool,
    notes: Option<String>,
}

async fn list_study_startup_checklist_items(
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
    let items = ctx
        .db
        .list_study_startup_checklist_items(project_id)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(items))
}

async fn set_study_startup_checklist_item(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
    Path(project_id): Path<Uuid>,
    Json(payload): Json<SetStudyStartupChecklistItemRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let project = ctx
        .db
        .get_project(project_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("project not found".to_string()))?;
    require_org_role(&user, project.organization_id, ROLE_ORG_MANAGERS)?;
    if payload.item_code.trim().is_empty() || payload.item_label.trim().is_empty() {
        return Err(ApiError::Validation(
            "item_code and item_label are required".to_string(),
        ));
    }
    let item = ctx
        .db
        .set_study_startup_checklist_item(
            project_id,
            payload.item_code.trim(),
            payload.item_label.trim(),
            payload.completed,
            if payload.completed {
                Some(user.user_id)
            } else {
                None
            },
            payload.notes.as_deref().unwrap_or("").trim(),
        )
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(item))
}

async fn get_study_operational_summary(
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
    let summary = ctx
        .db
        .study_operational_summary(project_id)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(summary))
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

#[derive(Debug, Default, Deserialize)]
struct StudyWorkbenchQuery {
    admin_email: Option<String>,
    organization_id: Option<String>,
    project_id: Option<String>,
    template_id: Option<String>,
    submission_id: Option<String>,
    notice: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StudyCreateForm {
    admin_email: String,
    organization_id: String,
    study_name: String,
    therapeutic_area: String,
    protocol_code: String,
    planned_enrollment: String,
    clinicaltrials_gov_id: String,
    study_summary: String,
}

#[derive(Debug, Deserialize)]
struct StudyPhaseTransitionForm {
    admin_email: String,
    next_phase: String,
    notes: String,
}

#[derive(Debug, Deserialize)]
struct StudyCrfTemplateForm {
    admin_email: String,
    name: String,
    description: String,
    applicable_phase: String,
}

#[derive(Debug, Deserialize)]
struct StudyCrfFieldForm {
    admin_email: String,
    field_key: String,
    field_label: String,
    field_type: String,
    required: Option<String>,
    options_json: String,
    display_order: String,
}

#[derive(Debug, Deserialize)]
struct StudyCrfPublishForm {
    admin_email: String,
}

#[derive(Debug, Deserialize)]
struct StudyVisitTemplateForm {
    admin_email: String,
    visit_code: String,
    visit_name: String,
    target_day: String,
    window_before_days: String,
    window_after_days: String,
    required: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StudyScheduleVisitForm {
    admin_email: String,
    patient_id: String,
    visit_template_id: String,
    scheduled_for: String,
}

#[derive(Debug, Deserialize)]
struct StudyCreateSubmissionForm {
    admin_email: String,
    template_id: String,
    patient_id: String,
    patient_visit_id: String,
    answers_json: String,
}

#[derive(Debug, Deserialize)]
struct StudySubmissionActionForm {
    admin_email: String,
}

#[derive(Debug, Deserialize)]
struct StudyCreateQueryForm {
    admin_email: String,
    submission_id: String,
    field_key: String,
    query_text: String,
}

#[derive(Debug, Deserialize)]
struct StudyRespondQueryForm {
    admin_email: String,
    response_text: String,
}

#[derive(Debug, Deserialize)]
struct StudyCloseQueryForm {
    admin_email: String,
}

#[derive(Debug, Deserialize)]
struct StudyChecklistItemForm {
    admin_email: String,
    item_code: String,
    item_label: String,
    completed: Option<String>,
    notes: String,
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

    let organization_options_html = organizations
        .iter()
        .map(|org| {
            format!(
                r#"<option value="{}">{}</option>"#,
                org.id,
                html_escape(&org.name)
            )
        })
        .collect::<Vec<_>>()
        .join("");
    let project_options_html = projects
        .iter()
        .map(|project| {
            format!(
                r#"<option value="{}">{}</option>"#,
                project.id,
                html_escape(&project.name)
            )
        })
        .collect::<Vec<_>>()
        .join("");
    let site_options_html = sites
        .iter()
        .map(|site| {
            format!(
                r#"<option value="{}">{}</option>"#,
                site.id,
                html_escape(&site.name)
            )
        })
        .collect::<Vec<_>>()
        .join("");
    let patient_options_html_app = patients
        .iter()
        .map(|patient| {
            format!(
                r#"<option value="{}">{}</option>"#,
                patient.id,
                html_escape(
                    &patient
                        .hex_code
                        .clone()
                        .unwrap_or_else(|| patient.id.to_string())
                )
            )
        })
        .collect::<Vec<_>>()
        .join("");
    let provider_options_html_app = providers
        .iter()
        .map(|provider| {
            format!(
                r#"<option value="{}">{}</option>"#,
                provider.id,
                html_escape(&provider.name)
            )
        })
        .collect::<Vec<_>>()
        .join("");

    let body = format!(
        r#"
<h1>Virivu Research Web App</h1>
<p class="muted">Unified operations workspace: institutions, trial setup, patient workflows, legal agreements, and analytics.</p>
{}
<div class="tab-shell">
<nav class="tab-bar" data-tab-group="app-tabs">
  <button type="button" class="tab-button is-active" data-tab-id="overview">Overview</button>
  <button type="button" class="tab-button" data-tab-id="projects">Projects</button>
  <button type="button" class="tab-button" data-tab-id="sites">Sites</button>
  <button type="button" class="tab-button" data-tab-id="patients">Patients</button>
  <button type="button" class="tab-button" data-tab-id="providers">Providers</button>
  <button type="button" class="tab-button" data-tab-id="analytics">Analytics</button>
  <button type="button" class="tab-button" data-tab-id="legal">Legal</button>
</nav>
<section class="card tab-panel is-active" data-tab-group="app-tabs" data-tab-panel="overview">
  <h2>Workspace Context</h2>
  <p><strong>Admin:</strong> {}</p>
  <p><strong>Organization:</strong> {}</p>
  <p><strong>Project:</strong> {}</p>
  <p><a href="/ui/studies">Open study lifecycle + CRF workbench</a></p>
  <p><a href="/ui/dua?admin_email={}">Open dedicated DUA console</a></p>
</section>

<section class="card tab-panel" data-tab-group="app-tabs" data-tab-panel="projects">
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

<section class="card tab-panel" data-tab-group="app-tabs" data-tab-panel="projects">
  <h2>2) Project Setup</h2>
  <form method="post" action="/ui/app/create-project">
    <label>Admin email</label>
    <input name="admin_email" value="{}" required />
    <label>Organization ID</label>
    <input name="organization_id" list="app-organization-options" value="{}" required />
    <label>Project name</label>
    <input name="project_name" placeholder="Stroke Registry 2026" required />
    <label>Therapeutic area</label>
    <input name="therapeutic_area" placeholder="Neurology" required />
    <button type="submit">Create Project</button>
  </form>
  <h3 style="margin-top:1rem;">Projects</h3>
  <ul>{}</ul>
</section>

<section class="card tab-panel" data-tab-group="app-tabs" data-tab-panel="sites">
  <h2>3) Site Setup</h2>
  <form method="post" action="/ui/app/create-site">
    <label>Admin email</label>
    <input name="admin_email" value="{}" required />
    <label>Project ID</label>
    <input name="project_id" list="app-project-options" value="{}" required />
    <label>Site name</label>
    <input name="site_name" placeholder="North Campus Site A" required />
    <label>Principal investigator</label>
    <input name="principal_investigator" placeholder="Dr. Example" required />
    <button type="submit">Create Site</button>
  </form>
  <h3 style="margin-top:1rem;">Sites</h3>
  <ul>{}</ul>
</section>

<section class="card tab-panel" data-tab-group="app-tabs" data-tab-panel="patients">
  <h2>4) Patient Workflow</h2>
  <form method="post" action="/ui/app/create-patient" style="margin-bottom:1rem;">
    <label>Admin email</label>
    <input name="admin_email" value="{}" required />
    <label>Site ID</label>
    <input name="site_id" list="app-site-options" placeholder="site-uuid" required />
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
    <input name="organization_id" list="app-organization-options" value="{}" required />
    <label>Project ID</label>
    <input name="project_id" list="app-project-options" value="{}" required />
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
    <input name="organization_id" list="app-organization-options" value="{}" required />
    <label>Project ID</label>
    <input name="project_id" list="app-project-options" value="{}" required />
    <label>Patient ID</label>
    <input name="patient_id" list="app-patient-options" placeholder="subject-001" required />
    <label>MIME type</label>
    <input name="mime_type" placeholder="video/mp4" required />
    <button type="submit">Generate Media Upload Link</button>
  </form>

  <h3 style="margin-top:1rem;">Patients</h3>
  <ul>{}</ul>
</section>

<section class="card tab-panel" data-tab-group="app-tabs" data-tab-panel="providers">
  <h2>5) Providers + Encounters</h2>
  <form method="post" action="/ui/app/create-provider" style="margin-bottom:1rem;">
    <label>Admin email</label>
    <input name="admin_email" value="{}" required />
    <label>Organization ID</label>
    <input name="organization_id" list="app-organization-options" value="{}" required />
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
    <input name="patient_id" list="app-patient-options" value="{}" placeholder="patient-uuid" required />
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
    <input name="provider_id" list="app-provider-options" placeholder="provider-uuid" />
    <label>Notes (optional)</label>
    <input name="notes" placeholder="Encounter notes" />
    <button type="submit">Create Encounter + Range-Aware Hex</button>
  </form>

  <h3 style="margin-top:1rem;">Providers</h3>
  <ul>{}</ul>
  <h3 style="margin-top:1rem;">Encounters for selected patient</h3>
  <ul>{}</ul>
</section>

<section class="card tab-panel" data-tab-group="app-tabs" data-tab-panel="analytics">
  <h2>6) Analytics Summary</h2>
  {}
  {}
</section>

<section class="card tab-panel" data-tab-group="app-tabs" data-tab-panel="legal">
  <h2>7) Legal / DUA</h2>
  <ul>{}</ul>
</section>
</div>

<datalist id="app-organization-options">{}</datalist>
<datalist id="app-project-options">{}</datalist>
<datalist id="app-site-options">{}</datalist>
<datalist id="app-patient-options">{}</datalist>
<datalist id="app-provider-options">{}</datalist>
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
        dua_html,
        organization_options_html,
        project_options_html,
        site_options_html,
        patient_options_html_app,
        provider_options_html_app
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

async fn render_study_workbench(
    State(ctx): State<AppContext>,
    Query(query): Query<StudyWorkbenchQuery>,
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
        .or_else(|| organizations.first().map(|o| o.id));
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
        .filter(|pid| projects.iter().any(|p| p.id == *pid))
        .or_else(|| projects.first().map(|p| p.id));
    let readiness = if let Some(project_id) = selected_project_id {
        Some(
            ctx.db
                .study_readiness(project_id)
                .await
                .map_err(ApiError::internal)?,
        )
    } else {
        None
    };
    let operational_summary = if let Some(project_id) = selected_project_id {
        Some(
            ctx.db
                .study_operational_summary(project_id)
                .await
                .map_err(ApiError::internal)?,
        )
    } else {
        None
    };
    let phase_events = if let Some(project_id) = selected_project_id {
        ctx.db
            .list_study_phase_events(project_id)
            .await
            .map_err(ApiError::internal)?
    } else {
        Vec::new()
    };
    let templates = if let Some(project_id) = selected_project_id {
        ctx.db
            .list_study_crf_templates(project_id)
            .await
            .map_err(ApiError::internal)?
    } else {
        Vec::new()
    };
    let selected_template_id = query
        .template_id
        .as_deref()
        .and_then(|raw| raw.parse::<Uuid>().ok())
        .filter(|tid| templates.iter().any(|t| t.id == *tid))
        .or_else(|| templates.first().map(|t| t.id));
    let fields = if let Some(template_id) = selected_template_id {
        ctx.db
            .list_study_crf_fields(template_id)
            .await
            .map_err(ApiError::internal)?
    } else {
        Vec::new()
    };

    let visit_templates = if let Some(project_id) = selected_project_id {
        ctx.db
            .list_study_visit_templates(project_id)
            .await
            .map_err(ApiError::internal)?
    } else {
        Vec::new()
    };
    let patient_visits = if let Some(project_id) = selected_project_id {
        ctx.db
            .list_patient_study_visits(project_id)
            .await
            .map_err(ApiError::internal)?
    } else {
        Vec::new()
    };
    let patients = if let Some(project_id) = selected_project_id {
        ctx.db
            .list_patients_by_project(project_id)
            .await
            .map_err(ApiError::internal)?
    } else {
        Vec::new()
    };
    let submissions = if let Some(project_id) = selected_project_id {
        ctx.db
            .list_study_crf_submissions(project_id)
            .await
            .map_err(ApiError::internal)?
    } else {
        Vec::new()
    };
    let selected_submission_id = query
        .submission_id
        .as_deref()
        .and_then(|raw| raw.parse::<Uuid>().ok())
        .filter(|sid| submissions.iter().any(|s| s.id == *sid))
        .or_else(|| submissions.first().map(|s| s.id));
    let data_queries = if let Some(project_id) = selected_project_id {
        ctx.db
            .list_study_data_queries(project_id)
            .await
            .map_err(ApiError::internal)?
    } else {
        Vec::new()
    };
    let checklist_items = if let Some(project_id) = selected_project_id {
        ctx.db
            .list_study_close_checklist_items(project_id)
            .await
            .map_err(ApiError::internal)?
    } else {
        Vec::new()
    };
    let startup_checklist_items = if let Some(project_id) = selected_project_id {
        ctx.db
            .list_study_startup_checklist_items(project_id)
            .await
            .map_err(ApiError::internal)?
    } else {
        Vec::new()
    };

    let notice_html = query
        .notice
        .map(|notice| format!(r#"<p class="notice">{}</p>"#, html_escape(notice.trim())))
        .unwrap_or_default();

    let study_rows_html = if projects.is_empty() {
        "<li>No studies yet for this organization.</li>".to_string()
    } else {
        projects
            .iter()
            .map(|project| {
                let selected_org = selected_org_id
                    .map(|org_id| format!("&organization_id={org_id}"))
                    .unwrap_or_default();
                format!(
                    r#"<li><a href="/ui/studies?admin_email={}{}&project_id={}">{}</a> <small>(phase: {} · hex: {} · target: {})</small></li>"#,
                    admin_email_q,
                    selected_org,
                    project.id,
                    html_escape(&project.name),
                    html_escape(&project.lifecycle_phase),
                    html_escape(project.hex_code.as_deref().unwrap_or("pending")),
                    project.planned_enrollment
                )
            })
            .collect::<Vec<_>>()
            .join("")
    };

    let readiness_html = if let Some(readiness) = readiness {
        format!(
            "<p><strong>Phase:</strong> {} · <strong>Sites:</strong> {} · <strong>Patients:</strong> {} · <strong>Encounters:</strong> {} · <strong>CRFs Published:</strong> {} · <strong>CRFs Draft:</strong> {}</p>",
            html_escape(&readiness.lifecycle_phase),
            readiness.total_sites,
            readiness.total_patients,
            readiness.total_encounters,
            readiness.published_crf_templates,
            readiness.draft_crf_templates
        )
    } else {
        "<p class=\"muted\">Select a study to view readiness.</p>".to_string()
    };
    let operational_summary_html = if let Some(summary) = operational_summary {
        format!(
            "<p><strong>Enrollment:</strong> {}/{} (gap {}) · <strong>Visits:</strong> {} scheduled / {} completed · <strong>Submissions:</strong> {} total / {} locked · <strong>Open Queries:</strong> {} · <strong>Startup Pending:</strong> {} · <strong>Close Pending:</strong> {}</p>",
            summary.enrolled_patients,
            summary.planned_enrollment,
            summary.enrollment_gap,
            summary.total_visits_scheduled,
            summary.completed_visits,
            summary.total_submissions,
            summary.locked_submissions,
            summary.open_data_queries,
            summary.startup_items_pending,
            summary.close_items_pending
        )
    } else {
        "<p class=\"muted\">Select a study to view operations summary.</p>".to_string()
    };

    let phase_events_html = if phase_events.is_empty() {
        "<li>No phase transitions recorded yet.</li>".to_string()
    } else {
        phase_events
            .iter()
            .map(|event| {
                format!(
                    "<li><strong>{}</strong> → <strong>{}</strong> at {} <small>({})</small></li>",
                    html_escape(&event.previous_phase),
                    html_escape(&event.new_phase),
                    event.created_at,
                    html_escape(&event.notes)
                )
            })
            .collect::<Vec<_>>()
            .join("")
    };

    let templates_html = if templates.is_empty() {
        "<li>No CRF templates yet.</li>".to_string()
    } else {
        templates
            .iter()
            .map(|template| {
                let selected_org = selected_org_id
                    .map(|org_id| format!("&organization_id={org_id}"))
                    .unwrap_or_default();
                let selected_project = selected_project_id
                    .map(|project_id| format!("&project_id={project_id}"))
                    .unwrap_or_default();
                let publish_button = if template.status == "published" {
                    "<small>published</small>".to_string()
                } else {
                    format!(
                        r#"<form method="post" action="/ui/studies/templates/{}/publish" style="display:inline;">
  <input type="hidden" name="admin_email" value="{}" />
  <button type="submit">Publish</button>
</form>"#,
                        template.id,
                        html_escape(admin_email.trim())
                    )
                };
                format!(
                    r#"<li><a href="/ui/studies?admin_email={}{}{}&template_id={}">{}</a> <small>(status: {} · phase: {})</small> {}</li>"#,
                    admin_email_q,
                    selected_org,
                    selected_project,
                    template.id,
                    html_escape(&template.name),
                    html_escape(&template.status),
                    html_escape(&template.applicable_phase),
                    publish_button
                )
            })
            .collect::<Vec<_>>()
            .join("")
    };

    let fields_html = if fields.is_empty() {
        "<li>No fields yet for selected template.</li>".to_string()
    } else {
        fields
            .iter()
            .map(|field| {
                format!(
                    "<li><strong>{}</strong> ({}) <small>key={} required={} options={}</small></li>",
                    html_escape(&field.field_label),
                    html_escape(&field.field_type),
                    html_escape(&field.field_key),
                    field.required,
                    html_escape(&field.options_json)
                )
            })
            .collect::<Vec<_>>()
            .join("")
    };

    let visit_templates_html = if visit_templates.is_empty() {
        "<li>No visit templates yet.</li>".to_string()
    } else {
        visit_templates
            .iter()
            .map(|visit| {
                format!(
                    "<li><strong>{}</strong> ({}) <small>day {} | window -{} / +{} | required={}</small></li>",
                    html_escape(&visit.visit_name),
                    html_escape(&visit.visit_code),
                    visit.target_day,
                    visit.window_before_days,
                    visit.window_after_days,
                    visit.required
                )
            })
            .collect::<Vec<_>>()
            .join("")
    };

    let patient_visits_html = if patient_visits.is_empty() {
        "<li>No scheduled patient visits yet.</li>".to_string()
    } else {
        patient_visits
            .iter()
            .take(20)
            .map(|visit| {
                format!(
                    "<li><strong>{}</strong> <small>patient={} template={} date={}</small></li>",
                    html_escape(&visit.status),
                    visit.patient_id,
                    visit.visit_template_id,
                    visit
                        .scheduled_for
                        .map(|d| d.to_string())
                        .unwrap_or_else(|| "unscheduled".to_string())
                )
            })
            .collect::<Vec<_>>()
            .join("")
    };

    let patients_html = if patients.is_empty() {
        "<li>No patients enrolled yet.</li>".to_string()
    } else {
        patients
            .iter()
            .map(|patient| {
                format!(
                    "<li>{} <small>(id: {})</small></li>",
                    html_escape(patient.hex_code.as_deref().unwrap_or("pending")),
                    patient.id
                )
            })
            .collect::<Vec<_>>()
            .join("")
    };

    let submissions_html = if submissions.is_empty() {
        "<li>No CRF submissions yet.</li>".to_string()
    } else {
        submissions
            .iter()
            .take(20)
            .map(|submission| {
                let selected_org = selected_org_id
                    .map(|org_id| format!("&organization_id={org_id}"))
                    .unwrap_or_default();
                let selected_project = selected_project_id
                    .map(|project_id| format!("&project_id={project_id}"))
                    .unwrap_or_default();
                format!(
                    r#"<li><a href="/ui/studies?admin_email={}{}{}&submission_id={}">{}</a> <small>(template={} patient={} status={})</small></li>"#,
                    admin_email_q,
                    selected_org,
                    selected_project,
                    submission.id,
                    submission.id,
                    submission.template_id,
                    submission.patient_id,
                    html_escape(&submission.status)
                )
            })
            .collect::<Vec<_>>()
            .join("")
    };

    let data_queries_html = if data_queries.is_empty() {
        "<li>No monitor queries yet.</li>".to_string()
    } else {
        data_queries
            .iter()
            .take(30)
            .map(|q| {
                let response_form = if q.status == "closed" {
                    "<small>closed</small>".to_string()
                } else {
                    format!(
                        r#"<form method="post" action="/ui/studies/queries/{}/respond" style="margin:0.4rem 0;">
  <input type="hidden" name="admin_email" value="{}" />
  <input name="response_text" placeholder="Response / correction note" />
  <button type="submit">Respond</button>
</form>
<form method="post" action="/ui/studies/queries/{}/close" style="margin:0;">
  <input type="hidden" name="admin_email" value="{}" />
  <button type="submit">Close query</button>
</form>"#,
                        q.id,
                        html_escape(admin_email.trim()),
                        q.id,
                        html_escape(admin_email.trim())
                    )
                };
                format!(
                    "<li><strong>{}</strong> <small>submission={} field={} status={} </small><div>{}</div>{}</li>",
                    html_escape(&q.query_text),
                    q.submission_id,
                    html_escape(&q.field_key),
                    html_escape(&q.status),
                    html_escape(&q.response_text),
                    response_form
                )
            })
            .collect::<Vec<_>>()
            .join("")
    };

    let checklist_html = if checklist_items.is_empty() {
        "<li>No close checklist items yet.</li>".to_string()
    } else {
        checklist_items
            .iter()
            .map(|item| {
                format!(
                    "<li><strong>{}</strong> <small>code={} completed={} notes={}</small></li>",
                    html_escape(&item.item_label),
                    html_escape(&item.item_code),
                    item.completed,
                    html_escape(&item.notes)
                )
            })
            .collect::<Vec<_>>()
            .join("")
    };
    let startup_checklist_html = if startup_checklist_items.is_empty() {
        "<li>No startup checklist items yet.</li>".to_string()
    } else {
        startup_checklist_items
            .iter()
            .map(|item| {
                format!(
                    "<li><strong>{}</strong> <small>code={} completed={} notes={}</small></li>",
                    html_escape(&item.item_label),
                    html_escape(&item.item_code),
                    item.completed,
                    html_escape(&item.notes)
                )
            })
            .collect::<Vec<_>>()
            .join("")
    };

    let patient_options_html = patients
        .iter()
        .map(|patient| {
            format!(
                r#"<option value="{}">{}</option>"#,
                patient.id,
                html_escape(
                    &patient
                        .hex_code
                        .clone()
                        .unwrap_or_else(|| patient.id.to_string())
                )
            )
        })
        .collect::<Vec<_>>()
        .join("");
    let template_options_html = templates
        .iter()
        .map(|template| {
            format!(
                r#"<option value="{}">{}</option>"#,
                template.id,
                html_escape(&template.name)
            )
        })
        .collect::<Vec<_>>()
        .join("");
    let visit_template_options_html = visit_templates
        .iter()
        .map(|visit| {
            format!(
                r#"<option value="{}">{} ({})</option>"#,
                visit.id,
                html_escape(&visit.visit_name),
                html_escape(&visit.visit_code)
            )
        })
        .collect::<Vec<_>>()
        .join("");
    let visit_options_html = patient_visits
        .iter()
        .map(|visit| {
            format!(
                r#"<option value="{}">{}</option>"#,
                visit.id,
                html_escape(&format!(
                    "{} / {}",
                    visit.patient_id, visit.visit_template_id
                ))
            )
        })
        .collect::<Vec<_>>()
        .join("");
    let submission_options_html = submissions
        .iter()
        .map(|submission| {
            format!(
                r#"<option value="{}">{}</option>"#,
                submission.id,
                html_escape(&format!("{} ({})", submission.id, submission.status))
            )
        })
        .collect::<Vec<_>>()
        .join("");

    let selected_org_value = selected_org_id.map(|id| id.to_string()).unwrap_or_default();
    let selected_project_value = selected_project_id
        .map(|id| id.to_string())
        .unwrap_or_default();
    let selected_template_value = selected_template_id
        .map(|id| id.to_string())
        .unwrap_or_default();
    let selected_submission_value = selected_submission_id
        .map(|id| id.to_string())
        .unwrap_or_default();
    let phase_action = selected_project_id
        .map(|id| format!("/ui/studies/{id}/phase"))
        .unwrap_or_else(|| "#".to_string());
    let crf_template_action = selected_project_id
        .map(|id| format!("/ui/studies/{id}/crf-template"))
        .unwrap_or_else(|| "#".to_string());
    let crf_field_action = selected_template_id
        .map(|id| format!("/ui/studies/templates/{id}/field"))
        .unwrap_or_else(|| "#".to_string());
    let visit_template_action = selected_project_id
        .map(|id| format!("/ui/studies/{id}/visit-template"))
        .unwrap_or_else(|| "#".to_string());
    let schedule_visit_action = selected_project_id
        .map(|id| format!("/ui/studies/{id}/schedule-visit"))
        .unwrap_or_else(|| "#".to_string());
    let create_submission_action = selected_project_id
        .map(|id| format!("/ui/studies/{id}/crf-submission"))
        .unwrap_or_else(|| "#".to_string());
    let submit_submission_action = selected_submission_id
        .map(|id| format!("/ui/studies/submissions/{id}/submit"))
        .unwrap_or_else(|| "#".to_string());
    let lock_submission_action = selected_submission_id
        .map(|id| format!("/ui/studies/submissions/{id}/lock"))
        .unwrap_or_else(|| "#".to_string());
    let create_query_action = selected_project_id
        .map(|id| format!("/ui/studies/{id}/query"))
        .unwrap_or_else(|| "#".to_string());
    let startup_checklist_action = selected_project_id
        .map(|id| format!("/ui/studies/{id}/startup-checklist"))
        .unwrap_or_else(|| "#".to_string());
    let checklist_action = selected_project_id
        .map(|id| format!("/ui/studies/{id}/close-checklist"))
        .unwrap_or_else(|| "#".to_string());

    let body = format!(
        r#"
<h1>Study Lifecycle + CRF Workbench</h1>
<p class="muted">Pre-study planning, initiation, activation, monitoring, and closure with operational CRF design.</p>
{}
<div class="tab-shell">
<nav class="tab-bar" data-tab-group="study-tabs">
  <button type="button" class="tab-button is-active" data-tab-id="overview">Overview</button>
  <button type="button" class="tab-button" data-tab-id="setup">Study Setup</button>
  <button type="button" class="tab-button" data-tab-id="lifecycle">Lifecycle</button>
  <button type="button" class="tab-button" data-tab-id="crf-templates">CRF Templates</button>
  <button type="button" class="tab-button" data-tab-id="crf-fields">CRF Fields</button>
  <button type="button" class="tab-button" data-tab-id="visits">Visits</button>
  <button type="button" class="tab-button" data-tab-id="submissions">Submissions</button>
  <button type="button" class="tab-button" data-tab-id="queries">Queries</button>
  <button type="button" class="tab-button" data-tab-id="startup">Startup</button>
  <button type="button" class="tab-button" data-tab-id="close">Close</button>
</nav>

<section class="card tab-panel is-active" data-tab-group="study-tabs" data-tab-panel="overview">
  <h2>Workspace</h2>
  <p><strong>Admin:</strong> {}</p>
  <p><strong>Organization:</strong> {}</p>
  <p><strong>Study:</strong> {}</p>
  <p><a href="/ui/app">Back to unified app dashboard</a></p>
</section>

<section class="card tab-panel" data-tab-group="study-tabs" data-tab-panel="setup">
  <h2>1) Create Study (clinicaltrials.gov-style metadata + internal ops)</h2>
  <form method="post" action="/ui/studies/create">
    <label>Admin email</label>
    <input name="admin_email" value="{}" required />
    <label>Organization ID</label>
    <input name="organization_id" value="{}" required />
    <label>Study name</label>
    <input name="study_name" placeholder="Acute Stroke Registry 2026" required />
    <label>Therapeutic area</label>
    <input name="therapeutic_area" placeholder="Neurology" required />
    <label>Protocol code</label>
    <input name="protocol_code" placeholder="VIR-STR-26-01" />
    <label>Planned enrollment</label>
    <input name="planned_enrollment" value="250" />
    <label>ClinicalTrials.gov ID (optional)</label>
    <input name="clinicaltrials_gov_id" placeholder="NCT01234567" />
    <label>Study summary</label>
    <textarea name="study_summary" placeholder="Primary objective, key endpoints, and operational plan"></textarea>
    <button type="submit">Create Study in Pre-Study Phase</button>
  </form>
  <h3 style="margin-top:1rem;">Study portfolio</h3>
  <ul>{}</ul>
</section>

<section class="card tab-panel" data-tab-group="study-tabs" data-tab-panel="lifecycle">
  <h2>2) Lifecycle Transition</h2>
  {}
  {}
  <form method="post" action="{}">
    <label>Admin email</label>
    <input name="admin_email" value="{}" required />
    <label>Next phase</label>
    <select name="next_phase" required>
      <option value="initiated">initiated</option>
      <option value="active">active</option>
      <option value="monitoring">monitoring</option>
      <option value="closed">closed</option>
    </select>
    <label>Transition notes</label>
    <input name="notes" placeholder="Reason for phase transition" />
    <button type="submit">Apply Phase Transition</button>
  </form>
  <h3 style="margin-top:1rem;">Phase events</h3>
  <ul>{}</ul>
</section>

<section class="card tab-panel" data-tab-group="study-tabs" data-tab-panel="crf-templates">
  <h2>3) CRF Template Design</h2>
  <form method="post" action="{}">
    <label>Admin email</label>
    <input name="admin_email" value="{}" required />
    <label>Template name</label>
    <input name="name" placeholder="Baseline Case Report Form" required />
    <label>Description</label>
    <input name="description" placeholder="Visit 1 baseline data capture" />
    <label>Applicable phase</label>
    <select name="applicable_phase" required>
      <option value="pre_study">pre_study</option>
      <option value="initiated">initiated</option>
      <option value="active">active</option>
      <option value="monitoring">monitoring</option>
      <option value="closed">closed</option>
    </select>
    <button type="submit">Create CRF Template (Draft)</button>
  </form>
  <h3 style="margin-top:1rem;">CRF templates</h3>
  <ul>{}</ul>
</section>

<section class="card tab-panel" data-tab-group="study-tabs" data-tab-panel="crf-fields">
  <h2>4) CRF Field Builder</h2>
  <p><strong>Selected template:</strong> {}</p>
  <form method="post" action="{}">
    <label>Admin email</label>
    <input name="admin_email" value="{}" required />
    <label>Field key</label>
    <input name="field_key" placeholder="systolic_bp" required />
    <label>Field label</label>
    <input name="field_label" placeholder="Systolic blood pressure" required />
    <label>Field type</label>
    <select name="field_type" required>
      <option value="text">text</option>
      <option value="textarea">textarea</option>
      <option value="number">number</option>
      <option value="date">date</option>
      <option value="datetime">datetime</option>
      <option value="boolean">boolean</option>
      <option value="single_select">single_select</option>
      <option value="multi_select">multi_select</option>
    </select>
    <label>Required</label>
    <input type="checkbox" name="required" value="true" />
    <label>Options JSON (for select fields)</label>
    <input name="options_json" placeholder='["Yes","No"]' />
    <label>Display order</label>
    <input name="display_order" value="0" />
    <button type="submit">Add Field</button>
  </form>
  <h3 style="margin-top:1rem;">Fields</h3>
  <ul>{}</ul>
</section>

<section class="card tab-panel" data-tab-group="study-tabs" data-tab-panel="visits">
  <h2>5) Visit Schedule Engine</h2>
  <form method="post" action="{}" style="margin-bottom:1rem;">
    <label>Admin email</label>
    <input name="admin_email" value="{}" required />
    <label>Visit code</label>
    <input name="visit_code" placeholder="SCREENING" required />
    <label>Visit name</label>
    <input name="visit_name" placeholder="Screening Visit" required />
    <label>Target day</label>
    <input name="target_day" value="0" />
    <label>Window before (days)</label>
    <input name="window_before_days" value="0" />
    <label>Window after (days)</label>
    <input name="window_after_days" value="7" />
    <label>Required</label>
    <input type="checkbox" name="required" value="true" checked />
    <button type="submit">Create Visit Template</button>
  </form>
  <ul>{}</ul>
  <form method="post" action="{}">
    <label>Admin email</label>
    <input name="admin_email" value="{}" required />
    <label>Patient ID</label>
    <input name="patient_id" list="study-patient-options" placeholder="patient-uuid" required />
    <label>Visit template ID</label>
    <input name="visit_template_id" list="study-visit-template-options" placeholder="visit-template-uuid" required />
    <label>Scheduled date (YYYY-MM-DD)</label>
    <input name="scheduled_for" placeholder="2026-06-01" />
    <button type="submit">Schedule Patient Visit</button>
  </form>
  <h3 style="margin-top:1rem;">Scheduled visits</h3>
  <ul>{}</ul>
</section>

<section class="card tab-panel" data-tab-group="study-tabs" data-tab-panel="submissions">
  <h2>6) CRF Submission Workflow</h2>
  <form method="post" action="{}" style="margin-bottom:1rem;">
    <label>Admin email</label>
    <input name="admin_email" value="{}" required />
    <label>Template ID</label>
    <input name="template_id" list="study-template-options" value="{}" required />
    <label>Patient ID</label>
    <input name="patient_id" list="study-patient-options" placeholder="patient-uuid" required />
    <label>Patient visit ID (optional)</label>
    <input name="patient_visit_id" list="study-visit-options" placeholder="patient-visit-uuid" />
    <label>Answers JSON</label>
    <textarea name="answers_json">{{}}</textarea>
    <button type="submit">Create CRF Submission (Draft)</button>
  </form>
  <p><strong>Selected submission:</strong> {}</p>
  <form method="post" action="{}" style="display:inline-block; margin-right:0.5rem;">
    <input type="hidden" name="admin_email" value="{}" />
    <button type="submit">Mark Submitted</button>
  </form>
  <form method="post" action="{}" style="display:inline-block;">
    <input type="hidden" name="admin_email" value="{}" />
    <button type="submit">Lock Submission</button>
  </form>
  <h3 style="margin-top:1rem;">Patients</h3>
  <ul>{}</ul>
  <h3 style="margin-top:1rem;">Submissions</h3>
  <ul>{}</ul>
</section>

<section class="card tab-panel" data-tab-group="study-tabs" data-tab-panel="queries">
  <h2>7) Monitor Query Management</h2>
  <form method="post" action="{}">
    <label>Admin email</label>
    <input name="admin_email" value="{}" required />
    <label>Submission ID</label>
    <input name="submission_id" list="study-submission-options" value="{}" placeholder="submission-uuid" required />
    <label>Field key</label>
    <input name="field_key" placeholder="systolic_bp" required />
    <label>Query text</label>
    <input name="query_text" placeholder="Please verify value source document." required />
    <button type="submit">Create Data Query</button>
  </form>
  <h3 style="margin-top:1rem;">Queries</h3>
  <ul>{}</ul>
</section>

<section class="card tab-panel" data-tab-group="study-tabs" data-tab-panel="startup">
  <h2>8) Study Startup Checklist</h2>
  <form method="post" action="{}">
    <label>Admin email</label>
    <input name="admin_email" value="{}" required />
    <label>Item code</label>
    <input name="item_code" placeholder="irb_approval_documented" required />
    <label>Item label</label>
    <input name="item_label" placeholder="IRB / ethics approval documented" required />
    <label>Completed</label>
    <input type="checkbox" name="completed" value="true" />
    <label>Notes</label>
    <input name="notes" placeholder="Startup note" />
    <button type="submit">Upsert Startup Item</button>
  </form>
  <ul>{}</ul>
</section>

<section class="card tab-panel" data-tab-group="study-tabs" data-tab-panel="close">
  <h2>9) Study Close Checklist</h2>
  <form method="post" action="{}">
    <label>Admin email</label>
    <input name="admin_email" value="{}" required />
    <label>Item code</label>
    <input name="item_code" placeholder="database_lock_complete" required />
    <label>Item label</label>
    <input name="item_label" placeholder="Database lock completed and signed off" required />
    <label>Completed</label>
    <input type="checkbox" name="completed" value="true" />
    <label>Notes</label>
    <input name="notes" placeholder="Closure note" />
    <button type="submit">Upsert Checklist Item</button>
  </form>
  <ul>{}</ul>
</section>
</div>

<datalist id="study-patient-options">{}</datalist>
<datalist id="study-template-options">{}</datalist>
<datalist id="study-visit-template-options">{}</datalist>
<datalist id="study-visit-options">{}</datalist>
<datalist id="study-submission-options">{}</datalist>
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
        html_escape(admin_email.trim()),
        selected_org_value,
        study_rows_html,
        readiness_html,
        operational_summary_html,
        phase_action,
        html_escape(admin_email.trim()),
        phase_events_html,
        crf_template_action,
        html_escape(admin_email.trim()),
        templates_html,
        if selected_template_value.is_empty() {
            "<span class=\"muted\">none selected</span>".to_string()
        } else {
            selected_template_value.clone()
        },
        crf_field_action,
        html_escape(admin_email.trim()),
        fields_html,
        visit_template_action,
        html_escape(admin_email.trim()),
        visit_templates_html,
        schedule_visit_action,
        html_escape(admin_email.trim()),
        patient_visits_html,
        create_submission_action,
        html_escape(admin_email.trim()),
        selected_template_value,
        if selected_submission_value.is_empty() {
            "<span class=\"muted\">none selected</span>".to_string()
        } else {
            selected_submission_value.clone()
        },
        submit_submission_action,
        html_escape(admin_email.trim()),
        lock_submission_action,
        html_escape(admin_email.trim()),
        patients_html,
        submissions_html,
        create_query_action,
        html_escape(admin_email.trim()),
        selected_submission_value,
        data_queries_html,
        startup_checklist_action,
        html_escape(admin_email.trim()),
        startup_checklist_html,
        checklist_action,
        html_escape(admin_email.trim()),
        checklist_html,
        patient_options_html,
        template_options_html,
        visit_template_options_html,
        visit_options_html,
        submission_options_html
    );
    Ok(Html(render_cingulum_page("Study Workbench", body)))
}

async fn submit_create_study_from_ui(
    State(ctx): State<AppContext>,
    Form(form): Form<StudyCreateForm>,
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
    if form.study_name.trim().is_empty() {
        return Err(ApiError::Validation("study_name is required".to_string()));
    }
    let planned_enrollment = form
        .planned_enrollment
        .trim()
        .parse::<i32>()
        .unwrap_or(0)
        .max(0);
    let study = ctx
        .db
        .create_study_project(
            organization_id,
            form.study_name.trim(),
            form.therapeutic_area.trim(),
            optional_non_empty(form.protocol_code.trim()),
            planned_enrollment,
            optional_non_empty(form.clinicaltrials_gov_id.trim()),
            form.study_summary.trim(),
        )
        .await
        .map_err(ApiError::internal)?;
    Ok(Redirect::to(&format!(
        "/ui/studies?admin_email={}&organization_id={}&project_id={}&notice={}",
        query_escape(form.admin_email.trim()),
        organization_id,
        study.id,
        query_escape("Study created in pre_study phase")
    )))
}

async fn submit_study_phase_transition(
    State(ctx): State<AppContext>,
    Path(project_id): Path<Uuid>,
    Form(form): Form<StudyPhaseTransitionForm>,
) -> Result<Redirect, ApiError> {
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
    let changed_by_user_id = ctx
        .db
        .get_user_by_email(form.admin_email.trim())
        .await
        .map_err(ApiError::internal)?
        .map(|u| u.id);
    ctx.db
        .transition_study_phase(
            project_id,
            form.next_phase.trim(),
            changed_by_user_id,
            form.notes.trim(),
        )
        .await
        .map_err(ApiError::internal)?;
    Ok(Redirect::to(&format!(
        "/ui/studies?admin_email={}&organization_id={}&project_id={}&notice={}",
        query_escape(form.admin_email.trim()),
        project.organization_id,
        project_id,
        query_escape("Study phase updated")
    )))
}

async fn submit_create_study_crf_template(
    State(ctx): State<AppContext>,
    Path(project_id): Path<Uuid>,
    Form(form): Form<StudyCrfTemplateForm>,
) -> Result<Redirect, ApiError> {
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
    let created_by_user_id = ctx
        .db
        .get_user_by_email(form.admin_email.trim())
        .await
        .map_err(ApiError::internal)?
        .map(|u| u.id);
    let template = ctx
        .db
        .create_study_crf_template(
            project_id,
            form.name.trim(),
            form.description.trim(),
            form.applicable_phase.trim(),
            created_by_user_id,
        )
        .await
        .map_err(ApiError::internal)?;
    Ok(Redirect::to(&format!(
        "/ui/studies?admin_email={}&organization_id={}&project_id={}&template_id={}&notice={}",
        query_escape(form.admin_email.trim()),
        project.organization_id,
        project_id,
        template.id,
        query_escape("CRF template created")
    )))
}

async fn submit_add_study_crf_field(
    State(ctx): State<AppContext>,
    Path(template_id): Path<Uuid>,
    Form(form): Form<StudyCrfFieldForm>,
) -> Result<Redirect, ApiError> {
    let template = ctx
        .db
        .get_study_crf_template(template_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("template not found".to_string()))?;
    let project = ctx
        .db
        .get_project(template.project_id)
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
    let options_json = normalize_options_json_input(form.options_json.trim());
    let display_order = form.display_order.trim().parse::<i32>().unwrap_or(0).max(0);
    ctx.db
        .add_study_crf_field(
            template_id,
            form.field_key.trim(),
            form.field_label.trim(),
            form.field_type.trim(),
            form.required.is_some(),
            &options_json,
            display_order,
        )
        .await
        .map_err(ApiError::internal)?;
    Ok(Redirect::to(&format!(
        "/ui/studies?admin_email={}&organization_id={}&project_id={}&template_id={}&notice={}",
        query_escape(form.admin_email.trim()),
        project.organization_id,
        project.id,
        template_id,
        query_escape("CRF field added")
    )))
}

async fn submit_publish_study_crf_template(
    State(ctx): State<AppContext>,
    Path(template_id): Path<Uuid>,
    Form(form): Form<StudyCrfPublishForm>,
) -> Result<Redirect, ApiError> {
    let template = ctx
        .db
        .get_study_crf_template(template_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("template not found".to_string()))?;
    let project = ctx
        .db
        .get_project(template.project_id)
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
        .publish_study_crf_template(template_id)
        .await
        .map_err(ApiError::internal)?;
    Ok(Redirect::to(&format!(
        "/ui/studies?admin_email={}&organization_id={}&project_id={}&template_id={}&notice={}",
        query_escape(form.admin_email.trim()),
        project.organization_id,
        project.id,
        template_id,
        query_escape("CRF template published")
    )))
}

async fn submit_create_study_visit_template(
    State(ctx): State<AppContext>,
    Path(project_id): Path<Uuid>,
    Form(form): Form<StudyVisitTemplateForm>,
) -> Result<Redirect, ApiError> {
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
    let target_day = form.target_day.trim().parse::<i32>().unwrap_or(0);
    let window_before_days = form
        .window_before_days
        .trim()
        .parse::<i32>()
        .unwrap_or(0)
        .max(0);
    let window_after_days = form
        .window_after_days
        .trim()
        .parse::<i32>()
        .unwrap_or(0)
        .max(0);
    ctx.db
        .create_study_visit_template(
            project_id,
            form.visit_code.trim(),
            form.visit_name.trim(),
            target_day,
            window_before_days,
            window_after_days,
            form.required.is_some(),
        )
        .await
        .map_err(ApiError::internal)?;
    Ok(Redirect::to(&format!(
        "/ui/studies?admin_email={}&organization_id={}&project_id={}&notice={}",
        query_escape(form.admin_email.trim()),
        project.organization_id,
        project_id,
        query_escape("Visit template created")
    )))
}

async fn submit_schedule_patient_visit(
    State(ctx): State<AppContext>,
    Path(project_id): Path<Uuid>,
    Form(form): Form<StudyScheduleVisitForm>,
) -> Result<Redirect, ApiError> {
    let patient_id = parse_uuid_field(&form.patient_id, "patient_id")?;
    let visit_template_id = parse_uuid_field(&form.visit_template_id, "visit_template_id")?;
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
    let scheduled_for = parse_optional_date(&form.scheduled_for)?;
    ctx.db
        .schedule_patient_study_visit(project_id, patient_id, visit_template_id, scheduled_for)
        .await
        .map_err(ApiError::internal)?;
    Ok(Redirect::to(&format!(
        "/ui/studies?admin_email={}&organization_id={}&project_id={}&notice={}",
        query_escape(form.admin_email.trim()),
        project.organization_id,
        project_id,
        query_escape("Patient visit scheduled")
    )))
}

async fn submit_create_study_crf_submission(
    State(ctx): State<AppContext>,
    Path(project_id): Path<Uuid>,
    Form(form): Form<StudyCreateSubmissionForm>,
) -> Result<Redirect, ApiError> {
    let template_id = parse_uuid_field(&form.template_id, "template_id")?;
    let patient_id = parse_uuid_field(&form.patient_id, "patient_id")?;
    let patient_visit_id = if form.patient_visit_id.trim().is_empty() {
        None
    } else {
        Some(parse_uuid_field(
            &form.patient_visit_id,
            "patient_visit_id",
        )?)
    };
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
    let entered_by_user_id = ctx
        .db
        .get_user_by_email(form.admin_email.trim())
        .await
        .map_err(ApiError::internal)?
        .map(|u| u.id);
    let submission = ctx
        .db
        .create_study_crf_submission(
            project_id,
            template_id,
            patient_id,
            patient_visit_id,
            form.answers_json.trim(),
            entered_by_user_id,
        )
        .await
        .map_err(ApiError::internal)?;
    Ok(Redirect::to(&format!(
        "/ui/studies?admin_email={}&organization_id={}&project_id={}&submission_id={}&notice={}",
        query_escape(form.admin_email.trim()),
        project.organization_id,
        project_id,
        submission.id,
        query_escape("CRF submission created")
    )))
}

async fn submit_mark_study_crf_submission_submitted(
    State(ctx): State<AppContext>,
    Path(submission_id): Path<Uuid>,
    Form(form): Form<StudySubmissionActionForm>,
) -> Result<Redirect, ApiError> {
    let submission = ctx
        .db
        .get_study_crf_submission(submission_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("submission not found".to_string()))?;
    let project = ctx
        .db
        .get_project(submission.project_id)
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
        .submit_study_crf_submission(submission_id)
        .await
        .map_err(ApiError::internal)?;
    Ok(Redirect::to(&format!(
        "/ui/studies?admin_email={}&organization_id={}&project_id={}&submission_id={}&notice={}",
        query_escape(form.admin_email.trim()),
        project.organization_id,
        project.id,
        submission_id,
        query_escape("Submission marked submitted")
    )))
}

async fn submit_lock_study_crf_submission(
    State(ctx): State<AppContext>,
    Path(submission_id): Path<Uuid>,
    Form(form): Form<StudySubmissionActionForm>,
) -> Result<Redirect, ApiError> {
    let submission = ctx
        .db
        .get_study_crf_submission(submission_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("submission not found".to_string()))?;
    let project = ctx
        .db
        .get_project(submission.project_id)
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
        .lock_study_crf_submission(submission_id)
        .await
        .map_err(ApiError::internal)?;
    Ok(Redirect::to(&format!(
        "/ui/studies?admin_email={}&organization_id={}&project_id={}&submission_id={}&notice={}",
        query_escape(form.admin_email.trim()),
        project.organization_id,
        project.id,
        submission_id,
        query_escape("Submission locked")
    )))
}

async fn submit_create_study_data_query(
    State(ctx): State<AppContext>,
    Path(project_id): Path<Uuid>,
    Form(form): Form<StudyCreateQueryForm>,
) -> Result<Redirect, ApiError> {
    let submission_id = parse_uuid_field(&form.submission_id, "submission_id")?;
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
    let raised_by_user_id = ctx
        .db
        .get_user_by_email(form.admin_email.trim())
        .await
        .map_err(ApiError::internal)?
        .map(|u| u.id);
    ctx.db
        .create_study_data_query(
            project_id,
            submission_id,
            form.field_key.trim(),
            form.query_text.trim(),
            raised_by_user_id,
        )
        .await
        .map_err(ApiError::internal)?;
    Ok(Redirect::to(&format!(
        "/ui/studies?admin_email={}&organization_id={}&project_id={}&submission_id={}&notice={}",
        query_escape(form.admin_email.trim()),
        project.organization_id,
        project_id,
        submission_id,
        query_escape("Data query created")
    )))
}

async fn submit_respond_study_data_query(
    State(ctx): State<AppContext>,
    Path(query_id): Path<Uuid>,
    Form(form): Form<StudyRespondQueryForm>,
) -> Result<Redirect, ApiError> {
    let query = ctx
        .db
        .get_study_data_query(query_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("data query not found".to_string()))?;
    let project = ctx
        .db
        .get_project(query.project_id)
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
        .respond_study_data_query(query_id, form.response_text.trim())
        .await
        .map_err(ApiError::internal)?;
    Ok(Redirect::to(&format!(
        "/ui/studies?admin_email={}&organization_id={}&project_id={}&submission_id={}&notice={}",
        query_escape(form.admin_email.trim()),
        project.organization_id,
        project.id,
        query.submission_id,
        query_escape("Data query response saved")
    )))
}

async fn submit_close_study_data_query(
    State(ctx): State<AppContext>,
    Path(query_id): Path<Uuid>,
    Form(form): Form<StudyCloseQueryForm>,
) -> Result<Redirect, ApiError> {
    let query = ctx
        .db
        .get_study_data_query(query_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("data query not found".to_string()))?;
    let project = ctx
        .db
        .get_project(query.project_id)
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
    let resolver = ctx
        .db
        .get_user_by_email(form.admin_email.trim())
        .await
        .map_err(ApiError::internal)?
        .map(|u| u.id);
    ctx.db
        .close_study_data_query(query_id, resolver)
        .await
        .map_err(ApiError::internal)?;
    Ok(Redirect::to(&format!(
        "/ui/studies?admin_email={}&organization_id={}&project_id={}&submission_id={}&notice={}",
        query_escape(form.admin_email.trim()),
        project.organization_id,
        project.id,
        query.submission_id,
        query_escape("Data query closed")
    )))
}

async fn submit_set_study_startup_checklist_item(
    State(ctx): State<AppContext>,
    Path(project_id): Path<Uuid>,
    Form(form): Form<StudyChecklistItemForm>,
) -> Result<Redirect, ApiError> {
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
    let completed = form.completed.is_some();
    let completed_by_user_id = if completed {
        ctx.db
            .get_user_by_email(form.admin_email.trim())
            .await
            .map_err(ApiError::internal)?
            .map(|u| u.id)
    } else {
        None
    };
    ctx.db
        .set_study_startup_checklist_item(
            project_id,
            form.item_code.trim(),
            form.item_label.trim(),
            completed,
            completed_by_user_id,
            form.notes.trim(),
        )
        .await
        .map_err(ApiError::internal)?;
    Ok(Redirect::to(&format!(
        "/ui/studies?admin_email={}&organization_id={}&project_id={}&notice={}",
        query_escape(form.admin_email.trim()),
        project.organization_id,
        project_id,
        query_escape("Startup checklist item updated")
    )))
}

async fn submit_set_study_close_checklist_item(
    State(ctx): State<AppContext>,
    Path(project_id): Path<Uuid>,
    Form(form): Form<StudyChecklistItemForm>,
) -> Result<Redirect, ApiError> {
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
    let completed = form.completed.is_some();
    let completed_by_user_id = if completed {
        ctx.db
            .get_user_by_email(form.admin_email.trim())
            .await
            .map_err(ApiError::internal)?
            .map(|u| u.id)
    } else {
        None
    };
    ctx.db
        .set_study_close_checklist_item(
            project_id,
            form.item_code.trim(),
            form.item_label.trim(),
            completed,
            completed_by_user_id,
            form.notes.trim(),
        )
        .await
        .map_err(ApiError::internal)?;
    Ok(Redirect::to(&format!(
        "/ui/studies?admin_email={}&organization_id={}&project_id={}&notice={}",
        query_escape(form.admin_email.trim()),
        project.organization_id,
        project_id,
        query_escape("Checklist item updated")
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

fn optional_non_empty(input: &str) -> Option<&str> {
    if input.trim().is_empty() {
        None
    } else {
        Some(input.trim())
    }
}

fn normalize_options_json_input(input: &str) -> String {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return "[]".to_string();
    }
    if trimmed.starts_with('[') {
        if serde_json::from_str::<serde_json::Value>(trimmed).is_ok() {
            return trimmed.to_string();
        }
        return "[]".to_string();
    }
    let values = trimmed
        .split(',')
        .map(|part| part.trim())
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    serde_json::to_string(&values).unwrap_or_else(|_| "[]".to_string())
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
      --cg-paper: #FFFDF8;
    }
    * { box-sizing: border-box; }
    body {
      margin: 0;
      font-family: Inter, Arial, sans-serif;
      color: var(--cg-navy);
      background:
        radial-gradient(circle at 10% 0%, rgba(240, 87, 8, 0.12) 0%, transparent 35%),
        radial-gradient(circle at 90% 12%, rgba(2, 24, 43, 0.12) 0%, transparent 32%),
        linear-gradient(180deg, #f7f5ef 0%, var(--cg-cream) 100%);
    }
    .page {
      max-width: 1180px;
      margin: 1.5rem auto;
      padding: 0 1rem 2.2rem;
    }
    .brand-wrap {
      display: flex;
      align-items: center;
      justify-content: space-between;
      background: rgba(255, 253, 248, 0.7);
      border: 1px solid rgba(197, 183, 171, 0.7);
      border-radius: 16px;
      padding: 0.95rem 1rem;
      backdrop-filter: blur(3px);
      box-shadow: 0 8px 24px rgba(2, 24, 43, 0.08);
      margin-bottom: 0.95rem;
    }
    .brand {
      color: var(--cg-forest);
      font-size: 0.88rem;
      letter-spacing: 0.04em;
      text-transform: uppercase;
      font-weight: 700;
      margin: 0;
    }
    .brand-sub {
      color: #334a61;
      font-size: 0.84rem;
    }
    .card {
      background: linear-gradient(180deg, rgba(255, 253, 248, 0.98) 0%, rgba(255, 250, 243, 0.96) 100%);
      border: 1px solid rgba(197, 183, 171, 0.9);
      border-radius: 16px;
      box-shadow: 0 14px 28px rgba(2, 24, 43, 0.09);
      padding: 1rem 1.1rem 1.15rem;
      margin-bottom: 1.05rem;
    }
    h1, h2, h3 { margin-top: 0; color: var(--cg-navy); letter-spacing: 0.01em; }
    h1 { font-size: 1.95rem; margin-bottom: 0.5rem; }
    h2 { font-size: 1.16rem; margin-bottom: 0.75rem; }
    h3 { font-size: 0.98rem; margin-bottom: 0.6rem; }
    p, li, label, small { color: #10263d; }
    a { color: var(--cg-forest); font-weight: 600; }
    a:hover { color: var(--cg-orange); }
    ul { padding-left: 1.1rem; margin: 0.5rem 0 0; }
    li { margin-bottom: 0.28rem; }
    label {
      font-size: 0.84rem;
      font-weight: 700;
      margin-bottom: 0.28rem;
      margin-top: 0.55rem;
      display: block;
    }
    input, textarea, select {
      width: 100%;
      border: 1px solid rgba(197, 183, 171, 0.95);
      border-radius: 11px;
      padding: 0.64rem;
      background: #fffefc;
      color: var(--cg-navy);
      box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.65);
    }
    input:focus, textarea:focus, select:focus {
      outline: 2px solid rgba(240, 87, 8, 0.3);
      border-color: var(--cg-orange);
    }
    textarea { min-height: 140px; }
    button {
      background: linear-gradient(135deg, #f36e1d 0%, var(--cg-orange) 100%);
      color: #fff;
      border: none;
      border-radius: 11px;
      padding: 0.64rem 0.95rem;
      cursor: pointer;
      font-weight: 700;
      margin-top: 0.62rem;
      box-shadow: 0 10px 18px rgba(240, 87, 8, 0.25);
    }
    button:hover { filter: brightness(0.97); transform: translateY(-1px); }
    .muted { color: #41566d; }
    .notice {
      background: #fff3ea;
      border: 1px solid #f4c6ad;
      border-left: 4px solid var(--cg-orange);
      border-radius: 10px;
      padding: 0.72rem 0.82rem;
      font-weight: 600;
      margin-bottom: 0.95rem;
    }
    .status-chip {
      display: inline-block;
      padding: 0.18rem 0.6rem;
      border-radius: 999px;
      background: #efe6de;
      color: var(--cg-forest);
      font-weight: 700;
      font-size: 0.84rem;
    }
    .tab-shell { margin-top: 0.9rem; }
    .tab-bar {
      display: flex;
      gap: 0.5rem;
      flex-wrap: wrap;
      margin-bottom: 0.8rem;
      position: sticky;
      top: 0.6rem;
      z-index: 3;
      background: rgba(247, 245, 239, 0.92);
      border: 1px solid rgba(197, 183, 171, 0.95);
      border-radius: 13px;
      padding: 0.45rem;
      backdrop-filter: blur(5px);
    }
    .tab-button {
      border: 1px solid transparent;
      border-radius: 999px;
      background: transparent;
      color: #30475f;
      font-weight: 700;
      font-size: 0.82rem;
      padding: 0.5rem 0.8rem;
      margin: 0;
      box-shadow: none;
    }
    .tab-button.is-active {
      background: var(--cg-navy);
      color: #fff;
      border-color: rgba(2, 24, 43, 0.4);
      box-shadow: 0 7px 16px rgba(2, 24, 43, 0.22);
    }
    .tab-panel { display: none; }
    .tab-panel.is-active { display: block; }
    @media (max-width: 740px) {
      .brand-wrap { flex-direction: column; align-items: flex-start; gap: 0.35rem; }
      .tab-bar { position: static; }
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
    <div class="brand-wrap">
      <div class="brand">Cingulum Foundation Inc.</div>
      <div class="brand-sub">Virivu Research Cloud</div>
    </div>
    {}
  </main>
  <script>
    (() => {{
      const bars = document.querySelectorAll('.tab-bar[data-tab-group]');
      bars.forEach((bar) => {{
        const group = bar.getAttribute('data-tab-group');
        const buttons = Array.from(bar.querySelectorAll('.tab-button[data-tab-id]'));
        const panels = Array.from(document.querySelectorAll(`.tab-panel[data-tab-group="${{group}}"]`));
        if (buttons.length === 0 || panels.length === 0) return;
        const activate = (tabId) => {{
          buttons.forEach((btn) => {{
            const active = btn.getAttribute('data-tab-id') === tabId;
            btn.classList.toggle('is-active', active);
            btn.setAttribute('aria-selected', active ? 'true' : 'false');
          }});
          panels.forEach((panel) => {{
            const active = panel.getAttribute('data-tab-panel') === tabId;
            panel.classList.toggle('is-active', active);
          }});
        }};
        let initial = buttons.find((b) => b.classList.contains('is-active'))?.getAttribute('data-tab-id');
        if (!initial) initial = buttons[0].getAttribute('data-tab-id');
        activate(initial);
        buttons.forEach((btn) => {{
          btn.addEventListener('click', () => activate(btn.getAttribute('data-tab-id')));
        }});
      }});
    }})();
  </script>
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
