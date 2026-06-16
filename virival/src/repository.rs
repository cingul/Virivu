use std::str::FromStr;

use async_trait::async_trait;
use tokio_postgres::Row;
use uuid::Uuid;

use crate::{
    db::DbPool,
    error::ApiError,
    models::{
        AgreementStatus, AppRole, AuditLogRecord, CloseoutChecklistItem, CrfSubmission,
        CrfTemplate, CrfTemplateVersion, DataQuery, DataQueryComment, DuaAgreement, DuaSignature,
        MediaAsset, MediaAssetStatus, NewAuditLogEntry, Organization, OrganizationMediaPolicy,
        OrganizationMediaUsage, OrganizationMembership, Patient, QueryStatus, ReminderJob,
        ReminderJobStatus, Site, Study, StudyPhase, StudyReadiness, SubmissionStatus, User, Visit,
        VisitScheduleTemplate, VisitStatus,
    },
};

#[async_trait]
pub trait Repository: Send + Sync {
    async fn create_organization(
        &self,
        name: &str,
        workspace_slug: &str,
    ) -> Result<Organization, ApiError>;
    async fn list_organizations(&self) -> Result<Vec<Organization>, ApiError>;
    async fn get_organization(&self, organization_id: Uuid) -> Result<Organization, ApiError>;

    async fn create_study(
        &self,
        organization_id: Uuid,
        short_code: &str,
        title: &str,
    ) -> Result<Study, ApiError>;
    async fn list_studies(&self) -> Result<Vec<Study>, ApiError>;
    async fn get_study(&self, study_id: Uuid) -> Result<Study, ApiError>;
    async fn update_study_phase(
        &self,
        study_id: Uuid,
        phase: StudyPhase,
    ) -> Result<Study, ApiError>;

    async fn create_site(
        &self,
        organization_id: Uuid,
        study_id: Option<Uuid>,
        name: &str,
        principal_investigator: &str,
    ) -> Result<Site, ApiError>;
    async fn list_sites(&self) -> Result<Vec<Site>, ApiError>;
    async fn get_site(&self, site_id: Uuid) -> Result<Site, ApiError>;
    async fn set_site_startup(
        &self,
        site_id: Uuid,
        startup_complete: bool,
    ) -> Result<Site, ApiError>;

    async fn create_patient(
        &self,
        organization_id: Uuid,
        study_id: Uuid,
        site_id: Option<Uuid>,
        external_id: &str,
    ) -> Result<Patient, ApiError>;
    async fn list_patients(&self) -> Result<Vec<Patient>, ApiError>;
    async fn get_patient(&self, patient_id: Uuid) -> Result<Patient, ApiError>;

    async fn create_visit(
        &self,
        study_id: Uuid,
        patient_id: Uuid,
        visit_name: &str,
        scheduled_for: chrono::DateTime<chrono::Utc>,
    ) -> Result<Visit, ApiError>;
    async fn list_visits(&self) -> Result<Vec<Visit>, ApiError>;
    async fn get_visit(&self, visit_id: Uuid) -> Result<Visit, ApiError>;

    async fn create_crf_template(
        &self,
        study_id: Uuid,
        name: &str,
    ) -> Result<CrfTemplate, ApiError>;
    async fn list_crf_templates(&self) -> Result<Vec<CrfTemplate>, ApiError>;
    async fn get_crf_template(&self, template_id: Uuid) -> Result<CrfTemplate, ApiError>;
    async fn publish_crf_template(&self, template_id: Uuid) -> Result<CrfTemplate, ApiError>;
    async fn create_crf_template_version(
        &self,
        template_id: Uuid,
        schema_json: &serde_json::Value,
    ) -> Result<CrfTemplateVersion, ApiError>;
    async fn list_crf_template_versions(
        &self,
        template_id: Uuid,
    ) -> Result<Vec<CrfTemplateVersion>, ApiError>;
    async fn publish_crf_template_version(
        &self,
        version_id: Uuid,
    ) -> Result<CrfTemplateVersion, ApiError>;

    async fn create_crf_submission(
        &self,
        study_id: Uuid,
        patient_id: Uuid,
        visit_id: Uuid,
        template_id: Uuid,
    ) -> Result<CrfSubmission, ApiError>;
    async fn list_crf_submissions(&self) -> Result<Vec<CrfSubmission>, ApiError>;
    async fn get_crf_submission(&self, submission_id: Uuid) -> Result<CrfSubmission, ApiError>;
    async fn lock_crf_submission(&self, submission_id: Uuid) -> Result<CrfSubmission, ApiError>;

    async fn create_data_query(
        &self,
        study_id: Uuid,
        submission_id: Uuid,
        summary: &str,
    ) -> Result<DataQuery, ApiError>;
    async fn list_data_queries(&self) -> Result<Vec<DataQuery>, ApiError>;
    async fn respond_data_query(&self, query_id: Uuid) -> Result<DataQuery, ApiError>;
    async fn close_data_query(&self, query_id: Uuid) -> Result<DataQuery, ApiError>;
    async fn create_data_query_comment(
        &self,
        query_id: Uuid,
        author_user_id: Option<Uuid>,
        comment_text: &str,
    ) -> Result<DataQueryComment, ApiError>;
    async fn list_data_query_comments(
        &self,
        query_id: Uuid,
    ) -> Result<Vec<DataQueryComment>, ApiError>;

    async fn create_visit_schedule_template(
        &self,
        study_id: Uuid,
        name: &str,
        day_offset: i32,
        window_before_days: i32,
        window_after_days: i32,
    ) -> Result<VisitScheduleTemplate, ApiError>;
    async fn list_visit_schedule_templates(
        &self,
        study_id: Uuid,
    ) -> Result<Vec<VisitScheduleTemplate>, ApiError>;

    async fn create_closeout_checklist_item(
        &self,
        study_id: Uuid,
        item_key: &str,
        item_label: &str,
        is_required: bool,
    ) -> Result<CloseoutChecklistItem, ApiError>;
    async fn list_closeout_checklist_items(
        &self,
        study_id: Uuid,
    ) -> Result<Vec<CloseoutChecklistItem>, ApiError>;
    async fn set_closeout_checklist_item_completion(
        &self,
        item_id: Uuid,
        is_complete: bool,
        completed_by_user_id: Option<Uuid>,
    ) -> Result<CloseoutChecklistItem, ApiError>;

    async fn create_dua(
        &self,
        organization_id: Uuid,
        counterparty: &str,
    ) -> Result<DuaAgreement, ApiError>;
    async fn get_dua(&self, dua_id: Uuid) -> Result<DuaAgreement, ApiError>;
    async fn list_duas(&self) -> Result<Vec<DuaAgreement>, ApiError>;
    async fn activate_dua(&self, dua_id: Uuid) -> Result<DuaAgreement, ApiError>;
    async fn create_dua_signature(
        &self,
        dua_id: Uuid,
        signer_name: &str,
        signer_email: &str,
        signer_role: &str,
        signature_text: &str,
    ) -> Result<DuaSignature, ApiError>;
    async fn list_dua_signatures(&self, dua_id: Uuid) -> Result<Vec<DuaSignature>, ApiError>;

    async fn create_reminder_job(
        &self,
        organization_id: Uuid,
        study_id: Option<Uuid>,
        patient_id: Option<Uuid>,
        visit_id: Option<Uuid>,
        channel: &str,
        recipient: &str,
        message: &str,
        scheduled_for: chrono::DateTime<chrono::Utc>,
    ) -> Result<ReminderJob, ApiError>;
    async fn list_reminder_jobs(&self, limit: i64) -> Result<Vec<ReminderJob>, ApiError>;
    async fn process_due_reminder_jobs(&self, limit: i64) -> Result<Vec<ReminderJob>, ApiError>;
    async fn create_media_asset(
        &self,
        organization_id: Uuid,
        study_id: Option<Uuid>,
        patient_id: Option<Uuid>,
        category: &str,
        filename: &str,
        object_key: &str,
        content_type: &str,
        expected_byte_size: i64,
        upload_expires_at: chrono::DateTime<chrono::Utc>,
        created_by_user_id: Option<Uuid>,
    ) -> Result<MediaAsset, ApiError>;
    async fn get_media_asset(&self, asset_id: Uuid) -> Result<MediaAsset, ApiError>;
    async fn list_media_assets_for_organization(
        &self,
        organization_id: Uuid,
        limit: i64,
    ) -> Result<Vec<MediaAsset>, ApiError>;
    async fn mark_media_asset_uploaded(
        &self,
        asset_id: Uuid,
        byte_size: i64,
    ) -> Result<MediaAsset, ApiError>;
    async fn get_organization_media_policy(
        &self,
        organization_id: Uuid,
    ) -> Result<Option<OrganizationMediaPolicy>, ApiError>;
    async fn upsert_organization_media_policy(
        &self,
        organization_id: Uuid,
        max_total_bytes: Option<i64>,
        max_asset_bytes: Option<i64>,
    ) -> Result<OrganizationMediaPolicy, ApiError>;
    async fn get_organization_media_usage(
        &self,
        organization_id: Uuid,
    ) -> Result<OrganizationMediaUsage, ApiError>;

    async fn compute_readiness(&self, study_id: Uuid) -> Result<StudyReadiness, ApiError>;

    async fn upsert_user(
        &self,
        subject: &str,
        email: Option<&str>,
        display_name: Option<&str>,
        platform_role: AppRole,
    ) -> Result<User, ApiError>;
    async fn upsert_organization_membership(
        &self,
        user_id: Uuid,
        organization_id: Uuid,
        role: AppRole,
    ) -> Result<OrganizationMembership, ApiError>;
    async fn list_organization_memberships(
        &self,
        organization_id: Uuid,
    ) -> Result<Vec<OrganizationMembership>, ApiError>;
    async fn get_membership_role(
        &self,
        user_id: Uuid,
        organization_id: Uuid,
    ) -> Result<Option<AppRole>, ApiError>;
    async fn insert_audit_log(&self, entry: NewAuditLogEntry) -> Result<(), ApiError>;
    async fn list_recent_audit_logs(&self, limit: i64) -> Result<Vec<AuditLogRecord>, ApiError>;
}

#[derive(Clone)]
pub struct PgRepository {
    pool: DbPool,
}

impl PgRepository {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    async fn client(&self) -> Result<deadpool_postgres::Client, ApiError> {
        self.pool
            .get()
            .await
            .map_err(|err| ApiError::Internal(format!("db connection error: {err}")))
    }
}

#[async_trait]
impl Repository for PgRepository {
    async fn create_organization(
        &self,
        name: &str,
        workspace_slug: &str,
    ) -> Result<Organization, ApiError> {
        let client = self.client().await?;
        let row = client
            .query_one(
                "INSERT INTO organizations(id, name, workspace_slug, created_at)
                 VALUES ($1, $2, $3, NOW())
                 RETURNING id, name, workspace_slug, created_at",
                &[&Uuid::new_v4(), &name, &workspace_slug],
            )
            .await
            .map_err(map_write_err)?;
        map_organization(&row)
    }

    async fn list_organizations(&self) -> Result<Vec<Organization>, ApiError> {
        let client = self.client().await?;
        let rows = client
            .query(
                "SELECT id, name, workspace_slug, created_at FROM organizations ORDER BY name",
                &[],
            )
            .await
            .map_err(map_query_err)?;
        rows.iter().map(map_organization).collect()
    }

    async fn get_organization(&self, organization_id: Uuid) -> Result<Organization, ApiError> {
        let client = self.client().await?;
        let row = client
            .query_opt(
                "SELECT id, name, workspace_slug, created_at
                 FROM organizations
                 WHERE id = $1",
                &[&organization_id],
            )
            .await
            .map_err(map_query_err)?
            .ok_or_else(|| ApiError::NotFound(format!("organization {organization_id}")))?;
        map_organization(&row)
    }

    async fn create_study(
        &self,
        organization_id: Uuid,
        short_code: &str,
        title: &str,
    ) -> Result<Study, ApiError> {
        let client = self.client().await?;
        let row = client
            .query_one(
                "INSERT INTO studies(id, organization_id, short_code, title, phase, created_at)
                 VALUES ($1, $2, $3, $4, $5, NOW())
                 RETURNING id, organization_id, short_code, title, phase, created_at",
                &[
                    &Uuid::new_v4(),
                    &organization_id,
                    &short_code,
                    &title,
                    &StudyPhase::PreStudy.as_db(),
                ],
            )
            .await
            .map_err(map_write_err)?;
        map_study(&row)
    }

    async fn list_studies(&self) -> Result<Vec<Study>, ApiError> {
        let client = self.client().await?;
        let rows = client
            .query(
                "SELECT id, organization_id, short_code, title, phase, created_at FROM studies ORDER BY short_code",
                &[],
            )
            .await
            .map_err(map_query_err)?;
        rows.iter().map(map_study).collect()
    }

    async fn get_study(&self, study_id: Uuid) -> Result<Study, ApiError> {
        let client = self.client().await?;
        let row = client
            .query_opt(
                "SELECT id, organization_id, short_code, title, phase, created_at FROM studies WHERE id = $1",
                &[&study_id],
            )
            .await
            .map_err(map_query_err)?
            .ok_or_else(|| ApiError::NotFound(format!("study {study_id}")))?;
        map_study(&row)
    }

    async fn update_study_phase(
        &self,
        study_id: Uuid,
        phase: StudyPhase,
    ) -> Result<Study, ApiError> {
        let client = self.client().await?;
        let row = client
            .query_opt(
                "UPDATE studies SET phase = $2
                 WHERE id = $1
                 RETURNING id, organization_id, short_code, title, phase, created_at",
                &[&study_id, &phase.as_db()],
            )
            .await
            .map_err(map_write_err)?
            .ok_or_else(|| ApiError::NotFound(format!("study {study_id}")))?;
        map_study(&row)
    }

    async fn create_site(
        &self,
        organization_id: Uuid,
        study_id: Option<Uuid>,
        name: &str,
        principal_investigator: &str,
    ) -> Result<Site, ApiError> {
        let client = self.client().await?;
        let row = client
            .query_one(
                "INSERT INTO sites(id, organization_id, study_id, name, principal_investigator, startup_complete, created_at)
                 VALUES ($1, $2, $3, $4, $5, FALSE, NOW())
                 RETURNING id, organization_id, study_id, name, principal_investigator, startup_complete, created_at",
                &[&Uuid::new_v4(), &organization_id, &study_id, &name, &principal_investigator],
            )
            .await
            .map_err(map_write_err)?;
        map_site(&row)
    }

    async fn list_sites(&self) -> Result<Vec<Site>, ApiError> {
        let client = self.client().await?;
        let rows = client
            .query(
                "SELECT id, organization_id, study_id, name, principal_investigator, startup_complete, created_at FROM sites ORDER BY name",
                &[],
            )
            .await
            .map_err(map_query_err)?;
        rows.iter().map(map_site).collect()
    }

    async fn get_site(&self, site_id: Uuid) -> Result<Site, ApiError> {
        let client = self.client().await?;
        let row = client
            .query_opt(
                "SELECT id, organization_id, study_id, name, principal_investigator, startup_complete, created_at FROM sites WHERE id = $1",
                &[&site_id],
            )
            .await
            .map_err(map_query_err)?
            .ok_or_else(|| ApiError::NotFound(format!("site {site_id}")))?;
        map_site(&row)
    }

    async fn set_site_startup(
        &self,
        site_id: Uuid,
        startup_complete: bool,
    ) -> Result<Site, ApiError> {
        let client = self.client().await?;
        let row = client
            .query_opt(
                "UPDATE sites SET startup_complete = $2
                 WHERE id = $1
                 RETURNING id, organization_id, study_id, name, principal_investigator, startup_complete, created_at",
                &[&site_id, &startup_complete],
            )
            .await
            .map_err(map_write_err)?
            .ok_or_else(|| ApiError::NotFound(format!("site {site_id}")))?;
        map_site(&row)
    }

    async fn create_patient(
        &self,
        organization_id: Uuid,
        study_id: Uuid,
        site_id: Option<Uuid>,
        external_id: &str,
    ) -> Result<Patient, ApiError> {
        let client = self.client().await?;
        let row = client
            .query_one(
                "INSERT INTO patients(id, organization_id, study_id, site_id, external_id, enrolled_at)
                 VALUES ($1, $2, $3, $4, $5, NOW())
                 RETURNING id, organization_id, study_id, site_id, external_id, enrolled_at",
                &[&Uuid::new_v4(), &organization_id, &study_id, &site_id, &external_id],
            )
            .await
            .map_err(map_write_err)?;
        map_patient(&row)
    }

    async fn list_patients(&self) -> Result<Vec<Patient>, ApiError> {
        let client = self.client().await?;
        let rows = client
            .query(
                "SELECT id, organization_id, study_id, site_id, external_id, enrolled_at FROM patients ORDER BY external_id",
                &[],
            )
            .await
            .map_err(map_query_err)?;
        rows.iter().map(map_patient).collect()
    }

    async fn get_patient(&self, patient_id: Uuid) -> Result<Patient, ApiError> {
        let client = self.client().await?;
        let row = client
            .query_opt(
                "SELECT id, organization_id, study_id, site_id, external_id, enrolled_at FROM patients WHERE id = $1",
                &[&patient_id],
            )
            .await
            .map_err(map_query_err)?
            .ok_or_else(|| ApiError::NotFound(format!("patient {patient_id}")))?;
        map_patient(&row)
    }

    async fn create_visit(
        &self,
        study_id: Uuid,
        patient_id: Uuid,
        visit_name: &str,
        scheduled_for: chrono::DateTime<chrono::Utc>,
    ) -> Result<Visit, ApiError> {
        let client = self.client().await?;
        let row = client
            .query_one(
                "INSERT INTO visits(id, study_id, patient_id, visit_name, scheduled_for, status)
                 VALUES ($1, $2, $3, $4, $5, $6)
                 RETURNING id, study_id, patient_id, visit_name, scheduled_for, status",
                &[
                    &Uuid::new_v4(),
                    &study_id,
                    &patient_id,
                    &visit_name,
                    &scheduled_for,
                    &VisitStatus::Planned.as_db(),
                ],
            )
            .await
            .map_err(map_write_err)?;
        map_visit(&row)
    }

    async fn list_visits(&self) -> Result<Vec<Visit>, ApiError> {
        let client = self.client().await?;
        let rows = client
            .query(
                "SELECT id, study_id, patient_id, visit_name, scheduled_for, status FROM visits ORDER BY scheduled_for",
                &[],
            )
            .await
            .map_err(map_query_err)?;
        rows.iter().map(map_visit).collect()
    }

    async fn get_visit(&self, visit_id: Uuid) -> Result<Visit, ApiError> {
        let client = self.client().await?;
        let row = client
            .query_opt(
                "SELECT id, study_id, patient_id, visit_name, scheduled_for, status FROM visits WHERE id = $1",
                &[&visit_id],
            )
            .await
            .map_err(map_query_err)?
            .ok_or_else(|| ApiError::NotFound(format!("visit {visit_id}")))?;
        map_visit(&row)
    }

    async fn create_crf_template(
        &self,
        study_id: Uuid,
        name: &str,
    ) -> Result<CrfTemplate, ApiError> {
        let client = self.client().await?;
        let row = client
            .query_one(
                "INSERT INTO crf_templates(id, study_id, name, published, created_at)
                 VALUES ($1, $2, $3, FALSE, NOW())
                 RETURNING id, study_id, name, published, created_at",
                &[&Uuid::new_v4(), &study_id, &name],
            )
            .await
            .map_err(map_write_err)?;
        map_template(&row)
    }

    async fn list_crf_templates(&self) -> Result<Vec<CrfTemplate>, ApiError> {
        let client = self.client().await?;
        let rows = client
            .query(
                "SELECT id, study_id, name, published, created_at FROM crf_templates ORDER BY name",
                &[],
            )
            .await
            .map_err(map_query_err)?;
        rows.iter().map(map_template).collect()
    }

    async fn get_crf_template(&self, template_id: Uuid) -> Result<CrfTemplate, ApiError> {
        let client = self.client().await?;
        let row = client
            .query_opt(
                "SELECT id, study_id, name, published, created_at FROM crf_templates WHERE id = $1",
                &[&template_id],
            )
            .await
            .map_err(map_query_err)?
            .ok_or_else(|| ApiError::NotFound(format!("template {template_id}")))?;
        map_template(&row)
    }

    async fn publish_crf_template(&self, template_id: Uuid) -> Result<CrfTemplate, ApiError> {
        let client = self.client().await?;
        let row = client
            .query_opt(
                "UPDATE crf_templates SET published = TRUE
                 WHERE id = $1
                 RETURNING id, study_id, name, published, created_at",
                &[&template_id],
            )
            .await
            .map_err(map_write_err)?
            .ok_or_else(|| ApiError::NotFound(format!("template {template_id}")))?;
        map_template(&row)
    }

    async fn create_crf_template_version(
        &self,
        template_id: Uuid,
        schema_json: &serde_json::Value,
    ) -> Result<CrfTemplateVersion, ApiError> {
        let client = self.client().await?;
        let row = client
            .query_one(
                "WITH next_version AS (
                    SELECT COALESCE(MAX(version_number), 0) + 1 AS version_number
                    FROM crf_template_versions
                    WHERE template_id = $1
                )
                INSERT INTO crf_template_versions(
                    id,
                    template_id,
                    version_number,
                    schema_json,
                    is_published,
                    created_at
                )
                SELECT
                    $2,
                    $1,
                    next_version.version_number,
                    $3,
                    FALSE,
                    NOW()
                FROM next_version
                RETURNING id, template_id, version_number, schema_json, is_published, created_at",
                &[&template_id, &Uuid::new_v4(), &schema_json],
            )
            .await
            .map_err(map_write_err)?;
        map_template_version(&row)
    }

    async fn list_crf_template_versions(
        &self,
        template_id: Uuid,
    ) -> Result<Vec<CrfTemplateVersion>, ApiError> {
        let client = self.client().await?;
        let rows = client
            .query(
                "SELECT id, template_id, version_number, schema_json, is_published, created_at
                 FROM crf_template_versions
                 WHERE template_id = $1
                 ORDER BY version_number DESC",
                &[&template_id],
            )
            .await
            .map_err(map_query_err)?;
        rows.iter().map(map_template_version).collect()
    }

    async fn publish_crf_template_version(
        &self,
        version_id: Uuid,
    ) -> Result<CrfTemplateVersion, ApiError> {
        let mut client = self.client().await?;
        let transaction = client
            .transaction()
            .await
            .map_err(|err| ApiError::Internal(format!("failed opening transaction: {err}")))?;
        let version_row = transaction
            .query_opt(
                "SELECT template_id
                 FROM crf_template_versions
                 WHERE id = $1",
                &[&version_id],
            )
            .await
            .map_err(map_write_err)?
            .ok_or_else(|| ApiError::NotFound(format!("template version {version_id}")))?;
        let template_id: Uuid = version_row.get("template_id");
        transaction
            .execute(
                "UPDATE crf_template_versions
                 SET is_published = FALSE
                 WHERE template_id = $1",
                &[&template_id],
            )
            .await
            .map_err(map_write_err)?;
        let row = transaction
            .query_opt(
                "UPDATE crf_template_versions
                 SET is_published = TRUE
                 WHERE id = $1
                 RETURNING id, template_id, version_number, schema_json, is_published, created_at",
                &[&version_id],
            )
            .await
            .map_err(map_write_err)?
            .ok_or_else(|| ApiError::NotFound(format!("template version {version_id}")))?;
        transaction
            .commit()
            .await
            .map_err(|err| ApiError::Internal(format!("failed committing transaction: {err}")))?;
        map_template_version(&row)
    }

    async fn create_crf_submission(
        &self,
        study_id: Uuid,
        patient_id: Uuid,
        visit_id: Uuid,
        template_id: Uuid,
    ) -> Result<CrfSubmission, ApiError> {
        let client = self.client().await?;
        let row = client
            .query_one(
                "INSERT INTO crf_submissions(id, study_id, patient_id, visit_id, template_id, status, captured_at)
                 VALUES ($1, $2, $3, $4, $5, $6, NOW())
                 RETURNING id, study_id, patient_id, visit_id, template_id, status, captured_at",
                &[&Uuid::new_v4(), &study_id, &patient_id, &visit_id, &template_id, &SubmissionStatus::Submitted.as_db()],
            )
            .await
            .map_err(map_write_err)?;
        map_submission(&row)
    }

    async fn list_crf_submissions(&self) -> Result<Vec<CrfSubmission>, ApiError> {
        let client = self.client().await?;
        let rows = client
            .query(
                "SELECT id, study_id, patient_id, visit_id, template_id, status, captured_at FROM crf_submissions ORDER BY captured_at",
                &[],
            )
            .await
            .map_err(map_query_err)?;
        rows.iter().map(map_submission).collect()
    }

    async fn get_crf_submission(&self, submission_id: Uuid) -> Result<CrfSubmission, ApiError> {
        let client = self.client().await?;
        let row = client
            .query_opt(
                "SELECT id, study_id, patient_id, visit_id, template_id, status, captured_at FROM crf_submissions WHERE id = $1",
                &[&submission_id],
            )
            .await
            .map_err(map_query_err)?
            .ok_or_else(|| ApiError::NotFound(format!("submission {submission_id}")))?;
        map_submission(&row)
    }

    async fn lock_crf_submission(&self, submission_id: Uuid) -> Result<CrfSubmission, ApiError> {
        let client = self.client().await?;
        let row = client
            .query_opt(
                "UPDATE crf_submissions SET status = $2
                 WHERE id = $1
                 RETURNING id, study_id, patient_id, visit_id, template_id, status, captured_at",
                &[&submission_id, &SubmissionStatus::Locked.as_db()],
            )
            .await
            .map_err(map_write_err)?
            .ok_or_else(|| ApiError::NotFound(format!("submission {submission_id}")))?;
        map_submission(&row)
    }

    async fn create_data_query(
        &self,
        study_id: Uuid,
        submission_id: Uuid,
        summary: &str,
    ) -> Result<DataQuery, ApiError> {
        let client = self.client().await?;
        let row = client
            .query_one(
                "INSERT INTO data_queries(id, study_id, submission_id, summary, status, raised_at)
                 VALUES ($1, $2, $3, $4, $5, NOW())
                 RETURNING id, study_id, submission_id, summary, status, raised_at",
                &[
                    &Uuid::new_v4(),
                    &study_id,
                    &submission_id,
                    &summary,
                    &QueryStatus::Open.as_db(),
                ],
            )
            .await
            .map_err(map_write_err)?;
        map_data_query(&row)
    }

    async fn list_data_queries(&self) -> Result<Vec<DataQuery>, ApiError> {
        let client = self.client().await?;
        let rows = client
            .query(
                "SELECT id, study_id, submission_id, summary, status, raised_at FROM data_queries ORDER BY raised_at",
                &[],
            )
            .await
            .map_err(map_query_err)?;
        rows.iter().map(map_data_query).collect()
    }

    async fn respond_data_query(&self, query_id: Uuid) -> Result<DataQuery, ApiError> {
        let client = self.client().await?;
        let row = client
            .query_opt(
                "UPDATE data_queries SET status = $2
                 WHERE id = $1 AND status <> 'closed'
                 RETURNING id, study_id, submission_id, summary, status, raised_at",
                &[&query_id, &QueryStatus::Responded.as_db()],
            )
            .await
            .map_err(map_write_err)?
            .ok_or_else(|| ApiError::NotFound(format!("query {query_id}")))?;
        map_data_query(&row)
    }

    async fn close_data_query(&self, query_id: Uuid) -> Result<DataQuery, ApiError> {
        let client = self.client().await?;
        let existing = client
            .query_opt(
                "SELECT status
                 FROM data_queries
                 WHERE id = $1",
                &[&query_id],
            )
            .await
            .map_err(map_query_err)?
            .ok_or_else(|| ApiError::NotFound(format!("query {query_id}")))?;
        let status = QueryStatus::from_str(existing.get::<_, &str>("status")).map_err(|err| {
            ApiError::Internal(format!("invalid query status in database: {err}"))
        })?;
        if status == QueryStatus::Open {
            return Err(ApiError::Conflict(
                "query must be responded before closure".to_string(),
            ));
        }
        let row = client
            .query_opt(
                "UPDATE data_queries SET status = $2
                 WHERE id = $1 AND status <> 'closed'
                 RETURNING id, study_id, submission_id, summary, status, raised_at",
                &[&query_id, &QueryStatus::Closed.as_db()],
            )
            .await
            .map_err(map_write_err)?
            .ok_or_else(|| ApiError::NotFound(format!("query {query_id}")))?;
        map_data_query(&row)
    }

    async fn create_data_query_comment(
        &self,
        query_id: Uuid,
        author_user_id: Option<Uuid>,
        comment_text: &str,
    ) -> Result<DataQueryComment, ApiError> {
        let client = self.client().await?;
        let row = client
            .query_one(
                "INSERT INTO data_query_comments(id, query_id, author_user_id, comment_text, created_at)
                 VALUES ($1, $2, $3, $4, NOW())
                 RETURNING id, query_id, author_user_id, comment_text, created_at",
                &[&Uuid::new_v4(), &query_id, &author_user_id, &comment_text],
            )
            .await
            .map_err(map_write_err)?;
        map_data_query_comment(&row)
    }

    async fn list_data_query_comments(
        &self,
        query_id: Uuid,
    ) -> Result<Vec<DataQueryComment>, ApiError> {
        let client = self.client().await?;
        let rows = client
            .query(
                "SELECT id, query_id, author_user_id, comment_text, created_at
                 FROM data_query_comments
                 WHERE query_id = $1
                 ORDER BY created_at ASC",
                &[&query_id],
            )
            .await
            .map_err(map_query_err)?;
        rows.iter().map(map_data_query_comment).collect()
    }

    async fn create_visit_schedule_template(
        &self,
        study_id: Uuid,
        name: &str,
        day_offset: i32,
        window_before_days: i32,
        window_after_days: i32,
    ) -> Result<VisitScheduleTemplate, ApiError> {
        let client = self.client().await?;
        let row = client
            .query_one(
                "INSERT INTO visit_schedule_templates(
                    id,
                    study_id,
                    name,
                    day_offset,
                    window_before_days,
                    window_after_days,
                    created_at
                ) VALUES ($1, $2, $3, $4, $5, $6, NOW())
                RETURNING id, study_id, name, day_offset, window_before_days, window_after_days, created_at",
                &[
                    &Uuid::new_v4(),
                    &study_id,
                    &name,
                    &day_offset,
                    &window_before_days,
                    &window_after_days,
                ],
            )
            .await
            .map_err(map_write_err)?;
        map_visit_schedule_template(&row)
    }

    async fn list_visit_schedule_templates(
        &self,
        study_id: Uuid,
    ) -> Result<Vec<VisitScheduleTemplate>, ApiError> {
        let client = self.client().await?;
        let rows = client
            .query(
                "SELECT id, study_id, name, day_offset, window_before_days, window_after_days, created_at
                 FROM visit_schedule_templates
                 WHERE study_id = $1
                 ORDER BY day_offset ASC, created_at ASC",
                &[&study_id],
            )
            .await
            .map_err(map_query_err)?;
        rows.iter().map(map_visit_schedule_template).collect()
    }

    async fn create_closeout_checklist_item(
        &self,
        study_id: Uuid,
        item_key: &str,
        item_label: &str,
        is_required: bool,
    ) -> Result<CloseoutChecklistItem, ApiError> {
        let client = self.client().await?;
        let row = client
            .query_one(
                "INSERT INTO study_closeout_checklist_items(
                    id,
                    study_id,
                    item_key,
                    item_label,
                    is_required,
                    is_complete,
                    completed_at,
                    completed_by_user_id,
                    created_at
                ) VALUES ($1, $2, $3, $4, $5, FALSE, NULL, NULL, NOW())
                RETURNING id, study_id, item_key, item_label, is_required, is_complete, completed_at, completed_by_user_id, created_at",
                &[&Uuid::new_v4(), &study_id, &item_key, &item_label, &is_required],
            )
            .await
            .map_err(map_write_err)?;
        map_closeout_checklist_item(&row)
    }

    async fn list_closeout_checklist_items(
        &self,
        study_id: Uuid,
    ) -> Result<Vec<CloseoutChecklistItem>, ApiError> {
        let client = self.client().await?;
        let rows = client
            .query(
                "SELECT id, study_id, item_key, item_label, is_required, is_complete, completed_at, completed_by_user_id, created_at
                 FROM study_closeout_checklist_items
                 WHERE study_id = $1
                 ORDER BY created_at ASC",
                &[&study_id],
            )
            .await
            .map_err(map_query_err)?;
        rows.iter().map(map_closeout_checklist_item).collect()
    }

    async fn set_closeout_checklist_item_completion(
        &self,
        item_id: Uuid,
        is_complete: bool,
        completed_by_user_id: Option<Uuid>,
    ) -> Result<CloseoutChecklistItem, ApiError> {
        let client = self.client().await?;
        let row = client
            .query_opt(
                "UPDATE study_closeout_checklist_items
                 SET
                    is_complete = $2,
                    completed_at = CASE WHEN $2 THEN NOW() ELSE NULL::timestamptz END,
                    completed_by_user_id = CASE WHEN $2 THEN CAST($3 AS UUID) ELSE NULL::UUID END
                 WHERE id = $1
                 RETURNING id, study_id, item_key, item_label, is_required, is_complete, completed_at, completed_by_user_id, created_at",
                &[&item_id, &is_complete, &completed_by_user_id],
            )
            .await
            .map_err(map_write_err)?
            .ok_or_else(|| ApiError::NotFound(format!("closeout checklist item {item_id}")))?;
        map_closeout_checklist_item(&row)
    }

    async fn create_dua(
        &self,
        organization_id: Uuid,
        counterparty: &str,
    ) -> Result<DuaAgreement, ApiError> {
        let client = self.client().await?;
        let row = client
            .query_one(
                "INSERT INTO dua_agreements(id, organization_id, counterparty, status, created_at)
                 VALUES ($1, $2, $3, $4, NOW())
                 RETURNING id, organization_id, counterparty, status, created_at",
                &[
                    &Uuid::new_v4(),
                    &organization_id,
                    &counterparty,
                    &AgreementStatus::PendingSignatures.as_db(),
                ],
            )
            .await
            .map_err(map_write_err)?;
        map_dua(&row)
    }

    async fn get_dua(&self, dua_id: Uuid) -> Result<DuaAgreement, ApiError> {
        let client = self.client().await?;
        let row = client
            .query_opt(
                "SELECT id, organization_id, counterparty, status, created_at
                 FROM dua_agreements
                 WHERE id = $1",
                &[&dua_id],
            )
            .await
            .map_err(map_query_err)?
            .ok_or_else(|| ApiError::NotFound(format!("dua {dua_id}")))?;
        map_dua(&row)
    }

    async fn list_duas(&self) -> Result<Vec<DuaAgreement>, ApiError> {
        let client = self.client().await?;
        let rows = client
            .query(
                "SELECT id, organization_id, counterparty, status, created_at FROM dua_agreements ORDER BY created_at",
                &[],
            )
            .await
            .map_err(map_query_err)?;
        rows.iter().map(map_dua).collect()
    }

    async fn activate_dua(&self, dua_id: Uuid) -> Result<DuaAgreement, ApiError> {
        let client = self.client().await?;
        let row = client
            .query_opt(
                "UPDATE dua_agreements SET status = $2
                 WHERE id = $1
                 RETURNING id, organization_id, counterparty, status, created_at",
                &[&dua_id, &AgreementStatus::Active.as_db()],
            )
            .await
            .map_err(map_write_err)?
            .ok_or_else(|| ApiError::NotFound(format!("dua {dua_id}")))?;
        map_dua(&row)
    }

    async fn create_dua_signature(
        &self,
        dua_id: Uuid,
        signer_name: &str,
        signer_email: &str,
        signer_role: &str,
        signature_text: &str,
    ) -> Result<DuaSignature, ApiError> {
        let client = self.client().await?;
        let row = client
            .query_one(
                "INSERT INTO dua_signatures(
                    id,
                    dua_agreement_id,
                    signer_name,
                    signer_email,
                    signer_role,
                    signature_text,
                    signed_at
                ) VALUES ($1, $2, $3, $4, $5, $6, NOW())
                RETURNING id, dua_agreement_id, signer_name, signer_email, signer_role, signature_text, signed_at",
                &[
                    &Uuid::new_v4(),
                    &dua_id,
                    &signer_name,
                    &signer_email,
                    &signer_role,
                    &signature_text,
                ],
            )
            .await
            .map_err(map_write_err)?;
        map_dua_signature(&row)
    }

    async fn list_dua_signatures(&self, dua_id: Uuid) -> Result<Vec<DuaSignature>, ApiError> {
        let client = self.client().await?;
        let rows = client
            .query(
                "SELECT id, dua_agreement_id, signer_name, signer_email, signer_role, signature_text, signed_at
                 FROM dua_signatures
                 WHERE dua_agreement_id = $1
                 ORDER BY signed_at ASC",
                &[&dua_id],
            )
            .await
            .map_err(map_query_err)?;
        rows.iter().map(map_dua_signature).collect()
    }

    async fn create_reminder_job(
        &self,
        organization_id: Uuid,
        study_id: Option<Uuid>,
        patient_id: Option<Uuid>,
        visit_id: Option<Uuid>,
        channel: &str,
        recipient: &str,
        message: &str,
        scheduled_for: chrono::DateTime<chrono::Utc>,
    ) -> Result<ReminderJob, ApiError> {
        let client = self.client().await?;
        let row = client
            .query_one(
                "INSERT INTO reminder_jobs(
                    id,
                    organization_id,
                    study_id,
                    patient_id,
                    visit_id,
                    channel,
                    recipient,
                    message,
                    status,
                    scheduled_for,
                    processed_at,
                    last_error,
                    created_at
                ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, NULL, NULL, NOW())
                RETURNING
                    id,
                    organization_id,
                    study_id,
                    patient_id,
                    visit_id,
                    channel,
                    recipient,
                    message,
                    status,
                    scheduled_for,
                    processed_at,
                    last_error,
                    created_at",
                &[
                    &Uuid::new_v4(),
                    &organization_id,
                    &study_id,
                    &patient_id,
                    &visit_id,
                    &channel,
                    &recipient,
                    &message,
                    &ReminderJobStatus::Pending.as_db(),
                    &scheduled_for,
                ],
            )
            .await
            .map_err(map_write_err)?;
        map_reminder_job(&row)
    }

    async fn list_reminder_jobs(&self, limit: i64) -> Result<Vec<ReminderJob>, ApiError> {
        let client = self.client().await?;
        let safe_limit = if limit <= 0 { 20 } else { limit.min(500) };
        let rows = client
            .query(
                "SELECT
                    id,
                    organization_id,
                    study_id,
                    patient_id,
                    visit_id,
                    channel,
                    recipient,
                    message,
                    status,
                    scheduled_for,
                    processed_at,
                    last_error,
                    created_at
                 FROM reminder_jobs
                 ORDER BY scheduled_for ASC
                 LIMIT $1",
                &[&safe_limit],
            )
            .await
            .map_err(map_query_err)?;
        rows.iter().map(map_reminder_job).collect()
    }

    async fn process_due_reminder_jobs(&self, limit: i64) -> Result<Vec<ReminderJob>, ApiError> {
        let client = self.client().await?;
        let safe_limit = if limit <= 0 { 20 } else { limit.min(500) };
        let rows = client
            .query(
                "WITH due AS (
                    SELECT id
                    FROM reminder_jobs
                    WHERE status = 'pending'
                      AND scheduled_for <= NOW()
                    ORDER BY scheduled_for ASC
                    LIMIT $1
                )
                UPDATE reminder_jobs r
                SET
                    status = 'sent',
                    processed_at = NOW(),
                    last_error = NULL
                WHERE r.id IN (SELECT id FROM due)
                RETURNING
                    r.id,
                    r.organization_id,
                    r.study_id,
                    r.patient_id,
                    r.visit_id,
                    r.channel,
                    r.recipient,
                    r.message,
                    r.status,
                    r.scheduled_for,
                    r.processed_at,
                    r.last_error,
                    r.created_at",
                &[&safe_limit],
            )
            .await
            .map_err(map_write_err)?;
        rows.iter().map(map_reminder_job).collect()
    }

    async fn create_media_asset(
        &self,
        organization_id: Uuid,
        study_id: Option<Uuid>,
        patient_id: Option<Uuid>,
        category: &str,
        filename: &str,
        object_key: &str,
        content_type: &str,
        expected_byte_size: i64,
        upload_expires_at: chrono::DateTime<chrono::Utc>,
        created_by_user_id: Option<Uuid>,
    ) -> Result<MediaAsset, ApiError> {
        let client = self.client().await?;
        let row = client
            .query_one(
                "INSERT INTO media_assets(
                    id,
                    organization_id,
                    study_id,
                    patient_id,
                    category,
                    filename,
                    object_key,
                    content_type,
                    expected_byte_size,
                    byte_size,
                    status,
                    upload_expires_at,
                    uploaded_at,
                    created_by_user_id,
                    created_at
                ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 0, $10, $11, NULL, $12, NOW())
                RETURNING
                    id,
                    organization_id,
                    study_id,
                    patient_id,
                    category,
                    filename,
                    object_key,
                    content_type,
                    expected_byte_size,
                    byte_size,
                    status,
                    upload_expires_at,
                    uploaded_at,
                    created_by_user_id,
                    created_at",
                &[
                    &Uuid::new_v4(),
                    &organization_id,
                    &study_id,
                    &patient_id,
                    &category,
                    &filename,
                    &object_key,
                    &content_type,
                    &expected_byte_size,
                    &MediaAssetStatus::PendingUpload.as_db(),
                    &upload_expires_at,
                    &created_by_user_id,
                ],
            )
            .await
            .map_err(map_write_err)?;
        map_media_asset(&row)
    }

    async fn get_media_asset(&self, asset_id: Uuid) -> Result<MediaAsset, ApiError> {
        let client = self.client().await?;
        let row = client
            .query_opt(
                "SELECT
                    id,
                    organization_id,
                    study_id,
                    patient_id,
                    category,
                    filename,
                    object_key,
                    content_type,
                    expected_byte_size,
                    byte_size,
                    status,
                    upload_expires_at,
                    uploaded_at,
                    created_by_user_id,
                    created_at
                 FROM media_assets
                 WHERE id = $1",
                &[&asset_id],
            )
            .await
            .map_err(map_query_err)?
            .ok_or_else(|| ApiError::NotFound(format!("media asset {asset_id}")))?;
        map_media_asset(&row)
    }

    async fn list_media_assets_for_organization(
        &self,
        organization_id: Uuid,
        limit: i64,
    ) -> Result<Vec<MediaAsset>, ApiError> {
        let client = self.client().await?;
        let safe_limit = if limit <= 0 { 20 } else { limit.min(500) };
        let rows = client
            .query(
                "SELECT
                    id,
                    organization_id,
                    study_id,
                    patient_id,
                    category,
                    filename,
                    object_key,
                    content_type,
                    expected_byte_size,
                    byte_size,
                    status,
                    upload_expires_at,
                    uploaded_at,
                    created_by_user_id,
                    created_at
                 FROM media_assets
                 WHERE organization_id = $1
                 ORDER BY created_at DESC
                 LIMIT $2",
                &[&organization_id, &safe_limit],
            )
            .await
            .map_err(map_query_err)?;
        rows.iter().map(map_media_asset).collect()
    }

    async fn mark_media_asset_uploaded(
        &self,
        asset_id: Uuid,
        byte_size: i64,
    ) -> Result<MediaAsset, ApiError> {
        let client = self.client().await?;
        let row = client
            .query_opt(
                "UPDATE media_assets
                 SET
                    status = $2,
                    byte_size = $3,
                    uploaded_at = NOW()
                 WHERE id = $1
                 RETURNING
                    id,
                    organization_id,
                    study_id,
                    patient_id,
                    category,
                    filename,
                    object_key,
                    content_type,
                    expected_byte_size,
                    byte_size,
                    status,
                    upload_expires_at,
                    uploaded_at,
                    created_by_user_id,
                    created_at",
                &[&asset_id, &MediaAssetStatus::Uploaded.as_db(), &byte_size],
            )
            .await
            .map_err(map_write_err)?
            .ok_or_else(|| ApiError::NotFound(format!("media asset {asset_id}")))?;
        map_media_asset(&row)
    }

    async fn get_organization_media_policy(
        &self,
        organization_id: Uuid,
    ) -> Result<Option<OrganizationMediaPolicy>, ApiError> {
        let client = self.client().await?;
        let row = client
            .query_opt(
                "SELECT organization_id, max_total_bytes, max_asset_bytes, updated_at
                 FROM organization_media_policies
                 WHERE organization_id = $1",
                &[&organization_id],
            )
            .await
            .map_err(map_query_err)?;
        row.map(|row| map_organization_media_policy(&row))
            .transpose()
    }

    async fn upsert_organization_media_policy(
        &self,
        organization_id: Uuid,
        max_total_bytes: Option<i64>,
        max_asset_bytes: Option<i64>,
    ) -> Result<OrganizationMediaPolicy, ApiError> {
        let client = self.client().await?;
        let row = client
            .query_one(
                "INSERT INTO organization_media_policies(
                    organization_id,
                    max_total_bytes,
                    max_asset_bytes,
                    updated_at
                ) VALUES ($1, $2, $3, NOW())
                ON CONFLICT(organization_id) DO UPDATE SET
                    max_total_bytes = EXCLUDED.max_total_bytes,
                    max_asset_bytes = EXCLUDED.max_asset_bytes,
                    updated_at = NOW()
                RETURNING organization_id, max_total_bytes, max_asset_bytes, updated_at",
                &[&organization_id, &max_total_bytes, &max_asset_bytes],
            )
            .await
            .map_err(map_write_err)?;
        map_organization_media_policy(&row)
    }

    async fn get_organization_media_usage(
        &self,
        organization_id: Uuid,
    ) -> Result<OrganizationMediaUsage, ApiError> {
        let client = self.client().await?;
        let row = client
            .query_one(
                "SELECT
                    COALESCE(SUM(byte_size) FILTER (WHERE status = 'uploaded'), 0)::BIGINT AS uploaded_bytes,
                    COALESCE(SUM(expected_byte_size) FILTER (WHERE status = 'pending_upload'), 0)::BIGINT AS pending_reserved_bytes,
                    COUNT(*) FILTER (WHERE status = 'uploaded') AS uploaded_asset_count,
                    COUNT(*) FILTER (WHERE status = 'pending_upload') AS pending_asset_count
                 FROM media_assets
                 WHERE organization_id = $1",
                &[&organization_id],
            )
            .await
            .map_err(map_query_err)?;
        Ok(OrganizationMediaUsage {
            organization_id,
            uploaded_bytes: row.get("uploaded_bytes"),
            pending_reserved_bytes: row.get("pending_reserved_bytes"),
            uploaded_asset_count: row.get("uploaded_asset_count"),
            pending_asset_count: row.get("pending_asset_count"),
        })
    }

    async fn compute_readiness(&self, study_id: Uuid) -> Result<StudyReadiness, ApiError> {
        let client = self.client().await?;
        let study_row = client
            .query_opt(
                "SELECT organization_id, phase FROM studies WHERE id = $1",
                &[&study_id],
            )
            .await
            .map_err(map_query_err)?
            .ok_or_else(|| ApiError::NotFound(format!("study {study_id}")))?;
        let organization_id: Uuid = study_row.get("organization_id");
        let phase = StudyPhase::from_str(study_row.get::<_, &str>("phase"))
            .map_err(|err| ApiError::Internal(format!("invalid phase in database: {err}")))?;

        let has_site: bool = client
            .query_one(
                "SELECT EXISTS(
                    SELECT 1 FROM sites WHERE study_id = $1 AND startup_complete = TRUE
                ) AS present",
                &[&study_id],
            )
            .await
            .map_err(map_query_err)?
            .get("present");
        let has_published_crf: bool = client
            .query_one(
                "SELECT EXISTS(
                    SELECT 1
                    FROM crf_templates t
                    WHERE t.study_id = $1
                      AND (
                        t.published = TRUE
                        OR EXISTS (
                            SELECT 1
                            FROM crf_template_versions v
                            WHERE v.template_id = t.id
                              AND v.is_published = TRUE
                        )
                      )
                ) AS present",
                &[&study_id],
            )
            .await
            .map_err(map_query_err)?
            .get("present");
        let has_enrolled_patient: bool = client
            .query_one(
                "SELECT EXISTS(
                    SELECT 1 FROM patients WHERE study_id = $1
                ) AS present",
                &[&study_id],
            )
            .await
            .map_err(map_query_err)?
            .get("present");
        let has_locked_submission: bool = client
            .query_one(
                "SELECT EXISTS(
                    SELECT 1 FROM crf_submissions WHERE study_id = $1 AND status = 'locked'
                ) AS present",
                &[&study_id],
            )
            .await
            .map_err(map_query_err)?
            .get("present");
        let open_query_count: i64 = client
            .query_one(
                "SELECT COUNT(*) AS count
                 FROM data_queries
                 WHERE study_id = $1 AND status <> 'closed'",
                &[&study_id],
            )
            .await
            .map_err(map_query_err)?
            .get("count");
        let pending_closeout_items: i64 = client
            .query_one(
                "SELECT COUNT(*) AS count
                 FROM study_closeout_checklist_items
                 WHERE study_id = $1
                   AND is_required = TRUE
                   AND is_complete = FALSE",
                &[&study_id],
            )
            .await
            .map_err(map_query_err)?
            .get("count");
        let has_active_dua: bool = client
            .query_one(
                "SELECT EXISTS(
                    SELECT 1 FROM dua_agreements WHERE organization_id = $1 AND status = 'active'
                ) AS present",
                &[&organization_id],
            )
            .await
            .map_err(map_query_err)?
            .get("present");

        let next_recommended_action = if !has_active_dua {
            "Activate a DUA for the study organization".to_string()
        } else if !has_site {
            "Mark at least one attached site as startup-complete".to_string()
        } else if !has_published_crf {
            "Create and publish at least one CRF template".to_string()
        } else if !has_enrolled_patient {
            "Enroll first patient to unlock active operations".to_string()
        } else if !has_locked_submission {
            "Capture and lock at least one CRF submission".to_string()
        } else if open_query_count > 0 {
            "Resolve all open data queries before closure".to_string()
        } else if pending_closeout_items > 0 {
            "Complete required closeout checklist items before closure".to_string()
        } else {
            "Study is ready for operational closeout".to_string()
        };

        Ok(StudyReadiness {
            study_id,
            phase,
            has_site,
            has_published_crf,
            has_enrolled_patient,
            has_locked_submission,
            open_query_count: open_query_count as usize,
            pending_closeout_items: pending_closeout_items as usize,
            has_active_dua,
            next_recommended_action,
        })
    }

    async fn upsert_user(
        &self,
        subject: &str,
        email: Option<&str>,
        display_name: Option<&str>,
        platform_role: AppRole,
    ) -> Result<User, ApiError> {
        let client = self.client().await?;
        let row = client
            .query_one(
                "INSERT INTO users(id, subject, email, display_name, platform_role, created_at)
                 VALUES ($1, $2, $3, $4, $5, NOW())
                 ON CONFLICT(subject) DO UPDATE SET
                     email = COALESCE(EXCLUDED.email, users.email),
                     display_name = COALESCE(EXCLUDED.display_name, users.display_name),
                     platform_role = EXCLUDED.platform_role
                 RETURNING id, subject, email, display_name, platform_role, created_at",
                &[
                    &Uuid::new_v4(),
                    &subject,
                    &email,
                    &display_name,
                    &platform_role.as_str(),
                ],
            )
            .await
            .map_err(map_write_err)?;
        map_user(&row)
    }

    async fn upsert_organization_membership(
        &self,
        user_id: Uuid,
        organization_id: Uuid,
        role: AppRole,
    ) -> Result<OrganizationMembership, ApiError> {
        let client = self.client().await?;
        let row = client
            .query_one(
                "INSERT INTO organization_memberships(id, user_id, organization_id, role, created_at)
                 VALUES ($1, $2, $3, $4, NOW())
                 ON CONFLICT(user_id, organization_id) DO UPDATE SET role = EXCLUDED.role
                 RETURNING id, user_id, organization_id, role, created_at",
                &[&Uuid::new_v4(), &user_id, &organization_id, &role.as_str()],
            )
            .await
            .map_err(map_write_err)?;
        map_membership(&row)
    }

    async fn list_organization_memberships(
        &self,
        organization_id: Uuid,
    ) -> Result<Vec<OrganizationMembership>, ApiError> {
        let client = self.client().await?;
        let rows = client
            .query(
                "SELECT id, user_id, organization_id, role, created_at
                 FROM organization_memberships
                 WHERE organization_id = $1
                 ORDER BY created_at DESC",
                &[&organization_id],
            )
            .await
            .map_err(map_query_err)?;
        rows.iter().map(map_membership).collect()
    }

    async fn get_membership_role(
        &self,
        user_id: Uuid,
        organization_id: Uuid,
    ) -> Result<Option<AppRole>, ApiError> {
        let client = self.client().await?;
        let row = client
            .query_opt(
                "SELECT role
                 FROM organization_memberships
                 WHERE user_id = $1 AND organization_id = $2",
                &[&user_id, &organization_id],
            )
            .await
            .map_err(map_query_err)?;
        row.map(|row| AppRole::from_str(row.get::<_, &str>("role")).map_err(ApiError::BadRequest))
            .transpose()
    }

    async fn insert_audit_log(&self, entry: NewAuditLogEntry) -> Result<(), ApiError> {
        let client = self.client().await?;
        client
            .execute(
                "INSERT INTO audit_logs(
                    id,
                    actor_user_id,
                    actor_subject,
                    actor_role,
                    actor_email,
                    auth_source,
                    method,
                    path,
                    action,
                    resource_type,
                    resource_id,
                    metadata_json,
                    status_code,
                    happened_at
                ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, NOW())",
                &[
                    &Uuid::new_v4(),
                    &entry.actor_user_id,
                    &entry.actor_subject,
                    &entry.actor_role.map(|role| role.as_str().to_string()),
                    &entry.actor_email,
                    &entry.auth_source,
                    &entry.method,
                    &entry.path,
                    &entry.action,
                    &entry.resource_type,
                    &entry.resource_id,
                    &entry.metadata_json,
                    &entry.status_code,
                ],
            )
            .await
            .map_err(map_write_err)?;
        Ok(())
    }

    async fn list_recent_audit_logs(&self, limit: i64) -> Result<Vec<AuditLogRecord>, ApiError> {
        let client = self.client().await?;
        let safe_limit = if limit <= 0 { 20 } else { limit.min(200) };
        let rows = client
            .query(
                "SELECT
                    id,
                    actor_user_id,
                    actor_subject,
                    actor_role,
                    actor_email,
                    auth_source,
                    method,
                    path,
                    action,
                    resource_type,
                    resource_id,
                    metadata_json,
                    status_code,
                    happened_at
                 FROM audit_logs
                 ORDER BY happened_at DESC
                 LIMIT $1",
                &[&safe_limit],
            )
            .await
            .map_err(map_query_err)?;
        rows.iter().map(map_audit_log).collect()
    }
}

fn map_write_err(err: tokio_postgres::Error) -> ApiError {
    if let Some(db_err) = err.as_db_error() {
        let code = db_err.code().code();
        return match code {
            "23505" => ApiError::Conflict(db_err.message().to_string()),
            "23503" => ApiError::BadRequest(db_err.message().to_string()),
            _ => ApiError::Internal(db_err.message().to_string()),
        };
    }
    ApiError::Internal(format!("database write error: {err}"))
}

fn map_query_err(err: tokio_postgres::Error) -> ApiError {
    ApiError::Internal(format!("database query error: {err}"))
}

fn map_organization(row: &Row) -> Result<Organization, ApiError> {
    Ok(Organization {
        id: row.get("id"),
        name: row.get("name"),
        workspace_slug: row.get("workspace_slug"),
        created_at: row.get("created_at"),
    })
}

fn map_study(row: &Row) -> Result<Study, ApiError> {
    let phase = StudyPhase::from_str(row.get("phase"))
        .map_err(|err| ApiError::Internal(format!("invalid study phase in database: {err}")))?;
    Ok(Study {
        id: row.get("id"),
        organization_id: row.get("organization_id"),
        short_code: row.get("short_code"),
        title: row.get("title"),
        phase,
        created_at: row.get("created_at"),
    })
}

fn map_site(row: &Row) -> Result<Site, ApiError> {
    Ok(Site {
        id: row.get("id"),
        organization_id: row.get("organization_id"),
        study_id: row.get("study_id"),
        name: row.get("name"),
        principal_investigator: row.get("principal_investigator"),
        startup_complete: row.get("startup_complete"),
        created_at: row.get("created_at"),
    })
}

fn map_patient(row: &Row) -> Result<Patient, ApiError> {
    Ok(Patient {
        id: row.get("id"),
        organization_id: row.get("organization_id"),
        study_id: row.get("study_id"),
        site_id: row.get("site_id"),
        external_id: row.get("external_id"),
        enrolled_at: row.get("enrolled_at"),
    })
}

fn map_visit(row: &Row) -> Result<Visit, ApiError> {
    let status = VisitStatus::from_str(row.get("status"))
        .map_err(|err| ApiError::Internal(format!("invalid visit status in database: {err}")))?;
    Ok(Visit {
        id: row.get("id"),
        study_id: row.get("study_id"),
        patient_id: row.get("patient_id"),
        visit_name: row.get("visit_name"),
        scheduled_for: row.get("scheduled_for"),
        status,
    })
}

fn map_template(row: &Row) -> Result<CrfTemplate, ApiError> {
    Ok(CrfTemplate {
        id: row.get("id"),
        study_id: row.get("study_id"),
        name: row.get("name"),
        published: row.get("published"),
        created_at: row.get("created_at"),
    })
}

fn map_template_version(row: &Row) -> Result<CrfTemplateVersion, ApiError> {
    Ok(CrfTemplateVersion {
        id: row.get("id"),
        template_id: row.get("template_id"),
        version_number: row.get("version_number"),
        schema_json: row.get("schema_json"),
        is_published: row.get("is_published"),
        created_at: row.get("created_at"),
    })
}

fn map_submission(row: &Row) -> Result<CrfSubmission, ApiError> {
    let status = SubmissionStatus::from_str(row.get("status")).map_err(|err| {
        ApiError::Internal(format!("invalid submission status in database: {err}"))
    })?;
    Ok(CrfSubmission {
        id: row.get("id"),
        study_id: row.get("study_id"),
        patient_id: row.get("patient_id"),
        visit_id: row.get("visit_id"),
        template_id: row.get("template_id"),
        status,
        captured_at: row.get("captured_at"),
    })
}

fn map_data_query(row: &Row) -> Result<DataQuery, ApiError> {
    let status = QueryStatus::from_str(row.get("status"))
        .map_err(|err| ApiError::Internal(format!("invalid query status in database: {err}")))?;
    Ok(DataQuery {
        id: row.get("id"),
        study_id: row.get("study_id"),
        submission_id: row.get("submission_id"),
        summary: row.get("summary"),
        status,
        raised_at: row.get("raised_at"),
    })
}

fn map_data_query_comment(row: &Row) -> Result<DataQueryComment, ApiError> {
    Ok(DataQueryComment {
        id: row.get("id"),
        query_id: row.get("query_id"),
        author_user_id: row.get("author_user_id"),
        comment_text: row.get("comment_text"),
        created_at: row.get("created_at"),
    })
}

fn map_visit_schedule_template(row: &Row) -> Result<VisitScheduleTemplate, ApiError> {
    Ok(VisitScheduleTemplate {
        id: row.get("id"),
        study_id: row.get("study_id"),
        name: row.get("name"),
        day_offset: row.get("day_offset"),
        window_before_days: row.get("window_before_days"),
        window_after_days: row.get("window_after_days"),
        created_at: row.get("created_at"),
    })
}

fn map_closeout_checklist_item(row: &Row) -> Result<CloseoutChecklistItem, ApiError> {
    Ok(CloseoutChecklistItem {
        id: row.get("id"),
        study_id: row.get("study_id"),
        item_key: row.get("item_key"),
        item_label: row.get("item_label"),
        is_required: row.get("is_required"),
        is_complete: row.get("is_complete"),
        completed_at: row.get("completed_at"),
        completed_by_user_id: row.get("completed_by_user_id"),
        created_at: row.get("created_at"),
    })
}

fn map_dua(row: &Row) -> Result<DuaAgreement, ApiError> {
    let status = AgreementStatus::from_str(row.get("status"))
        .map_err(|err| ApiError::Internal(format!("invalid DUA status in database: {err}")))?;
    Ok(DuaAgreement {
        id: row.get("id"),
        organization_id: row.get("organization_id"),
        counterparty: row.get("counterparty"),
        status,
        created_at: row.get("created_at"),
    })
}

fn map_dua_signature(row: &Row) -> Result<DuaSignature, ApiError> {
    Ok(DuaSignature {
        id: row.get("id"),
        dua_agreement_id: row.get("dua_agreement_id"),
        signer_name: row.get("signer_name"),
        signer_email: row.get("signer_email"),
        signer_role: row.get("signer_role"),
        signature_text: row.get("signature_text"),
        signed_at: row.get("signed_at"),
    })
}

fn map_reminder_job(row: &Row) -> Result<ReminderJob, ApiError> {
    let status = ReminderJobStatus::from_str(row.get("status"))
        .map_err(|err| ApiError::Internal(format!("invalid reminder status in database: {err}")))?;
    Ok(ReminderJob {
        id: row.get("id"),
        organization_id: row.get("organization_id"),
        study_id: row.get("study_id"),
        patient_id: row.get("patient_id"),
        visit_id: row.get("visit_id"),
        channel: row.get("channel"),
        recipient: row.get("recipient"),
        message: row.get("message"),
        status,
        scheduled_for: row.get("scheduled_for"),
        processed_at: row.get("processed_at"),
        last_error: row.get("last_error"),
        created_at: row.get("created_at"),
    })
}

fn map_media_asset(row: &Row) -> Result<MediaAsset, ApiError> {
    let status = MediaAssetStatus::from_str(row.get("status"))
        .map_err(|err| ApiError::Internal(format!("invalid media asset status: {err}")))?;
    Ok(MediaAsset {
        id: row.get("id"),
        organization_id: row.get("organization_id"),
        study_id: row.get("study_id"),
        patient_id: row.get("patient_id"),
        category: row.get("category"),
        filename: row.get("filename"),
        object_key: row.get("object_key"),
        content_type: row.get("content_type"),
        expected_byte_size: row.get("expected_byte_size"),
        byte_size: row.get("byte_size"),
        status,
        upload_expires_at: row.get("upload_expires_at"),
        uploaded_at: row.get("uploaded_at"),
        created_by_user_id: row.get("created_by_user_id"),
        created_at: row.get("created_at"),
    })
}

fn map_organization_media_policy(row: &Row) -> Result<OrganizationMediaPolicy, ApiError> {
    Ok(OrganizationMediaPolicy {
        organization_id: row.get("organization_id"),
        max_total_bytes: row.get("max_total_bytes"),
        max_asset_bytes: row.get("max_asset_bytes"),
        updated_at: row.get("updated_at"),
    })
}

fn map_user(row: &Row) -> Result<User, ApiError> {
    let platform_role = AppRole::from_str(row.get("platform_role"))
        .map_err(|err| ApiError::Internal(format!("invalid platform role in database: {err}")))?;
    Ok(User {
        id: row.get("id"),
        subject: row.get("subject"),
        email: row.get("email"),
        display_name: row.get("display_name"),
        platform_role,
        created_at: row.get("created_at"),
    })
}

fn map_membership(row: &Row) -> Result<OrganizationMembership, ApiError> {
    let role = AppRole::from_str(row.get("role"))
        .map_err(|err| ApiError::Internal(format!("invalid membership role in database: {err}")))?;
    Ok(OrganizationMembership {
        id: row.get("id"),
        user_id: row.get("user_id"),
        organization_id: row.get("organization_id"),
        role,
        created_at: row.get("created_at"),
    })
}

fn map_audit_log(row: &Row) -> Result<AuditLogRecord, ApiError> {
    let actor_role = row
        .get::<_, Option<&str>>("actor_role")
        .map(AppRole::from_str)
        .transpose()
        .map_err(|err| ApiError::Internal(format!("invalid actor role in audit log: {err}")))?;
    Ok(AuditLogRecord {
        id: row.get("id"),
        actor_user_id: row.get("actor_user_id"),
        actor_subject: row.get("actor_subject"),
        actor_role,
        actor_email: row.get("actor_email"),
        auth_source: row.get("auth_source"),
        method: row.get("method"),
        path: row.get("path"),
        action: row.get("action"),
        resource_type: row.get("resource_type"),
        resource_id: row.get("resource_id"),
        metadata_json: row.get("metadata_json"),
        status_code: row.get("status_code"),
        happened_at: row.get("happened_at"),
    })
}
