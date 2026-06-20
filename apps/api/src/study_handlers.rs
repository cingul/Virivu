//! Study lifecycle, CRF, submission, and checklist submit handlers.
//! Extracted from routes.rs to keep each module to a manageable size.

use axum::{
    extract::{Multipart, Path, State},
    response::Redirect,
    Form,
};
use lopdf::{Document as LopdfDocument, Object as LopdfObject};
use scraper::{Html as ParsedHtml, Selector};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

use crate::{
    auth::AuthenticatedUser,
    authz::{require_org_role, ROLE_COORDINATOR_OR_BETTER, ROLE_ORG_MANAGERS},
    error::{map_db_error, ApiError},
    models::StudyCrfField,
    routes::{
        AppContext, html_escape, optional_non_empty, parse_optional_date, parse_uuid_field,
        query_escape,
    },
};

// ── Form structs ─────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct StudyCreateForm {
    pub admin_email: String,
    pub organization_hex: String,
    pub study_name: String,
    pub therapeutic_area: String,
    pub protocol_code: String,
    pub planned_enrollment: String,
    pub clinicaltrials_gov_id: String,
    pub study_summary: String,
}

#[derive(Debug, Deserialize)]
pub struct StudyPhaseTransitionForm {
    pub project_id: Option<String>,
    pub admin_email: String,
    pub next_phase: String,
    pub notes: String,
}

#[derive(Debug, Deserialize)]
pub struct StudyCrfTemplateForm {
    pub admin_email: String,
    pub name: String,
    pub description: String,
    pub applicable_phase: String,
}

#[derive(Debug, Deserialize)]
pub struct StudyCrfFieldForm {
    pub admin_email: String,
    pub field_key: String,
    pub field_label: String,
    pub field_type: String,
    pub required: Option<String>,
    #[serde(default)]
    pub options_json: String,
    #[serde(default)]
    pub options_text: String,
    #[serde(default)]
    pub branching_logic_json: String,
    #[serde(default)]
    pub edit_checks_json: String,
    pub display_order: String,
}

#[derive(Debug, Deserialize)]
pub struct StudyCrfBulkDeleteForm {
    pub admin_email: String,
    pub confirmation_text: String,
}

#[derive(Debug, Deserialize)]
pub struct StudyCrfPublishForm {
    pub admin_email: String,
}

#[derive(Debug, Deserialize)]
pub struct StudyVisitTemplateForm {
    pub admin_email: String,
    pub visit_code: String,
    pub visit_name: String,
    pub target_day: String,
    pub window_before_days: String,
    pub window_after_days: String,
    pub required: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct StudyScheduleVisitForm {
    pub admin_email: String,
    pub patient_id: String,
    pub visit_template_id: String,
    pub scheduled_for: String,
}

#[derive(Debug, Deserialize)]
pub struct StudyCreateSubmissionForm {
    pub admin_email: String,
    pub template_id: String,
    pub patient_id: String,
    pub patient_visit_id: String,
    pub answers_json: String,
}

#[derive(Debug, Deserialize)]
pub struct StudySubmissionActionForm {
    pub admin_email: String,
}

#[derive(Debug, Deserialize)]
pub struct StudyCreateQueryForm {
    pub admin_email: String,
    pub submission_id: String,
    pub field_key: String,
    pub query_text: String,
}

#[derive(Debug, Deserialize)]
pub struct StudyRespondQueryForm {
    pub admin_email: String,
    pub response_text: String,
}

#[derive(Debug, Deserialize)]
pub struct StudyCloseQueryForm {
    pub admin_email: String,
}

#[derive(Debug, Deserialize)]
pub struct StudyChecklistItemForm {
    pub admin_email: String,
    pub item_code: String,
    pub item_label: String,
    pub completed: Option<String>,
    pub notes: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateStudyCrfSubmissionSdvForm {
    pub admin_email: String,
    pub sdv_status: String,
}

pub(crate) async fn submit_create_study_from_ui(
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

pub(crate) async fn submit_study_phase_transition(
    State(ctx): State<AppContext>,
    Path(project_id): Path<Uuid>,
    Form(form): Form<StudyPhaseTransitionForm>,
) -> Result<Redirect, ApiError> {
    perform_study_phase_transition(&ctx, project_id, &form).await
}

pub(crate) async fn submit_study_phase_transition_v2(
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

pub(crate) async fn submit_create_study_crf_template(
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

pub(crate) async fn submit_add_study_crf_field(
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

pub(crate) async fn submit_bulk_delete_study_crf_fields(
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

pub(crate) async fn submit_bulk_delete_corrupted_study_crf_fields(
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

pub(crate) async fn submit_import_study_crf_fields_html(
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

pub(crate) async fn submit_update_study_crf_field(
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

pub(crate) async fn submit_publish_study_crf_template(
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
    let field_count = ctx
        .db
        .list_study_crf_fields(template_id)
        .await
        .map(|f| f.len())
        .unwrap_or(0);

    ctx.db
        .publish_study_crf_template(template_id)
        .await
        .map_err(ApiError::internal)?;

    // Strong provenance for publish (CRF template is now locked for the study)
    let old_data = Some(format!(
        r#"{{"status":"draft","field_count":{}}}"#,
        field_count
    ));
    let new_data = Some(r#"{"status":"published"}"#.to_string());
    let _ = ctx
        .db
        .insert_audit_log(
            "study_crf_templates",
            template_id,
            "published",
            Some(user.user_id),
            old_data.as_deref(),
            new_data.as_deref(),
            Some(&format!(
                "CRF template published ({} fields) - template is now immutable",
                field_count
            )),
        )
        .await;

    Ok(Redirect::to(&format!(
        "/ui/studies?admin_email={}&organization_id={}&project_id={}&template_id={}&notice={}",
        query_escape(user.email.as_str()),
        project.organization_id,
        project.id,
        template_id,
        query_escape("CRF template published")
    )))
}

pub(crate) async fn submit_create_study_visit_template(
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

pub(crate) async fn submit_schedule_patient_visit(
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
    let visit = ctx
        .db
        .schedule_patient_study_visit(project_id, patient_id, visit_template_id, scheduled_for)
        .await
        .map_err(ApiError::internal)?;

    // Audit
    let _ = ctx
        .db
        .insert_audit_log(
            "patient_study_visits",
            visit.id,
            "scheduled",
            Some(user.user_id),
            None,
            None,
            None,
        )
        .await;

    Ok(Redirect::to(&format!(
        "/ui/studies?admin_email={}&organization_id={}&project_id={}&notice={}",
        query_escape(user.email.as_str()),
        project.organization_id,
        project_id,
        query_escape("Patient visit scheduled")
    )))
}

pub(crate) async fn submit_create_study_crf_submission(
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

pub(crate) async fn submit_mark_study_crf_submission_submitted(
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
        let _ = ctx
            .db
            .insert_audit_log(
                "study_crf_submissions",
                submission_id,
                "mutation_rejected",
                Some(user.user_id),
                None,
                None,
                Some("Attempt to mark locked submission as submitted via UI (answers frozen)"),
            )
            .await;
        return Err(ApiError::Validation(
            "This CRF submission is locked. Answers are frozen and it cannot be re-submitted."
                .to_string(),
        ));
    }
    if submission.status != "draft" {
        return Err(ApiError::Validation(
            "Only draft submissions can be marked submitted.".to_string(),
        ));
    }

    // Snapshot answers at submit for provenance
    let current = ctx
        .db
        .get_study_crf_submission(submission_id)
        .await
        .ok()
        .flatten();
    let snapshot = current.as_ref().map(|s| s.answers_json.clone());

    ctx.db
        .submit_study_crf_submission(submission_id)
        .await
        .map_err(ApiError::internal)?;

    // Audit: submission moved to submitted state with snapshot
    let _ = ctx
        .db
        .insert_audit_log(
            "study_crf_submissions",
            submission_id,
            "submitted",
            Some(user.user_id),
            snapshot.as_deref(),
            None,
            Some("Submitted - answers recorded at transition"),
        )
        .await;

    Ok(Redirect::to(&format!(
        "/ui/studies?admin_email={}&organization_id={}&project_id={}&submission_id={}&notice={}",
        query_escape(user.email.as_str()),
        project.organization_id,
        project.id,
        submission_id,
        query_escape("Submission marked submitted")
    )))
}

pub(crate) async fn submit_lock_study_crf_submission(
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
        let _ = ctx
            .db
            .insert_audit_log(
                "study_crf_submissions",
                submission_id,
                "mutation_rejected",
                Some(user.user_id),
                None,
                None,
                Some("Attempt to re-lock an already locked submission via UI"),
            )
            .await;
        return Err(ApiError::Validation(
            "This CRF submission is already locked. Answers are frozen.".to_string(),
        ));
    }

    // Capture current answers for provenance before locking
    let current_submission = ctx
        .db
        .get_study_crf_submission(submission_id)
        .await
        .ok()
        .flatten();
    let answers_snapshot = current_submission.as_ref().map(|s| s.answers_json.clone());

    ctx.db
        .lock_study_crf_submission(submission_id)
        .await
        .map_err(ApiError::internal)?;

    // Audit with snapshot for basic provenance
    let _ = ctx
        .db
        .insert_audit_log(
            "study_crf_submissions",
            submission_id,
            "locked",
            Some(user.user_id),
            answers_snapshot.as_deref(),
            None,
            Some("Locked via UI - answers frozen"),
        )
        .await;

    Ok(Redirect::to(&format!(
        "/ui/studies?admin_email={}&organization_id={}&project_id={}&submission_id={}&notice={}",
        query_escape(user.email.as_str()),
        project.organization_id,
        project.id,
        submission_id,
        query_escape("Submission locked")
    )))
}

pub(crate) async fn submit_update_study_crf_submission_sdv(
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
    let _ = ctx
        .db
        .insert_audit_log(
            "study_crf_submissions",
            submission_id,
            "sdv_updated",
            Some(user.user_id),
            old_data.as_deref(),
            new_data.as_deref(),
            Some(&format!(
                "SDV status changed: {} → {}",
                old_sdv_status, new_sdv_status
            )),
        )
        .await;

    Ok(Redirect::to(&format!(
        "/ui/studies?admin_email={}&organization_id={}&project_id={}&submission_id={}&view=submissions&notice={}",
        query_escape(user.email.as_str()),
        project.organization_id,
        project.id,
        submission_id,
        query_escape("SDV status updated successfully")
    )))
}

pub(crate) async fn submit_create_study_data_query(
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
    if submission.project_id != project_id {
        return Err(ApiError::Validation(
            "submission does not belong to project".to_string(),
        ));
    }

    if submission.status == "draft" {
        return Err(ApiError::Conflict(
            "Cannot raise queries on a draft submission.".to_string(),
        ));
    }

    let raised_by_user_id = ctx
        .db
        .get_user_by_email(user.email.as_str())
        .await
        .map_err(ApiError::internal)?
        .map(|u| u.id);
    let query = ctx
        .db
        .create_study_data_query(
            project_id,
            submission_id,
            form.field_key.trim(),
            form.query_text.trim(),
            raised_by_user_id,
        )
        .await
        .map_err(map_db_error)?;

    // Audit - data queries are key monitoring/compliance artifacts
    let _ = ctx
        .db
        .insert_audit_log(
            "study_data_queries",
            query.id,
            "created",
            Some(user.user_id),
            None,
            None,
            Some(&format!("Field: {}", form.field_key.trim())),
        )
        .await;

    Ok(Redirect::to(&format!(
        "/ui/studies?admin_email={}&organization_id={}&project_id={}&submission_id={}&notice={}",
        query_escape(user.email.as_str()),
        project.organization_id,
        project_id,
        submission_id,
        query_escape("Data query created")
    )))
}

pub(crate) async fn submit_respond_study_data_query(
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
    if response_text.is_empty() {
        return Err(ApiError::Validation(
            "Response text is required before marking a query responded.".to_string(),
        ));
    }

    ctx.db
        .respond_study_data_query(query_id, &response_text)
        .await
        .map_err(map_db_error)?;

    // Rich provenance for query response (key compliance artifact)
    let new_data = Some(format!(
        r#"{{"response":"{}"}}"#,
        serde_json::to_string(&response_text).unwrap_or_else(|_| response_text.clone())
    ));
    let _ = ctx
        .db
        .insert_audit_log(
            "study_data_queries",
            query_id,
            "responded",
            Some(user.user_id),
            None,
            new_data.as_deref(),
            Some(&format!("Query responded ({} chars)", response_text.len())),
        )
        .await;

    Ok(Redirect::to(&format!(
        "/ui/studies?admin_email={}&organization_id={}&project_id={}&submission_id={}&notice={}",
        query_escape(user.email.as_str()),
        project.organization_id,
        project.id,
        query.submission_id,
        query_escape("Data query response saved")
    )))
}

pub(crate) async fn submit_close_study_data_query(
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
        .map_err(map_db_error)?;

    // Rich provenance for query closure
    let _ = ctx
        .db
        .insert_audit_log(
            "study_data_queries",
            query_id,
            "closed",
            Some(user.user_id),
            None,
            None,
            Some("Data query closed by coordinator"),
        )
        .await;

    Ok(Redirect::to(&format!(
        "/ui/studies?admin_email={}&organization_id={}&project_id={}&submission_id={}&notice={}",
        query_escape(user.email.as_str()),
        project.organization_id,
        project.id,
        query.submission_id,
        query_escape("Data query closed")
    )))
}

pub(crate) async fn submit_set_study_startup_checklist_item(
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

pub(crate) async fn submit_set_site_startup_checklist_item(
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
    let project_id = site.project_id.ok_or_else(|| {
        ApiError::Validation("Site must be attached to a study for this action".to_string())
    })?;
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
    let completed_by_user_id = if completed { Some(user.user_id) } else { None };
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

pub(crate) async fn submit_set_study_close_checklist_item(
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

pub(crate) fn options_json_to_lines(options_json: &str) -> String {
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

pub(crate) fn generate_sample_answers_json(fields: &[StudyCrfField]) -> String {
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
                if let Ok(opts) =
                    serde_json::from_str::<Vec<serde_json::Value>>(&field.options_json)
                {
                    opts.first()
                        .cloned()
                        .unwrap_or(serde_json::Value::String("option1".to_string()))
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

pub(crate) fn render_crf_field_for_data_entry(field: &StudyCrfField, current_value: &str) -> String {
    let name = format!("field_{}", field.field_key);
    let required = if field.required { " required" } else { "" };
    let label = html_escape(&field.field_label);
    let value_esc = html_escape(current_value);

    match field.field_type.as_str() {
        "text" => format!(
            r#"<label style="display:block;margin-top:0.5rem;font-weight:600;">{} {}</label><input type="text" name="{}" value="{}" style="width:100%;padding:6px;"{} />"#,
            label,
            if field.required { "*" } else { "" },
            name,
            value_esc,
            required
        ),
        "textarea" => format!(
            r#"<label style="display:block;margin-top:0.5rem;font-weight:600;">{} {}</label><textarea name="{}" rows="3" style="width:100%;padding:6px;"{}>{}</textarea>"#,
            label,
            if field.required { "*" } else { "" },
            name,
            required,
            value_esc
        ),
        "number" => format!(
            r#"<label style="display:block;margin-top:0.5rem;font-weight:600;">{} {}</label><input type="number" name="{}" value="{}" style="width:100%;padding:6px;"{} />"#,
            label,
            if field.required { "*" } else { "" },
            name,
            value_esc,
            required
        ),
        "date" => format!(
            r#"<label style="display:block;margin-top:0.5rem;font-weight:600;">{} {}</label><input type="date" name="{}" value="{}" style="width:100%;padding:6px;"{} />"#,
            label,
            if field.required { "*" } else { "" },
            name,
            value_esc,
            required
        ),
        "boolean" => {
            let checked = if current_value == "true" || current_value == "1" {
                " checked"
            } else {
                ""
            };
            format!(
                r#"<label style="display:block;margin-top:0.5rem;"><input type="checkbox" name="{}" value="true"{} {} /> {}</label>"#,
                name, checked, required, label
            )
        }
        "single_select" => {
            let mut opts = String::new();
            if let Ok(items) = serde_json::from_str::<Vec<String>>(&field.options_json) {
                for item in items {
                    let sel = if item == current_value {
                        " selected"
                    } else {
                        ""
                    };
                    opts.push_str(&format!(
                        r#"<option value="{}"{}>{}</option>"#,
                        html_escape(&item),
                        sel,
                        html_escape(&item)
                    ));
                }
            }
            format!(
                r#"<label style="display:block;margin-top:0.5rem;font-weight:600;">{} {}</label><select name="{}" style="width:100%;padding:6px;"{}>{}</select>"#,
                label,
                if field.required { "*" } else { "" },
                name,
                required,
                opts
            )
        }
        "multi_select" => {
            let mut opts = String::new();
            let selected: Vec<&str> = current_value.split(',').collect();
            if let Ok(items) = serde_json::from_str::<Vec<String>>(&field.options_json) {
                for item in items {
                    let sel = if selected.contains(&item.as_str()) {
                        " checked"
                    } else {
                        ""
                    };
                    opts.push_str(&format!(r#"<label style="display:block;"><input type="checkbox" name="{}[]" value="{}"{} /> {}</label>"#, name, html_escape(&item), sel, html_escape(&item)));
                }
            }
            format!(
                r#"<label style="display:block;margin-top:0.5rem;font-weight:600;">{} {}</label><div style="padding-left:4px;">{}</div>"#,
                label,
                if field.required { "*" } else { "" },
                opts
            )
        }
        _ => format!(
            r#"<label style="display:block;margin-top:0.5rem;font-weight:600;">{} {}</label><input type="text" name="{}" value="{}" style="width:100%;padding:6px;"{} />"#,
            label,
            if field.required { "*" } else { "" },
            name,
            value_esc,
            required
        ),
    }
}

pub(crate) fn render_crf_field_type_options(selected_type: &str) -> String {
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
