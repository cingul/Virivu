use axum::{
    extract::{Path, State},
    middleware::from_fn_with_state,
    response::Html,
    routing::{get, post},
    Extension, Json, Router,
};
use chrono::Utc;
use uuid::Uuid;

use crate::{
    auth::{require_any_role, require_auth},
    error::ApiError,
    models::{
        AppRole, AuthenticatedUser, CreateCrfSubmissionRequest, CreateCrfTemplateRequest,
        CreateDataQueryRequest, CreateDuaRequest, CreateOrganizationRequest, CreateSiteRequest,
        CreateStudyRequest, CreateVisitRequest, HealthResponse, MarkSiteStartupRequest,
        StudyReadiness, TransitionStudyPhaseRequest,
    },
    state::AppState,
    workflow::validate_phase_transition,
};

pub fn router(state: AppState, app_name: String) -> Router {
    let public_router = Router::new().route("/health", get(move || health(app_name.clone())));

    let protected_router = Router::new()
        .route("/", get(render_wizard_shell))
        .route("/ui", get(render_wizard_shell))
        .route(
            "/api/v1/organizations",
            get(list_organizations).post(create_organization),
        )
        .route("/api/v1/studies", get(list_studies).post(create_study))
        .route(
            "/api/v1/studies/{study_id}/phase",
            post(transition_study_phase),
        )
        .route("/api/v1/studies/{study_id}/readiness", get(study_readiness))
        .route("/api/v1/sites", get(list_sites).post(create_site))
        .route("/api/v1/sites/{site_id}/startup", post(mark_site_startup))
        .route("/api/v1/patients", get(list_patients).post(enroll_patient))
        .route("/api/v1/visits", get(list_visits).post(create_visit))
        .route(
            "/api/v1/crf-templates",
            get(list_crf_templates).post(create_crf_template),
        )
        .route(
            "/api/v1/crf-templates/{template_id}/publish",
            post(publish_crf_template),
        )
        .route(
            "/api/v1/crf-submissions",
            get(list_crf_submissions).post(create_crf_submission),
        )
        .route(
            "/api/v1/crf-submissions/{submission_id}/lock",
            post(lock_crf_submission),
        )
        .route(
            "/api/v1/data-queries",
            get(list_data_queries).post(create_data_query),
        )
        .route(
            "/api/v1/data-queries/{query_id}/close",
            post(close_data_query),
        )
        .route("/api/v1/duas", get(list_duas).post(create_dua))
        .route("/api/v1/duas/{dua_id}/activate", post(activate_dua))
        .route_layer(from_fn_with_state(state.clone(), require_auth))
        .with_state(state);

    public_router.merge(protected_router)
}

async fn health(app_name: String) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        service: app_name,
        timestamp_utc: Utc::now(),
    })
}

fn can_read(user: &AuthenticatedUser) -> Result<(), ApiError> {
    require_any_role(
        user,
        &[
            AppRole::PlatformAdmin,
            AppRole::OrgAdmin,
            AppRole::Investigator,
            AppRole::SiteCoordinator,
            AppRole::Analyst,
            AppRole::Monitor,
        ],
    )
}

fn can_write(user: &AuthenticatedUser) -> Result<(), ApiError> {
    require_any_role(
        user,
        &[
            AppRole::PlatformAdmin,
            AppRole::OrgAdmin,
            AppRole::Investigator,
            AppRole::SiteCoordinator,
        ],
    )
}

fn can_admin(user: &AuthenticatedUser) -> Result<(), ApiError> {
    require_any_role(user, &[AppRole::PlatformAdmin, AppRole::OrgAdmin])
}

async fn create_organization(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Json(input): Json<CreateOrganizationRequest>,
) -> Result<Json<crate::models::Organization>, ApiError> {
    can_admin(&user)?;
    if input.name.trim().is_empty() || input.workspace_slug.trim().is_empty() {
        return Err(ApiError::BadRequest(
            "name and workspace_slug are required".to_string(),
        ));
    }
    let organization = state
        .repository
        .create_organization(
            input.name.trim(),
            &input.workspace_slug.trim().to_lowercase(),
        )
        .await?;
    Ok(Json(organization))
}

async fn list_organizations(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
) -> Result<Json<Vec<crate::models::Organization>>, ApiError> {
    can_read(&user)?;
    Ok(Json(state.repository.list_organizations().await?))
}

async fn create_study(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Json(input): Json<CreateStudyRequest>,
) -> Result<Json<crate::models::Study>, ApiError> {
    can_write(&user)?;
    if input.short_code.trim().is_empty() || input.title.trim().is_empty() {
        return Err(ApiError::BadRequest(
            "short_code and title are required".to_string(),
        ));
    }
    let study = state
        .repository
        .create_study(
            input.organization_id,
            &input.short_code.trim().to_uppercase(),
            input.title.trim(),
        )
        .await?;
    Ok(Json(study))
}

async fn list_studies(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
) -> Result<Json<Vec<crate::models::Study>>, ApiError> {
    can_read(&user)?;
    Ok(Json(state.repository.list_studies().await?))
}

async fn transition_study_phase(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Path(study_id): Path<Uuid>,
    Json(input): Json<TransitionStudyPhaseRequest>,
) -> Result<Json<crate::models::Study>, ApiError> {
    can_admin(&user)?;
    let study = state.repository.get_study(study_id).await?;
    let readiness = state.repository.compute_readiness(study_id).await?;
    validate_phase_transition(&study.phase, &input.phase, &readiness)?;
    Ok(Json(
        state
            .repository
            .update_study_phase(study_id, input.phase)
            .await?,
    ))
}

async fn study_readiness(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Path(study_id): Path<Uuid>,
) -> Result<Json<StudyReadiness>, ApiError> {
    can_read(&user)?;
    Ok(Json(state.repository.compute_readiness(study_id).await?))
}

async fn create_site(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Json(input): Json<CreateSiteRequest>,
) -> Result<Json<crate::models::Site>, ApiError> {
    can_write(&user)?;
    if input.name.trim().is_empty() || input.principal_investigator.trim().is_empty() {
        return Err(ApiError::BadRequest(
            "name and principal_investigator are required".to_string(),
        ));
    }
    if let Some(study_id) = input.study_id {
        let study = state.repository.get_study(study_id).await?;
        if study.organization_id != input.organization_id {
            return Err(ApiError::BadRequest(
                "site organization_id must match study organization".to_string(),
            ));
        }
    }
    let site = state
        .repository
        .create_site(
            input.organization_id,
            input.study_id,
            input.name.trim(),
            input.principal_investigator.trim(),
        )
        .await?;
    Ok(Json(site))
}

async fn list_sites(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
) -> Result<Json<Vec<crate::models::Site>>, ApiError> {
    can_read(&user)?;
    Ok(Json(state.repository.list_sites().await?))
}

async fn mark_site_startup(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Path(site_id): Path<Uuid>,
    Json(input): Json<MarkSiteStartupRequest>,
) -> Result<Json<crate::models::Site>, ApiError> {
    can_write(&user)?;
    Ok(Json(
        state
            .repository
            .set_site_startup(site_id, input.startup_complete)
            .await?,
    ))
}

async fn enroll_patient(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Json(input): Json<crate::models::EnrollPatientRequest>,
) -> Result<Json<crate::models::Patient>, ApiError> {
    can_write(&user)?;
    if input.external_id.trim().is_empty() {
        return Err(ApiError::BadRequest("external_id is required".to_string()));
    }
    let study = state.repository.get_study(input.study_id).await?;
    if study.organization_id != input.organization_id {
        return Err(ApiError::BadRequest(
            "organization_id must match study organization".to_string(),
        ));
    }
    if let Some(site_id) = input.site_id {
        let site = state.repository.get_site(site_id).await?;
        if site.organization_id != input.organization_id {
            return Err(ApiError::BadRequest(
                "site organization_id mismatch".to_string(),
            ));
        }
    }
    Ok(Json(
        state
            .repository
            .create_patient(
                input.organization_id,
                input.study_id,
                input.site_id,
                input.external_id.trim(),
            )
            .await?,
    ))
}

async fn list_patients(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
) -> Result<Json<Vec<crate::models::Patient>>, ApiError> {
    can_read(&user)?;
    Ok(Json(state.repository.list_patients().await?))
}

async fn create_visit(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Json(input): Json<CreateVisitRequest>,
) -> Result<Json<crate::models::Visit>, ApiError> {
    can_write(&user)?;
    if input.visit_name.trim().is_empty() {
        return Err(ApiError::BadRequest("visit_name is required".to_string()));
    }
    let patient = state.repository.get_patient(input.patient_id).await?;
    if patient.study_id != input.study_id {
        return Err(ApiError::BadRequest(
            "visit study_id must match patient study".to_string(),
        ));
    }
    Ok(Json(
        state
            .repository
            .create_visit(
                input.study_id,
                input.patient_id,
                input.visit_name.trim(),
                input.scheduled_for,
            )
            .await?,
    ))
}

async fn list_visits(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
) -> Result<Json<Vec<crate::models::Visit>>, ApiError> {
    can_read(&user)?;
    Ok(Json(state.repository.list_visits().await?))
}

async fn create_crf_template(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Json(input): Json<CreateCrfTemplateRequest>,
) -> Result<Json<crate::models::CrfTemplate>, ApiError> {
    can_write(&user)?;
    if input.name.trim().is_empty() {
        return Err(ApiError::BadRequest("name is required".to_string()));
    }
    state.repository.get_study(input.study_id).await?;
    Ok(Json(
        state
            .repository
            .create_crf_template(input.study_id, input.name.trim())
            .await?,
    ))
}

async fn list_crf_templates(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
) -> Result<Json<Vec<crate::models::CrfTemplate>>, ApiError> {
    can_read(&user)?;
    Ok(Json(state.repository.list_crf_templates().await?))
}

async fn publish_crf_template(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Path(template_id): Path<Uuid>,
) -> Result<Json<crate::models::CrfTemplate>, ApiError> {
    can_write(&user)?;
    Ok(Json(
        state.repository.publish_crf_template(template_id).await?,
    ))
}

async fn create_crf_submission(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Json(input): Json<CreateCrfSubmissionRequest>,
) -> Result<Json<crate::models::CrfSubmission>, ApiError> {
    can_write(&user)?;
    let patient = state.repository.get_patient(input.patient_id).await?;
    if patient.study_id != input.study_id {
        return Err(ApiError::BadRequest(
            "submission study_id must match patient study".to_string(),
        ));
    }
    let visit = state.repository.get_visit(input.visit_id).await?;
    if visit.study_id != input.study_id || visit.patient_id != input.patient_id {
        return Err(ApiError::BadRequest(
            "visit must belong to same study/patient".to_string(),
        ));
    }
    let template = state.repository.get_crf_template(input.template_id).await?;
    if template.study_id != input.study_id || !template.published {
        return Err(ApiError::Conflict(
            "template must be published and belong to study".to_string(),
        ));
    }
    Ok(Json(
        state
            .repository
            .create_crf_submission(
                input.study_id,
                input.patient_id,
                input.visit_id,
                input.template_id,
            )
            .await?,
    ))
}

async fn list_crf_submissions(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
) -> Result<Json<Vec<crate::models::CrfSubmission>>, ApiError> {
    can_read(&user)?;
    Ok(Json(state.repository.list_crf_submissions().await?))
}

async fn lock_crf_submission(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Path(submission_id): Path<Uuid>,
) -> Result<Json<crate::models::CrfSubmission>, ApiError> {
    can_write(&user)?;
    Ok(Json(
        state.repository.lock_crf_submission(submission_id).await?,
    ))
}

async fn create_data_query(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Json(input): Json<CreateDataQueryRequest>,
) -> Result<Json<crate::models::DataQuery>, ApiError> {
    can_write(&user)?;
    if input.summary.trim().is_empty() {
        return Err(ApiError::BadRequest("summary is required".to_string()));
    }
    let submission = state
        .repository
        .get_crf_submission(input.submission_id)
        .await?;
    if submission.study_id != input.study_id {
        return Err(ApiError::BadRequest(
            "query study_id must match submission study".to_string(),
        ));
    }
    Ok(Json(
        state
            .repository
            .create_data_query(input.study_id, input.submission_id, input.summary.trim())
            .await?,
    ))
}

async fn list_data_queries(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
) -> Result<Json<Vec<crate::models::DataQuery>>, ApiError> {
    can_read(&user)?;
    Ok(Json(state.repository.list_data_queries().await?))
}

async fn close_data_query(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Path(query_id): Path<Uuid>,
) -> Result<Json<crate::models::DataQuery>, ApiError> {
    can_write(&user)?;
    Ok(Json(state.repository.close_data_query(query_id).await?))
}

async fn create_dua(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Json(input): Json<CreateDuaRequest>,
) -> Result<Json<crate::models::DuaAgreement>, ApiError> {
    can_write(&user)?;
    if input.counterparty.trim().is_empty() {
        return Err(ApiError::BadRequest("counterparty is required".to_string()));
    }
    Ok(Json(
        state
            .repository
            .create_dua(input.organization_id, input.counterparty.trim())
            .await?,
    ))
}

async fn list_duas(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
) -> Result<Json<Vec<crate::models::DuaAgreement>>, ApiError> {
    can_read(&user)?;
    Ok(Json(state.repository.list_duas().await?))
}

async fn activate_dua(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Path(dua_id): Path<Uuid>,
) -> Result<Json<crate::models::DuaAgreement>, ApiError> {
    can_write(&user)?;
    Ok(Json(state.repository.activate_dua(dua_id).await?))
}

async fn render_wizard_shell(Extension(user): Extension<AuthenticatedUser>) -> Html<String> {
    let body = format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>Virival Workflow Shell</title>
  <style>
    :root {{
      --cream: #E7E5DA;
      --sand: #C5B7AB;
      --forest: #283E28;
      --navy: #02182B;
      --orange: #F05708;
    }}
    body {{
      margin: 0;
      background: linear-gradient(180deg, #f6f4ee 0%, var(--cream) 100%);
      color: var(--navy);
      font-family: Inter, Arial, sans-serif;
    }}
    .page {{ max-width: 1160px; margin: 0 auto; padding: 1.2rem; }}
    .hero {{
      border: 1px solid rgba(2,24,43,0.15);
      border-radius: 16px;
      background: #fffdf8;
      padding: 1rem 1.2rem;
      box-shadow: 0 10px 22px rgba(2,24,43,0.08);
    }}
    .hero h1 {{ margin: 0; font-size: 1.4rem; }}
    .muted {{ color: #35516e; }}
    .chip {{
      display: inline-block;
      margin-top: 0.5rem;
      background: var(--navy);
      color: white;
      border-radius: 999px;
      padding: 0.2rem 0.6rem;
      font-size: 0.75rem;
      font-weight: 700;
    }}
    .grid {{
      margin-top: 1rem;
      display: grid;
      grid-template-columns: repeat(auto-fit, minmax(260px, 1fr));
      gap: 0.85rem;
    }}
    .card {{
      border: 1px solid rgba(2,24,43,0.12);
      border-left: 5px solid var(--orange);
      border-radius: 14px;
      background: #fffefb;
      padding: 0.85rem 0.9rem;
    }}
    .card h3 {{ margin: 0 0 0.35rem; font-size: 1rem; }}
    .card p {{ margin: 0; font-size: 0.85rem; color: #314c66; line-height: 1.4; }}
    .api {{
      margin-top: 1rem;
      border: 1px solid rgba(2,24,43,0.14);
      border-radius: 14px;
      padding: 0.85rem 0.95rem;
      background: white;
    }}
    code {{
      font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
      font-size: 0.8rem;
      color: #0f3554;
    }}
  </style>
</head>
<body>
  <div class="page">
    <section class="hero">
      <h1>Virival · Phase 2 Wizard Shell</h1>
      <p class="muted">Authenticated as <strong>{subject}</strong> with role <strong>{role}</strong>. This shell is now backed by PostgreSQL + repository layer + RBAC middleware skeleton.</p>
      <span class="chip">workflow-first architecture mode</span>
    </section>
    <section class="grid">
      <div class="card"><h3>1. Setup organization</h3><p>Create an organization via <code>POST /api/v1/organizations</code>.</p></div>
      <div class="card"><h3>2. Launch study</h3><p>Create study, publish CRF template, and attach startup-ready site.</p></div>
      <div class="card"><h3>3. Enroll & execute</h3><p>Enroll patient, create visit, submit/lock CRFs, track data queries.</p></div>
      <div class="card"><h3>4. Govern lifecycle</h3><p>Use readiness + gated phase transitions to advance and close safely.</p></div>
    </section>
    <section class="api">
      <strong>Auth headers (dev mode)</strong>
      <p class="muted">Send <code>x-virival-user</code> and optional <code>x-virival-role</code> (platform_admin, org_admin, investigator, site_coordinator, analyst, monitor).</p>
    </section>
  </div>
</body>
</html>"#,
        subject = escape_html(&user.subject),
        role = user.role.as_str()
    );
    Html(body)
}

fn escape_html(raw: &str) -> String {
    raw.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
