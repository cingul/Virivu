use axum::{
    extract::{Form, Path, Query, State},
    middleware::from_fn_with_state,
    response::{Html, Redirect},
    routing::{get, post},
    Extension, Json, Router,
};
use chrono::Utc;
use serde::Deserialize;
use uuid::Uuid;

use crate::{
    auth::{require_any_role, require_auth},
    error::ApiError,
    models::{
        AppRole, AuthenticatedUser, CreateCrfSubmissionRequest, CreateCrfTemplateRequest,
        CreateDataQueryRequest, CreateDuaRequest, CreateMembershipRequest,
        CreateOrganizationRequest, CreateSiteRequest, CreateStudyRequest, CreateVisitRequest,
        HealthResponse, MarkSiteStartupRequest, StudyReadiness, TransitionStudyPhaseRequest,
    },
    state::AppState,
    workflow::validate_phase_transition,
};

pub fn router(state: AppState, app_name: String) -> Router {
    let public_router = Router::new().route("/health", get(move || health(app_name.clone())));

    let protected_router = Router::new()
        .route("/", get(render_wizard_shell))
        .route("/ui", get(render_wizard_shell))
        .route("/ui/admin/memberships", get(render_membership_admin))
        .route("/ui/admin/memberships", post(submit_membership_admin))
        .route("/ui/admin/audit", get(render_audit_console))
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
        .route("/api/v1/admin/memberships", post(create_membership))
        .route(
            "/api/v1/admin/organizations/{organization_id}/memberships",
            get(list_organization_memberships),
        )
        .route("/api/v1/admin/audit-logs", get(list_audit_logs))
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

fn can_platform_admin(user: &AuthenticatedUser) -> Result<(), ApiError> {
    require_any_role(user, &[AppRole::PlatformAdmin])
}

#[derive(Debug, Deserialize)]
struct MembershipAdminQuery {
    organization_id: Option<Uuid>,
    notice: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MembershipAdminForm {
    organization_id: Uuid,
    subject: String,
    email: Option<String>,
    display_name: Option<String>,
    role: AppRole,
}

#[derive(Debug, Deserialize)]
struct AuditConsoleQuery {
    limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct AuditApiQuery {
    limit: Option<i64>,
}

async fn create_membership(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Json(input): Json<CreateMembershipRequest>,
) -> Result<Json<crate::models::OrganizationMembership>, ApiError> {
    can_admin(&user)?;
    if input.subject.trim().is_empty() {
        return Err(ApiError::BadRequest(
            "subject is required to create membership".to_string(),
        ));
    }
    state
        .repository
        .get_organization(input.organization_id)
        .await?;
    let managed_user = state
        .repository
        .upsert_user(
            input.subject.trim(),
            input.email.as_deref(),
            input.display_name.as_deref(),
            AppRole::Investigator,
        )
        .await?;
    let membership = state
        .repository
        .upsert_organization_membership(managed_user.id, input.organization_id, input.role)
        .await?;
    Ok(Json(membership))
}

async fn list_organization_memberships(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Path(organization_id): Path<Uuid>,
) -> Result<Json<Vec<crate::models::OrganizationMembership>>, ApiError> {
    can_admin(&user)?;
    Ok(Json(
        state
            .repository
            .list_organization_memberships(organization_id)
            .await?,
    ))
}

async fn list_audit_logs(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Query(query): Query<AuditApiQuery>,
) -> Result<Json<Vec<crate::models::AuditLogRecord>>, ApiError> {
    can_platform_admin(&user)?;
    let limit = query.limit.unwrap_or(50);
    Ok(Json(state.repository.list_recent_audit_logs(limit).await?))
}

async fn render_membership_admin(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Query(query): Query<MembershipAdminQuery>,
) -> Result<Html<String>, ApiError> {
    can_platform_admin(&user)?;
    let organizations = state.repository.list_organizations().await?;
    let selected_org = query
        .organization_id
        .or_else(|| organizations.first().map(|org| org.id));
    let memberships = if let Some(organization_id) = selected_org {
        state
            .repository
            .list_organization_memberships(organization_id)
            .await?
    } else {
        vec![]
    };
    let org_options = organizations
        .iter()
        .map(|org| {
            let selected = if Some(org.id) == selected_org {
                "selected"
            } else {
                ""
            };
            format!(
                r#"<option value="{id}" {selected}>{name} ({slug})</option>"#,
                id = org.id,
                selected = selected,
                name = escape_html(&org.name),
                slug = escape_html(&org.workspace_slug)
            )
        })
        .collect::<String>();
    let membership_rows = if memberships.is_empty() {
        r#"<tr><td colspan="4" style="padding:0.8rem;color:#475569;">No memberships yet for selected organization.</td></tr>"#.to_string()
    } else {
        memberships
            .iter()
            .map(|membership| {
                format!(
                    r#"<tr>
  <td style="padding:0.55rem 0.5rem;">{user_id}</td>
  <td style="padding:0.55rem 0.5rem;">{role}</td>
  <td style="padding:0.55rem 0.5rem;">{created_at}</td>
  <td style="padding:0.55rem 0.5rem;">{org_id}</td>
</tr>"#,
                    user_id = membership.user_id,
                    role = membership.role.as_str(),
                    created_at = membership.created_at,
                    org_id = membership.organization_id,
                )
            })
            .collect::<String>()
    };
    let selected_org_value = selected_org
        .map(|org_id| org_id.to_string())
        .unwrap_or_else(String::new);
    let notice_html = query
        .notice
        .as_ref()
        .map(|notice| {
            format!(
                r#"<div style="margin-bottom:0.65rem;padding:0.55rem 0.65rem;border:1px solid #c5b7ab;border-radius:8px;background:#fffaf0;color:#1e3a56;">{}</div>"#,
                escape_html(notice)
            )
        })
        .unwrap_or_default();

    Ok(Html(format!(
        r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8" /><meta name="viewport" content="width=device-width, initial-scale=1" />
<title>Virival Membership Admin</title>
<style>
body {{ margin:0; font-family: Inter, Arial, sans-serif; background:#f7f4ee; color:#02182b; }}
.page {{ max-width:1080px; margin:0 auto; padding:1rem; }}
.card {{ background:white; border:1px solid #dbe3ea; border-radius:14px; padding:1rem; margin-bottom:0.9rem; }}
label {{ display:block; font-weight:700; margin:0.45rem 0 0.2rem; }}
input,select {{ width:min(100%,620px); padding:0.55rem; border-radius:10px; border:1px solid #cbd5e1; }}
button {{ margin-top:0.6rem; background:#02182b; color:white; border:none; border-radius:10px; padding:0.55rem 0.9rem; font-weight:700; cursor:pointer; }}
table {{ width:100%; border-collapse:collapse; }}
th {{ text-align:left; border-bottom:1px solid #e2e8f0; padding:0.55rem 0.5rem; font-size:0.82rem; color:#334155; }}
</style></head><body>
<div class="page">
  <div class="card">
    <h1 style="margin:0;">Virival Membership Admin</h1>
    <p style="margin:0.35rem 0 0;color:#35516e;">Persist org membership roles used by scoped RBAC resolution.</p>
  </div>
  <div class="card">
    {notice_html}
    <form method="post" action="/ui/admin/memberships">
      <label>Organization</label>
      <select name="organization_id" required>{org_options}</select>
      <label>User subject</label>
      <input name="subject" placeholder="user@cingulum.org" value="" required />
      <label>Email (optional)</label>
      <input name="email" placeholder="user@cingulum.org" />
      <label>Display name (optional)</label>
      <input name="display_name" placeholder="User Name" />
      <label>Role</label>
      <select name="role" required>
        <option value="org_admin">org_admin</option>
        <option value="site_coordinator">site_coordinator</option>
        <option value="investigator">investigator</option>
        <option value="analyst">analyst</option>
        <option value="monitor">monitor</option>
      </select>
      <button type="submit">Upsert Membership</button>
    </form>
  </div>
  <div class="card">
    <h3 style="margin:0 0 0.5rem;">Selected organization memberships</h3>
    <p style="margin:0 0 0.6rem;color:#4b5563;font-size:0.85rem;">Organization: <code>{selected_org_value}</code></p>
    <table>
      <thead><tr><th>User ID</th><th>Role</th><th>Created</th><th>Organization</th></tr></thead>
      <tbody>{membership_rows}</tbody>
    </table>
  </div>
  <div class="card"><a href="/ui" style="font-weight:700;color:#02182b;">← Back to workflow shell</a> &nbsp;|&nbsp; <a href="/ui/admin/audit" style="font-weight:700;color:#02182b;">Open audit console →</a></div>
</div>
</body></html>"#,
        notice_html = notice_html,
        org_options = org_options,
        membership_rows = membership_rows,
        selected_org_value = escape_html(&selected_org_value),
    )))
}

async fn submit_membership_admin(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Form(form): Form<MembershipAdminForm>,
) -> Result<Redirect, ApiError> {
    can_platform_admin(&user)?;
    if form.subject.trim().is_empty() {
        return Err(ApiError::BadRequest(
            "subject is required to create membership".to_string(),
        ));
    }
    state
        .repository
        .get_organization(form.organization_id)
        .await?;
    let managed_user = state
        .repository
        .upsert_user(
            form.subject.trim(),
            form.email.as_deref(),
            form.display_name.as_deref(),
            AppRole::Investigator,
        )
        .await?;
    state
        .repository
        .upsert_organization_membership(managed_user.id, form.organization_id, form.role)
        .await?;
    let notice = url_encode("Membership updated successfully");
    Ok(Redirect::to(&format!(
        "/ui/admin/memberships?organization_id={}&notice={}",
        form.organization_id, notice
    )))
}

async fn render_audit_console(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Query(query): Query<AuditConsoleQuery>,
) -> Result<Html<String>, ApiError> {
    can_platform_admin(&user)?;
    let limit = query.limit.unwrap_or(50);
    let audit_logs = state.repository.list_recent_audit_logs(limit).await?;
    let rows_html = if audit_logs.is_empty() {
        r#"<tr><td colspan="8" style="padding:0.8rem;color:#475569;">No audit events captured yet.</td></tr>"#.to_string()
    } else {
        audit_logs
            .iter()
            .map(|entry| {
                format!(
                    r#"<tr>
  <td style="padding:0.45rem;">{time}</td>
  <td style="padding:0.45rem;">{subject}</td>
  <td style="padding:0.45rem;">{role}</td>
  <td style="padding:0.45rem;">{method}</td>
  <td style="padding:0.45rem;">{path}</td>
  <td style="padding:0.45rem;">{action}</td>
  <td style="padding:0.45rem;">{resource}</td>
  <td style="padding:0.45rem;">{status}</td>
</tr>"#,
                    time = entry.happened_at,
                    subject = escape_html(&entry.actor_subject),
                    role = entry
                        .actor_role
                        .as_ref()
                        .map(|role| role.as_str())
                        .unwrap_or("-"),
                    method = escape_html(&entry.method),
                    path = escape_html(&entry.path),
                    action = escape_html(entry.action.as_deref().unwrap_or("-")),
                    resource = escape_html(
                        &entry
                            .resource_type
                            .as_ref()
                            .map(|rt| format!(
                                "{}:{}",
                                rt,
                                entry
                                    .resource_id
                                    .map(|id| id.to_string())
                                    .unwrap_or_else(|| "-".to_string())
                            ))
                            .unwrap_or_else(|| "-".to_string()),
                    ),
                    status = entry.status_code,
                )
            })
            .collect::<String>()
    };
    Ok(Html(format!(
        r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8" /><meta name="viewport" content="width=device-width, initial-scale=1" />
<title>Virival Audit Console</title>
<style>
body {{ margin:0; font-family: Inter, Arial, sans-serif; background:#f7f4ee; color:#02182b; }}
.page {{ max-width:1220px; margin:0 auto; padding:1rem; }}
.card {{ background:white; border:1px solid #dbe3ea; border-radius:14px; padding:1rem; margin-bottom:0.9rem; }}
table {{ width:100%; border-collapse:collapse; font-size:0.82rem; }}
th,td {{ border-bottom:1px solid #eef2f7; text-align:left; vertical-align:top; }}
th {{ padding:0.55rem 0.45rem; color:#334155; }}
</style></head><body>
<div class="page">
  <div class="card">
    <h1 style="margin:0;">Virival Audit Console</h1>
    <p style="margin:0.35rem 0 0;color:#35516e;">Showing last <strong>{limit}</strong> protected request events.</p>
  </div>
  <div class="card">
    <table>
      <thead><tr><th>Time</th><th>Actor</th><th>Role</th><th>Method</th><th>Path</th><th>Action</th><th>Resource</th><th>Status</th></tr></thead>
      <tbody>{rows_html}</tbody>
    </table>
  </div>
  <div class="card"><a href="/ui/admin/memberships" style="font-weight:700;color:#02182b;">← Membership admin</a> &nbsp;|&nbsp; <a href="/ui" style="font-weight:700;color:#02182b;">Workflow shell</a></div>
</div>
</body></html>"#,
        limit = limit,
        rows_html = rows_html
    )))
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
    let email = user
        .email
        .as_deref()
        .map(escape_html)
        .unwrap_or_else(|| "not provided".to_string());
    let org_scope = user
        .organization_scope
        .map(|org_id| org_id.to_string())
        .unwrap_or_else(|| "none".to_string());
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
      <h1>Virival · Phase 3 Wizard Shell</h1>
      <p class="muted">Authenticated as <strong>{subject}</strong> (<strong>{email}</strong>) with role <strong>{role}</strong>. Source: <strong>{auth_source}</strong>. Organization scope: <strong>{org_scope}</strong>.</p>
      <span class="chip">workflow-first architecture mode</span>
    </section>
    <section class="grid">
      <div class="card"><h3>1. Setup organization</h3><p>Create an organization via <code>POST /api/v1/organizations</code>.</p></div>
      <div class="card"><h3>2. Assign membership</h3><p>Use <code>POST /api/v1/admin/memberships</code> to persist org roles and then pass <code>x-virival-organization-id</code>.</p></div>
      <div class="card"><h3>3. Launch study</h3><p>Create study, publish CRF template, and attach startup-ready site.</p></div>
      <div class="card"><h3>4. Enroll & execute</h3><p>Enroll patient, create visit, submit/lock CRFs, track data queries.</p></div>
      <div class="card"><h3>5. Govern lifecycle</h3><p>Use readiness + gated phase transitions to advance and close safely.</p></div>
    </section>
    <section class="api">
      <strong>Auth options</strong>
      <p class="muted">Use Google OIDC bearer token in <code>Authorization: Bearer ...</code> or dev headers <code>x-virival-user</code>, <code>x-virival-role</code>, and optional <code>x-virival-organization-id</code>.</p>
      <p class="muted" style="margin-top:0.4rem;"><a href="/ui/admin/memberships" style="font-weight:700;color:#02182b;">Membership admin →</a> &nbsp;|&nbsp; <a href="/ui/admin/audit" style="font-weight:700;color:#02182b;">Audit console →</a></p>
    </section>
  </div>
</body>
</html>"#,
        subject = escape_html(&user.subject),
        email = email,
        role = user.role.as_str(),
        auth_source = escape_html(&user.auth_source),
        org_scope = escape_html(&org_scope)
    );
    Html(body)
}

fn escape_html(raw: &str) -> String {
    raw.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn url_encode(raw: &str) -> String {
    raw.bytes()
        .flat_map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                vec![byte as char]
            }
            b' ' => vec!['+'],
            _ => format!("%{:02X}", byte).chars().collect::<Vec<_>>(),
        })
        .collect()
}
