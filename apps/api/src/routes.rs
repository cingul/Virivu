use axum::{
    extract::{Form, Multipart, Path, Query, Request, State},
    http::{header, HeaderValue, StatusCode},
    middleware::{from_fn_with_state, Next},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use lopdf::{Document as LopdfDocument, Object as LopdfObject};
use scraper::{Html as ParsedHtml, Selector};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    net::IpAddr,
};
use uuid::Uuid;

use crate::{
    auth::{extract_bearer_token, verify_google_workspace_user, AuthError, AuthenticatedUser},
    config::Config,
    db::Db,
    models::{DataUseAgreement, DataUseAgreementSignature, OutboundEmail, StudyCrfField},
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
        .route("/favicon.ico", get(favicon))
        .route("/ui", get(render_ui_home))
        .route("/ui/foundation", get(render_foundation_command_center))
        .route("/ui/app", get(render_app_dashboard))
        .route("/ui/studies", get(render_study_workbench))
        .route("/ui/studies/create", post(submit_create_study_from_ui))
        .route("/ui/studies/phase", post(submit_study_phase_transition_v2))
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
            "/ui/studies/templates/{template_id}/import-html-fields",
            post(submit_import_study_crf_fields_html),
        )
        .route(
            "/ui/studies/templates/{template_id}/bulk-delete-fields",
            post(submit_bulk_delete_study_crf_fields),
        )
        .route(
            "/ui/studies/templates/{template_id}/bulk-delete-corrupted-fields",
            post(submit_bulk_delete_corrupted_study_crf_fields),
        )
        .route(
            "/ui/studies/fields/{field_id}/update",
            post(submit_update_study_crf_field),
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

async fn favicon() -> impl IntoResponse {
    StatusCode::NO_CONTENT
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
    parent_organization_id: Option<Uuid>,
    organization_kind: Option<String>,
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
        .create_organization(
            payload.name.trim(),
            payload.parent_organization_id,
            payload.organization_kind.as_deref().map(str::trim),
        )
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
    parent_organization_id: String,
    organization_kind: String,
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
    project_id: Option<String>,
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
    #[serde(default)]
    options_json: String,
    #[serde(default)]
    options_text: String,
    display_order: String,
}

#[derive(Debug, Deserialize)]
struct StudyCrfBulkDeleteForm {
    admin_email: String,
    confirmation_text: String,
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

#[derive(Debug, Default, Deserialize)]
struct UiHomeQuery {
    admin_email: Option<String>,
    notice: Option<String>,
}

async fn render_ui_home(Query(query): Query<UiHomeQuery>) -> Html<String> {
    let admin_email = query
        .admin_email
        .unwrap_or_else(|| "arcot@cingulum.org".to_string());
    let notice_html = query
        .notice
        .map(|notice| format!(r#"<p class="notice">{}</p>"#, html_escape(notice.trim())))
        .unwrap_or_default();

    let body = format!(
        r#"
<main class="home-shell">
  <section class="home-card logo-card">
    <div class="cx-logo-wrap">
      <div class="cx-logo">CX</div>
    </div>
    <h1>Cingulum Foundation Inc.</h1>
    <p class="muted">Virivu Research Cloud</p>
    <p class="home-copy">Accelerating research operations across hospitals, sponsors, and partner institutions through secure digital workflows.</p>
  </section>

  <section class="home-card login-card">
    <h2>Login to Command Center</h2>
    <p class="muted">Use your workspace admin email to open the foundation command center.</p>
    {}
    <form method="get" action="/ui/foundation">
      <label>Admin email</label>
      <input type="email" name="admin_email" value="{}" placeholder="name@cingulum.org" required />
      <button type="submit">Enter Command Center</button>
    </form>
  </section>
</main>
"#,
        notice_html,
        html_escape(admin_email.trim())
    );

    Html(render_home_page("Virivu Research Cloud", body))
}

async fn render_foundation_command_center(
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
        .filter(|org_id| organizations.iter().any(|org| org.id == *org_id))
        .or_else(|| {
            organizations
                .iter()
                .find(|org| {
                    org.organization_kind == "platform_root"
                        || org.name.eq_ignore_ascii_case("Cingulum Foundation Inc.")
                })
                .map(|org| org.id)
        })
        .or_else(|| organizations.first().map(|org| org.id));

    let selected_org = selected_org_id
        .and_then(|org_id| organizations.iter().find(|org| org.id == org_id).cloned());

    let child_organizations = if let Some(org_id) = selected_org_id {
        ctx.db
            .list_child_organizations(org_id)
            .await
            .map_err(ApiError::internal)?
    } else {
        Vec::new()
    };

    let mut research_network_count = 0usize;
    let mut hospital_count = 0usize;
    let mut tenant_count = 0usize;
    let mut sponsor_count = 0usize;
    let mut other_kind_count = 0usize;
    for org in &child_organizations {
        match org.organization_kind.trim().to_ascii_lowercase().as_str() {
            "research_network" => research_network_count += 1,
            "hospital" => hospital_count += 1,
            "tenant" => tenant_count += 1,
            "sponsor" => sponsor_count += 1,
            _ => other_kind_count += 1,
        }
    }

    let mut total_projects = 0usize;
    let mut total_sites = 0usize;
    let mut total_duas = 0usize;
    let mut pending_duas = 0usize;
    let mut managed_org_ids = Vec::new();
    if let Some(org_id) = selected_org_id {
        managed_org_ids.push(org_id);
    }
    managed_org_ids.extend(child_organizations.iter().map(|org| org.id));

    for org_id in managed_org_ids {
        let projects = ctx
            .db
            .list_projects_by_organization(org_id)
            .await
            .map_err(ApiError::internal)?;
        total_projects += projects.len();
        for project in projects {
            total_sites += ctx
                .db
                .list_sites_by_project(project.id)
                .await
                .map_err(ApiError::internal)?
                .len();
        }
        let org_duas = ctx
            .db
            .list_data_use_agreements(org_id)
            .await
            .map_err(ApiError::internal)?;
        total_duas += org_duas.len();
        pending_duas += org_duas
            .iter()
            .filter(|dua| {
                let status = dua.status.trim().to_ascii_lowercase();
                status.contains("pending") || status.contains("draft")
            })
            .count();
    }

    let selected_org_value = selected_org_id.map(|id| id.to_string()).unwrap_or_default();
    let selected_org_q = selected_org_id
        .map(|org_id| format!("&organization_id={org_id}"))
        .unwrap_or_default();
    let app_workspace_url = format!("/ui/app?admin_email={}{}", admin_email_q, selected_org_q);
    let study_workspace_url = format!(
        "/ui/studies?admin_email={}{}",
        admin_email_q, selected_org_q
    );
    let dua_workspace_url = format!("/ui/dua?admin_email={}{}", admin_email_q, selected_org_q);

    let notice_html = query
        .notice
        .map(|notice| format!(r#"<p class="notice">{}</p>"#, html_escape(notice.trim())))
        .unwrap_or_default();

    let selected_workspace_label = selected_org
        .as_ref()
        .map(|org| {
            format!(
                "{} ({})",
                html_escape(&org.name),
                html_escape(&org.organization_kind)
            )
        })
        .unwrap_or_else(|| "none selected".to_string());

    let organization_options_html = organizations
        .iter()
        .map(|org| {
            format!(
                r#"<option value="{}">{} ({})</option>"#,
                org.id,
                html_escape(&org.name),
                html_escape(&org.organization_kind)
            )
        })
        .collect::<Vec<_>>()
        .join("");

    let managed_organizations_html = if child_organizations.is_empty() {
        "<li>No partner organizations yet. Use Operations Workspace to create hospitals, tenants, and sponsors under Cingulum Foundation.</li>".to_string()
    } else {
        child_organizations
            .iter()
            .map(|org| {
                format!(
                    r#"<li><strong>{}</strong> <small>(kind: {} · workspace: {} · hex: {})</small><br/><a href="/ui/app?admin_email={}&organization_id={}">Operations</a> · <a href="/ui/studies?admin_email={}&organization_id={}">Studies</a> · <a href="/ui/dua?admin_email={}&organization_id={}">DUA</a></li>"#,
                    html_escape(&org.name),
                    html_escape(&org.organization_kind),
                    html_escape(org.workspace_slug.as_deref().unwrap_or("pending")),
                    html_escape(org.hex_code.as_deref().unwrap_or("pending")),
                    admin_email_q,
                    org.id,
                    admin_email_q,
                    org.id,
                    admin_email_q,
                    org.id
                )
            })
            .collect::<Vec<_>>()
            .join("")
    };

    let mut next_actions = Vec::new();
    if selected_org_id.is_none() {
        next_actions.push(
            "<li>Select the Cingulum Foundation workspace to activate network-level controls.</li>"
                .to_string(),
        );
    }
    if child_organizations.is_empty() {
        next_actions.push(format!(
            r#"<li>Onboard the first partner site or institution in <a href="{}">Operations Workspace</a>.</li>"#,
            app_workspace_url
        ));
    }
    if total_projects == 0 {
        next_actions.push(format!(
            r#"<li>Launch the first study in <a href="{}">Study Workbench</a> to kick off enrollment.</li>"#,
            study_workspace_url
        ));
    }
    if total_duas == 0 {
        next_actions.push(format!(
            r#"<li>Draft your first cross-site DUA in the <a href="{}">DUA Console</a>.</li>"#,
            dua_workspace_url
        ));
    }
    if pending_duas > 0 {
        next_actions.push(format!(
            r#"<li>Review {} pending DUA record(s) in the <a href="{}">DUA Console</a> to unblock study startup.</li>"#,
            pending_duas, dua_workspace_url
        ));
    }
    if next_actions.is_empty() {
        next_actions.push(format!(
            r#"<li>Network looks active. Continue monitoring execution in <a href="{}">Study Workbench</a> and <a href="{}">Operations Workspace</a>.</li>"#,
            study_workspace_url, app_workspace_url
        ));
    }
    let next_actions_html = next_actions.join("");

    let body = format!(
        r#"
<h1>Cingulum Foundation Command Center</h1>
<p class="muted">Administer all partner sites, coordinate legal and operational workflows, and accelerate research delivery through a single digital control plane.</p>
{}

<section class="card">
  <h2>Foundation workspace context</h2>
  <form method="get" action="/ui/foundation">
    <label>Foundation admin email</label>
    <input name="admin_email" value="{}" required />
    <label>Workspace to administer</label>
    <input name="organization_id" list="foundation-organization-options" value="{}" placeholder="Select Cingulum Foundation root" />
    <button type="submit">Load command center</button>
  </form>
  <p style="margin-top:0.75rem;"><strong>Active workspace:</strong> {}</p>
  <p class="muted">Tenant-isolated controls remain in effect. All actions are scoped to the selected organization and its managed sites.</p>
</section>

<div style="display:grid;grid-template-columns:repeat(auto-fit,minmax(220px,1fr));gap:0.8rem;">
  <section class="card">
    <h2>Network coverage</h2>
    <p><strong>Managed organizations:</strong> {}</p>
    <p><strong>Research networks:</strong> {}</p>
    <p><strong>Hospitals:</strong> {}</p>
    <p><strong>Tenants:</strong> {}</p>
    <p><strong>Sponsors:</strong> {}</p>
    <p><strong>Other kinds:</strong> {}</p>
  </section>
  <section class="card">
    <h2>Research operations snapshot</h2>
    <p><strong>Projects tracked:</strong> {}</p>
    <p><strong>Sites configured:</strong> {}</p>
    <p><strong>DUAs tracked:</strong> {}</p>
    <p><strong>Pending DUA actions:</strong> {}</p>
  </section>
</div>

<section class="card">
  <h2>Guided execution path</h2>
  <ol>
    <li><strong>Onboard institutions:</strong> setup organizations and site structure in <a href="{}">Operations Workspace</a>.</li>
    <li><strong>Launch and monitor studies:</strong> manage startup, CRFs, visits, and query resolution in <a href="{}">Study Workbench</a>.</li>
    <li><strong>Close legal bottlenecks:</strong> draft and finalize agreements in <a href="{}">DUA Console</a>.</li>
    <li><strong>Accelerate with digital tools:</strong> standardize data capture, reduce manual handoffs, and maintain real-time program visibility.</li>
  </ol>
</section>

<section class="card">
  <h2>Recommended next actions</h2>
  <ul>{}</ul>
</section>

<section class="card">
  <h2>Managed organizations</h2>
  <ul>{}</ul>
</section>
"#,
        notice_html,
        html_escape(admin_email.trim()),
        selected_org_value,
        selected_workspace_label,
        child_organizations.len(),
        research_network_count,
        hospital_count,
        tenant_count,
        sponsor_count,
        other_kind_count,
        total_projects,
        total_sites,
        total_duas,
        pending_duas,
        app_workspace_url,
        study_workspace_url,
        dua_workspace_url,
        next_actions_html,
        managed_organizations_html
    );

    let page = format!(
        r#"
{}
<datalist id="foundation-organization-options">{}</datalist>
"#,
        body, organization_options_html
    );

    Ok(Html(render_cingulum_page(
        "Cingulum Foundation Command Center",
        page,
    )))
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
        .or_else(|| {
            organizations
                .iter()
                .find(|org| org.organization_kind == "platform_root")
                .map(|org| org.id)
                .or_else(|| organizations.first().map(|org| org.id))
        });

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

    let child_organizations = if let Some(org_id) = selected_org_id {
        ctx.db
            .list_child_organizations(org_id)
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
                    r#"<li><a href="/ui/app?admin_email={}&organization_id={}">{}</a> <small>(kind: {} · id: {} · hex: {} · parent: {})</small></li>"#,
                    admin_email_q,
                    org.id,
                    html_escape(&org.name),
                    html_escape(&org.organization_kind),
                    org.id,
                    html_escape(org.hex_code.as_deref().unwrap_or("pending")),
                    org.parent_organization_id
                        .map(|id| id.to_string())
                        .unwrap_or_else(|| "none".to_string())
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
                    r#"<li><a href="/ui/dua/{}?admin_email={}">{}</a> <span class="status-chip">{}</span></li>"#,
                    dua.id,
                    admin_email_q,
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

    let child_organizations_html = if child_organizations.is_empty() {
        "<li>No child organizations under current workspace.</li>".to_string()
    } else {
        child_organizations
            .iter()
            .map(|org| {
                format!(
                    "<li><strong>{}</strong> <small>(kind: {} · id: {})</small></li>",
                    html_escape(&org.name),
                    html_escape(&org.organization_kind),
                    org.id
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
    let foundation_hub_url = selected_org_id
        .map(|org_id| {
            format!(
                "/ui/foundation?admin_email={}&organization_id={org_id}",
                admin_email_q
            )
        })
        .unwrap_or_else(|| format!("/ui/foundation?admin_email={}", admin_email_q));

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
  <h3 style="margin-top:0.8rem;">Child organizations</h3>
  <ul>{}</ul>
  <p><a href="{}">Open Cingulum Foundation command center</a></p>
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
    <label>Parent organization ID (optional; blank = Cingulum Foundation root)</label>
    <input name="parent_organization_id" list="app-organization-options" value="{}" placeholder="root-managed child org" />
    <label>Organization type</label>
    <select name="organization_kind">
      <option value="tenant" selected>tenant</option>
      <option value="research_network">research_network</option>
      <option value="hospital">hospital</option>
      <option value="sponsor">sponsor</option>
    </select>
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
        child_organizations_html,
        foundation_hub_url,
        admin_email_q,
        html_escape(admin_email.trim()),
        selected_org_value.clone(),
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
        .or_else(|| {
            organizations
                .iter()
                .find(|org| org.organization_kind == "platform_root")
                .map(|org| org.id)
                .or_else(|| organizations.first().map(|org| org.id))
        });
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
    if let Some(project_id) = selected_project_id {
        ctx.db
            .ensure_default_study_startup_checklist_items(project_id)
            .await
            .map_err(ApiError::internal)?;
    }
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
    let selected_org_q = selected_org_id
        .map(|org_id| format!("&organization_id={org_id}"))
        .unwrap_or_default();
    let selected_project_q = selected_project_id
        .map(|project_id| format!("&project_id={project_id}"))
        .unwrap_or_default();
    let app_dashboard_url = format!(
        "/ui/app?admin_email={}{}{}",
        admin_email_q, selected_org_q, selected_project_q
    );
    let setup_tab_url = format!(
        "/ui/studies?admin_email={}{}{}&tab=setup",
        admin_email_q, selected_org_q, selected_project_q
    );
    let startup_tab_url = format!(
        "/ui/studies?admin_email={}{}{}&tab=startup",
        admin_email_q, selected_org_q, selected_project_q
    );
    let templates_tab_url = format!(
        "/ui/studies?admin_email={}{}{}&tab=crf-templates",
        admin_email_q, selected_org_q, selected_project_q
    );
    let visits_tab_url = format!(
        "/ui/studies?admin_email={}{}{}&tab=visits",
        admin_email_q, selected_org_q, selected_project_q
    );
    let submissions_tab_url = format!(
        "/ui/studies?admin_email={}{}{}&tab=submissions",
        admin_email_q, selected_org_q, selected_project_q
    );
    let queries_tab_url = format!(
        "/ui/studies?admin_email={}{}{}&tab=queries",
        admin_email_q, selected_org_q, selected_project_q
    );
    let close_tab_url = format!(
        "/ui/studies?admin_email={}{}{}&tab=close",
        admin_email_q, selected_org_q, selected_project_q
    );
    let selected_study_label = selected_project_id
        .and_then(|project_id| projects.iter().find(|project| project.id == project_id))
        .map(|project| {
            format!(
                "{} (phase: {})",
                html_escape(&project.name),
                html_escape(&project.lifecycle_phase)
            )
        })
        .unwrap_or_else(|| "<span class=\"muted\">none selected</span>".to_string());

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
                    r#"<li><a href="/ui/studies?admin_email={}{}&project_id={}&tab=overview">{}</a> <small>(phase: {} · hex: {} · target: {})</small></li>"#,
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
    let active_studies_html = {
        let active_studies = projects
            .iter()
            .filter(|project| {
                matches!(
                    project.lifecycle_phase.trim().to_ascii_lowercase().as_str(),
                    "initiated" | "active" | "monitoring"
                )
            })
            .collect::<Vec<_>>();
        if active_studies.is_empty() {
            "<li>No studies are in initiated/active/monitoring yet.</li>".to_string()
        } else {
            active_studies
                .iter()
                .map(|project| {
                    format!(
                        r#"<li><a href="/ui/studies?admin_email={}{}&project_id={}&tab=overview">{}</a> <small>(phase: {} · enrollment target: {})</small></li>"#,
                        admin_email_q,
                        selected_org_q,
                        project.id,
                        html_escape(&project.name),
                        html_escape(&project.lifecycle_phase),
                        project.planned_enrollment
                    )
                })
                .collect::<Vec<_>>()
                .join("")
        }
    };
    let startup_pending_count = startup_checklist_items
        .iter()
        .filter(|item| !item.completed)
        .count();
    let close_pending_count = checklist_items
        .iter()
        .filter(|item| !item.completed)
        .count();
    let unpublished_templates_count = templates
        .iter()
        .filter(|template| template.status.trim().to_ascii_lowercase() != "published")
        .count();
    let draft_submission_count = submissions
        .iter()
        .filter(|submission| submission.status.trim().to_ascii_lowercase() == "draft")
        .count();
    let submitted_unlocked_count = submissions
        .iter()
        .filter(|submission| submission.status.trim().to_ascii_lowercase() == "submitted")
        .count();
    let open_query_count = data_queries
        .iter()
        .filter(|query| query.status.trim().to_ascii_lowercase() != "closed")
        .count();
    let today = Utc::now().date_naive();
    let overdue_visit_count = patient_visits
        .iter()
        .filter(|visit| {
            let status = visit.status.trim().to_ascii_lowercase();
            if status == "completed" || status == "cancelled" {
                return false;
            }
            match visit.scheduled_for {
                Some(scheduled_for) => scheduled_for < today,
                None => false,
            }
        })
        .count();
    let pending_actions_html = if selected_project_id.is_none() {
        format!(
            r#"<li>Select an active study from the <a href="{}">Study Setup</a> tab to view targeted next steps.</li>"#,
            setup_tab_url
        )
    } else {
        let mut actions = Vec::new();
        if startup_pending_count > 0 {
            actions.push(format!(
                r#"<li><strong>{}</strong> startup checklist item(s) are incomplete. <a href="{}">Complete startup tasks</a>.</li>"#,
                startup_pending_count, startup_tab_url
            ));
        }
        if unpublished_templates_count > 0 {
            actions.push(format!(
                r#"<li><strong>{}</strong> CRF template(s) are still draft. <a href="{}">Publish templates</a> before broad data capture.</li>"#,
                unpublished_templates_count, templates_tab_url
            ));
        }
        if overdue_visit_count > 0 {
            actions.push(format!(
                r#"<li><strong>{}</strong> visit(s) appear overdue. <a href="{}">Review visit schedule</a>.</li>"#,
                overdue_visit_count, visits_tab_url
            ));
        }
        if draft_submission_count > 0 {
            actions.push(format!(
                r#"<li><strong>{}</strong> CRF submission(s) remain in draft. <a href="{}">Submit or complete drafts</a>.</li>"#,
                draft_submission_count, submissions_tab_url
            ));
        }
        if submitted_unlocked_count > 0 {
            actions.push(format!(
                r#"<li><strong>{}</strong> submission(s) are submitted but not locked. <a href="{}">Lock finalized submissions</a>.</li>"#,
                submitted_unlocked_count, submissions_tab_url
            ));
        }
        if open_query_count > 0 {
            actions.push(format!(
                r#"<li><strong>{}</strong> monitor query(ies) are still open. <a href="{}">Respond to queries</a>.</li>"#,
                open_query_count, queries_tab_url
            ));
        }
        if close_pending_count > 0 {
            actions.push(format!(
                r#"<li><strong>{}</strong> close checklist item(s) remain. <a href="{}">Prepare close-out</a> when study reaches closure phase.</li>"#,
                close_pending_count, close_tab_url
            ));
        }
        if actions.is_empty() {
            actions.push(
                "<li>No blocking actions detected for the selected study right now.</li>"
                    .to_string(),
            );
        }
        actions.join("")
    };
    let lifecycle_gate_html = if let Some(readiness) = &readiness {
        let site_gate = if readiness.total_sites > 0 {
            "<span class=\"status-chip\">ready</span>"
        } else {
            "<span class=\"status-chip\">blocked</span>"
        };
        let crf_gate = if readiness.published_crf_templates > 0 {
            "<span class=\"status-chip\">ready</span>"
        } else {
            "<span class=\"status-chip\">blocked</span>"
        };
        let startup_gate = if startup_pending_count == 0 {
            "<span class=\"status-chip\">ready</span>"
        } else {
            "<span class=\"status-chip\">blocked</span>"
        };
        let active_gate = if readiness.total_patients > 0 {
            "<span class=\"status-chip\">ready</span>"
        } else {
            "<span class=\"status-chip\">blocked</span>"
        };
        let close_query_gate = if open_query_count == 0 {
            "<span class=\"status-chip\">ready</span>"
        } else {
            "<span class=\"status-chip\">blocked</span>"
        };
        let close_checklist_gate = if close_pending_count == 0 {
            "<span class=\"status-chip\">ready</span>"
        } else {
            "<span class=\"status-chip\">blocked</span>"
        };
        format!(
            r#"<section class="card" style="margin:0.75rem 0;">
  <h3>Phase readiness gates</h3>
  <ul>
    <li><strong>To initiate:</strong> site configured {} · CRF published {} · startup checklist complete {}</li>
    <li><strong>To set active:</strong> at least one enrolled patient {}</li>
    <li><strong>To close:</strong> no open queries {} · close checklist complete {}</li>
  </ul>
</section>"#,
            site_gate, crf_gate, startup_gate, active_gate, close_query_gate, close_checklist_gate
        )
    } else {
        "<p class=\"muted\">Select a study to view phase readiness gates.</p>".to_string()
    };

    let lifecycle_kpi_cards_html = if let (Some(readiness), Some(summary)) =
        (readiness.as_ref(), operational_summary.as_ref())
    {
        format!(
            r#"<div class="info-grid">
  <article class="info-card">
    <div class="metric-label">Phase</div>
    <div class="metric-value">{}</div>
  </article>
  <article class="info-card">
    <div class="metric-label">Sites configured</div>
    <div class="metric-value">{}</div>
  </article>
  <article class="info-card">
    <div class="metric-label">Published CRFs</div>
    <div class="metric-value">{}</div>
  </article>
  <article class="info-card">
    <div class="metric-label">Enrollment</div>
    <div class="metric-value">{}/{}</div>
  </article>
  <article class="info-card">
    <div class="metric-label">Open queries</div>
    <div class="metric-value">{}</div>
  </article>
  <article class="info-card">
    <div class="metric-label">Startup pending</div>
    <div class="metric-value">{}</div>
  </article>
</div>"#,
            html_escape(&readiness.lifecycle_phase),
            readiness.total_sites,
            readiness.published_crf_templates,
            summary.enrolled_patients,
            summary.planned_enrollment,
            summary.open_data_queries,
            summary.startup_items_pending
        )
    } else {
        "<p class=\"muted\">Select a study to view lifecycle status.</p>".to_string()
    };
    let lifecycle_action_cards_html = if selected_project_id.is_none() {
        "<p class=\"muted\">Choose a study first. You will then see clickable next-step cards here.</p>"
            .to_string()
    } else {
        let mut cards = Vec::new();
        if let Some(readiness) = readiness.as_ref() {
            if readiness.total_sites < 1 {
                cards.push(format!(
                    r#"<a class="action-card is-blocked" href="{}">
  <div class="action-title">Configure first site</div>
  <div class="action-desc">A study cannot be initiated until at least one site is configured.</div>
  <div class="action-tag">Go to Operations Workspace</div>
</a>"#,
                    app_dashboard_url
                ));
            }
            if readiness.published_crf_templates < 1 {
                cards.push(format!(
                    r#"<a class="action-card is-blocked" href="{}">
  <div class="action-title">Publish a CRF template</div>
  <div class="action-desc">At least one CRF template must be published before initiation.</div>
  <div class="action-tag">Open CRF Templates</div>
</a>"#,
                    templates_tab_url
                ));
            }
            if startup_pending_count > 0 {
                cards.push(format!(
                    r#"<a class="action-card is-blocked" href="{}">
  <div class="action-title">Finish startup checklist</div>
  <div class="action-desc">Complete remaining startup tasks to unlock phase initiation.</div>
  <div class="action-tag">Open Startup Checklist</div>
</a>"#,
                    startup_tab_url
                ));
            }
            if readiness.total_patients < 1 {
                cards.push(format!(
                    r#"<a class="action-card is-informative" href="{}">
  <div class="action-title">Prepare first enrollment</div>
  <div class="action-desc">You will need at least one enrolled patient before setting study to active.</div>
  <div class="action-tag">Open Operations Workspace</div>
</a>"#,
                    app_dashboard_url
                ));
            }
        }
        if open_query_count > 0 {
            cards.push(format!(
                r#"<a class="action-card is-informative" href="{}">
  <div class="action-title">Resolve open data queries</div>
  <div class="action-desc">Close open monitor queries to keep lifecycle transitions unblocked.</div>
  <div class="action-tag">Open Queries</div>
</a>"#,
                queries_tab_url
            ));
        }
        if cards.is_empty() {
            cards.push(
                r##"<a class="action-card is-ready" href="#phase-transition-form">
  <div class="action-title">Lifecycle gates look ready</div>
  <div class="action-desc">Core checks are satisfied. You can apply the next phase transition below.</div>
  <div class="action-tag">Go to Phase Transition</div>
</a>"##
                    .to_string(),
            );
        }
        format!(
            "<h3>What to do next</h3><div class=\"action-grid\">{}</div>",
            cards.join("")
        )
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

    let field_count = fields.len();
    let fields_html = if fields.is_empty() {
        "<li>No fields yet for selected template.</li>".to_string()
    } else {
        fields
            .iter()
            .map(|field| {
                let field_type_options_html = render_crf_field_type_options(&field.field_type);
                let options_text_value = options_json_to_lines(&field.options_json);
                format!(
                    r#"<li>
  <strong>{}</strong> <small>Type: {} · {}</small>
  <details style="margin-top:0.35rem;">
    <summary><strong>Edit field</strong></summary>
    <form method="post" action="/ui/studies/fields/{}/update" data-crf-field-form style="margin-top:0.55rem;">
      <label>Admin email</label>
      <input name="admin_email" value="{}" required />
      <label>Field key</label>
      <input name="field_key" value="{}" required />
      <label>Field label</label>
      <input name="field_label" value="{}" required />
      <label>Field type</label>
      <select name="field_type" data-field-type-select required>{}</select>
      <label>Required</label>
      <input type="checkbox" name="required" value="true" {} />
      <div data-options-section>
        <label>Choice options</label>
        <div data-option-list></div>
        <button type="button" data-add-option-button>Add option</button>
        <input type="hidden" data-options-initial value="{}" />
        <input type="hidden" name="options_text" value="" />
        <input type="hidden" name="options_json" value="{}" />
        <small class="muted">Used only for single-select or multi-select field types.</small>
      </div>
      <label>Display order</label>
      <input name="display_order" value="{}" />
      <button type="submit">Save Field Changes</button>
    </form>
  </details>
</li>"#,
                    html_escape(&field.field_label),
                    html_escape(&field.field_type),
                    if field.required { "Required" } else { "Optional" },
                    field.id,
                    html_escape(admin_email.trim()),
                    html_escape(&field.field_key),
                    html_escape(&field.field_label),
                    field_type_options_html,
                    if field.required { "checked" } else { "" },
                    html_escape(&options_text_value),
                    html_escape(&field.options_json),
                    field.display_order
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
                let status = if item.completed {
                    "<span class=\"status-chip\">completed</span>"
                } else {
                    "<span class=\"status-chip\">pending</span>"
                };
                let notes = if item.notes.trim().is_empty() {
                    String::new()
                } else {
                    format!(
                        " <small style=\"display:block;\">Notes: {}</small>",
                        html_escape(&item.notes)
                    )
                };
                let undo_action = if item.completed {
                    let mark_pending_action = selected_project_id
                        .map(|id| format!("/ui/studies/{id}/close-checklist"))
                        .unwrap_or_else(|| "#".to_string());
                    format!(
                        r#"<form method="post" action="{}" style="margin-top:0.4rem;display:grid;grid-template-columns:minmax(0,1fr) auto;gap:0.45rem;align-items:center;">
  <input type="hidden" name="admin_email" value="{}" />
  <input type="hidden" name="item_code" value="{}" />
  <input type="hidden" name="item_label" value="{}" />
  <input name="notes" value="{}" placeholder="Optional note" />
  <button type="submit">Undo complete</button>
</form>"#,
                        mark_pending_action,
                        html_escape(admin_email.trim()),
                        html_escape(&item.item_code),
                        html_escape(&item.item_label),
                        html_escape(&item.notes)
                    )
                } else {
                    String::new()
                };
                format!(
                    "<li><strong>{}</strong> {}{} {}</li>",
                    html_escape(&item.item_label),
                    status,
                    notes,
                    undo_action
                )
            })
            .collect::<Vec<_>>()
            .join("")
    };
    let startup_total_count = startup_checklist_items.len();
    let startup_completed_count = startup_checklist_items
        .iter()
        .filter(|item| item.completed)
        .count();
    let startup_next_task = startup_checklist_items
        .iter()
        .find(|item| !item.completed)
        .map(|item| html_escape(&item.item_label))
        .unwrap_or_else(|| "All startup tasks are complete.".to_string());
    let startup_summary_html = if selected_project_id.is_none() {
        "<p class=\"muted\">Select a study in Overview or Study Setup to manage startup tasks.</p>"
            .to_string()
    } else {
        format!(
            "<p><strong>Progress:</strong> {} / {} completed · {} pending</p><p><strong>Next task:</strong> {}</p>",
            startup_completed_count,
            startup_total_count,
            startup_pending_count,
            startup_next_task
        )
    };
    let startup_workflow_framework_html = if selected_project_id.is_none() {
        String::new()
    } else {
        "<h3>Startup workflow map</h3>
<ol>
  <li><strong>Protocol and budget readiness:</strong> final protocol package and site budget/contract alignment.</li>
  <li><strong>Regulatory approvals:</strong> IRB/ethics and essential regulatory documentation complete.</li>
  <li><strong>Site activation:</strong> site resources, investigator assignment, and operational readiness confirmed.</li>
  <li><strong>EDC/CRF go-live:</strong> forms, permissions, and data review configuration validated.</li>
  <li><strong>Team training:</strong> protocol and SOP training complete for all active staff.</li>
</ol>"
            .to_string()
    };
    let startup_next_action_html = if let Some(next_item) =
        startup_checklist_items.iter().find(|item| !item.completed)
    {
        let mark_complete_action = selected_project_id
            .map(|id| format!("/ui/studies/{id}/startup-checklist"))
            .unwrap_or_else(|| "#".to_string());
        format!(
            r#"<section id="startup-next-task" class="card" style="margin:0.7rem 0;">
  <h3>Next required task</h3>
  <p><strong>{}</strong></p>
  <p class="muted">Complete this task first to unblock study initiation.</p>
  <form method="post" action="{}" style="display:grid;grid-template-columns:minmax(0,1fr) auto;gap:0.5rem;align-items:center;">
    <input type="hidden" name="admin_email" value="{}" />
    <input type="hidden" name="item_code" value="{}" />
    <input type="hidden" name="item_label" value="{}" />
    <input type="hidden" name="completed" value="true" />
    <input name="notes" value="{}" placeholder="Completion note (optional)" />
    <button type="submit">Mark complete and continue</button>
  </form>
</section>"#,
            html_escape(&next_item.item_label),
            mark_complete_action,
            html_escape(admin_email.trim()),
            html_escape(&next_item.item_code),
            html_escape(&next_item.item_label),
            html_escape(&next_item.notes)
        )
    } else if selected_project_id.is_some() {
        format!(
            r#"<p class="notice">Startup checklist is complete. Next: <a href="{}">open Lifecycle tab</a> and transition study phase.</p>"#,
            format!(
                "/ui/studies?admin_email={}{}{}&tab=lifecycle",
                admin_email_q, selected_org_q, selected_project_q
            )
        )
    } else {
        String::new()
    };
    let startup_checklist_html = if startup_checklist_items.is_empty() {
        "<li>No startup checklist items yet. Add one in the advanced section below.</li>"
            .to_string()
    } else {
        startup_checklist_items
            .iter()
            .map(|item| {
                let status = if item.completed {
                    "<span class=\"status-chip\">completed</span>"
                } else {
                    "<span class=\"status-chip\">pending</span>"
                };
                let notes = if item.notes.trim().is_empty() {
                    String::new()
                } else {
                    format!(
                        "<small style=\"display:block;margin:0.25rem 0 0.5rem;\">Notes: {}</small>",
                        html_escape(&item.notes)
                    )
                };
                let completion_action = if item.completed {
                    let mark_pending_action = selected_project_id
                        .map(|id| format!("/ui/studies/{id}/startup-checklist"))
                        .unwrap_or_else(|| "#".to_string());
                    format!(
                        r#"<form method="post" action="{}" style="margin-top:0.45rem;display:grid;grid-template-columns:minmax(0,1fr) auto;gap:0.45rem;align-items:center;">
  <input type="hidden" name="admin_email" value="{}" />
  <input type="hidden" name="item_code" value="{}" />
  <input type="hidden" name="item_label" value="{}" />
  <input name="notes" value="{}" placeholder="Optional note" />
  <button type="submit">Undo complete</button>
</form>"#,
                        mark_pending_action,
                        html_escape(admin_email.trim()),
                        html_escape(&item.item_code),
                        html_escape(&item.item_label),
                        html_escape(&item.notes)
                    )
                } else {
                    let mark_complete_action = selected_project_id
                        .map(|id| format!("/ui/studies/{id}/startup-checklist"))
                        .unwrap_or_else(|| "#".to_string());
                    format!(
                        r#"<form method="post" action="{}" style="margin-top:0.45rem;display:grid;grid-template-columns:minmax(0,1fr) auto;gap:0.45rem;align-items:center;">
  <input type="hidden" name="admin_email" value="{}" />
  <input type="hidden" name="item_code" value="{}" />
  <input type="hidden" name="item_label" value="{}" />
  <input type="hidden" name="completed" value="true" />
  <input name="notes" value="{}" placeholder="Completion note (optional)" />
  <button type="submit">Mark complete</button>
</form>"#,
                        mark_complete_action,
                        html_escape(admin_email.trim()),
                        html_escape(&item.item_code),
                        html_escape(&item.item_label),
                        html_escape(&item.notes)
                    )
                };
                format!(
                    "<li><strong>{}</strong> {}{}{} </li>",
                    html_escape(&item.item_label),
                    status,
                    notes,
                    completion_action
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
    let phase_action = "/ui/studies/phase".to_string();
    let crf_template_action = selected_project_id
        .map(|id| format!("/ui/studies/{id}/crf-template"))
        .unwrap_or_else(|| "#".to_string());
    let crf_field_action = selected_template_id
        .map(|id| format!("/ui/studies/templates/{id}/field"))
        .unwrap_or_else(|| "#".to_string());
    let import_html_fields_action = selected_template_id
        .map(|id| format!("/ui/studies/templates/{id}/import-html-fields"))
        .unwrap_or_else(|| "#".to_string());
    let bulk_delete_fields_action = selected_template_id
        .map(|id| format!("/ui/studies/templates/{id}/bulk-delete-fields"))
        .unwrap_or_else(|| "#".to_string());
    let bulk_delete_corrupted_fields_action = selected_template_id
        .map(|id| format!("/ui/studies/templates/{id}/bulk-delete-corrupted-fields"))
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
  <h2>Overview</h2>
  <p class="muted">Start here: review active studies, then execute pending actions for the selected study.</p>
  <p><strong>Admin:</strong> {}</p>
  <p><strong>Organization:</strong> {}</p>
  <p><strong>Selected study:</strong> {}</p>
  <div style="display:grid;grid-template-columns:repeat(auto-fit,minmax(260px,1fr));gap:0.8rem;margin-top:0.7rem;">
    <section class="card" style="margin:0;">
      <h3>Active studies</h3>
      <ul>{}</ul>
    </section>
    <section class="card" style="margin:0;">
      <h3>Pending actions</h3>
      <ul>{}</ul>
    </section>
  </div>
  <p style="margin-top:0.8rem;"><a href="{}">Back to unified app dashboard</a></p>
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
  {}
  <form id="phase-transition-form" method="post" action="{}">
    <input type="hidden" name="project_id" value="{}" />
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
  <p class="muted">Add new fields below. To edit an existing field, use the <strong>Edit field</strong> option in the list.</p>
  <form method="post" action="{}" data-crf-field-form>
    <label>Admin email</label>
    <input name="admin_email" value="{}" required />
    <label>Field key</label>
    <input name="field_key" placeholder="systolic_bp" required />
    <label>Field label</label>
    <input name="field_label" placeholder="Systolic blood pressure" required />
    <label>Field type</label>
    <select name="field_type" data-field-type-select required>
      <option value="text" selected>Short text</option>
      <option value="textarea">Long text</option>
      <option value="number">Number</option>
      <option value="date">Date</option>
      <option value="datetime">Date + time</option>
      <option value="boolean">Yes / No</option>
      <option value="single_select">Single choice (one answer)</option>
      <option value="multi_select">Multiple choice (many answers)</option>
    </select>
    <label>Required</label>
    <input type="checkbox" name="required" value="true" />
    <div data-options-section>
      <label>Choice options</label>
      <div data-option-list></div>
      <button type="button" data-add-option-button>Add option</button>
      <input type="hidden" data-options-initial value="" />
      <input type="hidden" name="options_text" value="" />
      <input type="hidden" name="options_json" value="[]" />
      <small class="muted">Only needed for Single choice or Multiple choice fields.</small>
    </div>
    <label>Display order</label>
    <input name="display_order" value="0" />
    <button type="submit">Add Field</button>
  </form>
  <details style="margin-top:0.75rem;">
    <summary><strong>Import fields from HTML or PDF</strong></summary>
    <form method="post" action="{}" enctype="multipart/form-data" style="margin-top:0.6rem;">
      <label>Admin email</label>
      <input name="admin_email" value="{}" required />
      <label>Upload HTML/PDF file</label>
      <input type="file" name="html_file" accept=".html,.htm,.pdf,text/html,application/pdf" />
      <label>Or paste HTML markup</label>
      <textarea name="html_markup" placeholder="&lt;form&gt;...&lt;/form&gt;"></textarea>
      <button type="submit">Import Fields</button>
    </form>
  </details>
  <details style="margin-top:0.75rem;">
    <summary><strong style="color:#8d1f1f;">Bulk delete fields</strong></summary>
    <div class="danger-note" style="margin-top:0.6rem;">
      <strong>Warning:</strong> This permanently deletes all <strong>{}</strong> fields in the selected template and cannot be undone.
    </div>
    <form method="post" action="{}" style="margin-top:0.6rem;">
      <label>Admin email</label>
      <input name="admin_email" value="{}" required />
      <label>Type DELETE to confirm</label>
      <input name="confirmation_text" placeholder="DELETE" required />
      <button type="submit" class="danger-button">Bulk Delete All Fields</button>
    </form>
  </details>
  <details style="margin-top:0.75rem;">
    <summary><strong style="color:#a33434;">Delete only corrupted imported fields</strong></summary>
    <div class="danger-note danger-note-soft" style="margin-top:0.6rem;">
      <strong>Warning:</strong> This removes only fields matching known corruption patterns (for example: Identity-H / Unimplemented noise). Review the remaining list after this cleanup.
    </div>
    <form method="post" action="{}" style="margin-top:0.6rem;">
      <label>Admin email</label>
      <input name="admin_email" value="{}" required />
      <label>Type DELETE CORRUPTED to confirm</label>
      <input name="confirmation_text" placeholder="DELETE CORRUPTED" required />
      <button type="submit" class="danger-button danger-button-soft">Delete Corrupted Fields Only</button>
    </form>
  </details>
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

<section id="startup-checklist-panel" class="card tab-panel" data-tab-group="study-tabs" data-tab-panel="startup">
  <h2>8) Study Startup Checklist</h2>
  <p class="muted">Use this checklist to move from setup to launch. Work through pending tasks and mark them complete.</p>
  {}
  {}
  {}
  <ul>{}</ul>
  <details style="margin-top:0.9rem;">
    <summary><strong>Add or edit startup item (advanced)</strong></summary>
    <form method="post" action="{}" style="margin-top:0.6rem;">
      <label>Admin email</label>
      <input name="admin_email" value="{}" required />
      <label>Item code</label>
      <input name="item_code" placeholder="irb_approval_documented" required />
      <label>Item label</label>
      <input name="item_label" placeholder="IRB / ethics approval documented" required />
      <label>Mark as completed now</label>
      <input type="checkbox" name="completed" value="true" />
      <label>Notes</label>
      <input name="notes" placeholder="Startup note" />
      <button type="submit">Save Startup Item</button>
    </form>
  </details>
</section>

<section id="close-checklist-panel" class="card tab-panel" data-tab-group="study-tabs" data-tab-panel="close">
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
        selected_study_label,
        active_studies_html,
        pending_actions_html,
        app_dashboard_url,
        html_escape(admin_email.trim()),
        selected_org_value,
        study_rows_html,
        lifecycle_kpi_cards_html,
        lifecycle_action_cards_html,
        lifecycle_gate_html,
        phase_action,
        selected_project_value.clone(),
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
        import_html_fields_action,
        html_escape(admin_email.trim()),
        field_count,
        bulk_delete_fields_action,
        html_escape(admin_email.trim()),
        bulk_delete_corrupted_fields_action,
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
        startup_summary_html,
        startup_workflow_framework_html,
        startup_next_action_html,
        startup_checklist_html,
        startup_checklist_action,
        html_escape(admin_email.trim()),
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
    perform_study_phase_transition(&ctx, project_id, &form).await
}

async fn submit_study_phase_transition_v2(
    State(ctx): State<AppContext>,
    Form(form): Form<StudyPhaseTransitionForm>,
) -> Result<Redirect, ApiError> {
    let project_id = match form.project_id.as_deref().map(str::trim) {
        Some(value) if !value.is_empty() => match parse_uuid_field(value, "project_id") {
            Ok(project_id) => project_id,
            Err(_) => {
                return Ok(Redirect::to(&format!(
                    "/ui/studies?admin_email={}&notice={}",
                    query_escape(form.admin_email.trim()),
                    query_escape("Select a study before transitioning phase")
                )));
            }
        },
        _ => {
            return Ok(Redirect::to(&format!(
                "/ui/studies?admin_email={}&notice={}",
                query_escape(form.admin_email.trim()),
                query_escape("Select a study before transitioning phase")
            )));
        }
    };
    perform_study_phase_transition(&ctx, project_id, &form).await
}

async fn perform_study_phase_transition(
    ctx: &AppContext,
    project_id: Uuid,
    form: &StudyPhaseTransitionForm,
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
    if let Err(err) = ctx
        .db
        .transition_study_phase(
            project_id,
            form.next_phase.trim(),
            changed_by_user_id,
            form.notes.trim(),
        )
        .await
    {
        let error_text = err.to_string();
        if let Some(user_notice) = map_study_phase_transition_error_to_notice(&error_text) {
            return Ok(Redirect::to(&format!(
                "/ui/studies?admin_email={}&organization_id={}&project_id={}&tab=lifecycle&notice={}",
                query_escape(form.admin_email.trim()),
                project.organization_id,
                project_id,
                query_escape(&user_notice)
            )));
        }
        return Err(ApiError::internal(err));
    }
    Ok(Redirect::to(&format!(
        "/ui/studies?admin_email={}&organization_id={}&project_id={}&tab=lifecycle&notice={}",
        query_escape(form.admin_email.trim()),
        project.organization_id,
        project_id,
        query_escape("Study phase updated")
    )))
}

fn map_study_phase_transition_error_to_notice(error_text: &str) -> Option<String> {
    let normalized = error_text.trim().to_ascii_lowercase();
    if normalized
        .contains("cannot initiate study without at least one site and one published crf template")
    {
        return Some(
            "Cannot initiate study yet: add at least one site and publish at least one CRF template."
                .to_string(),
        );
    }
    if normalized.contains("cannot initiate study until startup checklist is fully completed") {
        return Some(
            "Cannot initiate study yet: complete all startup checklist tasks first.".to_string(),
        );
    }
    if normalized.contains("cannot set study active without at least one enrolled patient") {
        return Some(
            "Cannot set study to active yet: enroll at least one patient first.".to_string(),
        );
    }
    if normalized.contains("cannot close study while data queries remain open") {
        return Some("Cannot close study yet: resolve all open data queries first.".to_string());
    }
    if normalized.contains("cannot close study until close checklist is fully completed") {
        return Some(
            "Cannot close study yet: complete all close checklist tasks first.".to_string(),
        );
    }
    if normalized.contains("invalid phase transition") {
        return Some(
            "That phase transition is not allowed from the current study phase.".to_string(),
        );
    }
    None
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
    let options_json = normalize_crf_field_options(
        form.field_type.trim(),
        form.options_text.trim(),
        form.options_json.trim(),
    );
    let display_order = form.display_order.trim().parse::<i32>().unwrap_or(0).max(0);
    if let Err(err) = ctx
        .db
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
    {
        return Ok(Redirect::to(&format!(
            "/ui/studies?admin_email={}&organization_id={}&project_id={}&template_id={}&tab=crf-fields&notice={}",
            query_escape(form.admin_email.trim()),
            project.organization_id,
            project.id,
            template_id,
            query_escape(&map_crf_field_error_to_notice(
                "Could not add CRF field",
                &err.to_string(),
            ))
        )));
    }
    Ok(Redirect::to(&format!(
        "/ui/studies?admin_email={}&organization_id={}&project_id={}&template_id={}&tab=crf-fields&notice={}",
        query_escape(form.admin_email.trim()),
        project.organization_id,
        project.id,
        template_id,
        query_escape("CRF field added")
    )))
}

async fn submit_bulk_delete_study_crf_fields(
    State(ctx): State<AppContext>,
    Path(template_id): Path<Uuid>,
    Form(form): Form<StudyCrfBulkDeleteForm>,
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
    if form.confirmation_text.trim() != "DELETE" {
        return Ok(Redirect::to(&format!(
            "/ui/studies?admin_email={}&organization_id={}&project_id={}&template_id={}&tab=crf-fields&notice={}",
            query_escape(form.admin_email.trim()),
            project.organization_id,
            project.id,
            template.id,
            query_escape("Bulk delete cancelled: type DELETE exactly to confirm")
        )));
    }
    let deleted_count = ctx
        .db
        .delete_study_crf_fields_for_template(template_id)
        .await
        .map_err(ApiError::internal)?;
    Ok(Redirect::to(&format!(
        "/ui/studies?admin_email={}&organization_id={}&project_id={}&template_id={}&tab=crf-fields&notice={}",
        query_escape(form.admin_email.trim()),
        project.organization_id,
        project.id,
        template.id,
        query_escape(&format!(
            "Bulk delete complete: {} CRF fields removed",
            deleted_count
        ))
    )))
}

async fn submit_bulk_delete_corrupted_study_crf_fields(
    State(ctx): State<AppContext>,
    Path(template_id): Path<Uuid>,
    Form(form): Form<StudyCrfBulkDeleteForm>,
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
    if form.confirmation_text.trim() != "DELETE CORRUPTED" {
        return Ok(Redirect::to(&format!(
            "/ui/studies?admin_email={}&organization_id={}&project_id={}&template_id={}&tab=crf-fields&notice={}",
            query_escape(form.admin_email.trim()),
            project.organization_id,
            project.id,
            template.id,
            query_escape("Cleanup cancelled: type DELETE CORRUPTED exactly to confirm")
        )));
    }
    let existing_fields = ctx
        .db
        .list_study_crf_fields(template_id)
        .await
        .map_err(ApiError::internal)?;
    let corrupted_field_ids = existing_fields
        .iter()
        .filter(|field| is_corrupted_crf_field(field))
        .map(|field| field.id)
        .collect::<Vec<_>>();
    let deleted_count = ctx
        .db
        .delete_study_crf_fields_by_ids(template_id, &corrupted_field_ids)
        .await
        .map_err(ApiError::internal)?;
    Ok(Redirect::to(&format!(
        "/ui/studies?admin_email={}&organization_id={}&project_id={}&template_id={}&tab=crf-fields&notice={}",
        query_escape(form.admin_email.trim()),
        project.organization_id,
        project.id,
        template.id,
        query_escape(&format!(
            "Corrupted-field cleanup complete: {} removed",
            deleted_count
        ))
    )))
}

async fn submit_import_study_crf_fields_html(
    State(ctx): State<AppContext>,
    Path(template_id): Path<Uuid>,
    mut multipart: Multipart,
) -> Result<Redirect, ApiError> {
    let mut admin_email = String::new();
    let mut html_markup = String::new();
    let mut uploaded_file_bytes = Vec::new();
    let mut uploaded_file_name = String::new();
    let mut uploaded_content_type = String::new();
    while let Some(field) = multipart.next_field().await.map_err(ApiError::internal)? {
        let field_name = field.name().map(str::to_string).unwrap_or_default();
        match field_name.as_str() {
            "admin_email" => {
                admin_email = field.text().await.map_err(ApiError::internal)?;
            }
            "html_file" => {
                uploaded_file_name = field.file_name().unwrap_or("").to_string();
                uploaded_content_type = field
                    .content_type()
                    .map(ToString::to_string)
                    .unwrap_or_default();
                let bytes = field.bytes().await.map_err(ApiError::internal)?;
                if !bytes.is_empty() {
                    uploaded_file_bytes = bytes.to_vec();
                }
            }
            "html_markup" => {
                let text = field.text().await.map_err(ApiError::internal)?;
                if !text.trim().is_empty() {
                    html_markup = text;
                }
            }
            _ => {}
        }
    }

    if admin_email.trim().is_empty() {
        return Err(ApiError::Validation("admin_email is required".to_string()));
    }

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
        .email_has_org_manager_role(admin_email.trim(), project.organization_id)
        .await
        .map_err(ApiError::internal)?;
    if !allowed {
        return Err(ApiError::Auth(AuthError::Forbidden(
            "admin_email lacks organization manager access".to_string(),
        )));
    }

    if html_markup.trim().is_empty() && uploaded_file_bytes.is_empty() {
        return Ok(Redirect::to(&format!(
            "/ui/studies?admin_email={}&organization_id={}&project_id={}&template_id={}&tab=crf-fields&notice={}",
            query_escape(admin_email.trim()),
            project.organization_id,
            project.id,
            template.id,
            query_escape("Upload an HTML/PDF file or paste HTML before importing fields")
        )));
    }

    let file_name_lower = uploaded_file_name.to_ascii_lowercase();
    let content_type_lower = uploaded_content_type.to_ascii_lowercase();
    let is_pdf_upload = file_name_lower.ends_with(".pdf") || content_type_lower.contains("pdf");
    let (imported_fields, import_source_name, parse_notice): (
        Vec<ImportedCrfFieldDraft>,
        &str,
        Option<String>,
    ) = if !html_markup.trim().is_empty() {
        (parse_crf_fields_from_html(html_markup.trim()), "HTML", None)
    } else if is_pdf_upload {
        match parse_crf_fields_from_pdf_bytes(&uploaded_file_bytes) {
            Ok(fields) => (fields, "PDF", None),
            Err(error_notice) => (Vec::new(), "PDF", Some(error_notice)),
        }
    } else {
        let uploaded_markup = String::from_utf8_lossy(&uploaded_file_bytes).to_string();
        (
            parse_crf_fields_from_html(uploaded_markup.trim()),
            "HTML",
            None,
        )
    };
    if let Some(error_notice) = parse_notice {
        return Ok(Redirect::to(&format!(
            "/ui/studies?admin_email={}&organization_id={}&project_id={}&template_id={}&tab=crf-fields&notice={}",
            query_escape(admin_email.trim()),
            project.organization_id,
            project.id,
            template.id,
            query_escape(&error_notice)
        )));
    }
    if imported_fields.is_empty() {
        return Ok(Redirect::to(&format!(
            "/ui/studies?admin_email={}&organization_id={}&project_id={}&template_id={}&tab=crf-fields&notice={}",
            query_escape(admin_email.trim()),
            project.organization_id,
            project.id,
            template.id,
            query_escape(&format!(
                "No supported fields found in provided {}",
                import_source_name
            ))
        )));
    }

    let existing_fields = ctx
        .db
        .list_study_crf_fields(template_id)
        .await
        .map_err(ApiError::internal)?;
    let mut existing_by_key = existing_fields
        .into_iter()
        .map(|field| (field.field_key.trim().to_ascii_lowercase(), field.id))
        .collect::<HashMap<_, _>>();

    let mut added_count = 0usize;
    let mut updated_count = 0usize;
    let mut failed_count = 0usize;
    for (index, draft) in imported_fields.into_iter().enumerate() {
        let display_order = index as i32;
        let result = if let Some(field_id) = existing_by_key.get(&draft.field_key).copied() {
            ctx.db
                .update_study_crf_field(
                    field_id,
                    &draft.field_key,
                    &draft.field_label,
                    &draft.field_type,
                    draft.required,
                    &draft.options_json,
                    display_order,
                )
                .await
                .map(|_| {
                    updated_count += 1;
                })
        } else {
            ctx.db
                .add_study_crf_field(
                    template_id,
                    &draft.field_key,
                    &draft.field_label,
                    &draft.field_type,
                    draft.required,
                    &draft.options_json,
                    display_order,
                )
                .await
                .map(|field| {
                    existing_by_key.insert(draft.field_key.clone(), field.id);
                    added_count += 1;
                })
        };
        if result.is_err() {
            failed_count += 1;
        }
    }

    let notice = if failed_count == 0 {
        format!(
            "{} import complete: {} added, {} updated",
            import_source_name, added_count, updated_count
        )
    } else {
        format!(
            "{} import finished with issues: {} added, {} updated, {} skipped",
            import_source_name, added_count, updated_count, failed_count
        )
    };

    Ok(Redirect::to(&format!(
        "/ui/studies?admin_email={}&organization_id={}&project_id={}&template_id={}&tab=crf-fields&notice={}",
        query_escape(admin_email.trim()),
        project.organization_id,
        project.id,
        template.id,
        query_escape(&notice)
    )))
}

async fn submit_update_study_crf_field(
    State(ctx): State<AppContext>,
    Path(field_id): Path<Uuid>,
    Form(form): Form<StudyCrfFieldForm>,
) -> Result<Redirect, ApiError> {
    let existing_field = ctx
        .db
        .get_study_crf_field(field_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("CRF field not found".to_string()))?;
    let template = ctx
        .db
        .get_study_crf_template(existing_field.template_id)
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
    let options_json = normalize_crf_field_options(
        form.field_type.trim(),
        form.options_text.trim(),
        form.options_json.trim(),
    );
    let display_order = form.display_order.trim().parse::<i32>().unwrap_or(0).max(0);
    if let Err(err) = ctx
        .db
        .update_study_crf_field(
            field_id,
            form.field_key.trim(),
            form.field_label.trim(),
            form.field_type.trim(),
            form.required.is_some(),
            &options_json,
            display_order,
        )
        .await
    {
        return Ok(Redirect::to(&format!(
            "/ui/studies?admin_email={}&organization_id={}&project_id={}&template_id={}&tab=crf-fields&notice={}",
            query_escape(form.admin_email.trim()),
            project.organization_id,
            project.id,
            template.id,
            query_escape(&map_crf_field_error_to_notice(
                "Could not update CRF field",
                &err.to_string(),
            ))
        )));
    }
    Ok(Redirect::to(&format!(
        "/ui/studies?admin_email={}&organization_id={}&project_id={}&template_id={}&tab=crf-fields&notice={}",
        query_escape(form.admin_email.trim()),
        project.organization_id,
        project.id,
        template.id,
        query_escape("CRF field updated")
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
    let notice = if completed {
        "Startup task completed"
    } else {
        "Startup task marked pending"
    };
    Ok(Redirect::to(&format!(
        "/ui/studies?admin_email={}&organization_id={}&project_id={}&tab=startup&notice={}#startup-next-task",
        query_escape(form.admin_email.trim()),
        project.organization_id,
        project_id,
        query_escape(notice)
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
    let notice = if completed {
        "Close checklist task completed"
    } else {
        "Close checklist task marked pending"
    };
    Ok(Redirect::to(&format!(
        "/ui/studies?admin_email={}&organization_id={}&project_id={}&tab=close&notice={}#close-checklist-panel",
        query_escape(form.admin_email.trim()),
        project.organization_id,
        project_id,
        query_escape(notice)
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

#[derive(Debug, Deserialize)]
struct DuaAgreementPageQuery {
    admin_email: Option<String>,
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
    parent_organization_id: String,
    organization_kind: String,
}

#[derive(Debug, Deserialize)]
struct DuaOpenAgreementForm {
    admin_email: String,
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
        .or_else(|| {
            organizations
                .iter()
                .find(|org| org.organization_kind == "platform_root")
                .map(|org| org.id.to_string())
                .or_else(|| organizations.first().map(|o| o.id.to_string()))
        })
        .unwrap_or_default();
    let selected_organization_uuid = selected_organization_id.parse::<Uuid>().ok();

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
                    "<li><strong>{}</strong> — {} <small>(kind: {} · parent: {})</small></li>",
                    html_escape(&org.name),
                    org.id,
                    html_escape(&org.organization_kind),
                    org.parent_organization_id
                        .map(|id| id.to_string())
                        .unwrap_or_else(|| "none".to_string())
                )
            })
            .collect::<Vec<_>>()
            .join("")
    };
    let org_duas = if let Some(org_id) = selected_organization_uuid {
        ctx.db
            .list_data_use_agreements(org_id)
            .await
            .map_err(ApiError::internal)?
    } else {
        Vec::new()
    };
    let org_duas_html = if org_duas.is_empty() {
        "<li>No DUAs for selected organization yet.</li>".to_string()
    } else {
        org_duas
            .iter()
            .map(|dua| {
                format!(
                    r#"<li><a href="/ui/dua/{}?admin_email={}">{}</a> <span class="status-chip">{}</span></li>"#,
                    dua.id,
                    query_escape(admin_email.trim()),
                    html_escape(&dua.hospital_name),
                    html_escape(&dua.status)
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
      <label>Parent organization ID (optional; blank = Cingulum Foundation root)</label>
      <input name="parent_organization_id" list="organization-options" value="{}" placeholder="child under foundation" />
      <label>Organization type</label>
      <select name="organization_kind">
        <option value="tenant" selected>tenant</option>
        <option value="research_network">research_network</option>
        <option value="hospital">hospital</option>
        <option value="sponsor">sponsor</option>
      </select>

      <button type="submit">Create Organization</button>
    </form>
    <h3 style="margin-top:1rem;">Organizations available for this admin</h3>
    <ul>{}</ul>
  </section>

  <section class="card">
    <h2>Return to existing agreement workspace</h2>
    <form method="post" action="/ui/dua/open-agreement">
      <label>Admin email</label>
      <input name="admin_email" value="{}" required />
      <label>Agreement ID (UUID)</label>
      <input name="agreement_id" placeholder="agreement-uuid" required />
      <button type="submit">Open Agreement Workspace</button>
    </form>
  </section>

  <section class="card">
    <h2>Selected organization DUA workspace</h2>
    <p class="muted">DUAs below are scoped only to the chosen organization.</p>
    <ul>{}</ul>
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
        html_escape(&selected_organization_id),
        managed_orgs_html,
        html_escape(&admin_email),
        org_duas_html,
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
    if form.admin_email.trim().is_empty() {
        return Err(ApiError::Validation(
            "admin_email is required to open agreement workspace".to_string(),
        ));
    }
    let agreement_id = form
        .agreement_id
        .trim()
        .parse::<Uuid>()
        .map_err(|_| ApiError::Validation("agreement_id must be a valid UUID".to_string()))?;
    Ok(Redirect::to(&format!(
        "/ui/dua/{}?admin_email={}",
        agreement_id,
        query_escape(form.admin_email.trim())
    )))
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
  <p><a href="/ui/dua/{}?admin_email={}">Open agreement workspace</a></p>
  <p><a href="/ui/dua?admin_email={}">Create another agreement</a></p>
</section>
"#,
        agreement.id,
        html_escape(&agreement.status),
        html_escape(&signing_url),
        html_escape(&signing_url),
        agreement.id,
        query_escape(form.admin_email.trim()),
        query_escape(form.admin_email.trim())
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
  <p><a href="/ui/dua">Cingulum admin workspace</a></p>
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
  <p><a href="/ui/dua">Back to DUA home</a></p>
</section>
"#,
        agreement.id,
        html_escape(&agreement.status)
    );
    Ok(Html(render_cingulum_page("Signature Submitted", body)))
}

async fn render_dua_agreement_page(
    State(ctx): State<AppContext>,
    Path(agreement_id): Path<Uuid>,
    Query(query): Query<DuaAgreementPageQuery>,
) -> Result<Html<String>, ApiError> {
    let admin_email = query
        .admin_email
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .ok_or_else(|| {
            ApiError::Validation("admin_email query parameter is required for DUA workspace".into())
        })?;
    let agreement = ctx
        .db
        .get_data_use_agreement(agreement_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("data use agreement not found".to_string()))?;
    let allowed = ctx
        .db
        .email_has_org_manager_role(&admin_email, agreement.organization_id)
        .await
        .map_err(ApiError::internal)?;
    if !allowed {
        return Err(ApiError::Auth(AuthError::Forbidden(
            "admin_email lacks permission for this organization's DUA workspace".to_string(),
        )));
    }
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
  <p><strong>Organization ID:</strong> {}</p>
  <p><strong>Agreement ID:</strong> {}</p>
  <p><strong>Hospital:</strong> {}</p>
  <p><strong>Status:</strong> <span class="status-chip">{}</span></p>
  <p><strong>Hospital signing link:</strong> <a href="{}">{}</a></p>
</section>

<section class="card">
  <h2>Actions</h2>
  <form method="post" action="/ui/dua/{}/send-hospital-link" style="margin-bottom:1rem;">
    <label>Admin email for send action</label>
    <input name="admin_email" value="{}" required style="max-width:480px;" />
    <button type="submit">Queue Hospital Signing Email</button>
  </form>

  <form method="post" action="/ui/dua/{}/sign-cingulum" style="margin-bottom:1rem;">
    <input type="hidden" name="admin_email" value="{}" />
    <label>Cingulum signer name</label><input name="signer_name" required style="max-width:480px;" />
    <label>Cingulum signer email</label><input name="signer_email" required style="max-width:480px;" />
    <label>Cingulum signer title</label><input name="signer_title" required style="max-width:480px;" />
    <label>Signature text</label><input name="signature_text" placeholder="/s/ Name" required style="max-width:480px;" />
    <button type="submit">Apply Cingulum Signature</button>
  </form>

  <p><a href="/ui/dua/{}/export.pdf?admin_email={}">Download PDF</a></p>
  <p><a href="/ui/dua?admin_email={}">Back to DUA home</a></p>
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
        agreement.organization_id,
        agreement.id,
        html_escape(&agreement.hospital_name),
        html_escape(&agreement.status),
        signing_link,
        signing_link,
        agreement.id,
        html_escape(&admin_email),
        agreement.id,
        html_escape(&admin_email),
        agreement.id,
        query_escape(&admin_email),
        query_escape(&admin_email),
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
  <p><a href="/ui/dua/{}?admin_email={}">Back to agreement</a></p>
</section>
"#,
        agreement_id,
        agreement_id,
        query_escape(form.admin_email.trim())
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
  <p><a href="/ui/dua/{}?admin_email={}">Back to agreement</a></p>
</section>
"#,
        agreement_id,
        query_escape(form.admin_email.trim())
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

fn is_select_field_type(field_type: &str) -> bool {
    matches!(
        field_type.trim().to_ascii_lowercase().as_str(),
        "single_select" | "multi_select"
    )
}

fn normalize_crf_field_options(field_type: &str, options_text: &str, options_json: &str) -> String {
    if !is_select_field_type(field_type) {
        return "[]".to_string();
    }
    let options_text_trimmed = options_text.trim();
    if !options_text_trimmed.is_empty() {
        let values = options_text_trimmed
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>();
        return serde_json::to_string(&values).unwrap_or_else(|_| "[]".to_string());
    }
    normalize_options_json_input(options_json)
}

fn options_json_to_lines(options_json: &str) -> String {
    let trimmed = options_json.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        if let Some(array) = value.as_array() {
            return array
                .iter()
                .map(|item| {
                    item.as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| item.to_string())
                })
                .collect::<Vec<_>>()
                .join("\n");
        }
    }
    trimmed.to_string()
}

#[derive(Debug, Clone)]
struct ImportedCrfFieldDraft {
    field_key: String,
    field_label: String,
    field_type: String,
    required: bool,
    options_json: String,
}

fn parse_crf_fields_from_html(html_markup: &str) -> Vec<ImportedCrfFieldDraft> {
    let document = ParsedHtml::parse_document(html_markup);
    let input_selector = Selector::parse("input").expect("valid input selector");
    let textarea_selector = Selector::parse("textarea").expect("valid textarea selector");
    let select_selector = Selector::parse("select").expect("valid select selector");
    let option_selector = Selector::parse("option").expect("valid option selector");

    let mut fields = Vec::new();
    let mut seen_keys = HashSet::new();
    let mut grouped_choices: HashMap<String, (String, String, bool, Vec<String>)> = HashMap::new();
    let mut fallback_counter = 1usize;

    for input in document.select(&input_selector) {
        let raw_key = input
            .value()
            .attr("name")
            .or_else(|| input.value().attr("id"))
            .unwrap_or("")
            .trim();
        let mut field_key = normalize_html_field_key(raw_key);
        if field_key.is_empty() {
            field_key = format!("field_{fallback_counter}");
            fallback_counter += 1;
        }

        let input_type = input
            .value()
            .attr("type")
            .unwrap_or("text")
            .trim()
            .to_ascii_lowercase();
        if matches!(
            input_type.as_str(),
            "hidden" | "submit" | "button" | "reset" | "image" | "file"
        ) {
            continue;
        }

        if input_type == "radio" || input_type == "checkbox" {
            let group = grouped_choices.entry(field_key.clone()).or_insert_with(|| {
                (
                    infer_html_field_label(raw_key, input.value().attr("placeholder")),
                    if input_type == "radio" {
                        "single_select".to_string()
                    } else {
                        "multi_select".to_string()
                    },
                    false,
                    Vec::new(),
                )
            });
            group.2 = group.2 || input.value().attr("required").is_some();
            let choice_value = input
                .value()
                .attr("value")
                .map(normalize_whitespace)
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "Option".to_string());
            if !group
                .3
                .iter()
                .any(|existing| existing.eq_ignore_ascii_case(choice_value.as_str()))
            {
                group.3.push(choice_value);
            }
            continue;
        }

        if !seen_keys.insert(field_key.clone()) {
            continue;
        }
        let field_type = match input_type.as_str() {
            "number" | "range" => "number",
            "date" => "date",
            "datetime-local" => "datetime",
            "checkbox" => "boolean",
            _ => "text",
        }
        .to_string();

        fields.push(ImportedCrfFieldDraft {
            field_label: infer_html_field_label(raw_key, input.value().attr("placeholder")),
            field_key,
            field_type,
            required: input.value().attr("required").is_some(),
            options_json: "[]".to_string(),
        });
    }

    for textarea in document.select(&textarea_selector) {
        let raw_key = textarea
            .value()
            .attr("name")
            .or_else(|| textarea.value().attr("id"))
            .unwrap_or("")
            .trim();
        let mut field_key = normalize_html_field_key(raw_key);
        if field_key.is_empty() {
            field_key = format!("field_{fallback_counter}");
            fallback_counter += 1;
        }
        if !seen_keys.insert(field_key.clone()) {
            continue;
        }
        fields.push(ImportedCrfFieldDraft {
            field_label: infer_html_field_label(raw_key, textarea.value().attr("placeholder")),
            field_key,
            field_type: "textarea".to_string(),
            required: textarea.value().attr("required").is_some(),
            options_json: "[]".to_string(),
        });
    }

    for select in document.select(&select_selector) {
        let raw_key = select
            .value()
            .attr("name")
            .or_else(|| select.value().attr("id"))
            .unwrap_or("")
            .trim();
        let mut field_key = normalize_html_field_key(raw_key);
        if field_key.is_empty() {
            field_key = format!("field_{fallback_counter}");
            fallback_counter += 1;
        }
        if !seen_keys.insert(field_key.clone()) {
            continue;
        }
        let options = select
            .select(&option_selector)
            .filter_map(|option| {
                let value = option
                    .value()
                    .attr("value")
                    .map(normalize_whitespace)
                    .filter(|value| !value.is_empty())
                    .or_else(|| {
                        let text = normalize_whitespace(&option.text().collect::<String>());
                        if text.is_empty() {
                            None
                        } else {
                            Some(text)
                        }
                    })?;
                Some(value)
            })
            .collect::<Vec<_>>();
        let options_json = serde_json::to_string(&options).unwrap_or_else(|_| "[]".to_string());
        fields.push(ImportedCrfFieldDraft {
            field_label: infer_html_field_label(raw_key, None),
            field_key,
            field_type: if select.value().attr("multiple").is_some() {
                "multi_select".to_string()
            } else {
                "single_select".to_string()
            },
            required: select.value().attr("required").is_some(),
            options_json,
        });
    }

    for (field_key, (field_label, field_type, required, options)) in grouped_choices {
        if !seen_keys.insert(field_key.clone()) {
            continue;
        }
        let normalized_options = options
            .into_iter()
            .filter(|option| !option.trim().is_empty())
            .collect::<Vec<_>>();
        let field_type = if field_type == "multi_select" && normalized_options.len() <= 1 {
            "boolean".to_string()
        } else {
            field_type
        };
        let options_json = if field_type == "boolean" {
            "[]".to_string()
        } else {
            serde_json::to_string(&normalized_options).unwrap_or_else(|_| "[]".to_string())
        };
        fields.push(ImportedCrfFieldDraft {
            field_key,
            field_label,
            field_type,
            required,
            options_json,
        });
    }

    fields
}

fn parse_crf_fields_from_pdf_bytes(pdf_bytes: &[u8]) -> Result<Vec<ImportedCrfFieldDraft>, String> {
    let mut drafts = Vec::new();
    let mut extracted_text_sources = Vec::new();

    if let Ok(text) = pdf_extract::extract_text_from_mem(pdf_bytes) {
        if !text.trim().is_empty() {
            extracted_text_sources.push(text);
        }
    }

    if let Ok(document) = LopdfDocument::load_mem(pdf_bytes) {
        if let Ok(text) = extract_pdf_text_with_lopdf(&document) {
            if !text.trim().is_empty() {
                extracted_text_sources.push(text);
            }
        }
        drafts.extend(extract_pdf_acroform_field_drafts(&document));
    }

    drafts.extend(extract_pdf_raw_name_hints(pdf_bytes));
    for extracted_text in extracted_text_sources {
        drafts.extend(parse_crf_fields_from_pdf_text(&extracted_text));
    }
    let merged = merge_imported_pdf_field_drafts(drafts);
    if merged.is_empty() {
        return Err(
            "Could not detect fields from this PDF. Try a fillable PDF, use OCR first, or import HTML."
                .to_string(),
        );
    }
    Ok(merged)
}

fn extract_pdf_text_with_lopdf(document: &LopdfDocument) -> Result<String, String> {
    let page_numbers = document.get_pages().keys().copied().collect::<Vec<_>>();
    if page_numbers.is_empty() {
        return Err("No pages found in PDF".to_string());
    }
    document
        .extract_text(&page_numbers)
        .map_err(|_| "Could not extract text with lopdf parser".to_string())
}

fn extract_pdf_acroform_field_drafts(document: &LopdfDocument) -> Vec<ImportedCrfFieldDraft> {
    let mut drafts = Vec::new();
    let mut seen_keys = HashSet::new();
    let mut fallback_counter = 1usize;
    let Ok(catalog) = document.catalog() else {
        return drafts;
    };
    let Ok(acroform_object) = catalog.get(b"AcroForm") else {
        return drafts;
    };
    let Some(acroform_object) = resolve_lopdf_object(document, acroform_object) else {
        return drafts;
    };
    let Ok(acroform_dict) = acroform_object.as_dict() else {
        return drafts;
    };
    let Ok(field_objects) = acroform_dict.get(b"Fields").and_then(LopdfObject::as_array) else {
        return drafts;
    };
    for field_object in field_objects {
        collect_acroform_field_drafts(
            document,
            field_object,
            None,
            None,
            0,
            None,
            &mut drafts,
            &mut seen_keys,
            &mut fallback_counter,
        );
    }
    drafts
}

#[allow(clippy::too_many_arguments)]
fn collect_acroform_field_drafts(
    document: &LopdfDocument,
    field_object: &LopdfObject,
    parent_name: Option<String>,
    inherited_ft: Option<String>,
    inherited_flags: i64,
    inherited_options: Option<Vec<String>>,
    drafts: &mut Vec<ImportedCrfFieldDraft>,
    seen_keys: &mut HashSet<String>,
    fallback_counter: &mut usize,
) {
    let Some(resolved_object) = resolve_lopdf_object(document, field_object) else {
        return;
    };
    let Ok(field_dict) = resolved_object.as_dict() else {
        return;
    };

    let current_name = field_dict
        .get(b"T")
        .ok()
        .and_then(|value| decode_lopdf_text_object(document, value));
    let combined_name = combine_pdf_field_names(parent_name, current_name);
    let field_type_name = field_dict
        .get(b"FT")
        .ok()
        .and_then(|value| decode_lopdf_name_object(document, value))
        .or(inherited_ft);
    let field_flags = field_dict
        .get(b"Ff")
        .ok()
        .and_then(|value| decode_lopdf_integer(document, value))
        .unwrap_or(inherited_flags);
    let field_options = field_dict
        .get(b"Opt")
        .ok()
        .map(|value| decode_lopdf_option_values(document, value))
        .filter(|options| !options.is_empty())
        .or(inherited_options.clone());
    let alternate_label = field_dict
        .get(b"TU")
        .ok()
        .and_then(|value| decode_lopdf_text_object(document, value))
        .filter(|label| !label.trim().is_empty());

    let kids = field_dict
        .get(b"Kids")
        .ok()
        .and_then(|value| value.as_array().ok())
        .map(|kids| kids.clone());
    if let Some(kids) = kids {
        for kid in kids {
            collect_acroform_field_drafts(
                document,
                &kid,
                combined_name.clone(),
                field_type_name.clone(),
                field_flags,
                field_options.clone(),
                drafts,
                seen_keys,
                fallback_counter,
            );
        }
    }

    let Some(full_name) = combined_name else {
        return;
    };
    let lower_name = full_name.to_ascii_lowercase();
    if lower_name.ends_with(".widget")
        || lower_name.contains("signature")
        || lower_name.contains("btn")
        || lower_name.len() < 2
    {
        return;
    }
    let (field_type, mut options_json) = map_pdf_form_field_type(
        field_type_name.as_deref(),
        field_flags,
        field_options.as_ref(),
    );
    if field_type != "single_select" && field_type != "multi_select" {
        options_json = "[]".to_string();
    }
    let label = alternate_label.unwrap_or_else(|| humanize_pdf_field_label(&full_name));
    let base_key = normalize_html_field_key(&full_name);
    let field_key = unique_pdf_field_key(base_key, seen_keys, fallback_counter);
    let required = field_flags & 0b10 != 0;
    drafts.push(ImportedCrfFieldDraft {
        field_key,
        field_label: if label.trim().is_empty() {
            "Imported field".to_string()
        } else {
            label
        },
        field_type,
        required,
        options_json,
    });
}

fn resolve_lopdf_object<'a>(
    document: &'a LopdfDocument,
    object: &'a LopdfObject,
) -> Option<&'a LopdfObject> {
    let mut current = object;
    while let Ok(reference_id) = current.as_reference() {
        current = document.get_object(reference_id).ok()?;
    }
    Some(current)
}

fn decode_lopdf_text_object(document: &LopdfDocument, object: &LopdfObject) -> Option<String> {
    let resolved = resolve_lopdf_object(document, object)?;
    if let Ok(bytes) = resolved.as_str() {
        let decoded = decode_lopdf_bytes_best_effort(bytes);
        if is_plausible_import_label(&decoded) {
            return Some(decoded);
        }
    }
    if let Ok(name) = resolved.as_name_str() {
        let decoded = normalize_whitespace(name);
        if is_plausible_import_label(&decoded) {
            return Some(decoded);
        }
    }
    None
}

fn decode_lopdf_name_object(document: &LopdfDocument, object: &LopdfObject) -> Option<String> {
    let resolved = resolve_lopdf_object(document, object)?;
    resolved.as_name_str().ok().map(str::to_string)
}

fn decode_lopdf_integer(document: &LopdfDocument, object: &LopdfObject) -> Option<i64> {
    let resolved = resolve_lopdf_object(document, object)?;
    resolved.as_i64().ok()
}

fn decode_lopdf_option_values(document: &LopdfDocument, object: &LopdfObject) -> Vec<String> {
    let Some(resolved) = resolve_lopdf_object(document, object) else {
        return Vec::new();
    };
    let Ok(option_array) = resolved.as_array() else {
        return Vec::new();
    };
    let mut options = Vec::new();
    for option_object in option_array {
        let Some(resolved_option) = resolve_lopdf_object(document, option_object) else {
            continue;
        };
        if let Ok(text) = resolved_option.as_str() {
            let value = decode_lopdf_bytes_best_effort(text);
            if is_plausible_import_label(&value) {
                options.push(value);
            }
            continue;
        }
        if let Ok(values) = resolved_option.as_array() {
            if let Some(preferred) = values
                .get(1)
                .and_then(|value| decode_lopdf_text_object(document, value))
                .or_else(|| {
                    values
                        .first()
                        .and_then(|value| decode_lopdf_text_object(document, value))
                })
            {
                options.push(preferred);
            }
        }
    }
    dedupe_case_insensitive(options)
}

fn combine_pdf_field_names(parent: Option<String>, current: Option<String>) -> Option<String> {
    match (parent, current) {
        (Some(parent), Some(current)) if !current.is_empty() => Some(format!("{parent}.{current}")),
        (Some(parent), Some(_)) => Some(parent),
        (Some(parent), None) => Some(parent),
        (None, Some(current)) => Some(current),
        (None, None) => None,
    }
}

fn map_pdf_form_field_type(
    field_type_name: Option<&str>,
    field_flags: i64,
    options: Option<&Vec<String>>,
) -> (String, String) {
    let options_json =
        serde_json::to_string(options.unwrap_or(&Vec::new())).unwrap_or_else(|_| "[]".to_string());
    let Some(field_type_name) = field_type_name else {
        if options.map_or(0, |values| values.len()) > 1 {
            return ("single_select".to_string(), options_json);
        }
        return ("text".to_string(), "[]".to_string());
    };
    match field_type_name {
        "Btn" => {
            let radio = field_flags & 0x8000 != 0;
            let push_button = field_flags & 0x10000 != 0;
            if push_button {
                ("text".to_string(), "[]".to_string())
            } else if radio {
                ("single_select".to_string(), options_json)
            } else {
                ("boolean".to_string(), "[]".to_string())
            }
        }
        "Ch" => {
            let multi_select = field_flags & 0x200000 != 0;
            if multi_select {
                ("multi_select".to_string(), options_json)
            } else {
                ("single_select".to_string(), options_json)
            }
        }
        "Tx" => {
            let multiline = field_flags & 0x1000 != 0;
            if multiline {
                ("textarea".to_string(), "[]".to_string())
            } else {
                ("text".to_string(), "[]".to_string())
            }
        }
        "Sig" => ("text".to_string(), "[]".to_string()),
        _ => ("text".to_string(), "[]".to_string()),
    }
}

fn humanize_pdf_field_label(input: &str) -> String {
    let normalized = normalize_whitespace(
        &input
            .replace(['.', '_', '-'], " ")
            .replace("  ", " ")
            .trim_matches('.')
            .to_string(),
    );
    normalized
        .split(' ')
        .filter(|token| !token.is_empty())
        .map(title_case_token)
        .collect::<Vec<_>>()
        .join(" ")
}

fn extract_pdf_raw_name_hints(pdf_bytes: &[u8]) -> Vec<ImportedCrfFieldDraft> {
    let raw = String::from_utf8_lossy(pdf_bytes);
    let mut labels = extract_parenthesized_values_after_prefix(&raw, "/TU(");
    labels.extend(extract_parenthesized_values_after_prefix(&raw, "/T("));
    labels = dedupe_case_insensitive(labels);
    labels
        .into_iter()
        .filter(|label| looks_like_raw_pdf_label(label))
        .take(800)
        .filter_map(|label| {
            let sanitized_label = sanitize_pdf_field_label(&label);
            if !is_plausible_import_label(&sanitized_label) {
                return None;
            }
            let (field_type, options_json) = infer_pdf_field_type_and_options(&label);
            let display_label = strip_inline_option_hints(&sanitized_label);
            Some(ImportedCrfFieldDraft {
                field_key: normalize_html_field_key(&display_label),
                field_label: if display_label.is_empty() {
                    "Imported field".to_string()
                } else {
                    display_label
                },
                field_type,
                required: false,
                options_json,
            })
        })
        .collect::<Vec<_>>()
}

fn extract_parenthesized_values_after_prefix(content: &str, prefix: &str) -> Vec<String> {
    let mut results = Vec::new();
    let bytes = content.as_bytes();
    let prefix_bytes = prefix.as_bytes();
    let mut index = 0usize;
    while index + prefix_bytes.len() < bytes.len() {
        if &bytes[index..index + prefix_bytes.len()] != prefix_bytes {
            index += 1;
            continue;
        }
        let mut cursor = index + prefix_bytes.len();
        let mut value = String::new();
        let mut escaped = false;
        while cursor < bytes.len() {
            let ch = bytes[cursor] as char;
            cursor += 1;
            if escaped {
                value.push(ch);
                escaped = false;
                continue;
            }
            if ch == '\\' {
                escaped = true;
                continue;
            }
            if ch == ')' {
                break;
            }
            value.push(ch);
        }
        let normalized = normalize_whitespace(&value);
        if !normalized.is_empty() {
            results.push(normalized);
        }
        index = cursor;
    }
    results
}

fn looks_like_raw_pdf_label(label: &str) -> bool {
    let trimmed = label.trim();
    if trimmed.len() < 3 || trimmed.len() > 120 {
        return false;
    }
    if contains_pdf_encoding_noise(trimmed) {
        return false;
    }
    if looks_like_pdf_page_noise(trimmed) {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("font")
        || lower.starts_with("type")
        || lower.starts_with("size")
        || lower.contains("obj")
        || lower.contains("stream")
        || lower.contains("xref")
        || lower.contains("root")
        || lower.contains("producer")
    {
        return false;
    }
    trimmed.chars().any(|ch| ch.is_ascii_alphabetic())
}

fn merge_imported_pdf_field_drafts(
    drafts: Vec<ImportedCrfFieldDraft>,
) -> Vec<ImportedCrfFieldDraft> {
    let mut merged: Vec<ImportedCrfFieldDraft> = Vec::new();
    let mut key_index: HashMap<String, usize> = HashMap::new();
    let mut fallback_counter = 1usize;
    for mut draft in drafts {
        draft.field_label = sanitize_pdf_field_label(&draft.field_label);
        if !is_plausible_import_label(&draft.field_label) {
            continue;
        }
        draft.field_key = normalize_html_field_key(&draft.field_key);
        if draft.field_key.is_empty() {
            draft.field_key = normalize_html_field_key(&draft.field_label);
        }
        if draft.field_key.is_empty() {
            draft.field_key = format!("pdf_field_{fallback_counter}");
            fallback_counter += 1;
        }
        let normalized_key = draft.field_key.to_ascii_lowercase();
        if let Some(existing_index) = key_index.get(&normalized_key).copied() {
            let existing = &mut merged[existing_index];
            if existing.field_label == "Imported field" && draft.field_label != "Imported field" {
                existing.field_label = draft.field_label;
            }
            if existing.field_type == "text" && draft.field_type != "text" {
                existing.field_type = draft.field_type;
            }
            existing.required = existing.required || draft.required;
            if existing.options_json == "[]" && draft.options_json != "[]" {
                existing.options_json = draft.options_json;
            }
            continue;
        }
        key_index.insert(normalized_key, merged.len());
        merged.push(draft);
    }
    merged
}

fn is_corrupted_crf_field(field: &StudyCrfField) -> bool {
    let label = field.field_label.trim();
    let key = field.field_key.trim();
    let label_lower = label.to_ascii_lowercase();
    let key_lower = key.to_ascii_lowercase();
    if contains_pdf_encoding_noise(label) || contains_pdf_encoding_noise(key) {
        return true;
    }
    if (label_lower.contains("identity-h")
        || label_lower.contains("identity-v")
        || key_lower.contains("identity_h")
        || key_lower.contains("identity_v"))
        && (label_lower.contains("unimplemented")
            || key_lower.contains("unimplemented")
            || label_lower.contains("identity")
            || key_lower.contains("identity"))
    {
        return true;
    }
    if label_lower.matches("identity-h").count() >= 2
        || label_lower.matches("unimplemented").count() >= 2
    {
        return true;
    }
    label.len() > 220 && has_high_token_repetition(label)
}

fn has_high_token_repetition(text: &str) -> bool {
    let tokens = text
        .split_whitespace()
        .map(|token| token.to_ascii_lowercase())
        .collect::<Vec<_>>();
    if tokens.len() < 8 {
        return false;
    }
    let mut counts = HashMap::new();
    for token in &tokens {
        *counts.entry(token.clone()).or_insert(0usize) += 1;
    }
    let highest_count = counts.values().copied().max().unwrap_or(0);
    highest_count.saturating_mul(100) / tokens.len() >= 45
}

fn dedupe_case_insensitive(values: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut deduped = Vec::new();
    for value in values {
        let normalized = value.to_ascii_lowercase();
        if seen.insert(normalized) {
            deduped.push(value);
        }
    }
    deduped
}

fn parse_crf_fields_from_pdf_text(pdf_text: &str) -> Vec<ImportedCrfFieldDraft> {
    let mut fields = Vec::new();
    let mut seen_keys = HashSet::new();
    let mut fallback_counter = 1usize;

    let normalized_lines = pdf_text
        .lines()
        .map(normalize_whitespace)
        .filter(|line| !line.is_empty() && !contains_pdf_encoding_noise(line))
        .collect::<Vec<_>>();
    let merged_lines = merge_pdf_wrapped_lines(&normalized_lines);
    let mut candidate_lines = merged_lines
        .iter()
        .filter(|line| looks_like_pdf_field_line(line))
        .cloned()
        .collect::<Vec<_>>();

    // Large documents often lose punctuation/checkbox markers during extraction.
    // If strict matching yields very few fields, run a broader pass.
    if candidate_lines.len() < 18 {
        candidate_lines.extend(
            merged_lines
                .iter()
                .filter(|line| looks_like_pdf_field_line_relaxed(line))
                .cloned(),
        );
    }
    candidate_lines = dedupe_preserving_order(candidate_lines);

    for normalized_line in candidate_lines {
        let cleaned_label = sanitize_pdf_field_label(&normalized_line);
        if !is_plausible_import_label(&cleaned_label) {
            continue;
        }
        let (field_type, options_json) = infer_pdf_field_type_and_options(&cleaned_label);
        let display_label = strip_inline_option_hints(&cleaned_label);
        let base_key = normalize_html_field_key(&display_label);
        let field_key = unique_pdf_field_key(base_key, &mut seen_keys, &mut fallback_counter);
        let lower_line = normalized_line.to_ascii_lowercase();
        let required = lower_line.contains(" required")
            || lower_line.ends_with("required")
            || cleaned_label.contains('*');
        fields.push(ImportedCrfFieldDraft {
            field_key,
            field_label: if display_label.is_empty() {
                "Imported field".to_string()
            } else {
                display_label
            },
            field_type,
            required,
            options_json,
        });
    }

    fields
}

fn looks_like_pdf_field_line(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.len() < 3 {
        return false;
    }
    if contains_pdf_encoding_noise(trimmed) {
        return false;
    }
    if trimmed.ends_with('?') || trimmed.ends_with(':') || trimmed.contains("_____") {
        return true;
    }
    let lower = trimmed.to_ascii_lowercase();
    lower.contains("select one")
        || lower.contains("choose one")
        || lower.contains("select all")
        || lower.contains("yes/no")
        || lower.contains("yes or no")
        || lower.contains("tick one")
        || lower.contains("check all")
        || lower.contains("enter")
        || lower.contains("(required)")
}

fn looks_like_pdf_field_line_relaxed(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.len() < 4 || trimmed.len() > 140 {
        return false;
    }
    if contains_pdf_encoding_noise(trimmed) {
        return false;
    }
    if looks_like_pdf_page_noise(trimmed) {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.contains("http://")
        || lower.contains("https://")
        || lower.contains("@")
        || lower.starts_with("page ")
    {
        return false;
    }
    let word_count = trimmed.split_whitespace().count();
    if word_count == 0 || word_count > 18 {
        return false;
    }
    if trimmed.ends_with('.')
        && !lower.contains("other (")
        && !lower.contains("select")
        && !lower.contains("choose")
    {
        return false;
    }
    if lower.contains("name")
        || lower.contains("id")
        || lower.contains("date")
        || lower.contains("time")
        || lower.contains("age")
        || lower.contains("sex")
        || lower.contains("gender")
        || lower.contains("diagnosis")
        || lower.contains("symptom")
        || lower.contains("status")
        || lower.contains("result")
        || lower.contains("dose")
        || lower.contains("medication")
        || lower.contains("comment")
        || lower.contains("notes")
        || lower.contains("reason")
        || lower.contains("history")
        || lower.contains("visit")
        || lower.contains("consent")
        || lower.contains("severity")
        || lower.contains("site")
        || lower.contains("investigator")
    {
        return true;
    }
    (trimmed.ends_with('?') || trimmed.ends_with(':') || trimmed.contains("____"))
        || starts_with_bullet_or_numbering(trimmed)
}

fn looks_like_pdf_page_noise(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    if lower.starts_with("table of contents")
        || lower.starts_with("confidential")
        || lower.starts_with("copyright")
        || lower.starts_with("appendix")
    {
        return true;
    }
    let compact = lower.replace(' ', "");
    compact.starts_with("page") && compact[4..].chars().all(|ch| ch.is_ascii_digit())
}

fn starts_with_bullet_or_numbering(line: &str) -> bool {
    let trimmed = line.trim_start();
    if trimmed.starts_with('-')
        || trimmed.starts_with('*')
        || trimmed.starts_with('•')
        || trimmed.starts_with('○')
        || trimmed.starts_with('□')
        || trimmed.starts_with('☐')
        || trimmed.starts_with('☑')
    {
        return true;
    }
    let mut chars = trimmed.chars().peekable();
    let mut saw_digit = false;
    while let Some(ch) = chars.peek().copied() {
        if ch.is_ascii_digit() {
            saw_digit = true;
            chars.next();
            continue;
        }
        break;
    }
    if !saw_digit {
        return false;
    }
    matches!(chars.peek().copied(), Some('.') | Some(')'))
}

fn merge_pdf_wrapped_lines(lines: &[String]) -> Vec<String> {
    let mut merged = Vec::new();
    let mut buffer = String::new();
    for line in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if buffer.is_empty() {
            buffer = trimmed.to_string();
            continue;
        }

        let should_join = should_join_pdf_line(&buffer, trimmed);
        if should_join {
            buffer.push(' ');
            buffer.push_str(trimmed);
        } else {
            merged.push(buffer);
            buffer = trimmed.to_string();
        }
    }
    if !buffer.is_empty() {
        merged.push(buffer);
    }
    merged
}

fn should_join_pdf_line(previous: &str, next: &str) -> bool {
    if previous.len() + next.len() > 170 {
        return false;
    }
    if previous.ends_with('?')
        || previous.ends_with(':')
        || previous.ends_with('.')
        || previous.ends_with(';')
    {
        return false;
    }
    let next_lower = next.to_ascii_lowercase();
    next.chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_lowercase())
        || next_lower.starts_with("and ")
        || next_lower.starts_with("or ")
        || next_lower.starts_with("with ")
        || next_lower.starts_with("without ")
        || next_lower.starts_with("for ")
        || next_lower.starts_with("to ")
}

fn dedupe_preserving_order(values: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut deduped = Vec::new();
    for value in values {
        let key = value.to_ascii_lowercase();
        if seen.insert(key) {
            deduped.push(value);
        }
    }
    deduped
}

fn unique_pdf_field_key(
    base_key: String,
    seen_keys: &mut HashSet<String>,
    fallback_counter: &mut usize,
) -> String {
    let initial = if base_key.is_empty() {
        let generated = format!("pdf_field_{}", *fallback_counter);
        *fallback_counter += 1;
        generated
    } else {
        base_key
    };
    if seen_keys.insert(initial.clone()) {
        return initial;
    }
    let mut suffix = 2usize;
    loop {
        let candidate = format!("{initial}_{suffix}");
        if seen_keys.insert(candidate.clone()) {
            return candidate;
        }
        suffix += 1;
    }
}

fn sanitize_pdf_field_label(line: &str) -> String {
    let trimmed = line.trim();
    let without_bullets = trimmed.trim_start_matches(|ch: char| {
        matches!(
            ch,
            '-' | '*' | '•' | '●' | '○' | '◦' | '▪' | '□' | '☐' | '☑'
        )
    });
    let without_numbering = if let Some(space_index) = without_bullets.find(' ') {
        let first_token = &without_bullets[..space_index];
        if first_token
            .chars()
            .all(|ch| ch.is_ascii_digit() || ch == '.' || ch == ')')
        {
            &without_bullets[space_index + 1..]
        } else {
            without_bullets
        }
    } else {
        without_bullets
    };
    normalize_whitespace(without_numbering)
        .replace('\u{fffd}', " ")
        .replace("??", " ")
        .trim_end_matches(':')
        .trim()
        .to_string()
}

fn decode_lopdf_bytes_best_effort(bytes: &[u8]) -> String {
    let decoded_primary = normalize_whitespace(&LopdfDocument::decode_text(None, bytes));
    if is_plausible_import_label(&decoded_primary) {
        return decoded_primary;
    }
    let decoded_fallback = normalize_whitespace(&String::from_utf8_lossy(bytes));
    if is_plausible_import_label(&decoded_fallback) {
        return decoded_fallback;
    }
    decoded_primary
}

fn contains_pdf_encoding_noise(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    if lower.contains("unimplemented??identity-h")
        || lower.contains("unimplemented??identity-v")
        || lower.contains("identity-h unimplemented")
        || lower.contains("identity-v unimplemented")
        || lower.contains("unimplemented??")
    {
        return true;
    }
    lower.matches("unimplemented").count() >= 2
}

fn is_plausible_import_label(label: &str) -> bool {
    let trimmed = label.trim();
    if trimmed.len() < 2 || trimmed.len() > 180 {
        return false;
    }
    if contains_pdf_encoding_noise(trimmed) {
        return false;
    }
    if looks_like_pdf_page_noise(trimmed) {
        return false;
    }
    let alpha_count = trimmed
        .chars()
        .filter(|ch| ch.is_ascii_alphabetic())
        .count();
    if alpha_count < 2 {
        return false;
    }
    let tokens = trimmed
        .split_whitespace()
        .map(|token| token.to_ascii_lowercase())
        .collect::<Vec<_>>();
    if tokens.len() > 3 {
        let mut token_counts: HashMap<String, usize> = HashMap::new();
        for token in &tokens {
            *token_counts.entry(token.clone()).or_insert(0) += 1;
        }
        let max_count = token_counts.values().copied().max().unwrap_or(0);
        if max_count.saturating_mul(100) / tokens.len() >= 60 {
            return false;
        }
    }
    true
}

fn infer_pdf_field_type_and_options(label: &str) -> (String, String) {
    let lower = label.to_ascii_lowercase();
    if lower.contains("yes/no") || lower.contains("yes or no") {
        return ("boolean".to_string(), "[]".to_string());
    }
    if lower.contains("date and time") || lower.contains("datetime") {
        return ("datetime".to_string(), "[]".to_string());
    }
    if lower.contains("date") {
        return ("date".to_string(), "[]".to_string());
    }
    let inline_options = extract_inline_choice_options(label);
    if lower.contains("select all")
        || lower.contains("check all")
        || lower.contains("choose all")
        || lower.contains("multiple")
    {
        return (
            "multi_select".to_string(),
            serde_json::to_string(&inline_options).unwrap_or_else(|_| "[]".to_string()),
        );
    }
    if lower.contains("select one")
        || lower.contains("choose one")
        || lower.contains("pick one")
        || lower.contains("radio")
        || inline_options.len() > 1
    {
        return (
            "single_select".to_string(),
            serde_json::to_string(&inline_options).unwrap_or_else(|_| "[]".to_string()),
        );
    }
    if lower.contains("number") || lower.contains("age") {
        return ("number".to_string(), "[]".to_string());
    }
    if lower.contains("comment")
        || lower.contains("describe")
        || lower.contains("notes")
        || lower.contains("explain")
    {
        return ("textarea".to_string(), "[]".to_string());
    }
    ("text".to_string(), "[]".to_string())
}

fn extract_inline_choice_options(label: &str) -> Vec<String> {
    let mut candidates = Vec::new();
    if let (Some(start), Some(end)) = (label.find('('), label.rfind(')')) {
        if end > start + 1 {
            candidates.push(label[start + 1..end].to_string());
        }
    }
    if let Some(colon_index) = label.find(':') {
        candidates.push(label[colon_index + 1..].to_string());
    }

    for candidate in candidates {
        let separator = if candidate.contains('|') {
            '|'
        } else if candidate.contains('/') {
            '/'
        } else if candidate.contains(';') {
            ';'
        } else if candidate.contains(',') {
            ','
        } else {
            '\0'
        };
        if separator == '\0' {
            continue;
        }
        let values = candidate
            .split(separator)
            .map(normalize_whitespace)
            .map(|value| {
                value
                    .trim_matches(|ch: char| matches!(ch, '(' | ')' | '[' | ']'))
                    .trim()
                    .to_string()
            })
            .filter(|value| !value.is_empty() && value.len() <= 80)
            .collect::<Vec<_>>();
        if values.len() > 1 {
            return values;
        }
    }
    Vec::new()
}

fn strip_inline_option_hints(label: &str) -> String {
    if let (Some(start), Some(end)) = (label.find('('), label.rfind(')')) {
        if end > start {
            let inside = &label[start + 1..end];
            if inside.contains('/')
                || inside.contains('|')
                || inside.contains(';')
                || inside.contains(',')
            {
                return normalize_whitespace(
                    format!("{} {}", &label[..start], &label[end + 1..]).as_str(),
                );
            }
        }
    }
    label.to_string()
}

fn normalize_html_field_key(raw_key: &str) -> String {
    let mut normalized = String::new();
    let mut previous_was_separator = false;
    for ch in raw_key.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            normalized.push(ch.to_ascii_lowercase());
            previous_was_separator = false;
        } else if !previous_was_separator {
            normalized.push('_');
            previous_was_separator = true;
        }
    }
    normalized.trim_matches('_').to_string()
}

fn infer_html_field_label(raw_key: &str, fallback_label: Option<&str>) -> String {
    if let Some(label) = fallback_label {
        let normalized = normalize_whitespace(label);
        if !normalized.is_empty() {
            return normalized;
        }
    }
    let normalized_key = normalize_whitespace(&raw_key.replace(['_', '-'], " "));
    if normalized_key.is_empty() {
        return "Imported field".to_string();
    }
    normalized_key
        .split(' ')
        .filter(|token| !token.is_empty())
        .map(title_case_token)
        .collect::<Vec<_>>()
        .join(" ")
}

fn title_case_token(token: &str) -> String {
    let mut chars = token.chars();
    if let Some(first) = chars.next() {
        let mut result = String::new();
        result.push(first.to_ascii_uppercase());
        result.push_str(&chars.as_str().to_ascii_lowercase());
        result
    } else {
        String::new()
    }
}

fn normalize_whitespace(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn render_crf_field_type_options(selected_type: &str) -> String {
    [
        ("text", "Short text"),
        ("textarea", "Long text"),
        ("number", "Number"),
        ("date", "Date"),
        ("datetime", "Date + time"),
        ("boolean", "Yes / No"),
        ("single_select", "Single choice (one answer)"),
        ("multi_select", "Multiple choice (many answers)"),
    ]
    .iter()
    .map(|(field_type, label)| {
        let selected = if field_type.eq_ignore_ascii_case(selected_type) {
            " selected"
        } else {
            ""
        };
        format!(
            r#"<option value="{}"{}>{}</option>"#,
            field_type, selected, label
        )
    })
    .collect::<Vec<_>>()
    .join("")
}

fn map_crf_field_error_to_notice(prefix: &str, error_text: &str) -> String {
    let normalized = error_text.trim().to_ascii_lowercase();
    if normalized.contains("duplicate key value violates unique constraint")
        && normalized.contains("study_crf_fields_template_id_field_key_key")
    {
        return format!("{prefix}: field key already exists in this template.");
    }
    if normalized.contains("invalid field_type") {
        return format!("{prefix}: invalid field type.");
    }
    if normalized.contains("invalid input syntax for type json") {
        return format!("{prefix}: choice options format is invalid.");
    }
    format!("{prefix}: {error_text}")
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
      background-size: 120% 120%;
      animation: page-shift 14s ease-in-out infinite alternate;
    }
    @keyframes page-shift {
      from { background-position: 0% 0%; }
      to { background-position: 100% 8%; }
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
      background: linear-gradient(120deg, rgba(2, 24, 43, 0.95) 0%, rgba(40, 62, 40, 0.93) 65%, rgba(240, 87, 8, 0.93) 100%);
      border: 1px solid rgba(2, 24, 43, 0.35);
      border-radius: 16px;
      padding: 0.95rem 1rem;
      box-shadow: 0 10px 28px rgba(2, 24, 43, 0.2);
      margin-bottom: 0.95rem;
    }
    .brand {
      color: #f8f6f2;
      font-size: 0.88rem;
      letter-spacing: 0.04em;
      text-transform: uppercase;
      font-weight: 700;
      margin: 0;
    }
    .brand-sub {
      color: #f3eee7;
      font-size: 0.84rem;
      font-weight: 700;
      background: rgba(255, 255, 255, 0.18);
      border: 1px solid rgba(255, 255, 255, 0.3);
      border-radius: 999px;
      padding: 0.32rem 0.62rem;
      backdrop-filter: blur(3px);
    }
    .surface-glow {
      display: flex;
      align-items: center;
      gap: 0.45rem;
      font-size: 0.8rem;
      font-weight: 700;
      color: #173149;
      background: rgba(255, 250, 243, 0.94);
      border: 1px solid rgba(197, 183, 171, 0.9);
      border-radius: 999px;
      width: fit-content;
      padding: 0.35rem 0.72rem;
      margin: -0.25rem 0 0.9rem;
      box-shadow: 0 8px 18px rgba(2, 24, 43, 0.12);
    }
    .pulse-dot {
      width: 0.56rem;
      height: 0.56rem;
      border-radius: 50%;
      background: var(--cg-orange);
      box-shadow: 0 0 0 rgba(240, 87, 8, 0.45);
      animation: pulse 1.7s infinite;
    }
    @keyframes pulse {
      0% { box-shadow: 0 0 0 0 rgba(240, 87, 8, 0.45); }
      70% { box-shadow: 0 0 0 9px rgba(240, 87, 8, 0); }
      100% { box-shadow: 0 0 0 0 rgba(240, 87, 8, 0); }
    }
    .card {
      background: linear-gradient(180deg, rgba(255, 253, 248, 0.98) 0%, rgba(255, 250, 243, 0.96) 100%);
      border: 1px solid rgba(197, 183, 171, 0.9);
      border-radius: 16px;
      box-shadow: 0 14px 28px rgba(2, 24, 43, 0.09);
      padding: 1rem 1.1rem 1.15rem;
      margin-bottom: 1.05rem;
      transition: transform 160ms ease, box-shadow 220ms ease, border-color 160ms ease;
    }
    .card:hover {
      transform: translateY(-1.5px);
      box-shadow: 0 18px 34px rgba(2, 24, 43, 0.12);
      border-color: rgba(240, 87, 8, 0.36);
    }
    .info-grid {
      display: grid;
      grid-template-columns: repeat(auto-fit, minmax(150px, 1fr));
      gap: 0.65rem;
      margin: 0.55rem 0 0.7rem;
    }
    .info-card {
      border: 1px solid rgba(197, 183, 171, 0.88);
      border-radius: 12px;
      padding: 0.65rem 0.7rem;
      background: #fffefb;
      box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.7);
    }
    .metric-label {
      font-size: 0.77rem;
      color: #445b72;
      font-weight: 700;
      text-transform: uppercase;
      letter-spacing: 0.03em;
      margin-bottom: 0.2rem;
    }
    .metric-value {
      font-size: 1.03rem;
      font-weight: 800;
      color: var(--cg-navy);
    }
    .action-grid {
      display: grid;
      grid-template-columns: repeat(auto-fit, minmax(230px, 1fr));
      gap: 0.65rem;
      margin: 0.45rem 0 0.8rem;
    }
    .action-card {
      display: block;
      text-decoration: none;
      border: 1px solid rgba(197, 183, 171, 0.9);
      border-radius: 13px;
      padding: 0.72rem;
      background: #fffefb;
      color: #10263d;
      transition: transform 140ms ease, box-shadow 160ms ease, border-color 140ms ease;
      box-shadow: 0 9px 18px rgba(2, 24, 43, 0.08);
    }
    .action-card:hover {
      transform: translateY(-1px);
      box-shadow: 0 13px 22px rgba(2, 24, 43, 0.12);
      border-color: rgba(240, 87, 8, 0.42);
    }
    .action-card.is-blocked {
      border-left: 4px solid #cf5b1c;
      background: linear-gradient(180deg, #fffdf9 0%, #fff7f0 100%);
    }
    .action-card.is-informative {
      border-left: 4px solid #2f5878;
      background: linear-gradient(180deg, #fffefb 0%, #f7fbff 100%);
    }
    .action-card.is-ready {
      border-left: 4px solid #356a35;
      background: linear-gradient(180deg, #f9fff7 0%, #f3fff1 100%);
    }
    .action-title {
      font-size: 0.95rem;
      font-weight: 800;
      margin-bottom: 0.18rem;
      color: var(--cg-navy);
    }
    .action-desc {
      font-size: 0.84rem;
      color: #314c66;
      line-height: 1.4;
      margin-bottom: 0.45rem;
    }
    .action-tag {
      display: inline-block;
      font-size: 0.76rem;
      font-weight: 700;
      padding: 0.22rem 0.56rem;
      border-radius: 999px;
      background: rgba(2, 24, 43, 0.08);
      color: #26445f;
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
    form:not([style*="display:inline"]):not([style*="display:inline-block"]) {
      max-width: 760px;
      background: linear-gradient(180deg, #ffffff 0%, #f7fbff 100%);
      border: 1px solid rgba(39, 79, 112, 0.22);
      border-radius: 16px;
      padding: 0.9rem 1rem 1.05rem;
      box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.78), 0 8px 16px rgba(2, 24, 43, 0.07);
      margin-top: 0.7rem;
      margin-bottom: 0.8rem;
    }
    form[style*="display:inline"],
    form[style*="display:inline-block"] {
      max-width: none;
      background: transparent;
      border: none;
      border-radius: 0;
      padding: 0;
      box-shadow: none;
      margin: 0;
    }
    label {
      font-size: 0.84rem;
      font-weight: 800;
      margin-bottom: 0.24rem;
      margin-top: 0.62rem;
      display: block;
      color: #1b3956;
    }
    input:not([type="checkbox"]):not([type="radio"]), textarea, select {
      width: min(100%, 680px);
      border: 1px solid rgba(47, 88, 120, 0.28);
      border-radius: 14px;
      padding: 0.76rem 0.82rem;
      background: #ffffff;
      color: var(--cg-navy);
      box-shadow: inset 0 1px 2px rgba(15, 45, 72, 0.08);
    }
    input[type="checkbox"], input[type="radio"] {
      width: auto;
      accent-color: var(--cg-orange);
      transform: scale(1.05);
      margin-top: 0.18rem;
    }
    input:not([type="checkbox"]):not([type="radio"]):focus, textarea:focus, select:focus {
      outline: 2px solid rgba(240, 87, 8, 0.22);
      border-color: var(--cg-orange);
      box-shadow: 0 0 0 4px rgba(240, 87, 8, 0.08);
    }
    textarea {
      min-height: 190px;
      resize: vertical;
    }
    form[data-crf-field-form] [data-options-section] {
      margin-top: 0.35rem;
    }
    form[data-crf-field-form] [data-option-list] {
      margin-top: 0.2rem;
    }
    form[data-crf-field-form] .option-row {
      display: grid;
      grid-template-columns: minmax(0, 1fr) auto;
      gap: 0.45rem;
      margin-bottom: 0.42rem;
      align-items: center;
    }
    form[data-crf-field-form] .option-row button,
    form[data-crf-field-form] [data-add-option-button] {
      margin-top: 0;
      padding: 0.46rem 0.68rem;
      box-shadow: none;
      background: #edf1f5;
      color: #1c3a55;
      border: 1px solid rgba(47, 88, 120, 0.28);
      font-size: 0.8rem;
      font-weight: 700;
    }
    button {
      background: linear-gradient(135deg, #f36e1d 0%, var(--cg-orange) 100%);
      color: #fff;
      border: none;
      border-radius: 12px;
      padding: 0.66rem 1rem;
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
    .danger-note {
      background: #fff0f0;
      border: 1px solid #f1b6b6;
      border-left: 4px solid #c53232;
      color: #6a1717;
      border-radius: 10px;
      padding: 0.72rem 0.82rem;
      font-weight: 700;
    }
    .danger-note-soft {
      background: #fff7f4;
      border-color: #efc5bd;
      border-left-color: #d8624a;
      color: #703229;
    }
    .danger-button {
      background: linear-gradient(135deg, #c74343 0%, #a81717 100%);
      box-shadow: 0 10px 18px rgba(156, 24, 24, 0.28);
    }
    .danger-button:hover {
      filter: brightness(0.95);
    }
    .danger-button-soft {
      background: linear-gradient(135deg, #d56147 0%, #bb3f28 100%);
      box-shadow: 0 10px 18px rgba(181, 74, 47, 0.24);
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
    .tab-panel.is-active {
      display: block;
      animation: tab-fade-in 220ms ease;
    }
    @keyframes tab-fade-in {
      from { opacity: 0; transform: translateY(4px); }
      to { opacity: 1; transform: translateY(0); }
    }
    @media (max-width: 740px) {
      .brand-wrap { flex-direction: column; align-items: flex-start; gap: 0.35rem; }
      .tab-bar { position: static; }
    }
    "#
}

fn cingulum_home_css() -> &'static str {
    r#"
    body {
      min-height: 100vh;
      display: grid;
      place-items: center;
      padding: 1.2rem;
    }
    .home-shell {
      width: min(900px, 100%);
      display: grid;
      grid-template-columns: 1fr 1fr;
      gap: 1rem;
      animation: home-rise 360ms ease;
    }
    .home-card {
      background: linear-gradient(180deg, rgba(255, 253, 248, 0.97) 0%, rgba(255, 248, 238, 0.95) 100%);
      border: 1px solid rgba(197, 183, 171, 0.9);
      border-radius: 18px;
      padding: 1.35rem;
      box-shadow: 0 20px 38px rgba(2, 24, 43, 0.14);
      transition: transform 180ms ease, box-shadow 220ms ease;
      animation: card-fade 420ms ease both;
    }
    .home-card:hover {
      transform: translateY(-2px);
      box-shadow: 0 24px 44px rgba(2, 24, 43, 0.18);
    }
    .logo-card { animation-delay: 40ms; }
    .login-card { animation-delay: 110ms; }
    .cx-logo-wrap {
      display: flex;
      justify-content: center;
      margin-bottom: 0.9rem;
    }
    .cx-logo {
      width: 110px;
      height: 110px;
      border-radius: 26px;
      display: grid;
      place-items: center;
      font-size: 2.1rem;
      font-weight: 800;
      letter-spacing: 0.06em;
      color: #fff;
      background: linear-gradient(130deg, #02182B 0%, #283E28 55%, #F05708 100%);
      box-shadow: 0 18px 36px rgba(2, 24, 43, 0.28);
      animation: logo-breathe 2.6s ease-in-out infinite;
    }
    .home-copy {
      margin-top: 0.85rem;
      line-height: 1.48;
      font-size: 0.96rem;
    }
    .login-card h2 {
      margin-bottom: 0.45rem;
      font-size: 1.32rem;
    }
    .login-card form {
      margin-top: 0.55rem;
    }
    .login-card button {
      width: 100%;
      margin-top: 0.9rem;
      padding-top: 0.75rem;
      padding-bottom: 0.75rem;
    }
    @keyframes home-rise {
      from { opacity: 0; transform: translateY(8px); }
      to { opacity: 1; transform: translateY(0); }
    }
    @keyframes card-fade {
      from { opacity: 0; transform: translateY(10px); }
      to { opacity: 1; transform: translateY(0); }
    }
    @keyframes logo-breathe {
      0%, 100% { transform: translateY(0); box-shadow: 0 18px 36px rgba(2, 24, 43, 0.28); }
      50% { transform: translateY(-2px); box-shadow: 0 22px 44px rgba(2, 24, 43, 0.33); }
    }
    @media (max-width: 820px) {
      .home-shell {
        grid-template-columns: 1fr;
      }
    }
    "#
}

fn render_home_page(title: &str, body_content: String) -> String {
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>{}</title>
  <style>{}{}</style>
</head>
<body>
  {}
</body>
</html>"#,
        html_escape(title),
        cingulum_theme_css(),
        cingulum_home_css(),
        body_content
    )
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
      <div class="brand-sub">Virivu Research Cloud · UI Refresh v3</div>
    </div>
    <div class="surface-glow"><span class="pulse-dot"></span>Tenant-isolated workspace mode is active</div>
    {}
  </main>
  <script>
    (() => {{
      const bars = document.querySelectorAll('.tab-bar[data-tab-group]');
      const tabParam = new URLSearchParams(window.location.search).get('tab');
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
        if (tabParam && buttons.some((b) => b.getAttribute('data-tab-id') === tabParam)) {{
          initial = tabParam;
        }}
        if (!initial) initial = buttons[0].getAttribute('data-tab-id');
        activate(initial);
        buttons.forEach((btn) => {{
          btn.addEventListener('click', () => activate(btn.getAttribute('data-tab-id')));
        }});
      }});
      const fieldForms = Array.from(document.querySelectorAll('form[data-crf-field-form]'));
      const createOptionRow = (value = '') => {{
        const row = document.createElement('div');
        row.className = 'option-row';
        const input = document.createElement('input');
        input.type = 'text';
        input.placeholder = 'Choice option';
        input.value = value;
        input.setAttribute('data-option-input', 'true');
        const removeButton = document.createElement('button');
        removeButton.type = 'button';
        removeButton.textContent = 'Remove';
        removeButton.setAttribute('data-remove-option-button', 'true');
        row.appendChild(input);
        row.appendChild(removeButton);
        return row;
      }};
      fieldForms.forEach((form) => {{
        const fieldTypeSelect = form.querySelector('[data-field-type-select]');
        const optionsSection = form.querySelector('[data-options-section]');
        const optionList = form.querySelector('[data-option-list]');
        const addOptionButton = form.querySelector('[data-add-option-button]');
        const optionsTextInput = form.querySelector('input[name="options_text"]');
        const optionsInitialInput = form.querySelector('[data-options-initial]');
        if (!fieldTypeSelect || !optionsSection || !optionList || !addOptionButton || !optionsTextInput || !optionsInitialInput) return;
        const syncOptionsTextInput = () => {{
          const values = Array.from(optionList.querySelectorAll('[data-option-input]'))
            .map((input) => input.value.trim())
            .filter((value) => value.length > 0);
          optionsTextInput.value = values.join('\n');
        }};
        const ensureOptionRow = () => {{
          if (optionList.querySelectorAll('[data-option-input]').length === 0) {{
            optionList.appendChild(createOptionRow(''));
          }}
        }};
        const seedOptionRows = () => {{
          const seededValues = (optionsInitialInput.value || '')
            .split(/\r?\n/)
            .map((line) => line.trim())
            .filter((line) => line.length > 0);
          optionList.innerHTML = '';
          if (seededValues.length === 0) {{
            optionList.appendChild(createOptionRow(''));
          }} else {{
            seededValues.forEach((value) => optionList.appendChild(createOptionRow(value)));
          }}
          syncOptionsTextInput();
        }};
        seedOptionRows();
        optionList.addEventListener('input', (event) => {{
          if (event.target && event.target.matches('[data-option-input]')) {{
            syncOptionsTextInput();
          }}
        }});
        optionList.addEventListener('click', (event) => {{
          const target = event.target;
          if (!(target instanceof HTMLElement)) return;
          if (!target.matches('[data-remove-option-button]')) return;
          const row = target.closest('.option-row');
          if (row) row.remove();
          ensureOptionRow();
          syncOptionsTextInput();
        }});
        addOptionButton.addEventListener('click', () => {{
          optionList.appendChild(createOptionRow(''));
          syncOptionsTextInput();
        }});
        const syncFieldOptionsVisibility = () => {{
          const fieldType = (fieldTypeSelect.value || '').toLowerCase();
          const showOptions = fieldType === 'single_select' || fieldType === 'multi_select';
          optionsSection.style.display = showOptions ? 'block' : 'none';
          if (showOptions) {{
            ensureOptionRow();
          }}
          syncOptionsTextInput();
        }};
        form.addEventListener('submit', () => syncOptionsTextInput());
        fieldTypeSelect.addEventListener('change', syncFieldOptionsVisibility);
        syncFieldOptionsVisibility();
      }});
      const hashId = window.location.hash ? window.location.hash.slice(1) : '';
      if (hashId) {{
        const target = document.getElementById(hashId);
        if (target) {{
          window.requestAnimationFrame(() => {{
            target.scrollIntoView({{ behavior: 'smooth', block: 'start' }});
          }});
        }}
      }}
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
