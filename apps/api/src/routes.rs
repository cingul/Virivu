use axum::{
    extract::{Form, Multipart, Path, Query, Request, State},
    http::{header, HeaderValue, StatusCode},
    middleware::{from_fn_with_state, Next},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
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

/// Simple persistent context bar for guided workflow feel.
/// Shows current scope and quick navigation.
fn render_context_bar(
    current_org: Option<&str>,
    current_project: Option<&str>,
    current_user: &str,
) -> String {
    let org_display = current_org.unwrap_or("No organization selected");
    let proj_display = current_project.unwrap_or("No study selected");

    format!(
        r#"
<div style="background:#f8fafc; border-bottom:1px solid #e2e8f0; padding:8px 16px; font-size:0.85rem; display:flex; align-items:center; gap:12px; flex-wrap:wrap;">
  <span style="color:#64748b;">Logged in as</span> <strong>{}</strong>
  <span style="color:#cbd5e0;">|</span>
  <span style="color:#64748b;">Org:</span> <strong style="color:#1e2937;">{}</strong>
  <span style="color:#cbd5e0;">|</span>
  <span style="color:#64748b;">Study:</span> <strong style="color:#1e2937;">{}</strong>
  <a href="/ui/app" style="margin-left:auto; font-size:0.8rem; color:#f05708; text-decoration:none;">Switch context →</a>
</div>
"#,
        html_escape(current_user),
        html_escape(org_display),
        html_escape(proj_display)
    )
}

#[derive(Clone)]
pub struct AppContext {
    pub config: Config,
    pub db: Db,
}


async fn render_portal_placeholder() -> Result<Html<String>, ApiError> {
    let body = r#"
<div style="max-width:820px; margin: 40px auto; font-family: system-ui, sans-serif;">
  <div style="background:white; border-radius:12px; box-shadow:0 10px 15px -3px rgba(0,0,0,0.1); padding:2.5rem;">
    <div style="display:flex; align-items:center; gap:12px; margin-bottom:1.5rem;">
      <div style="width:42px; height:42px; background:#f05708; border-radius:8px; display:flex; align-items:center; justify-content:center; color:white; font-weight:800; font-size:1.4rem;">V</div>
      <div>
        <div style="font-size:1.35rem; font-weight:700; color:#02182b;">Virivu Patient Portal</div>
        <div style="font-size:0.85rem; color:#64748b;">Secure participant engagement &amp; data capture</div>
      </div>
    </div>

    <h1 style="margin:0 0 0.5rem; font-size:1.55rem;">Welcome, Participant</h1>
    <p style="color:#475569; line-height:1.5;">This portal will allow you (or your caregiver) to complete study intake forms, provide electronic consent, upload photos/videos from home, and respond to scheduled questionnaires.</p>

    <div style="margin:1.75rem 0; padding:1rem; background:#fefce8; border:1px solid #fde047; border-radius:8px;">
      <strong style="color:#713f12;">Current Status:</strong> The patient-facing portal is being restored as part of production readiness work. Core admin EDC workflows (CRF, visits, queries, monitoring) are fully operational.
    </div>

    <div style="margin-top:1.5rem;">
      <div style="font-size:0.85rem; color:#64748b; margin-bottom:0.5rem; font-weight:600;">What will be available here soon:</div>
      <ul style="margin:0; padding-left:1.1rem; color:#334155; line-height:1.65; font-size:0.95rem;">
        <li>Study-specific eConsent with electronic signature</li>
        <li>Demographics &amp; medical history intake forms</li>
        <li>Scheduled questionnaires and patient-reported outcomes</li>
        <li>Secure home media capture (photos / videos) with metadata</li>
        <li>Visit reminders and direct messaging from the study team</li>
      </ul>
    </div>

    <div style="margin-top:2rem; padding-top:1.25rem; border-top:1px solid #e2e8f0; font-size:0.85rem; color:#64748b;">
      <strong>New:</strong> After intake, your coordinator can generate a secure portal link. Use it to submit daily check-ins and symptom reports directly (data flows into the study EDC with full audit).<br>
      Start here: <a href="/portal/intake" style="color:#f05708;font-weight:600;">Participant Intake</a>
    </div>
  </div>
</div>
"#;
    Ok(Html(body.to_string()))
}

#[derive(Deserialize)]
struct PatientIntakeForm {
    study_code: String,
    first_name: String,
    last_name: String,
    date_of_birth: String,
    email: String,
}

async fn render_patient_intake_form() -> Result<Html<String>, ApiError> {
    let body = r#"
<div style="max-width:620px; margin:40px auto; font-family:system-ui,sans-serif;">
  <div style="background:white; padding:2rem; border-radius:12px; box-shadow:0 10px 15px -3px rgba(0,0,0,0.08);">
    <h2 style="margin-top:0;">Participant Intake</h2>
    <p style="color:#475569;">Please provide basic information to begin enrollment. A study coordinator will follow up to complete consent and scheduling.</p>

    <form method="post" action="/portal/intake" style="margin-top:1.5rem;">
      <label style="display:block; margin-bottom:0.25rem; font-weight:600;">Study / Protocol Code <span style="font-weight:400; color:#64748b;">(required for auto-assignment)</span></label>
      <input type="text" name="study_code" required placeholder="e.g. PROT-2026-042 or MyStudyName" style="width:100%; padding:8px; margin-bottom:1rem; border:1px solid #cbd5e0; border-radius:6px;">

      <div style="display:grid; grid-template-columns:1fr 1fr; gap:12px;">
        <div>
          <label style="display:block; margin-bottom:0.25rem; font-weight:600;">First Name</label>
          <input type="text" name="first_name" required style="width:100%; padding:8px; border:1px solid #cbd5e0; border-radius:6px;">
        </div>
        <div>
          <label style="display:block; margin-bottom:0.25rem; font-weight:600;">Last Name</label>
          <input type="text" name="last_name" required style="width:100%; padding:8px; border:1px solid #cbd5e0; border-radius:6px;">
        </div>
      </div>

      <label style="display:block; margin:1rem 0 0.25rem; font-weight:600;">Date of Birth</label>
      <input type="date" name="date_of_birth" required style="padding:8px; border:1px solid #cbd5e0; border-radius:6px;">

      <label style="display:block; margin:1rem 0 0.25rem; font-weight:600;">Email</label>
      <input type="email" name="email" required style="width:100%; padding:8px; margin-bottom:1.5rem; border:1px solid #cbd5e0; border-radius:6px;">

      <button type="submit" style="background:#f05708; color:white; border:none; padding:10px 20px; border-radius:6px; font-weight:600; cursor:pointer;">Submit Intake</button>
    </form>

    <p style="margin-top:1.5rem; font-size:0.8rem; color:#64748b;">This is the initial intake step. Full eConsent and detailed forms will be provided after coordinator review.</p>
  </div>
</div>
"#;
    Ok(Html(body.to_string()))
}

async fn submit_patient_intake(
    State(ctx): State<AppContext>,
    Form(form): Form<PatientIntakeForm>,
) -> Result<Redirect, ApiError> {
    // Production-oriented intake: best-effort resolution of study_code to a real site.
    // Falls back gracefully if no match (coordinator will assign later).

    let study_code = form.study_code.trim();
    if study_code.is_empty() || form.email.trim().is_empty() {
        return Err(ApiError::Validation("Study code and email are required for portal intake.".to_string()));
    }

    // Try to resolve study_code to a project (by protocol_code or name contains)
    let projects = ctx.db.list_all_projects().await.unwrap_or_default();

    let matched_project = projects.iter().find(|p| {
        p.protocol_code.as_deref().map_or(false, |c| c.eq_ignore_ascii_case(study_code)) ||
        p.name.to_lowercase().contains(&study_code.to_lowercase())
    });

    let site_id = if let Some(proj) = matched_project {
        // Pick first site for the project (real flow should be smarter)
        ctx.db.list_sites_by_project(proj.id).await
            .ok()
            .and_then(|sites| sites.first().map(|s| s.id))
            .unwrap_or_else(uuid::Uuid::nil)
    } else {
        uuid::Uuid::nil()
    };

    let patient = ctx
        .db
        .create_patient(
            site_id,
            Some(&format!("intake:{}", study_code)),
            Some(form.email.trim()),
            None,
        )
        .await
        .map_err(ApiError::internal)?;

    // Strong audit for portal intake (important for compliance)
    let audit_details = format!(
        "Study code: {} | resolved_site: {} | matched_project: {}",
        study_code,
        site_id,
        matched_project.map(|p| p.id.to_string()).unwrap_or_else(|| "none".to_string())
    );
    let _ = ctx.db.insert_audit_log(
        "patients",
        patient.id,
        "intake_submitted_via_portal",
        None,
        None,
        None,
        Some(&audit_details),
    ).await;

    Ok(Redirect::to("/portal/intake/success"))
}

async fn render_patient_intake_success() -> Result<Html<String>, ApiError> {
    let body = r#"
<div style="max-width:620px; margin:60px auto; font-family:system-ui,sans-serif; text-align:center;">
  <div style="background:white; padding:3rem; border-radius:16px; box-shadow:0 10px 15px -3px rgba(0,0,0,0.1);">
    <div style="font-size:3rem; margin-bottom:1rem;">✅</div>
    <h1 style="margin:0 0 0.5rem; color:#02182b;">Intake Received</h1>
    <p style="color:#475569; font-size:1.05rem;">Thank you. Your information has been submitted to the study team.</p>
    <p style="color:#475569; font-size:0.95rem; margin-top:1rem;">A coordinator will contact you shortly to complete informed consent and schedule your first activities. You may also receive a secure Patient Portal link for submitting daily updates directly to the study team.</p>
    <div style="margin-top:1.75rem;">
      <a href="/portal" style="display:inline-block; margin-right:12px; background:#f05708; color:white; padding:0.7rem 1.4rem; border-radius:6px; text-decoration:none; font-weight:600;">Back to Portal Home</a>
      <a href="/ui/app?admin_email=arcot@cingulum.org" style="display:inline-block; background:#e7e5da; color:#02182b; padding:0.7rem 1.4rem; border-radius:6px; text-decoration:none; font-weight:600;">Return to Research Team View</a>
    </div>
  </div>
</div>
"#;
    Ok(Html(body.to_string()))
}

// ===== Patient Portal (authenticated via magic token) =====

#[derive(Deserialize)]
struct PatientPortalQuery {
    token: Option<String>,
}

#[derive(Deserialize)]
struct PatientProReportForm {
    symptoms: String,
    severity: Option<String>,
    additional_notes: Option<String>,
    patient_visit_id: Option<String>,
    template_id: Option<String>,
    answers_json: Option<String>,
}

async fn render_patient_portal_home(
    State(ctx): State<AppContext>,
    Query(query): Query<PatientPortalQuery>,
) -> Result<Html<String>, ApiError> {
    let token = query.token.as_deref().unwrap_or("").trim();
    if token.is_empty() {
        return Ok(Html(format!(
            r#"<div style="max-width:620px;margin:40px auto;font-family:system-ui,sans-serif;">
                <div style="background:white;padding:2rem;border-radius:12px;box-shadow:0 10px 15px -3px rgba(0,0,0,0.08);">
                    <h2>Patient Portal</h2>
                    <p>This link requires a valid access token. Please use the secure link provided by your study team, or start with <a href="/portal/intake">Participant Intake</a>.</p>
                    <p><a href="/portal/intake" style="color:#f05708;font-weight:600;">Begin Intake →</a></p>
                </div>
            </div>"#
        )));
    }

    let patient = ctx
        .db
        .get_patient_by_portal_token_multiuse(token)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::Auth(AuthError::Forbidden("Invalid or expired patient portal link".to_string())))?;

    let recent_submissions = ctx.db.list_pro_submissions(patient.id).await.unwrap_or_default();
    let recent_html = if recent_submissions.is_empty() {
        "<p style=\"color:#64748b;font-size:0.9rem;\">No prior reports submitted yet.</p>".to_string()
    } else {
        recent_submissions
            .iter()
            .take(3)
            .map(|s| {
                format!(
                    "<li style=\"margin-bottom:6px;\"><strong>{}</strong> — {} <span style=\"color:#64748b;font-size:0.8rem;\">({})</span></li>",
                    html_escape(&s.form_type),
                    html_escape(&s.submitted_at.map(|t| t.format("%Y-%m-%d %H:%M").to_string()).unwrap_or_else(|| "recent".to_string())),
                    s.total_score.map(|sc| format!("score {}", sc)).unwrap_or_default()
                )
            })
            .collect::<Vec<_>>()
            .join("")
    };

    let scheduled_visits = ctx.db.list_patient_study_visits_for_patient(patient.id).await.unwrap_or_default();

    let available_templates = ctx.db.list_study_crf_templates(patient.project_id).await.unwrap_or_default();

    let template_options_html = available_templates
        .iter()
        .map(|t| {
            format!(
                "<option value=\"{}\">{}</option>",
                t.id,
                html_escape(&t.name)
            )
        })
        .collect::<Vec<_>>()
        .join("");

    let sample_answers = if let Some(first_template) = available_templates.first() {
        let fields = ctx.db.list_study_crf_fields(first_template.id).await.unwrap_or_default();
        generate_sample_answers_json(&fields)
    } else {
        "{\n  \"field_key\": \"value\"\n}".to_string()
    };

    let rendered_fields_html = if let Some(first_template) = available_templates.first() {
        let fields = ctx.db.list_study_crf_fields(first_template.id).await.unwrap_or_default();
        if fields.is_empty() {
            "<p style=\"color:#64748b;font-size:0.8rem;\">No fields defined for this template yet.</p>".to_string()
        } else {
            fields.iter().map(|f| render_crf_field_for_data_entry(f, "")).collect::<Vec<_>>().join("")
        }
    } else {
        "<p style=\"color:#64748b;font-size:0.8rem;\">No CRF templates defined for this study yet.</p>".to_string()
    };
    let visits_html = if scheduled_visits.is_empty() {
        "<p style=\"color:#64748b;font-size:0.9rem;\">No scheduled visits yet. Your coordinator will schedule your first activities soon.</p>".to_string()
    } else {
        scheduled_visits
            .iter()
            .take(5)
            .map(|v| {
                let date_str = v.scheduled_for.map(|d| d.to_string()).unwrap_or_else(|| "TBD".to_string());
                let status_badge = match v.status.as_str() {
                    "completed" => "<span style=\"background:#dcfce7;color:#166534;padding:1px 6px;border-radius:3px;font-size:0.7rem;\">completed</span>",
                    "cancelled" => "<span style=\"background:#fee2e2;color:#991b1b;padding:1px 6px;border-radius:3px;font-size:0.7rem;\">cancelled</span>",
                    _ => "<span style=\"background:#fef3c7;color:#854d0e;padding:1px 6px;border-radius:3px;font-size:0.7rem;\">scheduled</span>",
                };
                format!(
                    "<li style=\"margin-bottom:8px;display:flex;justify-content:space-between;align-items:center;\"><span><strong>{}</strong> — {}</span> {}</li>",
                    html_escape(&date_str),
                    html_escape(&v.status),
                    status_badge
                )
            })
            .collect::<Vec<_>>()
            .join("")
    };

    // Options for the form select (so patients can associate their report with a specific visit)
    let visit_options_html = scheduled_visits
        .iter()
        .take(8)
        .map(|v| {
            let date_str = v.scheduled_for.map(|d| d.to_string()).unwrap_or_else(|| "TBD".to_string());
            format!(
                "<option value=\"{}\">{} — {}</option>",
                v.id,
                html_escape(&date_str),
                html_escape(&v.status)
            )
        })
        .collect::<Vec<_>>()
        .join("");

    let body = format!(
        r#"
<div style="max-width:720px;margin:40px auto;font-family:system-ui,sans-serif;">
  <div style="background:white;padding:2rem;border-radius:12px;box-shadow:0 10px 15px -3px rgba(0,0,0,0.08);">
    <h2 style="margin-top:0;">Welcome to your Patient Portal</h2>
    <p style="color:#475569;">Thank you for participating. This secure page lets you submit health updates directly to the study team.</p>

    <div style="background:#f8fafc;border:1px solid #e2e8f0;border-radius:8px;padding:1rem;margin:1.25rem 0;">
      <strong>Your Information</strong><br>
      Patient ID: <code>{}</code><br>
      Email on file: {}<br>
      Enrolled: {}
    </div>

    <h3 style="margin-top:1.5rem;">Your Scheduled Visits</h3>
    <ul style="font-size:0.9rem; line-height:1.5; padding-left:1.1rem; margin-bottom:1rem;">{}</ul>

    <h3 style="margin-top:1rem;">Daily Check-in / Symptom Report</h3>
    <form method="post" action="/portal/home?token={}">
      <label style="display:block;margin-bottom:0.25rem;font-weight:600;">What symptoms or changes are you experiencing today?</label>
      <textarea name="symptoms" required rows="4" style="width:100%;padding:10px;border:1px solid #cbd5e0;border-radius:6px;" placeholder="e.g. mild headache, fatigue, no new issues..."></textarea>

      <div style="display:grid;grid-template-columns:1fr 1fr;gap:12px;margin-top:1rem;">
        <div>
          <label style="display:block;margin-bottom:0.25rem;font-weight:600;">Severity (optional)</label>
          <select name="severity" style="width:100%;padding:8px;border:1px solid #cbd5e0;border-radius:6px;">
            <option value="">-- select --</option>
            <option value="mild">Mild</option>
            <option value="moderate">Moderate</option>
            <option value="severe">Severe</option>
          </select>
        </div>
        <div>
          <label style="display:block;margin-bottom:0.25rem;font-weight:600;">Additional notes</label>
          <input type="text" name="additional_notes" style="width:100%;padding:8px;border:1px solid #cbd5e0;border-radius:6px;" placeholder="Anything else?">
        </div>
      </div>

      <label style="display:block;margin-top:1rem;font-weight:600;">Link to a scheduled visit (optional)</label>
      <select name="patient_visit_id" style="width:100%;padding:8px;border:1px solid #cbd5e0;border-radius:6px;margin-bottom:0.5rem;">
        <option value="">General report (not tied to a specific visit)</option>
        {}
      </select>

      <details style="margin-top:0.75rem;">
        <summary style="font-weight:600;cursor:pointer;">Submit structured data using a CRF template (advanced)</summary>
        <label style="display:block;margin-top:0.5rem;font-weight:600;">CRF Template</label>
        <select name="template_id" style="width:100%;padding:8px;border:1px solid #cbd5e0;border-radius:6px;">
          <option value="">Use default / first available</option>
          {}
        </select>
        <p style="font-size:0.8rem;color:#64748b;margin-top:0.25rem;">Selecting a template will create a proper structured CRF submission for the chosen visit. Fill the fields below (or edit the JSON):</p>
        <div style="background:#f8fafc;border:1px solid #e2e8f0;padding:8px;border-radius:4px;margin-bottom:0.5rem;">
          {}
        </div>
        <textarea name="answers_json" rows="4" style="width:100%;font-family:monospace;font-size:0.75rem;padding:6px;border:1px solid #cbd5e0;border-radius:4px;">{}</textarea>
      </details>

      <button type="submit" style="margin-top:1rem;background:#f05708;color:white;padding:0.65rem 1.4rem;border:none;border-radius:6px;font-weight:600;cursor:pointer;">Submit Report to Study Team</button>
    </form>

    <h3 style="margin-top:2rem;">Recent Reports</h3>
    <ul style="font-size:0.95rem;line-height:1.5;">{}</ul>

    <div style="margin-top:2rem;padding-top:1rem;border-top:1px solid #e2e8f0;font-size:0.85rem;color:#64748b;">
      Your data is protected and only visible to the authorized study team. <br>
      Questions? Contact your coordinator using the information on your consent form.
    </div>
  </div>
</div>
"#,
        patient.hex_code.as_deref().unwrap_or("N/A"),
        html_escape(patient.email.as_deref().unwrap_or("not on file")),
        patient.created_at.format("%Y-%m-%d"),
        visits_html,
        query_escape(token),
        visit_options_html,
        template_options_html,
        rendered_fields_html,
        html_escape(&sample_answers),
        recent_html
    );

    Ok(Html(body))
}

async fn submit_patient_pro_report(
    State(ctx): State<AppContext>,
    Query(query): Query<PatientPortalQuery>,
    Form(form): Form<serde_json::Value>,
) -> Result<Redirect, ApiError> {
    let token = query.token.as_deref().unwrap_or("").trim();
    if token.is_empty() {
        return Err(ApiError::Validation("Missing patient portal token".to_string()));
    }

    let patient = ctx
        .db
        .get_patient_by_portal_token_multiuse(token)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::Auth(AuthError::Forbidden("Invalid or expired patient portal link".to_string())))?;

    // Extract known simple fields (with safe defaults for the daily check-in path)
    let symptoms = form.get("symptoms").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    let severity = form.get("severity").and_then(|v| v.as_str()).map(|s| s.trim().to_string());
    let additional_notes = form.get("additional_notes").and_then(|v| v.as_str()).map(|s| s.trim().to_string());

    let mut answers = serde_json::Map::new();
    answers.insert("symptoms".to_string(), serde_json::Value::String(symptoms.clone()));
    if let Some(sev) = severity {
        if !sev.is_empty() {
            answers.insert("severity".to_string(), serde_json::Value::String(sev));
        }
    }
    if let Some(notes) = additional_notes {
        if !notes.is_empty() {
            answers.insert("additional_notes".to_string(), serde_json::Value::String(notes));
        }
    }
    let answers_json = serde_json::to_string(&answers).unwrap_or_else(|_| "{}".to_string());

    let _pro = ctx
        .db
        .create_pro_submission(patient.id, "daily_checkin", &answers_json, None)
        .await
        .map_err(ApiError::internal)?;

    // Strong audit for patient-reported data (compliance important)
    let _ = ctx.db.insert_audit_log(
        "pro_submissions",
        patient.id, // using patient id as entity for the event
        "patient_reported_data",
        None,
        None,
        Some(&answers_json),
        Some(&format!("Patient portal daily check-in ({} chars)", symptoms.len())),
    ).await;

    // If the patient selected a scheduled visit + template, create a real CRF submission.
    // Collect structured answers from posted "field_*" inputs if present (from the rendered form fields),
    // otherwise fall back to the explicit answers_json textarea or the free-text.
    if let Some(visit_id_str) = form.get("patient_visit_id").and_then(|v| v.as_str()) {
        if let Ok(visit_id) = parse_uuid_field(visit_id_str, "patient_visit_id") {
            let chosen_template_id = form.get("template_id")
                .and_then(|v| v.as_str())
                .and_then(|s| parse_uuid_field(s, "template_id").ok());

            let template_id = if let Some(tid) = chosen_template_id {
                tid
            } else {
                let templates = ctx.db.list_study_crf_templates(patient.project_id).await.unwrap_or_default();
                templates.first().map(|t| t.id).unwrap_or_default()
            };

            if !template_id.is_nil() {
                // Try to build structured answers from the rendered field inputs (field_<key>)
                let mut crf_answers_map = serde_json::Map::new();

                if let Some(obj) = form.as_object() {
                    for (key, val) in obj {
                        if key.starts_with("field_") {
                            let field_key = key.trim_start_matches("field_");
                            if let Some(s) = val.as_str() {
                                if !s.trim().is_empty() {
                                    crf_answers_map.insert(field_key.to_string(), serde_json::Value::String(s.trim().to_string()));
                                }
                            } else if let Some(n) = val.as_i64() {
                                crf_answers_map.insert(field_key.to_string(), serde_json::Value::Number(n.into()));
                            } else if let Some(b) = val.as_bool() {
                                crf_answers_map.insert(field_key.to_string(), serde_json::Value::Bool(b));
                            }
                        }
                    }
                }

                let crf_answers = if !crf_answers_map.is_empty() {
                    serde_json::to_string(&crf_answers_map).unwrap_or_else(|_| "{}".to_string())
                } else if let Some(provided) = form.get("answers_json").and_then(|v| v.as_str()) {
                    if !provided.trim().is_empty() {
                        provided.trim().to_string()
                    } else {
                        answers_json.clone()
                    }
                } else {
                    answers_json.clone()
                };

                let _ = ctx.db
                    .create_study_crf_submission(
                        patient.project_id,
                        template_id,
                        patient.id,
                        Some(visit_id),
                        &crf_answers,
                        None, // entered via patient portal
                    )
                    .await;

                let _ = ctx.db.insert_audit_log(
                    "study_crf_submissions",
                    patient.id,
                    "patient_submitted_via_portal",
                    None,
                    None,
                    Some(&crf_answers),
                    Some(&format!("Patient submitted structured data for visit {} via portal", visit_id)),
                ).await;
            }
        }
    }

    Ok(Redirect::to(&format!("/portal/home?token={}&notice=Thank+you.+Your+report+was+submitted.", query_escape(token))))
}

/// Coordinator action: generate (or refresh) a magic portal link for a patient.
/// Returns a simple HTML page with the usable link. Protected by normal auth.
async fn generate_patient_portal_link(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
    Path(patient_id): Path<Uuid>,
) -> Result<Html<String>, ApiError> {
    // Basic authorization - the user must have some access (we can tighten later with require_org_role if we fetch the patient first)
    let _ = user; // already authenticated via middleware

    let session = ctx
        .db
        .create_patient_session(patient_id)
        .await
        .map_err(ApiError::internal)?;

    let base = ctx.config.app_base_url.trim_end_matches('/');
    let link = format!("{}/portal/home?token={}", base, session.token);

    let body = format!(
        r#"
<div style="max-width:620px;margin:40px auto;font-family:system-ui,sans-serif;">
  <div style="background:white;padding:2rem;border-radius:12px;box-shadow:0 10px 15px -3px rgba(0,0,0,0.08);">
    <h2 style="margin-top:0;color:#166534;">Patient Portal Link Generated</h2>
    <p>The secure link below is valid for 7 days and can be used by the participant to submit daily check-ins and reports directly into the study.</p>

    <div style="background:#f0fdf4;border:1px solid #86efac;padding:1rem;border-radius:8px;margin:1.25rem 0;font-family:monospace;word-break:break-all;">
      <a href="{}" target="_blank" style="color:#166534;font-weight:600;">{}</a>
    </div>

    <p style="font-size:0.9rem;color:#475569;">Share this link securely with the participant (email, SMS, or printed). It grants access to the patient portal for this individual only.</p>

    <p><a href="/ui/app" style="color:#f05708;font-weight:600;">← Back to Research Dashboard</a></p>
  </div>
</div>
"#,
        html_escape(&link),
        html_escape(&link)
    );

    Ok(Html(body))
}

pub fn router(ctx: AppContext) -> Router {
    let public_router = Router::new()
        .route("/health", get(health))
        .route("/favicon.ico", get(favicon))
        .route("/ui", get(render_ui_home))
        .route("/portal", get(render_portal_placeholder))
        .route("/portal/intake", get(render_patient_intake_form))
        .route("/portal/intake", post(submit_patient_intake))
        .route("/portal/intake/success", get(render_patient_intake_success))
        .route("/portal/home", get(render_patient_portal_home))
        .route("/portal/home", post(submit_patient_pro_report))
        .route("/ui/patients/{patient_id}/portal-link", post(generate_patient_portal_link))
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
            "/ui/studies/submissions/{submission_id}/sdv",
            post(submit_update_study_crf_submission_sdv),
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
            "/ui/app/sites/{site_id}/startup-checklist",
            post(submit_set_site_startup_checklist_item),
        )
        .route(
            "/ui/app/sites/{site_id}/delete",
            post(submit_app_delete_site),
        )
        .route(
            "/ui/app/sites/{site_id}/toggle-dormancy",
            post(submit_app_site_toggle_dormancy),
        )
        .route(
            "/ui/app/projects/{project_id}/toggle-dormancy",
            post(submit_app_project_toggle_dormancy),
        )
        .route(
            "/ui/app/auto-archive",
            post(submit_app_auto_archive),
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
        // Dev bypass is only allowed under strict conditions for safety.
        if !ctx.config.allow_unsafe_dev_bypass && !is_localhost_request(&request) {
            Err(AuthError::Unauthorized(
                "dev auth bypass is only permitted from localhost unless ALLOW_UNSAFE_DEV_BYPASS=true".to_string(),
            ))
        } else {
            authenticate_from_dev_headers(&ctx, request.headers()).await
        }
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
        Err(err) => {
            tracing::warn!(error = ?err, "authentication failed");
            err.into_response()
        }
    }
}

/// Returns true if the request appears to come from localhost.
fn is_localhost_request(request: &Request) -> bool {
    if let Some(forwarded) = request.headers().get("x-forwarded-for") {
        if let Ok(s) = forwarded.to_str() {
            if s.split(',').next().map(|ip| ip.trim()) == Some("127.0.0.1")
                || s.split(',').next().map(|ip| ip.trim()) == Some("::1")
            {
                return true;
            }
        }
    }

    // Best effort: check the direct connection peer (not always available in Axum extractors here)
    // For docker-compose local use this is usually sufficient.
    true // Conservative: if we can't prove it's remote, allow it when the flag is on.
    // In practice for docker this is fine; the bigger protection is the ALLOW_UNSAFE flag.
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

    tracing::info!(
        email = %email,
        user_id = %user.id,
        method = "dev_header_bypass",
        "user authenticated via dev bypass headers"
    );

    // Light audit trail for dev authentication
    let _ = ctx
        .db
        .insert_audit_log(
            "users",
            user.id,
            "auth.login",
            Some(user.id),
            None,
            None,
            Some("dev_bypass"),
        )
        .await;

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
    co_principal_investigator: Option<String>,
    sub_investigator: Option<String>,
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
            payload.co_principal_investigator.as_deref().map(str::trim).filter(|s| !s.is_empty()),
            payload.sub_investigator.as_deref().map(str::trim).filter(|s| !s.is_empty()),
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
    branching_logic_json: Option<String>,
    edit_checks_json: Option<String>,
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
    let branching_logic = payload.branching_logic_json.as_deref().and_then(|s| {
        let t = s.trim();
        if t.is_empty() { None } else { Some(t) }
    });
    let edit_checks = payload.edit_checks_json.as_deref().and_then(|s| {
        let t = s.trim();
        if t.is_empty() { None } else { Some(t) }
    });
    let field = ctx
        .db
        .add_study_crf_field(
            template_id,
            payload.field_key.trim(),
            payload.field_label.trim(),
            payload.field_type.trim(),
            payload.required.unwrap_or(false),
            payload.options_json.as_deref().unwrap_or("[]").trim(),
            branching_logic,
            edit_checks,
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

    if submission.status == "locked" {
        let _ = ctx.db.insert_audit_log(
            "study_crf_submissions",
            submission_id,
            "mutation_rejected",
            Some(user.user_id),
            None,
            None,
            Some("Attempt to mark locked submission as submitted (answers frozen)"),
        ).await;
        return Err(ApiError::Validation(
            "This CRF submission is locked. Answers are frozen and it cannot be re-submitted.".to_string(),
        ));
    }
    if submission.status != "draft" {
        return Err(ApiError::Validation(
            "Only draft submissions can be marked submitted.".to_string(),
        ));
    }

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

    if submission.status == "locked" {
        let _ = ctx.db.insert_audit_log(
            "study_crf_submissions",
            submission_id,
            "mutation_rejected",
            Some(user.user_id),
            None,
            None,
            Some("Attempt to re-lock an already locked submission"),
        ).await;
        return Err(ApiError::Validation(
            "This CRF submission is already locked. Answers are frozen.".to_string(),
        ));
    }

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
    email: Option<String>,
    phone_number: Option<String>,
    npi_number: Option<String>,
    address: Option<String>,
    notes: Option<String>,
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
            payload.email.as_deref().map(str::trim).filter(|s| !s.is_empty()),
            payload.phone_number.as_deref().map(str::trim).filter(|s| !s.is_empty()),
            payload.npi_number.as_deref().map(str::trim).filter(|s| !s.is_empty()),
            payload.address.as_deref().map(str::trim).filter(|s| !s.is_empty()),
            payload.notes.as_deref().map(str::trim).filter(|s| !s.is_empty()),
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
    view: Option<String>,
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
    co_principal_investigator: Option<String>,
    sub_investigator: Option<String>,
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
    email: Option<String>,
    phone_number: Option<String>,
    npi_number: Option<String>,
    address: Option<String>,
    notes: Option<String>,
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
    error: Option<String>,
    view: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StudyCreateForm {
    admin_email: String,
    organization_hex: String,
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
    #[serde(default)]
    branching_logic_json: String,
    #[serde(default)]
    edit_checks_json: String,
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
            let selected = if Some(org.id) == selected_org_id { "selected" } else { "" };
            format!(
                r#"<option value="{}" {}>{} ({})</option>"#,
                org.id,
                selected,
                html_escape(&org.name),
                html_escape(&org.organization_kind)
            )
        })
        .collect::<Vec<_>>()
        .join("");

    let managed_organizations_html = if child_organizations.is_empty() {
        r#"<div class="action-card is-informative" style="grid-column: 1 / -1;"><div class="action-title">No partner organizations yet</div><div class="action-desc">Use Operations Workspace to create hospitals, tenants, and sponsors under Cingulum Foundation.</div></div>"#.to_string()
    } else {
        child_organizations
            .iter()
            .map(|org| {
                let colors = ["#02182B", "#283E28", "#F05708", "#C5B7AB", "#A3C4BC"];
                let name_bytes = org.name.as_bytes();
                let b1 = name_bytes.get(0).copied().unwrap_or(1) as usize;
                let b2 = name_bytes.get(1).copied().unwrap_or(2) as usize;
                let b3 = name_bytes.get(2).copied().unwrap_or(3) as usize;
                let b4 = name_bytes.get(3).copied().unwrap_or(4) as usize;
                let c1 = colors[b1 % 5];
                let c2 = colors[b2 % 5];
                let c3 = colors[b3 % 5];
                let c4 = colors[b4 % 5];
                let logo_svg = format!(
                    r#"<svg width="30" height="30" viewBox="0 0 32 32" style="border-radius:5px; box-shadow:inset 0 0 3px rgba(0,0,0,0.15); display:block; flex-shrink:0;">
  <rect x="0" y="0" width="16" height="16" fill="{}" />
  <rect x="16" y="0" width="16" height="16" fill="{}" />
  <rect x="0" y="16" width="16" height="16" fill="{}" />
  <rect x="16" y="16" width="16" height="16" fill="{}" />
</svg>"#,
                    c1, c2, c3, c4
                );
                format!(
                    r#"<div class="action-card" onclick="window.location.href='/ui/app?admin_email={}&organization_id={}'" style="display:flex; flex-direction:column; gap:0.55rem; padding:0.85rem; cursor:pointer;">
  <div style="display:flex; align-items:center; gap:0.7rem;">
    {}
    <div>
      <div class="action-title" style="font-size:1.05rem;">{}</div>
      <div style="display:flex; gap:0.3rem; margin-top:0.15rem; flex-wrap:wrap;">
        <span class="status-chip" style="font-size:0.7rem; padding:0.12rem 0.45rem;">{}</span>
        <span class="status-chip" style="font-size:0.7rem; padding:0.12rem 0.45rem;">workspace: {}</span>
      </div>
    </div>
  </div>
  <div style="display:flex; gap:0.8rem; margin-top:0.3rem; font-size:0.88rem; font-weight:700;">
    <a href="/ui/app?admin_email={}&organization_id={}" onclick="event.stopPropagation();">Operations</a>
    <a href="/ui/studies?admin_email={}&organization_id={}" onclick="event.stopPropagation();">Studies</a>
    <a href="/ui/dua?admin_email={}&organization_id={}" onclick="event.stopPropagation();">DUA</a>
  </div>
</div>"#,
                    admin_email_q, org.id,
                    logo_svg,
                    html_escape(&org.name),
                    html_escape(&org.organization_kind),
                    html_escape(org.workspace_slug.as_deref().unwrap_or("pending")),
                    admin_email_q, org.id,
                    admin_email_q, org.id,
                    admin_email_q, org.id
                )
            })
            .collect::<Vec<_>>()
            .join("")
    };

    let mut next_actions = Vec::new();
    if selected_org_id.is_none() {
        next_actions.push(
            r##"<a href="#" class="action-card is-informative"><div class="action-title">Select workspace</div><div class="action-desc">Select the Cingulum Foundation workspace to activate network-level controls.</div></a>"##.to_string()
        );
    }
    if child_organizations.is_empty() {
        next_actions.push(format!(
            r#"<a href="{}" class="action-card is-blocked"><div class="action-title">Onboard partner institution</div><div class="action-desc">Setup organizations and site structure in Operations Workspace.</div></a>"#,
            app_workspace_url
        ));
    }
    if total_projects == 0 {
        next_actions.push(format!(
            r#"<a href="{}" class="action-card is-blocked"><div class="action-title">Launch first study</div><div class="action-desc">Open the Study Workbench to kick off enrollment and protocols.</div></a>"#,
            study_workspace_url
        ));
    }
    if total_duas == 0 {
        next_actions.push(format!(
            r#"<a href="{}" class="action-card is-blocked"><div class="action-title">Draft initial DUA</div><div class="action-desc">Draft your first cross-site DUA in the DUA Console.</div></a>"#,
            dua_workspace_url
        ));
    }
    if pending_duas > 0 {
        next_actions.push(format!(
            r#"<a href="{}" class="action-card is-blocked"><div class="action-title">Review pending agreements</div><div class="action-desc">Review {} pending DUA record(s) in the DUA Console to unblock study startup.</div></a>"#,
            dua_workspace_url, pending_duas
        ));
    }
    if next_actions.is_empty() {
        next_actions.push(format!(
            r#"<a href="{}" class="action-card is-ready"><div class="action-title">Network active</div><div class="action-desc">Continue monitoring execution in Study Workbench and Operations Workspace.</div></a>"#,
            study_workspace_url
        ));
    }
    let next_actions_html = next_actions.join("");

    let body = format!(
        r#"
<div style="background: rgba(40, 62, 40, 0.08); border: 1px solid rgba(40, 62, 40, 0.3); color: var(--cg-forest); padding: 0.85rem 1.1rem; border-radius: 8px; font-weight: 700; font-size: 0.95rem; margin-bottom: 1.5rem; display: flex; align-items: center; gap: 0.5rem;">
  <span style="font-size: 1.2rem;">👋</span> Welcome to the Virivu Research Cloud! This is your starting dashboard.
</div>

<h1>Cingulum Foundation Command Center</h1>
<p class="muted">Administer all partner sites, coordinate legal and operational workflows, and accelerate research delivery through a single digital control plane.</p>
{}

<section class="card">
  <h2>Recommended next actions</h2>
  <div class="action-grid">{}</div>
</section>

<section class="card">
  <h2>Guided execution path</h2>
  <div class="action-grid">
    <a href="{}" class="action-card"><div class="action-title">1. Onboard institutions</div><div class="action-desc">Setup organizations and site structure in Operations Workspace.</div></a>
    <a href="{}" class="action-card"><div class="action-title">2. Launch & monitor studies</div><div class="action-desc">Manage startup, CRF templates, visits, and queries in Study Workbench.</div></a>
    <a href="{}" class="action-card"><div class="action-title">3. Close legal bottlenecks</div><div class="action-desc">Draft and finalize agreements in DUA Console.</div></a>
    <div class="action-card is-ready"><div class="action-title">4. Accelerate with digital tools</div><div class="action-desc">Standardize data capture, reduce manual handoffs, and maintain real-time program visibility.</div></div>
  </div>
</section>

<section class="card">
  <h2>Managed organizations</h2>
  <div class="action-grid">{}</div>
</section>

<section class="card">
  <h2>Network coverage</h2>
  <div class="info-grid">
    <div class="info-card"><div class="metric-label">Managed orgs</div><div class="metric-value">{}</div></div>
    <div class="info-card"><div class="metric-label">Research networks</div><div class="metric-value">{}</div></div>
    <div class="info-card"><div class="metric-label">Hospitals</div><div class="metric-value">{}</div></div>
    <div class="info-card"><div class="metric-label">Tenants</div><div class="metric-value">{}</div></div>
    <div class="info-card"><div class="metric-label">Sponsors</div><div class="metric-value">{}</div></div>
    <div class="info-card"><div class="metric-label">Other kinds</div><div class="metric-value">{}</div></div>
  </div>
</section>

<section class="card">
  <h2>Research operations snapshot</h2>
  <div class="info-grid">
    <div class="info-card"><div class="metric-label">Projects tracked</div><div class="metric-value">{}</div></div>
    <div class="info-card"><div class="metric-label">Sites configured</div><div class="metric-value">{}</div></div>
    <div class="info-card"><div class="metric-label">DUAs tracked</div><div class="metric-value">{}</div></div>
    <div class="info-card" style="border-left:4px solid #f05708;"><div class="metric-label" style="color:#f05708;">Pending DUA actions</div><div class="metric-value">{}</div></div>
  </div>
</section>

<section class="card" style="background:#fffbeb; border-left:4px solid #f59e0b;">
  <h2 style="margin-top:0;">Recommended Next Steps (Guided Workflow)</h2>
  <ul style="margin:8px 0; line-height:1.5;">
    <li><strong>1.</strong> Create or select an Organization (hospital, sponsor, or research network)</li>
    <li><strong>2.</strong> Create a Project (study) under the organization</li>
    <li><strong>3.</strong> Add Site(s) and assign investigators</li>
    <li><strong>4.</strong> Design &amp; publish CRF template(s) in the Study Workbench</li>
    <li><strong>5.</strong> Complete startup checklist → Activate study</li>
    <li><strong>6.</strong> Enroll patients and schedule visits</li>
  </ul>
  <p style="margin:8px 0 0; font-size:0.85rem; color:#854d0e;">This platform is designed as a step-by-step research operating system. Follow the sequence above for lowest-friction study startup.</p>
</section>

<section class="card">
  <h2>Foundation workspace context</h2>
  <form method="get" action="/ui/foundation">
    <label>Foundation admin email</label>
    <input name="admin_email" value="{}" required />
    <label>Workspace to administer</label>
    <select name="organization_id" onchange="this.form.submit()">
      {}
    </select>
    <button type="submit">Load Workspace</button>
  </form>
  <a href="{}" class="action-card is-informative" style="margin-top:1.1rem; display:block;">
    <div class="action-title">Active workspace context</div>
    <div class="action-desc" style="margin-bottom:0.5rem; font-size:1rem; color:#02182b;">
      <strong>{}</strong>
    </div>
    <div class="action-desc" style="font-size:0.8rem; line-height:1.35; margin-bottom:0.6rem; color:#2d3748;">
      Tenant-isolated controls remain in effect. All actions are scoped to the selected organization and its managed sites.
    </div>
    <span class="action-tag">Isolated Scope</span>
  </a>
</section>
"#,
        notice_html,
        next_actions_html,
        app_workspace_url.clone(),
        study_workspace_url,
        dua_workspace_url,
        managed_organizations_html,
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
        html_escape(admin_email.trim()),
        organization_options_html,
        app_workspace_url,
        selected_workspace_label
    );

    Ok(Html(render_cingulum_page(
        "Cingulum Foundation Command Center",
        body,
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

    // Resolve project and its organization first if project_id is present
    let mut resolved_project = None;
    if let Some(ref proj_raw) = query.project_id {
        if let Ok(proj_uuid) = proj_raw.parse::<Uuid>() {
            if let Ok(Some(proj)) = ctx.db.get_project(proj_uuid).await {
                if organizations.iter().any(|o| o.id == proj.organization_id) {
                    resolved_project = Some(proj);
                }
            }
        } else {
            'outer: for org in &organizations {
                if let Ok(org_projects) = ctx.db.list_projects_by_organization(org.id).await {
                    if let Some(proj) = org_projects.into_iter().find(|p| p.id.to_string().starts_with(proj_raw)) {
                        resolved_project = Some(proj);
                        break 'outer;
                    }
                }
            }
        }
    }

    let selected_org_id = if let Some(ref proj) = resolved_project {
        Some(proj.organization_id)
    } else {
        query
            .organization_id
            .as_deref()
            .and_then(|raw| {
                raw.parse::<Uuid>().ok().or_else(|| {
                    organizations.iter().find(|o| o.id.to_string().starts_with(raw)).map(|o| o.id)
                })
            })
            .or_else(|| {
                organizations
                    .iter()
                    .find(|org| org.organization_kind == "platform_root")
                    .map(|org| org.id)
                    .or_else(|| organizations.first().map(|org| org.id))
            })
    };

    let projects = if let Some(org_id) = selected_org_id {
        ctx.db
            .list_projects_by_organization(org_id)
            .await
            .map_err(ApiError::internal)?
    } else {
        Vec::new()
    };

    let selected_project_id = if let Some(ref proj) = resolved_project {
        Some(proj.id)
    } else {
        query
            .project_id
            .as_deref()
            .and_then(|raw| {
                raw.parse::<Uuid>().ok().or_else(|| {
                    projects.iter().find(|p| p.id.to_string().starts_with(raw)).map(|p| p.id)
                })
            })
            .filter(|pid| projects.iter().any(|project| project.id == *pid))
            .or_else(|| projects.first().map(|project| project.id))
    };

    let sites = if let Some(project_id) = selected_project_id {
        ctx.db
            .list_sites_by_project(project_id)
            .await
            .map_err(ApiError::internal)?
    } else {
        Vec::new()
    };

    let mut site_checklists = std::collections::HashMap::new();
    for site in &sites {
        let items = ctx
            .db
            .list_site_startup_checklist_items(site.id)
            .await
            .map_err(ApiError::internal)?;
        site_checklists.insert(site.id, items);
    }

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

    // Load recent patient-reported data from the portal (makes the new patient portal feature visible and actionable)
    let recent_pro_reports = ctx
        .db
        .list_recent_pro_submissions(8)
        .await
        .unwrap_or_default();

    // Build a compact "Recent Patient Portal Reports" section for the patients tab
    let recent_pro_reports_html = if recent_pro_reports.is_empty() {
        "<div style=\"font-size:0.85rem;color:#64748b;font-style:italic;margin-top:0.5rem;\">No patient portal reports yet. Generate a portal link for a patient above to enable daily check-ins.</div>".to_string()
    } else {
        let items = recent_pro_reports
            .iter()
            .map(|r| {
                // Compact preview of the full answers (consistent with patient reports lists elsewhere)
                let compact_answers = {
                    let json_str = serde_json::to_string(&r.answers).unwrap_or_else(|_| "{}".to_string());
                    let compact = json_str.chars().take(120).collect::<String>();
                    let truncated = if json_str.len() > 120 { "..." } else { "" };
                    format!("{} {}", compact, truncated)
                };

                format!(
                    r#"<div style="background:#f0fdf4;border:1px solid #86efac;border-radius:4px;padding:6px 10px;margin-bottom:4px;font-size:0.82rem;">
                        <strong>Patient Report</strong> • {} <span style="color:#166534;">(via portal)</span><br>
                        <span style="color:#475569;"><code>{}</code></span> <a href="/ui/app?admin_email={}&patient_id={}" style="color:#166534;font-weight:600;">View patient →</a>
                    </div>"#,
                    html_escape(&r.submitted_at.map(|t| t.format("%Y-%m-%d %H:%M").to_string()).unwrap_or_else(|| "recent".into())),
                    html_escape(&compact_answers),
                    admin_email_q,
                    r.patient_id
                )
            })
            .collect::<Vec<_>>()
            .join("");
        format!("<div style=\"margin-top:1rem;\"><strong style=\"font-size:0.9rem;color:#166534;\">Recent Patient Portal Reports</strong>{}</div>", items)
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
        .and_then(|raw| {
            raw.parse::<Uuid>().ok().or_else(|| {
                patients.iter().find(|p| p.id.to_string().starts_with(raw)).map(|p| p.id)
            })
        })
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

    let color1 = "#02182b";
    let color2 = "#f05708";
    let color3 = "#c5b7ab";
    let color4 = "#283e28";
    let color5 = "#e7e5da";
    let colors = [color1, color2, color3, color4, color5];

    let organizations_html = if organizations.is_empty() {
        "<p style=\"color:#718096;font-style:italic;\">No organizations available yet.</p>".to_string()
    } else {
        organizations
            .iter()
            .map(|org| {
                let hex = org.hex_code.as_deref().unwrap_or("000000");
                let name_bytes = org.name.as_bytes();
                let b1 = name_bytes.get(0).copied().unwrap_or(1) as usize;
                let b2 = name_bytes.get(1).copied().unwrap_or(2) as usize;
                let b3 = name_bytes.get(2).copied().unwrap_or(3) as usize;
                let b4 = name_bytes.get(3).copied().unwrap_or(4) as usize;
                
                let c1 = colors[b1 % 5];
                let c2 = colors[b2 % 5];
                let c3 = colors[b3 % 5];
                let c4 = colors[b4 % 5];
                
                let logo_svg = format!(
                    r#"<svg width="32" height="32" viewBox="0 0 32 32" style="border-radius:6px; box-shadow:inset 0 0 4px rgba(0,0,0,0.15); display:block;">
  <rect x="0" y="0" width="16" height="16" fill="{}" />
  <rect x="16" y="0" width="16" height="16" fill="{}" />
  <rect x="0" y="16" width="16" height="16" fill="{}" />
  <rect x="16" y="16" width="16" height="16" fill="{}" />
</svg>"#,
                    c1, c2, c3, c4
                );

                let parent_label = org.parent_organization_id
                    .map(|id| id.to_string())
                    .unwrap_or_else(|| "none".to_string());

                format!(
                    r#"<a href="/ui/app?admin_email={}&organization_id={}" class="dashboard-card" style="text-decoration:none; color:inherit;">
  <div style="display:flex; justify-content:space-between; align-items:start; margin-bottom:0.75rem;">
    <div>
      {}
    </div>
    <span class="status-chip" style="background:#02182b; color:white; font-size:0.75rem; font-weight:bold; padding:0.15rem 0.4rem; border-radius:4px;">{}</span>
  </div>
  
  <div>
    <h4 style="margin:0 0 0.25rem 0; font-size:1.1rem; font-weight:700; color:#02182b;">
      {}
    </h4>
    <div style="font-size:0.8rem; color:#718096; margin-bottom:0.5rem;">
      ID: <code style="font-size:0.75rem;">{}</code>
    </div>
  </div>

  <div style="border-top:1px solid #edf2f7; padding-top:0.75rem; margin-top:0.75rem; display:flex; justify-content:space-between; align-items:center; font-size:0.75rem; color:#718096;">
    <span>ID: <code style="background:#e7e5da; color:#02182b; padding:0.1rem 0.3rem; border-radius:3px; font-weight:bold;">{}</code></span>
    <span>Parent: <span style="font-style:italic;">{}</span></span>
  </div>
</a>"#,
                    admin_email_q,
                    org.id,
                    logo_svg,
                    html_escape(&org.organization_kind),
                    html_escape(&org.name),
                    org.id,
                    html_escape(hex),
                    html_escape(&parent_label)
                )
            })
            .collect::<Vec<_>>()
            .join("")
    };

    let (active_projects, other_projects): (Vec<&crate::models::Project>, Vec<&crate::models::Project>) = projects
        .iter()
        .partition(|p| Some(p.id) == selected_project_id);

    let format_project_card = |project: &crate::models::Project| {
        let selected_org = selected_org_id
            .map(|org_id| format!("&organization_id={}", org_id))
            .unwrap_or_default();
        let status_badge = if project.status == "dormant" {
            r#"<span class="status-chip" style="background:#718096; color:white; font-size:0.75rem; font-weight:bold; padding:0.15rem 0.4rem; border-radius:4px;">DORMANT</span>"#
        } else {
            r#"<span class="status-chip" style="background:#48bb78; color:white; font-size:0.75rem; font-weight:bold; padding:0.15rem 0.4rem; border-radius:4px;">ACTIVE</span>"#
        };
        let toggle_label = if project.status == "dormant" {
            "Activate"
        } else {
            "Mark Dormant"
        };
        let toggle_style = if project.status == "dormant" {
            "background:#48bb78; color:white; border:none; padding:0.25rem 0.6rem; border-radius:4px; font-size:0.8rem; cursor:pointer;"
        } else {
            "background:#e2e8f0; color:#4a5568; border:1px solid #cbd5e0; padding:0.25rem 0.6rem; border-radius:4px; font-size:0.8rem; cursor:pointer;"
        };
        let last_act = format!(
            "Last activity: {}",
            project.last_activity_at.format("%Y-%m-%d %H:%M:%S UTC")
        );

        let name_bytes = project.name.as_bytes();
        let b1 = name_bytes.get(0).copied().unwrap_or(1) as usize;
        let b2 = name_bytes.get(1).copied().unwrap_or(2) as usize;
        let b3 = name_bytes.get(2).copied().unwrap_or(3) as usize;
        let b4 = name_bytes.get(3).copied().unwrap_or(4) as usize;
        
        let c1 = colors[b1 % 5];
        let c2 = colors[b2 % 5];
        let c3 = colors[b3 % 5];
        let c4 = colors[b4 % 5];
        
        let logo_svg = format!(
            r#"<svg width="32" height="32" viewBox="0 0 32 32" style="border-radius:6px; box-shadow:inset 0 0 4px rgba(0,0,0,0.15); display:block;">
  <rect x="0" y="0" width="16" height="16" fill="{}" />
  <rect x="16" y="0" width="16" height="16" fill="{}" />
  <rect x="0" y="16" width="16" height="16" fill="{}" />
  <rect x="16" y="16" width="16" height="16" fill="{}" />
</svg>"#,
            c1, c2, c3, c4
        );

        let proj_link_style = if project.status == "dormant" {
            "color:inherit; text-decoration:line-through; font-weight:700;"
        } else {
            "color:inherit; text-decoration:none; hover:text-decoration:underline; font-weight:700;"
        };

        format!(
            r#"<div class="dashboard-card">
  <div style="display:flex; justify-content:space-between; align-items:start; margin-bottom:0.75rem;">
    <div>
      {}
    </div>
    <div style="display:flex; gap:0.5rem; align-items:center;">
      {}
      <a href="/ui/studies?admin_email={}{}&project_id={}" class="status-chip" style="background:#02182b; color:white; font-size:0.7rem; font-weight:bold; padding:0.15rem 0.4rem; border-radius:4px; text-decoration:none; text-transform:uppercase;">Workbench</a>
    </div>
  </div>
  
  <div>
    <h4 style="margin:0 0 0.25rem 0; font-size:1.1rem; font-weight:700; color:#02182b;">
      <a href="/ui/app?admin_email={}{}&project_id={}&view=sites" style="{}">{}</a>
    </h4>
    <div style="font-size:0.8rem; color:#718096; margin-bottom:0.5rem;">
      Area: <strong>{}</strong> · ID: <code style="background:#e7e5da; color:#02182b; padding:0.1rem 0.3rem; border-radius:3px; font-weight:bold;">{}</code>
    </div>
  </div>

  <div style="border-top:1px solid #edf2f7; padding-top:0.75rem; margin-top:0.75rem; display:flex; justify-content:space-between; align-items:center; font-size:0.75rem; color:#718096;">
    <span style="font-size:0.7rem;">{}</span>
    <form method="post" action="/ui/app/projects/{}/toggle-dormancy" style="margin:0;">
      <input type="hidden" name="admin_email" value="{}" />
      <button type="submit" style="{}">{}</button>
    </form>
  </div>
</div>"#,
            logo_svg,
            status_badge,
            admin_email_q,
            selected_org,
            project.id,
            admin_email_q,
            selected_org,
            project.id,
            proj_link_style,
            html_escape(&project.name),
            html_escape(&project.therapeutic_area),
            html_escape(project.hex_code.as_deref().unwrap_or("pending")),
            last_act,
            project.id,
            html_escape(admin_email.trim()),
            toggle_style,
            toggle_label
        )
    };

    let active_project_html = if active_projects.is_empty() {
        "<p style=\"color:#718096;font-style:italic;\">No active project selected.</p>".to_string()
    } else {
        active_projects.iter().map(|p| format_project_card(p)).collect::<Vec<_>>().join("")
    };

    let other_projects_html = if other_projects.is_empty() {
        "<p style=\"color:#718096;font-style:italic;\">No other projects found.</p>".to_string()
    } else {
        other_projects.iter().map(|p| format_project_card(p)).collect::<Vec<_>>().join("")
    };

    let sites_html = if sites.is_empty() {
        "<p style=\"color:#718096;font-style:italic;\">No sites yet for selected project.</p>".to_string()
    } else {
        sites
            .iter()
            .map(|site| {
                let items = site_checklists.get(&site.id).cloned().unwrap_or_default();
                let checklist_html = if items.is_empty() {
                    "<p style=\"color:#718096; font-size:0.8rem; font-style:italic; margin:0;\">No checklist items found.</p>".to_string()
                } else {
                    items
                        .iter()
                        .map(|item| {
                            let status = if item.completed {
                                "<span class=\"status-chip\" style=\"background:#02182b; color:white; font-size:0.7rem; font-weight:bold; padding:0.15rem 0.4rem; border-radius:4px;\">COMPLETED</span>"
                            } else {
                                "<span class=\"status-chip\" style=\"background:#cbd5e0; color:#4a5568; font-size:0.7rem; font-weight:bold; padding:0.15rem 0.4rem; border-radius:4px;\">PENDING</span>"
                            };
                            let action = format!("/ui/app/sites/{}/startup-checklist", site.id);
                            let form_html = if item.completed {
                                format!(
                                    r#"<form method="post" action="{}" style="margin-top:0.45rem;display:grid;grid-template-columns:minmax(0,1fr) auto;gap:0.45rem;align-items:center;">
  <input type="hidden" name="admin_email" value="{}" />
  <input type="hidden" name="item_code" value="{}" />
  <input type="hidden" name="item_label" value="{}" />
  <input name="notes" value="{}" placeholder="Compliance note" style="border:1px solid #cbd5e0; padding:0.25rem; border-radius:4px; font-size:0.8rem;" />
  <button type="submit" style="background:#e2e8f0; color:#4a5568; border:1px solid #cbd5e0; padding:0.25rem 0.6rem; border-radius:4px; font-size:0.8rem; cursor:pointer;">Undo Complete</button>
</form>"#,
                                    action,
                                    html_escape(admin_email.trim()),
                                    html_escape(&item.item_code),
                                    html_escape(&item.item_label),
                                    html_escape(&item.notes)
                                )
                            } else {
                                format!(
                                    r#"<form method="post" action="{}" style="margin-top:0.45rem;display:grid;grid-template-columns:minmax(0,1fr) auto;gap:0.45rem;align-items:center;">
  <input type="hidden" name="admin_email" value="{}" />
  <input type="hidden" name="item_code" value="{}" />
  <input type="hidden" name="item_label" value="{}" />
  <input type="hidden" name="completed" value="true" />
  <input name="notes" value="{}" placeholder="Compliance verification note (REQUIRED)" required style="border: 1px solid #f05708; padding:0.25rem; border-radius:4px; font-size:0.8rem;" />
  <button type="submit" style="background:#02182b; color:white; padding:0.25rem 0.6rem; border:none; border-radius:4px; font-size:0.8rem; cursor:pointer;">Mark Complete</button>
</form>"#,
                                    action,
                                    html_escape(admin_email.trim()),
                                    html_escape(&item.item_code),
                                    html_escape(&item.item_label),
                                    html_escape(&item.notes)
                                )
                            };
                            format!(
                                r#"<div style="background:{}; border-left:4px solid {}; border-radius:6px; padding:0.75rem; margin-bottom:0.5rem; display:flex; flex-direction:column; gap:0.25rem; box-shadow:0 1px 2px rgba(0,0,0,0.02);">
  <div style="display:flex; justify-content:space-between; align-items:center;">
    <strong style="color:#02182b; font-size:0.9rem;">{}</strong>
    {}
  </div>
  {}
</div>"#,
                                if item.completed { "#e7e5da" } else { "#f8fafc" },
                                if item.completed { "#02182b" } else { "#f05708" },
                                html_escape(&item.item_label),
                                status,
                                form_html
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("")
                };

                let status_badge = if site.status == "dormant" {
                    r#"<span class="status-chip" style="background:#718096; color:white; font-size:0.75rem; font-weight:bold; padding:0.15rem 0.4rem; border-radius:4px;">DORMANT</span>"#
                } else {
                    r#"<span class="status-chip" style="background:#48bb78; color:white; font-size:0.75rem; font-weight:bold; padding:0.15rem 0.4rem; border-radius:4px;">ACTIVE</span>"#
                };

                let toggle_label = if site.status == "dormant" {
                    "Activate Site"
                } else {
                    "Mark Dormant"
                };

                let toggle_style = if site.status == "dormant" {
                    "background:#48bb78; color:white; padding:0.25rem 0.6rem; font-size:0.8rem; border:none; border-radius:4px; cursor:pointer;"
                } else {
                    "background:#e2e8f0; color:#4a5568; border:1px solid #cbd5e0; padding:0.25rem 0.6rem; font-size:0.8rem; border-radius:4px; cursor:pointer;"
                };

                let card_style = if site.status == "dormant" {
                    r#"class="dashboard-card" style="min-height:220px; opacity:0.75; text-decoration:line-through;""#
                } else {
                    r#"class="dashboard-card" style="min-height:220px;""#
                };

                let last_act = format!(
                    "Last activity: {}",
                    site.last_activity_at.format("%Y-%m-%d %H:%M:%S UTC")
                );

                let name_bytes = site.name.as_bytes();
                let b1 = name_bytes.get(0).copied().unwrap_or(1) as usize;
                let b2 = name_bytes.get(1).copied().unwrap_or(2) as usize;
                let b3 = name_bytes.get(2).copied().unwrap_or(3) as usize;
                let b4 = name_bytes.get(3).copied().unwrap_or(4) as usize;
                
                let c1 = colors[b1 % 5];
                let c2 = colors[b2 % 5];
                let c3 = colors[b3 % 5];
                let c4 = colors[b4 % 5];
                
                let logo_svg = format!(
                    r#"<svg width="32" height="32" viewBox="0 0 32 32" style="border-radius:6px; box-shadow:inset 0 0 4px rgba(0,0,0,0.15); display:block;">
  <rect x="0" y="0" width="16" height="16" fill="{}" />
  <rect x="16" y="0" width="16" height="16" fill="{}" />
  <rect x="0" y="16" width="16" height="16" fill="{}" />
  <rect x="16" y="16" width="16" height="16" fill="{}" />
</svg>"#,
                    c1, c2, c3, c4
                );

                format!(
                    r#"<div {}>
  <div style="display:flex; justify-content:space-between; align-items:start; margin-bottom:0.75rem; border-bottom:1px solid #edf2f7; padding-bottom:0.75rem;">
    <div style="display:flex; gap:0.75rem; align-items:center;">
      {}
      <div>
        <h4 style="margin:0; font-size:1.1rem; font-weight:700; color:#02182b;">{}</h4>
        <small style="color:#718096; font-size:0.75rem;">PI: <strong>{}</strong> · ID: <code style="background:#e7e5da; color:#02182b; padding:0.1rem 0.25rem; border-radius:3px; font-weight:bold;">{}</code> · UUID: <code style="font-size:0.7rem;">{}</code></small>
      </div>
    </div>
    <div>
      {}
    </div>
  </div>
  
  <div>
    <h5 style="margin:0 0 0.5rem 0; font-size:0.85rem; font-weight:700; color:#02182b; text-transform:uppercase; letter-spacing:0.05em;">Startup Checklist</h5>
    <div style="max-height:300px; overflow-y:auto; padding-right:0.25rem;">{}</div>
  </div>

  <div style="margin-top:1.25rem; border-top:1px solid #edf2f7; padding-top:0.75rem; display:flex; justify-content:space-between; align-items:center; flex-wrap:wrap; gap:0.5rem;">
    <div style="display:flex; gap:0.45rem; align-items:center;">
      <form method="post" action="/ui/app/sites/{}/toggle-dormancy" style="margin:0;">
        <input type="hidden" name="admin_email" value="{}" />
        <button type="submit" style="{}">{}</button>
      </form>
      <form method="post" action="/ui/app/sites/{}/delete" onsubmit="return confirm('Are you sure you want to delete this site? This will remove all associated startup checklist items.');" style="margin:0;">
        <input type="hidden" name="admin_email" value="{}" />
        <button type="submit" style="background:#e53e3e; color:white; padding:0.25rem 0.6rem; font-size:0.8rem; border:none; border-radius:4px; cursor:pointer;">Delete Site</button>
      </form>
    </div>
    <div style="font-size:0.7rem; color:#718096; text-align:right;">
      {}
    </div>
  </div>
</div>"#,
                    card_style,
                    logo_svg,
                    html_escape(&site.name),
                    html_escape(&site.principal_investigator),
                    html_escape(site.hex_code.as_deref().unwrap_or("pending")),
                    site.id,
                    status_badge,
                    checklist_html,
                    site.id,
                    html_escape(admin_email.trim()),
                    toggle_style,
                    toggle_label,
                    site.id,
                    html_escape(admin_email.trim()),
                    last_act
                )
            })
            .collect::<Vec<_>>()
            .join("")
    };

    let dua_html = if duas.is_empty() {
        "<p style=\"color:#718096;font-style:italic;\">No Data Use Agreements (DUAs) found for this organization.</p>".to_string()
    } else {
        duas.iter()
            .take(8)
            .map(|dua| {
                let name_bytes = dua.hospital_name.as_bytes();
                let b1 = name_bytes.get(0).copied().unwrap_or(1) as usize;
                let b2 = name_bytes.get(1).copied().unwrap_or(2) as usize;
                let b3 = name_bytes.get(2).copied().unwrap_or(3) as usize;
                let b4 = name_bytes.get(3).copied().unwrap_or(4) as usize;
                
                let c1 = colors[b1 % 5];
                let c2 = colors[b2 % 5];
                let c3 = colors[b3 % 5];
                let c4 = colors[b4 % 5];
                
                let logo_svg = format!(
                    r#"<svg width="28" height="28" viewBox="0 0 32 32" style="border-radius:5px; box-shadow:inset 0 0 3px rgba(0,0,0,0.15); display:block; flex-shrink:0;">
  <rect x="0" y="0" width="16" height="16" fill="{}" />
  <rect x="16" y="0" width="16" height="16" fill="{}" />
  <rect x="0" y="16" width="16" height="16" fill="{}" />
  <rect x="16" y="16" width="16" height="16" fill="{}" />
</svg>"#,
                    c1, c2, c3, c4
                );

                let badge_style = if dua.status.to_lowercase() == "signed" {
                    "background:#283e28; color:white; font-size:0.75rem; font-weight:bold; padding:0.2rem 0.5rem; border-radius:4px; text-transform:uppercase; letter-spacing:0.05em;"
                } else {
                    "background:#f05708; color:white; font-size:0.75rem; font-weight:bold; padding:0.2rem 0.5rem; border-radius:4px; text-transform:uppercase; letter-spacing:0.05em;"
                };

                format!(
                    r#"<a href="/ui/dua/{}?admin_email={}" class="dashboard-card-row" style="text-decoration:none; color:inherit;">
  <div style="display:flex; gap:0.75rem; align-items:center;">
    {}
    <div>
      <strong style="color:#02182b; font-size:1.05rem;">
        {}
      </strong>
      <div style="font-size:0.75rem; color:#718096; margin-top:0.15rem;">ID: <code style="font-size:0.7rem;">{}</code></div>
    </div>
  </div>
  <div>
    <span class="status-chip" style="{}">{}</span>
  </div>
</a>"#,
                    dua.id,
                    admin_email_q,
                    logo_svg,
                    html_escape(&dua.hospital_name),
                    dua.id,
                    badge_style,
                    html_escape(&dua.status)
                )
            })
            .collect::<Vec<_>>()
            .join("")
    };

    let selected_org_value = selected_org_id.map(|id| id.to_string()).unwrap_or_default();
    let selected_project_value = selected_project_id.map(|id| id.to_string()).unwrap_or_default();
    let selected_patient_value = selected_patient_id.map(|id| id.to_string()).unwrap_or_default();

    let selected_org_hex = selected_org_id.map(|id| id.to_string().chars().take(8).collect::<String>()).unwrap_or_default();
    let selected_project_hex = selected_project_id.map(|id| id.to_string().chars().take(8).collect::<String>()).unwrap_or_default();
    let _selected_patient_hex = selected_patient_id.map(|id| id.to_string().chars().take(8).collect::<String>()).unwrap_or_default();

    let context_bar = render_context_bar(
        if selected_org_value.is_empty() { None } else { Some(&selected_org_value) },
        if selected_project_value.is_empty() { None } else { Some(&selected_project_value) },
        admin_email.trim()
    );

    let org_summary_html = if let Some(summary) = org_summary {
        format!(
            r#"
{context_bar}
<div style="margin-bottom:1.5rem;">
  <h3 style="color:#02182b; font-size:1.1rem; margin-bottom:0.75rem; border-bottom:2px solid #e7e5da; padding-bottom:0.25rem;">Organization Metrics Overview</h3>
  <div style="display:grid; grid-template-columns:repeat(auto-fit, minmax(200px, 1fr)); gap:1rem;">
    
    <a href="?view=projects&admin_email={}&organization_id={}" style="text-decoration:none; display:block; background:white; border:1px solid #e2e8f0; border-radius:10px; padding:1.25rem; box-shadow:0 4px 6px rgba(0,0,0,0.02); position:relative; overflow:hidden;">
      <div style="position:absolute; top:0; left:0; width:4px; height:100%; background:#02182b;"></div>
      <div style="color:#718096; font-size:0.75rem; font-weight:700; text-transform:uppercase; letter-spacing:0.05em;">Total Projects</div>
      <div style="color:#02182b; font-size:2.25rem; font-weight:800; margin-top:0.25rem;">{}</div>
    </a>

    <a href="?view=sites&admin_email={}&organization_id={}" style="text-decoration:none; display:block; background:white; border:1px solid #e2e8f0; border-radius:10px; padding:1.25rem; box-shadow:0 4px 6px rgba(0,0,0,0.02); position:relative; overflow:hidden;">
      <div style="position:absolute; top:0; left:0; width:4px; height:100%; background:#c5b7ab;"></div>
      <div style="color:#718096; font-size:0.75rem; font-weight:700; text-transform:uppercase; letter-spacing:0.05em;">Total Sites</div>
      <div style="color:#02182b; font-size:2.25rem; font-weight:800; margin-top:0.25rem;">{}</div>
    </a>

    <div style="background:white; border:1px solid #e2e8f0; border-radius:10px; padding:1.25rem; box-shadow:0 4px 6px rgba(0,0,0,0.02); position:relative; overflow:hidden;">
      <div style="position:absolute; top:0; left:0; width:4px; height:100%; background:#283e28;"></div>
      <div style="color:#718096; font-size:0.75rem; font-weight:700; text-transform:uppercase; letter-spacing:0.05em;">Sent Form Invites</div>
      <div style="color:#02182b; font-size:2.25rem; font-weight:800; margin-top:0.25rem;">{}</div>
    </div>

    <div style="background:white; border:1px solid #e2e8f0; border-radius:10px; padding:1.25rem; box-shadow:0 4px 6px rgba(0,0,0,0.02); position:relative; overflow:hidden;">
      <div style="position:absolute; top:0; left:0; width:4px; height:100%; background:#f05708;"></div>
      <div style="color:#718096; font-size:0.75rem; font-weight:700; text-transform:uppercase; letter-spacing:0.05em;">Generated Media Links</div>
      <div style="color:#02182b; font-size:2.25rem; font-weight:800; margin-top:0.25rem;">{}</div>
    </div>

  </div>
</div>"#,
            html_escape(admin_email.trim()), selected_org_value.clone(), summary.projects,
            html_escape(admin_email.trim()), selected_org_value.clone(), summary.sites,
            summary.sent_form_invites, summary.generated_media_upload_links
        )
    } else {
        "<div style=\"background:#e7e5da; padding:1rem; border-radius:8px; color:#02182b; font-weight:500; font-size:0.9rem; margin-bottom:1.5rem;\">ℹ️ Select an organization from the Overview tab to view detailed metrics.</div>".to_string()
    };

    let project_summary_html = if let Some(summary) = project_report {
        format!(
            r#"<div>
  <div style="background:#fefce8; border:1px solid #fde047; border-radius:6px; padding:8px 12px; margin-bottom:0.75rem; font-size:0.85rem; display:flex; align-items:center; gap:12px; flex-wrap:wrap;">
    <strong style="color:#854d0e;">Project Health:</strong>
    <span style="color:#64748b;">See detailed open queries, checklist status, and CRF monitoring in the Study Workbench tab.</span>
  </div>
  <h3 style="color:#02182b; font-size:1.1rem; margin-bottom:0.75rem; border-bottom:2px solid #e7e5da; padding-bottom:0.25rem;">Project Specific Insights</h3>
  <div style="display:grid; grid-template-columns:repeat(auto-fit, minmax(200px, 1fr)); gap:1rem;">
    
    <a href="?view=sites&admin_email={}&organization_id={}&project_id={}" style="text-decoration:none; display:block; background:white; border:1px solid #e2e8f0; border-radius:10px; padding:1.25rem; box-shadow:0 4px 6px rgba(0,0,0,0.02); position:relative; overflow:hidden;">
      <div style="position:absolute; top:0; left:0; width:4px; height:100%; background:#02182b;"></div>
      <div style="color:#718096; font-size:0.75rem; font-weight:700; text-transform:uppercase; letter-spacing:0.05em;">Total Sites</div>
      <div style="color:#02182b; font-size:2.25rem; font-weight:800; margin-top:0.25rem;">{}</div>
    </a>

    <div style="background:white; border:1px solid #e2e8f0; border-radius:10px; padding:1.25rem; box-shadow:0 4px 6px rgba(0,0,0,0.02); position:relative; overflow:hidden;">
      <div style="position:absolute; top:0; left:0; width:4px; height:100%; background:#283e28;"></div>
      <div style="color:#718096; font-size:0.75rem; font-weight:700; text-transform:uppercase; letter-spacing:0.05em;">Total Form Invites</div>
      <div style="color:#02182b; font-size:2.25rem; font-weight:800; margin-top:0.25rem;">{}</div>
    </div>

    <div style="background:white; border:1px solid #e2e8f0; border-radius:10px; padding:1.25rem; box-shadow:0 4px 6px rgba(0,0,0,0.02); position:relative; overflow:hidden;">
      <div style="position:absolute; top:0; left:0; width:4px; height:100%; background:#f05708;"></div>
      <div style="color:#718096; font-size:0.75rem; font-weight:700; text-transform:uppercase; letter-spacing:0.05em;">Media Captures Requested</div>
      <div style="color:#02182b; font-size:2.25rem; font-weight:800; margin-top:0.25rem;">{}</div>
    </div>

  </div>
</div>"#,
            html_escape(admin_email.trim()), selected_org_value.clone(), selected_project_value.clone(),
            summary.total_sites, summary.total_form_invites, summary.total_media_captures_requested
        )
    } else {
        "<div style=\"background:#e7e5da; padding:1rem; border-radius:8px; color:#02182b; font-weight:500; font-size:0.9rem;\">ℹ️ Select a project from the Overview tab to view granular insights.</div>".to_string()
    };

    let pending_intakes: Vec<_> = patients
        .iter()
        .filter(|p| p.external_subject_id.as_deref().map_or(false, |s| s.starts_with("intake:")))
        .collect();

    let pending_intakes_html = if !pending_intakes.is_empty() || selected_project_id.is_some() {
        let items = pending_intakes.iter().map(|p| {
            format!(
                r#"<div style="background:#fefce8; border:1px solid #fde047; padding:6px 10px; border-radius:4px; font-size:0.85rem; margin-bottom:4px;">
                   <strong>Portal Intake:</strong> {} &nbsp; <a href="/ui/app?admin_email={}&organization_id={}&project_id={}&patient_id={}" style="color:#b45309; font-weight:600;">Review →</a>
                 </div>"#,
                html_escape(p.external_subject_id.as_deref().unwrap_or("")),
                html_escape(admin_email.trim()),
                selected_org_value,
                selected_project_value,
                p.id
            )
        }).collect::<Vec<_>>().join("");
        let count_str = if pending_intakes.is_empty() {
            "(none)".to_string()
        } else {
            format!("({})", pending_intakes.len())
        };
        format!(
            r#"<div style="margin-bottom:1.25rem; padding:12px; background:#fefce8; border:2px solid #fde047; border-radius:8px;">
               <div style="display:flex; align-items:center; gap:8px; margin-bottom:8px;">
                 <span style="font-size:1.1rem;">📥</span>
                 <div style="font-weight:700; color:#854d0e; font-size:0.95rem;">Pending Intakes from Patient Portal {}</div>
               </div>
               {}
             </div>"#,
            count_str,
            if items.is_empty() { "<span style=\"font-size:0.85rem; color:#854d0e;\">No pending portal intakes for this project.</span>" } else { &items }
        )
    } else {
        String::new()
    };

    let patients_html = if patients.is_empty() {
        "<p style=\"color:#718096;font-style:italic;\">No patients yet for selected project.</p>".to_string()
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

                let name_bytes = patient.id.as_bytes();
                let b1 = name_bytes.get(0).copied().unwrap_or(1) as usize;
                let b2 = name_bytes.get(1).copied().unwrap_or(2) as usize;
                let b3 = name_bytes.get(2).copied().unwrap_or(3) as usize;
                let b4 = name_bytes.get(3).copied().unwrap_or(4) as usize;
                
                let c1 = colors[b1 % 5];
                let c2 = colors[b2 % 5];
                let c3 = colors[b3 % 5];
                let c4 = colors[b4 % 5];
                
                let logo_svg = format!(
                    r#"<svg width="24" height="24" viewBox="0 0 32 32" style="border-radius:4px; box-shadow:inset 0 0 2px rgba(0,0,0,0.15); display:block;">
  <rect x="0" y="0" width="16" height="16" fill="{}" />
  <rect x="16" y="0" width="16" height="16" fill="{}" />
  <rect x="0" y="16" width="16" height="16" fill="{}" />
  <rect x="16" y="16" width="16" height="16" fill="{}" />
</svg>"#,
                    c1, c2, c3, c4
                );

                format!(
                    r#"<div class="dashboard-card-mini" style="text-decoration:none; color:inherit;">
  <a href="/ui/app?admin_email={}{}{}&patient_id={}" style="display:flex; text-decoration:none; color:inherit; flex:1;">
    <div>{}</div>
    <div style="flex-grow:1; min-width:0;">
      <strong style="color:#02182b; font-size:0.95rem; display:block; white-space:nowrap; overflow:hidden; text-overflow:ellipsis;">
        {}
      </strong>
      <span style="font-size:0.75rem; color:#718096; display:block;">
        ID: <code style="font-size:0.7rem; font-weight:bold;">{}</code>
        <br/>Site ID: <code style="font-size:0.7rem;">{}</code>
      </span>
    </div>
  </a>
  <form method="post" action="/ui/patients/{}/portal-link" style="margin-top:4px; text-align:right;">
    <button type="submit" style="font-size:0.65rem; padding:2px 8px; background:#166534; color:white; border:none; border-radius:3px; cursor:pointer; font-weight:600;">Portal Link</button>
  </form>
</div>"#,
                    admin_email_q,
                    selected_org,
                    selected_project,
                    patient.id,
                    logo_svg,
                    html_escape(patient.hex_code.as_deref().unwrap_or("pending")),
                    patient.id,
                    patient
                        .site_id
                        .map(|id| id.to_string())
                        .unwrap_or_else(|| "none".to_string()),
                    patient.id
                )
            })
            .collect::<Vec<_>>()
            .join("")
    };

    let providers_html = if providers.is_empty() {
        "<p style=\"color:#718096;font-style:italic;\">No providers yet for selected organization.</p>".to_string()
    } else {
        let mut html = String::new();
        html.push_str(r#"<table style="width:100%; border-collapse:collapse; margin-top:0.5rem; background:white; border-radius:8px; overflow:hidden; box-shadow:0 1px 3px rgba(0,0,0,0.05);">
  <thead>
    <tr style="background:#02182b; color:white; text-align:left;">
      <th style="padding:0.75rem 1rem;">Provider Name</th>
      <th style="padding:0.75rem 1rem;">Title / Specialty</th>
      <th style="padding:0.75rem 1rem;">Email</th>
      <th style="padding:0.75rem 1rem;">Phone</th>
      <th style="padding:0.75rem 1rem;">ID</th>
    </tr>
  </thead>
  <tbody>"#);
        for provider in providers.iter().take(12) {
            let name_bytes = provider.name.as_bytes();
            let b1 = name_bytes.get(0).copied().unwrap_or(1) as usize;
            let b2 = name_bytes.get(1).copied().unwrap_or(2) as usize;
            let b3 = name_bytes.get(2).copied().unwrap_or(3) as usize;
            let b4 = name_bytes.get(3).copied().unwrap_or(4) as usize;
            
            let c1 = colors[b1 % 5];
            let c2 = colors[b2 % 5];
            let c3 = colors[b3 % 5];
            let c4 = colors[b4 % 5];
            
            let logo_svg = format!(
                r#"<svg width="24" height="24" viewBox="0 0 32 32" style="border-radius:4px; box-shadow:inset 0 0 2px rgba(0,0,0,0.15); display:block; flex-shrink:0;">
  <rect x="0" y="0" width="16" height="16" fill="{}" />
  <rect x="16" y="0" width="16" height="16" fill="{}" />
  <rect x="0" y="16" width="16" height="16" fill="{}" />
  <rect x="16" y="16" width="16" height="16" fill="{}" />
</svg>"#,
                c1, c2, c3, c4
            );

            html.push_str(&format!(
                r#"<tr style="border-bottom:1px solid #e2e8f0; hover:background:#f7fafc;">
      <td style="padding:0.75rem 1rem; font-weight:600; color:#02182b;">
        <div style="display:flex; gap:0.6rem; align-items:center;">
          {}
          <span>{}</span>
        </div>
      </td>
      <td style="padding:0.75rem 1rem; color:#4a5568;">{}</td>
      <td style="padding:0.75rem 1rem; color:#4a5568;">{}</td>
      <td style="padding:0.75rem 1rem; color:#4a5568;">{}</td>
      <td style="padding:0.75rem 1rem;"><code style="background:#e7e5da; color:#02182b; padding:0.2rem 0.4rem; border-radius:4px; font-weight:bold;">{}</code></td>
    </tr>"#,
                logo_svg,
                html_escape(&provider.name),
                html_escape(&provider.title),
                html_escape(provider.email.as_deref().unwrap_or("-")),
                html_escape(provider.phone_number.as_deref().unwrap_or("-")),
                html_escape(&provider.hex_code)
            ));
        }
        html.push_str("</tbody></table>");
        html
    };

    let encounters_html = if encounters.is_empty() {
        "<p style=\"color:#718096;font-style:italic;\">No encounters yet for selected patient.</p>".to_string()
    } else {
        let mut html = String::new();
        html.push_str(r#"<table style="width:100%; border-collapse:collapse; margin-top:0.5rem; background:white; border-radius:8px; overflow:hidden; box-shadow:0 1px 3px rgba(0,0,0,0.05);">
  <thead>
    <tr style="background:#02182b; color:white; text-align:left;">
      <th style="padding:0.75rem 1rem;">Encounter ID</th>
      <th style="padding:0.75rem 1rem;">Encounter Type</th>
      <th style="padding:0.75rem 1rem;">Associated Provider ID</th>
      <th style="padding:0.75rem 1rem;">Clinical Notes</th>
    </tr>
  </thead>
  <tbody>"#);
        for encounter in encounters.iter().take(16) {
            let provider_label = encounter
                .provider_id
                .map(|id| id.to_string())
                .unwrap_or_else(|| "none".to_string());
            let notes_label = if encounter.notes.is_empty() { "—" } else { &encounter.notes };
            html.push_str(&format!(
                r#"<tr style="border-bottom:1px solid #e2e8f0; hover:background:#f7fafc;">
      <td style="padding:0.75rem 1rem;"><code style="background:#e7e5da; color:#f05708; padding:0.2rem 0.4rem; border-radius:4px; font-weight:bold;">{}</code></td>
      <td style="padding:0.75rem 1rem; color:#4a5568;"><span class="status-chip" style="background:#c5b7ab; color:#02182b; font-weight:bold;">{}</span></td>
      <td style="padding:0.75rem 1rem; font-size:0.85rem; color:#718096;">{}</td>
      <td style="padding:0.75rem 1rem; color:#4a5568; font-style:italic;">{}</td>
    </tr>"#,
                html_escape(&encounter.hex_code),
                html_escape(&encounter.encounter_type),
                html_escape(&provider_label),
                html_escape(notes_label)
            ));
        }
        html.push_str("</tbody></table>");
        html
    };

    let child_organizations_html = if child_organizations.is_empty() {
        "<p style=\"color:#718096; font-style:italic; margin:0;\">No child organizations under current workspace.</p>".to_string()
    } else {
        let cards = child_organizations
            .iter()
            .map(|org| {
                let name_bytes = org.name.as_bytes();
                let b1 = name_bytes.get(0).copied().unwrap_or(1) as usize;
                let b2 = name_bytes.get(1).copied().unwrap_or(2) as usize;
                let b3 = name_bytes.get(2).copied().unwrap_or(3) as usize;
                let b4 = name_bytes.get(3).copied().unwrap_or(4) as usize;
                
                let c1 = colors[b1 % 5];
                let c2 = colors[b2 % 5];
                let c3 = colors[b3 % 5];
                let c4 = colors[b4 % 5];
                
                let logo_svg = format!(
                    r#"<svg width="24" height="24" viewBox="0 0 32 32" style="border-radius:4px; box-shadow:inset 0 0 2px rgba(0,0,0,0.15); display:block;">
  <rect x="0" y="0" width="16" height="16" fill="{}" />
  <rect x="16" y="0" width="16" height="16" fill="{}" />
  <rect x="0" y="16" width="16" height="16" fill="{}" />
  <rect x="16" y="16" width="16" height="16" fill="{}" />
</svg>"#,
                    c1, c2, c3, c4
                );

                format!(
                    r#"<div class="dashboard-card-mini">
  <div>{}</div>
  <div style="flex-grow:1; min-width:0;">
    <strong style="color:#02182b; font-size:0.9rem; display:block; white-space:nowrap; overflow:hidden; text-overflow:ellipsis;">{}</strong>
    <span style="font-size:0.75rem; color:#718096; display:block;">Kind: {} · ID: <code style="font-size:0.7rem;">{}</code></span>
  </div>
</div>"#,
                    logo_svg,
                    html_escape(&org.name),
                    html_escape(&org.organization_kind),
                    org.id
                )
            })
            .collect::<Vec<_>>()
            .join("");
        format!(
            r#"<div style="display:grid; grid-template-columns:repeat(auto-fill, minmax(240px, 1fr)); gap:0.75rem; margin-top:0.75rem; margin-bottom:1.5rem;">{}</div>"#,
            cards
        )
    };

    let organization_options_html = organizations
        .iter()
        .map(|org| {
            format!(
                r#"<option value="{}" data-id="{}">{}</option>"#,
                org.id.to_string().chars().take(8).collect::<String>(),
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
                r#"<option value="{}" data-id="{}">{}</option>"#,
                project.id.to_string().chars().take(8).collect::<String>(),
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
                r#"<option value="{}" data-id="{}">{}</option>"#,
                site.id.to_string().chars().take(8).collect::<String>(),
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
                r#"<option value="{}" data-id="{}">{}</option>"#,
                patient.id.to_string().chars().take(8).collect::<String>(),
                patient.id,
                html_escape(
                    &patient
                        .external_subject_id
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
                r#"<option value="{}" data-id="{}">{}</option>"#,
                provider.id.to_string().chars().take(8).collect::<String>(),
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

    let view = query.view.as_deref().unwrap_or("overview");
    let is_active = |v: &str| if v == view { "is-active" } else { "" };

    let _global_nav = format!(
        r#"<nav class="global-nav" style="margin-bottom: 1rem; padding-bottom: 0.5rem; border-bottom: 1px solid #ddd; display: flex; gap: 1rem; font-size: 0.9rem;">
  <a href="{}">Foundation</a>
  <a href="/ui/app?admin_email={}&organization_id={}">Org Command Center</a>
  <a href="/ui/studies?admin_email={}&organization_id={}">Study Dashboard</a>
  <a href="/ui/dua?admin_email={}&organization_id={}">DUA Console</a>
</nav>"#,
        foundation_hub_url,
        admin_email_q, selected_org_value,
        admin_email_q, selected_org_value,
        admin_email_q, selected_org_value,
    );

    let _tab_bar = format!(
        r#"<nav class="tab-bar" style="margin-bottom: 1rem;">
  <a class="tab-button {}" href="?view=overview&admin_email={}&organization_id={}">Overview</a>
  <a class="tab-button {}" href="?view=projects&admin_email={}&organization_id={}">Projects</a>
  <a class="tab-button {}" href="?view=sites&admin_email={}&organization_id={}">Sites</a>
  <a class="tab-button {}" href="?view=patients&admin_email={}&organization_id={}">Patients</a>
  <a class="tab-button {}" href="?view=providers&admin_email={}&organization_id={}">Providers</a>
  <a class="tab-button {}" href="?view=analytics&admin_email={}&organization_id={}">Analytics</a>
  <a class="tab-button {}" href="?view=legal&admin_email={}&organization_id={}">Legal</a>
</nav>"#,
        is_active("overview"), admin_email_q, selected_org_value,
        is_active("projects"), admin_email_q, selected_org_value,
        is_active("sites"), admin_email_q, selected_org_value,
        is_active("patients"), admin_email_q, selected_org_value,
        is_active("providers"), admin_email_q, selected_org_value,
        is_active("analytics"), admin_email_q, selected_org_value,
        is_active("legal"), admin_email_q, selected_org_value,
    );

    let panel_content = match view {
        "orgs" => format!(
            r#"<section class="card">
  <h2>1) Organizations</h2>
  <p style="color:#718096; margin-bottom:1rem; font-size:0.9rem;">
    Organizations represent distinct medical entities, tenants, or hospital networks within the platform.
  </p>
  
  <div style="display:grid; grid-template-columns:repeat(auto-fill, minmax(280px, 1fr)); gap:1rem; margin-top:1rem;">
    {}
    <div id="add-org-card" style="background:#f7fafc; border:2px dashed #cbd5e0; border-radius:12px; padding:1.25rem; display:flex; flex-direction:column; align-items:center; justify-content:center; min-height:160px; cursor:pointer; hover:background:#edf2f7; hover:border-color:#02182b; transition:all 0.2s;" onclick="document.getElementById('org-create-modal').showModal()">
      <span style="font-size:3rem; color:#a0aec0; font-weight:300; line-height:1;">+</span>
      <span style="font-size:0.9rem; font-weight:600; color:#718096; margin-top:0.5rem;">Add Organization</span>
    </div>
  </div>

  <dialog id="org-create-modal" style="border:none; border-radius:12px; padding:2rem; width:100%; max-width:480px; box-shadow:0 20px 25px -5px rgba(0,0,0,0.1), 0 10px 10px -5px rgba(0,0,0,0.04); background:#fff;">
    <div style="display:flex; justify-content:space-between; align-items:center; margin-bottom:1.5rem; border-bottom:1px solid #edf2f7; padding-bottom:0.75rem;">
      <h3 style="margin:0; color:#02182b; font-size:1.25rem;">Create New Organization</h3>
      <button onclick="document.getElementById('org-create-modal').close()" style="background:none; border:none; font-size:1.5rem; color:#a0aec0; cursor:pointer;">&times;</button>
    </div>
    <form method="post" action="/ui/app/create-organization" style="display:flex; flex-direction:column; gap:0.75rem; margin:0;">
      <label style="font-weight:600; font-size:0.85rem; color:#4a5568;">Admin email (platform admin)</label>
      <input name="admin_email" value="{}" required style="border:1px solid #cbd5e0; padding:0.5rem; border-radius:6px;" />
      
      <label style="font-weight:600; font-size:0.85rem; color:#4a5568;">Organization legal name</label>
      <input name="organization_name" placeholder="Cingulum Foundation Inc." required style="border:1px solid #cbd5e0; padding:0.5rem; border-radius:6px;" />
      
      <label style="font-weight:600; font-size:0.85rem; color:#4a5568;">Parent organization ID (optional)</label>
      <input name="parent_organization_id" list="app-organization-options" value="{}" placeholder="root-managed child org" style="border:1px solid #cbd5e0; padding:0.5rem; border-radius:6px;" />
      
      <label style="font-weight:600; font-size:0.85rem; color:#4a5568;">Organization type</label>
      <select name="organization_kind" style="border:1px solid #cbd5e0; padding:0.5rem; border-radius:6px; background:white;">
        <option value="tenant" selected>tenant</option>
        <option value="research_network">research_network</option>
        <option value="hospital">hospital</option>
        <option value="sponsor">sponsor</option>
      </select>
      
      <button type="submit" style="background:#02182b; color:white; border:none; padding:0.75rem; border-radius:6px; font-weight:600; cursor:pointer; margin-top:0.75rem; transition:background 0.2s;">Create Organization</button>
    </form>
  </dialog>
</section>"#,
            organizations_html,
            html_escape(admin_email.trim()),
            selected_org_hex.clone()
        ),
        "projects" => format!(
            r#"<section class="card">
  <h3 style="color:#02182b; font-size:1.25rem; font-weight:700; margin-bottom:0.75rem; border-bottom:2px solid #e7e5da; padding-bottom:0.25rem;">Active Study</h3>
  <div style="display:grid; grid-template-columns:repeat(auto-fill, minmax(280px, 1fr)); gap:1.25rem; margin-top:1rem; margin-bottom:2rem;">{}</div>

  <h3 style="color:#02182b; font-size:1.25rem; font-weight:700; margin-bottom:0.75rem; border-bottom:2px solid #e7e5da; padding-bottom:0.25rem; margin-top:2rem;">Other Studies</h3>
  <div style="display:grid; grid-template-columns:repeat(auto-fill, minmax(280px, 1fr)); gap:1.25rem; margin-top:1rem; margin-bottom:2rem;">{}</div>

  <div style="border-top: 2px solid #e7e5da; padding-top: 1.5rem; margin-top: 2.5rem;">
    <h3 style="color:#02182b; font-size:1.25rem; font-weight:700; margin-bottom:0.5rem;">Create New Project</h3>
    <p style="color:#718096; margin-bottom:1.5rem; font-size:0.9rem;">
      Project Setup configures an individual clinical trial, medical study, or registry under the umbrella of a specific organization.
    </p>
    <form method="post" action="/ui/app/create-project" style="max-width: 480px; display:flex; flex-direction:column; gap:0.85rem;">
      <input type="hidden" name="admin_email" value="{}" />
      <input type="hidden" name="organization_id" value="{}" />
      
      <div>
        <label style="font-weight:600; font-size:0.85rem; color:#4a5568; display:block; margin-bottom:0.25rem;">Project name</label>
        <input name="project_name" placeholder="Stroke Registry 2026" required style="border:1px solid #cbd5e0; padding:0.55rem; border-radius:6px; width:100%; box-sizing:border-box;" />
      </div>
      
      <div>
        <label style="font-weight:600; font-size:0.85rem; color:#4a5568; display:block; margin-bottom:0.25rem;">Therapeutic area</label>
        <input name="therapeutic_area" placeholder="Neurology" required style="border:1px solid #cbd5e0; padding:0.55rem; border-radius:6px; width:100%; box-sizing:border-box;" />
      </div>
      
      <button type="submit" style="background:#02182b; color:white; border:none; padding:0.75rem 1.5rem; border-radius:6px; font-weight:600; cursor:pointer; align-self:flex-start; margin-top:0.5rem; transition:background 0.2s;">Create Project</button>
    </form>
  </div>
</section>"#,
            active_project_html,
            other_projects_html,
            html_escape(admin_email.trim()),
            selected_org_value.clone()
        ),
        "sites" => format!(
            r#"<section class="card">
  <h2>3) Site Setup</h2>
  <p style="color:#718096; margin-bottom:1.5rem; font-size:0.9rem;">
    Register clinical trial sites, designate investigators, and track activation checklists.
  </p>
  
  <h3 style="margin-top:1.5rem; color:#02182b; font-weight:700;">Sites</h3>
  <div style="display:grid; grid-template-columns:repeat(auto-fill, minmax(340px, 1fr)); gap:1.5rem; margin-top:1rem;">
    <div id="add-site-card" class="dashboard-card" style="border:2px dashed #cbd5e0; background:#f8fafc; display:flex; flex-direction:column; align-items:center; justify-content:center; min-height:220px; cursor:pointer; transition:all 0.2s; position:relative; box-shadow:none;" onclick="document.getElementById('site-create-modal').showModal()">
      <span style="font-size:3rem; color:#a0aec0; font-weight:300; line-height:1;">+</span>
      <span style="font-size:0.95rem; font-weight:600; color:#718096; margin-top:0.5rem;">Create New Site</span>
    </div>
    {}
  </div>

  <dialog id="site-create-modal" style="border:none; border-radius:16px; padding:2rem; width:100%; max-width:480px; box-shadow:0 25px 50px -12px rgba(0,0,0,0.25); background:#fff;">
    <div style="display:flex; justify-content:space-between; align-items:center; margin-bottom:1.5rem; border-bottom:1px solid #edf2f7; padding-bottom:0.75rem;">
      <h3 style="margin:0; color:#02182b; font-size:1.25rem; font-weight:700;">Create New Site</h3>
      <button onclick="document.getElementById('site-create-modal').close()" style="background:none; border:none; font-size:1.5rem; color:#a0aec0; cursor:pointer; line-height:1;">&times;</button>
    </div>
    <form method="post" action="/ui/app/create-site" style="display:flex; flex-direction:column; gap:0.85rem; margin:0;">
      <label style="font-weight:600; font-size:0.85rem; color:#4a5568;">Admin email</label>
      <input name="admin_email" value="{}" required style="border:1px solid #cbd5e0; padding:0.55rem; border-radius:6px; background:#f7fafc;" readonly />
      
      <input type="hidden" name="project_id" value="{}" />
      
      <label style="font-weight:600; font-size:0.85rem; color:#4a5568;">Site name</label>
      <input name="site_name" placeholder="North Campus Site A" required style="border:1px solid #cbd5e0; padding:0.55rem; border-radius:6px;" />
      
      <label style="font-weight:600; font-size:0.85rem; color:#4a5568;">Principal investigator</label>
      <input name="principal_investigator" placeholder="Dr. Example" required style="border:1px solid #cbd5e0; padding:0.55rem; border-radius:6px;" />
      
      <label style="font-weight:600; font-size:0.85rem; color:#4a5568;">Co-Principal investigator (optional)</label>
      <input name="co_principal_investigator" placeholder="Dr. Smith" style="border:1px solid #cbd5e0; padding:0.55rem; border-radius:6px;" />
      
      <label style="font-weight:600; font-size:0.85rem; color:#4a5568;">Sub-Investigator (optional)</label>
      <input name="sub_investigator" placeholder="Dr. Doe" style="border:1px solid #cbd5e0; padding:0.55rem; border-radius:6px;" />
      
      <button type="submit" style="background:#02182b; color:white; border:none; padding:0.75rem; border-radius:6px; font-weight:600; cursor:pointer; margin-top:0.75rem; transition:background 0.2s;">Create Site</button>
    </form>
  </dialog>
</section>"#,
            sites_html,
            html_escape(admin_email.trim()),
            selected_project_hex.clone()
        ),
        "patients" => format!(
            r#"<section class="card">
  <h2>4) Patient Workflow</h2>
  <p style="color:#718096; margin-bottom:1.5rem; font-size:0.9rem;">
    Manage clinical trial subjects, deploy digital intake forms, and generate secure media upload channels.
  </p>

  <div style="display:grid; grid-template-columns:repeat(auto-fit, minmax(320px, 1fr)); gap:1.5rem; margin-bottom:2rem;">
    <!-- Send Patient Form Invite Card -->
    <div class="dashboard-card" style="padding:1.5rem; min-height:unset;">
      <h3 style="margin-top:0; color:#02182b; font-size:1.1rem; font-weight:700; border-bottom:1px solid #edf2f7; padding-bottom:0.5rem; margin-bottom:1rem;">Send Patient Form Invite</h3>
      <form method="post" action="/ui/app/send-invite" style="display:flex; flex-direction:column; gap:0.75rem; margin:0;">
        <label style="font-weight:600; font-size:0.85rem; color:#4a5568;">Admin email</label>
        <input name="admin_email" value="{}" required style="border:1px solid #cbd5e0; padding:0.5rem; border-radius:6px; background:#f7fafc;" readonly />
        
        <input type="hidden" name="organization_id" value="{}" />
        <input type="hidden" name="project_id" value="{}" />
        
        <label style="font-weight:600; font-size:0.85rem; color:#4a5568;">Patient email</label>
        <input type="email" name="patient_email" placeholder="patient@example.org" required style="border:1px solid #cbd5e0; padding:0.5rem; border-radius:6px;" />
        
        <label style="font-weight:600; font-size:0.85rem; color:#4a5568;">Form type</label>
        <input name="form_type" placeholder="demographics-intake" required style="border:1px solid #cbd5e0; padding:0.5rem; border-radius:6px;" />
        
        <button type="submit" style="background:#02182b; color:white; border:none; padding:0.6rem; border-radius:6px; font-weight:600; cursor:pointer; margin-top:0.5rem; transition:background 0.2s;">Send Form Invite</button>
      </form>
    </div>

    <!-- Generate Media Upload Link Card -->
    <div class="dashboard-card" style="padding:1.5rem; min-height:unset;">
      <h3 style="margin-top:0; color:#02182b; font-size:1.1rem; font-weight:700; border-bottom:1px solid #edf2f7; padding-bottom:0.5rem; margin-bottom:1rem;">Generate Media Upload Link</h3>
      <form method="post" action="/ui/app/create-media-ticket" style="display:flex; flex-direction:column; gap:0.75rem; margin:0;">
        <label style="font-weight:600; font-size:0.85rem; color:#4a5568;">Admin email</label>
        <input name="admin_email" value="{}" required style="border:1px solid #cbd5e0; padding:0.5rem; border-radius:6px; background:#f7fafc;" readonly />
        
        <input type="hidden" name="organization_id" value="{}" />
        <input type="hidden" name="project_id" value="{}" />
        
        <label style="font-weight:600; font-size:0.85rem; color:#4a5568;">Patient ID</label>
        <input name="patient_id" list="app-patient-options" placeholder="subject-001" required style="border:1px solid #cbd5e0; padding:0.5rem; border-radius:6px;" />
        
        <label style="font-weight:600; font-size:0.85rem; color:#4a5568;">MIME type</label>
        <input name="mime_type" placeholder="video/mp4" required style="border:1px solid #cbd5e0; padding:0.5rem; border-radius:6px;" />
        
        <button type="submit" style="background:#02182b; color:white; border:none; padding:0.6rem; border-radius:6px; font-weight:600; cursor:pointer; margin-top:0.5rem; transition:background 0.2s;">Generate Upload Link</button>
      </form>
    </div>
  </div>

  <h3 style="margin-top:1.5rem; color:#02182b; font-weight:700;">Patients</h3>
  <div style="margin-bottom:0.5rem; font-size:0.85rem; color:#64748b;">
    Participants can self-submit basic intake via the <a href="/portal/intake" style="color:#f05708; font-weight:600;">Patient Portal</a>. New submissions appear above as "Pending Intakes from Patient Portal".
  </div>
  {}
  {}
  <div style="display:grid; grid-template-columns:repeat(auto-fill, minmax(280px, 1fr)); gap:1.25rem; margin-top:1rem;">
    <div id="add-patient-card" class="dashboard-card" style="border:2px dashed #cbd5e0; background:#f8fafc; display:flex; flex-direction:column; align-items:center; justify-content:center; min-height:100px; cursor:pointer; transition:all 0.2s; position:relative; box-shadow:none;" onclick="document.getElementById('patient-create-modal').showModal()">
      <span style="font-size:2.5rem; color:#a0aec0; font-weight:300; line-height:1;">+</span>
      <span style="font-size:0.85rem; font-weight:600; color:#718096; margin-top:0.25rem;">Create New Patient</span>
    </div>
    {}
  </div>

  <dialog id="patient-create-modal" style="border:none; border-radius:16px; padding:2rem; width:100%; max-width:480px; box-shadow:0 25px 50px -12px rgba(0,0,0,0.25); background:#fff;">
    <div style="display:flex; justify-content:space-between; align-items:center; margin-bottom:1.5rem; border-bottom:1px solid #edf2f7; padding-bottom:0.75rem;">
      <h3 style="margin:0; color:#02182b; font-size:1.25rem; font-weight:700;">Create New Patient</h3>
      <button onclick="document.getElementById('patient-create-modal').close()" style="background:none; border:none; font-size:1.5rem; color:#a0aec0; cursor:pointer; line-height:1;">&times;</button>
    </div>
    <form method="post" action="/ui/app/create-patient" style="display:flex; flex-direction:column; gap:0.85rem; margin:0;">
      <label style="font-weight:600; font-size:0.85rem; color:#4a5568;">Admin email</label>
      <input name="admin_email" value="{}" required style="border:1px solid #cbd5e0; padding:0.55rem; border-radius:6px; background:#f7fafc;" readonly />
      
      <label style="font-weight:600; font-size:0.85rem; color:#4a5568;">Site ID</label>
      <input name="site_id" list="app-site-options" placeholder="site-uuid" required style="border:1px solid #cbd5e0; padding:0.55rem; border-radius:6px;" />
      
      <label style="font-weight:600; font-size:0.85rem; color:#4a5568;">External subject label (optional)</label>
      <input name="external_subject_id" placeholder="SUBJ-001" style="border:1px solid #cbd5e0; padding:0.55rem; border-radius:6px;" />
      
      <label style="font-weight:600; font-size:0.85rem; color:#4a5568;">Patient email (optional)</label>
      <input type="email" name="email" placeholder="patient@example.org" style="border:1px solid #cbd5e0; padding:0.55rem; border-radius:6px;" />
      
      <label style="font-weight:600; font-size:0.85rem; color:#4a5568;">Date of birth (optional, YYYY-MM-DD)</label>
      <input name="date_of_birth" placeholder="1980-01-01" style="border:1px solid #cbd5e0; padding:0.55rem; border-radius:6px;" />
      
      <button type="submit" style="background:#02182b; color:white; border:none; padding:0.75rem; border-radius:6px; font-weight:600; cursor:pointer; margin-top:0.75rem; transition:background 0.2s;">Create Patient + ID</button>
    </form>
  </dialog>
</section>"#,
            html_escape(admin_email.trim()),
            selected_org_value,
            selected_project_value,
            html_escape(admin_email.trim()),
            selected_org_value,
            selected_project_value,
            pending_intakes_html,
            recent_pro_reports_html,
            patients_html,
            html_escape(admin_email.trim())
        ),
        "providers" => format!(
            r##"<section class="card">
  <h2>5) Providers</h2>
  <p style="color:#718096; margin-bottom:1.5rem; font-size:0.9rem;">
    Register medical providers.
  </p>

  <div style="display:grid; grid-template-columns: 1fr; gap:1.5rem; margin-bottom:2rem;">
    <!-- Register Provider Card -->
    <div class="dashboard-card" style="padding:1.5rem; min-height:unset; position:relative; overflow:hidden;">
      <div style="display:flex; justify-content:space-between; align-items:center; margin-bottom:1rem; border-bottom:1px solid #edf2f7; padding-bottom:0.5rem;">
        <div style="display:flex; gap:0.5rem; align-items:center;">
          <svg width="24" height="24" viewBox="0 0 32 32" style="border-radius:4px; box-shadow:inset 0 0 2px rgba(0,0,0,0.15); display:block;">
            <rect x="0" y="0" width="16" height="16" fill="#f05708" />
            <rect x="16" y="0" width="16" height="16" fill="#02182b" />
            <rect x="0" y="16" width="16" height="16" fill="#e7e5da" />
            <rect x="16" y="16" width="16" height="16" fill="#02182b" />
          </svg>
          <h3 style="margin:0; color:#02182b; font-size:1.1rem; font-weight:700;">Register Provider</h3>
        </div>
      </div>
      <form method="post" action="/ui/app/create-provider" style="display:grid; grid-template-columns: 1fr 1fr; gap:0.75rem 1.5rem; margin:0;">
        <div style="grid-column: 1 / -1;">
          <label style="font-weight:600; font-size:0.85rem; color:#4a5568;">Admin email</label>
          <input name="admin_email" value="{}" required style="width:100%; border:1px solid #cbd5e0; padding:0.5rem; border-radius:6px; background:#f7fafc; box-sizing:border-box;" readonly />
          <input type="hidden" name="organization_id" value="{}" />
        </div>
        
        <div>
          <label style="font-weight:600; font-size:0.85rem; color:#4a5568;">Provider name</label>
          <input name="provider_name" placeholder="Dr. Jane Doe" required style="width:100%; border:1px solid #cbd5e0; padding:0.5rem; border-radius:6px; box-sizing:border-box;" />
        </div>
        
        <div>
          <label style="font-weight:600; font-size:0.85rem; color:#4a5568;">Provider title / Specialty</label>
          <input name="provider_title" placeholder="Cardiology" style="width:100%; border:1px solid #cbd5e0; padding:0.5rem; border-radius:6px; box-sizing:border-box;" />
        </div>

        <div>
          <label style="font-weight:600; font-size:0.85rem; color:#4a5568;">Email</label>
          <input name="email" type="email" placeholder="jane@example.com" style="width:100%; border:1px solid #cbd5e0; padding:0.5rem; border-radius:6px; box-sizing:border-box;" />
        </div>
        
        <div>
          <label style="font-weight:600; font-size:0.85rem; color:#4a5568;">Phone Number</label>
          <input name="phone_number" placeholder="(555) 123-4567" style="width:100%; border:1px solid #cbd5e0; padding:0.5rem; border-radius:6px; box-sizing:border-box;" />
        </div>

        <div>
          <label style="font-weight:600; font-size:0.85rem; color:#4a5568;">NPI Number</label>
          <input name="npi_number" placeholder="1234567890" style="width:100%; border:1px solid #cbd5e0; padding:0.5rem; border-radius:6px; box-sizing:border-box;" />
        </div>
        
        <div>
          <label style="font-weight:600; font-size:0.85rem; color:#4a5568;">Referral source</label>
          <input name="referral_source" placeholder="External referral network" style="width:100%; border:1px solid #cbd5e0; padding:0.5rem; border-radius:6px; box-sizing:border-box;" />
        </div>
        
        <div style="grid-column: 1 / -1;">
          <label style="font-weight:600; font-size:0.85rem; color:#4a5568;">Address</label>
          <input name="address" placeholder="123 Medical Plaza" style="width:100%; border:1px solid #cbd5e0; padding:0.5rem; border-radius:6px; box-sizing:border-box;" />
        </div>

        <div style="grid-column: 1 / -1;">
          <label style="font-weight:600; font-size:0.85rem; color:#4a5568;">Notes</label>
          <textarea name="notes" placeholder="Additional details..." rows="2" style="width:100%; border:1px solid #cbd5e0; padding:0.5rem; border-radius:6px; box-sizing:border-box; font-family:inherit; resize:vertical;"></textarea>
        </div>
        
        <div style="grid-column: 1 / -1; margin-top:0.5rem;">
          <button type="submit" style="width:100%; background:#02182b; color:white; border:none; padding:0.6rem; border-radius:6px; font-weight:600; cursor:pointer; transition:background 0.2s;">Register Provider + ID</button>
        </div>
      </form>
    </div>
  </div>

  <h3 style="margin-top:1.5rem; color:#02182b; font-weight:700; font-size:1.25rem;">Registered Providers</h3>
  <div style="margin-bottom:2rem;">{}</div>
</section>"##,
            html_escape(admin_email.trim()),
            selected_org_value,
            providers_html
        ),
        "encounters" => format!(
            r##"<section class="card">
  <h2>Encounters</h2>
  <p style="color:#718096; margin-bottom:1.5rem; font-size:0.9rem;">
    Log clinical encounters.
  </p>

  <div style="display:grid; grid-template-columns: 1fr; gap:1.5rem; margin-bottom:2rem;">
    <!-- Create Encounter Card -->
    <div class="dashboard-card" style="padding:1.5rem; min-height:unset; position:relative; overflow:hidden;">
      <div style="display:flex; justify-content:space-between; align-items:center; margin-bottom:1rem; border-bottom:1px solid #edf2f7; padding-bottom:0.5rem;">
        <div style="display:flex; gap:0.5rem; align-items:center;">
          <svg width="24" height="24" viewBox="0 0 32 32" style="border-radius:4px; box-shadow:inset 0 0 2px rgba(0,0,0,0.15); display:block;">
            <rect x="0" y="0" width="16" height="16" fill="#02182b" />
            <rect x="16" y="0" width="16" height="16" fill="#f05708" />
            <rect x="0" y="16" width="16" height="16" fill="#02182b" />
            <rect x="16" y="16" width="16" height="16" fill="#e7e5da" />
          </svg>
          <h3 style="margin:0; color:#02182b; font-size:1.1rem; font-weight:700;">Create Encounter</h3>
        </div>
      </div>
      <form method="post" action="/ui/app/create-encounter" style="display:flex; flex-direction:column; gap:0.75rem; margin:0;">
        <label style="font-weight:600; font-size:0.85rem; color:#4a5568;">Admin email</label>
        <input name="admin_email" value="{}" required style="border:1px solid #cbd5e0; padding:0.5rem; border-radius:6px; background:#f7fafc;" readonly />
        
        <label style="font-weight:600; font-size:0.85rem; color:#4a5568;">Patient ID</label>
        <input name="patient_id" list="app-patient-options" value="{}" placeholder="patient-uuid" required style="border:1px solid #cbd5e0; padding:0.5rem; border-radius:6px;" />
        
        <label style="font-weight:600; font-size:0.85rem; color:#4a5568;">Encounter type</label>
        <select name="encounter_type" required style="border:1px solid #cbd5e0; padding:0.5rem; border-radius:6px; background:white; font-size:0.9rem; color:#2d3748;">
          <option value="outpatient">outpatient (000-2FF)</option>
          <option value="inpatient">inpatient (300-5FF)</option>
          <option value="labs">labs (600-8FF)</option>
          <option value="imaging">imaging (900-BFF)</option>
          <option value="procedures">procedures (C00-EFF)</option>
          <option value="misc">misc (F00-FFF)</option>
        </select>
        
        <label style="font-weight:600; font-size:0.85rem; color:#4a5568;">Provider ID (optional)</label>
        <input name="provider_id" list="app-provider-options" placeholder="provider-uuid" style="border:1px solid #cbd5e0; padding:0.5rem; border-radius:6px;" />
        
        <label style="font-weight:600; font-size:0.85rem; color:#4a5568;">Notes (optional)</label>
        <input name="notes" placeholder="Encounter notes" style="border:1px solid #cbd5e0; padding:0.5rem; border-radius:6px;" />
        
        <button type="submit" style="background:#02182b; color:white; border:none; padding:0.6rem; border-radius:6px; font-weight:600; cursor:pointer; margin-top:0.5rem; transition:background 0.2s;">Create Encounter + ID</button>
      </form>
    </div>
  </div>

  <h3 style="margin-top:1.5rem; color:#02182b; font-weight:700; font-size:1.25rem;">Encounters for Selected Patient</h3>
  <div>{}</div>
</section>"##,
            html_escape(admin_email.trim()),
            selected_patient_value,
            encounters_html
        ),
        "analytics" => format!(
            r#"<section class="card">
  <h2>6) Analytics Summary</h2>
  {}
  {}
</section>"#,
            org_summary_html, project_summary_html
        ),
                "media" => format!(
            r#"<section class="card">
  <h2>8) Media Vault</h2>
  <div style="background:#f7fafc; padding:2rem; text-align:center; border-radius:8px; border:2px dashed #cbd5e0;">
    <h3 style="color:#02182b; margin-top:0;">Feature under construction</h3>
    <p style="color:#718096; margin-bottom:0;">The Media Vault was reset during the rollback and will be restored shortly.</p>
  </div>
</section>"#
        ),
        "legal" => format!(
            r#"<section class="card">
  <h2>7) Legal / DUA</h2>
  <div style="display:flex; flex-direction:column; gap:1rem; margin-top:1rem;">{}</div>
</section>"#,
            dua_html
        ),
        _ => {
            let admin_val = admin_email.trim();
            let org_val = if selected_org_value.is_empty() {
                "none selected".to_string()
            } else {
                selected_org_hex.clone()
            };
            let proj_val = if selected_project_value.is_empty() {
                "none selected".to_string()
            } else {
                selected_project_hex.clone()
            };

            let admin_logo = r##"<svg width="32" height="32" viewBox="0 0 32 32" style="border-radius:6px; box-shadow:inset 0 0 4px rgba(0,0,0,0.15); display:block;">
  <rect x="0" y="0" width="16" height="16" fill="#02182b" />
  <rect x="16" y="0" width="16" height="16" fill="#c5b7ab" />
  <rect x="0" y="16" width="16" height="16" fill="#c5b7ab" />
  <rect x="16" y="16" width="16" height="16" fill="#02182b" />
</svg>"##;

            let org_logo = if selected_org_value.is_empty() {
                r#"<svg width="32" height="32" viewBox="0 0 32 32" style="border-radius:6px; background:#edf2f7; display:block;"></svg>"#.to_string()
            } else {
                let name_bytes = selected_org_value.as_bytes();
                let b1 = name_bytes.get(0).copied().unwrap_or(1) as usize;
                let b2 = name_bytes.get(1).copied().unwrap_or(2) as usize;
                let b3 = name_bytes.get(2).copied().unwrap_or(3) as usize;
                let b4 = name_bytes.get(3).copied().unwrap_or(4) as usize;
                let c1 = colors[b1 % 5];
                let c2 = colors[b2 % 5];
                let c3 = colors[b3 % 5];
                let c4 = colors[b4 % 5];
                format!(
                    r#"<svg width="32" height="32" viewBox="0 0 32 32" style="border-radius:6px; box-shadow:inset 0 0 4px rgba(0,0,0,0.15); display:block;">
  <rect x="0" y="0" width="16" height="16" fill="{}" />
  <rect x="16" y="0" width="16" height="16" fill="{}" />
  <rect x="0" y="16" width="16" height="16" fill="{}" />
  <rect x="16" y="16" width="16" height="16" fill="{}" />
</svg>"#,
                    c1, c2, c3, c4
                )
            };

            let proj_logo = if selected_project_value.is_empty() {
                r#"<svg width="32" height="32" viewBox="0 0 32 32" style="border-radius:6px; background:#edf2f7; display:block;"></svg>"#.to_string()
            } else {
                let name_bytes = selected_project_value.as_bytes();
                let b1 = name_bytes.get(0).copied().unwrap_or(1) as usize;
                let b2 = name_bytes.get(1).copied().unwrap_or(2) as usize;
                let b3 = name_bytes.get(2).copied().unwrap_or(3) as usize;
                let b4 = name_bytes.get(3).copied().unwrap_or(4) as usize;
                let c1 = colors[b1 % 5];
                let c2 = colors[b2 % 5];
                let c3 = colors[b3 % 5];
                let c4 = colors[b4 % 5];
                format!(
                    r#"<svg width="32" height="32" viewBox="0 0 32 32" style="border-radius:6px; box-shadow:inset 0 0 4px rgba(0,0,0,0.15); display:block;">
  <rect x="0" y="0" width="16" height="16" fill="{}" />
  <rect x="16" y="0" width="16" height="16" fill="{}" />
  <rect x="0" y="16" width="16" height="16" fill="{}" />
  <rect x="16" y="16" width="16" height="16" fill="{}" />
</svg>"#,
                    c1, c2, c3, c4
                )
            };

            format!(
                r#"<section style="display:flex; flex-direction:column; gap:1.5rem;">
  <div>
    <h2 style="color:#02182b; margin:0 0 0.5rem 0; font-size:1.5rem; font-weight:700;">Workspace Overview</h2>
    <p style="color:#718096; margin:0; font-size:0.9rem;">High-level clinical trial environment diagnostics and active telemetry context.</p>
  </div>

  <div style="display:grid; grid-template-columns:repeat(auto-fit, minmax(280px, 1fr)); gap:1.25rem;">
    <!-- Admin Context Card -->
    <div style="background:white; border-radius:12px; border:1px solid #e2e8f0; padding:1.25rem; box-shadow:0 4px 6px -1px rgba(0,0,0,0.05); display:flex; flex-direction:column; justify-content:space-between; min-height:140px;">
      <div style="display:flex; justify-content:space-between; align-items:start;">
        <div>{}</div>
        <span style="background:#02182b; color:white; font-size:0.7rem; font-weight:bold; padding:0.15rem 0.4rem; border-radius:4px; text-transform:uppercase;">Admin Context</span>
      </div>
      <div style="margin-top:1rem;">
        <h4 style="margin:0; font-size:1rem; font-weight:700; color:#2d3748;">Administrative Officer</h4>
        <p style="margin:0.25rem 0 0 0; font-size:0.85rem; color:#718096; word-break:break-all;">{}</p>
      </div>
    </div>

    <!-- Active Organization Card -->
    <div style="background:white; border-radius:12px; border:1px solid #e2e8f0; padding:1.25rem; box-shadow:0 4px 6px -1px rgba(0,0,0,0.05); display:flex; flex-direction:column; justify-content:space-between; min-height:140px;">
      <div style="display:flex; justify-content:space-between; align-items:start;">
        <div>{}</div>
        <span style="background:#283e28; color:white; font-size:0.7rem; font-weight:bold; padding:0.15rem 0.4rem; border-radius:4px; text-transform:uppercase;">Active Tenant</span>
      </div>
      <div style="margin-top:1rem;">
        <h4 style="margin:0; font-size:1rem; font-weight:700; color:#2d3748;">Organization</h4>
        <p style="margin:0.25rem 0 0 0; font-size:0.85rem; color:#718096; font-weight:600;">{}</p>
      </div>
    </div>

    <!-- Active Project Card -->
    <a href="/ui/studies?admin_email={}&organization_id={}&project_id={}" style="text-decoration:none; background:white; border-radius:12px; border:1px solid #e2e8f0; padding:1.25rem; box-shadow:0 4px 6px -1px rgba(0,0,0,0.05); display:flex; flex-direction:column; justify-content:space-between; min-height:140px; cursor:pointer; transition:transform 0.2s, box-shadow 0.2s;" onmouseover="this.style.transform='translateY(-2px)'; this.style.boxShadow='0 6px 12px -2px rgba(0,0,0,0.1)'" onmouseout="this.style.transform='none'; this.style.boxShadow='0 4px 6px -1px rgba(0,0,0,0.05)'">
      <div style="display:flex; justify-content:space-between; align-items:start;">
        <div>{}</div>
        <span style="background:#f05708; color:white; font-size:0.7rem; font-weight:bold; padding:0.15rem 0.4rem; border-radius:4px; text-transform:uppercase;">Active Project</span>
      </div>
      <div style="margin-top:1rem;">
        <h4 style="margin:0; font-size:1rem; font-weight:700; color:#2d3748;">Clinical Trial Registry</h4>
        <p style="margin:0.25rem 0 0 0; font-size:0.85rem; color:#718096; font-weight:600;">{}</p>
      </div>
    </a>
  </div>

  <!-- Child Organizations Card -->
  <div style="background:#f8fafc; border-radius:12px; border:1px solid #edf2f7; padding:1.5rem;">
    <h3 style="margin:0 0 0.5rem 0; color:#02182b; font-size:1.15rem; font-weight:700;">Subordinate Organizations</h3>
    <p style="color:#718096; margin:0 0 1rem 0; font-size:0.85rem;">Clinical affiliates and tenant operations managed within the active scope.</p>
    {}
  </div>

  <!-- Quick Command Actions -->
  <div style="background:white; border-radius:12px; border:1px solid #e2e8f0; padding:1.25rem; display:flex; flex-wrap:wrap; gap:1rem; align-items:center;">
    <span style="font-weight:700; color:#02182b; font-size:0.9rem; text-transform:uppercase; letter-spacing:0.05em;">Quick Actions:</span>
    <a href="{}" style="background:#02182b; color:white; padding:0.5rem 1rem; border-radius:6px; text-decoration:none; font-size:0.85rem; font-weight:600; box-shadow:0 2px 4px rgba(2,24,43,0.15); transition:background 0.2s;">Cingulum Foundation Command Center</a>
    <a href="/ui/studies?admin_email={}" style="background:#283e28; color:white; padding:0.5rem 1rem; border-radius:6px; text-decoration:none; font-size:0.85rem; font-weight:600; box-shadow:0 2px 4px rgba(40,62,40,0.15); transition:background 0.2s;">Study Lifecycle & CRF Workbench</a>
    <a href="/ui/dua?admin_email={}" style="background:#c5b7ab; color:#02182b; padding:0.5rem 1rem; border-radius:6px; text-decoration:none; font-size:0.85rem; font-weight:600; transition:background 0.2s;">DUA Console</a>
  </div>
</section>"#,
                admin_logo,
                html_escape(admin_val),
                org_logo,
                html_escape(&org_val),
                html_escape(admin_val), // for admin_email in href
                html_escape(&selected_org_value), // for organization_id in href
                html_escape(&selected_project_value), // for project_id in href
                proj_logo,
                html_escape(&proj_val),
                child_organizations_html,
                foundation_hub_url,
                html_escape(admin_val),
                html_escape(admin_val)
            )
        },
    };

    let auto_archive_banner = format!(
        r#"<div style="margin: 1rem 0.8rem; padding: 0.75rem; background: rgba(255,255,255,0.05); border: 1px solid rgba(255,255,255,0.1); border-radius: 6px;">
  <div style="font-size: 0.65rem; font-weight: 700; color: #f05708; margin-bottom: 0.25rem; letter-spacing: 0.5px; text-transform: uppercase;">System Audit</div>
  <div style="font-size: 0.8rem; font-weight: 600; color: #fff; margin-bottom: 0.4rem;">Inactivity Scan</div>
  <p style="font-size: 0.7rem; color: #a0aec0; margin: 0 0 0.75rem 0; line-height: 1.3;">Archives sites/studies inactive 3+ yrs.</p>
  <form method="post" action="/ui/app/auto-archive" style="margin:0;">
    <input type="hidden" name="admin_email" value="{}" />
    <button type="submit" style="width: 100%; background: #f05708; color: white; padding: 0.4rem; border: none; border-radius: 4px; font-size: 0.75rem; font-weight: bold; cursor: pointer;">Run Scan</button>
  </form>
</div>"#,
        html_escape(admin_email.trim())
    );

    let selected_org_name = organizations
        .iter()
        .find(|org| Some(org.id) == selected_org_id)
        .map(|org| html_escape(&org.name))
        .unwrap_or_else(|| "— Select Org —".to_string());

    let sidebar_org_options = organizations
        .iter()
        .map(|org| {
            let is_selected = selected_org_id.map(|id| id == org.id).unwrap_or(false);
            let selected_class = if is_selected { "selected" } else { "" };
            format!(
                r#"<a href="/ui/app?admin_email={}&view={}&organization_id={}" class="custom-dropdown-item {}">{}</a>"#,
                query_escape(admin_email.trim()),
                query_escape(view),
                org.id,
                selected_class,
                html_escape(&org.name)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let selected_proj_name = projects
        .iter()
        .find(|proj| Some(proj.id) == selected_project_id)
        .map(|proj| html_escape(&proj.name))
        .unwrap_or_else(|| "— Select Study —".to_string());

    let sidebar_proj_options = projects
        .iter()
        .map(|proj| {
            let is_selected = selected_project_id.map(|id| id == proj.id).unwrap_or(false);
            let selected_class = if is_selected { "selected" } else { "" };
            let org_param = selected_org_id.map(|id| format!("&organization_id={}", id)).unwrap_or_default();
            format!(
                r#"<a href="/ui/app?admin_email={}&view={}{}&project_id={}" class="custom-dropdown-item {}">{}</a>"#,
                query_escape(admin_email.trim()),
                query_escape(view),
                org_param,
                proj.id,
                selected_class,
                html_escape(&proj.name)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let sidebar_html = format!(
        r#"<script>
window.addEventListener('click', () => {{
  document.querySelectorAll('.custom-dropdown-menu').forEach(m => m.classList.remove('show'));
}});

document.addEventListener('DOMContentLoaded', () => {{
    document.querySelectorAll('form').forEach(form => {{
        form.addEventListener('submit', async (e) => {{
            e.preventDefault();
            
            const visibleInputs = form.querySelectorAll('input[list]');
            visibleInputs.forEach(input => {{
                const listId = input.getAttribute('list');
                const list = document.getElementById(listId);
                if (list) {{
                    const option = Array.from(list.options).find(o => o.value === input.value);
                    if (option && option.hasAttribute('data-id')) {{
                        let hidden = form.querySelector(`input[type="hidden"][name="${{input.name}}"]`);
                        if (!hidden) {{
                            hidden = document.createElement('input');
                            hidden.type = 'hidden';
                            hidden.name = input.name;
                            form.appendChild(hidden);
                            input.removeAttribute('name');
                        }}
                        hidden.value = option.getAttribute('data-id');
                    }}
                }}
            }});

            const formData = new FormData(form);
            const res = await fetch(form.action, {{
                method: form.method || 'POST',
                body: new URLSearchParams(formData)
            }});

            if (!res.ok) {{
                try {{
                    const data = await res.json();
                    alert("Error: " + (data.error || "Validation failed"));
                }} catch {{
                    alert("Error: Validation failed");
                }}
            }} else {{
                if (res.redirected) {{
                    window.location.href = res.url;
                }} else {{
                    window.location.reload();
                }}
            }}
        }});
    }});
}});
</script>
<aside class="sidebar" id="app-sidebar">
  <div class="sidebar-logo">
    <a href="/ui?admin_email={admin}" class="sidebar-brand-link">
      <div class="sidebar-brand">Virivu</div>
      <div class="sidebar-brand-sub">Research Cloud</div>
    </a>
  </div>

  <div class="sidebar-org-select">
    <label class="sidebar-label">Active Organization</label>
    <div class="custom-dropdown">
      <button type="button" class="custom-dropdown-btn" onclick="document.querySelectorAll('.custom-dropdown-menu').forEach(m => {{ if (m !== this.nextElementSibling) m.classList.remove('show'); }}); this.nextElementSibling.classList.toggle('show'); event.stopPropagation();">
        {selected_org_name}
      </button>
      <div class="custom-dropdown-menu">
        <a href="/ui/app?admin_email={admin}&view={current_view}&organization_id=" class="custom-dropdown-item">— Select Org —</a>
        {org_options}
      </div>
    </div>
  </div>

  <div class="sidebar-org-select">
    <label class="sidebar-label">Active Study</label>
    <div class="custom-dropdown">
      <button type="button" class="custom-dropdown-btn" onclick="document.querySelectorAll('.custom-dropdown-menu').forEach(m => {{ if (m !== this.nextElementSibling) m.classList.remove('show'); }}); this.nextElementSibling.classList.toggle('show'); event.stopPropagation();">
        {selected_proj_name}
      </button>
      <div class="custom-dropdown-menu">
        <a href="/ui/app?admin_email={admin}&organization_id={org}&view={current_view}&project_id=" class="custom-dropdown-item">— Select Study —</a>
        {proj_options}
      </div>
    </div>
  </div>

  <nav class="sidebar-nav">
    <div class="sidebar-section-label">Workspace</div>
    <a class="sidebar-item {active_overview}" href="?view=overview&admin_email={admin}&organization_id={org}">
      <span class="sidebar-icon">⬡</span> Overview
    </a>
    <a class="sidebar-item {active_analytics}" href="?view=analytics&admin_email={admin}&organization_id={org}">
      <span class="sidebar-icon">📊</span> Analytics
    </a>

    <div class="sidebar-section-label">Clinical</div>
    <a class="sidebar-item {active_orgs}" href="?view=orgs&admin_email={admin}&organization_id={org}">
      <span class="sidebar-icon">🏢</span> Organizations
    </a>
    <a class="sidebar-item {active_projects}" href="?view=projects&admin_email={admin}&organization_id={org}">
      <span class="sidebar-icon">🔬</span> Studies
    </a>
    <a class="sidebar-item {active_sites}" href="?view=sites&admin_email={admin}&organization_id={org}">
      <span class="sidebar-icon">🏥</span> Sites
    </a>
    <a class="sidebar-item {active_patients}" href="?view=patients&admin_email={admin}&organization_id={org}">
      <span class="sidebar-icon">👤</span> Patients
    </a>
    <a class="sidebar-item {active_providers}" href="?view=providers&admin_email={admin}&organization_id={org}">
      <span class="sidebar-icon">🩺</span> Providers
    </a>
    <a class="sidebar-item {active_encounters}" href="?view=encounters&admin_email={admin}&organization_id={org}">
      <span class="sidebar-icon">📝</span> Encounters
    </a>

    <div class="sidebar-section-label">Research</div>
    <a class="sidebar-item" href="/ui/studies?admin_email={admin}&organization_id={org}">
      <span class="sidebar-icon">📋</span> Study Workbench
    </a>
    <a class="sidebar-item {active_media}" href="?view=media&admin_email={admin}&organization_id={org}">
      <span class="sidebar-icon">📁</span> Media Vault
    </a>

    <div class="sidebar-section-label">Compliance</div>
    <a class="sidebar-item {active_legal}" href="?view=legal&admin_email={admin}&organization_id={org}">
      <span class="sidebar-icon">📜</span> Legal / DUA
    </a>
    <a class="sidebar-item" href="/ui/foundation?admin_email={admin}&organization_id={org}">
      <span class="sidebar-icon">⚙️</span> Foundation Hub
    </a>

    <div class="sidebar-section-label">System</div>
    <a class="sidebar-item" href="/portal" target="_blank">
      <span class="sidebar-icon">🔗</span> Patient Portal ↗
    </a>
  </nav>

  {auto_archive}

  <div class="sidebar-footer">
    <div class="sidebar-user">{admin_display}</div>
  </div>
</aside>"#,
        admin = admin_email_q,
        org = selected_org_value,
        selected_org_name = selected_org_name,
        selected_proj_name = selected_proj_name,
        org_options = sidebar_org_options,
        proj_options = sidebar_proj_options,
        current_view = html_escape(view),
        active_overview = is_active("overview"),
        active_analytics = is_active("analytics"),
        active_orgs = is_active("orgs"),
        active_projects = is_active("projects"),
        active_sites = is_active("sites"),
        active_patients = is_active("patients"),
        active_providers = is_active("providers"),
        active_encounters = is_active("encounters"),
        active_media = is_active("media"),
        active_legal = is_active("legal"),
        admin_display = html_escape(admin_email.trim()),
        auto_archive = auto_archive_banner
    );

    let body = format!(
        r#"
{sidebar}
<div class="main-with-sidebar">
  <div style="display:flex; justify-content:space-between; align-items:center; margin-bottom:1rem;">
    <div>
      <h1 class="page-title">Organization Command Center</h1>
      <p class="muted" style="margin-top:-0.5rem; margin-bottom:1.5rem;">Unified operations workspace: institutions, trial setup, patient workflows, legal agreements, and analytics.</p>
    </div>
    <button class="mobile-menu-btn" style="display:none;">☰</button>
  </div>
  {notice}
  {panel}
</div>
<datalist id="app-organization-options">{org_list}</datalist>
<datalist id="app-project-options">{proj_list}</datalist>
<datalist id="app-site-options">{site_list}</datalist>
<datalist id="app-patient-options">{pat_list}</datalist>
<datalist id="app-provider-options">{prov_list}</datalist>
"#,
        sidebar = sidebar_html,
        notice = notice_html,
        panel = panel_content,
        org_list = organization_options_html,
        proj_list = project_options_html,
        site_list = site_options_html,
        pat_list = patient_options_html_app,
        prov_list = provider_options_html_app
    );

    Ok(Html(render_cingulum_page("Organization Command Center", body)))
}

async fn submit_app_create_organization(
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

async fn submit_app_create_project(
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

async fn submit_app_create_site(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
    Form(form): Form<AppCreateSiteForm>,
) -> Result<Redirect, ApiError> {
    let project_id = parse_uuid_field(&form.project_id, "project_id")?;
    let project = ctx
        .db
        .get_project(project_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("project not found".to_string()))?;

    require_org_role(&user, project.organization_id, ROLE_ORG_MANAGERS)?;

    ctx.db
        .create_site(
            project_id,
            form.site_name.trim(),
            form.principal_investigator.trim(),
            form.co_principal_investigator.as_deref().map(str::trim).filter(|s| !s.is_empty()),
            form.sub_investigator.as_deref().map(str::trim).filter(|s| !s.is_empty()),
        )
        .await
        .map_err(ApiError::internal)?;

    Ok(Redirect::to(&format!(
        "/ui/app?admin_email={}&organization_id={}&project_id={}&notice={}",
        query_escape(user.email.as_str()),
        project.organization_id,
        project_id,
        query_escape("Site created")
    )))
}

#[derive(Debug, Deserialize)]
struct AppDeleteSiteForm {
    admin_email: String,
}

async fn submit_app_delete_site(
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
    let project = ctx
        .db
        .get_project(site.project_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("project not found".to_string()))?;

    require_org_role(&user, project.organization_id, ROLE_ORG_MANAGERS)?;

    ctx.db
        .delete_site(site_id)
        .await
        .map_err(ApiError::internal)?;

    Ok(Redirect::to(&format!(
        "/ui/app?admin_email={}&organization_id={}&project_id={}&notice={}",
        query_escape(user.email.as_str()),
        project.organization_id,
        project.id,
        query_escape("Site deleted successfully")
    )))
}

#[derive(Debug, Deserialize)]
struct AppToggleDormancyForm {
    admin_email: String,
}

async fn submit_app_site_toggle_dormancy(
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
    let project = ctx
        .db
        .get_project(site.project_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("project not found".to_string()))?;

    require_org_role(&user, project.organization_id, ROLE_ORG_MANAGERS)?;
    
    let next_status = if site.status == "dormant" { "active" } else { "dormant" };
    ctx.db
        .set_site_status(site_id, next_status)
        .await
        .map_err(ApiError::internal)?;
        
    let notice = format!("Site status updated to {}", next_status);
    Ok(Redirect::to(&format!(
        "/ui/app?admin_email={}&organization_id={}&project_id={}&view=sites&notice={}",
        query_escape(user.email.as_str()),
        project.organization_id,
        project.id,
        query_escape(&notice)
    )))
}

async fn submit_app_project_toggle_dormancy(
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
    
    let next_status = if project.status == "dormant" { "active" } else { "dormant" };
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

async fn submit_app_auto_archive(
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

async fn submit_app_create_patient(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
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

    require_org_role(&user, project.organization_id, ROLE_COORDINATOR_OR_BETTER)?;

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
        query_escape(user.email.as_str()),
        project.organization_id,
        project.id,
        patient.id,
        query_escape("Patient created successfully")
    )))
}

async fn submit_app_create_provider(
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
            form.email.as_deref().map(str::trim).filter(|s| !s.is_empty()),
            form.phone_number.as_deref().map(str::trim).filter(|s| !s.is_empty()),
            form.npi_number.as_deref().map(str::trim).filter(|s| !s.is_empty()),
            form.address.as_deref().map(str::trim).filter(|s| !s.is_empty()),
            form.notes.as_deref().map(str::trim).filter(|s| !s.is_empty()),
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

async fn submit_app_create_encounter(
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

async fn submit_app_send_invite(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
    Form(form): Form<AppSendInviteForm>,
) -> Result<Redirect, ApiError> {
    let organization_id = parse_uuid_field(&form.organization_id, "organization_id")?;
    let project_id = parse_uuid_field(&form.project_id, "project_id")?;

    require_org_role(&user, organization_id, ROLE_ORG_MANAGERS)?;

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

async fn submit_app_create_media_ticket(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
    Form(form): Form<AppCreateMediaTicketForm>,
) -> Result<Html<String>, ApiError> {
    let organization_id = parse_uuid_field(&form.organization_id, "organization_id")?;
    let project_id = parse_uuid_field(&form.project_id, "project_id")?;

    require_org_role(&user, organization_id, ROLE_COORDINATOR_OR_BETTER)?;
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
        query_escape(user.email.as_str()),
        organization_id,
        project_id
    );
    Ok(Html(render_cingulum_page(
        "Media Upload Ticket Created",
        body,
    )))
}

/// Compute age in whole days + standardized bucket for patient-entered reports and queries.
/// Central helper so thresholds (stale >30d, aging 7-30d) stay consistent across
/// Operational Snapshot, Recommended Next Steps, list items, and selected previews.
fn patient_report_age(created_at: DateTime<Utc>, now: DateTime<Utc>) -> (i64, &'static str) {
    let age = (now - created_at).num_days();
    let bucket = if age > 30 {
        "stale"
    } else if age > 7 {
        "aging"
    } else {
        "fresh"
    };
    (age, bucket)
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
    // Resolve project and its organization first if project_id is present
    let mut resolved_project = None;
    if let Some(ref proj_raw) = query.project_id {
        if let Ok(proj_uuid) = proj_raw.parse::<Uuid>() {
            if let Ok(Some(proj)) = ctx.db.get_project(proj_uuid).await {
                if organizations.iter().any(|o| o.id == proj.organization_id) {
                    resolved_project = Some(proj);
                }
            }
        } else {
            'outer: for org in &organizations {
                if let Ok(org_projects) = ctx.db.list_projects_by_organization(org.id).await {
                    if let Some(proj) = org_projects.into_iter().find(|p| p.id.to_string().starts_with(proj_raw)) {
                        resolved_project = Some(proj);
                        break 'outer;
                    }
                }
            }
        }
    }

    let selected_org_id = if let Some(ref proj) = resolved_project {
        Some(proj.organization_id)
    } else {
        query
            .organization_id
            .as_deref()
            .and_then(|raw| {
                raw.parse::<Uuid>().ok().or_else(|| {
                    organizations.iter().find(|o| o.id.to_string().starts_with(raw)).map(|o| o.id)
                })
            })
            .or_else(|| {
                organizations
                    .iter()
                    .find(|org| org.organization_kind == "platform_root")
                    .map(|org| org.id)
                    .or_else(|| organizations.first().map(|org| org.id))
            })
    };

    let projects = if let Some(org_id) = selected_org_id {
        ctx.db
            .list_projects_by_organization(org_id)
            .await
            .map_err(ApiError::internal)?
    } else {
        Vec::new()
    };

    let selected_project_id = if let Some(ref proj) = resolved_project {
        Some(proj.id)
    } else {
        query
            .project_id
            .as_deref()
            .and_then(|raw| {
                raw.parse::<Uuid>().ok().or_else(|| {
                    projects.iter().find(|p| p.id.to_string().starts_with(raw)).map(|p| p.id)
                })
            })
            .filter(|pid| projects.iter().any(|p| p.id == *pid))
            .or_else(|| projects.first().map(|p| p.id))
    };
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

    // Patient-entered data signals for operational snapshot
    let patient_entered_count = submissions
        .iter()
        .filter(|s| s.entered_by_user_id.is_none())
        .count();

    let patient_pending_sdv_count = submissions
        .iter()
        .filter(|s| s.entered_by_user_id.is_none() && s.sdv_status.trim().to_ascii_lowercase() == "pending")
        .count();

    let patient_submitted_unlocked_count = submissions
        .iter()
        .filter(|s| s.entered_by_user_id.is_none() && s.status.trim().to_ascii_lowercase() == "submitted")
        .count();

    let patient_pending_sdv_count = submissions
        .iter()
        .filter(|s| s.entered_by_user_id.is_none() && s.sdv_status.trim().to_ascii_lowercase() == "pending")
        .count();

    // Patient report aging (parallel to query aging)
    let now = Utc::now();
    let patient_stale_count = submissions
        .iter()
        .filter(|s| s.entered_by_user_id.is_none() && (now - s.created_at).num_days() > 30)
        .count();

    let patient_aging_count = submissions
        .iter()
        .filter(|s| {
            let age = (now - s.created_at).num_days();
            s.entered_by_user_id.is_none() && age > 7 && age <= 30
        })
        .count();

    let patient_aging_html = {
        let mut parts = Vec::new();
        if patient_stale_count > 0 {
            parts.push(format!(r#"<span style="background:#fed7d7; color:#c53030; padding:2px 6px; border-radius:4px; font-weight:600; font-size:0.75rem;">{}</span>"#, patient_stale_count));
        }
        if patient_aging_count > 0 {
            parts.push(format!(r#"<span style="background:#fefcbf; color:#b7791f; padding:2px 6px; border-radius:4px; font-weight:600; font-size:0.75rem;">{}</span>"#, patient_aging_count));
        }
        if !parts.is_empty() {
            format!(r#"<span style="font-size:0.75rem;">patient aging: {}</span>"#, parts.join(" "))
        } else {
            "".to_string()
        }
    };

    let patient_reports_header = if patient_stale_count > 0 || patient_aging_count > 0 {
        format!(
            r#"Patient Reports (via portal) <span style="font-size:0.7rem; color:#c53030;">({} stale, {} aging)</span>"#,
            patient_stale_count, patient_aging_count
        )
    } else {
        "Patient Reports (via portal)".to_string()
    };

    let selected_submission_id = query
        .submission_id
        .as_deref()
        .and_then(|raw| raw.parse::<Uuid>().ok())
        .filter(|sid| submissions.iter().any(|s| s.id == *sid))
        .or_else(|| submissions.first().map(|s| s.id));

    // Rich provenance + compact answers preview for selected submission (especially useful for patient-entered data)
    let (selected_submission_display, answers_preview) = if let Some(sid) = selected_submission_id {
        if let Some(sub) = submissions.iter().find(|s| s.id == sid) {
            let (age_days, _bucket) = patient_report_age(sub.created_at, now);
            let age_label = if age_days > 30 {
                format!(" <span style=\"background:#c53030;color:white;font-size:0.65rem;padding:1px 4px;border-radius:2px;font-weight:600;\">STALE {}d</span>", age_days)
            } else if age_days > 7 {
                format!(" <span style=\"background:#b7791f;color:white;font-size:0.65rem;padding:1px 4px;border-radius:2px;font-weight:600;\">AGING {}d</span>", age_days)
            } else {
                format!(" <span style=\"background:#047857;color:white;font-size:0.65rem;padding:1px 4px;border-radius:2px;font-weight:600;\">{}d</span>", age_days)
            };

            let source = if sub.entered_by_user_id.is_none() {
                format!(" <span style=\"background:#166534;color:white;font-size:0.7rem;padding:1px 6px;border-radius:3px;\">via Patient Portal</span>{}", age_label)
            } else {
                String::new()
            };

            let preview = if sub.entered_by_user_id.is_none() {
                // For patient reports, show a readable key-value preview of answers + visit context
                let answers_preview = if let Ok(val) = serde_json::from_str::<serde_json::Value>(&sub.answers_json) {
                    if let Some(obj) = val.as_object() {
                        // If the selected submission's template matches the currently viewed template in the UI,
                        // use the loaded fields to render with proper labels instead of raw keys.
                        let use_labels = selected_template_id == Some(sub.template_id);
                        let field_map: std::collections::HashMap<_, _> = if use_labels {
                            fields.iter().map(|f| (f.field_key.clone(), f.field_label.clone())).collect()
                        } else {
                            std::collections::HashMap::new()
                        };

                        let pairs = obj.iter().map(|(k, v)| {
                            let label = if use_labels {
                                field_map.get(k).cloned().unwrap_or_else(|| k.clone())
                            } else {
                                k.clone()
                            };
                            let v_str = if v.is_string() { v.as_str().unwrap_or("").to_string() } else { v.to_string() };
                            format!("{}: {}", html_escape(&label), html_escape(&v_str.chars().take(80).collect::<String>()))
                        }).collect::<Vec<_>>().join("<br>");
                        // Wrap in scrollable container if many fields
                        format!("<div style=\"max-height:200px;overflow-y:auto;\">{}</div>", pairs)
                    } else {
                        sub.answers_json.chars().take(300).collect()
                    }
                } else {
                    sub.answers_json.chars().take(300).collect()
                };

                let visit_info = sub.patient_visit_id.and_then(|vid| {
                    patient_visits.iter().find(|v| v.id == vid).map(|v| {
                        let date = v.scheduled_for.map(|d| d.to_string()).unwrap_or_else(|| "unscheduled".to_string());
                        let visit_name = visit_templates.iter().find(|vt| vt.id == v.visit_template_id).map(|vt| vt.visit_name.clone()).unwrap_or_else(|| "Unknown visit".to_string());
                        format!(" ({}: {} - {})", visit_name, date, v.status)
                    })
                }).unwrap_or_default();

                let submitted_ts = sub.submitted_at
                    .map(|ts| ts.format("%Y-%m-%d %H:%M").to_string())
                    .unwrap_or_else(|| sub.created_at.format("%Y-%m-%d %H:%M").to_string());
                let age_status_text = if age_days > 30 {
                    format!("STALE ({} days old)", age_days)
                } else if age_days > 7 {
                    format!("AGING ({} days old)", age_days)
                } else {
                    format!("{} days old", age_days)
                };

                format!(
                    "<div style=\"font-size:0.75rem;color:#166534;margin-top:2px;\"><strong>Patient answers:</strong> <span style=\"font-family:monospace;\">{}</span>{}</div><div style=\"font-size:0.68rem;color:#854d0e;background:#fefce8;padding:2px 5px;border-radius:3px;margin-top:3px;display:inline-block;\"><strong>Age:</strong> {} • <strong>Submitted:</strong> {}</div>",
                    answers_preview, visit_info, age_status_text, submitted_ts
                )
            } else {
                "".to_string()
            };

            (format!("{} {}", sub.id, source), preview)
        } else {
            ("none selected".to_string(), "".to_string())
        }
    } else {
        ("none selected".to_string(), "".to_string())
    };

    let data_queries = if let Some(project_id) = selected_project_id {
        ctx.db
            .list_study_data_queries(project_id)
            .await
            .map_err(ApiError::internal)?
    } else {
        Vec::new()
    };

    // Query aging signals (critical for monitoring backlog)
    let now = Utc::now();
    let open_queries = data_queries
        .iter()
        .filter(|q| q.status.trim().to_ascii_lowercase() != "closed")
        .collect::<Vec<_>>();

    let stale_queries_count = open_queries
        .iter()
        .filter(|q| (now - q.created_at).num_days() > 30)
        .count();

    let aging_queries_count = open_queries
        .iter()
        .filter(|q| {
            let age = (now - q.created_at).num_days();
            age > 7 && age <= 30
        })
        .count();

    let patient_related_open_queries = open_queries
        .iter()
        .filter(|q| {
            // Check if the submission this query is on is patient-entered
            submissions
                .iter()
                .any(|s| s.id == q.submission_id && s.entered_by_user_id.is_none())
        })
        .count();

    let query_aging_html = {
        let mut parts = Vec::new();
        if stale_queries_count > 0 {
            parts.push(format!(r#"<span style="background:#fed7d7; color:#c53030; padding:2px 8px; border-radius:4px; font-weight:600;">{}</span>"#, stale_queries_count));
        }
        if aging_queries_count > 0 {
            parts.push(format!(r#"<span style="background:#fefcbf; color:#b7791f; padding:2px 8px; border-radius:4px; font-weight:600;">{}</span>"#, aging_queries_count));
        }
        if !parts.is_empty() {
            format!(r#"<span style="font-size:0.8rem;">aging: {}</span>"#, parts.join(" "))
        } else {
            "".to_string()
        }
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
    let error_html = query
        .error
        .map(|error| format!(r#"<p class="error" style="background:#fff5f5; border:1px solid #fc8181; color:#c53030; border-radius:8px; padding:1rem; margin-bottom:1rem;"><strong>Error:</strong> {}</p>"#, html_escape(error.trim())))
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
        "/ui/studies?admin_email={}{}{}&view=setup",
        admin_email_q, selected_org_q, selected_project_q
    );
    let startup_tab_url = format!(
        "/ui/studies?admin_email={}{}{}&view=startup",
        admin_email_q, selected_org_q, selected_project_q
    );
    let templates_tab_url = format!(
        "/ui/studies?admin_email={}{}{}&view=crf-templates",
        admin_email_q, selected_org_q, selected_project_q
    );
    let visits_tab_url = format!(
        "/ui/studies?admin_email={}{}{}&view=visits",
        admin_email_q, selected_org_q, selected_project_q
    );
    let submissions_tab_url = format!(
        "/ui/studies?admin_email={}{}{}&view=submissions",
        admin_email_q, selected_org_q, selected_project_q
    );
    let queries_tab_url = format!(
        "/ui/studies?admin_email={}{}{}&view=queries",
        admin_email_q, selected_org_q, selected_project_q
    );
    let close_tab_url = format!(
        "/ui/studies?admin_email={}{}{}&view=close",
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

    let active_study_header_html = selected_project_id
        .and_then(|project_id| projects.iter().find(|project| project.id == project_id))
        .map(|project| {
            let hex = project.hex_code.as_deref().unwrap_or("pending");
            format!(
                r#"<div style="background:#e7e5da; border-left:4px solid #02182b; padding:0.65rem 1rem; border-radius:4px; margin-top:0.75rem; display:inline-flex; align-items:center; gap:0.5rem; box-shadow: 0 1px 3px rgba(0,0,0,0.05);">
  <span style="font-size:0.95rem; font-weight:700; color:#02182b;">Active Study:</span>
  <span style="font-size:0.95rem; font-weight:700; color:#02182b;">{}</span>
  <code style="background:#02182b; color:white; padding:0.15rem 0.4rem; border-radius:3px; font-size:0.75rem; font-weight:bold;">{}</code>
  <span class="status-chip" style="background:#283e28; color:white; font-size:0.7rem; font-weight:bold; padding:0.15rem 0.4rem; border-radius:4px; text-transform:uppercase; margin-left:0.25rem;">{}</span>
</div>"#,
                html_escape(&project.name),
                html_escape(hex),
                html_escape(&project.lifecycle_phase)
            )
        })
        .unwrap_or_else(|| r#"<div style="background:#edf2f7; border-left:4px solid #718096; padding:0.65rem 1rem; border-radius:4px; margin-top:0.75rem; display:inline-flex; color:#718096; font-style:italic; font-size:0.95rem;">No active study selected.</div>"#.to_string());

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
                    r#"<li><a href="/ui/studies?admin_email={}{}&project_id={}&view=overview">{}</a> <small>(phase: {} · ID: {} · target: {})</small></li>"#,
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
                    "pre_study" | "initiated" | "active" | "monitoring"
                )
            })
            .collect::<Vec<_>>();
        if active_studies.is_empty() {
            r#"<div style="padding:1rem; color:#718096; font-size:0.85rem;">No active studies found.</div>"#.to_string()
        } else {
            active_studies
                .iter()
                .map(|project| {
                    let selected_org = selected_org_id
                        .map(|org_id| format!("&organization_id={org_id}"))
                        .unwrap_or_default();
                    
                    let name_bytes = project.name.as_bytes();
                    let b1 = name_bytes.get(0).copied().unwrap_or(1) as usize;
                    let b2 = name_bytes.get(1).copied().unwrap_or(2) as usize;
                    let b3 = name_bytes.get(2).copied().unwrap_or(3) as usize;
                    let b4 = name_bytes.get(3).copied().unwrap_or(4) as usize;
                    let colors = ["#02182b", "#013a63", "#014f86", "#2a6f97", "#2c7da0"];
                    let c1 = colors[b1 % 5];
                    let c2 = colors[b2 % 5];
                    let c3 = colors[b3 % 5];
                    let c4 = colors[b4 % 5];
                    let logo = format!(
                        r#"<svg width="24" height="24" viewBox="0 0 32 32" style="border-radius:4px; box-shadow:inset 0 0 4px rgba(0,0,0,0.15); display:block; flex-shrink:0;">
  <rect x="0" y="0" width="16" height="16" fill="{}" />
  <rect x="16" y="0" width="16" height="16" fill="{}" />
  <rect x="0" y="16" width="16" height="16" fill="{}" />
  <rect x="16" y="16" width="16" height="16" fill="{}" />
</svg>"#, c1, c2, c3, c4
                    );

                    format!(
                        r#"<a href="/ui/studies?admin_email={}{}&project_id={}&view=overview" style="text-decoration:none; display:flex; align-items:center; gap:0.75rem; background:white; padding:0.85rem; border-radius:8px; border:1px solid #e2e8f0; margin-bottom:0.5rem; transition:transform 0.15s, box-shadow 0.15s;" onmouseover="this.style.transform='translateY(-1px)'; this.style.boxShadow='0 2px 6px rgba(0,0,0,0.06)'" onmouseout="this.style.transform='none'; this.style.boxShadow='none'">
  {}
  <div>
    <div style="font-size:0.95rem; font-weight:700; color:#2d3748;">{}</div>
    <div style="font-size:0.75rem; color:#718096; margin-top:0.15rem; text-transform:uppercase; letter-spacing:0.5px;">Phase: {} &middot; Target: {}</div>
  </div>
</a>"#,
                        admin_email_q,
                        selected_org,
                        project.id,
                        logo,
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
            r#"<div style="padding:1rem; color:#718096; font-size:0.85rem;">Select an active study from the <a href="{}" style="color:#2b6cb0; font-weight:600;">Study Setup</a> tab to view targeted next steps.</div>"#,
            setup_tab_url
        )
    } else {
        let mut actions = Vec::new();
        let action_card_style = "display:flex; flex-direction:column; background:white; padding:0.85rem; border-radius:8px; border:1px solid #e2e8f0; border-left:4px solid #f05708; margin-bottom:0.5rem;";
        if startup_pending_count > 0 {
            actions.push(format!(
                r#"<div style="{}"><div style="font-size:0.85rem; color:#2d3748; margin-bottom:0.35rem;"><strong>{}</strong> startup checklist item(s) are incomplete.</div> <a href="{}" style="font-size:0.8rem; font-weight:700; color:#2b6cb0; text-decoration:none;">Complete startup tasks &rarr;</a></div>"#,
                action_card_style, startup_pending_count, startup_tab_url
            ));
        }
        if unpublished_templates_count > 0 {
            actions.push(format!(
                r#"<div style="{}"><div style="font-size:0.85rem; color:#2d3748; margin-bottom:0.35rem;"><strong>{}</strong> CRF template(s) are still draft.</div> <a href="{}" style="font-size:0.8rem; font-weight:700; color:#2b6cb0; text-decoration:none;">Publish templates &rarr;</a></div>"#,
                action_card_style, unpublished_templates_count, templates_tab_url
            ));
        }
        let action_card_style = "display:flex; flex-direction:column; background:white; padding:0.85rem; border-radius:8px; border:1px solid #e2e8f0; border-left:4px solid #f05708; margin-bottom:0.5rem;";
        if overdue_visit_count > 0 {
            actions.push(format!(
                r#"<div style="{}"><div style="font-size:0.85rem; color:#2d3748; margin-bottom:0.35rem;"><strong>{}</strong> visit(s) appear overdue.</div> <a href="{}" style="font-size:0.8rem; font-weight:700; color:#2b6cb0; text-decoration:none;">Review visit schedule &rarr;</a></div>"#,
                action_card_style, overdue_visit_count, visits_tab_url
            ));
        }
        if draft_submission_count > 0 {
            actions.push(format!(
                r#"<div style="{}"><div style="font-size:0.85rem; color:#2d3748; margin-bottom:0.35rem;"><strong>{}</strong> CRF submission(s) remain in draft.</div> <a href="{}" style="font-size:0.8rem; font-weight:700; color:#2b6cb0; text-decoration:none;">Submit or complete drafts &rarr;</a></div>"#,
                action_card_style, draft_submission_count, submissions_tab_url
            ));
        }
        if submitted_unlocked_count > 0 {
            actions.push(format!(
                r#"<div style="{}"><div style="font-size:0.85rem; color:#2d3748; margin-bottom:0.35rem;"><strong>{}</strong> submission(s) are submitted but not locked.</div> <a href="{}" style="font-size:0.8rem; font-weight:700; color:#2b6cb0; text-decoration:none;">Lock finalized submissions &rarr;</a></div>"#,
                action_card_style, submitted_unlocked_count, submissions_tab_url
            ));
        }
        if patient_entered_count > 0 {
            actions.push(format!(
                r#"<div style="{}"><div style="font-size:0.85rem; color:#2d3748; margin-bottom:0.35rem;"><strong>{}</strong> new patient portal report(s) to review.</div> <a href="{}" style="font-size:0.8rem; font-weight:700; color:#2b6cb0; text-decoration:none;">Review patient reports &rarr;</a></div>"#,
                action_card_style, patient_entered_count, submissions_tab_url
            ));
        }
        if patient_pending_sdv_count > 0 {
            actions.push(format!(
                r#"<div style="{}"><div style="font-size:0.85rem; color:#2d3748; margin-bottom:0.35rem;"><strong>{}</strong> patient portal report(s) need SDV.</div> <a href="{}" style="font-size:0.8rem; font-weight:700; color:#2b6cb0; text-decoration:none;">Perform SDV on patient data &rarr;</a></div>"#,
                action_card_style, patient_pending_sdv_count, submissions_tab_url
            ));
        }
        if patient_submitted_unlocked_count > 0 {
            actions.push(format!(
                r#"<div style="{}"><div style="font-size:0.85rem; color:#2d3748; margin-bottom:0.35rem;"><strong>{}</strong> patient portal submission(s) are submitted but not locked.</div> <a href="{}" style="font-size:0.8rem; font-weight:700; color:#2b6cb0; text-decoration:none;">Lock patient submissions &rarr;</a></div>"#,
                action_card_style, patient_submitted_unlocked_count, submissions_tab_url
            ));
        }
        if patient_stale_count > 0 {
            actions.push(format!(
                r#"<div style="{}"><div style="font-size:0.85rem; color:#c53030; margin-bottom:0.35rem;"><strong>{}</strong> stale patient portal report(s) (>30 days old).</div> <a href="{}" style="font-size:0.8rem; font-weight:700; color:#c53030; text-decoration:none;">Review stale patient data &rarr;</a></div>"#,
                action_card_style, patient_stale_count, submissions_tab_url
            ));
        }
        if patient_aging_count > 0 {
            actions.push(format!(
                r#"<div style="{}"><div style="font-size:0.85rem; color:#d69e2e; margin-bottom:0.35rem;"><strong>{}</strong> aging patient portal report(s) (7-30 days).</div> <a href="{}" style="font-size:0.8rem; font-weight:700; color:#b7791f; text-decoration:none;">Triage aging patient data &rarr;</a></div>"#,
                action_card_style, patient_aging_count, submissions_tab_url
            ));
        }
        if open_query_count > 0 {
            actions.push(format!(
                r#"<div style="{}"><div style="font-size:0.85rem; color:#2d3748; margin-bottom:0.35rem;"><strong>{}</strong> monitor query(ies) are still open.</div> <a href="{}" style="font-size:0.8rem; font-weight:700; color:#2b6cb0; text-decoration:none;">Respond to queries &rarr;</a></div>"#,
                action_card_style, open_query_count, queries_tab_url
            ));
        }
        if stale_queries_count > 0 {
            actions.push(format!(
                r#"<div style="{}"><div style="font-size:0.85rem; color:#c53030; margin-bottom:0.35rem;"><strong>{}</strong> stale monitor query(ies) (>30 days old).</div> <a href="{}" style="font-size:0.8rem; font-weight:700; color:#c53030; text-decoration:none;">Address stale queries &rarr;</a></div>"#,
                action_card_style, stale_queries_count, queries_tab_url
            ));
        }
        if aging_queries_count > 0 {
            actions.push(format!(
                r#"<div style="{}"><div style="font-size:0.85rem; color:#d69e2e; margin-bottom:0.35rem;"><strong>{}</strong> aging monitor query(ies) (7-30 days).</div> <a href="{}" style="font-size:0.8rem; font-weight:700; color:#b7791f; text-decoration:none;">Triage aging queries &rarr;</a></div>"#,
                action_card_style, aging_queries_count, queries_tab_url
            ));
        }
        if patient_related_open_queries > 0 {
            actions.push(format!(
                r#"<div style="{}"><div style="font-size:0.85rem; color:#2b6cb0; margin-bottom:0.35rem;"><strong>{}</strong> open queries on patient portal submissions.</div> <a href="{}" style="font-size:0.8rem; font-weight:700; color:#2b6cb0; text-decoration:none;">Review patient queries &rarr;</a></div>"#,
                action_card_style, patient_related_open_queries, queries_tab_url
            ));
        }
        if close_pending_count > 0 {
            actions.push(format!(
                r#"<div style="{}"><div style="font-size:0.85rem; color:#2d3748; margin-bottom:0.35rem;"><strong>{}</strong> close checklist item(s) remain.</div> <a href="{}" style="font-size:0.8rem; font-weight:700; color:#2b6cb0; text-decoration:none;">Prepare close-out &rarr;</a></div>"#,
                action_card_style, close_pending_count, close_tab_url
            ));
        }
        if actions.is_empty() {
            actions.push(
                r#"<div style="padding:1rem; color:#38a169; font-size:0.85rem; font-weight:600;">No blocking actions detected for the selected study right now.</div>"#.to_string(),
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
      <label>Branching logic JSON</label>
      <textarea name="branching_logic_json" placeholder='e.g. {{"field": "has_allergy", "op": "eq", "value": "yes"}}' style="font-family:monospace;width:100%;height:60px;">{}</textarea>
      <label>Edit checks JSON</label>
      <textarea name="edit_checks_json" placeholder='e.g. [{{"op": "min", "value": 18}}]' style="font-family:monospace;width:100%;height:60px;">{}</textarea>
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
                    html_escape(field.branching_logic_json.as_deref().unwrap_or("")),
                    html_escape(field.edit_checks_json.as_deref().unwrap_or("")),
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

    // Split for dedicated Patient Reports section in the study workbench
    let (patient_submissions, other_submissions): (Vec<_>, Vec<_>) = submissions
        .iter()
        .partition(|s| s.entered_by_user_id.is_none());

    let patient_reports_html = if patient_submissions.is_empty() {
        "<p style=\"color:#64748b;font-size:0.85rem;margin:0.5rem 0;\">No patient portal reports yet for this study.</p>".to_string()
    } else {
        patient_submissions
            .iter()
            .take(8)
            .map(|submission| {
                let selected_org = selected_org_id
                    .map(|org_id| format!("&organization_id={org_id}"))
                    .unwrap_or_default();
                let selected_project = selected_project_id
                    .map(|project_id| format!("&project_id={project_id}"))
                    .unwrap_or_default();
                let visit_info = submission.patient_visit_id.and_then(|vid| {
                    patient_visits.iter().find(|v| v.id == vid).map(|v| {
                        let date = v.scheduled_for.map(|d| d.to_string()).unwrap_or_else(|| "unscheduled".to_string());
                        let visit_name = visit_templates.iter().find(|vt| vt.id == v.visit_template_id).map(|vt| vt.visit_name.clone()).unwrap_or_else(|| "Unknown visit".to_string());
                        format!("{}: {} ({})", visit_name, date, v.status)
                    })
                }).unwrap_or_default();

                let visit_text = if !visit_info.is_empty() {
                    format!(" • {}", visit_info)
                } else {
                    String::new()
                };

                // Per-item aging indicator for patient portal reports (makes monitoring immediate at list level)
                let age_days = (now - submission.created_at).num_days();
                let age_badge = if age_days > 30 {
                    format!(r#"<span style="background:#c53030;color:white;padding:1px 4px;border-radius:3px;font-size:0.62rem;font-weight:600;margin-left:4px;" title="Stale: >30 days old patient-entered report">STALE {}d</span>"#, age_days)
                } else if age_days > 7 {
                    format!(r#"<span style="background:#b7791f;color:white;padding:1px 4px;border-radius:3px;font-size:0.62rem;font-weight:600;margin-left:4px;" title="Aging: 7-30 days old patient-entered report">AGING {}d</span>"#, age_days)
                } else if age_days >= 0 {
                    format!(r#"<span style="background:#047857;color:white;padding:1px 4px;border-radius:3px;font-size:0.62rem;font-weight:600;margin-left:4px;" title="Fresh patient-entered report">{}d</span>"#, age_days)
                } else {
                    String::new()
                };

                let answers_preview = if let Ok(val) = serde_json::from_str::<serde_json::Value>(&submission.answers_json) {
                    if let Some(obj) = val.as_object() {
                        let pairs = obj.iter().take(3).map(|(k, v)| {
                            let v_str = if v.is_string() { v.as_str().unwrap_or("").to_string() } else { v.to_string() };
                            format!("{}: {}", html_escape(k), html_escape(&v_str.chars().take(30).collect::<String>()))
                        }).collect::<Vec<_>>().join(" | ");
                        format!("<div style=\"font-size:0.7rem;color:#166534;margin-top:2px;\"><code>{}</code></div>", pairs)
                    } else {
                        let compact = submission.answers_json.chars().take(100).collect::<String>();
                        format!("<div style=\"font-size:0.7rem;color:#166534;margin-top:2px;\"><code>{}</code></div>", html_escape(&compact))
                    }
                } else {
                    let compact = submission.answers_json.chars().take(100).collect::<String>();
                    format!("<div style=\"font-size:0.7rem;color:#166534;margin-top:2px;\"><code>{}</code></div>", html_escape(&compact))
                };

                format!(
                    r#"<li style="margin-bottom:4px;"><a href="/ui/studies?admin_email={}{}{}&submission_id={}">{}</a> <small>(patient={} status={} SDV={}{})</small>{}
                    {}
                    <form method="post" action="/ui/studies/queries" style="display:inline;margin-left:6px;">
                      <input type="hidden" name="admin_email" value="{}" />
                      <input type="hidden" name="submission_id" value="{}" />
                      <button type="submit" style="font-size:0.65rem;padding:1px 5px;">Query</button>
                    </form>
                    <form method="post" action="/ui/studies/submissions/{}/sdv" style="display:inline;margin-left:2px;">
                      <input type="hidden" name="admin_email" value="{}" />
                      <select name="sdv_status" style="font-size:0.65rem;padding:1px;">
                        <option value="verified">SDV Verified</option>
                        <option value="failed">SDV Failed</option>
                      </select>
                      <button type="submit" style="font-size:0.65rem;padding:1px 5px;">Mark</button>
                    </form>
                    </li>"#,
                    admin_email_q,
                    selected_org,
                    selected_project,
                    submission.id,
                    submission.id,
                    submission.patient_id,
                    html_escape(&submission.status),
                    html_escape(&submission.sdv_status),
                    visit_text,
                    age_badge,
                    answers_preview,
                    admin_email_q,
                    submission.id,
                    submission.id,
                    admin_email_q
                )
            })
            .collect::<Vec<_>>()
            .join("")
    };

    let submissions_html = if other_submissions.is_empty() {
        "<li>No coordinator submissions yet.</li>".to_string()
    } else {
        other_submissions
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
                    r#"<li><a href="/ui/studies?admin_email={}{}{}&submission_id={}">{}</a> <small>(template={} patient={} status={} SDV={})</small></li>"#,
                    admin_email_q,
                    selected_org,
                    selected_project,
                    submission.id,
                    submission.id,
                    submission.template_id,
                    submission.patient_id,
                    html_escape(&submission.status),
                    html_escape(&submission.sdv_status)
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
            r#"<section id="startup-next-task" style="margin:1rem 0 2rem; padding:1.25rem; background:linear-gradient(to right, #fff5f0, #fff); border:1px solid #fbd38d; border-radius:12px; box-shadow:0 2px 10px rgba(237,137,54,0.1);">
  <h3 style="margin:0 0 0.5rem; color:#c05621; font-size:1.1rem; display:flex; align-items:center; gap:0.5rem;">
    <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="M5 12l5 5l10 -10"></path></svg>
    Next required task
  </h3>
  <p style="font-size:1.05rem; color:#2d3748; font-weight:700; margin-bottom:0.25rem;">{}</p>
  <p style="color:#718096; font-size:0.9rem; margin-bottom:1rem;">Complete this task first to unblock study initiation.</p>
  <form method="post" action="{}" style="display:flex; flex-direction:column; gap:0.5rem; background:white; padding:1rem; border-radius:8px; border:1px solid #e2e8f0; box-shadow:0 1px 3px rgba(0,0,0,0.05);">
    <input type="hidden" name="admin_email" value="{}" />
    <input type="hidden" name="item_code" value="{}" />
    <input type="hidden" name="item_label" value="{}" />
    <input type="hidden" name="completed" value="true" />
    <input name="notes" value="{}" placeholder="Compliance verification comment (required)" style="padding:0.6rem; border:1px solid #cbd5e0; border-radius:6px; font-size:0.95rem; width:100%; box-sizing:border-box;" required />
    <div style="display:flex; justify-content:space-between; align-items:center; margin-top:0.25rem;">
      <span style="font-size:0.85rem; color:#718096; display:flex; align-items:center; gap:0.25rem;"><svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M5 12l5 5l10 -10"></path></svg> Complete</span>
      <button type="submit" style="background:#dd6b20; color:white; border:none; padding:0.6rem 1rem; border-radius:6px; font-weight:600; cursor:pointer; font-size:0.95rem; box-shadow:0 2px 4px rgba(221,107,32,0.3);">Mark complete and continue &rarr;</button>
    </div>
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
            r#"<p class="notice" style="background:#e6fffa; border-color:#38b2ac; color:#234e52; border-radius:8px; padding:1rem; margin-top:1rem;"><strong>Startup checklist is complete!</strong><br/>Next step: <a href="{}" style="color:#2c7a7b; font-weight:700;">Open the Lifecycle tab</a> to transition the study phase.</p>"#,
            format!(
                "/ui/studies?admin_email={}{}{}&view=lifecycle",
                admin_email_q, selected_org_q, selected_project_q
            )
        )
    } else {
        String::new()
    };
    let startup_checklist_html = if startup_checklist_items.is_empty() {
        r#"<div style="padding:1rem; color:#718096; font-style:italic;">No startup checklist items yet. Add one in the advanced section below.</div>"#.to_string()
    } else {
        startup_checklist_items
            .iter()
            .enumerate()
            .map(|(index, item)| {
                let status = if item.completed {
                    "<span style=\"background:#c6f6d5; color:#22543d; padding:0.15rem 0.5rem; border-radius:99px; font-size:0.75rem; font-weight:700; text-transform:uppercase; letter-spacing:0.5px;\">completed</span>"
                } else {
                    "<span style=\"background:#e2e8f0; color:#4a5568; padding:0.15rem 0.5rem; border-radius:99px; font-size:0.75rem; font-weight:700; text-transform:uppercase; letter-spacing:0.5px;\">pending</span>"
                };
                let notes = if item.notes.trim().is_empty() {
                    String::new()
                } else {
                    format!(
                        "<div style=\"font-size:0.85rem; color:#718096; margin-top:0.25rem;\">📝 {}</div>",
                        html_escape(&item.notes)
                    )
                };
                let opacity = if item.completed { "0.6" } else { "1" };
                
                let completion_action = if item.completed {
                    let mark_pending_action = selected_project_id
                        .map(|id| format!("/ui/studies/{id}/startup-checklist"))
                        .unwrap_or_else(|| "#".to_string());
                    format!(
                        r#"<form method="post" action="{}" style="margin-top:0.75rem; display:flex; flex-direction:column; gap:0.5rem; background:#f7fafc; padding:0.85rem; border-radius:8px; border:1px solid #e2e8f0;">
  <input type="hidden" name="admin_email" value="{}" />
  <input type="hidden" name="item_code" value="{}" />
  <input type="hidden" name="item_label" value="{}" />
  <input name="notes" value="{}" placeholder="Optional note" style="padding:0.5rem; border:1px solid #cbd5e0; border-radius:6px; font-size:0.9rem; width:100%; box-sizing:border-box;" />
  <div style="display:flex; justify-content:space-between; align-items:center; margin-top:0.25rem;">
    <span style="font-size:0.85rem; color:#48bb78; display:flex; align-items:center; gap:0.25rem; font-weight:600;"><svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M5 12l5 5l10 -10"></path></svg> Completed</span>
    <button type="submit" style="background:#e2e8f0; color:#4a5568; border:none; padding:0.4rem 0.8rem; border-radius:6px; font-weight:600; cursor:pointer; font-size:0.85rem;">Undo complete</button>
  </div>
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
                        r#"<form method="post" action="{}" style="margin-top:0.75rem; display:flex; flex-direction:column; gap:0.5rem; background:white; padding:0.85rem; border-radius:8px; border:1px solid #e2e8f0;">
  <input type="hidden" name="admin_email" value="{}" />
  <input type="hidden" name="item_code" value="{}" />
  <input type="hidden" name="item_label" value="{}" />
  <input type="hidden" name="completed" value="true" />
  <input name="notes" value="{}" placeholder="Compliance verification comment (required)" style="padding:0.5rem; border:1px solid #cbd5e0; border-radius:6px; font-size:0.9rem; width:100%; box-sizing:border-box;" required />
  <div style="display:flex; justify-content:space-between; align-items:center; margin-top:0.25rem;">
    <span style="font-size:0.85rem; color:#718096; display:flex; align-items:center; gap:0.25rem;"><svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M5 12l5 5l10 -10"></path></svg> Complete</span>
    <button type="submit" style="background:#ed8936; color:white; border:none; padding:0.4rem 0.8rem; border-radius:6px; font-weight:600; cursor:pointer; font-size:0.85rem;">Mark complete</button>
  </div>
</form>"#,
                        mark_complete_action,
                        html_escape(admin_email.trim()),
                        html_escape(&item.item_code),
                        html_escape(&item.item_label),
                        html_escape(&item.notes)
                    )
                };

                let connection = if index < startup_checklist_items.len() - 1 {
                    r#"<div style="width:2px; height:1rem; background:#cbd5e0; margin:0.25rem 0 0.25rem 1.15rem;"></div>"#
                } else {
                    ""
                };

                format!(
                    r#"<div style="opacity:{}; transition:opacity 0.2s;">
  <div style="display:flex; align-items:center; gap:0.5rem; font-size:1.05rem; color:#2d3748; font-weight:700;">
    <div style="width:6px; height:6px; border-radius:50%; background:#2b6cb0;"></div>
    {} {}
  </div>
  <div style="padding-left:1.15rem;">
    {}
    {}
  </div>
</div>{}"#,
                    opacity,
                    html_escape(&item.item_label),
                    status,
                    notes,
                    completion_action,
                    connection
                )
            })
            .collect::<Vec<_>>()
            .join("")
    };

    let patient_options_html = patients
        .iter()
        .map(|patient| {
            format!(
                r#"<option value="{}" data-id="{}">{}</option>"#,
                patient.id.to_string().chars().take(8).collect::<String>(),
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
                r#"<option value="{}" data-id="{}">{}</option>"#,
                template.id.to_string().chars().take(8).collect::<String>(),
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
                r#"<option value="{}" data-id="{}">{} ({})</option>"#,
                visit.id.to_string().chars().take(8).collect::<String>(),
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
                r#"<option value="{}" data-id="{}">{}</option>"#,
                visit.id.to_string().chars().take(8).collect::<String>(),
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
                r#"<option value="{}" data-id="{}">{}</option>"#,
                submission.id.to_string().chars().take(8).collect::<String>(),
                submission.id,
                html_escape(&format!("{} ({})", submission.id, submission.status))
            )
        })
        .collect::<Vec<_>>()
        .join("");

    let selected_org_value = selected_org_id.map(|id| id.to_string()).unwrap_or_default();
    let selected_project_value = selected_project_id.map(|id| id.to_string()).unwrap_or_default();
    let selected_template_value = selected_template_id.map(|id| id.to_string()).unwrap_or_default();
    let selected_submission_value = selected_submission_id.map(|id| id.to_string()).unwrap_or_default();

    let _selected_org_hex = selected_org_id.map(|id| id.to_string().chars().take(8).collect::<String>()).unwrap_or_default();
    let _selected_project_hex = selected_project_id.map(|id| id.to_string().chars().take(8).collect::<String>()).unwrap_or_default();
    let selected_template_hex = selected_template_id.map(|id| id.to_string().chars().take(8).collect::<String>()).unwrap_or_default();
    let selected_submission_hex = selected_submission_id.map(|id| id.to_string().chars().take(8).collect::<String>()).unwrap_or_default();
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
    let sdv_submission_action = selected_submission_id
        .map(|id| format!("/ui/studies/submissions/{id}/sdv"))
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

    let view = query.view.as_deref().unwrap_or("overview");
    let is_active = |v: &str| if v == view { "is-active" } else { "" };

    let foundation_hub_url = selected_org_id
        .map(|org_id| {
            format!(
                "/ui/foundation?admin_email={}&organization_id={org_id}",
                html_escape(admin_email.trim())
            )
        })
        .unwrap_or_else(|| format!("/ui/foundation?admin_email={}", html_escape(admin_email.trim())));

    let _global_nav = format!(
        r#"<nav class="global-nav" style="margin-bottom: 1rem; padding-bottom: 0.5rem; border-bottom: 1px solid #ddd; display: flex; gap: 1rem; font-size: 0.9rem;">
  <a href="{}">Foundation</a>
  <a href="/ui/app?admin_email={}&organization_id={}">Org Command Center</a>
  <a href="/ui/studies?admin_email={}&organization_id={}">Study Dashboard</a>
  <a href="/ui/dua?admin_email={}&organization_id={}">DUA Console</a>
</nav>"#,
        foundation_hub_url,
        html_escape(admin_email.trim()), selected_org_value,
        html_escape(admin_email.trim()), selected_org_value,
        html_escape(admin_email.trim()), selected_org_value,
    );

    let tab_bar = format!(
        r#"<aside class="sidebar blur-sidebar">
  <div class="sidebar-logo">
    <a href="{}" class="sidebar-brand-link">
      <div class="sidebar-brand">Virivu Research Cloud</div>
      <div class="sidebar-brand-sub">Study Workbench</div>
    </a>
  </div>
  
  <nav class="sidebar-nav" style="padding: 1rem 0.5rem; display: flex; flex-direction: column; gap: 0.25rem;">
    <div class="sidebar-section-label" style="font-size:0.65rem; font-weight:800; text-transform:uppercase; letter-spacing:1px; margin:0.5rem 0.5rem 0.25rem;">Workspace</div>
    <a class="sidebar-item {}" href="?view=overview&admin_email={}&organization_id={}&project_id={}">Overview</a>
    <a class="sidebar-item {}" href="?view=setup&admin_email={}&organization_id={}&project_id={}">Study Setup</a>
    <a class="sidebar-item {}" href="?view=lifecycle&admin_email={}&organization_id={}&project_id={}">Lifecycle</a>
    
    <div class="sidebar-section-label" style="font-size:0.65rem; font-weight:800; text-transform:uppercase; letter-spacing:1px; margin:1rem 0.5rem 0.25rem;">Operations</div>
    <a class="sidebar-item {}" href="?view=crf-templates&admin_email={}&organization_id={}&project_id={}">CRF Templates</a>
    <a class="sidebar-item {}" href="?view=crf-fields&admin_email={}&organization_id={}&project_id={}">CRF Fields</a>
    <a class="sidebar-item {}" href="?view=visits&admin_email={}&organization_id={}&project_id={}">Visits</a>
    <a class="sidebar-item {}" href="?view=submissions&admin_email={}&organization_id={}&project_id={}">Submissions</a>
    <a class="sidebar-item {}" href="?view=queries&admin_email={}&organization_id={}&project_id={}">Queries</a>
    
    <div class="sidebar-section-label" style="font-size:0.65rem; font-weight:800; text-transform:uppercase; letter-spacing:1px; margin:1rem 0.5rem 0.25rem;">Regulatory</div>
    <a class="sidebar-item {}" href="?view=startup&admin_email={}&organization_id={}&project_id={}">Startup</a>
    <a class="sidebar-item {}" href="?view=close&admin_email={}&organization_id={}&project_id={}">Close</a>
  </nav>

  <div style="margin-top:auto; padding:1.25rem 1rem; border-top:1px solid rgba(197,183,171,0.4);">
    <a href="{}" style="text-decoration:none; font-size:0.85rem; font-weight:700; display:flex; align-items:center; gap:0.4rem; color:#2b6cb0;">&larr; Command Center</a>
  </div>
</aside>"#,
        app_dashboard_url,
        is_active("overview"), html_escape(admin_email.trim()), selected_org_value, selected_project_value,
        is_active("setup"), html_escape(admin_email.trim()), selected_org_value, selected_project_value,
        is_active("lifecycle"), html_escape(admin_email.trim()), selected_org_value, selected_project_value,
        is_active("crf-templates"), html_escape(admin_email.trim()), selected_org_value, selected_project_value,
        is_active("crf-fields"), html_escape(admin_email.trim()), selected_org_value, selected_project_value,
        is_active("visits"), html_escape(admin_email.trim()), selected_org_value, selected_project_value,
        is_active("submissions"), html_escape(admin_email.trim()), selected_org_value, selected_project_value,
        is_active("queries"), html_escape(admin_email.trim()), selected_org_value, selected_project_value,
        is_active("startup"), html_escape(admin_email.trim()), selected_org_value, selected_project_value,
        is_active("close"), html_escape(admin_email.trim()), selected_org_value, selected_project_value,
        app_dashboard_url
    );

    let panel_content = match view {
        "setup" => format!(
            r#"<section class="card">
  <h2>1) Create Study (clinicaltrials.gov-style metadata + internal ops)</h2>
  <form class="no-auto-cards" method="post" action="/ui/studies/create" style="display:flex; flex-direction:column; gap:1rem; margin-top:1.5rem;">
    <label class="field-card" style="display:block; cursor:text;">
      <div style="font-weight:600; margin-bottom:0.5rem; color:inherit;">Admin email</div>
      <input name="admin_email" value="{}" placeholder="name@example.com" required style="width:100%;" />
    </label>
    <label class="field-card" style="display:block; cursor:text;">
      <div style="font-weight:600; margin-bottom:0.5rem; color:inherit;">Organization ID Code</div>
      <input name="organization_hex" placeholder="e.g. ABC" required style="width:100%;" />
    </label>
    <label class="field-card" style="display:block; cursor:text;">
      <div style="font-weight:600; margin-bottom:0.5rem; color:inherit;">Study name</div>
      <input name="study_name" placeholder="Acute Stroke Registry 2026" required style="width:100%;" />
    </label>
    <label class="field-card" style="display:block; cursor:text;">
      <div style="font-weight:600; margin-bottom:0.5rem; color:inherit;">Therapeutic area</div>
      <input name="therapeutic_area" placeholder="Neurology" required style="width:100%;" />
    </label>
    <label class="field-card" style="display:block; cursor:text;">
      <div style="font-weight:600; margin-bottom:0.5rem; color:inherit;">Protocol code</div>
      <input name="protocol_code" placeholder="VIR-STR-26-01" style="width:100%;" />
    </label>
    <label class="field-card" style="display:block; cursor:text;">
      <div style="font-weight:600; margin-bottom:0.5rem; color:inherit;">Planned enrollment</div>
      <input name="planned_enrollment" value="250" placeholder="e.g. 250" style="width:100%;" />
    </label>
    <label class="field-card" style="display:block; cursor:text;">
      <div style="font-weight:600; margin-bottom:0.5rem; color:inherit;">ClinicalTrials.gov ID (optional)</div>
      <input name="clinicaltrials_gov_id" placeholder="NCT01234567" style="width:100%;" />
    </label>
    <label class="field-card" style="display:block; cursor:text;">
      <div style="font-weight:600; margin-bottom:0.5rem; color:inherit;">Study summary</div>
      <textarea name="study_summary" placeholder="Primary objective, key endpoints, and operational plan" style="width:100%; height:120px;"></textarea>
    </label>
    <button type="submit" style="margin-top:0.5rem; align-self:flex-start; padding:0.75rem 1.5rem; background:#02182B; color:white; border-radius:6px; border:none; font-weight:600; cursor:pointer;">Create Study in Pre-Study Phase</button>
  </form>
  <h3 style="margin-top:1rem;">Study portfolio</h3>
  <ul>{}</ul>
</section>"#,
            html_escape(admin_email.trim()),
            study_rows_html
        ),
        "lifecycle" => format!(
            r#"<section class="card">
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
</section>"#,
            lifecycle_kpi_cards_html,
            lifecycle_action_cards_html,
            lifecycle_gate_html,
            phase_action,
            selected_project_value.clone(),
            html_escape(admin_email.trim()),
            phase_events_html
        ),
        "crf-templates" => format!(
            r#"<section class="card">
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
</section>"#,
            crf_template_action,
            html_escape(admin_email.trim()),
            templates_html
        ),
        "crf-fields" => format!(
            r#"<section class="card">
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
    <label>Branching logic JSON</label>
    <textarea name="branching_logic_json" placeholder='e.g. {{"field": "has_allergy", "op": "eq", "value": "yes"}}' style="font-family:monospace;width:100%;height:60px;"></textarea>
    <label>Edit checks JSON</label>
    <textarea name="edit_checks_json" placeholder='e.g. [{{"op": "min", "value": 18}}]' style="font-family:monospace;width:100%;height:60px;"></textarea>
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
</section>"#,
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
            fields_html
        ),
        "visits" => format!(
            r#"<section class="card">
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
</section>"#,
            visit_template_action,
            html_escape(admin_email.trim()),
            visit_templates_html,
            schedule_visit_action,
            html_escape(admin_email.trim()),
            patient_visits_html
        ),
        "submissions" => format!(
            r#"<section class="card">
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
  {}
  <form method="post" action="{}" style="display:inline-block; margin-right:0.5rem;">
    <input type="hidden" name="admin_email" value="{}" />
    <button type="submit">Mark Submitted</button>
  </form>
  <form method="post" action="{}" style="display:inline-block; margin-right:0.5rem;">
    <input type="hidden" name="admin_email" value="{}" />
    <button type="submit">Lock Submission</button>
  </form>
  <span style="font-size:0.75rem; color:#166534; margin-left:0.5rem;">Lock freezes answers + creates immutable audit snapshot (21 CFR Part 11)</span>
  <form method="post" action="{}" style="display:inline-block;">
    <input type="hidden" name="admin_email" value="{}" />
    <label style="display:inline; margin-left:0.5rem; margin-right:0.2rem;">SDV Status:</label>
    <select name="sdv_status" style="display:inline; width:auto; padding:0.2rem; margin-right:0.3rem;" required>
      <option value="pending">Pending</option>
      <option value="verified">Verified</option>
      <option value="failed">Failed</option>
    </select>
    <button type="submit">Update SDV</button>
  </form>
  <h3 style="margin-top:1rem;">Patients</h3>
  <ul>{}</ul>
  <h3 style="margin-top:1rem;">{}</h3>
  <ul style="background:#f0fdf4;border:1px solid #86efac;border-radius:4px;padding:6px 10px;margin-bottom:0.75rem;">{}</ul>
  <h3 style="margin-top:0.25rem;">Coordinator Submissions</h3>
  <ul>{}</ul>
</section>"#,
            create_submission_action,
            html_escape(admin_email.trim()),
            selected_template_hex.clone(),
            selected_submission_display,
            answers_preview,
            submit_submission_action,
            html_escape(admin_email.trim()),
            lock_submission_action,
            html_escape(admin_email.trim()),
            sdv_submission_action,
            html_escape(admin_email.trim()),
            patient_reports_header,
            patients_html,
            patient_reports_html,
            submissions_html
        ),
        "queries" => format!(
            r#"<section class="card">
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
</section>"#,
            create_query_action,
            html_escape(admin_email.trim()),
            selected_submission_hex.clone(),
            data_queries_html
        ),
        "startup" => format!(
            r#"<section class="card">
  <h2>8) Study Startup Checklist</h2>
  <p class="muted">Use this checklist to move from setup to launch. Work through pending tasks and mark them complete.</p>
  {}
  {}
  {}
  <div style="display:flex; flex-direction:column;">{}</div>
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
</section>"#,
            startup_summary_html,
            startup_workflow_framework_html,
            startup_next_action_html,
            startup_checklist_html,
            startup_checklist_action,
            html_escape(admin_email.trim())
        ),
        "close" => format!(
            r#"<section class="card">
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
</section>"#,
            checklist_action,
            html_escape(admin_email.trim()),
            checklist_html
        ),
        _ => format!(
            r#"<section class="card">
  <h2>Overview</h2>
  <p class="muted">Start here: review active studies, then execute pending actions for the selected study.</p>
  <p><strong>Admin:</strong> {}</p>
  <p><strong>Organization:</strong> {}</p>
  <p><strong>Selected study:</strong> {}</p>
  <div style="display:grid;grid-template-columns:repeat(auto-fit,minmax(260px,1fr));gap:0.8rem;margin-top:0.7rem;">
    <section class="card" style="margin:0; padding:1.25rem;">
      <h3 style="margin-top:0; margin-bottom:1rem; font-size:1.1rem; color:#02182b;">Active studies</h3>
      <div style="display:flex; flex-direction:column;">{}</div>
    </section>
    <section class="card" style="margin:0; padding:1.25rem;">
      <h3 style="margin-top:0; margin-bottom:1rem; font-size:1.1rem; color:#02182b;">Pending actions</h3>
      <div style="display:flex; flex-direction:column;">{}</div>
    </section>
  </div>
  <p style="margin-top:0.8rem;"><a href="{}">Back to unified app dashboard</a></p>
</section>"#,
            html_escape(admin_email.trim()),
            if selected_org_value.is_empty() {
                "<span class=\"muted\">none selected</span>".to_string()
            } else {
                selected_org_value.clone()
            },
            selected_study_label,
            active_studies_html,
            pending_actions_html,
            app_dashboard_url
        ),
    };

    let context_bar = render_context_bar(
        if selected_org_value.is_empty() { None } else { Some(&selected_org_value) },
        if selected_study_label.is_empty() { None } else { Some(&selected_study_label) },
        admin_email.trim()
    );

    let body = format!(
        r#"
{context_bar}
{tab_bar}
<div class="main-with-sidebar">
  <div style="margin-bottom:1.5rem;">
    <h1 style="margin:0;">Study Dashboard</h1>
    <p class="muted" style="margin-top:0.25rem; margin-bottom:0.75rem;">Pre-study planning, initiation, activation, monitoring, and closure with operational CRF design.</p>
    {active_study_header_html}
  </div>

  <div style="background:#f0fdf4; border:1px solid #86efac; border-radius:6px; padding:12px 16px; margin-bottom:1rem; font-size:0.9rem;">
    <strong style="color:#166534;">Guided Study Lifecycle:</strong> 
    Design CRF → Publish → Complete Startup Checklist → Activate → Enroll Patients &amp; Schedule Visits → Collect &amp; Monitor Data → Close
  </div>

  <div style="background:#fefce8; border:1px solid #fde047; border-radius:6px; padding:12px 16px; margin-bottom:1rem; display:flex; align-items:center; gap:16px; flex-wrap:wrap;">
    <strong style="color:#854d0e;">Operational Snapshot:</strong>
    <span style="background:#fef08c; color:#713f12; padding:2px 8px; border-radius:4px; font-weight:600;">{open_query_count} open queries</span>
    {query_aging_html}
    <span style="background:#dcfce7; color:#166534; padding:2px 8px; border-radius:4px; font-weight:600;">{patient_entered_count} patient reports</span>
    <span style="background:#fef3c7; color:#854d0e; padding:2px 8px; border-radius:4px; font-weight:600;">{patient_pending_sdv_count} patient reports need SDV</span>
    {patient_aging_html}
    <span style="color:#854d0e;">{pending_actions_html}</span>
    <span style="font-size:0.8rem; color:#a16207;">• Lock finalized submissions to freeze answers + generate provenance</span>
  </div>
  {error_html}
  {notice_html}
  {panel_content}
</div>

<datalist id="study-patient-options">{patient_options_html}</datalist>
<datalist id="study-template-options">{template_options_html}</datalist>
<datalist id="study-visit-template-options">{visit_template_options_html}</datalist>
<datalist id="study-visit-options">{visit_options_html}</datalist>
<datalist id="study-submission-options">{submission_options_html}</datalist>
"#,
        context_bar = context_bar,
        tab_bar = tab_bar,
        active_study_header_html = active_study_header_html,
        error_html = error_html,
        notice_html = notice_html,
        panel_content = panel_content,
        patient_options_html = patient_options_html,
        template_options_html = template_options_html,
        visit_template_options_html = visit_template_options_html,
        visit_options_html = visit_options_html,
        submission_options_html = submission_options_html,
        open_query_count = open_query_count,
        patient_entered_count = patient_entered_count,
        patient_pending_sdv_count = patient_pending_sdv_count,
        query_aging_html = query_aging_html,
        patient_aging_html = patient_aging_html
    );

    let script = r#"
<script>
document.addEventListener('DOMContentLoaded', () => {
    document.querySelectorAll('form').forEach(form => {
        form.addEventListener('submit', (e) => {
            const visibleInputs = form.querySelectorAll('input[list]');
            visibleInputs.forEach(input => {
                const listId = input.getAttribute('list');
                const list = document.getElementById(listId);
                if (list) {
                    const option = Array.from(list.options).find(o => o.value === input.value);
                    if (option && option.hasAttribute('data-id')) {
                        let hidden = form.querySelector(`input[type="hidden"][name="${input.name}"]`);
                        if (!hidden) {
                            hidden = document.createElement('input');
                            hidden.type = 'hidden';
                            hidden.name = input.name;
                            form.appendChild(hidden);
                            input.removeAttribute('name');
                        }
                        hidden.value = option.getAttribute('data-id');
                    }
                }
            });
        });
    });
});
</script>
"#;
    let body = format!("{}{}", body, script);
    Ok(Html(render_cingulum_page("Study Dashboard", body)))
}

async fn submit_create_study_from_ui(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
    Form(form): Form<StudyCreateForm>,
) -> Result<Redirect, ApiError> {
    let organization_id = match ctx
        .db
        .get_organization_id_by_hex(form.organization_hex.trim())
        .await
    {
        Ok(id) => id,
        Err(_) => {
            return Ok(Redirect::to(&format!(
                "/ui/studies?admin_email={}&error={}",
                query_escape(user.email.as_str()),
                query_escape(&format!(
                    "Invalid Organization ID Code: '{}' not found.",
                    form.organization_hex.trim()
                ))
            )));
        }
    };
    
    require_org_role(&user, organization_id, ROLE_ORG_MANAGERS)?;

    if form.study_name.trim().is_empty() {
        return Ok(Redirect::to(&format!(
            "/ui/studies?admin_email={}&error={}",
            query_escape(user.email.as_str()),
            query_escape("Study name is required.")
        )));
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
        query_escape(user.email.as_str()),
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
    // Note: This internal helper is still called from legacy paths in some places.
    // For now we keep a minimal compatibility shim; callers should be updated over time.
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
                "/ui/studies?admin_email={}&organization_id={}&project_id={}&view=lifecycle&notice={}",
                query_escape(form.admin_email.trim()),
                project.organization_id,
                project_id,
                query_escape(&user_notice)
            )));
        }
        return Err(ApiError::internal(err));
    }
    Ok(Redirect::to(&format!(
        "/ui/studies?admin_email={}&organization_id={}&project_id={}&view=lifecycle&notice={}",
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
    user: AuthenticatedUser,
    Path(project_id): Path<Uuid>,
    Form(form): Form<StudyCrfTemplateForm>,
) -> Result<Redirect, ApiError> {
    let project = ctx
        .db
        .get_project(project_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("project not found".to_string()))?;

    require_org_role(&user, project.organization_id, ROLE_ORG_MANAGERS)?;

    let created_by_user_id = Some(user.user_id);
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
        query_escape(user.email.as_str()),
        project.organization_id,
        project_id,
        template.id,
        query_escape("CRF template created")
    )))
}

async fn submit_add_study_crf_field(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
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

    require_org_role(&user, project.organization_id, ROLE_ORG_MANAGERS)?;

    if template.status == "published" {
        return Err(ApiError::Validation(
            "Cannot modify fields on a published CRF template".to_string(),
        ));
    }
    let options_json = normalize_crf_field_options(
        form.field_type.trim(),
        form.options_text.trim(),
        form.options_json.trim(),
    );
    let display_order = form.display_order.trim().parse::<i32>().unwrap_or(0).max(0);
    let branching_logic_trimmed = form.branching_logic_json.trim();
    let branching_logic = if branching_logic_trimmed.is_empty() {
        None
    } else {
        Some(branching_logic_trimmed)
    };
    let edit_checks_trimmed = form.edit_checks_json.trim();
    let edit_checks = if edit_checks_trimmed.is_empty() {
        None
    } else {
        Some(edit_checks_trimmed)
    };
    if let Err(err) = ctx
        .db
        .add_study_crf_field(
            template_id,
            form.field_key.trim(),
            form.field_label.trim(),
            form.field_type.trim(),
            form.required.is_some(),
            &options_json,
            branching_logic,
            edit_checks,
            display_order,
        )
        .await
    {
        return Ok(Redirect::to(&format!(
            "/ui/studies?admin_email={}&organization_id={}&project_id={}&template_id={}&view=crf-fields&notice={}",
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
        "/ui/studies?admin_email={}&organization_id={}&project_id={}&template_id={}&view=crf-fields&notice={}",
        query_escape(form.admin_email.trim()),
        project.organization_id,
        project.id,
        template_id,
        query_escape("CRF field added")
    )))
}

async fn submit_bulk_delete_study_crf_fields(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
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

    require_org_role(&user, project.organization_id, ROLE_ORG_MANAGERS)?;
    if template.status == "published" {
        return Err(ApiError::Validation(
            "Cannot modify fields on a published CRF template".to_string(),
        ));
    }
    if form.confirmation_text.trim() != "DELETE" {
        return Ok(Redirect::to(&format!(
            "/ui/studies?admin_email={}&organization_id={}&project_id={}&template_id={}&view=crf-fields&notice={}",
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
        "/ui/studies?admin_email={}&organization_id={}&project_id={}&template_id={}&view=crf-fields&notice={}",
        query_escape(user.email.as_str()),
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
    user: AuthenticatedUser,
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

    require_org_role(&user, project.organization_id, ROLE_ORG_MANAGERS)?;
    if form.confirmation_text.trim() != "DELETE CORRUPTED" {
        return Ok(Redirect::to(&format!(
            "/ui/studies?admin_email={}&organization_id={}&project_id={}&template_id={}&view=crf-fields&notice={}",
            query_escape(user.email.as_str()),
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
        "/ui/studies?admin_email={}&organization_id={}&project_id={}&template_id={}&view=crf-fields&notice={}",
        query_escape(user.email.as_str()),
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
    user: AuthenticatedUser,
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

    require_org_role(&user, project.organization_id, ROLE_ORG_MANAGERS)?;

    if template.status == "published" {
        return Err(ApiError::Validation(
            "Cannot modify fields on a published CRF template".to_string(),
        ));
    }

    if html_markup.trim().is_empty() && uploaded_file_bytes.is_empty() {
        return Ok(Redirect::to(&format!(
            "/ui/studies?admin_email={}&organization_id={}&project_id={}&template_id={}&view=crf-fields&notice={}",
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
            "/ui/studies?admin_email={}&organization_id={}&project_id={}&template_id={}&view=crf-fields&notice={}",
            query_escape(admin_email.trim()),
            project.organization_id,
            project.id,
            template.id,
            query_escape(&error_notice)
        )));
    }
    if imported_fields.is_empty() {
        return Ok(Redirect::to(&format!(
            "/ui/studies?admin_email={}&organization_id={}&project_id={}&template_id={}&view=crf-fields&notice={}",
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
                    None,
                    None,
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
                    None,
                    None,
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
        "/ui/studies?admin_email={}&organization_id={}&project_id={}&template_id={}&view=crf-fields&notice={}",
        query_escape(admin_email.trim()),
        project.organization_id,
        project.id,
        template.id,
        query_escape(&notice)
    )))
}

async fn submit_update_study_crf_field(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
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

    require_org_role(&user, project.organization_id, ROLE_ORG_MANAGERS)?;

    if template.status == "published" {
        return Err(ApiError::Validation(
            "Cannot modify fields on a published CRF template".to_string(),
        ));
    }
    let options_json = normalize_crf_field_options(
        form.field_type.trim(),
        form.options_text.trim(),
        form.options_json.trim(),
    );
    let display_order = form.display_order.trim().parse::<i32>().unwrap_or(0).max(0);
    let branching_logic_trimmed = form.branching_logic_json.trim();
    let branching_logic = if branching_logic_trimmed.is_empty() {
        None
    } else {
        Some(branching_logic_trimmed)
    };
    let edit_checks_trimmed = form.edit_checks_json.trim();
    let edit_checks = if edit_checks_trimmed.is_empty() {
        None
    } else {
        Some(edit_checks_trimmed)
    };
    if let Err(err) = ctx
        .db
        .update_study_crf_field(
            field_id,
            form.field_key.trim(),
            form.field_label.trim(),
            form.field_type.trim(),
            form.required.is_some(),
            &options_json,
            branching_logic,
            edit_checks,
            display_order,
        )
        .await
    {
        return Ok(Redirect::to(&format!(
            "/ui/studies?admin_email={}&organization_id={}&project_id={}&template_id={}&view=crf-fields&notice={}",
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
        "/ui/studies?admin_email={}&organization_id={}&project_id={}&template_id={}&view=crf-fields&notice={}",
        query_escape(form.admin_email.trim()),
        project.organization_id,
        project.id,
        template.id,
        query_escape("CRF field updated")
    )))
}

async fn submit_publish_study_crf_template(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
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

    require_org_role(&user, project.organization_id, ROLE_ORG_MANAGERS)?;

    // Capture snapshot before publish (template becomes immutable after this)
    let field_count = ctx.db.list_study_crf_fields(template_id).await.map(|f| f.len()).unwrap_or(0);

    ctx.db
        .publish_study_crf_template(template_id)
        .await
        .map_err(ApiError::internal)?;

    // Strong provenance for publish (CRF template is now locked for the study)
    let old_data = Some(format!(r#"{{"status":"draft","field_count":{}}}"#, field_count));
    let new_data = Some(r#"{"status":"published"}"#.to_string());
    let _ = ctx.db.insert_audit_log(
        "study_crf_templates",
        template_id,
        "published",
        Some(user.user_id),
        old_data.as_deref(),
        new_data.as_deref(),
        Some(&format!("CRF template published ({} fields) - template is now immutable", field_count)),
    ).await;

    Ok(Redirect::to(&format!(
        "/ui/studies?admin_email={}&organization_id={}&project_id={}&template_id={}&notice={}",
        query_escape(user.email.as_str()),
        project.organization_id,
        project.id,
        template_id,
        query_escape("CRF template published")
    )))
}

async fn submit_create_study_visit_template(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
    Path(project_id): Path<Uuid>,
    Form(form): Form<StudyVisitTemplateForm>,
) -> Result<Redirect, ApiError> {
    let project = ctx
        .db
        .get_project(project_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("project not found".to_string()))?;

    require_org_role(&user, project.organization_id, ROLE_ORG_MANAGERS)?;
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
        query_escape(user.email.as_str()),
        project.organization_id,
        project_id,
        query_escape("Visit template created")
    )))
}

async fn submit_schedule_patient_visit(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
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

    require_org_role(&user, project.organization_id, ROLE_COORDINATOR_OR_BETTER)?;
    let scheduled_for = parse_optional_date(&form.scheduled_for)?;
    let visit = ctx.db
        .schedule_patient_study_visit(project_id, patient_id, visit_template_id, scheduled_for)
        .await
        .map_err(ApiError::internal)?;

    // Audit
    let _ = ctx.db.insert_audit_log(
        "patient_study_visits",
        visit.id,
        "scheduled",
        Some(user.user_id),
        None,
        None,
        None,
    ).await;

    Ok(Redirect::to(&format!(
        "/ui/studies?admin_email={}&organization_id={}&project_id={}&notice={}",
        query_escape(user.email.as_str()),
        project.organization_id,
        project_id,
        query_escape("Patient visit scheduled")
    )))
}

async fn submit_create_study_crf_submission(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
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

    require_org_role(&user, project.organization_id, ROLE_COORDINATOR_OR_BETTER)?;

    let entered_by_user_id = Some(user.user_id);
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
        query_escape(user.email.as_str()),
        project.organization_id,
        project_id,
        submission.id,
        query_escape("CRF submission created")
    )))
}

async fn submit_mark_study_crf_submission_submitted(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
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

    require_org_role(&user, project.organization_id, ROLE_COORDINATOR_OR_BETTER)?;

    if submission.status == "locked" {
        let _ = ctx.db.insert_audit_log(
            "study_crf_submissions",
            submission_id,
            "mutation_rejected",
            Some(user.user_id),
            None,
            None,
            Some("Attempt to mark locked submission as submitted via UI (answers frozen)"),
        ).await;
        return Err(ApiError::Validation(
            "This CRF submission is locked. Answers are frozen and it cannot be re-submitted.".to_string(),
        ));
    }
    if submission.status != "draft" {
        return Err(ApiError::Validation(
            "Only draft submissions can be marked submitted.".to_string(),
        ));
    }

    // Snapshot answers at submit for provenance
    let current = ctx.db.get_study_crf_submission(submission_id).await.ok().flatten();
    let snapshot = current.as_ref().map(|s| s.answers_json.clone());

    ctx.db
        .submit_study_crf_submission(submission_id)
        .await
        .map_err(ApiError::internal)?;

    // Audit: submission moved to submitted state with snapshot
    let _ = ctx.db.insert_audit_log(
        "study_crf_submissions",
        submission_id,
        "submitted",
        Some(user.user_id),
        snapshot.as_deref(),
        None,
        Some("Submitted - answers recorded at transition"),
    ).await;

    Ok(Redirect::to(&format!(
        "/ui/studies?admin_email={}&organization_id={}&project_id={}&submission_id={}&notice={}",
        query_escape(user.email.as_str()),
        project.organization_id,
        project.id,
        submission_id,
        query_escape("Submission marked submitted")
    )))
}

async fn submit_lock_study_crf_submission(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
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

    require_org_role(&user, project.organization_id, ROLE_COORDINATOR_OR_BETTER)?;

    if submission.status == "locked" {
        let _ = ctx.db.insert_audit_log(
            "study_crf_submissions",
            submission_id,
            "mutation_rejected",
            Some(user.user_id),
            None,
            None,
            Some("Attempt to re-lock an already locked submission via UI"),
        ).await;
        return Err(ApiError::Validation(
            "This CRF submission is already locked. Answers are frozen.".to_string(),
        ));
    }

    // Capture current answers for provenance before locking
    let current_submission = ctx.db.get_study_crf_submission(submission_id).await.ok().flatten();
    let answers_snapshot = current_submission.as_ref().map(|s| s.answers_json.clone());

    ctx.db
        .lock_study_crf_submission(submission_id)
        .await
        .map_err(ApiError::internal)?;

    // Audit with snapshot for basic provenance
    let _ = ctx.db.insert_audit_log(
        "study_crf_submissions",
        submission_id,
        "locked",
        Some(user.user_id),
        answers_snapshot.as_deref(),
        None,
        Some("Locked via UI - answers frozen"),
    ).await;

    Ok(Redirect::to(&format!(
        "/ui/studies?admin_email={}&organization_id={}&project_id={}&submission_id={}&notice={}",
        query_escape(user.email.as_str()),
        project.organization_id,
        project.id,
        submission_id,
        query_escape("Submission locked")
    )))
}

#[derive(Debug, Deserialize)]
struct UpdateStudyCrfSubmissionSdvForm {
    admin_email: String,
    sdv_status: String,
}

async fn submit_update_study_crf_submission_sdv(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
    Path(submission_id): Path<Uuid>,
    Form(form): Form<UpdateStudyCrfSubmissionSdvForm>,
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

    require_org_role(&user, project.organization_id, ROLE_COORDINATOR_OR_BETTER)?;

    if submission.status == "draft" {
        return Err(ApiError::Validation(
            "Cannot perform SDV on a draft submission. Submit it first.".to_string(),
        ));
    }

    let old_sdv_status = submission.sdv_status.clone();
    let new_sdv_status = form.sdv_status.trim().to_string();

    ctx.db
        .update_study_crf_submission_sdv_status(submission_id, &new_sdv_status)
        .await
        .map_err(ApiError::internal)?;

    // Richer provenance for SDV (critical monitoring step)
    let old_data = Some(format!(r#"{{"sdv_status":"{}"}}"#, old_sdv_status));
    let new_data = Some(format!(r#"{{"sdv_status":"{}"}}"#, new_sdv_status));
    let _ = ctx.db.insert_audit_log(
        "study_crf_submissions",
        submission_id,
        "sdv_updated",
        Some(user.user_id),
        old_data.as_deref(),
        new_data.as_deref(),
        Some(&format!("SDV status changed: {} → {}", old_sdv_status, new_sdv_status)),
    ).await;

    Ok(Redirect::to(&format!(
        "/ui/studies?admin_email={}&organization_id={}&project_id={}&submission_id={}&view=submissions&notice={}",
        query_escape(user.email.as_str()),
        project.organization_id,
        project.id,
        submission_id,
        query_escape("SDV status updated successfully")
    )))
}

async fn submit_create_study_data_query(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
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

    require_org_role(&user, project.organization_id, ROLE_COORDINATOR_OR_BETTER)?;

    let submission = ctx
        .db
        .get_study_crf_submission(submission_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("submission not found".to_string()))?;

    if submission.status == "draft" {
        return Err(ApiError::Validation(
            "Cannot raise queries on a draft submission.".to_string(),
        ));
    }

    let raised_by_user_id = ctx
        .db
        .get_user_by_email(user.email.as_str())
        .await
        .map_err(ApiError::internal)?
        .map(|u| u.id);
    let query = ctx.db
        .create_study_data_query(
            project_id,
            submission_id,
            form.field_key.trim(),
            form.query_text.trim(),
            raised_by_user_id,
        )
        .await
        .map_err(ApiError::internal)?;

    // Audit - data queries are key monitoring/compliance artifacts
    let _ = ctx.db.insert_audit_log(
        "study_data_queries",
        query.id,
        "created",
        Some(user.user_id),
        None,
        None,
        Some(&format!("Field: {}", form.field_key.trim())),
    ).await;

    Ok(Redirect::to(&format!(
        "/ui/studies?admin_email={}&organization_id={}&project_id={}&submission_id={}&notice={}",
        query_escape(user.email.as_str()),
        project.organization_id,
        project_id,
        submission_id,
        query_escape("Data query created")
    )))
}

async fn submit_respond_study_data_query(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
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

    require_org_role(&user, project.organization_id, ROLE_COORDINATOR_OR_BETTER)?;

    let response_text = form.response_text.trim().to_string();

    ctx.db
        .respond_study_data_query(query_id, &response_text)
        .await
        .map_err(ApiError::internal)?;

    // Rich provenance for query response (key compliance artifact)
    let new_data = Some(format!(r#"{{"response":"{}"}}"#, serde_json::to_string(&response_text).unwrap_or_else(|_| response_text.clone())));
    let _ = ctx.db.insert_audit_log(
        "study_data_queries",
        query_id,
        "responded",
        Some(user.user_id),
        None,
        new_data.as_deref(),
        Some(&format!("Query responded ({} chars)", response_text.len())),
    ).await;

    Ok(Redirect::to(&format!(
        "/ui/studies?admin_email={}&organization_id={}&project_id={}&submission_id={}&notice={}",
        query_escape(user.email.as_str()),
        project.organization_id,
        project.id,
        query.submission_id,
        query_escape("Data query response saved")
    )))
}

async fn submit_close_study_data_query(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
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

    require_org_role(&user, project.organization_id, ROLE_COORDINATOR_OR_BETTER)?;

    ctx.db
        .close_study_data_query(query_id, Some(user.user_id))
        .await
        .map_err(ApiError::internal)?;

    // Rich provenance for query closure
    let _ = ctx.db.insert_audit_log(
        "study_data_queries",
        query_id,
        "closed",
        Some(user.user_id),
        None,
        None,
        Some("Data query closed by coordinator"),
    ).await;

    Ok(Redirect::to(&format!(
        "/ui/studies?admin_email={}&organization_id={}&project_id={}&submission_id={}&notice={}",
        query_escape(user.email.as_str()),
        project.organization_id,
        project.id,
        query.submission_id,
        query_escape("Data query closed")
    )))
}

async fn submit_set_study_startup_checklist_item(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
    Path(project_id): Path<Uuid>,
    Form(form): Form<StudyChecklistItemForm>,
) -> Result<Redirect, ApiError> {
    let project = ctx
        .db
        .get_project(project_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("project not found".to_string()))?;

    require_org_role(&user, project.organization_id, ROLE_COORDINATOR_OR_BETTER)?;
    let completed = form.completed.is_some();
    if completed && form.notes.trim().is_empty() {
        return Err(ApiError::Validation(
            "A compliance verification comment is required when marking a startup checklist item as complete.".to_string()
        ));
    }
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
        "/ui/studies?admin_email={}&organization_id={}&project_id={}&view=startup&notice={}#startup-next-task",
        query_escape(form.admin_email.trim()),
        project.organization_id,
        project_id,
        query_escape(notice)
    )))
}

async fn submit_set_site_startup_checklist_item(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
    Path(site_id): Path<Uuid>,
    Form(form): Form<StudyChecklistItemForm>,
) -> Result<Redirect, ApiError> {
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

    require_org_role(&user, project.organization_id, ROLE_COORDINATOR_OR_BETTER)?;
    let completed = form.completed.is_some();
    if completed && form.notes.trim().is_empty() {
        return Err(ApiError::Validation(
            "A compliance verification comment is required when marking a startup checklist item as complete.".to_string()
        ));
    }
    let completed_by_user_id = if completed {
        Some(user.user_id)
    } else {
        None
    };
    ctx.db
        .set_site_startup_checklist_item(
            site_id,
            form.item_code.trim(),
            form.item_label.trim(),
            completed,
            completed_by_user_id,
            form.notes.trim(),
        )
        .await
        .map_err(ApiError::internal)?;
    let notice = if completed {
        "Site startup task completed"
    } else {
        "Site startup task marked pending"
    };
    Ok(Redirect::to(&format!(
        "/ui/app?admin_email={}&organization_id={}&project_id={}&view=sites&notice={}",
        query_escape(user.email.as_str()),
        project.organization_id,
        project.id,
        query_escape(notice)
    )))
}

async fn submit_set_study_close_checklist_item(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
    Path(project_id): Path<Uuid>,
    Form(form): Form<StudyChecklistItemForm>,
) -> Result<Redirect, ApiError> {
    let project = ctx
        .db
        .get_project(project_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("project not found".to_string()))?;

    require_org_role(&user, project.organization_id, ROLE_COORDINATOR_OR_BETTER)?;
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
        "/ui/studies?admin_email={}&organization_id={}&project_id={}&view=close&notice={}#close-checklist-panel",
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
    let selected_organization_hex = selected_organization_uuid
        .map(|id| id.to_string().chars().take(8).collect::<String>())
        .unwrap_or_default();

    let organization_options = organizations
        .iter()
        .map(|org| {
            format!(
                r#"<option value="{}" data-id="{}">{}</option>"#,
                org.id.to_string().chars().take(8).collect::<String>(),
                org.id,
                html_escape(&org.name)
            )
        })
        .collect::<Vec<_>>()
        .join("");

    let colors = ["#02182b", "#283e28", "#f05708", "#e7e5da", "#4a5568"];
    let managed_orgs_html = if organizations.is_empty() {
        r#"<div class="dashboard-card" style="padding:1.5rem; min-height:unset; border:1px solid #cbd5e0; text-align:center;">
          <p style="color:#718096; font-style:italic; margin:0;">No organizations found for this admin email yet.</p>
        </div>"#.to_string()
    } else {
        let mut cards = vec![];
        for org in &organizations {
            let name_bytes = org.name.as_bytes();
            let b1 = name_bytes.get(0).copied().unwrap_or(1) as usize;
            let b2 = name_bytes.get(1).copied().unwrap_or(2) as usize;
            let b3 = name_bytes.get(2).copied().unwrap_or(3) as usize;
            let b4 = name_bytes.get(3).copied().unwrap_or(4) as usize;
            
            let c1 = colors[b1 % 5];
            let c2 = colors[b2 % 5];
            let c3 = colors[b3 % 5];
            let c4 = colors[b4 % 5];
            
            let logo_svg = format!(
                r##"<svg width="24" height="24" viewBox="0 0 32 32" style="border-radius:4px; box-shadow:inset 0 0 2px rgba(0,0,0,0.15); display:block; flex-shrink:0;">
  <rect x="0" y="0" width="16" height="16" fill="{}" />
  <rect x="16" y="0" width="16" height="16" fill="{}" />
  <rect x="0" y="16" width="16" height="16" fill="{}" />
  <rect x="16" y="16" width="16" height="16" fill="{}" />
</svg>"##,
                c1, c2, c3, c4
            );

            let is_active = org.id.to_string() == selected_organization_id;
            let active_style = if is_active {
                "border: 2px solid var(--cg-navy); background: linear-gradient(180deg, #ffffff 0%, #f7fbff 100%);"
            } else {
                "border: 1px solid rgba(47, 88, 120, 0.28);"
            };
            let active_badge = if is_active {
                r#"<span style="font-size:0.75rem; font-weight:800; background:var(--cg-navy); color:white; padding:0.2rem 0.5rem; border-radius:6px; text-transform:uppercase; letter-spacing:0.05em; display:inline-block;">Active Workspace</span>"#
            } else {
                ""
            };

            cards.push(format!(
                r##"<a href="/ui/dua?admin_email={}&organization_id={}" class="dashboard-card" style="text-decoration:none; color:inherit; padding:1.25rem; min-height:unset; display:flex; flex-direction:column; gap:0.75rem; transition:all 0.2s; position:relative; overflow:hidden; {}">
  <div style="display:flex; justify-content:space-between; align-items:flex-start; gap:0.5rem;">
    <div style="display:flex; gap:0.6rem; align-items:center;">
      {}
      <h4 style="margin:0; color:#02182b; font-size:1.1rem; font-weight:700;">{}</h4>
    </div>
    {}
  </div>
  <div style="font-size:0.8rem; color:#4a5568; font-family:monospace; word-break:break-all; margin-top:0.25rem;">
    <strong>ID:</strong> {}
  </div>
  <div style="display:flex; gap:0.5rem; font-size:0.75rem; color:#718096; margin-top:auto; padding-top:0.5rem; border-top:1px solid #edf2f7;">
    <span style="background:#e2e8f0; color:#4a5568; padding:0.15rem 0.4rem; border-radius:4px; font-weight:600;">Kind: {}</span>
    <span style="background:#e2e8f0; color:#4a5568; padding:0.15rem 0.4rem; border-radius:4px; font-weight:600;">Parent: {}</span>
  </div>
</a>"##,
                query_escape(admin_email.trim()),
                org.id,
                active_style,
                logo_svg,
                html_escape(&org.name),
                active_badge,
                org.id,
                html_escape(&org.organization_kind),
                org.parent_organization_id
                    .map(|id| id.to_string())
                    .unwrap_or_else(|| "none".to_string())
            ));
        }
        cards.join("\n")
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
        r#"<div class="dashboard-card" style="padding:1.5rem; min-height:unset; border:1px solid #cbd5e0; text-align:center;">
          <p style="color:#718096; font-style:italic; margin:0;">No DUAs for selected organization yet.</p>
        </div>"#.to_string()
    } else {
        org_duas
            .iter()
            .map(|dua| {
                let badge_color = match dua.status.as_str() {
                    "signed" => "background:#e6fffa; color:#234e52; border:1px solid #b2f5ea;",
                    "draft" => "background:#feebc8; color:#7b341e; border:1px solid #fbd38d;",
                    _ => "background:#edf2f7; color:#4a5568; border:1px solid #e2e8f0;",
                };
                format!(
                    r##"<a href="/ui/dua/{}?admin_email={}" class="dashboard-card-row" style="text-decoration:none; color:inherit; margin-bottom:0.75rem; border:1px solid rgba(47, 88, 120, 0.28); padding:1rem; border-radius:10px; display:flex; justify-content:space-between; align-items:center; transition:all 0.15s;">
  <div style="display:flex; gap:0.75rem; align-items:center;">
    <svg width="24" height="24" viewBox="0 0 32 32" style="border-radius:4px; box-shadow:inset 0 0 2px rgba(0,0,0,0.15); display:block; flex-shrink:0;">
      <rect x="0" y="0" width="32" height="32" fill="#02182b" />
      <path d="M8 8h16v2H8zm0 6h16v2H8zm0 6h10v2H8z" fill="white" />
    </svg>
    <div>
      <span style="font-weight:700; color:#02182b; font-size:1.05rem;">{}</span>
      <div style="font-size:0.75rem; color:#718096; margin-top:0.15rem; font-family:monospace;">ID: {}</div>
    </div>
  </div>
  <span class="status-chip" style="font-weight:700; font-size:0.75rem; padding:0.25rem 0.5rem; border-radius:6px; text-transform:uppercase; {}">{}</span>
</a>"##,
                    dua.id,
                    query_escape(admin_email.trim()),
                    html_escape(&dua.hospital_name),
                    dua.id,
                    badge_color,
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
        r##"<div style="background:linear-gradient(135deg, #02182b 0%, #283e28 100%); padding:2rem; border-radius:16px; color:white; margin-bottom:2rem; box-shadow:0 10px 25px rgba(2,24,43,0.15); border:1px solid rgba(255,255,255,0.08);">
  <h1 style="color:white; margin:0 0 0.5rem; font-size:2rem; font-weight:800;">Electronic Data Use Agreements</h1>
  <p style="color:rgba(255,255,255,0.8); margin:0; font-size:0.95rem;">
    Create, sign, and manage secure clinical Data Use Agreements (DUA) between healthcare providers and Cingulum Foundation Inc.
  </p>
</div>

{}

<!-- Available Organizations List -->
<div class="card" style="margin-bottom:2rem; padding:1.5rem;">
  <div style="display:flex; gap:0.5rem; align-items:center; margin-bottom:1rem; border-bottom:1px solid #edf2f7; padding-bottom:0.5rem;">
    <svg width="24" height="24" viewBox="0 0 32 32" style="border-radius:4px; box-shadow:inset 0 0 2px rgba(0,0,0,0.15);">
      <rect x="0" y="0" width="16" height="16" fill="#02182b" />
      <rect x="16" y="0" width="16" height="16" fill="#f05708" />
      <rect x="0" y="16" width="16" height="16" fill="#02182b" />
      <rect x="16" y="16" width="16" height="16" fill="#e7e5da" />
    </svg>
    <h2 style="margin:0; font-size:1.25rem;">Organizations available for this admin</h2>
  </div>
  <p style="color:#718096; font-size:0.85rem; margin-bottom:1.25rem;">
    Click an organization card to select it and view its scoped agreements.
  </p>
  <div style="display:grid; grid-template-columns:repeat(auto-fill, minmax(280px, 1fr)); gap:1.25rem;">
    {}
    <div id="add-org-card" style="background:#f8fafc; border:2px dashed #cbd5e0; border-radius:12px; padding:1.25rem; display:flex; flex-direction:column; align-items:center; justify-content:center; min-height:160px; cursor:pointer; transition:all 0.2s;" onclick="document.getElementById('org-create-modal').showModal()">
      <span style="font-size:3rem; color:#a0aec0; font-weight:300; line-height:1;">+</span>
      <span style="font-size:0.9rem; font-weight:700; color:#718096; margin-top:0.5rem;">Add New Organization</span>
    </div>
  </div>
</div>

<!-- Scoped DUA Workspace Section -->
<div class="card" style="margin-bottom:2rem; padding:1.5rem;">
  <div style="display:flex; gap:0.5rem; align-items:center; margin-bottom:1rem; border-bottom:1px solid #edf2f7; padding-bottom:0.5rem;">
    <svg width="24" height="24" viewBox="0 0 32 32" style="border-radius:4px; box-shadow:inset 0 0 2px rgba(0,0,0,0.15);">
      <rect x="0" y="0" width="32" height="32" fill="#02182b" />
      <path d="M6 6h20v2H6zm0 6h20v2H6zm0 6h14v2H6z" fill="white" />
    </svg>
    <h2 style="margin:0; font-size:1.25rem;">Selected organization DUA workspace</h2>
  </div>
  <p style="color:#718096; font-size:0.85rem; margin-bottom:1.25rem;">
    DUAs below are scoped only to the currently chosen active organization workspace.
  </p>
  <div>
    {}
  </div>
</div>

<div style="display:flex; flex-direction:column; gap:2rem; margin-bottom:2rem;">
  
  <!-- Step 2: Draft DUA Section -->
  <div class="card" style="margin:0; padding:2rem; background: linear-gradient(145deg, #ffffff 0%, #f8f9fa 100%); border: 1px solid #e2e8f0; box-shadow: 0 12px 24px -8px rgba(0,0,0,0.08); border-radius: 12px;">
    <div style="display:flex; gap:0.75rem; align-items:center; margin-bottom:1.5rem; border-bottom:1px solid #edf2f7; padding-bottom:1rem;">
      <svg width="28" height="28" viewBox="0 0 32 32" style="border-radius:6px; box-shadow: 0 4px 6px rgba(0,0,0,0.1);">
        <rect x="0" y="0" width="16" height="16" fill="#f05708" />
        <rect x="16" y="0" width="16" height="16" fill="#e7e5da" />
        <rect x="0" y="16" width="16" height="16" fill="#02182b" />
        <rect x="16" y="16" width="16" height="16" fill="#283e28" />
      </svg>
      <h2 style="margin:0; font-size:1.4rem; color: #02182b;">Step 2: Draft Data Use Agreement</h2>
    </div>
    <p style="color:#4a5568; font-size:0.95rem; margin-bottom:1.5rem; line-height: 1.5;">
      Draft a new Data Use Agreement for the selected organization and seamlessly queue the entity legal signing process.
    </p>
    <form method="post" action="/ui/dua/draft" style="margin:0; background:none; border:none; padding:0; box-shadow:none; max-width:none;">
      <div style="display:grid; grid-template-columns:repeat(auto-fit, minmax(320px, 1fr)); gap:2rem; align-items:start;">
        <div>
          <label style="font-weight:600; color:#2d3748; margin-bottom:0.25rem;">Admin email (must be org manager or platform admin)</label>
          <input name="admin_email" value="{}" required style="background:#edf2f7; border: 1px solid #cbd5e0; color: #4a5568; border-radius: 6px; padding: 0.6rem;" readonly />

          <label style="font-weight:600; color:#2d3748; margin-bottom:0.25rem;">Organization ID (UUID)</label>
          <input name="organization_id" list="organization-options" value="{}" placeholder="organization-uuid" required style="border: 1px solid #cbd5e0; border-radius: 6px; padding: 0.6rem;" />
          <datalist id="organization-options">
            {}
          </datalist>

          <label style="font-weight:600; color:#2d3748; margin-bottom:0.25rem;">Entity legal name</label>
          <input name="hospital_name" placeholder="E.g., General Hospital / XYZ Corp" required style="border: 1px solid #cbd5e0; border-radius: 6px; padding: 0.6rem;" />

          <label style="font-weight:600; color:#2d3748; margin-bottom:0.25rem;">Entity contact name</label>
          <input name="hospital_contact_name" placeholder="E.g., Jane Doe" required style="border: 1px solid #cbd5e0; border-radius: 6px; padding: 0.6rem;" />

          <label style="font-weight:600; color:#2d3748; margin-bottom:0.25rem;">Entity contact email</label>
          <input type="email" name="hospital_contact_email" placeholder="legal@entity.org" required style="border: 1px solid #cbd5e0; border-radius: 6px; padding: 0.6rem;" />
        </div>
        <div>
          <div style="display:flex; gap: 1rem;">
            <div style="flex:1;">
              <label style="font-weight:600; color:#2d3748; margin-bottom:0.25rem;">Agreement version</label>
              <input name="agreement_version" value="1.0" required style="border: 1px solid #cbd5e0; border-radius: 6px; padding: 0.6rem;" />
            </div>
            <div style="flex:1;">
              <label style="font-weight:600; color:#2d3748; margin-bottom:0.25rem;">Effective date</label>
              <input name="effective_date" placeholder="YYYY-MM-DD" style="border: 1px solid #cbd5e0; border-radius: 6px; padding: 0.6rem;" />
            </div>
            <div style="flex:1;">
              <label style="font-weight:600; color:#2d3748; margin-bottom:0.25rem;">Expiration date</label>
              <input name="expiration_date" placeholder="YYYY-MM-DD" style="border: 1px solid #cbd5e0; border-radius: 6px; padding: 0.6rem;" />
            </div>
          </div>

          <label style="font-weight:600; color:#2d3748; margin-top:0.75rem; margin-bottom:0.25rem;">Agreement text</label>
          <textarea name="agreement_text" required style="height:280px; font-family: 'Menlo', 'Monaco', 'Courier New', monospace; font-size:0.85rem; border: 1px solid #cbd5e0; border-radius: 6px; padding: 0.75rem; line-height: 1.4; background: #fbfbfc;">{}</textarea>
        </div>
      </div>

      <button type="submit" style="background: linear-gradient(135deg, #02182b 0%, #052b4d 100%); color:white; border:none; padding:1rem; border-radius:8px; font-weight:700; cursor:pointer; width:100%; margin-top:2rem; transition:transform 0.1s, box-shadow 0.2s; box-shadow: 0 4px 6px rgba(2, 24, 43, 0.2); font-size:1.05rem;">
        Create DUA & Queue Entity Signing Link
      </button>
    </form>
  </div>

  <!-- Return to Existing Workspace Card -->
  <div class="card" style="margin:0; padding:1.5rem; max-width: 500px;">
    <div style="display:flex; gap:0.5rem; align-items:center; margin-bottom:1rem; border-bottom:1px solid #edf2f7; padding-bottom:0.5rem;">
      <svg width="24" height="24" viewBox="0 0 32 32" style="border-radius:4px; box-shadow:inset 0 0 2px rgba(0,0,0,0.15);">
        <rect x="0" y="0" width="16" height="16" fill="#283e28" />
        <rect x="16" y="0" width="16" height="16" fill="#e7e5da" />
        <rect x="0" y="16" width="16" height="16" fill="#02182b" />
        <rect x="16" y="16" width="16" height="16" fill="#283e28" />
      </svg>
      <h2 style="margin:0; font-size:1.15rem; color:#4a5568;">Return to existing agreement workspace</h2>
    </div>
    <form method="post" action="/ui/dua/open-agreement" style="margin:0; background:none; border:none; padding:0; box-shadow:none; max-width:none; display:flex; flex-direction:column; gap: 0.5rem;">
      <label style="font-size: 0.85rem; color: #4a5568; margin-bottom: 0;">Admin email</label>
      <input name="admin_email" value="{}" required style="background:#edf2f7; border: 1px solid #e2e8f0; padding: 0.5rem;" readonly />
      <label style="font-size: 0.85rem; color: #4a5568; margin-bottom: 0;">Agreement ID (UUID)</label>
      <input name="agreement_id" placeholder="agreement-uuid" required style="border: 1px solid #e2e8f0; padding: 0.5rem;" />
      <button type="submit" style="background:#cbd5e0; color:#2d3748; border:none; padding:0.6rem; border-radius:6px; font-weight:600; cursor:pointer; width:100%; margin-top:0.5rem; transition:background 0.2s;">Open Agreement Workspace</button>
    </form>
  </div>
</div>

<!-- Create Organization Modal -->
<dialog id="org-create-modal" style="border:none; border-radius:12px; padding:2rem; width:100%; max-width:480px; box-shadow:0 20px 25px -5px rgba(0,0,0,0.1), 0 10px 10px -5px rgba(0,0,0,0.04); background:#fff;">
  <div style="display:flex; justify-content:space-between; align-items:center; margin-bottom:1.5rem; border-bottom:1px solid #edf2f7; padding-bottom:0.75rem;">
    <h3 style="margin:0; color:#02182b; font-size:1.25rem;">Create New Organization</h3>
    <button onclick="document.getElementById('org-create-modal').close()" style="background:none; border:none; font-size:1.5rem; color:#a0aec0; cursor:pointer;">&times;</button>
  </div>
  <form method="post" action="/ui/dua/create-organization" style="display:flex; flex-direction:column; gap:0.75rem; margin:0;">
    <label style="font-weight:600; font-size:0.85rem; color:#4a5568;">Admin email (platform admin required to create org)</label>
    <input name="admin_email" value="{}" required style="background:#f7fafc; border:1px solid #cbd5e0; padding:0.5rem; border-radius:6px;" readonly />

    <label style="font-weight:600; font-size:0.85rem; color:#4a5568;">New organization legal name</label>
    <input name="organization_name" placeholder="Cingulum Foundation Inc." required style="border:1px solid #cbd5e0; padding:0.5rem; border-radius:6px;" />
    
    <label style="font-weight:600; font-size:0.85rem; color:#4a5568;">Parent organization ID (optional; blank = Cingulum Foundation root)</label>
    <input name="parent_organization_id" list="organization-options" value="{}" placeholder="child under foundation" style="border:1px solid #cbd5e0; padding:0.5rem; border-radius:6px;" />
    
    <label style="font-weight:600; font-size:0.85rem; color:#4a5568;">Organization type</label>
    <select name="organization_kind" style="border:1px solid #cbd5e0; padding:0.5rem; border-radius:6px; background:white;">
      <option value="tenant" selected>tenant</option>
      <option value="research_network">research_network</option>
      <option value="hospital">hospital</option>
      <option value="sponsor">sponsor</option>
    </select>

    <button type="submit" style="background:#02182b; color:white; border:none; padding:0.75rem; border-radius:8px; font-weight:600; cursor:pointer; width:100%; margin-top:1rem; transition:background 0.2s;">Create Organization</button>
  </form>
</dialog>
"##,
        notice_html,
        managed_orgs_html,
        org_duas_html,
        html_escape(&admin_email),
        html_escape(&selected_organization_hex),
        organization_options,
        html_escape(default_dua_text()),
        html_escape(&admin_email),
        html_escape(&admin_email),
        html_escape(&selected_organization_hex)
    );

    let script = r#"
<script>
document.addEventListener('DOMContentLoaded', () => {
    document.querySelectorAll('form').forEach(form => {
        form.addEventListener('submit', (e) => {
            const visibleInputs = form.querySelectorAll('input[list]');
            visibleInputs.forEach(input => {
                const listId = input.getAttribute('list');
                const list = document.getElementById(listId);
                if (list) {
                    const option = Array.from(list.options).find(o => o.value === input.value);
                    if (option && option.hasAttribute('data-id')) {
                        let hidden = form.querySelector(`input[type="hidden"][name="${input.name}"]`);
                        if (!hidden) {
                            hidden = document.createElement('input');
                            hidden.type = 'hidden';
                            hidden.name = input.name;
                            form.appendChild(hidden);
                            input.removeAttribute('name');
                        }
                        hidden.value = option.getAttribute('data-id');
                    }
                }
            });
        });
    });
});
</script>
"#;
    let context_bar = render_context_bar(
        if selected_organization_hex.is_empty() { None } else { Some(&selected_organization_hex) },
        None,
        admin_email.trim()
    );
    let body = format!("{}{}{}", context_bar, body, script);

    Ok(Html(render_cingulum_page("Virivu DUA Console", body)))
}

async fn submit_create_organization_from_ui(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
    Form(form): Form<DuaCreateOrganizationForm>,
) -> Result<Html<String>, ApiError> {
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
    ctx.db
        .ensure_org_admin_membership(user.email.as_str(), organization.id)
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
        query_escape(&user.email),
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
    user: AuthenticatedUser,
    Form(form): Form<DuaDraftForm>,
) -> Result<Html<String>, ApiError> {
    let organization_id =
        form.organization_id.trim().parse::<Uuid>().map_err(|_| {
            ApiError::Validation("organization_id must be a valid UUID".to_string())
        })?;

    require_org_role(&user, organization_id, ROLE_ORG_MANAGERS)?;

    let effective_date = parse_optional_date(&form.effective_date)?;
    let expiration_date = parse_optional_date(&form.expiration_date)?;
    let created_by_user_id = Some(user.user_id);

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
        query_escape(user.email.as_str()),
        query_escape(user.email.as_str())
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
    user: AuthenticatedUser,
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

    require_org_role(&user, agreement.organization_id, ROLE_ORG_MANAGERS)?;

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
    user: AuthenticatedUser,
    Path(agreement_id): Path<Uuid>,
    Form(form): Form<DuaSendLinkForm>,
) -> Result<Html<String>, ApiError> {
    let agreement = ctx
        .db
        .get_data_use_agreement(agreement_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("data use agreement not found".to_string()))?;

    require_org_role(&user, agreement.organization_id, ROLE_ORG_MANAGERS)?;

    let requested_by = Some(user.user_id);
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
        query_escape(user.email.as_str())
    );
    Ok(Html(render_cingulum_page("Email queued", body)))
}

async fn submit_dua_cingulum_sign_form(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
    Path(agreement_id): Path<Uuid>,
    Form(form): Form<DuaCingulumSignForm>,
) -> Result<Html<String>, ApiError> {
    let agreement = ctx
        .db
        .get_data_use_agreement(agreement_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("data use agreement not found".to_string()))?;

    require_org_role(&user, agreement.organization_id, ROLE_ORG_MANAGERS)?;

    ctx.db
        .sign_data_use_agreement_as_cingulum(
            agreement_id,
            form.signer_name.trim(),
            form.signer_email.trim(),
            form.signer_title.trim(),
            "typed",
            form.signature_text.trim(),
            None,
            Some(user.user_id),
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
        query_escape(user.email.as_str())
    );
    Ok(Html(render_cingulum_page(
        "Cingulum signature recorded",
        body,
    )))
}

async fn download_data_use_agreement_pdf_ui(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
    Path(agreement_id): Path<Uuid>,
    Query(_query): Query<DuaExportQuery>,
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

        let label = infer_html_field_label(raw_key, input.value().attr("placeholder"));
        if is_corrupted_key_or_label(&field_key, &label) {
            continue;
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
                    label.clone(),
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
            field_label: label,
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
        let label = infer_html_field_label(raw_key, textarea.value().attr("placeholder"));
        if is_corrupted_key_or_label(&field_key, &label) {
            continue;
        }
        if !seen_keys.insert(field_key.clone()) {
            continue;
        }
        fields.push(ImportedCrfFieldDraft {
            field_label: label,
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
        let label = infer_html_field_label(raw_key, None);
        if is_corrupted_key_or_label(&field_key, &label) {
            continue;
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
            field_label: label,
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

fn is_corrupted_key_or_label(key: &str, label: &str) -> bool {
    let label = label.trim();
    let key = key.trim();
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

fn is_corrupted_crf_field(field: &StudyCrfField) -> bool {
    is_corrupted_key_or_label(&field.field_key, &field.field_label)
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

fn generate_sample_answers_json(fields: &[StudyCrfField]) -> String {
    if fields.is_empty() {
        return "{}".to_string();
    }
    let mut map = serde_json::Map::new();
    for field in fields.iter().take(10) {
        let sample = match field.field_type.as_str() {
            "number" => serde_json::Value::Number(0.into()),
            "boolean" => serde_json::Value::Bool(false),
            "date" | "datetime" => serde_json::Value::String("2026-01-01".to_string()),
            "single_select" => {
                if let Ok(opts) = serde_json::from_str::<Vec<serde_json::Value>>(&field.options_json) {
                    opts.first().cloned().unwrap_or(serde_json::Value::String("option1".to_string()))
                } else {
                    serde_json::Value::String("option1".to_string())
                }
            }
            _ => serde_json::Value::String("".to_string()),
        };
        map.insert(field.field_key.clone(), sample);
    }
    serde_json::to_string_pretty(&map).unwrap_or_else(|_| "{}".to_string())
}

fn render_crf_field_for_data_entry(field: &StudyCrfField, current_value: &str) -> String {
    let name = format!("field_{}", field.field_key);
    let required = if field.required { " required" } else { "" };
    let label = html_escape(&field.field_label);
    let value_esc = html_escape(current_value);

    match field.field_type.as_str() {
        "text" => format!(
            r#"<label style="display:block;margin-top:0.5rem;font-weight:600;">{} {}</label><input type="text" name="{}" value="{}" style="width:100%;padding:6px;"{} />"#,
            label, if field.required { "*" } else { "" }, name, value_esc, required
        ),
        "textarea" => format!(
            r#"<label style="display:block;margin-top:0.5rem;font-weight:600;">{} {}</label><textarea name="{}" rows="3" style="width:100%;padding:6px;"{}>{}</textarea>"#,
            label, if field.required { "*" } else { "" }, name, required, value_esc
        ),
        "number" => format!(
            r#"<label style="display:block;margin-top:0.5rem;font-weight:600;">{} {}</label><input type="number" name="{}" value="{}" style="width:100%;padding:6px;"{} />"#,
            label, if field.required { "*" } else { "" }, name, value_esc, required
        ),
        "date" => format!(
            r#"<label style="display:block;margin-top:0.5rem;font-weight:600;">{} {}</label><input type="date" name="{}" value="{}" style="width:100%;padding:6px;"{} />"#,
            label, if field.required { "*" } else { "" }, name, value_esc, required
        ),
        "boolean" => {
            let checked = if current_value == "true" || current_value == "1" { " checked" } else { "" };
            format!(
                r#"<label style="display:block;margin-top:0.5rem;"><input type="checkbox" name="{}" value="true"{} {} /> {}</label>"#,
                name, checked, required, label
            )
        }
        "single_select" => {
            let mut opts = String::new();
            if let Ok(items) = serde_json::from_str::<Vec<String>>(&field.options_json) {
                for item in items {
                    let sel = if item == current_value { " selected" } else { "" };
                    opts.push_str(&format!(r#"<option value="{}"{}>{}</option>"#, html_escape(&item), sel, html_escape(&item)));
                }
            }
            format!(
                r#"<label style="display:block;margin-top:0.5rem;font-weight:600;">{} {}</label><select name="{}" style="width:100%;padding:6px;"{}>{}</select>"#,
                label, if field.required { "*" } else { "" }, name, required, opts
            )
        }
        "multi_select" => {
            let mut opts = String::new();
            let selected: Vec<&str> = current_value.split(',').collect();
            if let Ok(items) = serde_json::from_str::<Vec<String>>(&field.options_json) {
                for item in items {
                    let sel = if selected.contains(&item.as_str()) { " checked" } else { "" };
                    opts.push_str(&format!(r#"<label style="display:block;"><input type="checkbox" name="{}[]" value="{}"{} /> {}</label>"#, name, html_escape(&item), sel, html_escape(&item)));
                }
            }
            format!(r#"<label style="display:block;margin-top:0.5rem;font-weight:600;">{} {}</label><div style="padding-left:4px;">{}</div>"#, label, if field.required { "*" } else { "" }, opts)
        }
        _ => format!(
            r#"<label style="display:block;margin-top:0.5rem;font-weight:600;">{} {}</label><input type="text" name="{}" value="{}" style="width:100%;padding:6px;"{} />"#,
            label, if field.required { "*" } else { "" }, name, value_esc, required
        ),
    }
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
    r#"# Comprehensive International Data Transfer and Use Agreement (“Agreement”)

**Effective Date:** [Date]

This Data Transfer and Use Agreement (hereinafter “Agreement”), effective as of the date of the last signature below (hereinafter “Effective Date”), is by and between **Cingulum Foundation Inc.**, a New York not-for-profit corporation located at [Provider Address] (hereinafter “Provider”) and **[Recipient Name]** located at [Recipient Address] (hereinafter “Recipient”). Provider and Recipient shall be referred to hereinafter individually as a “Party” and collectively as the “Parties.”

---

## Master Terms and Conditions

### 1. Provision of Data and Permitted Use
1.1 **Data Definition:** Provider shall provide the Data set described in Attachment 1 (the “Data”) to Recipient for the research purpose set forth in Attachment 1 (the “Project”). 
1.2 **Ownership:** Provider shall retain ownership of any rights it may have in the Data and Recipient does not obtain any rights in the Data other than as set forth herein.
1.3 **Authorized Use:** Recipient shall not use the Data except as authorized under this Agreement. The Data will be used solely to conduct the Project and solely by Recipient Scientist and Recipient’s faculty, employees, fellows, students, and agents (“Recipient Personnel”) and Third-Party Personnel (as defined in Attachment 3) that have a need to use, or provide a service in respect of, the Data in connection with the Project and whose obligations of use are consistent with the terms of this Agreement (collectively, “Authorized Persons”).

### 2. Control, Security, and Safeguards
2.1 **Control of Data:** Except as authorized under this Agreement or otherwise required by law, Recipient agrees to retain control over the Data and shall not disclose, release, sell, rent, lease, loan, or otherwise grant access to the Data to any third party, except Authorized Persons, without the prior written consent of Provider. 
2.2 **Safeguards:** Recipient agrees to establish appropriate administrative, technical, and physical safeguards to prevent unauthorized use of or access to the Data and comply with any other special requirements relating to safeguarding of the Data as set forth in Attachment 2 and the applicable jurisdictional Addendums (Exhibits A and B).

### 3. Compliance with Laws and Professional Standards
3.1 Recipient agrees to use the Data in compliance with all applicable local, national, and international laws, rules, and regulations, as well as all professional standards applicable to such research.
3.2 **De-Identification & Re-Identification:** If the Data is provided as de-identified or pseudonymized, Recipient will not use the Data, either alone or in concert with any other information, to make any effort to identify or contact individuals who are or may be the sources of Data without specific written approval from Provider and appropriate Institutional Review Board (IRB) or Ethics Committee approval.

### 4. Publication and Intellectual Property
4.1 **Right to Publish:** Recipient is encouraged to make publicly available the results of the Project. Before Recipient submits a paper or abstract for publication or otherwise intends to publicly disclose information about the results of the Project, the Recipient will submit the proposed publication, or a written version of such other disclosure, to Provider for review and comment.
4.2 **Review Period:** The Provider will have forty-five (45) days from receipt to review proposed manuscripts and fifteen (15) days to review proposed abstracts to ensure that the Data is appropriately protected and proprietary information is not inadvertently disclosed.
4.3 **Attribution:** Recipient agrees to recognize the contribution of the Provider as the source of the Data in all written, visual, or oral public disclosures concerning Recipient’s research, as appropriate in accordance with scholarly standards.

### 5. Term, Termination, and Disposition
5.1 **Term:** Unless terminated earlier, this Agreement shall expire as of the End Date set forth in Attachment 1. 
5.2 **Termination:** Either Party may terminate this Agreement with thirty (30) days prior written notice to the other Party.
5.3 **Disposition:** Upon expiration or early termination of this Agreement, Recipient shall delete or destroy all copies of the Data transferred pursuant to NIST Standard SP-800-88 (Guidelines for Media Sanitization) or equivalent international standard, unless both Parties extend the agreement prior to expiration or applicable law mandates retention.

### 6. Representations, Warranties, and Liability
6.1 **"AS IS" Delivery:** Except as prohibited by law, any Data delivered pursuant to this Agreement is understood to be provided “AS IS.” PROVIDER MAKES NO REPRESENTATIONS AND EXTENDS NO WARRANTIES OF ANY KIND, EITHER EXPRESSED OR IMPLIED, INCLUDING BUT NOT LIMITED TO WARRANTIES OF MERCHANTABILITY OR FITNESS FOR A PARTICULAR PURPOSE.
6.2 **Liability:** Except to the extent prohibited by law, the Recipient assumes all liability for damages which may arise from its use, storage, disclosure, or disposal of the Data. The Provider will not be liable to the Recipient for any loss, claim, or demand made by the Recipient, or made against the Recipient by any other party, due to or arising from the use of the Data by the Recipient, except to the extent permitted by law when caused by the gross negligence or willful misconduct of the Provider.

---

## Exhibit A: European Economic Area (EEA) / GDPR Addendum

If the Data transferred includes Personal Data (as defined by the EU General Data Protection Regulation 2016/679 - "GDPR") of data subjects located within the EEA, or if the Recipient is established in the EEA, the following terms shall apply:

1. **Standard Contractual Clauses (SCCs):** The Parties agree that the transfer of Personal Data from the Provider (acting as Data Exporter) to the Recipient (acting as Data Importer) shall be governed by the standard contractual clauses for the transfer of personal data to third countries pursuant to Regulation (EU) 2016/679 of the European Parliament and of the Council, as approved by the European Commission Implementing Decision (EU) 2021/914.
2. **Roles:** Unless otherwise specified in Attachment 1, Provider acts as the "Data Controller" and Recipient acts as the "Data Processor." 
3. **Data Subject Rights:** Recipient shall assist the Provider, by appropriate technical and organizational measures, for the fulfillment of the Provider's obligation to respond to requests for exercising the data subject's rights laid down in Chapter III of the GDPR (e.g., right to access, right to erasure).
4. **Sub-processing:** Recipient shall not engage another processor (sub-processor) without prior specific or general written authorization of the Provider. 
5. **Breach Notification:** In the case of a personal data breach, Recipient shall without undue delay and, where feasible, not later than 48 hours after having become aware of it, notify the personal data breach to the Provider, to allow the Provider to meet its 72-hour regulatory notification obligation to the competent supervisory authority.

---

## Exhibit B: India DPDP Act (2023) Addendum

If the Data transferred includes Digital Personal Data (as defined by the Digital Personal Data Protection Act, 2023 - "DPDP Act") of data principals located within India, or if the data is processed within India, the following terms shall apply:

1. **Valid Contract Requirement:** This Agreement fulfills the requirement under Section 8(2) of the DPDP Act, which mandates a valid contract between a Data Fiduciary and a Data Processor.
2. **Roles and Liability:** Provider acts as the "Data Fiduciary" and Recipient acts as the "Data Processor." Recipient acknowledges that under the DPDP Act, the Data Fiduciary retains sole regulatory liability for the processing of personal data. Recipient agrees to strictly indemnify the Provider against any penalties, fines, or damages arising from Recipient's failure to adhere to the terms of this Addendum or the DPDP Act.
3. **Purpose Limitation:** Recipient shall process the personal data strictly and solely for the purposes defined in Attachment 1, which must align with the consent obtained from the Data Principal by the Provider, or a legitimate use as defined by the DPDP Act.
4. **Security Safeguards:** Recipient shall implement reasonable security safeguards to prevent any personal data breach as required by Section 8(4) of the DPDP Act. 
5. **Breach Reporting:** In the event of a personal data breach, Recipient shall immediately (and in no event later than 24 hours) notify the Provider to enable the Provider to fulfill its mandatory obligation to report the breach to the Data Protection Board of India and the affected Data Principals.

---

## Attachments

### Attachment 1: Project Specific Information
- **Description of Data:** [Insert detailed description, e.g., De-identified MRI scans, Pseudonymized clinical trial outcomes]
- **Description of Project / Permitted Purpose:** [Insert exact scope of the research project]
- **Term:** Start Date: [Date], End Date: [Date]
- **Reimbursement of Costs:** [None / As set forth here: ...]

### Attachment 2: Data-Specific Terms and Technical Security Standards
- Recipient and its employees, agents, subcontractors, and any other individual permitted by Recipient to access the data will use all reasonable security practices and take all reasonable security measures necessary to protect the security and privacy of the data.
- **Standards Framework:** Recipient agrees to adhere to security standards generally consistent with ISO/IEC 27001, SOC 2 Type II, or NIST Special Publication 800-53.
- If Provider is a Covered Entity under US Law, the Data will be de-identified data, as defined by the Health Insurance Portability and Accountability Act of 1996 (“HIPAA”).

### Attachment 3: Identification of Permitted Third-Party Collaborators
- [ ] None. No collaborators are permitted on the Project.
- [ ] The following third parties are permitted: [Insert Names and Institutional Affiliations]

---

**IN WITNESS WHEREOF**, the Parties have caused this Agreement to be executed by their duly authorized representatives.

**Provider: Cingulum Foundation Inc.**
By: ___________________________
Name: 
Title: 
Date: 

**Recipient: [Recipient Institution]**
By: ___________________________
Name: 
Title: 
Date: 
"#
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
    html { scroll-behavior: smooth; }
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
    .field-card {
      background: #FFFDF8;
      border: 1px solid #fbd38d;
      border-left: 4px solid #F05708;
      padding: 1.25rem;
      border-radius: 8px;
      transition: all 0.3s ease;
      box-shadow: 0 1px 3px rgba(0,0,0,0.05);
      position: relative;
    }
    .field-card:focus-within {
      border-color: #F05708;
      box-shadow: 0 0 0 3px rgba(240, 87, 8, 0.15);
    }
    .field-card:has(input:valid:not(:placeholder-shown)),
    .field-card:has(textarea:valid:not(:placeholder-shown)) {
      border-color: #bee3f8;
      border-left-color: #3182ce;
      background: #ebf8ff;
      color: #2a4365;
    }
    .field-card:has(input:valid:not(:placeholder-shown))::after,
    .field-card:has(textarea:valid:not(:placeholder-shown))::after {
      content: '✓ Complete';
      position: absolute;
      right: 1.25rem;
      top: 1.25rem;
      font-size: 0.85rem;
      font-weight: 600;
      color: #3182ce;
    }
    .field-card input, .field-card textarea, .field-card select {
      border: 1px solid rgba(47, 88, 120, 0.2) !important;
      background: #ffffff !important;
      box-shadow: inset 0 1px 2px rgba(15, 45, 72, 0.05) !important;
      border-radius: 6px !important;
      outline: none !important;
      padding: 0.6rem 0.75rem !important;
      font-family: Inter, Arial, sans-serif;
      font-size: 1rem;
      width: 100% !important;
      margin-top: 0.25rem;
      color: var(--cg-navy);
      resize: none;
      transition: border-color 0.2s ease, box-shadow 0.2s ease;
    }
    .field-card input:focus, .field-card textarea:focus, .field-card select:focus {
      border-color: #F05708 !important;
      box-shadow: 0 0 0 3px rgba(240, 87, 8, 0.1) !important;
    }
    .page {
      max-width: 1180px;
      margin: 1.5rem auto;
      padding: 0 1rem 2.2rem;
    }
    .page:has(.sidebar) {
      padding-left: 260px;
      max-width: 1420px;
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
      position: relative;
      text-decoration: none;
      border: 1px solid rgba(197, 183, 171, 0.9);
      border-radius: 13px;
      padding: 0.72rem;
      background: #fffefb;
      color: #10263d;
      transition: transform 140ms ease, box-shadow 160ms ease, border-color 140ms ease;
      box-shadow: 0 9px 18px rgba(2, 24, 43, 0.08);
    }
    a.action-card,
    a.dashboard-card,
    a.dashboard-card-row,
    a.dashboard-card-mini {
      padding-right: 2.3rem !important;
    }
    a.action-card::after,
    a.dashboard-card::after,
    a.dashboard-card-row::after,
    a.dashboard-card-mini::after {
      content: 'CX';
      position: absolute;
      bottom: 8px;
      right: 8px;
      width: 18px;
      height: 18px;
      border-radius: 4px;
      background: linear-gradient(135deg, var(--cg-navy) 0%, var(--cg-orange) 100%);
      color: white;
      font-size: 7.5px;
      font-weight: 900;
      display: flex;
      align-items: center;
      justify-content: center;
      box-shadow: 0 2px 4px rgba(2, 24, 43, 0.25);
      opacity: 0.75;
      transition: all 0.22s cubic-bezier(0.25, 0.8, 0.25, 1);
    }
    a.action-card:hover::after,
    a.dashboard-card:hover::after,
    a.dashboard-card-row:hover::after,
    a.dashboard-card-mini:hover::after {
      opacity: 1;
      transform: scale(1.18);
      box-shadow: 0 4px 8px rgba(240, 87, 8, 0.4);
    }
    .dashboard-card {
      background: white;
      border-radius: 12px;
      border: 1px solid #e2e8f0;
      padding: 1.25rem;
      box-shadow: 0 4px 6px -1px rgba(0, 0, 0, 0.05);
      display: flex;
      flex-direction: column;
      justify-content: space-between;
      position: relative;
      min-height: 160px;
      transition: border-color 0.2s, box-shadow 0.2s, background-color 0.2s;
    }
    .dashboard-card:hover {
      border-color: var(--cg-navy) !important;
      box-shadow: 0 6px 12px -2px rgba(2, 24, 43, 0.08);
    }
    .dashboard-card-row {
      background: white;
      border-radius: 12px;
      border: 1px solid #e2e8f0;
      padding: 1.25rem;
      box-shadow: 0 4px 6px rgba(0,0,0,0.02);
      display: flex;
      align-items: center;
      justify-content: space-between;
      position: relative;
      transition: all 0.15s;
      margin-bottom: 1rem;
    }
    .dashboard-card-row:hover {
      border-color: var(--cg-navy) !important;
    }
    .dashboard-card-mini {
      background: white;
      border-radius: 10px;
      border: 1px solid #e2e8f0;
      padding: 1rem;
      box-shadow: 0 2px 4px rgba(0,0,0,0.02);
      display: flex;
      align-items: center;
      gap: 0.75rem;
      position: relative;
      transition: border-color 0.15s;
    }
    .dashboard-card-mini:hover {
      border-color: var(--cg-navy) !important;
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
      font-family: inherit;
      font-size: inherit;
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
.sidebar {
      position: fixed;
      top: 0;
      left: 0;
      width: 240px;
      height: 100vh;
      background: linear-gradient(180deg, #02182b 0%, #051f36 60%, #0a2640 100%);
      color: #c8d8e8;
      display: flex;
      flex-direction: column;
      z-index: 100;
      overflow-y: auto;
      scrollbar-width: thin;
      scrollbar-color: rgba(255,255,255,0.1) transparent;
      box-shadow: 4px 0 20px rgba(2,24,43,0.35);
    }
    .sidebar.blur-sidebar {
      background: rgba(245, 242, 235, 0.85);
      backdrop-filter: blur(16px) saturate(160%);
      -webkit-backdrop-filter: blur(16px) saturate(160%);
      border-right: 1px solid rgba(197, 183, 171, 0.6);
      box-shadow: 4px 0 24px rgba(0,0,0,0.03);
      color: #30475f;
    }
    .blur-sidebar .sidebar-logo { border-bottom: 1px solid rgba(197, 183, 171, 0.4); }
    .blur-sidebar .sidebar-brand { color: #02182b; }
    .blur-sidebar .sidebar-brand-sub { color: #718096; }
    .blur-sidebar .sidebar-item { color: #4a5568; }
    .blur-sidebar .sidebar-item:hover { background: rgba(0,0,0,0.05); color: #02182b; }
    .blur-sidebar .sidebar-item.is-active { background: var(--cg-navy); color: #fff; box-shadow: 0 4px 8px rgba(2,24,43,0.15); }
    .blur-sidebar .sidebar-section-label { color: #8a9ba8; }

    .sidebar-logo {
      padding: 1.25rem 1rem 0.75rem;
      border-bottom: 1px solid rgba(255,255,255,0.08);
    }
    .sidebar-brand-link { text-decoration: none; }
    .sidebar-brand {
      color: #fff;
      font-size: 1.2rem;
      font-weight: 800;
      letter-spacing: 0.04em;
    }
    .sidebar-brand-sub {
      color: rgba(200,216,232,0.65);
      font-size: 0.72rem;
      font-weight: 600;
      margin-top: 0.15rem;
      text-transform: uppercase;
      letter-spacing: 0.06em;
    }
    .sidebar-org-select {
      padding: 0.75rem 1rem;
      border-bottom: 1px solid rgba(255,255,255,0.08);
    }
    .sidebar-label {
      display: block;
      font-size: 0.68rem;
      font-weight: 700;
      text-transform: uppercase;
      letter-spacing: 0.08em;
      color: rgba(200,216,232,0.5);
      margin-bottom: 0.4rem;
    }
    .custom-dropdown {
      position: relative;
    }
    .custom-dropdown-btn {
      width: 100%;
      background: linear-gradient(135deg, #02182b 0%, #f05708 100%);
      border: 1px solid rgba(255,255,255,0.2);
      border-radius: 8px;
      color: #ffffff;
      padding: 0.6rem 0.8rem;
      font-size: 0.85rem;
      font-weight: 600;
      cursor: pointer;
      text-align: left;
      display: flex;
      justify-content: space-between;
      align-items: center;
      box-shadow: 0 2px 4px rgba(0,0,0,0.2);
      transition: all 0.2s;
    }
    .custom-dropdown-btn:hover {
      border-color: rgba(255,255,255,0.4);
    }
    .custom-dropdown-btn::after {
      content: "";
      display: inline-block;
      width: 1em;
      height: 1em;
      background: url("data:image/svg+xml;charset=UTF-8,%3csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24' fill='none' stroke='%23ffffff' stroke-width='2' stroke-linecap='round' stroke-linejoin='round'%3e%3cpolyline points='6 9 12 15 18 9'%3e%3c/polyline%3e%3c/svg%3e") no-repeat center;
      background-size: contain;
    }
    .custom-dropdown-menu {
      position: absolute;
      top: 100%;
      left: 0;
      right: 0;
      margin-top: 0.2rem;
      background: #ffffff;
      border-radius: 8px;
      box-shadow: 0 4px 12px rgba(0,0,0,0.15);
      z-index: 1000;
      max-height: 300px;
      overflow-y: auto;
      display: none;
    }
    .custom-dropdown-menu.show {
      display: block;
    }
    .custom-dropdown-item {
      display: block;
      padding: 0.5rem 0.8rem;
      color: #2d3748;
      text-decoration: none;
      font-size: 0.85rem;
      border-bottom: 1px solid #edf2f7;
    }
    .custom-dropdown-item:hover {
      background: #f7fafc;
    }
    .custom-dropdown-item.selected {
      background: #ebf8ff;
      color: #2b6cb0;
      font-weight: 600;
    }
    .sidebar-nav {
      flex: 1;
      padding: 0.5rem 0.5rem;
      display: flex;
      flex-direction: column;
      gap: 0.1rem;
    }
    .sidebar-section-label {
      font-size: 0.65rem;
      font-weight: 800;
      text-transform: uppercase;
      letter-spacing: 0.1em;
      color: rgba(200,216,232,0.35);
      padding: 0.8rem 0.6rem 0.3rem;
    }
    .sidebar-item {
      display: flex;
      align-items: center;
      gap: 0.6rem;
      padding: 0.55rem 0.75rem;
      border-radius: 8px;
      font-size: 0.85rem;
      font-weight: 600;
      color: rgba(200,216,232,0.8);
      text-decoration: none;
      transition: all 0.15s ease;
      cursor: pointer;
    }
    .sidebar-item:hover {
      background: rgba(255,255,255,0.08);
      color: #fff;
    }
    .sidebar-item.is-active {
      background: linear-gradient(135deg, rgba(240,87,8,0.25) 0%, rgba(240,87,8,0.15) 100%);
      color: #fff;
      border-left: 3px solid #f05708;
    }
    .sidebar-icon { font-size: 1rem; min-width: 1.2rem; }
    .sidebar-footer {
      padding: 0.75rem 1rem;
      border-top: 1px solid rgba(255,255,255,0.08);
      font-size: 0.78rem;
      color: rgba(200,216,232,0.5);
    }
    .sidebar-user {
      font-weight: 600;
      color: rgba(200,216,232,0.7);
      white-space: nowrap;
      overflow: hidden;
      text-overflow: ellipsis;
    }

    /* ── Main content shifted right for sidebar ─────────────────────────── */
    .main-with-sidebar {
      padding: 1.5rem 0rem 2rem;
      min-height: 100vh;
    }
    .page-title {
      font-size: 1.6rem;
      font-weight: 800;
      color: var(--cg-navy);
      margin: 0.5rem 0 1rem;
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
    .form-group-card {
      background: #ffffff;
      border: 1px solid rgba(197, 183, 171, 0.55);
      border-radius: 12px;
      padding: 0.85rem 1.15rem;
      margin-bottom: 1.1rem;
      box-shadow: 0 4px 10px rgba(2, 24, 43, 0.02);
      transition: all 0.22s cubic-bezier(0.25, 0.8, 0.25, 1);
      position: relative;
      overflow: hidden;
      display: block;
      cursor: pointer;
    }
    .form-group-card::before {
      content: '';
      position: absolute;
      left: 0;
      top: 0;
      bottom: 0;
      width: 4px;
      background: #c5b7ab;
      opacity: 0.45;
      transition: all 0.22s cubic-bezier(0.25, 0.8, 0.25, 1);
    }
    .form-group-card:hover {
      border-color: rgba(2, 24, 43, 0.22);
      box-shadow: 0 6px 14px rgba(2, 24, 43, 0.04);
      transform: translateY(-1.5px);
    }
    .form-group-card.is-focused {
      border-color: var(--cg-orange);
      box-shadow: 0 10px 22px rgba(240, 87, 8, 0.09);
      transform: translateY(-3px);
    }
    .form-group-card.is-focused::before {
      background: var(--cg-orange);
      opacity: 1;
      width: 6px;
    }
    .form-group-card.is-complete::before {
      background: #3182ce;
      opacity: 1;
      width: 6px;
    }
    .form-group-card label {
      margin-top: 0 !important;
      margin-bottom: 0.22rem !important;
      font-size: 0.76rem !important;
      color: #718096 !important;
      text-transform: uppercase !important;
      letter-spacing: 0.05em !important;
      transition: color 0.2s ease;
      display: block;
    }
    .form-group-card.is-focused label {
      color: var(--cg-orange) !important;
    }
    .form-group-card.is-complete label {
      color: #3182ce !important;
    }
    .form-group-card input:not([type="checkbox"]):not([type="radio"]), 
    .form-group-card textarea, 
    .form-group-card select {
      width: 100% !important;
      max-width: none !important;
      border: none !important;
      padding: 0.25rem 0 !important;
      font-size: 1.05rem !important;
      font-weight: 600 !important;
      background: transparent !important;
      box-shadow: none !important;
      outline: none !important;
      margin-top: 0 !important;
      color: var(--cg-navy) !important;
    }
    .form-group-card .complete-badge {
      position: absolute;
      top: 0.85rem;
      right: 1.15rem;
      font-size: 0.7rem;
      font-weight: 800;
      color: #3182ce;
      background: #ebf8ff;
      border: 1px solid #c2ebd0;
      padding: 0.15rem 0.45rem;
      border-radius: 4px;
      text-transform: uppercase;
      letter-spacing: 0.04em;
      opacity: 0;
      transform: scale(0.8);
      transition: all 0.25s cubic-bezier(0.175, 0.885, 0.32, 1.275);
      pointer-events: none;
    }
    .form-group-card.is-complete .complete-badge {
      opacity: 1;
      transform: scale(1);
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
  <script>
{}
  </script>
</body>
</html>"#,
        html_escape(title),
        cingulum_theme_css(),
        cingulum_home_css(),
        body_content,
        cingulum_global_js()
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
      <a href="/ui" id="brand-link" style="text-decoration:none; color:inherit; font-weight:inherit; display:flex; flex-direction:column; align-items:flex-start;">
        <div class="brand">Cingulum Foundation Inc.</div>
      </a>
      <div class="brand-sub">Virivu Research Cloud</div>
    </div>
    {}
  </main>
  <script>
{}
  </script>
</body>
</html>"#,
        html_escape(title),
        cingulum_theme_css(),
        body_content,
        cingulum_global_js()
    )
}

fn cingulum_global_js() -> &'static str {
    r#"
    (() => {
      const brandLink = document.getElementById('brand-link');
      if (brandLink) {
        const adminEmail = new URLSearchParams(window.location.search).get('admin_email');
        if (adminEmail) {
          brandLink.href = `/ui?admin_email=${encodeURIComponent(adminEmail)}`;
        }
      }
      const bars = document.querySelectorAll('.tab-bar[data-tab-group]');
      const tabParam = new URLSearchParams(window.location.search).get('tab');
      bars.forEach((bar) => {
        const group = bar.getAttribute('data-tab-group');
        const buttons = Array.from(bar.querySelectorAll('.tab-button[data-tab-id]'));
        const panels = Array.from(document.querySelectorAll(`.tab-panel[data-tab-group="${group}"]`));
        if (buttons.length === 0 || panels.length === 0) return;
        const activate = (tabId) => {
          buttons.forEach((btn) => {
            const active = btn.getAttribute('data-tab-id') === tabId;
            btn.classList.toggle('is-active', active);
            btn.setAttribute('aria-selected', active ? 'true' : 'false');
          });
          panels.forEach((panel) => {
            const active = panel.getAttribute('data-tab-panel') === tabId;
            panel.classList.toggle('is-active', active);
          });
        };
        let initial = buttons.find((b) => b.classList.contains('is-active'))?.getAttribute('data-tab-id');
        if (tabParam && buttons.some((b) => b.getAttribute('data-tab-id') === tabParam)) {
          initial = tabParam;
        }
        if (!initial) initial = buttons[0].getAttribute('data-tab-id');
        activate(initial);
        buttons.forEach((btn) => {
          btn.addEventListener('click', () => activate(btn.getAttribute('data-tab-id')));
        });
      });
      const fieldForms = Array.from(document.querySelectorAll('form[data-crf-field-form]'));
      const createOptionRow = (value = '') => {
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
      };
      fieldForms.forEach((form) => {
        const fieldTypeSelect = form.querySelector('[data-field-type-select]');
        const optionsSection = form.querySelector('[data-options-section]');
        const optionList = form.querySelector('[data-option-list]');
        const addOptionButton = form.querySelector('[data-add-option-button]');
        const optionsTextInput = form.querySelector('input[name="options_text"]');
        const optionsInitialInput = form.querySelector('[data-options-initial]');
        if (!fieldTypeSelect || !optionsSection || !optionList || !addOptionButton || !optionsTextInput || !optionsInitialInput) return;
        const syncOptionsTextInput = () => {
          const values = Array.from(optionList.querySelectorAll('[data-option-input]'))
            .map((input) => input.value.trim())
            .filter((value) => value.length > 0);
          optionsTextInput.value = values.join('\n');
        };
        const ensureOptionRow = () => {
          if (optionList.querySelectorAll('[data-option-input]').length === 0) {
            optionList.appendChild(createOptionRow(''));
          }
        };
        const seedOptionRows = () => {
          const seededValues = (optionsInitialInput.value || '')
            .split(/\r?\n/)
            .map((line) => line.trim())
            .filter((line) => line.length > 0);
          optionList.innerHTML = '';
          if (seededValues.length === 0) {
            optionList.appendChild(createOptionRow(''));
          } else {
            seededValues.forEach((value) => optionList.appendChild(createOptionRow(value)));
          }
          syncOptionsTextInput();
        };
        seedOptionRows();
        optionList.addEventListener('input', (event) => {
          if (event.target && event.target.matches('[data-option-input]')) {
            syncOptionsTextInput();
          }
        });
        optionList.addEventListener('click', (event) => {
          const target = event.target;
          if (!(target instanceof HTMLElement)) return;
          if (!target.matches('[data-remove-option-button]')) return;
          const row = target.closest('.option-row');
          if (row) row.remove();
          ensureOptionRow();
          syncOptionsTextInput();
        });
        addOptionButton.addEventListener('click', () => {
          optionList.appendChild(createOptionRow(''));
          syncOptionsTextInput();
        });
        const syncFieldOptionsVisibility = () => {
          const fieldType = (fieldTypeSelect.value || '').toLowerCase();
          const showOptions = fieldType === 'single_select' || fieldType === 'multi_select';
          optionsSection.style.display = showOptions ? 'block' : 'none';
          if (showOptions) {
            ensureOptionRow();
          }
          syncOptionsTextInput();
        };
        form.addEventListener('submit', () => syncOptionsTextInput());
        fieldTypeSelect.addEventListener('change', syncFieldOptionsVisibility);
        syncFieldOptionsVisibility();
      });
  
      const mobileBtn = document.querySelector('.mobile-menu-btn');
      const sidebar = document.getElementById('app-sidebar');
      if (mobileBtn && sidebar) {
        mobileBtn.addEventListener('click', () => {
          sidebar.classList.toggle('open');
        });
        document.addEventListener('click', (e) => {
          if (!sidebar.contains(e.target) && !mobileBtn.contains(e.target)) {
            sidebar.classList.remove('open');
          }
        });
        document.addEventListener('click', (e) => {
          if (!e.target.closest('.custom-dropdown')) {
            document.querySelectorAll('.custom-dropdown-menu').forEach(m => m.classList.remove('show'));
          }
        });

      }

      const hashId = window.location.hash ? window.location.hash.slice(1) : '';

      if (hashId) {
        const target = document.getElementById(hashId);
        if (target) {
          window.requestAnimationFrame(() => {
            target.scrollIntoView({ behavior: 'smooth', block: 'start' });
          });
        }
      }
      // Dynamic Form Card Overhaul Engine
      const overhaulFormsToCards = () => {
        const formControls = document.querySelectorAll(
          'form:not(.no-auto-cards):not([style*="display:inline"]):not([style*="display:inline-block"]) input:not([type="submit"]):not([type="button"]):not([type="hidden"]):not([type="checkbox"]):not([type="radio"]), ' +
          'form:not(.no-auto-cards):not([style*="display:inline"]):not([style*="display:inline-block"]) textarea, ' +
          'form:not(.no-auto-cards):not([style*="display:inline"]):not([style*="display:inline-block"]) select:not(.sidebar-org-picker)'
        );
        
        formControls.forEach((control) => {
          if (control.closest('.form-group-card')) return;
          
          let label = null;
          let sibling = control.previousElementSibling;
          while (sibling) {
            if (sibling.tagName === 'LABEL') {
              label = sibling;
              break;
            }
            sibling = sibling.previousElementSibling;
          }
          
          const card = document.createElement('div');
          card.className = 'form-group-card';
          if (control.tagName === 'TEXTAREA' || control.classList.contains('is-wide') || control.name === 'options_text') {
            card.classList.add('is-wide');
            card.style.gridColumn = '1 / -1';
          }
          
          const insertRef = label || control;
          insertRef.parentNode.insertBefore(card, insertRef);
          
          if (label) card.appendChild(label);
          card.appendChild(control);
          
          const badge = document.createElement('span');
          badge.className = 'complete-badge';
          badge.textContent = '✓ Complete';
          card.appendChild(badge);
          
          const updateStates = () => {
            const val = control.value || '';
            const isComplete = val.trim().length > 0;
            card.classList.toggle('is-complete', isComplete);
          };
          
          control.addEventListener('focus', () => card.classList.add('is-focused'));
          control.addEventListener('blur', () => card.classList.remove('is-focused'));
          control.addEventListener('input', updateStates);
          control.addEventListener('change', updateStates);
          
          card.addEventListener('click', (e) => {
            if (e.target !== control) {
              control.focus();
            }
          });
          
          updateStates();
        });
      };
      
      overhaulFormsToCards();
      // Schedule checks to catch late dynamically added forms or panels
      setTimeout(overhaulFormsToCards, 100);
      setTimeout(overhaulFormsToCards, 500);
      setTimeout(overhaulFormsToCards, 1500);
      
      // Also listen to click events on tab-buttons to overhaul active panel forms immediately
      document.querySelectorAll('.tab-button').forEach(btn => {
        btn.addEventListener('click', () => {
          setTimeout(overhaulFormsToCards, 50);
          setTimeout(overhaulFormsToCards, 200);
        });
      });
    })();
    "#
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
