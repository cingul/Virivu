use axum::{
    extract::{Path, Request, State},
    http::StatusCode,
    middleware::{from_fn_with_state, Next},
    response::{IntoResponse, Response},
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
    models::{DataUseAgreement, DataUseAgreementSignature},
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
        .route(
            "/v1/auth/google/token-introspect",
            post(google_token_introspect),
        )
        .route(
            "/v1/legal/data-use-agreements/sign-hospital",
            post(sign_data_use_agreement_hospital),
        );

    let protected_router = Router::new()
        .route("/v1/organizations", post(create_organization))
        .route("/v1/projects", post(create_project))
        .route("/v1/sites", post(create_site))
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
        .layer(from_fn_with_state(ctx.clone(), require_auth));

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
        agreement,
        signatures,
        hospital_signature_endpoint: "/v1/legal/data-use-agreements/sign-hospital".to_string(),
    }))
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
        agreement,
        signatures,
        hospital_signature_endpoint: "/v1/legal/data-use-agreements/sign-hospital".to_string(),
    }))
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
