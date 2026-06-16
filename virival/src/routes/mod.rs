use axum::{
    body::Body,
    extract::{Form, Path, Query, State},
    http::{
        header::{CONTENT_DISPOSITION, CONTENT_TYPE},
        HeaderValue,
    },
    middleware::from_fn_with_state,
    response::{Html, Redirect, Response},
    routing::{get, post},
    Extension, Json, Router,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use uuid::Uuid;

use crate::{
    auth::{require_any_role, require_auth},
    error::ApiError,
    models::{
        AppRole, AuthenticatedUser, CompleteCloseoutChecklistItemRequest,
        CreateCloseoutChecklistItemRequest, CreateCrfSubmissionRequest, CreateCrfTemplateRequest,
        CreateCrfTemplateVersionRequest, CreateDataQueryCommentRequest, CreateDataQueryRequest,
        CreateDuaRequest, CreateDuaSignatureRequest, CreateMembershipRequest,
        CreateOrganizationRequest, CreateReminderJobRequest, CreateSiteRequest, CreateStudyRequest,
        CreateVisitRequest, CreateVisitScheduleTemplateRequest, HealthResponse,
        MarkSiteStartupRequest, ProcessReminderJobsRequest, ProcessReminderJobsResponse,
        RespondDataQueryRequest, StudyPhase, StudyReadiness, TransitionStudyPhaseRequest,
    },
    state::AppState,
    workflow::validate_phase_transition,
};

pub fn router(state: AppState, app_name: String) -> Router {
    let public_router = Router::new().route("/health", get(move || health(app_name.clone())));

    let protected_router = Router::new()
        .route("/", get(render_wizard_shell))
        .route("/ui", get(render_wizard_shell))
        .route("/ui/workbench", get(render_workbench))
        .route(
            "/ui/workbench/organizations",
            post(submit_workbench_organization),
        )
        .route("/ui/workbench/studies", post(submit_workbench_study))
        .route("/ui/workbench/sites", post(submit_workbench_site))
        .route("/ui/workbench/duas", post(submit_workbench_dua))
        .route(
            "/ui/workbench/dua-signatures",
            post(submit_workbench_dua_signature),
        )
        .route(
            "/ui/workbench/crf-design",
            post(submit_workbench_crf_design),
        )
        .route(
            "/ui/workbench/visit-schedules",
            post(submit_workbench_visit_schedule),
        )
        .route("/ui/workbench/patients", post(submit_workbench_patient))
        .route("/ui/workbench/visits", post(submit_workbench_visit))
        .route(
            "/ui/workbench/submissions",
            post(submit_workbench_submission),
        )
        .route("/ui/workbench/queries", post(submit_workbench_query))
        .route(
            "/ui/workbench/closeout-items",
            post(submit_workbench_closeout_item),
        )
        .route("/ui/workbench/reminders", post(submit_workbench_reminder))
        .route(
            "/ui/workbench/reminders/process",
            post(submit_workbench_process_reminders),
        )
        .route(
            "/ui/workbench/studies/{study_id}/phase",
            post(submit_workbench_phase_transition),
        )
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
            "/api/v1/crf-templates/{template_id}/versions",
            get(list_crf_template_versions).post(create_crf_template_version),
        )
        .route(
            "/api/v1/crf-templates/{template_id}/publish",
            post(publish_crf_template),
        )
        .route(
            "/api/v1/crf-template-versions/{version_id}/publish",
            post(publish_crf_template_version),
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
            "/api/v1/data-queries/{query_id}/comments",
            get(list_data_query_comments).post(create_data_query_comment),
        )
        .route(
            "/api/v1/data-queries/{query_id}/respond",
            post(respond_data_query),
        )
        .route(
            "/api/v1/data-queries/{query_id}/close",
            post(close_data_query),
        )
        .route(
            "/api/v1/studies/{study_id}/visit-schedule-templates",
            get(list_visit_schedule_templates).post(create_visit_schedule_template),
        )
        .route(
            "/api/v1/studies/{study_id}/closeout-checklist",
            get(list_closeout_checklist_items).post(create_closeout_checklist_item),
        )
        .route(
            "/api/v1/closeout-checklist/{item_id}/complete",
            post(complete_closeout_checklist_item),
        )
        .route("/api/v1/duas", get(list_duas).post(create_dua))
        .route("/api/v1/duas/{dua_id}/activate", post(activate_dua))
        .route(
            "/api/v1/duas/{dua_id}/signatures",
            get(list_dua_signatures).post(create_dua_signature),
        )
        .route("/api/v1/duas/{dua_id}/pdf", get(download_dua_pdf))
        .route("/api/v1/admin/memberships", post(create_membership))
        .route(
            "/api/v1/admin/organizations/{organization_id}/memberships",
            get(list_organization_memberships),
        )
        .route("/api/v1/admin/audit-logs", get(list_audit_logs))
        .route(
            "/api/v1/admin/reminder-jobs",
            get(list_reminder_jobs).post(create_reminder_job),
        )
        .route(
            "/api/v1/admin/reminder-jobs/process",
            post(process_reminder_jobs),
        )
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

#[derive(Debug, Deserialize)]
struct ReminderJobsQuery {
    limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct WorkbenchQuery {
    organization_id: Option<Uuid>,
    study_id: Option<Uuid>,
    tab: Option<String>,
    notice: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WorkbenchOrganizationForm {
    name: String,
    workspace_slug: String,
}

#[derive(Debug, Deserialize)]
struct WorkbenchStudyForm {
    organization_id: Uuid,
    short_code: String,
    title: String,
}

#[derive(Debug, Deserialize)]
struct WorkbenchSiteForm {
    organization_id: Uuid,
    study_id: Uuid,
    name: String,
    principal_investigator: String,
    startup_complete: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WorkbenchDuaForm {
    organization_id: Uuid,
    counterparty: String,
}

#[derive(Debug, Deserialize)]
struct WorkbenchDuaSignatureForm {
    dua_id: Uuid,
    signer_name: String,
    signer_email: String,
    signer_role: String,
    signature_text: String,
}

#[derive(Debug, Deserialize)]
struct WorkbenchCrfDesignForm {
    study_id: Uuid,
    template_name: String,
    schema_json: String,
    publish_version: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WorkbenchVisitScheduleForm {
    study_id: Uuid,
    name: String,
    day_offset: i32,
    window_before_days: i32,
    window_after_days: i32,
}

#[derive(Debug, Deserialize)]
struct WorkbenchPatientForm {
    organization_id: Uuid,
    study_id: Uuid,
    site_id: Option<Uuid>,
    external_id: String,
}

#[derive(Debug, Deserialize)]
struct WorkbenchVisitForm {
    study_id: Uuid,
    patient_id: Uuid,
    visit_name: String,
    scheduled_for: String,
}

#[derive(Debug, Deserialize)]
struct WorkbenchSubmissionForm {
    study_id: Uuid,
    patient_id: Uuid,
    visit_id: Uuid,
    template_id: Uuid,
    lock_submission: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WorkbenchQueryForm {
    study_id: Uuid,
    action: String,
    submission_id: Option<Uuid>,
    summary: Option<String>,
    query_id: Option<Uuid>,
    comment_text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WorkbenchCloseoutItemForm {
    study_id: Uuid,
    action: String,
    item_key: Option<String>,
    item_label: Option<String>,
    is_required: Option<String>,
    item_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
struct WorkbenchReminderForm {
    organization_id: Uuid,
    study_id: Option<Uuid>,
    patient_id: Option<Uuid>,
    visit_id: Option<Uuid>,
    channel: String,
    recipient: String,
    message: String,
    scheduled_for: String,
}

#[derive(Debug, Deserialize)]
struct WorkbenchReminderProcessForm {
    organization_id: Uuid,
    study_id: Option<Uuid>,
    limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct WorkbenchPhaseTransitionForm {
    phase: StudyPhase,
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

async fn create_crf_template_version(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Path(template_id): Path<Uuid>,
    Json(input): Json<CreateCrfTemplateVersionRequest>,
) -> Result<Json<crate::models::CrfTemplateVersion>, ApiError> {
    can_write(&user)?;
    state.repository.get_crf_template(template_id).await?;
    Ok(Json(
        state
            .repository
            .create_crf_template_version(template_id, &input.schema_json)
            .await?,
    ))
}

async fn list_crf_template_versions(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Path(template_id): Path<Uuid>,
) -> Result<Json<Vec<crate::models::CrfTemplateVersion>>, ApiError> {
    can_read(&user)?;
    state.repository.get_crf_template(template_id).await?;
    Ok(Json(
        state
            .repository
            .list_crf_template_versions(template_id)
            .await?,
    ))
}

async fn publish_crf_template_version(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Path(version_id): Path<Uuid>,
) -> Result<Json<crate::models::CrfTemplateVersion>, ApiError> {
    can_write(&user)?;
    Ok(Json(
        state
            .repository
            .publish_crf_template_version(version_id)
            .await?,
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
    if template.study_id != input.study_id {
        return Err(ApiError::Conflict(
            "template must belong to study".to_string(),
        ));
    }
    if !template.published {
        let versions = state
            .repository
            .list_crf_template_versions(template.id)
            .await?;
        let has_published_version = versions.iter().any(|version| version.is_published);
        if !has_published_version {
            return Err(ApiError::Conflict(
                "template must be published or have a published version".to_string(),
            ));
        }
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

async fn create_data_query_comment(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Path(query_id): Path<Uuid>,
    Json(input): Json<CreateDataQueryCommentRequest>,
) -> Result<Json<crate::models::DataQueryComment>, ApiError> {
    can_write(&user)?;
    if input.comment_text.trim().is_empty() {
        return Err(ApiError::BadRequest("comment_text is required".to_string()));
    }
    Ok(Json(
        state
            .repository
            .create_data_query_comment(query_id, Some(user.user_id), input.comment_text.trim())
            .await?,
    ))
}

async fn list_data_query_comments(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Path(query_id): Path<Uuid>,
) -> Result<Json<Vec<crate::models::DataQueryComment>>, ApiError> {
    can_read(&user)?;
    Ok(Json(
        state.repository.list_data_query_comments(query_id).await?,
    ))
}

async fn respond_data_query(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Path(query_id): Path<Uuid>,
    Json(input): Json<RespondDataQueryRequest>,
) -> Result<Json<crate::models::DataQuery>, ApiError> {
    can_write(&user)?;
    let response = state.repository.respond_data_query(query_id).await?;
    if let Some(comment_text) = input.comment_text.as_deref() {
        if !comment_text.trim().is_empty() {
            state
                .repository
                .create_data_query_comment(query_id, Some(user.user_id), comment_text.trim())
                .await?;
        }
    }
    Ok(Json(response))
}

async fn close_data_query(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Path(query_id): Path<Uuid>,
) -> Result<Json<crate::models::DataQuery>, ApiError> {
    can_write(&user)?;
    Ok(Json(state.repository.close_data_query(query_id).await?))
}

async fn create_visit_schedule_template(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Path(study_id): Path<Uuid>,
    Json(input): Json<CreateVisitScheduleTemplateRequest>,
) -> Result<Json<crate::models::VisitScheduleTemplate>, ApiError> {
    can_write(&user)?;
    if input.name.trim().is_empty() {
        return Err(ApiError::BadRequest(
            "name is required for visit schedule template".to_string(),
        ));
    }
    if input.window_before_days < 0 || input.window_after_days < 0 {
        return Err(ApiError::BadRequest(
            "visit windows cannot be negative".to_string(),
        ));
    }
    state.repository.get_study(study_id).await?;
    Ok(Json(
        state
            .repository
            .create_visit_schedule_template(
                study_id,
                input.name.trim(),
                input.day_offset,
                input.window_before_days,
                input.window_after_days,
            )
            .await?,
    ))
}

async fn list_visit_schedule_templates(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Path(study_id): Path<Uuid>,
) -> Result<Json<Vec<crate::models::VisitScheduleTemplate>>, ApiError> {
    can_read(&user)?;
    state.repository.get_study(study_id).await?;
    Ok(Json(
        state
            .repository
            .list_visit_schedule_templates(study_id)
            .await?,
    ))
}

async fn create_closeout_checklist_item(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Path(study_id): Path<Uuid>,
    Json(input): Json<CreateCloseoutChecklistItemRequest>,
) -> Result<Json<crate::models::CloseoutChecklistItem>, ApiError> {
    can_admin(&user)?;
    if input.item_key.trim().is_empty() || input.item_label.trim().is_empty() {
        return Err(ApiError::BadRequest(
            "item_key and item_label are required".to_string(),
        ));
    }
    state.repository.get_study(study_id).await?;
    Ok(Json(
        state
            .repository
            .create_closeout_checklist_item(
                study_id,
                input.item_key.trim(),
                input.item_label.trim(),
                input.is_required,
            )
            .await?,
    ))
}

async fn list_closeout_checklist_items(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Path(study_id): Path<Uuid>,
) -> Result<Json<Vec<crate::models::CloseoutChecklistItem>>, ApiError> {
    can_read(&user)?;
    state.repository.get_study(study_id).await?;
    Ok(Json(
        state
            .repository
            .list_closeout_checklist_items(study_id)
            .await?,
    ))
}

async fn complete_closeout_checklist_item(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Path(item_id): Path<Uuid>,
    Json(input): Json<CompleteCloseoutChecklistItemRequest>,
) -> Result<Json<crate::models::CloseoutChecklistItem>, ApiError> {
    can_write(&user)?;
    let completed_by_user_id = if input.is_complete {
        Some(user.user_id)
    } else {
        None
    };
    Ok(Json(
        state
            .repository
            .set_closeout_checklist_item_completion(
                item_id,
                input.is_complete,
                completed_by_user_id,
            )
            .await?,
    ))
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

async fn create_dua_signature(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Path(dua_id): Path<Uuid>,
    Json(input): Json<CreateDuaSignatureRequest>,
) -> Result<Json<crate::models::DuaSignature>, ApiError> {
    can_write(&user)?;
    if input.signer_name.trim().is_empty()
        || input.signer_email.trim().is_empty()
        || input.signer_role.trim().is_empty()
        || input.signature_text.trim().is_empty()
    {
        return Err(ApiError::BadRequest(
            "signer_name, signer_email, signer_role, and signature_text are required".to_string(),
        ));
    }
    state.repository.get_dua(dua_id).await?;
    Ok(Json(
        state
            .repository
            .create_dua_signature(
                dua_id,
                input.signer_name.trim(),
                input.signer_email.trim(),
                input.signer_role.trim(),
                input.signature_text.trim(),
            )
            .await?,
    ))
}

async fn list_dua_signatures(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Path(dua_id): Path<Uuid>,
) -> Result<Json<Vec<crate::models::DuaSignature>>, ApiError> {
    can_read(&user)?;
    state.repository.get_dua(dua_id).await?;
    Ok(Json(state.repository.list_dua_signatures(dua_id).await?))
}

async fn download_dua_pdf(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Path(dua_id): Path<Uuid>,
) -> Result<Response, ApiError> {
    can_read(&user)?;
    let dua = state.repository.get_dua(dua_id).await?;
    let organization = state
        .repository
        .get_organization(dua.organization_id)
        .await?;
    let signatures = state.repository.list_dua_signatures(dua_id).await?;
    let content = build_dua_pdf_bytes(
        &organization.name,
        &dua.counterparty,
        &dua.status,
        &signatures,
    );
    let filename = format!("virival-dua-{}.pdf", dua.id);
    let mut response = Response::new(Body::from(content));
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("application/pdf"));
    let content_disposition = format!("attachment; filename=\"{filename}\"");
    response.headers_mut().insert(
        CONTENT_DISPOSITION,
        HeaderValue::from_str(&content_disposition).map_err(|err| {
            ApiError::Internal(format!("invalid content disposition header: {err}"))
        })?,
    );
    Ok(response)
}

async fn create_reminder_job(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Json(input): Json<CreateReminderJobRequest>,
) -> Result<Json<crate::models::ReminderJob>, ApiError> {
    can_admin(&user)?;
    if input.channel.trim().is_empty()
        || input.recipient.trim().is_empty()
        || input.message.trim().is_empty()
    {
        return Err(ApiError::BadRequest(
            "channel, recipient, and message are required".to_string(),
        ));
    }
    state
        .repository
        .get_organization(input.organization_id)
        .await?;
    Ok(Json(
        state
            .repository
            .create_reminder_job(
                input.organization_id,
                input.study_id,
                input.patient_id,
                input.visit_id,
                input.channel.trim(),
                input.recipient.trim(),
                input.message.trim(),
                input.scheduled_for,
            )
            .await?,
    ))
}

async fn list_reminder_jobs(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Query(query): Query<ReminderJobsQuery>,
) -> Result<Json<Vec<crate::models::ReminderJob>>, ApiError> {
    can_platform_admin(&user)?;
    let limit = query.limit.unwrap_or(50);
    Ok(Json(state.repository.list_reminder_jobs(limit).await?))
}

async fn process_reminder_jobs(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Json(input): Json<ProcessReminderJobsRequest>,
) -> Result<Json<ProcessReminderJobsResponse>, ApiError> {
    can_platform_admin(&user)?;
    let jobs = state
        .repository
        .process_due_reminder_jobs(input.limit.unwrap_or(50))
        .await?;
    Ok(Json(ProcessReminderJobsResponse {
        processed_count: jobs.len(),
        jobs,
    }))
}

fn escape_pdf_text(raw: &str) -> String {
    raw.replace('\\', "\\\\")
        .replace('(', "\\(")
        .replace(')', "\\)")
}

fn build_dua_pdf_bytes(
    organization_name: &str,
    counterparty: &str,
    status: &crate::models::AgreementStatus,
    signatures: &[crate::models::DuaSignature],
) -> Vec<u8> {
    let mut lines = vec![
        "Virival Data Use Agreement".to_string(),
        format!("Generated at: {}", Utc::now()),
        format!("Organization: {organization_name}"),
        format!("Counterparty: {counterparty}"),
        format!("Agreement status: {}", status.as_db()),
        " ".to_string(),
        "Signatures".to_string(),
    ];
    if signatures.is_empty() {
        lines.push("No signatures yet.".to_string());
    } else {
        for signature in signatures {
            lines.push(format!(
                "{} ({}) · role={} · signed_at={}",
                signature.signer_name,
                signature.signer_email,
                signature.signer_role,
                signature.signed_at
            ));
        }
    }

    let mut content_stream = String::from("BT\n/F1 12 Tf\n50 760 Td\n");
    for (index, line) in lines.iter().enumerate() {
        if index > 0 {
            content_stream.push_str("0 -16 Td\n");
        }
        content_stream.push_str(&format!("({}) Tj\n", escape_pdf_text(line)));
    }
    content_stream.push_str("ET");
    let length = content_stream.len();

    let objects = vec![
        "1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n".to_string(),
        "2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n".to_string(),
        "3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>\nendobj\n".to_string(),
        format!(
            "4 0 obj\n<< /Length {} >>\nstream\n{}\nendstream\nendobj\n",
            length, content_stream
        ),
        "5 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n".to_string(),
    ];

    let mut pdf = Vec::new();
    pdf.extend_from_slice(b"%PDF-1.4\n");

    let mut offsets = Vec::new();
    for object in objects {
        offsets.push(pdf.len());
        pdf.extend_from_slice(object.as_bytes());
    }
    let xref_offset = pdf.len();
    pdf.extend_from_slice(format!("xref\n0 {}\n", offsets.len() + 1).as_bytes());
    pdf.extend_from_slice(b"0000000000 65535 f \n");
    for offset in offsets {
        pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
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

fn workbench_tab(raw: Option<&str>) -> String {
    match raw.unwrap_or("setup") {
        "setup" | "design" | "execute" | "monitor" | "close" | "analytics" => {
            raw.unwrap_or("setup").to_string()
        }
        _ => "setup".to_string(),
    }
}

fn workbench_href(
    organization_id: Option<Uuid>,
    study_id: Option<Uuid>,
    tab: &str,
    notice: Option<&str>,
) -> String {
    let mut query = Vec::new();
    if let Some(org_id) = organization_id {
        query.push(format!("organization_id={org_id}"));
    }
    if let Some(study_id) = study_id {
        query.push(format!("study_id={study_id}"));
    }
    query.push(format!("tab={}", url_encode(tab)));
    if let Some(notice) = notice {
        query.push(format!("notice={}", url_encode(notice)));
    }
    format!("/ui/workbench?{}", query.join("&"))
}

fn phase_to_tab(phase: &StudyPhase) -> &'static str {
    match phase {
        StudyPhase::PreStudy | StudyPhase::Initiation => "setup",
        StudyPhase::Active => "execute",
        StudyPhase::Monitoring => "monitor",
        StudyPhase::Closed => "close",
    }
}

async fn submit_workbench_organization(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Form(form): Form<WorkbenchOrganizationForm>,
) -> Result<Redirect, ApiError> {
    can_admin(&user)?;
    if form.name.trim().is_empty() || form.workspace_slug.trim().is_empty() {
        return Err(ApiError::BadRequest(
            "name and workspace_slug are required".to_string(),
        ));
    }
    let organization = state
        .repository
        .create_organization(form.name.trim(), form.workspace_slug.trim())
        .await?;
    Ok(Redirect::to(&workbench_href(
        Some(organization.id),
        None,
        "setup",
        Some("Organization created"),
    )))
}

async fn submit_workbench_study(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Form(form): Form<WorkbenchStudyForm>,
) -> Result<Redirect, ApiError> {
    can_write(&user)?;
    if form.short_code.trim().is_empty() || form.title.trim().is_empty() {
        return Err(ApiError::BadRequest(
            "short_code and title are required".to_string(),
        ));
    }
    let study = state
        .repository
        .create_study(
            form.organization_id,
            form.short_code.trim(),
            form.title.trim(),
        )
        .await?;
    Ok(Redirect::to(&workbench_href(
        Some(form.organization_id),
        Some(study.id),
        "setup",
        Some("Study created"),
    )))
}

async fn submit_workbench_site(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Form(form): Form<WorkbenchSiteForm>,
) -> Result<Redirect, ApiError> {
    can_write(&user)?;
    if form.name.trim().is_empty() || form.principal_investigator.trim().is_empty() {
        return Err(ApiError::BadRequest(
            "site name and principal investigator are required".to_string(),
        ));
    }
    let site = state
        .repository
        .create_site(
            form.organization_id,
            Some(form.study_id),
            form.name.trim(),
            form.principal_investigator.trim(),
        )
        .await?;
    if form.startup_complete.is_some() {
        state.repository.set_site_startup(site.id, true).await?;
    }
    Ok(Redirect::to(&workbench_href(
        Some(form.organization_id),
        Some(form.study_id),
        "setup",
        Some("Site saved"),
    )))
}

async fn submit_workbench_dua(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Form(form): Form<WorkbenchDuaForm>,
) -> Result<Redirect, ApiError> {
    can_write(&user)?;
    if form.counterparty.trim().is_empty() {
        return Err(ApiError::BadRequest("counterparty is required".to_string()));
    }
    let dua = state
        .repository
        .create_dua(form.organization_id, form.counterparty.trim())
        .await?;
    state.repository.activate_dua(dua.id).await?;
    Ok(Redirect::to(&workbench_href(
        Some(form.organization_id),
        None,
        "setup",
        Some("DUA activated"),
    )))
}

async fn submit_workbench_dua_signature(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Form(form): Form<WorkbenchDuaSignatureForm>,
) -> Result<Redirect, ApiError> {
    can_write(&user)?;
    if form.signer_name.trim().is_empty()
        || form.signer_email.trim().is_empty()
        || form.signer_role.trim().is_empty()
        || form.signature_text.trim().is_empty()
    {
        return Err(ApiError::BadRequest(
            "signer_name, signer_email, signer_role, and signature_text are required".to_string(),
        ));
    }
    let dua = state.repository.get_dua(form.dua_id).await?;
    state
        .repository
        .create_dua_signature(
            dua.id,
            form.signer_name.trim(),
            form.signer_email.trim(),
            form.signer_role.trim(),
            form.signature_text.trim(),
        )
        .await?;
    Ok(Redirect::to(&workbench_href(
        Some(dua.organization_id),
        None,
        "setup",
        Some("DUA signature captured"),
    )))
}

async fn submit_workbench_crf_design(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Form(form): Form<WorkbenchCrfDesignForm>,
) -> Result<Redirect, ApiError> {
    can_write(&user)?;
    if form.template_name.trim().is_empty() {
        return Err(ApiError::BadRequest(
            "template_name is required".to_string(),
        ));
    }
    let schema_json: serde_json::Value = serde_json::from_str(form.schema_json.trim())
        .map_err(|err| ApiError::BadRequest(format!("invalid schema_json: {err}")))?;
    let template = state
        .repository
        .create_crf_template(form.study_id, form.template_name.trim())
        .await?;
    let version = state
        .repository
        .create_crf_template_version(template.id, &schema_json)
        .await?;
    if form.publish_version.is_some() {
        state
            .repository
            .publish_crf_template_version(version.id)
            .await?;
    }
    Ok(Redirect::to(&workbench_href(
        None,
        Some(form.study_id),
        "design",
        Some("CRF template version saved"),
    )))
}

async fn submit_workbench_visit_schedule(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Form(form): Form<WorkbenchVisitScheduleForm>,
) -> Result<Redirect, ApiError> {
    can_write(&user)?;
    if form.name.trim().is_empty() {
        return Err(ApiError::BadRequest(
            "schedule name is required".to_string(),
        ));
    }
    if form.window_before_days < 0 || form.window_after_days < 0 {
        return Err(ApiError::BadRequest(
            "visit windows cannot be negative".to_string(),
        ));
    }
    state
        .repository
        .create_visit_schedule_template(
            form.study_id,
            form.name.trim(),
            form.day_offset,
            form.window_before_days,
            form.window_after_days,
        )
        .await?;
    Ok(Redirect::to(&workbench_href(
        None,
        Some(form.study_id),
        "design",
        Some("Visit schedule template saved"),
    )))
}

async fn submit_workbench_patient(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Form(form): Form<WorkbenchPatientForm>,
) -> Result<Redirect, ApiError> {
    can_write(&user)?;
    if form.external_id.trim().is_empty() {
        return Err(ApiError::BadRequest("external_id is required".to_string()));
    }
    state
        .repository
        .create_patient(
            form.organization_id,
            form.study_id,
            form.site_id,
            form.external_id.trim(),
        )
        .await?;
    Ok(Redirect::to(&workbench_href(
        Some(form.organization_id),
        Some(form.study_id),
        "execute",
        Some("Patient enrolled"),
    )))
}

async fn submit_workbench_visit(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Form(form): Form<WorkbenchVisitForm>,
) -> Result<Redirect, ApiError> {
    can_write(&user)?;
    if form.visit_name.trim().is_empty() {
        return Err(ApiError::BadRequest("visit_name is required".to_string()));
    }
    let scheduled_for = DateTime::parse_from_rfc3339(form.scheduled_for.trim())
        .map_err(|err| ApiError::BadRequest(format!("scheduled_for must be RFC3339: {err}")))?
        .with_timezone(&Utc);
    state
        .repository
        .create_visit(
            form.study_id,
            form.patient_id,
            form.visit_name.trim(),
            scheduled_for,
        )
        .await?;
    Ok(Redirect::to(&workbench_href(
        None,
        Some(form.study_id),
        "execute",
        Some("Visit created"),
    )))
}

async fn submit_workbench_submission(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Form(form): Form<WorkbenchSubmissionForm>,
) -> Result<Redirect, ApiError> {
    can_write(&user)?;
    let submission = state
        .repository
        .create_crf_submission(
            form.study_id,
            form.patient_id,
            form.visit_id,
            form.template_id,
        )
        .await?;
    if form.lock_submission.is_some() {
        state.repository.lock_crf_submission(submission.id).await?;
    }
    Ok(Redirect::to(&workbench_href(
        None,
        Some(form.study_id),
        "execute",
        Some("CRF submission recorded"),
    )))
}

async fn submit_workbench_query(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Form(form): Form<WorkbenchQueryForm>,
) -> Result<Redirect, ApiError> {
    can_write(&user)?;
    match form.action.as_str() {
        "create" => {
            let submission_id = form
                .submission_id
                .ok_or_else(|| ApiError::BadRequest("submission_id is required".to_string()))?;
            let summary = form
                .summary
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| ApiError::BadRequest("summary is required".to_string()))?;
            state
                .repository
                .create_data_query(form.study_id, submission_id, summary)
                .await?;
        }
        "respond" => {
            let query_id = form
                .query_id
                .ok_or_else(|| ApiError::BadRequest("query_id is required".to_string()))?;
            state.repository.respond_data_query(query_id).await?;
            if let Some(comment_text) = form.comment_text.as_deref() {
                let trimmed = comment_text.trim();
                if !trimmed.is_empty() {
                    state
                        .repository
                        .create_data_query_comment(query_id, Some(user.user_id), trimmed)
                        .await?;
                }
            }
        }
        "close" => {
            let query_id = form
                .query_id
                .ok_or_else(|| ApiError::BadRequest("query_id is required".to_string()))?;
            state.repository.close_data_query(query_id).await?;
        }
        _ => {
            return Err(ApiError::BadRequest("unknown query action".to_string()));
        }
    }
    Ok(Redirect::to(&workbench_href(
        None,
        Some(form.study_id),
        "monitor",
        Some("Query workflow updated"),
    )))
}

async fn submit_workbench_closeout_item(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Form(form): Form<WorkbenchCloseoutItemForm>,
) -> Result<Redirect, ApiError> {
    match form.action.as_str() {
        "create" => {
            can_admin(&user)?;
            let item_key = form
                .item_key
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| ApiError::BadRequest("item_key is required".to_string()))?;
            let item_label = form
                .item_label
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| ApiError::BadRequest("item_label is required".to_string()))?;
            state
                .repository
                .create_closeout_checklist_item(
                    form.study_id,
                    item_key,
                    item_label,
                    form.is_required.is_some(),
                )
                .await?;
        }
        "complete" => {
            can_write(&user)?;
            let item_id = form
                .item_id
                .ok_or_else(|| ApiError::BadRequest("item_id is required".to_string()))?;
            state
                .repository
                .set_closeout_checklist_item_completion(item_id, true, Some(user.user_id))
                .await?;
        }
        _ => {
            return Err(ApiError::BadRequest("unknown closeout action".to_string()));
        }
    }
    Ok(Redirect::to(&workbench_href(
        None,
        Some(form.study_id),
        "close",
        Some("Closeout checklist updated"),
    )))
}

async fn submit_workbench_reminder(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Form(form): Form<WorkbenchReminderForm>,
) -> Result<Redirect, ApiError> {
    can_admin(&user)?;
    if form.channel.trim().is_empty()
        || form.recipient.trim().is_empty()
        || form.message.trim().is_empty()
    {
        return Err(ApiError::BadRequest(
            "channel, recipient, and message are required".to_string(),
        ));
    }
    let scheduled_for = DateTime::parse_from_rfc3339(form.scheduled_for.trim())
        .map_err(|err| ApiError::BadRequest(format!("scheduled_for must be RFC3339: {err}")))?
        .with_timezone(&Utc);
    state
        .repository
        .create_reminder_job(
            form.organization_id,
            form.study_id,
            form.patient_id,
            form.visit_id,
            form.channel.trim(),
            form.recipient.trim(),
            form.message.trim(),
            scheduled_for,
        )
        .await?;
    Ok(Redirect::to(&workbench_href(
        Some(form.organization_id),
        form.study_id,
        "monitor",
        Some("Reminder job scheduled"),
    )))
}

async fn submit_workbench_process_reminders(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Form(form): Form<WorkbenchReminderProcessForm>,
) -> Result<Redirect, ApiError> {
    can_admin(&user)?;
    let limit = form.limit.unwrap_or(20);
    let processed = state.repository.process_due_reminder_jobs(limit).await?;
    Ok(Redirect::to(&workbench_href(
        Some(form.organization_id),
        form.study_id,
        "monitor",
        Some(&format!("Processed {} due reminder jobs", processed.len())),
    )))
}

async fn submit_workbench_phase_transition(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Path(study_id): Path<Uuid>,
    Form(form): Form<WorkbenchPhaseTransitionForm>,
) -> Result<Redirect, ApiError> {
    can_admin(&user)?;
    let current = state.repository.get_study(study_id).await?;
    let readiness = state.repository.compute_readiness(study_id).await?;
    validate_phase_transition(&current.phase, &form.phase, &readiness)?;
    state
        .repository
        .update_study_phase(study_id, form.phase.clone())
        .await?;
    Ok(Redirect::to(&workbench_href(
        Some(current.organization_id),
        Some(study_id),
        phase_to_tab(&form.phase),
        Some("Study phase updated"),
    )))
}

async fn render_workbench(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Query(query): Query<WorkbenchQuery>,
) -> Result<Html<String>, ApiError> {
    can_read(&user)?;
    let active_tab = workbench_tab(query.tab.as_deref());
    let organizations = state.repository.list_organizations().await?;
    let all_studies = state.repository.list_studies().await?;
    let all_sites = state.repository.list_sites().await?;
    let all_patients = state.repository.list_patients().await?;
    let all_visits = state.repository.list_visits().await?;
    let all_templates = state.repository.list_crf_templates().await?;
    let all_submissions = state.repository.list_crf_submissions().await?;
    let all_queries = state.repository.list_data_queries().await?;
    let all_duas = state.repository.list_duas().await?;
    let all_reminder_jobs = state.repository.list_reminder_jobs(200).await?;

    let selected_org = query
        .organization_id
        .filter(|org_id| organizations.iter().any(|org| org.id == *org_id))
        .or_else(|| organizations.first().map(|org| org.id));
    let studies_for_org = all_studies
        .iter()
        .filter(|study| Some(study.organization_id) == selected_org)
        .cloned()
        .collect::<Vec<_>>();
    let selected_study = query
        .study_id
        .filter(|study_id| studies_for_org.iter().any(|study| study.id == *study_id))
        .or_else(|| studies_for_org.first().map(|study| study.id));

    let study_sites = all_sites
        .iter()
        .filter(|site| site.study_id == selected_study)
        .cloned()
        .collect::<Vec<_>>();
    let study_patients = all_patients
        .iter()
        .filter(|patient| Some(patient.study_id) == selected_study)
        .cloned()
        .collect::<Vec<_>>();
    let study_visits = all_visits
        .iter()
        .filter(|visit| Some(visit.study_id) == selected_study)
        .cloned()
        .collect::<Vec<_>>();
    let study_templates = all_templates
        .iter()
        .filter(|template| Some(template.study_id) == selected_study)
        .cloned()
        .collect::<Vec<_>>();
    let study_submissions = all_submissions
        .iter()
        .filter(|submission| Some(submission.study_id) == selected_study)
        .cloned()
        .collect::<Vec<_>>();
    let study_queries = all_queries
        .iter()
        .filter(|query_entry| Some(query_entry.study_id) == selected_study)
        .cloned()
        .collect::<Vec<_>>();
    let org_duas = all_duas
        .iter()
        .filter(|dua| Some(dua.organization_id) == selected_org)
        .cloned()
        .collect::<Vec<_>>();
    let scoped_reminders = all_reminder_jobs
        .iter()
        .filter(|job| Some(job.organization_id) == selected_org)
        .filter(|job| selected_study.is_none() || job.study_id == selected_study)
        .cloned()
        .collect::<Vec<_>>();
    let visit_schedule_templates = if let Some(study_id) = selected_study {
        state
            .repository
            .list_visit_schedule_templates(study_id)
            .await?
    } else {
        vec![]
    };
    let closeout_items = if let Some(study_id) = selected_study {
        state
            .repository
            .list_closeout_checklist_items(study_id)
            .await?
    } else {
        vec![]
    };
    let mut template_versions = Vec::new();
    for template in &study_templates {
        let mut versions = state
            .repository
            .list_crf_template_versions(template.id)
            .await?;
        template_versions.append(&mut versions);
    }
    let readiness = if let Some(study_id) = selected_study {
        Some(state.repository.compute_readiness(study_id).await?)
    } else {
        None
    };

    let setup_complete = readiness
        .as_ref()
        .map(|value| value.has_site && value.has_active_dua)
        .unwrap_or(false);
    let design_complete = readiness
        .as_ref()
        .map(|value| value.has_published_crf && !visit_schedule_templates.is_empty())
        .unwrap_or(false);
    let execute_complete = readiness
        .as_ref()
        .map(|value| value.has_enrolled_patient && value.has_locked_submission)
        .unwrap_or(false);
    let monitor_complete = readiness
        .as_ref()
        .map(|value| value.open_query_count == 0)
        .unwrap_or(false);
    let close_ready = readiness
        .as_ref()
        .map(|value| value.pending_closeout_items == 0 && value.open_query_count == 0)
        .unwrap_or(false);

    let org_options = organizations
        .iter()
        .map(|org| {
            let selected_attr = if Some(org.id) == selected_org {
                "selected"
            } else {
                ""
            };
            format!(
                r#"<option value="{id}" {selected}>{name} ({slug})</option>"#,
                id = org.id,
                selected = selected_attr,
                name = escape_html(&org.name),
                slug = escape_html(&org.workspace_slug)
            )
        })
        .collect::<String>();
    let study_options = studies_for_org
        .iter()
        .map(|study| {
            let selected_attr = if Some(study.id) == selected_study {
                "selected"
            } else {
                ""
            };
            format!(
                r#"<option value="{id}" {selected}>{code} · {title}</option>"#,
                id = study.id,
                selected = selected_attr,
                code = escape_html(&study.short_code),
                title = escape_html(&study.title),
            )
        })
        .collect::<String>();
    let study_select_options = studies_for_org
        .iter()
        .map(|study| {
            format!(
                r#"<option value="{id}">{code} · {title}</option>"#,
                id = study.id,
                code = escape_html(&study.short_code),
                title = escape_html(&study.title),
            )
        })
        .collect::<String>();
    let site_options = study_sites
        .iter()
        .map(|site| {
            format!(
                r#"<option value="{id}">{name}</option>"#,
                id = site.id,
                name = escape_html(&site.name)
            )
        })
        .collect::<String>();
    let patient_options = study_patients
        .iter()
        .map(|patient| {
            format!(
                r#"<option value="{id}">{external_id}</option>"#,
                id = patient.id,
                external_id = escape_html(&patient.external_id)
            )
        })
        .collect::<String>();
    let visit_options = study_visits
        .iter()
        .map(|visit| {
            format!(
                r#"<option value="{id}">{name} ({status})</option>"#,
                id = visit.id,
                name = escape_html(&visit.visit_name),
                status = visit.status.as_db()
            )
        })
        .collect::<String>();
    let template_options = study_templates
        .iter()
        .map(|template| {
            format!(
                r#"<option value="{id}">{name}</option>"#,
                id = template.id,
                name = escape_html(&template.name)
            )
        })
        .collect::<String>();
    let submission_options = study_submissions
        .iter()
        .map(|submission| {
            format!(
                r#"<option value="{id}">{id} ({status})</option>"#,
                id = submission.id,
                status = submission.status.as_db()
            )
        })
        .collect::<String>();
    let query_options = study_queries
        .iter()
        .filter(|query_entry| query_entry.status.as_db() != "closed")
        .map(|query_entry| {
            format!(
                r#"<option value="{id}">{summary} ({status})</option>"#,
                id = query_entry.id,
                summary = escape_html(&query_entry.summary),
                status = query_entry.status.as_db()
            )
        })
        .collect::<String>();
    let dua_options = org_duas
        .iter()
        .map(|dua| {
            format!(
                r#"<option value="{id}">{counterparty} ({status})</option>"#,
                id = dua.id,
                counterparty = escape_html(&dua.counterparty),
                status = dua.status.as_db()
            )
        })
        .collect::<String>();
    let incomplete_closeout_options = closeout_items
        .iter()
        .filter(|item| !item.is_complete)
        .map(|item| {
            format!(
                r#"<option value="{id}">{label}</option>"#,
                id = item.id,
                label = escape_html(&item.item_label)
            )
        })
        .collect::<String>();

    let required_closeout_total = closeout_items
        .iter()
        .filter(|item| item.is_required)
        .count();
    let required_closeout_complete = closeout_items
        .iter()
        .filter(|item| item.is_required && item.is_complete)
        .count();
    let closeout_percent = if required_closeout_total == 0 {
        100
    } else {
        (required_closeout_complete * 100) / required_closeout_total
    };
    let open_queries = study_queries
        .iter()
        .filter(|query_entry| query_entry.status.as_db() == "open")
        .count();
    let responded_queries = study_queries
        .iter()
        .filter(|query_entry| query_entry.status.as_db() == "responded")
        .count();
    let closed_queries = study_queries
        .iter()
        .filter(|query_entry| query_entry.status.as_db() == "closed")
        .count();
    let locked_submissions = study_submissions
        .iter()
        .filter(|submission| submission.status.as_db() == "locked")
        .count();
    let pending_reminders = scoped_reminders
        .iter()
        .filter(|job| job.status.as_db() == "pending")
        .count();
    let sent_reminders = scoped_reminders
        .iter()
        .filter(|job| job.status.as_db() == "sent")
        .count();
    let failed_reminders = scoped_reminders
        .iter()
        .filter(|job| job.status.as_db() == "failed")
        .count();
    let reminder_rows = if scoped_reminders.is_empty() {
        r#"<tr><td colspan="5">No reminder jobs in current scope.</td></tr>"#.to_string()
    } else {
        scoped_reminders
            .iter()
            .take(12)
            .map(|job| {
                format!(
                    r#"<tr><td>{recipient}</td><td>{channel}</td><td>{status}</td><td>{scheduled_for}</td><td>{processed_at}</td></tr>"#,
                    recipient = escape_html(&job.recipient),
                    channel = escape_html(&job.channel),
                    status = job.status.as_db(),
                    scheduled_for = job.scheduled_for,
                    processed_at = job
                        .processed_at
                        .map(|value| value.to_rfc3339())
                        .unwrap_or_else(|| "-".to_string())
                )
            })
            .collect::<String>()
    };

    let mut dua_signature_rows = String::new();
    let mut total_dua_signatures = 0usize;
    for dua in &org_duas {
        let signatures = state.repository.list_dua_signatures(dua.id).await?;
        total_dua_signatures += signatures.len();
        if signatures.is_empty() {
            dua_signature_rows.push_str(&format!(
                r#"<tr><td>{counterparty}</td><td>none</td><td>-</td><td>-</td></tr>"#,
                counterparty = escape_html(&dua.counterparty)
            ));
        } else {
            for signature in signatures {
                dua_signature_rows.push_str(&format!(
                    r#"<tr><td>{counterparty}</td><td>{signer}</td><td>{role}</td><td>{signed_at}</td></tr>"#,
                    counterparty = escape_html(&dua.counterparty),
                    signer = escape_html(&signature.signer_email),
                    role = escape_html(&signature.signer_role),
                    signed_at = signature.signed_at
                ));
            }
        }
    }
    if dua_signature_rows.is_empty() {
        dua_signature_rows = r#"<tr><td colspan="4">No DUA signatures yet.</td></tr>"#.to_string();
    }
    let dua_download_links = if org_duas.is_empty() {
        "<span class=\"muted\">No DUAs yet.</span>".to_string()
    } else {
        org_duas
            .iter()
            .map(|dua| {
                format!(
                    r#"<a href="/api/v1/duas/{dua_id}/pdf" style="margin-right:0.4rem;">Download DUA PDF ({counterparty})</a>"#,
                    dua_id = dua.id,
                    counterparty = escape_html(&dua.counterparty)
                )
            })
            .collect::<String>()
    };

    let notice_html = query
        .notice
        .as_ref()
        .map(|notice| format!(r#"<div class="notice">{}</div>"#, escape_html(notice)))
        .unwrap_or_default();
    let guidance = readiness
        .as_ref()
        .map(|value| escape_html(&value.next_recommended_action))
        .unwrap_or_else(|| "Create an organization to start the wizard.".to_string());
    let selected_org_value = selected_org
        .map(|org_id| org_id.to_string())
        .unwrap_or_else(String::new);
    let selected_study_value = selected_study
        .map(|study_id| study_id.to_string())
        .unwrap_or_else(String::new);
    let process_reminder_form = if let Some(org_id) = selected_org {
        let study_hidden = selected_study
            .map(|study_id| {
                format!(r#"<input type="hidden" name="study_id" value="{study_id}" />"#)
            })
            .unwrap_or_default();
        format!(
            r#"<form method="post" action="/ui/workbench/reminders/process">
  <input type="hidden" name="organization_id" value="{org_id}" />
  {study_hidden}
  <input type="hidden" name="limit" value="25" />
  <button type="submit">Process due reminders now</button>
</form>"#
        )
    } else {
        "<p class=\"muted\">Select organization context to process reminders.</p>".to_string()
    };
    let tab_links = [
        "setup",
        "design",
        "execute",
        "monitor",
        "close",
        "analytics",
    ]
    .iter()
    .map(|tab| {
        let active_class = if active_tab == *tab { "active" } else { "" };
        let label = match *tab {
            "setup" => "Setup",
            "design" => "Design",
            "execute" => "Execute",
            "monitor" => "Monitor",
            "close" => "Closeout",
            "analytics" => "Analytics",
            _ => *tab,
        };
        format!(
            r#"<a class="tab {active_class}" href="{href}">{label}</a>"#,
            active_class = active_class,
            href = workbench_href(selected_org, selected_study, tab, None),
            label = label,
        )
    })
    .collect::<String>();

    let stage_cards = format!(
        r#"<section class="stage-grid">
  <div class="stage-card {setup_class}"><strong>1. Setup</strong><p>Org, study, site startup, and active DUA.</p></div>
  <div class="stage-card {design_class}"><strong>2. Design</strong><p>Versioned CRF schema + visit schedule templates.</p></div>
  <div class="stage-card {execute_class}"><strong>3. Execute</strong><p>Enroll patients, schedule visits, capture CRFs.</p></div>
  <div class="stage-card {monitor_class}"><strong>4. Monitor</strong><p>Run open → responded → closed query workflow.</p></div>
  <div class="stage-card {close_class}"><strong>5. Closeout</strong><p>Complete required checklist and close safely.</p></div>
</section>"#,
        setup_class = if setup_complete { "complete" } else { "" },
        design_class = if design_complete { "complete" } else { "" },
        execute_class = if execute_complete { "complete" } else { "" },
        monitor_class = if monitor_complete { "complete" } else { "" },
        close_class = if close_ready { "complete" } else { "" },
    );

    let selected_study_phase = studies_for_org
        .iter()
        .find(|study| Some(study.id) == selected_study)
        .map(|study| study.phase.as_db().to_string())
        .unwrap_or_else(|| "not selected".to_string());

    let tab_content = match active_tab.as_str() {
        "setup" => format!(
            r#"<section class="panel-grid">
  <article class="panel">
    <h3>Create organization</h3>
    <form method="post" action="/ui/workbench/organizations">
      <label>Name</label><input name="name" placeholder="Cingulum Foundation Inc" required />
      <label>Workspace slug</label><input name="workspace_slug" placeholder="cingulum-foundation" required />
      <button type="submit">Create organization</button>
    </form>
  </article>
  <article class="panel">
    <h3>Create study</h3>
    <form method="post" action="/ui/workbench/studies">
      <label>Organization</label><select name="organization_id" required>{org_options}</select>
      <label>Short code</label><input name="short_code" placeholder="ALS-001" required />
      <label>Title</label><input name="title" placeholder="ALS Longitudinal Study" required />
      <button type="submit">Create study</button>
    </form>
  </article>
  <article class="panel">
    <h3>Attach site and mark startup</h3>
    <form method="post" action="/ui/workbench/sites">
      <label>Organization</label><select name="organization_id" required>{org_options}</select>
      <label>Study</label><select name="study_id" required>{study_select_options}</select>
      <label>Site name</label><input name="name" placeholder="Wyckoff Heights Medical Center" required />
      <label>Principal investigator</label><input name="principal_investigator" placeholder="Dr. Example" required />
      <label class="checkbox"><input type="checkbox" name="startup_complete" /> Mark startup complete now</label>
      <button type="submit">Save site</button>
    </form>
  </article>
  <article class="panel">
    <h3>Create + activate DUA</h3>
    <form method="post" action="/ui/workbench/duas">
      <label>Organization</label><select name="organization_id" required>{org_options}</select>
      <label>Counterparty</label><input name="counterparty" placeholder="Hospital Partner" required />
      <button type="submit">Create and activate DUA</button>
    </form>
    <p class="muted">Current study phase: <strong>{selected_study_phase}</strong></p>
  </article>
  <article class="panel">
    <h3>Capture DUA e-signature</h3>
    <form method="post" action="/ui/workbench/dua-signatures">
      <label>DUA</label><select name="dua_id" required>{dua_options}</select>
      <label>Signer name</label><input name="signer_name" placeholder="Jane Doe" required />
      <label>Signer email</label><input name="signer_email" placeholder="jane@hospital.org" required />
      <label>Signer role</label><input name="signer_role" placeholder="Legal Signatory" required />
      <label>Signature text</label><input name="signature_text" placeholder="Jane Doe /s/" required />
      <button type="submit">Record signature</button>
    </form>
    <p class="muted">{dua_download_links}</p>
  </article>
  <article class="panel full">
    <h3>DUA signature ledger</h3>
    <p class="muted">Captured signatures: <strong>{total_dua_signatures}</strong></p>
    <div class="table-wrap">
      <table><thead><tr><th>Counterparty</th><th>Signer</th><th>Role</th><th>Signed at</th></tr></thead><tbody>{dua_signature_rows}</tbody></table>
    </div>
  </article>
</section>"#,
            org_options = org_options.as_str(),
            study_select_options = study_select_options.as_str(),
            selected_study_phase = escape_html(&selected_study_phase),
            dua_options = dua_options.as_str(),
            dua_download_links = dua_download_links.as_str(),
            total_dua_signatures = total_dua_signatures,
            dua_signature_rows = dua_signature_rows.as_str()
        ),
        "design" => format!(
            r#"<section class="panel-grid">
  <article class="panel">
    <h3>CRF template + schema version</h3>
    <form method="post" action="/ui/workbench/crf-design">
      <label>Study</label><select name="study_id" required>{study_select_options}</select>
      <label>Template name</label><input name="template_name" placeholder="Baseline Vitals" required />
      <label>Schema JSON</label>
      <textarea name="schema_json" rows="6" required>{{"fields":[{{"key":"bp_systolic","type":"number","required":true}}]}}</textarea>
      <label class="checkbox"><input type="checkbox" name="publish_version" /> Publish this version now</label>
      <button type="submit">Save template version</button>
    </form>
  </article>
  <article class="panel">
    <h3>Visit schedule template</h3>
    <form method="post" action="/ui/workbench/visit-schedules">
      <label>Study</label><select name="study_id" required>{study_select_options}</select>
      <label>Name</label><input name="name" placeholder="Baseline" required />
      <label>Day offset</label><input name="day_offset" type="number" value="0" required />
      <label>Window before (days)</label><input name="window_before_days" type="number" value="2" required />
      <label>Window after (days)</label><input name="window_after_days" type="number" value="3" required />
      <button type="submit">Add visit schedule</button>
    </form>
  </article>
  <article class="panel full">
    <h3>Design inventory</h3>
    <p class="muted">Templates: <strong>{template_count}</strong> · Versions: <strong>{version_count}</strong> · Visit schedules: <strong>{schedule_count}</strong></p>
    <div class="table-wrap">
      <table><thead><tr><th>Template</th><th>Version</th><th>Published</th></tr></thead><tbody>{version_rows}</tbody></table>
    </div>
  </article>
</section>"#,
            study_select_options = study_select_options.as_str(),
            template_count = study_templates.len(),
            version_count = template_versions.len(),
            schedule_count = visit_schedule_templates.len(),
            version_rows = if template_versions.is_empty() {
                r#"<tr><td colspan="3">No template versions yet.</td></tr>"#.to_string()
            } else {
                template_versions
                    .iter()
                    .map(|version| {
                        let template_name = study_templates
                            .iter()
                            .find(|template| template.id == version.template_id)
                            .map(|template| escape_html(&template.name))
                            .unwrap_or_else(|| version.template_id.to_string());
                        format!(
                            r#"<tr><td>{template_name}</td><td>v{version_number}</td><td>{published}</td></tr>"#,
                            template_name = template_name,
                            version_number = version.version_number,
                            published = if version.is_published { "yes" } else { "no" }
                        )
                    })
                    .collect::<String>()
            }
        ),
        "execute" => format!(
            r#"<section class="panel-grid">
  <article class="panel">
    <h3>Enroll patient</h3>
    <form method="post" action="/ui/workbench/patients">
      <label>Organization</label><select name="organization_id" required>{org_options}</select>
      <label>Study</label><select name="study_id" required>{study_select_options}</select>
      <label>Site (optional)</label><select name="site_id"><option value="">No site</option>{site_options}</select>
      <label>Patient external ID</label><input name="external_id" placeholder="PT-0001" required />
      <button type="submit">Enroll patient</button>
    </form>
  </article>
  <article class="panel">
    <h3>Create visit</h3>
    <form method="post" action="/ui/workbench/visits">
      <label>Study</label><select name="study_id" required>{study_select_options}</select>
      <label>Patient</label><select name="patient_id" required>{patient_options}</select>
      <label>Visit name</label><input name="visit_name" placeholder="Baseline" required />
      <label>Scheduled for (RFC3339)</label><input name="scheduled_for" placeholder="2026-06-16T00:00:00Z" required />
      <button type="submit">Create visit</button>
    </form>
  </article>
  <article class="panel">
    <h3>Capture CRF submission</h3>
    <form method="post" action="/ui/workbench/submissions">
      <label>Study</label><select name="study_id" required>{study_select_options}</select>
      <label>Patient</label><select name="patient_id" required>{patient_options}</select>
      <label>Visit</label><select name="visit_id" required>{visit_options}</select>
      <label>Template</label><select name="template_id" required>{template_options}</select>
      <label class="checkbox"><input type="checkbox" name="lock_submission" /> Lock immediately after capture</label>
      <button type="submit">Save submission</button>
    </form>
  </article>
</section>"#,
            org_options = org_options.as_str(),
            study_select_options = study_select_options.as_str(),
            site_options = site_options.as_str(),
            patient_options = patient_options.as_str(),
            visit_options = visit_options.as_str(),
            template_options = template_options.as_str(),
        ),
        "monitor" => format!(
            r#"<section class="panel-grid">
  <article class="panel">
    <h3>Raise data query</h3>
    <form method="post" action="/ui/workbench/queries">
      <input type="hidden" name="action" value="create" />
      <label>Study</label><select name="study_id" required>{study_select_options}</select>
      <label>Submission</label><select name="submission_id" required>{submission_options}</select>
      <label>Summary</label><input name="summary" placeholder="Please confirm units" required />
      <button type="submit">Raise query</button>
    </form>
  </article>
  <article class="panel">
    <h3>Respond to query</h3>
    <form method="post" action="/ui/workbench/queries">
      <input type="hidden" name="action" value="respond" />
      <label>Study</label><select name="study_id" required>{study_select_options}</select>
      <label>Query</label><select name="query_id" required>{query_options}</select>
      <label>Response comment (optional)</label><input name="comment_text" placeholder="Updated source doc attached" />
      <button type="submit">Mark responded</button>
    </form>
  </article>
  <article class="panel">
    <h3>Close query</h3>
    <form method="post" action="/ui/workbench/queries">
      <input type="hidden" name="action" value="close" />
      <label>Study</label><select name="study_id" required>{study_select_options}</select>
      <label>Query</label><select name="query_id" required>{query_options}</select>
      <button type="submit">Close query</button>
    </form>
    <p class="muted">Open: <strong>{open_queries}</strong> · Responded: <strong>{responded_queries}</strong> · Closed: <strong>{closed_queries}</strong></p>
  </article>
  <article class="panel">
    <h3>Schedule reminder job</h3>
    <form method="post" action="/ui/workbench/reminders">
      <label>Organization</label><select name="organization_id" required>{org_options}</select>
      <label>Study (optional)</label><select name="study_id"><option value="">None</option>{study_select_options}</select>
      <label>Patient (optional)</label><select name="patient_id"><option value="">None</option>{patient_options}</select>
      <label>Visit (optional)</label><select name="visit_id"><option value="">None</option>{visit_options}</select>
      <label>Channel</label><input name="channel" placeholder="email" value="email" required />
      <label>Recipient</label><input name="recipient" placeholder="patient@domain.org" required />
      <label>Message</label><input name="message" placeholder="Reminder: visit tomorrow at 10:00" required />
      <label>Scheduled for (RFC3339)</label><input name="scheduled_for" placeholder="2026-06-16T14:00:00Z" required />
      <button type="submit">Queue reminder</button>
    </form>
    <p class="muted">Pending: <strong>{pending_reminders}</strong> · Sent: <strong>{sent_reminders}</strong> · Failed: <strong>{failed_reminders}</strong></p>
    {process_reminder_form}
  </article>
  <article class="panel full">
    <h3>Reminder queue snapshot</h3>
    <div class="table-wrap">
      <table><thead><tr><th>Recipient</th><th>Channel</th><th>Status</th><th>Scheduled</th><th>Processed</th></tr></thead><tbody>{reminder_rows}</tbody></table>
    </div>
  </article>
</section>"#,
            study_select_options = study_select_options.as_str(),
            submission_options = submission_options.as_str(),
            query_options = query_options.as_str(),
            org_options = org_options.as_str(),
            patient_options = patient_options.as_str(),
            visit_options = visit_options.as_str(),
            open_queries = open_queries,
            responded_queries = responded_queries,
            closed_queries = closed_queries,
            pending_reminders = pending_reminders,
            sent_reminders = sent_reminders,
            failed_reminders = failed_reminders,
            reminder_rows = reminder_rows.as_str(),
            process_reminder_form = process_reminder_form.as_str(),
        ),
        "close" => format!(
            r#"<section class="panel-grid">
  <article class="panel">
    <h3>Add closeout checklist item</h3>
    <form method="post" action="/ui/workbench/closeout-items">
      <input type="hidden" name="action" value="create" />
      <label>Study</label><select name="study_id" required>{study_select_options}</select>
      <label>Item key</label><input name="item_key" placeholder="db_reconciliation" required />
      <label>Item label</label><input name="item_label" placeholder="Database reconciliation complete" required />
      <label class="checkbox"><input type="checkbox" name="is_required" checked /> Required for closure</label>
      <button type="submit">Add checklist item</button>
    </form>
  </article>
  <article class="panel">
    <h3>Complete closeout checklist item</h3>
    <form method="post" action="/ui/workbench/closeout-items">
      <input type="hidden" name="action" value="complete" />
      <label>Study</label><select name="study_id" required>{study_select_options}</select>
      <label>Incomplete item</label><select name="item_id" required>{incomplete_closeout_options}</select>
      <button type="submit">Mark complete</button>
    </form>
    <p class="muted">Required completion: <strong>{required_closeout_complete}/{required_closeout_total}</strong> ({closeout_percent}%)</p>
  </article>
  <article class="panel">
    <h3>Phase transition</h3>
    <p class="muted">Current phase: <strong>{selected_study_phase}</strong></p>
    {phase_controls}
  </article>
</section>"#,
            study_select_options = study_select_options.as_str(),
            incomplete_closeout_options = incomplete_closeout_options.as_str(),
            required_closeout_complete = required_closeout_complete,
            required_closeout_total = required_closeout_total,
            closeout_percent = closeout_percent,
            selected_study_phase = escape_html(&selected_study_phase),
            phase_controls = if let Some(study_id) = selected_study {
                format!(
                    r#"<div class="phase-actions">
  <form method="post" action="/ui/workbench/studies/{study_id}/phase"><input type="hidden" name="phase" value="initiation" /><button type="submit">Move to initiation</button></form>
  <form method="post" action="/ui/workbench/studies/{study_id}/phase"><input type="hidden" name="phase" value="active" /><button type="submit">Move to active</button></form>
  <form method="post" action="/ui/workbench/studies/{study_id}/phase"><input type="hidden" name="phase" value="monitoring" /><button type="submit">Move to monitoring</button></form>
  <form method="post" action="/ui/workbench/studies/{study_id}/phase"><input type="hidden" name="phase" value="closed" /><button type="submit">Move to closed</button></form>
</div>"#,
                    study_id = study_id
                )
            } else {
                "<p class=\"muted\">Select a study first.</p>".to_string()
            }
        ),
        _ => format!(
            r#"<section class="analytics-grid">
  <article class="kpi"><h4>Sites</h4><strong>{sites}</strong></article>
  <article class="kpi"><h4>Patients</h4><strong>{patients}</strong></article>
  <article class="kpi"><h4>Visits</h4><strong>{visits}</strong></article>
  <article class="kpi"><h4>Submissions (locked)</h4><strong>{locked}/{total_submissions}</strong></article>
  <article class="kpi"><h4>Queries (open/responded/closed)</h4><strong>{open}/{responded}/{closed}</strong></article>
  <article class="kpi"><h4>DUA signatures</h4><strong>{dua_signatures}</strong></article>
  <article class="kpi"><h4>Reminders (pending/sent/failed)</h4><strong>{pending_reminders}/{sent_reminders}/{failed_reminders}</strong></article>
  <article class="kpi"><h4>Required closeout completion</h4><strong>{closeout_percent}%</strong></article>
</section>
<section class="panel" style="margin-top:0.75rem;">
  <h3>Readiness signal</h3>
  <p class="muted">Next recommended action: <strong>{guidance}</strong></p>
</section>"#,
            sites = study_sites.len(),
            patients = study_patients.len(),
            visits = study_visits.len(),
            locked = locked_submissions,
            total_submissions = study_submissions.len(),
            open = open_queries,
            responded = responded_queries,
            closed = closed_queries,
            dua_signatures = total_dua_signatures,
            pending_reminders = pending_reminders,
            sent_reminders = sent_reminders,
            failed_reminders = failed_reminders,
            closeout_percent = closeout_percent,
            guidance = guidance.as_str()
        ),
    };

    Ok(Html(format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>Virival Workbench</title>
  <style>
    :root {{ --cream:#E7E5DA; --sand:#C5B7AB; --forest:#283E28; --navy:#02182B; --orange:#F05708; }}
    * {{ box-sizing:border-box; }}
    body {{ margin:0; background:linear-gradient(180deg,#f6f4ee 0%,var(--cream) 100%); color:var(--navy); font-family:Inter,Arial,sans-serif; }}
    .page {{ max-width:1200px; margin:0 auto; padding:1rem; }}
    .hero {{ background:#fffdf8; border:1px solid rgba(2,24,43,.12); border-radius:16px; padding:1rem; box-shadow:0 12px 28px rgba(2,24,43,.08); animation:fade .25s ease-out; }}
    .hero h1 {{ margin:0; font-size:1.35rem; }}
    .muted {{ color:#365067; }}
    .notice {{ margin:0.8rem 0; border:1px solid #d9c8ba; background:#fff7ef; color:#4a3827; padding:0.55rem 0.65rem; border-radius:10px; }}
    .stage-grid {{ margin-top:0.8rem; display:grid; grid-template-columns:repeat(auto-fit,minmax(180px,1fr)); gap:0.6rem; }}
    .stage-card {{ border:1px solid rgba(2,24,43,.12); border-left:4px solid var(--orange); border-radius:12px; background:#fffefb; padding:0.65rem; }}
    .stage-card.complete {{ border-left-color:#2f7d32; background:#f5fbf5; }}
    .tab-row {{ margin-top:0.8rem; display:flex; flex-wrap:wrap; gap:0.45rem; }}
    .tab {{ text-decoration:none; color:var(--navy); background:#fffefb; border:1px solid #cfd9e4; border-radius:999px; padding:0.35rem 0.75rem; font-size:.85rem; font-weight:700; }}
    .tab.active {{ background:var(--navy); color:#fff; border-color:var(--navy); }}
    .context {{ margin-top:0.8rem; display:grid; grid-template-columns:repeat(auto-fit,minmax(260px,1fr)); gap:0.6rem; }}
    .context .card {{ background:#fff; border:1px solid #d9e1ea; border-radius:12px; padding:0.7rem; }}
    label {{ display:block; font-weight:700; font-size:.83rem; margin:.45rem 0 .2rem; }}
    input,select,textarea {{ width:100%; border:1px solid #cbd5e1; border-radius:10px; padding:.52rem; font:inherit; }}
    textarea {{ resize:vertical; }}
    button {{ margin-top:.55rem; background:var(--navy); color:#fff; border:0; border-radius:10px; padding:.5rem .78rem; font-weight:700; cursor:pointer; }}
    .checkbox {{ display:flex; align-items:center; gap:0.45rem; font-weight:600; }}
    .checkbox input {{ width:auto; }}
    .panel-grid {{ margin-top:0.8rem; display:grid; grid-template-columns:repeat(auto-fit,minmax(320px,1fr)); gap:0.7rem; }}
    .panel {{ background:#fff; border:1px solid #dce3eb; border-radius:14px; padding:.8rem .85rem; }}
    .panel.full {{ grid-column:1/-1; }}
    .panel h3 {{ margin:0 0 .45rem; }}
    .table-wrap {{ overflow:auto; }}
    table {{ width:100%; border-collapse:collapse; }}
    th,td {{ text-align:left; padding:.45rem; border-bottom:1px solid #e2e8f0; font-size:.84rem; }}
    .phase-actions form {{ margin-bottom:.35rem; }}
    .analytics-grid {{ margin-top:.8rem; display:grid; grid-template-columns:repeat(auto-fit,minmax(170px,1fr)); gap:.6rem; }}
    .kpi {{ background:#fff; border:1px solid #dbe5ee; border-radius:12px; padding:.7rem; }}
    .kpi h4 {{ margin:0; font-size:.8rem; color:#415a71; }}
    .kpi strong {{ display:block; margin-top:.25rem; font-size:1.2rem; }}
    @keyframes fade {{ from {{ opacity:0; transform:translateY(4px); }} to {{ opacity:1; transform:translateY(0); }} }}
  </style>
</head>
<body>
  <div class="page">
    <section class="hero">
      <h1>Virival Phase 7 · Research Workbench</h1>
      <p class="muted">Authenticated as <strong>{subject}</strong> ({role}). Focus organization: <strong>{selected_org}</strong>. Focus study: <strong>{selected_study}</strong>.</p>
      <p class="muted"><strong>Workflow coach:</strong> {guidance}</p>
      {stage_cards}
    </section>
    {notice_html}
    <section class="tab-row">{tab_links}</section>
    <section class="context">
      <form class="card" method="get" action="/ui/workbench">
        <h3 style="margin:0 0 0.35rem;">Context selector</h3>
        <label>Organization</label><select name="organization_id">{org_options}</select>
        <label>Study</label><select name="study_id"><option value="">None selected</option>{study_options}</select>
        <input type="hidden" name="tab" value="{active_tab}" />
        <button type="submit">Apply context</button>
      </form>
      <div class="card">
        <h3 style="margin:0 0 0.35rem;">Quick admin links</h3>
        <p class="muted" style="margin:0 0 .4rem;"><a href="/ui/admin/memberships">Membership admin</a> · <a href="/ui/admin/audit">Audit console</a></p>
        <p class="muted" style="margin:0;">Role source: <strong>{auth_source}</strong></p>
      </div>
    </section>
    {tab_content}
  </div>
</body>
</html>"#,
        subject = escape_html(&user.subject),
        role = user.role.as_str(),
        selected_org = escape_html(&selected_org_value),
        selected_study = escape_html(&selected_study_value),
        guidance = guidance.as_str(),
        stage_cards = stage_cards.as_str(),
        notice_html = notice_html.as_str(),
        tab_links = tab_links.as_str(),
        org_options = org_options.as_str(),
        study_options = study_options.as_str(),
        active_tab = active_tab.as_str(),
        auth_source = escape_html(&user.auth_source),
        tab_content = tab_content.as_str(),
    )))
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
      <h1>Virival · Phase 7 Product Shell</h1>
      <p class="muted">Authenticated as <strong>{subject}</strong> (<strong>{email}</strong>) with role <strong>{role}</strong>. Source: <strong>{auth_source}</strong>. Organization scope: <strong>{org_scope}</strong>.</p>
      <span class="chip">workflow-first architecture mode</span>
      <p style="margin:0.55rem 0 0;"><a href="/ui/workbench" style="display:inline-block;background:#02182b;color:#fff;text-decoration:none;border-radius:10px;padding:0.5rem 0.75rem;font-weight:700;">Open Research Workbench →</a></p>
    </section>
    <section class="grid">
      <div class="card"><h3>1. Open the Workbench</h3><p>Use <code>/ui/workbench</code> for tabbed setup, design, execute, monitor, closeout, and analytics workflows.</p></div>
      <div class="card"><h3>2. Assign membership</h3><p>Use <code>POST /api/v1/admin/memberships</code> or the membership admin UI to persist org roles.</p></div>
      <div class="card"><h3>3. Compliance workflows</h3><p>Capture DUA signatures, download DUA PDF exports, and schedule reminder jobs from the workbench.</p></div>
      <div class="card"><h3>4. Audit governance</h3><p>Track identity, resource actions, and outcomes in <code>/ui/admin/audit</code>.</p></div>
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
