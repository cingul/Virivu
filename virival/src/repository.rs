use std::str::FromStr;

use async_trait::async_trait;
use tokio_postgres::Row;
use uuid::Uuid;

use crate::{
    db::DbPool,
    error::ApiError,
    models::{
        AgreementStatus, CrfSubmission, CrfTemplate, DataQuery, DuaAgreement, Organization,
        Patient, QueryStatus, Site, Study, StudyPhase, StudyReadiness, SubmissionStatus, Visit,
        VisitStatus,
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
    async fn close_data_query(&self, query_id: Uuid) -> Result<DataQuery, ApiError>;

    async fn create_dua(
        &self,
        organization_id: Uuid,
        counterparty: &str,
    ) -> Result<DuaAgreement, ApiError>;
    async fn list_duas(&self) -> Result<Vec<DuaAgreement>, ApiError>;
    async fn activate_dua(&self, dua_id: Uuid) -> Result<DuaAgreement, ApiError>;

    async fn compute_readiness(&self, study_id: Uuid) -> Result<StudyReadiness, ApiError>;
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

    async fn close_data_query(&self, query_id: Uuid) -> Result<DataQuery, ApiError> {
        let client = self.client().await?;
        let row = client
            .query_opt(
                "UPDATE data_queries SET status = $2
                 WHERE id = $1
                 RETURNING id, study_id, submission_id, summary, status, raised_at",
                &[&query_id, &QueryStatus::Closed.as_db()],
            )
            .await
            .map_err(map_write_err)?
            .ok_or_else(|| ApiError::NotFound(format!("query {query_id}")))?;
        map_data_query(&row)
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
                    SELECT 1 FROM crf_templates WHERE study_id = $1 AND published = TRUE
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
            has_active_dua,
            next_recommended_action,
        })
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
