use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    config::Config,
    models::{FormInvite, MediaUploadTicket, Organization, Project, Site},
    state::SharedState,
};

#[derive(Clone)]
pub struct AppContext {
    pub config: Config,
    pub state: SharedState,
}

pub fn router(ctx: AppContext) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/v1/organizations", post(create_organization))
        .route("/v1/projects", post(create_project))
        .route("/v1/sites", post(create_site))
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
        .with_state(ctx)
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

#[derive(Debug, Deserialize)]
struct CreateOrganizationRequest {
    name: String,
}

async fn create_organization(
    State(ctx): State<AppContext>,
    Json(payload): Json<CreateOrganizationRequest>,
) -> impl IntoResponse {
    if payload.name.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "organization name is required" })),
        );
    }

    let org = Organization {
        id: Uuid::new_v4(),
        name: payload.name.trim().to_string(),
        created_at: Utc::now(),
    };

    let mut state = ctx.state.write().await;
    state.organizations.push(org.clone());

    (StatusCode::CREATED, Json(serde_json::json!(org)))
}

#[derive(Debug, Deserialize)]
struct CreateProjectRequest {
    organization_id: Uuid,
    name: String,
    therapeutic_area: String,
}

async fn create_project(
    State(ctx): State<AppContext>,
    Json(payload): Json<CreateProjectRequest>,
) -> impl IntoResponse {
    let project = Project {
        id: Uuid::new_v4(),
        organization_id: payload.organization_id,
        name: payload.name.trim().to_string(),
        therapeutic_area: payload.therapeutic_area.trim().to_string(),
        created_at: Utc::now(),
    };

    let mut state = ctx.state.write().await;
    state.projects.push(project.clone());

    (StatusCode::CREATED, Json(serde_json::json!(project)))
}

#[derive(Debug, Deserialize)]
struct CreateSiteRequest {
    project_id: Uuid,
    name: String,
    principal_investigator: String,
}

async fn create_site(
    State(ctx): State<AppContext>,
    Json(payload): Json<CreateSiteRequest>,
) -> impl IntoResponse {
    let site = Site {
        id: Uuid::new_v4(),
        project_id: payload.project_id,
        name: payload.name.trim().to_string(),
        principal_investigator: payload.principal_investigator.trim().to_string(),
        created_at: Utc::now(),
    };

    let mut state = ctx.state.write().await;
    state.sites.push(site.clone());

    (StatusCode::CREATED, Json(serde_json::json!(site)))
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
    Json(payload): Json<SendFormInviteRequest>,
) -> impl IntoResponse {
    let invite = FormInvite {
        id: Uuid::new_v4(),
        organization_id: payload.organization_id,
        project_id: payload.project_id,
        patient_email: payload.patient_email.trim().to_string(),
        form_type: payload.form_type.trim().to_string(),
        status: "sent".to_string(),
        created_at: Utc::now(),
    };

    let mut state = ctx.state.write().await;
    state.form_invites.push(invite.clone());

    (StatusCode::CREATED, Json(serde_json::json!(invite)))
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
    Json(payload): Json<PresignMediaUploadRequest>,
) -> impl IntoResponse {
    let id = Uuid::new_v4();
    let expires_at = Utc::now() + Duration::minutes(10);
    let upload_url = format!(
        "https://upload.virivu.example/v1/media/{id}?content_type={}",
        payload.mime_type
    );

    let ticket = MediaUploadTicket {
        id,
        organization_id: payload.organization_id,
        project_id: payload.project_id,
        patient_id: payload.patient_id.trim().to_string(),
        mime_type: payload.mime_type.trim().to_string(),
        upload_url,
        expires_at,
    };

    let mut state = ctx.state.write().await;
    state.media_tickets.push(ticket.clone());

    (StatusCode::CREATED, Json(serde_json::json!(ticket)))
}

#[derive(Debug, Serialize)]
struct OrganizationSummary {
    organization_id: Uuid,
    projects: usize,
    sites: usize,
    sent_form_invites: usize,
    generated_media_upload_links: usize,
}

async fn organization_summary(
    State(ctx): State<AppContext>,
    Path(org_id): Path<Uuid>,
) -> impl IntoResponse {
    let state = ctx.state.read().await;

    let projects = state
        .projects
        .iter()
        .filter(|p| p.organization_id == org_id)
        .count();

    let project_ids: Vec<Uuid> = state
        .projects
        .iter()
        .filter(|p| p.organization_id == org_id)
        .map(|p| p.id)
        .collect();

    let sites = state
        .sites
        .iter()
        .filter(|s| project_ids.contains(&s.project_id))
        .count();

    let sent_form_invites = state
        .form_invites
        .iter()
        .filter(|f| f.organization_id == org_id)
        .count();

    let generated_media_upload_links = state
        .media_tickets
        .iter()
        .filter(|m| m.organization_id == org_id)
        .count();

    Json(OrganizationSummary {
        organization_id: org_id,
        projects,
        sites,
        sent_form_invites,
        generated_media_upload_links,
    })
}

#[derive(Debug, Serialize)]
struct ProjectProgressReport {
    project_id: Uuid,
    total_sites: usize,
    total_form_invites: usize,
    total_media_captures_requested: usize,
    report_generated_at: String,
}

async fn project_progress_report(
    State(ctx): State<AppContext>,
    Path(project_id): Path<Uuid>,
) -> impl IntoResponse {
    let state = ctx.state.read().await;
    let total_sites = state
        .sites
        .iter()
        .filter(|s| s.project_id == project_id)
        .count();
    let total_form_invites = state
        .form_invites
        .iter()
        .filter(|s| s.project_id == project_id)
        .count();
    let total_media_captures_requested = state
        .media_tickets
        .iter()
        .filter(|s| s.project_id == project_id)
        .count();

    Json(ProjectProgressReport {
        project_id,
        total_sites,
        total_form_invites,
        total_media_captures_requested,
        report_generated_at: Utc::now().to_rfc3339(),
    })
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

async fn generate_doctor_patient_note(Json(payload): Json<TranscriptRequest>) -> impl IntoResponse {
    let transcript_excerpt: String = payload.transcript.chars().take(240).collect();
    let summary_note = format!(
        "Draft note for clinician {} and patient {}. Transcript excerpt: {}",
        payload.clinician_name, payload.patient_name, transcript_excerpt
    );

    (
        StatusCode::OK,
        Json(TranscriptResponse {
            organization_id: payload.organization_id,
            project_id: payload.project_id,
            summary_note,
            reminder: "This is a non-diagnostic draft and requires clinician review.",
        }),
    )
}
