use axum::{
    extract::{Path, State},
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use uuid::Uuid;

use crate::{
    error::ApiError,
    models::{
        AgreementStatus, CreateCrfSubmissionRequest, CreateCrfTemplateRequest,
        CreateDataQueryRequest, CreateDuaRequest, CreateOrganizationRequest, CreateSiteRequest,
        CreateStudyRequest, CreateVisitRequest, CrfSubmission, CrfTemplate, DataQuery,
        DuaAgreement, EnrollPatientRequest, HealthResponse, MarkSiteStartupRequest, Organization,
        Patient, QueryStatus, Site, Study, StudyPhase, SubmissionStatus,
        TransitionStudyPhaseRequest, Visit, VisitStatus,
    },
    state::AppState,
    workflow::{compute_readiness, validate_phase_transition},
};

pub fn router(state: AppState, app_name: String) -> Router {
    Router::new()
        .route("/health", get(move || health(app_name.clone())))
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
        .with_state(state)
}

async fn health(app_name: String) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        service: app_name,
        timestamp_utc: Utc::now(),
    })
}

async fn create_organization(
    State(state): State<AppState>,
    Json(input): Json<CreateOrganizationRequest>,
) -> Result<Json<Organization>, ApiError> {
    if input.name.trim().is_empty() || input.workspace_slug.trim().is_empty() {
        return Err(ApiError::BadRequest(
            "name and workspace_slug are required".to_string(),
        ));
    }
    let mut store = state.store.write().await;
    if store.organizations.values().any(|org| {
        org.workspace_slug
            .eq_ignore_ascii_case(input.workspace_slug.trim())
    }) {
        return Err(ApiError::Conflict(
            "workspace_slug already exists".to_string(),
        ));
    }

    let organization = Organization {
        id: Uuid::new_v4(),
        name: input.name.trim().to_string(),
        workspace_slug: input.workspace_slug.trim().to_lowercase(),
        created_at: Utc::now(),
    };
    store
        .organizations
        .insert(organization.id, organization.clone());
    Ok(Json(organization))
}

async fn list_organizations(State(state): State<AppState>) -> Json<Vec<Organization>> {
    let store = state.store.read().await;
    let mut organizations = store.organizations.values().cloned().collect::<Vec<_>>();
    organizations.sort_by(|a, b| a.name.cmp(&b.name));
    Json(organizations)
}

async fn create_study(
    State(state): State<AppState>,
    Json(input): Json<CreateStudyRequest>,
) -> Result<Json<Study>, ApiError> {
    if input.short_code.trim().is_empty() || input.title.trim().is_empty() {
        return Err(ApiError::BadRequest(
            "short_code and title are required".to_string(),
        ));
    }
    let mut store = state.store.write().await;
    if !store.organizations.contains_key(&input.organization_id) {
        return Err(ApiError::NotFound(format!(
            "organization {}",
            input.organization_id
        )));
    }
    if store.studies.values().any(|study| {
        study.organization_id == input.organization_id
            && study
                .short_code
                .eq_ignore_ascii_case(input.short_code.trim())
    }) {
        return Err(ApiError::Conflict(
            "short_code already exists for this organization".to_string(),
        ));
    }

    let study = Study {
        id: Uuid::new_v4(),
        organization_id: input.organization_id,
        short_code: input.short_code.trim().to_uppercase(),
        title: input.title.trim().to_string(),
        phase: StudyPhase::PreStudy,
        created_at: Utc::now(),
    };
    store.studies.insert(study.id, study.clone());
    Ok(Json(study))
}

async fn list_studies(State(state): State<AppState>) -> Json<Vec<Study>> {
    let store = state.store.read().await;
    let mut studies = store.studies.values().cloned().collect::<Vec<_>>();
    studies.sort_by(|a, b| a.short_code.cmp(&b.short_code));
    Json(studies)
}

async fn transition_study_phase(
    State(state): State<AppState>,
    Path(study_id): Path<Uuid>,
    Json(input): Json<TransitionStudyPhaseRequest>,
) -> Result<Json<Study>, ApiError> {
    let mut store = state.store.write().await;
    let current_phase = store
        .studies
        .get(&study_id)
        .ok_or_else(|| ApiError::NotFound(format!("study {study_id}")))?
        .phase
        .clone();
    let readiness = compute_readiness(&store, study_id)?;
    validate_phase_transition(&current_phase, &input.phase, &readiness)?;

    let study = store
        .studies
        .get_mut(&study_id)
        .ok_or_else(|| ApiError::NotFound(format!("study {study_id}")))?;
    study.phase = input.phase;
    Ok(Json(study.clone()))
}

async fn study_readiness(
    State(state): State<AppState>,
    Path(study_id): Path<Uuid>,
) -> Result<Json<crate::models::StudyReadiness>, ApiError> {
    let store = state.store.read().await;
    let readiness = compute_readiness(&store, study_id)?;
    Ok(Json(readiness))
}

async fn create_site(
    State(state): State<AppState>,
    Json(input): Json<CreateSiteRequest>,
) -> Result<Json<Site>, ApiError> {
    if input.name.trim().is_empty() || input.principal_investigator.trim().is_empty() {
        return Err(ApiError::BadRequest(
            "name and principal_investigator are required".to_string(),
        ));
    }
    let mut store = state.store.write().await;
    if !store.organizations.contains_key(&input.organization_id) {
        return Err(ApiError::NotFound(format!(
            "organization {}",
            input.organization_id
        )));
    }
    if let Some(study_id) = input.study_id {
        let study = store
            .studies
            .get(&study_id)
            .ok_or_else(|| ApiError::NotFound(format!("study {study_id}")))?;
        if study.organization_id != input.organization_id {
            return Err(ApiError::BadRequest(
                "site organization_id must match study organization".to_string(),
            ));
        }
    }
    let site = Site {
        id: Uuid::new_v4(),
        organization_id: input.organization_id,
        study_id: input.study_id,
        name: input.name.trim().to_string(),
        principal_investigator: input.principal_investigator.trim().to_string(),
        startup_complete: false,
        created_at: Utc::now(),
    };
    store.sites.insert(site.id, site.clone());
    Ok(Json(site))
}

async fn list_sites(State(state): State<AppState>) -> Json<Vec<Site>> {
    let store = state.store.read().await;
    let mut sites = store.sites.values().cloned().collect::<Vec<_>>();
    sites.sort_by(|a, b| a.name.cmp(&b.name));
    Json(sites)
}

async fn mark_site_startup(
    State(state): State<AppState>,
    Path(site_id): Path<Uuid>,
    Json(input): Json<MarkSiteStartupRequest>,
) -> Result<Json<Site>, ApiError> {
    let mut store = state.store.write().await;
    let site = store
        .sites
        .get_mut(&site_id)
        .ok_or_else(|| ApiError::NotFound(format!("site {site_id}")))?;
    site.startup_complete = input.startup_complete;
    Ok(Json(site.clone()))
}

async fn enroll_patient(
    State(state): State<AppState>,
    Json(input): Json<EnrollPatientRequest>,
) -> Result<Json<Patient>, ApiError> {
    if input.external_id.trim().is_empty() {
        return Err(ApiError::BadRequest("external_id is required".to_string()));
    }
    let mut store = state.store.write().await;
    let study = store
        .studies
        .get(&input.study_id)
        .ok_or_else(|| ApiError::NotFound(format!("study {}", input.study_id)))?;
    if study.organization_id != input.organization_id {
        return Err(ApiError::BadRequest(
            "organization_id must match study organization".to_string(),
        ));
    }
    if let Some(site_id) = input.site_id {
        let site = store
            .sites
            .get(&site_id)
            .ok_or_else(|| ApiError::NotFound(format!("site {site_id}")))?;
        if site.organization_id != input.organization_id {
            return Err(ApiError::BadRequest(
                "site organization_id mismatch".to_string(),
            ));
        }
    }
    let patient = Patient {
        id: Uuid::new_v4(),
        organization_id: input.organization_id,
        study_id: input.study_id,
        site_id: input.site_id,
        external_id: input.external_id.trim().to_string(),
        enrolled_at: Utc::now(),
    };
    store.patients.insert(patient.id, patient.clone());
    Ok(Json(patient))
}

async fn list_patients(State(state): State<AppState>) -> Json<Vec<Patient>> {
    let store = state.store.read().await;
    let mut patients = store.patients.values().cloned().collect::<Vec<_>>();
    patients.sort_by(|a, b| a.external_id.cmp(&b.external_id));
    Json(patients)
}

async fn create_visit(
    State(state): State<AppState>,
    Json(input): Json<CreateVisitRequest>,
) -> Result<Json<Visit>, ApiError> {
    if input.visit_name.trim().is_empty() {
        return Err(ApiError::BadRequest("visit_name is required".to_string()));
    }
    let mut store = state.store.write().await;
    if !store.studies.contains_key(&input.study_id) {
        return Err(ApiError::NotFound(format!("study {}", input.study_id)));
    }
    let patient = store
        .patients
        .get(&input.patient_id)
        .ok_or_else(|| ApiError::NotFound(format!("patient {}", input.patient_id)))?;
    if patient.study_id != input.study_id {
        return Err(ApiError::BadRequest(
            "visit study_id must match patient study".to_string(),
        ));
    }

    let visit = Visit {
        id: Uuid::new_v4(),
        study_id: input.study_id,
        patient_id: input.patient_id,
        visit_name: input.visit_name.trim().to_string(),
        scheduled_for: input.scheduled_for,
        status: VisitStatus::Planned,
    };
    store.visits.insert(visit.id, visit.clone());
    Ok(Json(visit))
}

async fn list_visits(State(state): State<AppState>) -> Json<Vec<Visit>> {
    let store = state.store.read().await;
    let mut visits = store.visits.values().cloned().collect::<Vec<_>>();
    visits.sort_by(|a, b| a.scheduled_for.cmp(&b.scheduled_for));
    Json(visits)
}

async fn create_crf_template(
    State(state): State<AppState>,
    Json(input): Json<CreateCrfTemplateRequest>,
) -> Result<Json<CrfTemplate>, ApiError> {
    if input.name.trim().is_empty() {
        return Err(ApiError::BadRequest("name is required".to_string()));
    }
    let mut store = state.store.write().await;
    if !store.studies.contains_key(&input.study_id) {
        return Err(ApiError::NotFound(format!("study {}", input.study_id)));
    }
    let template = CrfTemplate {
        id: Uuid::new_v4(),
        study_id: input.study_id,
        name: input.name.trim().to_string(),
        published: false,
        created_at: Utc::now(),
    };
    store.crf_templates.insert(template.id, template.clone());
    Ok(Json(template))
}

async fn list_crf_templates(State(state): State<AppState>) -> Json<Vec<CrfTemplate>> {
    let store = state.store.read().await;
    let mut templates = store.crf_templates.values().cloned().collect::<Vec<_>>();
    templates.sort_by(|a, b| a.name.cmp(&b.name));
    Json(templates)
}

async fn publish_crf_template(
    State(state): State<AppState>,
    Path(template_id): Path<Uuid>,
) -> Result<Json<CrfTemplate>, ApiError> {
    let mut store = state.store.write().await;
    let template = store
        .crf_templates
        .get_mut(&template_id)
        .ok_or_else(|| ApiError::NotFound(format!("template {template_id}")))?;
    template.published = true;
    Ok(Json(template.clone()))
}

async fn create_crf_submission(
    State(state): State<AppState>,
    Json(input): Json<CreateCrfSubmissionRequest>,
) -> Result<Json<CrfSubmission>, ApiError> {
    let mut store = state.store.write().await;
    let patient = store
        .patients
        .get(&input.patient_id)
        .ok_or_else(|| ApiError::NotFound(format!("patient {}", input.patient_id)))?;
    if patient.study_id != input.study_id {
        return Err(ApiError::BadRequest(
            "submission study_id must match patient study".to_string(),
        ));
    }
    let visit = store
        .visits
        .get(&input.visit_id)
        .ok_or_else(|| ApiError::NotFound(format!("visit {}", input.visit_id)))?;
    if visit.study_id != input.study_id || visit.patient_id != input.patient_id {
        return Err(ApiError::BadRequest(
            "visit must belong to same study/patient".to_string(),
        ));
    }
    let template = store
        .crf_templates
        .get(&input.template_id)
        .ok_or_else(|| ApiError::NotFound(format!("template {}", input.template_id)))?;
    if template.study_id != input.study_id || !template.published {
        return Err(ApiError::Conflict(
            "template must be published and belong to study".to_string(),
        ));
    }
    let submission = CrfSubmission {
        id: Uuid::new_v4(),
        study_id: input.study_id,
        patient_id: input.patient_id,
        visit_id: input.visit_id,
        template_id: input.template_id,
        status: SubmissionStatus::Submitted,
        captured_at: Utc::now(),
    };
    store
        .crf_submissions
        .insert(submission.id, submission.clone());
    Ok(Json(submission))
}

async fn list_crf_submissions(State(state): State<AppState>) -> Json<Vec<CrfSubmission>> {
    let store = state.store.read().await;
    let mut submissions = store.crf_submissions.values().cloned().collect::<Vec<_>>();
    submissions.sort_by(|a, b| a.captured_at.cmp(&b.captured_at));
    Json(submissions)
}

async fn lock_crf_submission(
    State(state): State<AppState>,
    Path(submission_id): Path<Uuid>,
) -> Result<Json<CrfSubmission>, ApiError> {
    let mut store = state.store.write().await;
    let submission = store
        .crf_submissions
        .get_mut(&submission_id)
        .ok_or_else(|| ApiError::NotFound(format!("submission {submission_id}")))?;
    submission.status = SubmissionStatus::Locked;
    Ok(Json(submission.clone()))
}

async fn create_data_query(
    State(state): State<AppState>,
    Json(input): Json<CreateDataQueryRequest>,
) -> Result<Json<DataQuery>, ApiError> {
    if input.summary.trim().is_empty() {
        return Err(ApiError::BadRequest("summary is required".to_string()));
    }
    let mut store = state.store.write().await;
    let submission = store
        .crf_submissions
        .get(&input.submission_id)
        .ok_or_else(|| ApiError::NotFound(format!("submission {}", input.submission_id)))?;
    if submission.study_id != input.study_id {
        return Err(ApiError::BadRequest(
            "query study_id must match submission study".to_string(),
        ));
    }
    let query = DataQuery {
        id: Uuid::new_v4(),
        study_id: input.study_id,
        submission_id: input.submission_id,
        summary: input.summary.trim().to_string(),
        status: QueryStatus::Open,
        raised_at: Utc::now(),
    };
    store.data_queries.insert(query.id, query.clone());
    Ok(Json(query))
}

async fn list_data_queries(State(state): State<AppState>) -> Json<Vec<DataQuery>> {
    let store = state.store.read().await;
    let mut queries = store.data_queries.values().cloned().collect::<Vec<_>>();
    queries.sort_by(|a, b| a.raised_at.cmp(&b.raised_at));
    Json(queries)
}

async fn close_data_query(
    State(state): State<AppState>,
    Path(query_id): Path<Uuid>,
) -> Result<Json<DataQuery>, ApiError> {
    let mut store = state.store.write().await;
    let query = store
        .data_queries
        .get_mut(&query_id)
        .ok_or_else(|| ApiError::NotFound(format!("query {query_id}")))?;
    query.status = QueryStatus::Closed;
    Ok(Json(query.clone()))
}

async fn create_dua(
    State(state): State<AppState>,
    Json(input): Json<CreateDuaRequest>,
) -> Result<Json<DuaAgreement>, ApiError> {
    if input.counterparty.trim().is_empty() {
        return Err(ApiError::BadRequest("counterparty is required".to_string()));
    }
    let mut store = state.store.write().await;
    if !store.organizations.contains_key(&input.organization_id) {
        return Err(ApiError::NotFound(format!(
            "organization {}",
            input.organization_id
        )));
    }
    let agreement = DuaAgreement {
        id: Uuid::new_v4(),
        organization_id: input.organization_id,
        counterparty: input.counterparty.trim().to_string(),
        status: AgreementStatus::PendingSignatures,
        created_at: Utc::now(),
    };
    store.duas.insert(agreement.id, agreement.clone());
    Ok(Json(agreement))
}

async fn list_duas(State(state): State<AppState>) -> Json<Vec<DuaAgreement>> {
    let store = state.store.read().await;
    let mut agreements = store.duas.values().cloned().collect::<Vec<_>>();
    agreements.sort_by(|a, b| a.created_at.cmp(&b.created_at));
    Json(agreements)
}

async fn activate_dua(
    State(state): State<AppState>,
    Path(dua_id): Path<Uuid>,
) -> Result<Json<DuaAgreement>, ApiError> {
    let mut store = state.store.write().await;
    let agreement = store
        .duas
        .get_mut(&dua_id)
        .ok_or_else(|| ApiError::NotFound(format!("dua {dua_id}")))?;
    agreement.status = AgreementStatus::Active;
    Ok(Json(agreement.clone()))
}
