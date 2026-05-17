use anyhow::anyhow;
use chrono::{Duration, NaiveDate, Utc};
use deadpool_postgres::{Manager, ManagerConfig, Pool, RecyclingMethod, Runtime};
use std::net::IpAddr;
use tokio_postgres::{NoTls, Row};
use uuid::Uuid;

use crate::models::{
    DataUseAgreement, DataUseAgreementSignature, Encounter, FormInvite, MediaUploadTicket,
    Organization, OrganizationSummaryRow, OutboundEmail, Patient, PatientStudyVisit, Project,
    ProjectProgressRow, Provider, Site, StudyCloseChecklistItem, StudyCrfField, StudyCrfSubmission,
    StudyCrfTemplate, StudyDataQuery, StudyOperationalSummary, StudyPhaseEvent, StudyReadiness,
    StudyStartupChecklistItem, StudyVisitTemplate, User, UserMembership,
};

#[derive(Clone)]
pub struct Db {
    pool: Pool,
}

const DEFAULT_STUDY_STARTUP_CHECKLIST_ITEMS: [(&str, &str); 8] = [
    (
        "protocol_finalized",
        "Protocol and schedule of assessments finalized for operational launch",
    ),
    (
        "budget_contracts_executed",
        "Study budget and site contract package executed",
    ),
    (
        "irb_approval_documented",
        "IRB / ethics approval documented for study launch",
    ),
    (
        "regulatory_documents_complete",
        "Essential regulatory documents (delegation log, CVs, disclosures, approvals) complete",
    ),
    (
        "site_activation_complete",
        "At least one site is activated with investigator assignment",
    ),
    (
        "crf_publish_complete",
        "Core CRF templates are published and validated",
    ),
    (
        "edc_permissions_validated",
        "EDC roles, permissions, and production-ready data review views validated",
    ),
    (
        "team_training_complete",
        "Study team training and SOP acknowledgement completed",
    ),
];

impl Db {
    pub async fn connect(database_url: &str) -> anyhow::Result<Self> {
        let pg_config: tokio_postgres::Config = database_url.parse()?;
        let manager = Manager::from_config(
            pg_config,
            NoTls,
            ManagerConfig {
                recycling_method: RecyclingMethod::Fast,
            },
        );
        let pool = Pool::builder(manager)
            .runtime(Runtime::Tokio1)
            .max_size(16)
            .build()?;
        Ok(Self { pool })
    }

    pub async fn create_organization(
        &self,
        name: &str,
        parent_organization_id: Option<Uuid>,
        organization_kind: Option<&str>,
    ) -> anyhow::Result<Organization> {
        let client = self.pool.get().await?;
        let kind = normalize_organization_kind(organization_kind.unwrap_or("tenant"))
            .ok_or_else(|| anyhow!("invalid organization_kind"))?;
        if kind == "platform_root" {
            let row = client
                .query_one(
                    "SELECT EXISTS(SELECT 1 FROM organizations WHERE organization_kind = 'platform_root') AS exists",
                    &[],
                )
                .await?;
            let has_root: bool = row.get("exists");
            if has_root {
                return Err(anyhow!(
                    "platform_root organization already exists; only one root is supported"
                ));
            }
        }
        let effective_parent_organization_id = if kind == "platform_root" {
            None
        } else if let Some(explicit_parent) = parent_organization_id {
            Some(explicit_parent)
        } else {
            Some(self.get_platform_root_organization_id(&client).await?)
        };
        let workspace_slug = self.generate_unique_workspace_slug(&client, name).await?;
        let hex_code = self.generate_unique_org_hex(&client).await?;
        let row = client
            .query_one(
                r#"
                INSERT INTO organizations (
                    name,
                    parent_organization_id,
                    organization_kind,
                    workspace_slug,
                    hex_code
                )
                VALUES ($1, $2, $3, $4, $5)
                RETURNING
                    id,
                    name,
                    parent_organization_id,
                    organization_kind,
                    workspace_slug,
                    hex_code,
                    created_at
                "#,
                &[
                    &name,
                    &effective_parent_organization_id,
                    &kind,
                    &workspace_slug,
                    &hex_code,
                ],
            )
            .await?;
        Ok(row_to_organization(&row))
    }

    pub async fn list_projects_by_organization(
        &self,
        organization_id: Uuid,
    ) -> anyhow::Result<Vec<Project>> {
        let client = self.pool.get().await?;
        let rows = client
            .query(
                r#"
                SELECT
                    id,
                    organization_id,
                    name,
                    therapeutic_area,
                    protocol_code,
                    lifecycle_phase,
                    planned_enrollment,
                    clinicaltrials_gov_id,
                    study_summary,
                    phase_changed_at,
                    hex_code,
                    created_at
                FROM projects
                WHERE organization_id = $1
                ORDER BY created_at DESC
                "#,
                &[&organization_id],
            )
            .await?;
        Ok(rows
            .iter()
            .map(|r| Project {
                id: r.get("id"),
                organization_id: r.get("organization_id"),
                name: r.get("name"),
                therapeutic_area: r.get("therapeutic_area"),
                protocol_code: r.get("protocol_code"),
                lifecycle_phase: r.get("lifecycle_phase"),
                planned_enrollment: r.get("planned_enrollment"),
                clinicaltrials_gov_id: r.get("clinicaltrials_gov_id"),
                study_summary: r.get("study_summary"),
                phase_changed_at: r.get("phase_changed_at"),
                hex_code: r.get("hex_code"),
                created_at: r.get("created_at"),
            })
            .collect())
    }

    pub async fn list_sites_by_project(&self, project_id: Uuid) -> anyhow::Result<Vec<Site>> {
        let client = self.pool.get().await?;
        let rows = client
            .query(
                r#"
                SELECT id, project_id, name, principal_investigator, hex_code, created_at
                FROM sites
                WHERE project_id = $1
                ORDER BY created_at DESC
                "#,
                &[&project_id],
            )
            .await?;
        Ok(rows
            .iter()
            .map(|r| Site {
                id: r.get("id"),
                project_id: r.get("project_id"),
                name: r.get("name"),
                principal_investigator: r.get("principal_investigator"),
                hex_code: r.get("hex_code"),
                created_at: r.get("created_at"),
            })
            .collect())
    }

    pub async fn create_project(
        &self,
        organization_id: Uuid,
        name: &str,
        therapeutic_area: &str,
    ) -> anyhow::Result<Project> {
        let client = self.pool.get().await?;
        let org_hex = self
            .ensure_organization_hex_code(&client, organization_id)
            .await?;
        let hex_code = self.generate_unique_project_hex(&client, &org_hex).await?;
        let row = client
            .query_one(
                r#"
                INSERT INTO projects (organization_id, name, therapeutic_area, hex_code)
                VALUES ($1, $2, $3, $4)
                RETURNING
                    id,
                    organization_id,
                    name,
                    therapeutic_area,
                    protocol_code,
                    lifecycle_phase,
                    planned_enrollment,
                    clinicaltrials_gov_id,
                    study_summary,
                    phase_changed_at,
                    hex_code,
                    created_at
                "#,
                &[&organization_id, &name, &therapeutic_area, &hex_code],
            )
            .await?;
        Ok(Project {
            id: row.get("id"),
            organization_id: row.get("organization_id"),
            name: row.get("name"),
            therapeutic_area: row.get("therapeutic_area"),
            protocol_code: row.get("protocol_code"),
            lifecycle_phase: row.get("lifecycle_phase"),
            planned_enrollment: row.get("planned_enrollment"),
            clinicaltrials_gov_id: row.get("clinicaltrials_gov_id"),
            study_summary: row.get("study_summary"),
            phase_changed_at: row.get("phase_changed_at"),
            hex_code: row.get("hex_code"),
            created_at: row.get("created_at"),
        })
    }

    pub async fn create_study_project(
        &self,
        organization_id: Uuid,
        name: &str,
        therapeutic_area: &str,
        protocol_code: Option<&str>,
        planned_enrollment: i32,
        clinicaltrials_gov_id: Option<&str>,
        study_summary: &str,
    ) -> anyhow::Result<Project> {
        let client = self.pool.get().await?;
        let org_hex = self
            .ensure_organization_hex_code(&client, organization_id)
            .await?;
        let hex_code = self.generate_unique_project_hex(&client, &org_hex).await?;
        let row = client
            .query_one(
                r#"
                INSERT INTO projects (
                    organization_id,
                    name,
                    therapeutic_area,
                    protocol_code,
                    planned_enrollment,
                    clinicaltrials_gov_id,
                    study_summary,
                    hex_code
                )
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                RETURNING
                    id,
                    organization_id,
                    name,
                    therapeutic_area,
                    protocol_code,
                    lifecycle_phase,
                    planned_enrollment,
                    clinicaltrials_gov_id,
                    study_summary,
                    phase_changed_at,
                    hex_code,
                    created_at
                "#,
                &[
                    &organization_id,
                    &name,
                    &therapeutic_area,
                    &protocol_code,
                    &planned_enrollment,
                    &clinicaltrials_gov_id,
                    &study_summary,
                    &hex_code,
                ],
            )
            .await?;
        let project_id: Uuid = row.get("id");
        for (item_code, item_label) in DEFAULT_STUDY_STARTUP_CHECKLIST_ITEMS {
            client
                .execute(
                    r#"
                    INSERT INTO study_startup_checklist_items (project_id, item_code, item_label)
                    VALUES ($1, $2, $3)
                    ON CONFLICT (project_id, item_code) DO NOTHING
                    "#,
                    &[&project_id, &item_code, &item_label],
                )
                .await?;
        }
        for (item_code, item_label) in [
            (
                "data_cleaning_complete",
                "Data cleaning completed and final CRF review done",
            ),
            (
                "pending_queries_resolved",
                "All monitor/data-management queries are resolved",
            ),
            (
                "final_monitoring_complete",
                "Final monitoring review completed and documented",
            ),
            (
                "regulatory_package_archived",
                "Regulatory and compliance package archived",
            ),
        ] {
            client
                .execute(
                    r#"
                    INSERT INTO study_close_checklist_items (project_id, item_code, item_label)
                    VALUES ($1, $2, $3)
                    ON CONFLICT (project_id, item_code) DO NOTHING
                    "#,
                    &[&project_id, &item_code, &item_label],
                )
                .await?;
        }
        Ok(row_to_project(&row))
    }

    pub async fn study_readiness(&self, project_id: Uuid) -> anyhow::Result<StudyReadiness> {
        let client = self.pool.get().await?;
        let row = client
            .query_one(
                r#"
                SELECT
                    p.lifecycle_phase,
                    (SELECT COUNT(*)::BIGINT FROM sites s WHERE s.project_id = p.id) AS total_sites,
                    (SELECT COUNT(*)::BIGINT FROM patients pt WHERE pt.project_id = p.id) AS total_patients,
                    (
                        SELECT COUNT(*)::BIGINT
                        FROM encounters e
                        JOIN patients pt ON pt.id = e.patient_id
                        WHERE pt.project_id = p.id
                    ) AS total_encounters,
                    (
                        SELECT COUNT(*)::BIGINT
                        FROM study_crf_templates t
                        WHERE t.project_id = p.id AND t.status = 'published'
                    ) AS published_crf_templates,
                    (
                        SELECT COUNT(*)::BIGINT
                        FROM study_crf_templates t
                        WHERE t.project_id = p.id AND t.status = 'draft'
                    ) AS draft_crf_templates
                FROM projects p
                WHERE p.id = $1
                "#,
                &[&project_id],
            )
            .await?;
        Ok(StudyReadiness {
            lifecycle_phase: row.get("lifecycle_phase"),
            total_sites: row.get("total_sites"),
            total_patients: row.get("total_patients"),
            total_encounters: row.get("total_encounters"),
            published_crf_templates: row.get("published_crf_templates"),
            draft_crf_templates: row.get("draft_crf_templates"),
        })
    }

    pub async fn transition_study_phase(
        &self,
        project_id: Uuid,
        new_phase: &str,
        changed_by_user_id: Option<Uuid>,
        notes: &str,
    ) -> anyhow::Result<Project> {
        let target_phase = normalize_study_phase(new_phase)
            .ok_or_else(|| anyhow!("invalid lifecycle phase: {new_phase}"))?;
        let client = self.pool.get().await?;
        let row = client
            .query_opt(
                "SELECT lifecycle_phase FROM projects WHERE id = $1",
                &[&project_id],
            )
            .await?
            .ok_or_else(|| anyhow!("project not found"))?;
        let current_phase: String = row.get("lifecycle_phase");
        if current_phase == target_phase {
            return self
                .get_project(project_id)
                .await?
                .ok_or_else(|| anyhow!("project not found"));
        }
        if !is_valid_phase_transition(&current_phase, &target_phase) {
            return Err(anyhow!(
                "invalid phase transition from {current_phase} to {target_phase}"
            ));
        }

        let readiness = self.study_readiness(project_id).await?;
        if target_phase == "initiated"
            && (readiness.total_sites < 1 || readiness.published_crf_templates < 1)
        {
            return Err(anyhow!(
                "cannot initiate study without at least one site and one published CRF template"
            ));
        }
        if target_phase == "initiated" {
            let row = client
                .query_one(
                    r#"
                    SELECT COUNT(*)::BIGINT AS startup_items_pending
                    FROM study_startup_checklist_items
                    WHERE project_id = $1 AND completed = FALSE
                    "#,
                    &[&project_id],
                )
                .await?;
            let startup_items_pending: i64 = row.get("startup_items_pending");
            if startup_items_pending > 0 {
                return Err(anyhow!(
                    "cannot initiate study until startup checklist is fully completed"
                ));
            }
        }
        if target_phase == "active" && readiness.total_patients < 1 {
            return Err(anyhow!(
                "cannot set study active without at least one enrolled patient"
            ));
        }
        if target_phase == "closed" {
            let row = client
                .query_one(
                    r#"
                    SELECT
                        (
                            SELECT COUNT(*)::BIGINT
                            FROM study_data_queries q
                            WHERE q.project_id = $1 AND q.status <> 'closed'
                        ) AS open_queries,
                        (
                            SELECT COUNT(*)::BIGINT
                            FROM study_close_checklist_items c
                            WHERE c.project_id = $1
                              AND c.completed = FALSE
                        ) AS incomplete_checklist_items
                    "#,
                    &[&project_id],
                )
                .await?;
            let open_queries: i64 = row.get("open_queries");
            let incomplete_checklist_items: i64 = row.get("incomplete_checklist_items");
            if open_queries > 0 {
                return Err(anyhow!(
                    "cannot close study while data queries remain open/responded"
                ));
            }
            if incomplete_checklist_items > 0 {
                return Err(anyhow!(
                    "cannot close study until all close checklist items are completed"
                ));
            }
        }

        client
            .execute(
                r#"
                UPDATE projects
                SET lifecycle_phase = $2, phase_changed_at = NOW()
                WHERE id = $1
                "#,
                &[&project_id, &target_phase],
            )
            .await?;
        client
            .execute(
                r#"
                INSERT INTO study_phase_events (
                    project_id,
                    previous_phase,
                    new_phase,
                    changed_by_user_id,
                    notes
                )
                VALUES ($1, $2, $3, $4, $5)
                "#,
                &[
                    &project_id,
                    &current_phase,
                    &target_phase,
                    &changed_by_user_id,
                    &notes,
                ],
            )
            .await?;
        self.get_project(project_id)
            .await?
            .ok_or_else(|| anyhow!("project not found"))
    }

    pub async fn list_study_phase_events(
        &self,
        project_id: Uuid,
    ) -> anyhow::Result<Vec<StudyPhaseEvent>> {
        let client = self.pool.get().await?;
        let rows = client
            .query(
                r#"
                SELECT
                    id,
                    project_id,
                    previous_phase,
                    new_phase,
                    changed_by_user_id,
                    notes,
                    created_at
                FROM study_phase_events
                WHERE project_id = $1
                ORDER BY created_at DESC
                LIMIT 25
                "#,
                &[&project_id],
            )
            .await?;
        Ok(rows.iter().map(row_to_study_phase_event).collect())
    }

    pub async fn create_study_crf_template(
        &self,
        project_id: Uuid,
        name: &str,
        description: &str,
        applicable_phase: &str,
        created_by_user_id: Option<Uuid>,
    ) -> anyhow::Result<StudyCrfTemplate> {
        let phase = normalize_study_phase(applicable_phase)
            .ok_or_else(|| anyhow!("invalid applicable_phase"))?;
        let client = self.pool.get().await?;
        let row = client
            .query_one(
                r#"
                INSERT INTO study_crf_templates (
                    project_id,
                    name,
                    description,
                    applicable_phase,
                    created_by_user_id
                )
                VALUES ($1, $2, $3, $4, $5)
                RETURNING
                    id,
                    project_id,
                    name,
                    description,
                    version,
                    status,
                    applicable_phase,
                    created_by_user_id,
                    created_at,
                    updated_at
                "#,
                &[
                    &project_id,
                    &name,
                    &description,
                    &phase,
                    &created_by_user_id,
                ],
            )
            .await?;
        Ok(row_to_study_crf_template(&row))
    }

    pub async fn list_study_crf_templates(
        &self,
        project_id: Uuid,
    ) -> anyhow::Result<Vec<StudyCrfTemplate>> {
        let client = self.pool.get().await?;
        let rows = client
            .query(
                r#"
                SELECT
                    id,
                    project_id,
                    name,
                    description,
                    version,
                    status,
                    applicable_phase,
                    created_by_user_id,
                    created_at,
                    updated_at
                FROM study_crf_templates
                WHERE project_id = $1
                ORDER BY created_at DESC
                "#,
                &[&project_id],
            )
            .await?;
        Ok(rows.iter().map(row_to_study_crf_template).collect())
    }

    pub async fn get_study_crf_template(
        &self,
        template_id: Uuid,
    ) -> anyhow::Result<Option<StudyCrfTemplate>> {
        let client = self.pool.get().await?;
        let row = client
            .query_opt(
                r#"
                SELECT
                    id,
                    project_id,
                    name,
                    description,
                    version,
                    status,
                    applicable_phase,
                    created_by_user_id,
                    created_at,
                    updated_at
                FROM study_crf_templates
                WHERE id = $1
                "#,
                &[&template_id],
            )
            .await?;
        Ok(row.as_ref().map(row_to_study_crf_template))
    }

    pub async fn publish_study_crf_template(
        &self,
        template_id: Uuid,
    ) -> anyhow::Result<StudyCrfTemplate> {
        let client = self.pool.get().await?;
        let row = client
            .query_one(
                r#"
                UPDATE study_crf_templates
                SET status = 'published', updated_at = NOW()
                WHERE id = $1
                RETURNING
                    id,
                    project_id,
                    name,
                    description,
                    version,
                    status,
                    applicable_phase,
                    created_by_user_id,
                    created_at,
                    updated_at
                "#,
                &[&template_id],
            )
            .await?;
        Ok(row_to_study_crf_template(&row))
    }

    pub async fn add_study_crf_field(
        &self,
        template_id: Uuid,
        field_key: &str,
        field_label: &str,
        field_type: &str,
        required: bool,
        options_json: &str,
        display_order: i32,
    ) -> anyhow::Result<StudyCrfField> {
        if normalize_crf_field_type(field_type).is_none() {
            return Err(anyhow!("invalid field_type"));
        }
        let client = self.pool.get().await?;
        let row = client
            .query_one(
                r#"
                INSERT INTO study_crf_fields (
                    template_id,
                    field_key,
                    field_label,
                    field_type,
                    required,
                    options_json,
                    display_order
                )
                VALUES ($1, $2, $3, $4, $5, $6::TEXT::JSONB, $7)
                RETURNING
                    id,
                    template_id,
                    field_key,
                    field_label,
                    field_type,
                    required,
                    options_json::TEXT AS options_json,
                    display_order,
                    created_at
                "#,
                &[
                    &template_id,
                    &field_key,
                    &field_label,
                    &field_type,
                    &required,
                    &options_json,
                    &display_order,
                ],
            )
            .await?;
        Ok(row_to_study_crf_field(&row))
    }

    pub async fn get_study_crf_field(
        &self,
        field_id: Uuid,
    ) -> anyhow::Result<Option<StudyCrfField>> {
        let client = self.pool.get().await?;
        let row = client
            .query_opt(
                r#"
                SELECT
                    id,
                    template_id,
                    field_key,
                    field_label,
                    field_type,
                    required,
                    options_json::TEXT AS options_json,
                    display_order,
                    created_at
                FROM study_crf_fields
                WHERE id = $1
                "#,
                &[&field_id],
            )
            .await?;
        Ok(row.as_ref().map(row_to_study_crf_field))
    }

    pub async fn update_study_crf_field(
        &self,
        field_id: Uuid,
        field_key: &str,
        field_label: &str,
        field_type: &str,
        required: bool,
        options_json: &str,
        display_order: i32,
    ) -> anyhow::Result<StudyCrfField> {
        if normalize_crf_field_type(field_type).is_none() {
            return Err(anyhow!("invalid field_type"));
        }
        let client = self.pool.get().await?;
        let row = client
            .query_one(
                r#"
                UPDATE study_crf_fields
                SET
                    field_key = $2,
                    field_label = $3,
                    field_type = $4,
                    required = $5,
                    options_json = $6::TEXT::JSONB,
                    display_order = $7
                WHERE id = $1
                RETURNING
                    id,
                    template_id,
                    field_key,
                    field_label,
                    field_type,
                    required,
                    options_json::TEXT AS options_json,
                    display_order,
                    created_at
                "#,
                &[
                    &field_id,
                    &field_key,
                    &field_label,
                    &field_type,
                    &required,
                    &options_json,
                    &display_order,
                ],
            )
            .await?;
        Ok(row_to_study_crf_field(&row))
    }

    pub async fn list_study_crf_fields(
        &self,
        template_id: Uuid,
    ) -> anyhow::Result<Vec<StudyCrfField>> {
        let client = self.pool.get().await?;
        let rows = client
            .query(
                r#"
                SELECT
                    id,
                    template_id,
                    field_key,
                    field_label,
                    field_type,
                    required,
                    options_json::TEXT AS options_json,
                    display_order,
                    created_at
                FROM study_crf_fields
                WHERE template_id = $1
                ORDER BY display_order ASC, created_at ASC
                "#,
                &[&template_id],
            )
            .await?;
        Ok(rows.iter().map(row_to_study_crf_field).collect())
    }

    pub async fn create_study_visit_template(
        &self,
        project_id: Uuid,
        visit_code: &str,
        visit_name: &str,
        target_day: i32,
        window_before_days: i32,
        window_after_days: i32,
        required: bool,
    ) -> anyhow::Result<StudyVisitTemplate> {
        let client = self.pool.get().await?;
        let row = client
            .query_one(
                r#"
                INSERT INTO study_visit_templates (
                    project_id,
                    visit_code,
                    visit_name,
                    target_day,
                    window_before_days,
                    window_after_days,
                    required
                )
                VALUES ($1, $2, $3, $4, $5, $6, $7)
                RETURNING
                    id,
                    project_id,
                    visit_code,
                    visit_name,
                    target_day,
                    window_before_days,
                    window_after_days,
                    required,
                    created_at
                "#,
                &[
                    &project_id,
                    &visit_code,
                    &visit_name,
                    &target_day,
                    &window_before_days,
                    &window_after_days,
                    &required,
                ],
            )
            .await?;
        Ok(row_to_study_visit_template(&row))
    }

    pub async fn list_study_visit_templates(
        &self,
        project_id: Uuid,
    ) -> anyhow::Result<Vec<StudyVisitTemplate>> {
        let client = self.pool.get().await?;
        let rows = client
            .query(
                r#"
                SELECT
                    id,
                    project_id,
                    visit_code,
                    visit_name,
                    target_day,
                    window_before_days,
                    window_after_days,
                    required,
                    created_at
                FROM study_visit_templates
                WHERE project_id = $1
                ORDER BY target_day ASC, created_at ASC
                "#,
                &[&project_id],
            )
            .await?;
        Ok(rows.iter().map(row_to_study_visit_template).collect())
    }

    pub async fn schedule_patient_study_visit(
        &self,
        project_id: Uuid,
        patient_id: Uuid,
        visit_template_id: Uuid,
        scheduled_for: Option<NaiveDate>,
    ) -> anyhow::Result<PatientStudyVisit> {
        let client = self.pool.get().await?;
        let relation = client
            .query_one(
                r#"
                SELECT
                    (SELECT p.project_id FROM patients p WHERE p.id = $1) AS patient_project_id,
                    (SELECT vt.project_id FROM study_visit_templates vt WHERE vt.id = $2) AS template_project_id
                "#,
                &[&patient_id, &visit_template_id],
            )
            .await?;
        let patient_project_id: Option<Uuid> = relation.get("patient_project_id");
        let template_project_id: Option<Uuid> = relation.get("template_project_id");
        if patient_project_id != Some(project_id) || template_project_id != Some(project_id) {
            return Err(anyhow!(
                "patient or visit template does not belong to provided project"
            ));
        }
        let row = client
            .query_one(
                r#"
                INSERT INTO patient_study_visits (
                    project_id,
                    patient_id,
                    visit_template_id,
                    scheduled_for
                )
                VALUES ($1, $2, $3, $4)
                RETURNING
                    id,
                    project_id,
                    patient_id,
                    visit_template_id,
                    scheduled_for,
                    status,
                    completed_at,
                    locked,
                    locked_at,
                    locked_by_user_id,
                    created_at
                "#,
                &[&project_id, &patient_id, &visit_template_id, &scheduled_for],
            )
            .await?;
        Ok(row_to_patient_study_visit(&row))
    }

    pub async fn list_patient_study_visits(
        &self,
        project_id: Uuid,
    ) -> anyhow::Result<Vec<PatientStudyVisit>> {
        let client = self.pool.get().await?;
        let rows = client
            .query(
                r#"
                SELECT
                    id,
                    project_id,
                    patient_id,
                    visit_template_id,
                    scheduled_for,
                    status,
                    completed_at,
                    locked,
                    locked_at,
                    locked_by_user_id,
                    created_at
                FROM patient_study_visits
                WHERE project_id = $1
                ORDER BY created_at DESC
                LIMIT 200
                "#,
                &[&project_id],
            )
            .await?;
        Ok(rows.iter().map(row_to_patient_study_visit).collect())
    }

    pub async fn create_study_crf_submission(
        &self,
        project_id: Uuid,
        template_id: Uuid,
        patient_id: Uuid,
        patient_visit_id: Option<Uuid>,
        answers_json: &str,
        entered_by_user_id: Option<Uuid>,
    ) -> anyhow::Result<StudyCrfSubmission> {
        let client = self.pool.get().await?;
        let relation = client
            .query_one(
                r#"
                SELECT
                    (SELECT t.project_id FROM study_crf_templates t WHERE t.id = $1) AS template_project_id,
                    (SELECT p.project_id FROM patients p WHERE p.id = $2) AS patient_project_id
                "#,
                &[&template_id, &patient_id],
            )
            .await?;
        let template_project_id: Option<Uuid> = relation.get("template_project_id");
        let patient_project_id: Option<Uuid> = relation.get("patient_project_id");
        if template_project_id != Some(project_id) || patient_project_id != Some(project_id) {
            return Err(anyhow!(
                "template or patient does not belong to provided project"
            ));
        }
        let row = client
            .query_one(
                r#"
                INSERT INTO study_crf_submissions (
                    project_id,
                    template_id,
                    patient_id,
                    patient_visit_id,
                    answers_json,
                    entered_by_user_id
                )
                VALUES ($1, $2, $3, $4, $5::TEXT::JSONB, $6)
                RETURNING
                    id,
                    project_id,
                    template_id,
                    patient_id,
                    patient_visit_id,
                    answers_json::TEXT AS answers_json,
                    status,
                    entered_by_user_id,
                    submitted_at,
                    locked_at,
                    created_at,
                    updated_at
                "#,
                &[
                    &project_id,
                    &template_id,
                    &patient_id,
                    &patient_visit_id,
                    &answers_json,
                    &entered_by_user_id,
                ],
            )
            .await?;
        Ok(row_to_study_crf_submission(&row))
    }

    pub async fn list_study_crf_submissions(
        &self,
        project_id: Uuid,
    ) -> anyhow::Result<Vec<StudyCrfSubmission>> {
        let client = self.pool.get().await?;
        let rows = client
            .query(
                r#"
                SELECT
                    id,
                    project_id,
                    template_id,
                    patient_id,
                    patient_visit_id,
                    answers_json::TEXT AS answers_json,
                    status,
                    entered_by_user_id,
                    submitted_at,
                    locked_at,
                    created_at,
                    updated_at
                FROM study_crf_submissions
                WHERE project_id = $1
                ORDER BY created_at DESC
                LIMIT 200
                "#,
                &[&project_id],
            )
            .await?;
        Ok(rows.iter().map(row_to_study_crf_submission).collect())
    }

    pub async fn get_study_crf_submission(
        &self,
        submission_id: Uuid,
    ) -> anyhow::Result<Option<StudyCrfSubmission>> {
        let client = self.pool.get().await?;
        let row = client
            .query_opt(
                r#"
                SELECT
                    id,
                    project_id,
                    template_id,
                    patient_id,
                    patient_visit_id,
                    answers_json::TEXT AS answers_json,
                    status,
                    entered_by_user_id,
                    submitted_at,
                    locked_at,
                    created_at,
                    updated_at
                FROM study_crf_submissions
                WHERE id = $1
                "#,
                &[&submission_id],
            )
            .await?;
        Ok(row.as_ref().map(row_to_study_crf_submission))
    }

    pub async fn submit_study_crf_submission(
        &self,
        submission_id: Uuid,
    ) -> anyhow::Result<StudyCrfSubmission> {
        let client = self.pool.get().await?;
        let row = client
            .query_one(
                r#"
                UPDATE study_crf_submissions
                SET
                    status = CASE WHEN status = 'locked' THEN status ELSE 'submitted' END,
                    submitted_at = CASE WHEN status = 'locked' THEN submitted_at ELSE NOW() END,
                    updated_at = NOW()
                WHERE id = $1
                RETURNING
                    id,
                    project_id,
                    template_id,
                    patient_id,
                    patient_visit_id,
                    answers_json::TEXT AS answers_json,
                    status,
                    entered_by_user_id,
                    submitted_at,
                    locked_at,
                    created_at,
                    updated_at
                "#,
                &[&submission_id],
            )
            .await?;
        Ok(row_to_study_crf_submission(&row))
    }

    pub async fn lock_study_crf_submission(
        &self,
        submission_id: Uuid,
    ) -> anyhow::Result<StudyCrfSubmission> {
        let client = self.pool.get().await?;
        let row = client
            .query_one(
                r#"
                SELECT COUNT(*)::BIGINT AS open_queries
                FROM study_data_queries
                WHERE submission_id = $1 AND status <> 'closed'
                "#,
                &[&submission_id],
            )
            .await?;
        let open_queries: i64 = row.get("open_queries");
        if open_queries > 0 {
            return Err(anyhow!(
                "cannot lock CRF submission while data queries remain open/responded"
            ));
        }
        let row = client
            .query_one(
                r#"
                UPDATE study_crf_submissions
                SET status = 'locked', locked_at = NOW(), updated_at = NOW()
                WHERE id = $1
                RETURNING
                    id,
                    project_id,
                    template_id,
                    patient_id,
                    patient_visit_id,
                    answers_json::TEXT AS answers_json,
                    status,
                    entered_by_user_id,
                    submitted_at,
                    locked_at,
                    created_at,
                    updated_at
                "#,
                &[&submission_id],
            )
            .await?;
        Ok(row_to_study_crf_submission(&row))
    }

    pub async fn create_study_data_query(
        &self,
        project_id: Uuid,
        submission_id: Uuid,
        field_key: &str,
        query_text: &str,
        raised_by_user_id: Option<Uuid>,
    ) -> anyhow::Result<StudyDataQuery> {
        let client = self.pool.get().await?;
        let row = client
            .query_one(
                r#"
                INSERT INTO study_data_queries (
                    project_id,
                    submission_id,
                    field_key,
                    query_text,
                    raised_by_user_id
                )
                VALUES ($1, $2, $3, $4, $5)
                RETURNING
                    id,
                    project_id,
                    submission_id,
                    field_key,
                    query_text,
                    status,
                    response_text,
                    raised_by_user_id,
                    resolved_by_user_id,
                    created_at,
                    updated_at
                "#,
                &[
                    &project_id,
                    &submission_id,
                    &field_key,
                    &query_text,
                    &raised_by_user_id,
                ],
            )
            .await?;
        Ok(row_to_study_data_query(&row))
    }

    pub async fn list_study_data_queries(
        &self,
        project_id: Uuid,
    ) -> anyhow::Result<Vec<StudyDataQuery>> {
        let client = self.pool.get().await?;
        let rows = client
            .query(
                r#"
                SELECT
                    id,
                    project_id,
                    submission_id,
                    field_key,
                    query_text,
                    status,
                    response_text,
                    raised_by_user_id,
                    resolved_by_user_id,
                    created_at,
                    updated_at
                FROM study_data_queries
                WHERE project_id = $1
                ORDER BY created_at DESC
                LIMIT 300
                "#,
                &[&project_id],
            )
            .await?;
        Ok(rows.iter().map(row_to_study_data_query).collect())
    }

    pub async fn get_study_data_query(
        &self,
        query_id: Uuid,
    ) -> anyhow::Result<Option<StudyDataQuery>> {
        let client = self.pool.get().await?;
        let row = client
            .query_opt(
                r#"
                SELECT
                    id,
                    project_id,
                    submission_id,
                    field_key,
                    query_text,
                    status,
                    response_text,
                    raised_by_user_id,
                    resolved_by_user_id,
                    created_at,
                    updated_at
                FROM study_data_queries
                WHERE id = $1
                "#,
                &[&query_id],
            )
            .await?;
        Ok(row.as_ref().map(row_to_study_data_query))
    }

    pub async fn respond_study_data_query(
        &self,
        query_id: Uuid,
        response_text: &str,
    ) -> anyhow::Result<StudyDataQuery> {
        let client = self.pool.get().await?;
        let row = client
            .query_one(
                r#"
                UPDATE study_data_queries
                SET
                    status = 'responded',
                    response_text = $2,
                    updated_at = NOW()
                WHERE id = $1
                RETURNING
                    id,
                    project_id,
                    submission_id,
                    field_key,
                    query_text,
                    status,
                    response_text,
                    raised_by_user_id,
                    resolved_by_user_id,
                    created_at,
                    updated_at
                "#,
                &[&query_id, &response_text],
            )
            .await?;
        Ok(row_to_study_data_query(&row))
    }

    pub async fn close_study_data_query(
        &self,
        query_id: Uuid,
        resolved_by_user_id: Option<Uuid>,
    ) -> anyhow::Result<StudyDataQuery> {
        let client = self.pool.get().await?;
        let row = client
            .query_one(
                r#"
                UPDATE study_data_queries
                SET
                    status = 'closed',
                    resolved_by_user_id = $2,
                    updated_at = NOW()
                WHERE id = $1
                RETURNING
                    id,
                    project_id,
                    submission_id,
                    field_key,
                    query_text,
                    status,
                    response_text,
                    raised_by_user_id,
                    resolved_by_user_id,
                    created_at,
                    updated_at
                "#,
                &[&query_id, &resolved_by_user_id],
            )
            .await?;
        Ok(row_to_study_data_query(&row))
    }

    pub async fn list_study_close_checklist_items(
        &self,
        project_id: Uuid,
    ) -> anyhow::Result<Vec<StudyCloseChecklistItem>> {
        let client = self.pool.get().await?;
        let rows = client
            .query(
                r#"
                SELECT
                    id,
                    project_id,
                    item_code,
                    item_label,
                    completed,
                    completed_by_user_id,
                    completed_at,
                    notes,
                    created_at
                FROM study_close_checklist_items
                WHERE project_id = $1
                ORDER BY created_at ASC
                "#,
                &[&project_id],
            )
            .await?;
        Ok(rows.iter().map(row_to_study_close_checklist_item).collect())
    }

    pub async fn set_study_close_checklist_item(
        &self,
        project_id: Uuid,
        item_code: &str,
        item_label: &str,
        completed: bool,
        completed_by_user_id: Option<Uuid>,
        notes: &str,
    ) -> anyhow::Result<StudyCloseChecklistItem> {
        let client = self.pool.get().await?;
        let row = client
            .query_one(
                r#"
                INSERT INTO study_close_checklist_items (
                    project_id,
                    item_code,
                    item_label,
                    completed,
                    completed_by_user_id,
                    completed_at,
                    notes
                )
                VALUES (
                    $1,
                    $2,
                    $3,
                    $4,
                    $5,
                    CASE WHEN $4 THEN NOW() ELSE NULL END,
                    $6
                )
                ON CONFLICT (project_id, item_code)
                DO UPDATE SET
                    item_label = EXCLUDED.item_label,
                    completed = EXCLUDED.completed,
                    completed_by_user_id = EXCLUDED.completed_by_user_id,
                    completed_at = CASE
                        WHEN EXCLUDED.completed THEN NOW()
                        ELSE NULL
                    END,
                    notes = EXCLUDED.notes
                RETURNING
                    id,
                    project_id,
                    item_code,
                    item_label,
                    completed,
                    completed_by_user_id,
                    completed_at,
                    notes,
                    created_at
                "#,
                &[
                    &project_id,
                    &item_code,
                    &item_label,
                    &completed,
                    &completed_by_user_id,
                    &notes,
                ],
            )
            .await?;
        Ok(row_to_study_close_checklist_item(&row))
    }

    pub async fn list_study_startup_checklist_items(
        &self,
        project_id: Uuid,
    ) -> anyhow::Result<Vec<StudyStartupChecklistItem>> {
        let client = self.pool.get().await?;
        let rows = client
            .query(
                r#"
                SELECT
                    id,
                    project_id,
                    item_code,
                    item_label,
                    completed,
                    completed_by_user_id,
                    completed_at,
                    notes,
                    created_at
                FROM study_startup_checklist_items
                WHERE project_id = $1
                ORDER BY created_at ASC
                "#,
                &[&project_id],
            )
            .await?;
        Ok(rows
            .iter()
            .map(row_to_study_startup_checklist_item)
            .collect())
    }

    pub async fn ensure_default_study_startup_checklist_items(
        &self,
        project_id: Uuid,
    ) -> anyhow::Result<()> {
        let client = self.pool.get().await?;
        for (item_code, item_label) in DEFAULT_STUDY_STARTUP_CHECKLIST_ITEMS {
            client
                .execute(
                    r#"
                    INSERT INTO study_startup_checklist_items (project_id, item_code, item_label)
                    VALUES ($1, $2, $3)
                    ON CONFLICT (project_id, item_code) DO NOTHING
                    "#,
                    &[&project_id, &item_code, &item_label],
                )
                .await?;
        }
        Ok(())
    }

    pub async fn set_study_startup_checklist_item(
        &self,
        project_id: Uuid,
        item_code: &str,
        item_label: &str,
        completed: bool,
        completed_by_user_id: Option<Uuid>,
        notes: &str,
    ) -> anyhow::Result<StudyStartupChecklistItem> {
        let client = self.pool.get().await?;
        let row = client
            .query_one(
                r#"
                INSERT INTO study_startup_checklist_items (
                    project_id,
                    item_code,
                    item_label,
                    completed,
                    completed_by_user_id,
                    completed_at,
                    notes
                )
                VALUES (
                    $1,
                    $2,
                    $3,
                    $4,
                    $5,
                    CASE WHEN $4 THEN NOW() ELSE NULL END,
                    $6
                )
                ON CONFLICT (project_id, item_code)
                DO UPDATE SET
                    item_label = EXCLUDED.item_label,
                    completed = EXCLUDED.completed,
                    completed_by_user_id = EXCLUDED.completed_by_user_id,
                    completed_at = CASE
                        WHEN EXCLUDED.completed THEN NOW()
                        ELSE NULL
                    END,
                    notes = EXCLUDED.notes
                RETURNING
                    id,
                    project_id,
                    item_code,
                    item_label,
                    completed,
                    completed_by_user_id,
                    completed_at,
                    notes,
                    created_at
                "#,
                &[
                    &project_id,
                    &item_code,
                    &item_label,
                    &completed,
                    &completed_by_user_id,
                    &notes,
                ],
            )
            .await?;
        Ok(row_to_study_startup_checklist_item(&row))
    }

    pub async fn study_operational_summary(
        &self,
        project_id: Uuid,
    ) -> anyhow::Result<StudyOperationalSummary> {
        let client = self.pool.get().await?;
        let row = client
            .query_one(
                r#"
                SELECT
                    p.lifecycle_phase,
                    p.planned_enrollment,
                    (
                        SELECT COUNT(*)::BIGINT
                        FROM patients pt
                        WHERE pt.project_id = p.id
                    ) AS enrolled_patients,
                    (
                        SELECT COUNT(*)::BIGINT
                        FROM patient_study_visits pv
                        WHERE pv.project_id = p.id
                    ) AS total_visits_scheduled,
                    (
                        SELECT COUNT(*)::BIGINT
                        FROM patient_study_visits pv
                        WHERE pv.project_id = p.id AND pv.status = 'completed'
                    ) AS completed_visits,
                    (
                        SELECT COUNT(*)::BIGINT
                        FROM study_crf_submissions s
                        WHERE s.project_id = p.id
                    ) AS total_submissions,
                    (
                        SELECT COUNT(*)::BIGINT
                        FROM study_crf_submissions s
                        WHERE s.project_id = p.id AND s.status = 'locked'
                    ) AS locked_submissions,
                    (
                        SELECT COUNT(*)::BIGINT
                        FROM study_data_queries q
                        WHERE q.project_id = p.id AND q.status <> 'closed'
                    ) AS open_data_queries,
                    (
                        SELECT COUNT(*)::BIGINT
                        FROM study_startup_checklist_items c
                        WHERE c.project_id = p.id AND c.completed = FALSE
                    ) AS startup_items_pending,
                    (
                        SELECT COUNT(*)::BIGINT
                        FROM study_close_checklist_items c
                        WHERE c.project_id = p.id AND c.completed = FALSE
                    ) AS close_items_pending
                FROM projects p
                WHERE p.id = $1
                "#,
                &[&project_id],
            )
            .await?;

        let planned_enrollment: i32 = row.get("planned_enrollment");
        let enrolled_patients: i64 = row.get("enrolled_patients");
        Ok(StudyOperationalSummary {
            lifecycle_phase: row.get("lifecycle_phase"),
            planned_enrollment,
            enrolled_patients,
            enrollment_gap: (planned_enrollment as i64 - enrolled_patients).max(0),
            total_visits_scheduled: row.get("total_visits_scheduled"),
            completed_visits: row.get("completed_visits"),
            total_submissions: row.get("total_submissions"),
            locked_submissions: row.get("locked_submissions"),
            open_data_queries: row.get("open_data_queries"),
            startup_items_pending: row.get("startup_items_pending"),
            close_items_pending: row.get("close_items_pending"),
        })
    }

    pub async fn create_site(
        &self,
        project_id: Uuid,
        name: &str,
        principal_investigator: &str,
    ) -> anyhow::Result<Site> {
        let client = self.pool.get().await?;
        let project_hex = self.ensure_project_hex_code(&client, project_id).await?;
        let hex_code = self.generate_unique_site_hex(&client, &project_hex).await?;
        let row = client
            .query_one(
                r#"
                INSERT INTO sites (project_id, name, principal_investigator, hex_code)
                VALUES ($1, $2, $3, $4)
                RETURNING id, project_id, name, principal_investigator, hex_code, created_at
                "#,
                &[&project_id, &name, &principal_investigator, &hex_code],
            )
            .await?;
        Ok(Site {
            id: row.get("id"),
            project_id: row.get("project_id"),
            name: row.get("name"),
            principal_investigator: row.get("principal_investigator"),
            hex_code: row.get("hex_code"),
            created_at: row.get("created_at"),
        })
    }

    pub async fn create_form_invite(
        &self,
        organization_id: Uuid,
        project_id: Uuid,
        patient_email: &str,
        form_type: &str,
    ) -> anyhow::Result<FormInvite> {
        let client = self.pool.get().await?;
        let row = client
            .query_one(
                r#"
                INSERT INTO form_invites (organization_id, project_id, patient_email, form_type, status)
                VALUES ($1, $2, $3, $4, 'sent')
                RETURNING id, organization_id, project_id, patient_email, form_type, status, created_at
                "#,
                &[&organization_id, &project_id, &patient_email, &form_type],
            )
            .await?;
        Ok(FormInvite {
            id: row.get("id"),
            organization_id: row.get("organization_id"),
            project_id: row.get("project_id"),
            patient_email: row.get("patient_email"),
            form_type: row.get("form_type"),
            status: row.get("status"),
            created_at: row.get("created_at"),
        })
    }

    pub async fn create_media_upload_ticket(
        &self,
        organization_id: Uuid,
        project_id: Uuid,
        patient_id: &str,
        mime_type: &str,
    ) -> anyhow::Result<MediaUploadTicket> {
        let id = Uuid::new_v4();
        let expires_at = Utc::now() + Duration::minutes(10);
        let upload_url = format!(
            "https://upload.virivu.example/v1/media/{id}?content_type={}",
            mime_type
        );

        let client = self.pool.get().await?;
        let row = client
            .query_one(
                r#"
                INSERT INTO media_upload_tickets (id, organization_id, project_id, patient_id, mime_type, upload_url, expires_at)
                VALUES ($1, $2, $3, $4, $5, $6, $7)
                RETURNING id, organization_id, project_id, patient_id, mime_type, upload_url, expires_at
                "#,
                &[
                    &id,
                    &organization_id,
                    &project_id,
                    &patient_id,
                    &mime_type,
                    &upload_url,
                    &expires_at,
                ],
            )
            .await?;
        Ok(MediaUploadTicket {
            id: row.get("id"),
            organization_id: row.get("organization_id"),
            project_id: row.get("project_id"),
            patient_id: row.get("patient_id"),
            mime_type: row.get("mime_type"),
            upload_url: row.get("upload_url"),
            expires_at: row.get("expires_at"),
        })
    }

    pub async fn get_project(&self, project_id: Uuid) -> anyhow::Result<Option<Project>> {
        let client = self.pool.get().await?;
        let row = client
            .query_opt(
                r#"
                SELECT
                    id,
                    organization_id,
                    name,
                    therapeutic_area,
                    protocol_code,
                    lifecycle_phase,
                    planned_enrollment,
                    clinicaltrials_gov_id,
                    study_summary,
                    phase_changed_at,
                    hex_code,
                    created_at
                FROM projects
                WHERE id = $1
                "#,
                &[&project_id],
            )
            .await?;

        Ok(row.map(|r| Project {
            id: r.get("id"),
            organization_id: r.get("organization_id"),
            name: r.get("name"),
            therapeutic_area: r.get("therapeutic_area"),
            protocol_code: r.get("protocol_code"),
            lifecycle_phase: r.get("lifecycle_phase"),
            planned_enrollment: r.get("planned_enrollment"),
            clinicaltrials_gov_id: r.get("clinicaltrials_gov_id"),
            study_summary: r.get("study_summary"),
            phase_changed_at: r.get("phase_changed_at"),
            hex_code: r.get("hex_code"),
            created_at: r.get("created_at"),
        }))
    }

    pub async fn get_site(&self, site_id: Uuid) -> anyhow::Result<Option<Site>> {
        let client = self.pool.get().await?;
        let row = client
            .query_opt(
                r#"
                SELECT id, project_id, name, principal_investigator, hex_code, created_at
                FROM sites
                WHERE id = $1
                "#,
                &[&site_id],
            )
            .await?;
        Ok(row.map(|r| Site {
            id: r.get("id"),
            project_id: r.get("project_id"),
            name: r.get("name"),
            principal_investigator: r.get("principal_investigator"),
            hex_code: r.get("hex_code"),
            created_at: r.get("created_at"),
        }))
    }

    pub async fn organization_summary(
        &self,
        organization_id: Uuid,
    ) -> anyhow::Result<OrganizationSummaryRow> {
        let client = self.pool.get().await?;
        let row = client
            .query_one(
                r#"
                SELECT
                    (SELECT COUNT(*)::BIGINT FROM projects p WHERE p.organization_id = $1) AS projects,
                    (
                        SELECT COUNT(*)::BIGINT
                        FROM sites s
                        JOIN projects p ON p.id = s.project_id
                        WHERE p.organization_id = $1
                    ) AS sites,
                    (SELECT COUNT(*)::BIGINT FROM form_invites fi WHERE fi.organization_id = $1) AS sent_form_invites,
                    (
                        SELECT COUNT(*)::BIGINT
                        FROM media_upload_tickets mt
                        WHERE mt.organization_id = $1
                    ) AS generated_media_upload_links
                "#,
                &[&organization_id],
            )
            .await?;

        Ok(OrganizationSummaryRow {
            projects: row.get("projects"),
            sites: row.get("sites"),
            sent_form_invites: row.get("sent_form_invites"),
            generated_media_upload_links: row.get("generated_media_upload_links"),
        })
    }

    pub async fn project_progress_report(
        &self,
        project_id: Uuid,
    ) -> anyhow::Result<ProjectProgressRow> {
        let client = self.pool.get().await?;
        let row = client
            .query_one(
                r#"
                SELECT
                    (SELECT COUNT(*)::BIGINT FROM sites s WHERE s.project_id = $1) AS total_sites,
                    (
                        SELECT COUNT(*)::BIGINT
                        FROM form_invites fi
                        WHERE fi.project_id = $1
                    ) AS total_form_invites,
                    (
                        SELECT COUNT(*)::BIGINT
                        FROM media_upload_tickets mt
                        WHERE mt.project_id = $1
                    ) AS total_media_captures_requested
                "#,
                &[&project_id],
            )
            .await?;
        Ok(ProjectProgressRow {
            total_sites: row.get("total_sites"),
            total_form_invites: row.get("total_form_invites"),
            total_media_captures_requested: row.get("total_media_captures_requested"),
        })
    }

    pub async fn upsert_user(
        &self,
        email: &str,
        google_subject: &str,
        display_name: &str,
    ) -> anyhow::Result<User> {
        let client = self.pool.get().await?;
        let row = client
            .query_one(
                r#"
                INSERT INTO users (email, google_subject, display_name)
                VALUES ($1, $2, $3)
                ON CONFLICT (email)
                DO UPDATE SET
                    google_subject = EXCLUDED.google_subject,
                    display_name = EXCLUDED.display_name
                RETURNING id, email, google_subject, display_name, created_at
                "#,
                &[&email, &google_subject, &display_name],
            )
            .await?;

        Ok(User {
            id: row.get("id"),
            email: row.get("email"),
            google_subject: row.get("google_subject"),
            display_name: row.get("display_name"),
            created_at: row.get("created_at"),
        })
    }

    pub async fn load_memberships_for_email(
        &self,
        email: &str,
    ) -> anyhow::Result<Vec<UserMembership>> {
        let client = self.pool.get().await?;
        let rows = client
            .query(
                r#"
                SELECT um.organization_id, um.project_id, um.role
                FROM user_memberships um
                JOIN users u ON u.id = um.user_id
                WHERE u.email = $1 AND um.status = 'active'
                "#,
                &[&email],
            )
            .await?;

        Ok(rows
            .into_iter()
            .map(|row| UserMembership {
                organization_id: row.get("organization_id"),
                project_id: row.get("project_id"),
                role: row.get("role"),
            })
            .collect())
    }

    pub async fn get_user_by_email(&self, email: &str) -> anyhow::Result<Option<User>> {
        let client = self.pool.get().await?;
        let row = client
            .query_opt(
                r#"
                SELECT id, email, google_subject, display_name, created_at
                FROM users
                WHERE email = $1
                "#,
                &[&email],
            )
            .await?;
        Ok(row.map(|r| User {
            id: r.get("id"),
            email: r.get("email"),
            google_subject: r.get("google_subject"),
            display_name: r.get("display_name"),
            created_at: r.get("created_at"),
        }))
    }

    pub async fn email_has_platform_admin_role(&self, email: &str) -> anyhow::Result<bool> {
        let client = self.pool.get().await?;
        let row = client
            .query_one(
                r#"
                SELECT EXISTS(
                    SELECT 1
                    FROM user_memberships um
                    JOIN users u ON u.id = um.user_id
                    WHERE u.email = $1
                      AND um.status = 'active'
                      AND um.organization_id IS NULL
                      AND um.project_id IS NULL
                      AND um.role = 'platform_admin'
                ) AS has_access
                "#,
                &[&email],
            )
            .await?;
        Ok(row.get("has_access"))
    }

    pub async fn list_organizations_for_email(
        &self,
        email: &str,
    ) -> anyhow::Result<Vec<Organization>> {
        let client = self.pool.get().await?;
        if self.email_has_platform_admin_role(email).await? {
            let rows = client
                .query(
                    r#"
                    SELECT
                        id,
                        name,
                        parent_organization_id,
                        organization_kind,
                        workspace_slug,
                        hex_code,
                        created_at
                    FROM organizations
                    ORDER BY created_at DESC
                    "#,
                    &[],
                )
                .await?;
            return Ok(rows.iter().map(row_to_organization).collect());
        }

        let rows = client
            .query(
                r#"
                SELECT DISTINCT
                    o.id,
                    o.name,
                    o.parent_organization_id,
                    o.organization_kind,
                    o.workspace_slug,
                    o.hex_code,
                    o.created_at
                FROM organizations o
                JOIN user_memberships um ON um.organization_id = o.id
                JOIN users u ON u.id = um.user_id
                WHERE u.email = $1
                  AND um.status = 'active'
                  AND um.role IN ('org_admin')
                ORDER BY o.created_at DESC
                "#,
                &[&email],
            )
            .await?;
        Ok(rows.iter().map(row_to_organization).collect())
    }

    pub async fn list_child_organizations(
        &self,
        parent_organization_id: Uuid,
    ) -> anyhow::Result<Vec<Organization>> {
        let client = self.pool.get().await?;
        let rows = client
            .query(
                r#"
                SELECT
                    id,
                    name,
                    parent_organization_id,
                    organization_kind,
                    workspace_slug,
                    hex_code,
                    created_at
                FROM organizations
                WHERE parent_organization_id = $1
                ORDER BY created_at DESC
                "#,
                &[&parent_organization_id],
            )
            .await?;
        Ok(rows.iter().map(row_to_organization).collect())
    }

    pub async fn ensure_org_admin_membership(
        &self,
        email: &str,
        organization_id: Uuid,
    ) -> anyhow::Result<()> {
        let client = self.pool.get().await?;
        let user = match self.get_user_by_email(email).await? {
            Some(user) => user,
            None => {
                let dev_subject = format!("dev-{}", email);
                self.upsert_user(email, &dev_subject, email).await?
            }
        };

        client
            .execute(
                r#"
                INSERT INTO user_memberships (user_id, organization_id, role, status)
                SELECT $1, $2, 'org_admin', 'active'
                WHERE NOT EXISTS (
                    SELECT 1
                    FROM user_memberships
                    WHERE user_id = $1
                      AND organization_id = $2
                      AND role = 'org_admin'
                      AND status = 'active'
                )
                "#,
                &[&user.id, &organization_id],
            )
            .await?;
        Ok(())
    }

    pub async fn email_has_org_manager_role(
        &self,
        email: &str,
        organization_id: Uuid,
    ) -> anyhow::Result<bool> {
        let client = self.pool.get().await?;
        let row = client
            .query_one(
                r#"
                SELECT EXISTS(
                    SELECT 1
                    FROM user_memberships um
                    JOIN users u ON u.id = um.user_id
                    WHERE u.email = $1
                      AND um.status = 'active'
                      AND (
                        (um.organization_id = $2 AND um.role IN ('org_admin'))
                        OR (um.organization_id IS NULL AND um.project_id IS NULL AND um.role = 'platform_admin')
                      )
                ) AS has_access
                "#,
                &[&email, &organization_id],
            )
            .await?;
        Ok(row.get("has_access"))
    }

    pub async fn create_patient(
        &self,
        site_id: Uuid,
        external_subject_id: Option<&str>,
        email: Option<&str>,
        date_of_birth: Option<NaiveDate>,
    ) -> anyhow::Result<Patient> {
        let client = self.pool.get().await?;
        let relation = client
            .query_one(
                r#"
                SELECT p.id AS project_id, p.organization_id
                FROM sites s
                JOIN projects p ON p.id = s.project_id
                WHERE s.id = $1
                "#,
                &[&site_id],
            )
            .await?;

        let project_id: Uuid = relation.get("project_id");
        let organization_id: Uuid = relation.get("organization_id");
        let site_hex = self.ensure_site_hex_code(&client, site_id).await?;
        let hex_code = self.generate_unique_patient_hex(&client, &site_hex).await?;

        let row = client
            .query_one(
                r#"
                INSERT INTO patients (
                    organization_id,
                    project_id,
                    site_id,
                    external_subject_id,
                    email,
                    date_of_birth,
                    hex_code
                )
                VALUES ($1, $2, $3, $4, $5, $6, $7)
                RETURNING
                    id,
                    organization_id,
                    project_id,
                    site_id,
                    external_subject_id,
                    email,
                    date_of_birth,
                    hex_code,
                    created_at
                "#,
                &[
                    &organization_id,
                    &project_id,
                    &site_id,
                    &external_subject_id,
                    &email,
                    &date_of_birth,
                    &hex_code,
                ],
            )
            .await?;
        Ok(row_to_patient(&row))
    }

    pub async fn list_patients_by_project(&self, project_id: Uuid) -> anyhow::Result<Vec<Patient>> {
        let client = self.pool.get().await?;
        let rows = client
            .query(
                r#"
                SELECT
                    id,
                    organization_id,
                    project_id,
                    site_id,
                    external_subject_id,
                    email,
                    date_of_birth,
                    hex_code,
                    created_at
                FROM patients
                WHERE project_id = $1
                ORDER BY created_at DESC
                "#,
                &[&project_id],
            )
            .await?;
        Ok(rows.iter().map(row_to_patient).collect())
    }

    pub async fn create_provider(
        &self,
        organization_id: Uuid,
        name: &str,
        title: &str,
        referral_source: &str,
    ) -> anyhow::Result<Provider> {
        let client = self.pool.get().await?;
        let org_hex = self
            .ensure_organization_hex_code(&client, organization_id)
            .await?;
        let hex_code = self.generate_unique_provider_hex(&client, &org_hex).await?;
        let row = client
            .query_one(
                r#"
                INSERT INTO providers (organization_id, name, title, referral_source, hex_code)
                VALUES ($1, $2, $3, $4, $5)
                RETURNING id, organization_id, name, title, referral_source, hex_code, created_at
                "#,
                &[&organization_id, &name, &title, &referral_source, &hex_code],
            )
            .await?;
        Ok(row_to_provider(&row))
    }

    pub async fn list_providers_by_organization(
        &self,
        organization_id: Uuid,
    ) -> anyhow::Result<Vec<Provider>> {
        let client = self.pool.get().await?;
        let rows = client
            .query(
                r#"
                SELECT id, organization_id, name, title, referral_source, hex_code, created_at
                FROM providers
                WHERE organization_id = $1
                ORDER BY created_at DESC
                "#,
                &[&organization_id],
            )
            .await?;
        Ok(rows.iter().map(row_to_provider).collect())
    }

    pub async fn get_patient(&self, patient_id: Uuid) -> anyhow::Result<Option<Patient>> {
        let client = self.pool.get().await?;
        let row = client
            .query_opt(
                r#"
                SELECT
                    id,
                    organization_id,
                    project_id,
                    site_id,
                    external_subject_id,
                    email,
                    date_of_birth,
                    hex_code,
                    created_at
                FROM patients
                WHERE id = $1
                "#,
                &[&patient_id],
            )
            .await?;
        Ok(row.as_ref().map(row_to_patient))
    }

    pub async fn create_encounter(
        &self,
        patient_id: Uuid,
        encounter_type: &str,
        provider_id: Option<Uuid>,
        notes: &str,
    ) -> anyhow::Result<Encounter> {
        let normalized_type = normalize_encounter_type(encounter_type)
            .ok_or_else(|| anyhow!("unsupported encounter_type"))?;
        let client = self.pool.get().await?;
        let patient_hex = self.ensure_patient_hex_code(&client, patient_id).await?;
        let (range_start, range_end) = encounter_range(&normalized_type);

        let row = client
            .query_one(
                r#"
                SELECT COUNT(*)::BIGINT AS total
                FROM encounters
                WHERE patient_id = $1 AND encounter_type = $2
                "#,
                &[&patient_id, &normalized_type],
            )
            .await?;
        let current_total: i64 = row.get("total");
        let next_value = range_start + current_total as i32;
        if next_value > range_end {
            return Err(anyhow!(
                "encounter code range exhausted for encounter_type={normalized_type}"
            ));
        }
        let suffix = format!("{next_value:03X}");
        let hex_code = format!("{patient_hex}{suffix}");

        let row = client
            .query_one(
                r#"
                INSERT INTO encounters (
                    patient_id,
                    provider_id,
                    encounter_type,
                    notes,
                    hex_code
                )
                VALUES ($1, $2, $3, $4, $5)
                RETURNING
                    id,
                    patient_id,
                    provider_id,
                    encounter_type,
                    notes,
                    hex_code,
                    created_at
                "#,
                &[
                    &patient_id,
                    &provider_id,
                    &normalized_type,
                    &notes,
                    &hex_code,
                ],
            )
            .await?;
        Ok(row_to_encounter(&row))
    }

    pub async fn list_encounters_by_patient(
        &self,
        patient_id: Uuid,
    ) -> anyhow::Result<Vec<Encounter>> {
        let client = self.pool.get().await?;
        let rows = client
            .query(
                r#"
                SELECT
                    id,
                    patient_id,
                    provider_id,
                    encounter_type,
                    notes,
                    hex_code,
                    created_at
                FROM encounters
                WHERE patient_id = $1
                ORDER BY created_at DESC
                "#,
                &[&patient_id],
            )
            .await?;
        Ok(rows.iter().map(row_to_encounter).collect())
    }

    async fn ensure_organization_hex_code(
        &self,
        client: &deadpool_postgres::Client,
        organization_id: Uuid,
    ) -> anyhow::Result<String> {
        let row = client
            .query_opt(
                "SELECT hex_code FROM organizations WHERE id = $1",
                &[&organization_id],
            )
            .await?
            .ok_or_else(|| anyhow!("organization not found"))?;
        let current_code: Option<String> = row.get("hex_code");
        if let Some(code) = current_code {
            if !code.trim().is_empty() {
                return Ok(code);
            }
        }

        let candidate = self.generate_unique_org_hex(client).await?;
        client
            .execute(
                "UPDATE organizations SET hex_code = $1 WHERE id = $2",
                &[&candidate, &organization_id],
            )
            .await?;
        Ok(candidate)
    }

    async fn ensure_project_hex_code(
        &self,
        client: &deadpool_postgres::Client,
        project_id: Uuid,
    ) -> anyhow::Result<String> {
        let row = client
            .query_opt(
                "SELECT organization_id, hex_code FROM projects WHERE id = $1",
                &[&project_id],
            )
            .await?
            .ok_or_else(|| anyhow!("project not found"))?;
        let current_code: Option<String> = row.get("hex_code");
        if let Some(code) = current_code {
            if !code.trim().is_empty() {
                return Ok(code);
            }
        }
        let organization_id: Uuid = row.get("organization_id");
        let org_hex = self
            .ensure_organization_hex_code(client, organization_id)
            .await?;
        let candidate = self.generate_unique_project_hex(client, &org_hex).await?;
        client
            .execute(
                "UPDATE projects SET hex_code = $1 WHERE id = $2",
                &[&candidate, &project_id],
            )
            .await?;
        Ok(candidate)
    }

    async fn ensure_site_hex_code(
        &self,
        client: &deadpool_postgres::Client,
        site_id: Uuid,
    ) -> anyhow::Result<String> {
        let row = client
            .query_opt(
                "SELECT project_id, hex_code FROM sites WHERE id = $1",
                &[&site_id],
            )
            .await?
            .ok_or_else(|| anyhow!("site not found"))?;
        let current_code: Option<String> = row.get("hex_code");
        if let Some(code) = current_code {
            if !code.trim().is_empty() {
                return Ok(code);
            }
        }
        let project_id: Uuid = row.get("project_id");
        let project_hex = self.ensure_project_hex_code(client, project_id).await?;
        let candidate = self.generate_unique_site_hex(client, &project_hex).await?;
        client
            .execute(
                "UPDATE sites SET hex_code = $1 WHERE id = $2",
                &[&candidate, &site_id],
            )
            .await?;
        Ok(candidate)
    }

    async fn ensure_patient_hex_code(
        &self,
        client: &deadpool_postgres::Client,
        patient_id: Uuid,
    ) -> anyhow::Result<String> {
        let row = client
            .query_opt(
                "SELECT site_id, hex_code FROM patients WHERE id = $1",
                &[&patient_id],
            )
            .await?
            .ok_or_else(|| anyhow!("patient not found"))?;
        let current_code: Option<String> = row.get("hex_code");
        if let Some(code) = current_code {
            if !code.trim().is_empty() {
                return Ok(code);
            }
        }
        let site_id: Option<Uuid> = row.get("site_id");
        let site_id = site_id.ok_or_else(|| anyhow!("patient has no site_id for hex cascading"))?;
        let site_hex = self.ensure_site_hex_code(client, site_id).await?;
        let candidate = self.generate_unique_patient_hex(client, &site_hex).await?;
        client
            .execute(
                "UPDATE patients SET hex_code = $1 WHERE id = $2",
                &[&candidate, &patient_id],
            )
            .await?;
        Ok(candidate)
    }

    async fn generate_unique_org_hex(
        &self,
        client: &deadpool_postgres::Client,
    ) -> anyhow::Result<String> {
        for _ in 0..1024 {
            let candidate = random_hex_segment(3, 1);
            let row = client
                .query_opt(
                    "SELECT 1 FROM organizations WHERE hex_code = $1 LIMIT 1",
                    &[&candidate],
                )
                .await?;
            if row.is_none() {
                return Ok(candidate);
            }
        }
        Err(anyhow!("unable to allocate unique organization hex code"))
    }

    async fn generate_unique_project_hex(
        &self,
        client: &deadpool_postgres::Client,
        org_hex: &str,
    ) -> anyhow::Result<String> {
        for _ in 0..4096 {
            let candidate = format!("{org_hex}{}", random_hex_segment(3, 1));
            let row = client
                .query_opt(
                    "SELECT 1 FROM projects WHERE hex_code = $1 LIMIT 1",
                    &[&candidate],
                )
                .await?;
            if row.is_none() {
                return Ok(candidate);
            }
        }
        Err(anyhow!("unable to allocate unique project hex code"))
    }

    async fn generate_unique_site_hex(
        &self,
        client: &deadpool_postgres::Client,
        project_hex: &str,
    ) -> anyhow::Result<String> {
        for _ in 0..4096 {
            let candidate = format!("{project_hex}{}", random_hex_segment(3, 1));
            let row = client
                .query_opt(
                    "SELECT 1 FROM sites WHERE hex_code = $1 LIMIT 1",
                    &[&candidate],
                )
                .await?;
            if row.is_none() {
                return Ok(candidate);
            }
        }
        Err(anyhow!("unable to allocate unique site hex code"))
    }

    async fn generate_unique_patient_hex(
        &self,
        client: &deadpool_postgres::Client,
        site_hex: &str,
    ) -> anyhow::Result<String> {
        for _ in 0..8192 {
            let candidate = format!("{site_hex}{}", random_hex_segment(4, 1));
            let row = client
                .query_opt(
                    "SELECT 1 FROM patients WHERE hex_code = $1 LIMIT 1",
                    &[&candidate],
                )
                .await?;
            if row.is_none() {
                return Ok(candidate);
            }
        }
        Err(anyhow!("unable to allocate unique patient hex code"))
    }

    async fn generate_unique_provider_hex(
        &self,
        client: &deadpool_postgres::Client,
        org_hex: &str,
    ) -> anyhow::Result<String> {
        for _ in 0..4096 {
            let candidate = format!("{org_hex}{}", random_hex_segment(3, 1));
            let row = client
                .query_opt(
                    "SELECT 1 FROM providers WHERE hex_code = $1 LIMIT 1",
                    &[&candidate],
                )
                .await?;
            if row.is_none() {
                return Ok(candidate);
            }
        }
        Err(anyhow!("unable to allocate unique provider hex code"))
    }

    async fn get_platform_root_organization_id(
        &self,
        client: &deadpool_postgres::Client,
    ) -> anyhow::Result<Uuid> {
        let row = client
            .query_opt(
                r#"
                SELECT id
                FROM organizations
                WHERE organization_kind = 'platform_root'
                ORDER BY created_at ASC
                LIMIT 1
                "#,
                &[],
            )
            .await?;
        match row {
            Some(row) => Ok(row.get("id")),
            None => Err(anyhow!(
                "platform_root organization not found; run latest migrations first"
            )),
        }
    }

    async fn generate_unique_workspace_slug(
        &self,
        client: &deadpool_postgres::Client,
        name: &str,
    ) -> anyhow::Result<String> {
        let base_slug = slugify_workspace(name);
        for attempt in 0..1024 {
            let candidate = if attempt == 0 {
                base_slug.clone()
            } else {
                format!("{base_slug}-{attempt}")
            };
            let row = client
                .query_opt(
                    "SELECT 1 FROM organizations WHERE workspace_slug = $1 LIMIT 1",
                    &[&candidate],
                )
                .await?;
            if row.is_none() {
                return Ok(candidate);
            }
        }
        Err(anyhow!("unable to allocate unique workspace_slug"))
    }

    pub async fn queue_hospital_signing_email(
        &self,
        agreement_id: Uuid,
        requested_by_user_id: Option<Uuid>,
        app_base_url: &str,
    ) -> anyhow::Result<OutboundEmail> {
        let agreement = self
            .get_data_use_agreement(agreement_id)
            .await?
            .ok_or_else(|| anyhow!("data use agreement not found"))?;

        let signing_url = format!(
            "{}/ui/dua/sign/{}",
            app_base_url.trim_end_matches('/'),
            agreement.hospital_signing_token
        );
        let subject = format!(
            "Signature request: Data Use Agreement with {}",
            agreement.counterparty_name
        );
        let body = format!(
            "Hello {},\n\nA Data Use Agreement is ready for your electronic signature.\n\nHospital: {}\nCounterparty: {}\nAgreement ID: {}\n\nReview and sign here:\n{}\n\nThank you,\nCingulum Foundation Inc.",
            agreement.hospital_contact_name,
            agreement.hospital_name,
            agreement.counterparty_name,
            agreement.id,
            signing_url
        );

        let client = self.pool.get().await?;
        let row = client
            .query_one(
                r#"
                INSERT INTO outbound_emails (
                    agreement_id,
                    recipient_email,
                    subject,
                    body,
                    status,
                    requested_by_user_id
                )
                VALUES ($1, $2, $3, $4, 'queued', $5)
                RETURNING
                    id,
                    agreement_id,
                    recipient_email,
                    subject,
                    body,
                    status,
                    requested_by_user_id,
                    created_at
                "#,
                &[
                    &agreement.id,
                    &agreement.hospital_contact_email,
                    &subject,
                    &body,
                    &requested_by_user_id,
                ],
            )
            .await?;
        Ok(row_to_outbound_email(&row))
    }

    pub async fn list_outbound_emails_for_agreement(
        &self,
        agreement_id: Uuid,
    ) -> anyhow::Result<Vec<OutboundEmail>> {
        let client = self.pool.get().await?;
        let rows = client
            .query(
                r#"
                SELECT
                    id,
                    agreement_id,
                    recipient_email,
                    subject,
                    body,
                    status,
                    requested_by_user_id,
                    created_at
                FROM outbound_emails
                WHERE agreement_id = $1
                ORDER BY created_at DESC
                "#,
                &[&agreement_id],
            )
            .await?;
        Ok(rows.iter().map(row_to_outbound_email).collect())
    }

    pub async fn create_data_use_agreement(
        &self,
        organization_id: Uuid,
        hospital_name: &str,
        hospital_contact_name: &str,
        hospital_contact_email: &str,
        agreement_version: &str,
        effective_date: Option<chrono::NaiveDate>,
        expiration_date: Option<chrono::NaiveDate>,
        agreement_text: &str,
        created_by_user_id: Option<Uuid>,
    ) -> anyhow::Result<DataUseAgreement> {
        let hospital_signing_token = Uuid::new_v4();
        let client = self.pool.get().await?;
        let row = client
            .query_one(
                r#"
                INSERT INTO data_use_agreements (
                    organization_id,
                    hospital_name,
                    hospital_contact_name,
                    hospital_contact_email,
                    agreement_version,
                    effective_date,
                    expiration_date,
                    agreement_text,
                    hospital_signing_token,
                    created_by_user_id
                )
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
                RETURNING
                    id,
                    organization_id,
                    hospital_name,
                    hospital_contact_name,
                    hospital_contact_email,
                    counterparty_name,
                    agreement_version,
                    status,
                    effective_date,
                    expiration_date,
                    agreement_text,
                    hospital_signing_token,
                    created_by_user_id,
                    signed_at,
                    created_at,
                    updated_at
                "#,
                &[
                    &organization_id,
                    &hospital_name,
                    &hospital_contact_name,
                    &hospital_contact_email,
                    &agreement_version,
                    &effective_date,
                    &expiration_date,
                    &agreement_text,
                    &hospital_signing_token,
                    &created_by_user_id,
                ],
            )
            .await?;
        Ok(row_to_data_use_agreement(&row))
    }

    pub async fn list_data_use_agreements(
        &self,
        organization_id: Uuid,
    ) -> anyhow::Result<Vec<DataUseAgreement>> {
        let client = self.pool.get().await?;
        let rows = client
            .query(
                r#"
                SELECT
                    id,
                    organization_id,
                    hospital_name,
                    hospital_contact_name,
                    hospital_contact_email,
                    counterparty_name,
                    agreement_version,
                    status,
                    effective_date,
                    expiration_date,
                    agreement_text,
                    hospital_signing_token,
                    created_by_user_id,
                    signed_at,
                    created_at,
                    updated_at
                FROM data_use_agreements
                WHERE organization_id = $1
                ORDER BY created_at DESC
                "#,
                &[&organization_id],
            )
            .await?;
        Ok(rows.iter().map(row_to_data_use_agreement).collect())
    }

    pub async fn get_data_use_agreement(
        &self,
        agreement_id: Uuid,
    ) -> anyhow::Result<Option<DataUseAgreement>> {
        let client = self.pool.get().await?;
        let row = client
            .query_opt(
                r#"
                SELECT
                    id,
                    organization_id,
                    hospital_name,
                    hospital_contact_name,
                    hospital_contact_email,
                    counterparty_name,
                    agreement_version,
                    status,
                    effective_date,
                    expiration_date,
                    agreement_text,
                    hospital_signing_token,
                    created_by_user_id,
                    signed_at,
                    created_at,
                    updated_at
                FROM data_use_agreements
                WHERE id = $1
                "#,
                &[&agreement_id],
            )
            .await?;
        Ok(row.as_ref().map(row_to_data_use_agreement))
    }

    pub async fn get_data_use_agreement_by_signing_token(
        &self,
        signing_token: Uuid,
    ) -> anyhow::Result<Option<DataUseAgreement>> {
        let client = self.pool.get().await?;
        let row = client
            .query_opt(
                r#"
                SELECT
                    id,
                    organization_id,
                    hospital_name,
                    hospital_contact_name,
                    hospital_contact_email,
                    counterparty_name,
                    agreement_version,
                    status,
                    effective_date,
                    expiration_date,
                    agreement_text,
                    hospital_signing_token,
                    created_by_user_id,
                    signed_at,
                    created_at,
                    updated_at
                FROM data_use_agreements
                WHERE hospital_signing_token = $1
                "#,
                &[&signing_token],
            )
            .await?;
        Ok(row.as_ref().map(row_to_data_use_agreement))
    }

    pub async fn list_data_use_agreement_signatures(
        &self,
        agreement_id: Uuid,
    ) -> anyhow::Result<Vec<DataUseAgreementSignature>> {
        let client = self.pool.get().await?;
        let rows = client
            .query(
                r#"
                SELECT
                    id,
                    agreement_id,
                    signer_role,
                    signer_name,
                    signer_email,
                    signer_title,
                    signer_organization,
                    signature_method,
                    signature_text,
                    ip_address::TEXT AS ip_address,
                    signed_by_user_id,
                    signed_at
                FROM data_use_agreement_signatures
                WHERE agreement_id = $1
                ORDER BY signed_at ASC
                "#,
                &[&agreement_id],
            )
            .await?;
        Ok(rows
            .iter()
            .map(row_to_data_use_agreement_signature)
            .collect())
    }

    pub async fn sign_data_use_agreement_as_cingulum(
        &self,
        agreement_id: Uuid,
        signer_name: &str,
        signer_email: &str,
        signer_title: &str,
        signature_method: &str,
        signature_text: &str,
        ip_address: Option<IpAddr>,
        signed_by_user_id: Option<Uuid>,
    ) -> anyhow::Result<DataUseAgreementSignature> {
        let client = self.pool.get().await?;
        let row = client
            .query_one(
                r#"
                INSERT INTO data_use_agreement_signatures (
                    agreement_id,
                    signer_role,
                    signer_name,
                    signer_email,
                    signer_title,
                    signer_organization,
                    signature_method,
                    signature_text,
                    ip_address,
                    signed_by_user_id
                )
                VALUES ($1, 'cingulum', $2, $3, $4, 'Cingulum Foundation Inc.', $5, $6, $7, $8)
                ON CONFLICT (agreement_id, signer_role)
                DO UPDATE SET
                    signer_name = EXCLUDED.signer_name,
                    signer_email = EXCLUDED.signer_email,
                    signer_title = EXCLUDED.signer_title,
                    signer_organization = EXCLUDED.signer_organization,
                    signature_method = EXCLUDED.signature_method,
                    signature_text = EXCLUDED.signature_text,
                    ip_address = EXCLUDED.ip_address,
                    signed_by_user_id = EXCLUDED.signed_by_user_id,
                    signed_at = NOW()
                RETURNING
                    id,
                    agreement_id,
                    signer_role,
                    signer_name,
                    signer_email,
                    signer_title,
                    signer_organization,
                    signature_method,
                    signature_text,
                    ip_address::TEXT AS ip_address,
                    signed_by_user_id,
                    signed_at
                "#,
                &[
                    &agreement_id,
                    &signer_name,
                    &signer_email,
                    &signer_title,
                    &signature_method,
                    &signature_text,
                    &ip_address,
                    &signed_by_user_id,
                ],
            )
            .await?;

        self.refresh_data_use_agreement_status(&client, agreement_id)
            .await?;

        Ok(row_to_data_use_agreement_signature(&row))
    }

    pub async fn sign_data_use_agreement_by_token(
        &self,
        signing_token: Uuid,
        signer_name: &str,
        signer_email: &str,
        signer_title: &str,
        signer_organization: &str,
        signature_method: &str,
        signature_text: &str,
        ip_address: Option<IpAddr>,
    ) -> anyhow::Result<(DataUseAgreement, DataUseAgreementSignature)> {
        let client = self.pool.get().await?;
        let agreement_row = client
            .query_opt(
                r#"
                SELECT
                    id,
                    organization_id,
                    hospital_name,
                    hospital_contact_name,
                    hospital_contact_email,
                    counterparty_name,
                    agreement_version,
                    status,
                    effective_date,
                    expiration_date,
                    agreement_text,
                    hospital_signing_token,
                    created_by_user_id,
                    signed_at,
                    created_at,
                    updated_at
                FROM data_use_agreements
                WHERE hospital_signing_token = $1
                "#,
                &[&signing_token],
            )
            .await?;

        let agreement_row =
            agreement_row.ok_or_else(|| anyhow!("data use agreement not found for token"))?;
        let agreement_id: Uuid = agreement_row.get("id");

        let signature_row = client
            .query_one(
                r#"
                INSERT INTO data_use_agreement_signatures (
                    agreement_id,
                    signer_role,
                    signer_name,
                    signer_email,
                    signer_title,
                    signer_organization,
                    signature_method,
                    signature_text,
                    ip_address
                )
                VALUES ($1, 'hospital', $2, $3, $4, $5, $6, $7, $8)
                ON CONFLICT (agreement_id, signer_role)
                DO UPDATE SET
                    signer_name = EXCLUDED.signer_name,
                    signer_email = EXCLUDED.signer_email,
                    signer_title = EXCLUDED.signer_title,
                    signer_organization = EXCLUDED.signer_organization,
                    signature_method = EXCLUDED.signature_method,
                    signature_text = EXCLUDED.signature_text,
                    ip_address = EXCLUDED.ip_address,
                    signed_at = NOW()
                RETURNING
                    id,
                    agreement_id,
                    signer_role,
                    signer_name,
                    signer_email,
                    signer_title,
                    signer_organization,
                    signature_method,
                    signature_text,
                    ip_address::TEXT AS ip_address,
                    signed_by_user_id,
                    signed_at
                "#,
                &[
                    &agreement_id,
                    &signer_name,
                    &signer_email,
                    &signer_title,
                    &signer_organization,
                    &signature_method,
                    &signature_text,
                    &ip_address,
                ],
            )
            .await?;

        self.refresh_data_use_agreement_status(&client, agreement_id)
            .await?;

        let updated_agreement_row = client
            .query_one(
                r#"
                SELECT
                    id,
                    organization_id,
                    hospital_name,
                    hospital_contact_name,
                    hospital_contact_email,
                    counterparty_name,
                    agreement_version,
                    status,
                    effective_date,
                    expiration_date,
                    agreement_text,
                    hospital_signing_token,
                    created_by_user_id,
                    signed_at,
                    created_at,
                    updated_at
                FROM data_use_agreements
                WHERE id = $1
                "#,
                &[&agreement_id],
            )
            .await?;

        Ok((
            row_to_data_use_agreement(&updated_agreement_row),
            row_to_data_use_agreement_signature(&signature_row),
        ))
    }

    async fn refresh_data_use_agreement_status(
        &self,
        client: &deadpool_postgres::Client,
        agreement_id: Uuid,
    ) -> anyhow::Result<()> {
        client
            .execute(
                r#"
                WITH signature_flags AS (
                    SELECT
                        EXISTS(
                            SELECT 1
                            FROM data_use_agreement_signatures
                            WHERE agreement_id = $1 AND signer_role = 'hospital'
                        ) AS has_hospital_signature,
                        EXISTS(
                            SELECT 1
                            FROM data_use_agreement_signatures
                            WHERE agreement_id = $1 AND signer_role = 'cingulum'
                        ) AS has_cingulum_signature
                )
                UPDATE data_use_agreements dua
                SET
                    status = CASE
                        WHEN sf.has_hospital_signature AND sf.has_cingulum_signature
                            THEN 'active'
                        ELSE dua.status
                    END,
                    signed_at = CASE
                        WHEN sf.has_hospital_signature AND sf.has_cingulum_signature
                            THEN COALESCE(dua.signed_at, NOW())
                        ELSE dua.signed_at
                    END,
                    updated_at = NOW()
                FROM signature_flags sf
                WHERE dua.id = $1
                "#,
                &[&agreement_id],
            )
            .await?;
        Ok(())
    }
}

fn normalize_encounter_type(raw: &str) -> Option<String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "outpatient" => Some("outpatient".to_string()),
        "inpatient" => Some("inpatient".to_string()),
        "labs" => Some("labs".to_string()),
        "imaging" => Some("imaging".to_string()),
        "procedures" => Some("procedures".to_string()),
        "misc" => Some("misc".to_string()),
        _ => None,
    }
}

fn normalize_organization_kind(raw: &str) -> Option<String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "platform_root" => Some("platform_root".to_string()),
        "research_network" => Some("research_network".to_string()),
        "hospital" => Some("hospital".to_string()),
        "tenant" => Some("tenant".to_string()),
        "sponsor" => Some("sponsor".to_string()),
        _ => None,
    }
}

fn slugify_workspace(raw: &str) -> String {
    let mut slug = String::new();
    let mut previous_was_dash = false;
    for ch in raw.chars() {
        let mapped = match ch {
            'A'..='Z' => ch.to_ascii_lowercase(),
            'a'..='z' | '0'..='9' => ch,
            _ => '-',
        };
        if mapped == '-' {
            if !previous_was_dash && !slug.is_empty() {
                slug.push('-');
                previous_was_dash = true;
            }
            continue;
        }
        previous_was_dash = false;
        slug.push(mapped);
    }
    let slug = slug.trim_matches('-').to_string();
    if slug.is_empty() {
        "workspace".to_string()
    } else {
        slug
    }
}

fn normalize_study_phase(raw: &str) -> Option<String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "pre_study" => Some("pre_study".to_string()),
        "initiated" => Some("initiated".to_string()),
        "active" => Some("active".to_string()),
        "monitoring" => Some("monitoring".to_string()),
        "closed" => Some("closed".to_string()),
        _ => None,
    }
}

fn is_valid_phase_transition(current: &str, target: &str) -> bool {
    matches!(
        (current, target),
        ("pre_study", "initiated")
            | ("initiated", "active")
            | ("active", "monitoring")
            | ("active", "closed")
            | ("monitoring", "active")
            | ("monitoring", "closed")
    )
}

fn normalize_crf_field_type(raw: &str) -> Option<String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "text" => Some("text".to_string()),
        "textarea" => Some("textarea".to_string()),
        "number" => Some("number".to_string()),
        "date" => Some("date".to_string()),
        "datetime" => Some("datetime".to_string()),
        "boolean" => Some("boolean".to_string()),
        "single_select" => Some("single_select".to_string()),
        "multi_select" => Some("multi_select".to_string()),
        _ => None,
    }
}

fn encounter_range(encounter_type: &str) -> (i32, i32) {
    match encounter_type {
        "outpatient" => (0x000, 0x2FF),
        "inpatient" => (0x300, 0x5FF),
        "labs" => (0x600, 0x8FF),
        "imaging" => (0x900, 0xBFF),
        "procedures" => (0xC00, 0xEFF),
        "misc" => (0xF00, 0xFFF),
        _ => (0xF00, 0xFFF),
    }
}

fn random_hex_segment(len: usize, min_letters: usize) -> String {
    loop {
        let raw = Uuid::new_v4().as_simple().to_string().to_uppercase();
        let candidate = &raw[..len];
        let letter_count = candidate.chars().filter(|c| matches!(c, 'A'..='F')).count();
        if letter_count >= min_letters {
            return candidate.to_string();
        }
    }
}

fn row_to_data_use_agreement(row: &Row) -> DataUseAgreement {
    DataUseAgreement {
        id: row.get("id"),
        organization_id: row.get("organization_id"),
        hospital_name: row.get("hospital_name"),
        hospital_contact_name: row.get("hospital_contact_name"),
        hospital_contact_email: row.get("hospital_contact_email"),
        counterparty_name: row.get("counterparty_name"),
        agreement_version: row.get("agreement_version"),
        status: row.get("status"),
        effective_date: row.get("effective_date"),
        expiration_date: row.get("expiration_date"),
        agreement_text: row.get("agreement_text"),
        hospital_signing_token: row.get("hospital_signing_token"),
        created_by_user_id: row.get("created_by_user_id"),
        signed_at: row.get("signed_at"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

fn row_to_data_use_agreement_signature(row: &Row) -> DataUseAgreementSignature {
    DataUseAgreementSignature {
        id: row.get("id"),
        agreement_id: row.get("agreement_id"),
        signer_role: row.get("signer_role"),
        signer_name: row.get("signer_name"),
        signer_email: row.get("signer_email"),
        signer_title: row.get("signer_title"),
        signer_organization: row.get("signer_organization"),
        signature_method: row.get("signature_method"),
        signature_text: row.get("signature_text"),
        ip_address: row.get("ip_address"),
        signed_by_user_id: row.get("signed_by_user_id"),
        signed_at: row.get("signed_at"),
    }
}

fn row_to_outbound_email(row: &Row) -> OutboundEmail {
    OutboundEmail {
        id: row.get("id"),
        agreement_id: row.get("agreement_id"),
        recipient_email: row.get("recipient_email"),
        subject: row.get("subject"),
        body: row.get("body"),
        status: row.get("status"),
        requested_by_user_id: row.get("requested_by_user_id"),
        created_at: row.get("created_at"),
    }
}

fn row_to_organization(row: &Row) -> Organization {
    Organization {
        id: row.get("id"),
        name: row.get("name"),
        parent_organization_id: row.get("parent_organization_id"),
        organization_kind: row.get("organization_kind"),
        workspace_slug: row.get("workspace_slug"),
        hex_code: row.get("hex_code"),
        created_at: row.get("created_at"),
    }
}

fn row_to_project(row: &Row) -> Project {
    Project {
        id: row.get("id"),
        organization_id: row.get("organization_id"),
        name: row.get("name"),
        therapeutic_area: row.get("therapeutic_area"),
        protocol_code: row.get("protocol_code"),
        lifecycle_phase: row.get("lifecycle_phase"),
        planned_enrollment: row.get("planned_enrollment"),
        clinicaltrials_gov_id: row.get("clinicaltrials_gov_id"),
        study_summary: row.get("study_summary"),
        phase_changed_at: row.get("phase_changed_at"),
        hex_code: row.get("hex_code"),
        created_at: row.get("created_at"),
    }
}

fn row_to_patient(row: &Row) -> Patient {
    Patient {
        id: row.get("id"),
        organization_id: row.get("organization_id"),
        project_id: row.get("project_id"),
        site_id: row.get("site_id"),
        external_subject_id: row.get("external_subject_id"),
        email: row.get("email"),
        date_of_birth: row.get("date_of_birth"),
        hex_code: row.get("hex_code"),
        created_at: row.get("created_at"),
    }
}

fn row_to_study_phase_event(row: &Row) -> StudyPhaseEvent {
    StudyPhaseEvent {
        id: row.get("id"),
        project_id: row.get("project_id"),
        previous_phase: row.get("previous_phase"),
        new_phase: row.get("new_phase"),
        changed_by_user_id: row.get("changed_by_user_id"),
        notes: row.get("notes"),
        created_at: row.get("created_at"),
    }
}

fn row_to_study_crf_template(row: &Row) -> StudyCrfTemplate {
    StudyCrfTemplate {
        id: row.get("id"),
        project_id: row.get("project_id"),
        name: row.get("name"),
        description: row.get("description"),
        version: row.get("version"),
        status: row.get("status"),
        applicable_phase: row.get("applicable_phase"),
        created_by_user_id: row.get("created_by_user_id"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

fn row_to_study_crf_field(row: &Row) -> StudyCrfField {
    StudyCrfField {
        id: row.get("id"),
        template_id: row.get("template_id"),
        field_key: row.get("field_key"),
        field_label: row.get("field_label"),
        field_type: row.get("field_type"),
        required: row.get("required"),
        options_json: row.get("options_json"),
        display_order: row.get("display_order"),
        created_at: row.get("created_at"),
    }
}

fn row_to_study_visit_template(row: &Row) -> StudyVisitTemplate {
    StudyVisitTemplate {
        id: row.get("id"),
        project_id: row.get("project_id"),
        visit_code: row.get("visit_code"),
        visit_name: row.get("visit_name"),
        target_day: row.get("target_day"),
        window_before_days: row.get("window_before_days"),
        window_after_days: row.get("window_after_days"),
        required: row.get("required"),
        created_at: row.get("created_at"),
    }
}

fn row_to_patient_study_visit(row: &Row) -> PatientStudyVisit {
    PatientStudyVisit {
        id: row.get("id"),
        project_id: row.get("project_id"),
        patient_id: row.get("patient_id"),
        visit_template_id: row.get("visit_template_id"),
        scheduled_for: row.get("scheduled_for"),
        status: row.get("status"),
        completed_at: row.get("completed_at"),
        locked: row.get("locked"),
        locked_at: row.get("locked_at"),
        locked_by_user_id: row.get("locked_by_user_id"),
        created_at: row.get("created_at"),
    }
}

fn row_to_study_crf_submission(row: &Row) -> StudyCrfSubmission {
    StudyCrfSubmission {
        id: row.get("id"),
        project_id: row.get("project_id"),
        template_id: row.get("template_id"),
        patient_id: row.get("patient_id"),
        patient_visit_id: row.get("patient_visit_id"),
        answers_json: row.get("answers_json"),
        status: row.get("status"),
        entered_by_user_id: row.get("entered_by_user_id"),
        submitted_at: row.get("submitted_at"),
        locked_at: row.get("locked_at"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

fn row_to_study_data_query(row: &Row) -> StudyDataQuery {
    StudyDataQuery {
        id: row.get("id"),
        project_id: row.get("project_id"),
        submission_id: row.get("submission_id"),
        field_key: row.get("field_key"),
        query_text: row.get("query_text"),
        status: row.get("status"),
        response_text: row.get("response_text"),
        raised_by_user_id: row.get("raised_by_user_id"),
        resolved_by_user_id: row.get("resolved_by_user_id"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

fn row_to_study_close_checklist_item(row: &Row) -> StudyCloseChecklistItem {
    StudyCloseChecklistItem {
        id: row.get("id"),
        project_id: row.get("project_id"),
        item_code: row.get("item_code"),
        item_label: row.get("item_label"),
        completed: row.get("completed"),
        completed_by_user_id: row.get("completed_by_user_id"),
        completed_at: row.get("completed_at"),
        notes: row.get("notes"),
        created_at: row.get("created_at"),
    }
}

fn row_to_study_startup_checklist_item(row: &Row) -> StudyStartupChecklistItem {
    StudyStartupChecklistItem {
        id: row.get("id"),
        project_id: row.get("project_id"),
        item_code: row.get("item_code"),
        item_label: row.get("item_label"),
        completed: row.get("completed"),
        completed_by_user_id: row.get("completed_by_user_id"),
        completed_at: row.get("completed_at"),
        notes: row.get("notes"),
        created_at: row.get("created_at"),
    }
}

fn row_to_provider(row: &Row) -> Provider {
    Provider {
        id: row.get("id"),
        organization_id: row.get("organization_id"),
        name: row.get("name"),
        title: row.get("title"),
        referral_source: row.get("referral_source"),
        hex_code: row.get("hex_code"),
        created_at: row.get("created_at"),
    }
}

fn row_to_encounter(row: &Row) -> Encounter {
    Encounter {
        id: row.get("id"),
        patient_id: row.get("patient_id"),
        provider_id: row.get("provider_id"),
        encounter_type: row.get("encounter_type"),
        notes: row.get("notes"),
        hex_code: row.get("hex_code"),
        created_at: row.get("created_at"),
    }
}
