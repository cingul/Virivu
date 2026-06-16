use anyhow::anyhow;
use chrono::{Duration, NaiveDate, Utc};
use deadpool_postgres::{Manager, ManagerConfig, Pool, RecyclingMethod, Runtime};
use std::net::IpAddr;
use tokio_postgres::{NoTls, Row};
use uuid::Uuid;

use crate::models::{
    AuditLog, DataUseAgreement, DataUseAgreementSignature, Encounter, FormInvite,
    MediaUploadTicket, Organization, OrganizationSummaryRow, OutboundEmail, Patient,
    PatientSession, PatientStudyVisit, ProSubmission, Project, ProjectProgressRow, Provider, Site,
    SiteStartupChecklistItem, StudyCloseChecklistItem, StudyCrfField, StudyCrfSubmission,
    StudyCrfTemplate, StudyDataQuery, StudyOperationalSummary, StudyPhaseEvent, StudyReadiness,
    StudyStartupChecklistItem, StudyVisitTemplate, User, UserMembership,
};
use crate::workflow::{query_can_close, query_can_respond, query_is_closed, QUERY_STATUS_RESPONDED};

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

const DEFAULT_SITE_STARTUP_CHECKLIST_ITEMS: [(&str, &str); 8] = [
    ("data_use_agreement", "Data Use Agreement (DUA) Executed"),
    (
        "security_risk_assessment",
        "IT & Cybersecurity Risk Assessment Approved",
    ),
    (
        "irb_approval",
        "Local IRB/Ethics Committee Approval Documented",
    ),
    (
        "investigator_qualifications",
        "PI & Sub-I CVs and Medical Licenses Collected",
    ),
    (
        "financial_disclosure",
        "Financial Disclosure Forms Collected",
    ),
    (
        "protocol_training",
        "Protocol and EDC System Training Completed",
    ),
    ("delegation_log", "Delegation of Authority Log Signed"),
    (
        "statement_of_investigator",
        "Statement of Investigator (e.g., FDA Form 1572) Signed",
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
                    status,
                    last_activity_at,
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
                status: r.get("status"),
                last_activity_at: r.get("last_activity_at"),
                created_at: r.get("created_at"),
            })
            .collect())
    }

    pub async fn get_organization_id_by_hex(&self, hex_code: &str) -> anyhow::Result<Uuid> {
        let client = self.pool.get().await?;
        let row = client
            .query_opt(
                "SELECT id FROM organizations WHERE hex_code = $1",
                &[&hex_code.to_uppercase()],
            )
            .await?;
        match row {
            Some(r) => Ok(r.get("id")),
            None => Err(anyhow::anyhow!(
                "organization not found with hex code: {}",
                hex_code
            )),
        }
    }

    pub async fn get_project_id_by_hex(&self, hex_code: &str) -> anyhow::Result<Uuid> {
        let client = self.pool.get().await?;
        let row = client
            .query_opt(
                "SELECT id FROM projects WHERE hex_code = $1",
                &[&hex_code.to_uppercase()],
            )
            .await?;
        match row {
            Some(r) => Ok(r.get("id")),
            None => Err(anyhow::anyhow!(
                "project not found with hex code: {}",
                hex_code
            )),
        }
    }

    pub async fn get_site_id_by_hex(&self, hex_code: &str) -> anyhow::Result<Uuid> {
        let client = self.pool.get().await?;
        let row = client
            .query_opt(
                "SELECT id FROM sites WHERE hex_code = $1",
                &[&hex_code.to_uppercase()],
            )
            .await?;
        match row {
            Some(r) => Ok(r.get("id")),
            None => Err(anyhow::anyhow!(
                "site not found with hex code: {}",
                hex_code
            )),
        }
    }

    pub async fn get_patient_id_by_hex(&self, hex_code: &str) -> anyhow::Result<Uuid> {
        let client = self.pool.get().await?;
        let row = client
            .query_opt(
                "SELECT id FROM patients WHERE hex_code = $1",
                &[&hex_code.to_uppercase()],
            )
            .await?;
        match row {
            Some(r) => Ok(r.get("id")),
            None => Err(anyhow::anyhow!(
                "patient not found with hex code: {}",
                hex_code
            )),
        }
    }

    pub async fn list_sites_by_project(&self, project_id: Uuid) -> anyhow::Result<Vec<Site>> {
        let client = self.pool.get().await?;
        let rows = client
            .query(
                r#"
                SELECT id, organization_id, project_id, name, principal_investigator, co_principal_investigator, sub_investigator, hex_code, status, last_activity_at, created_at
                FROM sites
                WHERE project_id = $1
                ORDER BY created_at DESC
                "#,
                &[&project_id],
            )
            .await?;
        Ok(rows.iter().map(row_to_site).collect())
    }

    pub async fn list_sites_by_organization(
        &self,
        organization_id: Uuid,
    ) -> anyhow::Result<Vec<Site>> {
        let client = self.pool.get().await?;
        let rows = client
            .query(
                r#"
                SELECT id, organization_id, project_id, name, principal_investigator, co_principal_investigator, sub_investigator, hex_code, status, last_activity_at, created_at
                FROM sites
                WHERE organization_id = $1
                ORDER BY created_at DESC
                "#,
                &[&organization_id],
            )
            .await?;
        Ok(rows.iter().map(row_to_site).collect())
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
                    status,
                    last_activity_at,
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
            status: row.get("status"),
            last_activity_at: row.get("last_activity_at"),
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
                    status,
                    last_activity_at,
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
        branching_logic_json: Option<&str>,
        edit_checks_json: Option<&str>,
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
                    branching_logic_json,
                    edit_checks_json,
                    display_order
                )
                VALUES ($1, $2, $3, $4, $5, $6::TEXT::JSONB, $7::TEXT::JSONB, $8::TEXT::JSONB, $9)
                RETURNING
                    id,
                    template_id,
                    field_key,
                    field_label,
                    field_type,
                    required,
                    options_json::TEXT AS options_json,
                    branching_logic_json::TEXT AS branching_logic_json,
                    edit_checks_json::TEXT AS edit_checks_json,
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
                    &branching_logic_json,
                    &edit_checks_json,
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
                    branching_logic_json::TEXT AS branching_logic_json,
                    edit_checks_json::TEXT AS edit_checks_json,
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
        branching_logic_json: Option<&str>,
        edit_checks_json: Option<&str>,
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
                    branching_logic_json = $7::TEXT::JSONB,
                    edit_checks_json = $8::TEXT::JSONB,
                    display_order = $9
                WHERE id = $1
                RETURNING
                    id,
                    template_id,
                    field_key,
                    field_label,
                    field_type,
                    required,
                    options_json::TEXT AS options_json,
                    branching_logic_json::TEXT AS branching_logic_json,
                    edit_checks_json::TEXT AS edit_checks_json,
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
                    &branching_logic_json,
                    &edit_checks_json,
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
                    branching_logic_json::TEXT AS branching_logic_json,
                    edit_checks_json::TEXT AS edit_checks_json,
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

    pub async fn delete_study_crf_fields_for_template(
        &self,
        template_id: Uuid,
    ) -> anyhow::Result<u64> {
        let client = self.pool.get().await?;
        let deleted_count = client
            .execute(
                r#"
                DELETE FROM study_crf_fields
                WHERE template_id = $1
                "#,
                &[&template_id],
            )
            .await?;
        Ok(deleted_count)
    }

    pub async fn delete_study_crf_fields_by_ids(
        &self,
        template_id: Uuid,
        field_ids: &[Uuid],
    ) -> anyhow::Result<u64> {
        if field_ids.is_empty() {
            return Ok(0);
        }
        let client = self.pool.get().await?;
        let ids = field_ids.to_vec();
        let deleted_count = client
            .execute(
                r#"
                DELETE FROM study_crf_fields
                WHERE template_id = $1
                  AND id = ANY($2)
                "#,
                &[&template_id, &ids],
            )
            .await?;
        Ok(deleted_count)
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

    /// List scheduled visits for a specific patient (used in patient portal)
    pub async fn list_patient_study_visits_for_patient(
        &self,
        patient_id: Uuid,
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
                WHERE patient_id = $1
                ORDER BY scheduled_for ASC NULLS LAST, created_at DESC
                LIMIT 50
                "#,
                &[&patient_id],
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
                    sdv_status,
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

        let site_opt: Option<Uuid> = client
            .query_opt("SELECT site_id FROM patients WHERE id = $1", &[&patient_id])
            .await?
            .and_then(|r| r.get("site_id"));
        if let Some(site_id) = site_opt {
            self.touch_site_activity(site_id).await?;
        } else {
            self.touch_project_activity(project_id).await?;
        }

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
                    sdv_status,
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
                    sdv_status,
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
                    sdv_status,
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
                    sdv_status,
                    created_at,
                    updated_at
                "#,
                &[&submission_id],
            )
            .await?;
        Ok(row_to_study_crf_submission(&row))
    }

    pub async fn update_study_crf_submission_sdv_status(
        &self,
        submission_id: Uuid,
        sdv_status: &str,
    ) -> anyhow::Result<StudyCrfSubmission> {
        let client = self.pool.get().await?;
        let row = client
            .query_one(
                r#"
                UPDATE study_crf_submissions
                SET sdv_status = $2, updated_at = NOW()
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
                    sdv_status,
                    created_at,
                    updated_at
                "#,
                &[&submission_id, &sdv_status],
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
        let submission = client
            .query_opt(
                r#"
                SELECT project_id, status
                FROM study_crf_submissions
                WHERE id = $1
                "#,
                &[&submission_id],
            )
            .await?
            .ok_or_else(|| anyhow!("conflict: submission not found"))?;
        let submission_project_id: Uuid = submission.get("project_id");
        if submission_project_id != project_id {
            return Err(anyhow!("conflict: submission does not belong to project"));
        }
        let submission_status: String = submission.get("status");
        if submission_status.trim().eq_ignore_ascii_case("draft") {
            return Err(anyhow!(
                "conflict: cannot raise data query on a draft submission"
            ));
        }
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
        if response_text.trim().is_empty() {
            return Err(anyhow!("validation: response_text is required"));
        }
        let existing = client
            .query_opt(
                "SELECT status FROM study_data_queries WHERE id = $1",
                &[&query_id],
            )
            .await?
            .ok_or_else(|| anyhow!("conflict: data query not found"))?;
        let existing_status: String = existing.get("status");
        if !query_can_respond(&existing_status) {
            return Err(anyhow!(
                "conflict: data query must be open before it can be responded"
            ));
        }
        let row = client
            .query_one(
                r#"
                UPDATE study_data_queries
                SET
                    status = $3,
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
                &[&query_id, &response_text, &QUERY_STATUS_RESPONDED],
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
        let existing = client
            .query_opt(
                "SELECT status FROM study_data_queries WHERE id = $1",
                &[&query_id],
            )
            .await?
            .ok_or_else(|| anyhow!("conflict: data query not found"))?;
        let existing_status: String = existing.get("status");
        if query_is_closed(&existing_status) {
            return Err(anyhow!("conflict: data query is already closed"));
        }
        if !query_can_close(&existing_status) {
            return Err(anyhow!(
                "conflict: data query must be responded before it can be closed"
            ));
        }
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

    pub async fn list_site_startup_checklist_items(
        &self,
        site_id: Uuid,
    ) -> anyhow::Result<Vec<SiteStartupChecklistItem>> {
        let client = self.pool.get().await?;
        let rows = client
            .query(
                r#"
                SELECT
                    id,
                    site_id,
                    item_code,
                    item_label,
                    completed,
                    completed_by_user_id,
                    completed_at,
                    notes,
                    created_at
                FROM site_startup_checklist_items
                WHERE site_id = $1
                ORDER BY created_at ASC
                "#,
                &[&site_id],
            )
            .await?;
        Ok(rows
            .iter()
            .map(row_to_site_startup_checklist_item)
            .collect())
    }

    pub async fn ensure_default_site_startup_checklist_items(
        &self,
        site_id: Uuid,
    ) -> anyhow::Result<()> {
        let client = self.pool.get().await?;
        for (item_code, item_label) in DEFAULT_SITE_STARTUP_CHECKLIST_ITEMS {
            client
                .execute(
                    r#"
                    INSERT INTO site_startup_checklist_items (site_id, item_code, item_label, completed, notes)
                    VALUES ($1, $2, $3, false, '')
                    ON CONFLICT (site_id, item_code) DO NOTHING
                    "#,
                    &[&site_id, &item_code, &item_label],
                )
                .await?;
        }
        Ok(())
    }

    pub async fn reset_site_startup_checklist_items(&self, site_id: Uuid) -> anyhow::Result<()> {
        let client = self.pool.get().await?;
        client
            .execute(
                r#"
                UPDATE site_startup_checklist_items
                SET completed = false,
                    completed_by_user_id = NULL,
                    completed_at = NULL,
                    notes = ''
                WHERE site_id = $1
                "#,
                &[&site_id],
            )
            .await?;
        Ok(())
    }

    pub async fn set_site_startup_checklist_item(
        &self,
        site_id: Uuid,
        item_code: &str,
        item_label: &str,
        completed: bool,
        completed_by_user_id: Option<Uuid>,
        notes: &str,
    ) -> anyhow::Result<SiteStartupChecklistItem> {
        let client = self.pool.get().await?;
        let row = client
            .query_one(
                r#"
                INSERT INTO site_startup_checklist_items (
                    site_id,
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
                ON CONFLICT (site_id, item_code)
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
                    site_id,
                    item_code,
                    item_label,
                    completed,
                    completed_by_user_id,
                    completed_at,
                    notes,
                    created_at
                "#,
                &[
                    &site_id,
                    &item_code,
                    &item_label,
                    &completed,
                    &completed_by_user_id,
                    &notes,
                ],
            )
            .await?;
        Ok(row_to_site_startup_checklist_item(&row))
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
        organization_id: Uuid,
        project_id: Option<Uuid>,
        name: &str,
        principal_investigator: &str,
        co_principal_investigator: Option<&str>,
        sub_investigator: Option<&str>,
    ) -> anyhow::Result<Site> {
        let client = self.pool.get().await?;

        // Hex generation: prefer project hex if attached to a study (6-char project prefix + 3 = 9 chars for site),
        // otherwise for true org-level sites use org prefix (3) + 6 random chars = 9 chars to satisfy
        // the sites_hex_code_format constraint (exactly 9 uppercase hex chars containing at least one A-F).
        let hex_code = if let Some(pid) = project_id {
            let project_hex = self.ensure_project_hex_code(&client, pid).await?;
            self.generate_unique_site_hex(&client, &project_hex).await?
        } else {
            let org_hex = self
                .ensure_organization_hex_code(&client, organization_id)
                .await?;
            self.generate_unique_org_level_site_hex(&client, &org_hex)
                .await?
        };

        let row = client
            .query_one(
                r#"
                INSERT INTO sites (organization_id, project_id, name, principal_investigator, co_principal_investigator, sub_investigator, hex_code)
                VALUES ($1, $2, $3, $4, $5, $6, $7)
                RETURNING id, organization_id, project_id, name, principal_investigator, co_principal_investigator, sub_investigator, hex_code, status, last_activity_at, created_at
                "#,
                &[&organization_id, &project_id, &name, &principal_investigator, &co_principal_investigator, &sub_investigator, &hex_code],
            )
            .await?;
        let site_id: Uuid = row.get("id");
        self.ensure_default_site_startup_checklist_items(site_id)
            .await?;

        Ok(Site {
            id: site_id,
            organization_id: row.get("organization_id"),
            project_id: row.get("project_id"),
            name: row.get("name"),
            principal_investigator: row.get("principal_investigator"),
            co_principal_investigator: row.get("co_principal_investigator"),
            sub_investigator: row.get("sub_investigator"),
            hex_code: row.get("hex_code"),
            status: row.get("status"),
            last_activity_at: row.get("last_activity_at"),
            created_at: row.get("created_at"),
        })
    }

    pub async fn delete_site(&self, site_id: Uuid) -> anyhow::Result<()> {
        let client = self.pool.get().await?;
        client
            .execute("DELETE FROM sites WHERE id = $1", &[&site_id])
            .await?;
        Ok(())
    }

    pub async fn delete_organization(&self, organization_id: Uuid) -> anyhow::Result<()> {
        let client = self.pool.get().await?;
        client
            .execute(
                "DELETE FROM organizations WHERE id = $1",
                &[&organization_id],
            )
            .await?;
        Ok(())
    }

    pub async fn reset_and_seed_db(&self) -> anyhow::Result<()> {
        let mut client = self.pool.get().await?;
        let tx = client.transaction().await?;

        // ── 0. Attempt schema migration (add new columns if missing) ──────────
        let _ = tx
            .execute(
                "ALTER TABLE media_upload_tickets ADD COLUMN IF NOT EXISTS site_id UUID",
                &[],
            )
            .await;
        let _ = tx
            .execute(
                "ALTER TABLE media_upload_tickets ADD COLUMN IF NOT EXISTS encounter_id UUID",
                &[],
            )
            .await;
        let _ = tx.execute("ALTER TABLE media_upload_tickets ADD COLUMN IF NOT EXISTS entity_type VARCHAR(64) NOT NULL DEFAULT 'patient'", &[]).await;
        let _ = tx
            .execute(
                "ALTER TABLE media_upload_tickets ADD COLUMN IF NOT EXISTS entity_id UUID",
                &[],
            )
            .await;
        let _ = tx
            .execute(
                "ALTER TABLE media_upload_tickets ADD COLUMN IF NOT EXISTS file_name VARCHAR(256)",
                &[],
            )
            .await;
        let _ = tx.execute("ALTER TABLE media_upload_tickets ADD COLUMN IF NOT EXISTS description VARCHAR(512)", &[]).await;

        // ── 1. Wipe all data in dependency order ──────────────────────────────
        tx.execute("DELETE FROM data_use_agreement_signatures", &[])
            .await?;
        tx.execute("DELETE FROM data_use_agreements", &[]).await?;
        tx.execute("DELETE FROM media_upload_tickets", &[]).await?;
        tx.execute("DELETE FROM form_invites", &[]).await?;
        tx.execute("DELETE FROM encounters", &[]).await?;
        tx.execute("DELETE FROM patient_study_visits", &[]).await?;
        tx.execute("DELETE FROM study_crf_submissions", &[]).await?;
        tx.execute("DELETE FROM study_data_queries", &[]).await?;
        tx.execute("DELETE FROM patients", &[]).await?;
        tx.execute("DELETE FROM providers", &[]).await?;
        tx.execute("DELETE FROM study_crf_fields", &[]).await?;
        tx.execute("DELETE FROM study_crf_templates", &[]).await?;
        tx.execute("DELETE FROM study_visit_templates", &[]).await?;
        tx.execute("DELETE FROM study_phase_events", &[]).await?;
        tx.execute("DELETE FROM study_close_checklist_items", &[])
            .await?;
        tx.execute("DELETE FROM study_startup_checklist_items", &[])
            .await?;
        tx.execute("DELETE FROM sites", &[]).await?;
        tx.execute("DELETE FROM projects", &[]).await?;
        tx.execute("DELETE FROM user_memberships WHERE organization_id IS NOT NULL OR project_id IS NOT NULL", &[]).await?;
        tx.execute("DELETE FROM organizations", &[]).await?;

        // ── 2. Organizations (11) ─────────────────────────────────────────────
        tx.execute(r#"
            INSERT INTO organizations (id, name, organization_kind, workspace_slug, hex_code) VALUES
            ('00000000-0000-0000-0000-000000000001', 'Cingulum Foundation Inc.',  'platform_root',    'cingulum-foundation',   'A1B'),
            ('00000000-0000-0000-0000-000000000002', 'Alpha Health Research',     'research_network', 'alpha-health',          'A01'),
            ('00000000-0000-0000-0000-000000000003', 'Beta Medical Center',       'hospital',         'beta-medical',          'A02'),
            ('00000000-0000-0000-0000-000000000004', 'Gamma Clinical Inc.',       'tenant',           'gamma-clinical',        'A03'),
            ('00000000-0000-0000-0000-000000000005', 'Delta Pharma',              'sponsor',          'delta-pharma',          'A04'),
            ('00000000-0000-0000-0000-000000000006', 'Epsilon Therapeutics',      'sponsor',          'epsilon-therapeutics',  'A05'),
            ('00000000-0000-0000-0000-000000000007', 'Zeta Hospital Network',     'hospital',         'zeta-hospital',         'A06'),
            ('00000000-0000-0000-0000-000000000008', 'Eta Alliance',              'research_network', 'eta-alliance',          'A07'),
            ('00000000-0000-0000-0000-000000000009', 'Theta Oncology',            'tenant',           'theta-oncology',        'A08'),
            ('00000000-0000-0000-0000-000000000010', 'Iota Care Group',           'hospital',         'iota-care',             'A09'),
            ('00000000-0000-0000-0000-000000000011', 'Kappa Solutions',           'tenant',           'kappa-solutions',       'A0A')
        "#, &[]).await?;

        tx.execute(
            "UPDATE organizations SET parent_organization_id = '00000000-0000-0000-0000-000000000001' WHERE id <> '00000000-0000-0000-0000-000000000001'",
            &[],
        ).await?;

        // ── 3. Studies / Projects (10) ────────────────────────────────────────
        tx.execute(r#"
            INSERT INTO projects (id, organization_id, name, therapeutic_area, protocol_code, lifecycle_phase, planned_enrollment, status, hex_code, study_summary) VALUES
            ('00000000-0000-0000-0000-000000000101','00000000-0000-0000-0000-000000000002','Alpha Stroke Trial',       'Neurology',          'PROT-A01','active',100,'active','A01001','Randomised trial assessing clot-retrieval efficacy in acute ischaemic stroke.'),
            ('00000000-0000-0000-0000-000000000102','00000000-0000-0000-0000-000000000003','Beta Diabetes Study',      'Endocrinology',      'PROT-B02','active',150,'active','A02002','CGM-guided insulin dosing in newly diagnosed Type-2 diabetic adults.'),
            ('00000000-0000-0000-0000-000000000103','00000000-0000-0000-0000-000000000004','Gamma Cardiac Registry',   'Cardiology',         'PROT-C03','active',200,'active','A03003','Observational registry of STEMI outcomes across five regional centres.'),
            ('00000000-0000-0000-0000-000000000104','00000000-0000-0000-0000-000000000005','Delta Covid Monitor',      'Infectious Diseases','PROT-D04','active',300,'active','A04004','Longitudinal post-COVID pulmonary function monitoring cohort.'),
            ('00000000-0000-0000-0000-000000000105','00000000-0000-0000-0000-000000000006','Epsilon Lupus Trial',      'Rheumatology',       'PROT-E05','active', 80,'active','A05005','Phase III trial of belimumab dose-reduction in SLE remission maintenance.'),
            ('00000000-0000-0000-0000-000000000106','00000000-0000-0000-0000-000000000007','Zeta Asthma Initiative',   'Pulmonology',        'PROT-F06','active',120,'active','A06006','Biologic add-on therapy vs. standard ICS in moderate-severe asthma.'),
            ('00000000-0000-0000-0000-000000000107','00000000-0000-0000-0000-000000000008','Eta Parkinson Survey',     'Neurology',          'PROT-G07','active', 50,'active','A07007','Digital-biomarker gait analysis in early-stage Parkinson''s disease.'),
            ('00000000-0000-0000-0000-000000000108','00000000-0000-0000-0000-000000000009','Theta Lymphoma Phase II',  'Oncology',           'PROT-H08','active', 90,'active','A08008','CAR-T cell therapy safety run-in for relapsed/refractory DLBCL.'),
            ('00000000-0000-0000-0000-000000000109','00000000-0000-0000-0000-000000000010','Iota Alzheimer Project',   'Neurology',          'PROT-I09','active',250,'active','A09009','MRI volumetric tracking with plasma amyloid biomarker in prodromal AD.'),
            ('00000000-0000-0000-0000-000000000110','00000000-0000-0000-0000-000000000011','Kappa Rheumatoid Study',   'Immunology',         'PROT-J10','active',110,'active','A0A00A','JAK-inhibitor vs. MTX head-to-head in DMARD-naïve RA patients.')
        "#, &[]).await?;

        // ── 4. Sites (12 — 10 studies get 1 each, 2 studies get an extra) ─────
        tx.execute(r#"
            INSERT INTO sites (id, project_id, name, principal_investigator, co_principal_investigator, status, hex_code) VALUES
            ('00000000-0000-0000-0000-000000000201','00000000-0000-0000-0000-000000000101','Alpha Neurology Center',       'Dr. Alice Vance',      'Dr. Tom Finch',    'active','A01001001'),
            ('00000000-0000-0000-0000-000000000202','00000000-0000-0000-0000-000000000102','Beta Diabetes Clinic',         'Dr. Bob Miller',       NULL,               'active','A02002002'),
            ('00000000-0000-0000-0000-000000000203','00000000-0000-0000-0000-000000000103','Gamma Heart Institute',        'Dr. Charlie Song',     'Dr. Maya Patel',   'active','A03003003'),
            ('00000000-0000-0000-0000-000000000204','00000000-0000-0000-0000-000000000104','Delta Virology Lab',           'Dr. Diana Prince',     NULL,               'active','A04004004'),
            ('00000000-0000-0000-0000-000000000205','00000000-0000-0000-0000-000000000105','Epsilon Rheumatology Lab',     'Dr. Evan Wright',      'Dr. Sara Gold',    'active','A05005005'),
            ('00000000-0000-0000-0000-000000000206','00000000-0000-0000-0000-000000000106','Zeta Pulmonology Clinic',      'Dr. Fiona Gallagher',  NULL,               'active','A06006006'),
            ('00000000-0000-0000-0000-000000000207','00000000-0000-0000-0000-000000000107','Eta Parkinson Center',         'Dr. George Harrison',  NULL,               'active','A07007007'),
            ('00000000-0000-0000-0000-000000000208','00000000-0000-0000-0000-000000000108','Theta Oncology Center',        'Dr. Helen Cho',        'Dr. Jim Park',     'active','A08008008'),
            ('00000000-0000-0000-0000-000000000209','00000000-0000-0000-0000-000000000109','Iota Memory Clinic',           'Dr. Ian McKellen',     'Dr. Rosa Diaz',    'active','A09009009'),
            ('00000000-0000-0000-0000-000000000210','00000000-0000-0000-0000-000000000110','Kappa Immunology Hub',         'Dr. Julia Roberts',    NULL,               'active','A0A00A00A'),
            ('00000000-0000-0000-0000-000000000211','00000000-0000-0000-0000-000000000101','Alpha Neurology East Wing',    'Dr. Sam Torres',       NULL,               'active','A01001002'),
            ('00000000-0000-0000-0000-000000000212','00000000-0000-0000-0000-000000000103','Gamma Cardio Satellite',       'Dr. Wei Huang',        NULL,               'active','A03003004')
        "#, &[]).await?;

        // ── 5. Patients (30 — 3 per study, 2 sites for studies 101 and 103) ───
        tx.execute(r#"
            INSERT INTO patients (id, organization_id, project_id, site_id, external_subject_id, email, date_of_birth, hex_code) VALUES
            -- Alpha Stroke Trial (site 201, 211)
            ('00000000-0000-0000-0000-000000000301','00000000-0000-0000-0000-000000000002','00000000-0000-0000-0000-000000000101','00000000-0000-0000-0000-000000000201','SUBJ-A01','alice.neuro1@alpha.org',  '1975-04-12','A010010010001'),
            ('00000000-0000-0000-0000-000000000302','00000000-0000-0000-0000-000000000002','00000000-0000-0000-0000-000000000101','00000000-0000-0000-0000-000000000201','SUBJ-A02','alice.neuro2@alpha.org',  '1962-08-19','A010010010002'),
            ('00000000-0000-0000-0000-000000000303','00000000-0000-0000-0000-000000000002','00000000-0000-0000-0000-000000000101','00000000-0000-0000-0000-000000000211','SUBJ-A03','alice.neuro3@alpha.org',  '1948-03-05','A010010020001'),
            -- Beta Diabetes Study
            ('00000000-0000-0000-0000-000000000304','00000000-0000-0000-0000-000000000003','00000000-0000-0000-0000-000000000102','00000000-0000-0000-0000-000000000202','SUBJ-B01','bob.diab1@beta.org',      '1982-08-22','A020020020001'),
            ('00000000-0000-0000-0000-000000000305','00000000-0000-0000-0000-000000000003','00000000-0000-0000-0000-000000000102','00000000-0000-0000-0000-000000000202','SUBJ-B02','bob.diab2@beta.org',      '1970-01-11','A020020020002'),
            ('00000000-0000-0000-0000-000000000306','00000000-0000-0000-0000-000000000003','00000000-0000-0000-0000-000000000102','00000000-0000-0000-0000-000000000202','SUBJ-B03','bob.diab3@beta.org',      '1955-11-30','A020020020003'),
            -- Gamma Cardiac Registry (sites 203, 212)
            ('00000000-0000-0000-0000-000000000307','00000000-0000-0000-0000-000000000004','00000000-0000-0000-0000-000000000103','00000000-0000-0000-0000-000000000203','SUBJ-C01','charlie.card1@gamma.org', '1969-11-05','A030030030001'),
            ('00000000-0000-0000-0000-000000000308','00000000-0000-0000-0000-000000000004','00000000-0000-0000-0000-000000000103','00000000-0000-0000-0000-000000000203','SUBJ-C02','charlie.card2@gamma.org', '1978-06-20','A030030030002'),
            ('00000000-0000-0000-0000-000000000309','00000000-0000-0000-0000-000000000004','00000000-0000-0000-0000-000000000103','00000000-0000-0000-0000-000000000212','SUBJ-C03','charlie.card3@gamma.org', '1985-02-14','A030030040001'),
            -- Delta Covid Monitor
            ('00000000-0000-0000-0000-000000000310','00000000-0000-0000-0000-000000000005','00000000-0000-0000-0000-000000000104','00000000-0000-0000-0000-000000000204','SUBJ-D01','diana.cov1@delta.org',   '1990-01-30','A040040040001'),
            ('00000000-0000-0000-0000-000000000311','00000000-0000-0000-0000-000000000005','00000000-0000-0000-0000-000000000104','00000000-0000-0000-0000-000000000204','SUBJ-D02','diana.cov2@delta.org',   '1984-09-17','A040040040002'),
            ('00000000-0000-0000-0000-000000000312','00000000-0000-0000-0000-000000000005','00000000-0000-0000-0000-000000000104','00000000-0000-0000-0000-000000000204','SUBJ-D03','diana.cov3@delta.org',   '1976-07-04','A040040040003'),
            -- Epsilon Lupus Trial
            ('00000000-0000-0000-0000-000000000313','00000000-0000-0000-0000-000000000006','00000000-0000-0000-0000-000000000105','00000000-0000-0000-0000-000000000205','SUBJ-E01','evan.lup1@epsilon.org',  '1978-06-15','A050050050001'),
            ('00000000-0000-0000-0000-000000000314','00000000-0000-0000-0000-000000000006','00000000-0000-0000-0000-000000000105','00000000-0000-0000-0000-000000000205','SUBJ-E02','evan.lup2@epsilon.org',  '1991-04-23','A050050050002'),
            ('00000000-0000-0000-0000-000000000315','00000000-0000-0000-0000-000000000006','00000000-0000-0000-0000-000000000105','00000000-0000-0000-0000-000000000205','SUBJ-E03','evan.lup3@epsilon.org',  '1965-12-08','A050050050003'),
            -- Zeta Asthma Initiative
            ('00000000-0000-0000-0000-000000000316','00000000-0000-0000-0000-000000000007','00000000-0000-0000-0000-000000000106','00000000-0000-0000-0000-000000000206','SUBJ-F01','fiona.ast1@zeta.org',    '1985-10-24','A060060060001'),
            ('00000000-0000-0000-0000-000000000317','00000000-0000-0000-0000-000000000007','00000000-0000-0000-0000-000000000106','00000000-0000-0000-0000-000000000206','SUBJ-F02','fiona.ast2@zeta.org',    '1999-03-11','A060060060002'),
            ('00000000-0000-0000-0000-000000000318','00000000-0000-0000-0000-000000000007','00000000-0000-0000-0000-000000000106','00000000-0000-0000-0000-000000000206','SUBJ-F03','fiona.ast3@zeta.org',    '1973-08-18','A060060060003'),
            -- Eta Parkinson Survey
            ('00000000-0000-0000-0000-000000000319','00000000-0000-0000-0000-000000000008','00000000-0000-0000-0000-000000000107','00000000-0000-0000-0000-000000000207','SUBJ-G01','george.park1@eta.org',   '1960-02-14','A070070070001'),
            ('00000000-0000-0000-0000-000000000320','00000000-0000-0000-0000-000000000008','00000000-0000-0000-0000-000000000107','00000000-0000-0000-0000-000000000207','SUBJ-G02','george.park2@eta.org',   '1953-05-29','A070070070002'),
            ('00000000-0000-0000-0000-000000000321','00000000-0000-0000-0000-000000000008','00000000-0000-0000-0000-000000000107','00000000-0000-0000-0000-000000000207','SUBJ-G03','george.park3@eta.org',   '1967-09-02','A070070070003'),
            -- Theta Lymphoma Phase II
            ('00000000-0000-0000-0000-000000000322','00000000-0000-0000-0000-000000000009','00000000-0000-0000-0000-000000000108','00000000-0000-0000-0000-000000000208','SUBJ-H01','helen.lymp1@theta.org',  '1955-09-08','A080080080001'),
            ('00000000-0000-0000-0000-000000000323','00000000-0000-0000-0000-000000000009','00000000-0000-0000-0000-000000000108','00000000-0000-0000-0000-000000000208','SUBJ-H02','helen.lymp2@theta.org',  '1968-01-25','A080080080002'),
            ('00000000-0000-0000-0000-000000000324','00000000-0000-0000-0000-000000000009','00000000-0000-0000-0000-000000000108','00000000-0000-0000-0000-000000000208','SUBJ-H03','helen.lymp3@theta.org',  '1944-11-14','A080080080003'),
            -- Iota Alzheimer Project
            ('00000000-0000-0000-0000-000000000325','00000000-0000-0000-0000-000000000010','00000000-0000-0000-0000-000000000109','00000000-0000-0000-0000-000000000209','SUBJ-I01','ian.alz1@iota.org',      '1995-12-12','A090090090001'),
            ('00000000-0000-0000-0000-000000000326','00000000-0000-0000-0000-000000000010','00000000-0000-0000-0000-000000000109','00000000-0000-0000-0000-000000000209','SUBJ-I02','ian.alz2@iota.org',      '1949-03-27','A090090090002'),
            ('00000000-0000-0000-0000-000000000327','00000000-0000-0000-0000-000000000010','00000000-0000-0000-0000-000000000109','00000000-0000-0000-0000-000000000209','SUBJ-I03','ian.alz3@iota.org',      '1938-07-06','A090090090003'),
            -- Kappa Rheumatoid Study
            ('00000000-0000-0000-0000-000000000328','00000000-0000-0000-0000-000000000011','00000000-0000-0000-0000-000000000110','00000000-0000-0000-0000-000000000210','SUBJ-J01','julia.rhe1@kappa.org',   '1972-07-07','A0A00A00A0001'),
            ('00000000-0000-0000-0000-000000000329','00000000-0000-0000-0000-000000000011','00000000-0000-0000-0000-000000000110','00000000-0000-0000-0000-000000000210','SUBJ-J02','julia.rhe2@kappa.org',   '1980-10-19','A0A00A00A0002'),
            ('00000000-0000-0000-0000-000000000330','00000000-0000-0000-0000-000000000011','00000000-0000-0000-0000-000000000110','00000000-0000-0000-0000-000000000210','SUBJ-J03','julia.rhe3@kappa.org',   '1963-02-28','A0A00A00A0003')
        "#, &[]).await?;

        // ── 6. Providers (12) ─────────────────────────────────────────────────
        tx.execute(r#"
            INSERT INTO providers (id, organization_id, name, title, referral_source, hex_code) VALUES
            ('00000000-0000-0000-0000-000000000401','00000000-0000-0000-0000-000000000002','Dr. Alice Vance',       'Principal Investigator',  'Internal',  'A01401'),
            ('00000000-0000-0000-0000-000000000402','00000000-0000-0000-0000-000000000002','Dr. Tom Finch',         'Co-Investigator',          'Referral',  'A01402'),
            ('00000000-0000-0000-0000-000000000403','00000000-0000-0000-0000-000000000003','Dr. Bob Miller',        'Principal Investigator',  'Internal',  'A02401'),
            ('00000000-0000-0000-0000-000000000404','00000000-0000-0000-0000-000000000004','Dr. Charlie Song',      'Principal Investigator',  'Internal',  'A03401'),
            ('00000000-0000-0000-0000-000000000405','00000000-0000-0000-0000-000000000004','Dr. Maya Patel',        'Research Nurse',           'Hospital',  'A03402'),
            ('00000000-0000-0000-0000-000000000406','00000000-0000-0000-0000-000000000005','Dr. Diana Prince',      'Principal Investigator',  'Internal',  'A04401'),
            ('00000000-0000-0000-0000-000000000407','00000000-0000-0000-0000-000000000006','Dr. Evan Wright',       'Principal Investigator',  'Internal',  'A05401'),
            ('00000000-0000-0000-0000-000000000408','00000000-0000-0000-0000-000000000007','Dr. Fiona Gallagher',   'Principal Investigator',  'Internal',  'A06401'),
            ('00000000-0000-0000-0000-000000000409','00000000-0000-0000-0000-000000000008','Dr. George Harrison',   'Principal Investigator',  'Internal',  'A07401'),
            ('00000000-0000-0000-0000-000000000410','00000000-0000-0000-0000-000000000009','Dr. Helen Cho',         'Principal Investigator',  'Internal',  'A08401'),
            ('00000000-0000-0000-0000-000000000411','00000000-0000-0000-0000-000000000010','Dr. Ian McKellen',      'Principal Investigator',  'Internal',  'A09401'),
            ('00000000-0000-0000-0000-000000000412','00000000-0000-0000-0000-000000000011','Dr. Julia Roberts',     'Principal Investigator',  'Internal',  'A0A401')
        "#, &[]).await?;

        // ── 7. Encounters (30 — 3 per first 10 patients) ──────────────────────
        tx.execute(r#"
            INSERT INTO encounters (id, patient_id, provider_id, encounter_type, notes, hex_code) VALUES
            -- Patient 301 (SUBJ-A01) — Stroke  (hex: 16 chars A-F0-9)
            ('00000000-0000-0000-0000-000000000501','00000000-0000-0000-0000-000000000301','00000000-0000-0000-0000-000000000401','outpatient',  'Baseline neurological assessment. NIHSS score 8. CT clear.', 'A01001001001A001'),
            ('00000000-0000-0000-0000-000000000502','00000000-0000-0000-0000-000000000301','00000000-0000-0000-0000-000000000401','imaging',     'MRI brain — confirmed left MCA territory infarct 12 mm.', 'A01001001001B001'),
            ('00000000-0000-0000-0000-000000000503','00000000-0000-0000-0000-000000000301','00000000-0000-0000-0000-000000000401','imaging',     'Follow-up MRI Day 7 — stable infarct, no haemorrhagic change.', 'A01001001001B002'),
            -- Patient 304 (SUBJ-B01) — Diabetes
            ('00000000-0000-0000-0000-000000000504','00000000-0000-0000-0000-000000000304','00000000-0000-0000-0000-000000000403','outpatient',  'Baseline HbA1c 8.2%. CGM device fitted. Dietary counselling given.', 'A02002001001A001'),
            ('00000000-0000-0000-0000-000000000505','00000000-0000-0000-0000-000000000304','00000000-0000-0000-0000-000000000403','labs',        'Fasting glucose panel, lipids, renal function — within protocol range.', 'A02002001001C001'),
            ('00000000-0000-0000-0000-000000000506','00000000-0000-0000-0000-000000000304','00000000-0000-0000-0000-000000000403','outpatient',  '3-month review: HbA1c 7.4%. CGM time-in-range improved to 68%.', 'A02002001001A002'),
            -- Patient 307 (SUBJ-C01) — Cardiac
            ('00000000-0000-0000-0000-000000000507','00000000-0000-0000-0000-000000000307','00000000-0000-0000-0000-000000000404','outpatient',  'Post-STEMI Day 30 review. EF 45%. Started ARNI therapy.', 'A03003001001A001'),
            ('00000000-0000-0000-0000-000000000508','00000000-0000-0000-0000-000000000307','00000000-0000-0000-0000-000000000404','imaging',     'Echocardiogram: EF 48%, moderate MR, no pericardial effusion.', 'A03003001001B001'),
            ('00000000-0000-0000-0000-000000000509','00000000-0000-0000-0000-000000000307','00000000-0000-0000-0000-000000000404','procedures',  'Right-heart catheterisation — PCWP 18 mmHg, CO 4.1 L/min.', 'A03003001001D001'),
            -- Patient 310 (SUBJ-D01) — Covid
            ('00000000-0000-0000-0000-000000000510','00000000-0000-0000-0000-000000000310','00000000-0000-0000-0000-000000000406','outpatient',  'Persistent dyspnoea 6 months post-COVID. SpO2 94% on exertion.', 'A04004001001A001'),
            ('00000000-0000-0000-0000-000000000511','00000000-0000-0000-0000-000000000310','00000000-0000-0000-0000-000000000406','imaging',     'CT thorax: bilateral ground-glass opacities (5%), mild bronchiectasis.', 'A04004001001B001'),
            ('00000000-0000-0000-0000-000000000512','00000000-0000-0000-0000-000000000310','00000000-0000-0000-0000-000000000406','labs',        'PFTs: FVC 78% predicted, DLCO 65% predicted — mild restriction.', 'A04004001001C001'),
            -- Patient 313 (SUBJ-E01) — Lupus
            ('00000000-0000-0000-0000-000000000513','00000000-0000-0000-0000-000000000313','00000000-0000-0000-0000-000000000407','outpatient',  'Belimumab dose step-down to 200 mg/month. SLEDAI-2K score 4.', 'A05005001001A001'),
            ('00000000-0000-0000-0000-000000000514','00000000-0000-0000-0000-000000000313','00000000-0000-0000-0000-000000000407','labs',        'Complement C3/C4 normal. Anti-dsDNA weakly positive. eGFR 88.', 'A05005001001C001'),
            ('00000000-0000-0000-0000-000000000515','00000000-0000-0000-0000-000000000313','00000000-0000-0000-0000-000000000407','outpatient',  '6-month flare assessment: Arthritis flare. Dose reinstated.', 'A05005001001A002'),
            -- Patient 316 (SUBJ-F01) — Asthma
            ('00000000-0000-0000-0000-000000000516','00000000-0000-0000-0000-000000000316','00000000-0000-0000-0000-000000000408','outpatient',  'Baseline spirometry: FEV1/FVC 0.64. Dupilumab 300 mg SC initiated.', 'A06006001001A001'),
            ('00000000-0000-0000-0000-000000000517','00000000-0000-0000-0000-000000000316','00000000-0000-0000-0000-000000000408','labs',        'FeNO 42 ppb, blood eosinophils 0.45×10^9/L — T2-high phenotype.', 'A06006001001C001'),
            ('00000000-0000-0000-0000-000000000518','00000000-0000-0000-0000-000000000316','00000000-0000-0000-0000-000000000408','outpatient',  '12-week review: FEV1 improved +18%, zero OCS courses.', 'A06006001001A002'),
            -- Patient 319 (SUBJ-G01) — Parkinson
            ('00000000-0000-0000-0000-000000000519','00000000-0000-0000-0000-000000000319','00000000-0000-0000-0000-000000000409','outpatient',  'Gait sensor belt fitted. Baseline MDS-UPDRS part III = 24.', 'A07007001001A001'),
            ('00000000-0000-0000-0000-000000000520','00000000-0000-0000-0000-000000000319','00000000-0000-0000-0000-000000000409','imaging',     'DaT-SCAN: bilateral nigrostriatal deficit, L>R asymmetry confirmed.', 'A07007001001B001'),
            ('00000000-0000-0000-0000-000000000521','00000000-0000-0000-0000-000000000319','00000000-0000-0000-0000-000000000409','procedures',  'Gait lab digital biomarker extraction — 4 min walk, dual-task.', 'A07007001001D001'),
            -- Patient 322 (SUBJ-H01) — Lymphoma
            ('00000000-0000-0000-0000-000000000522','00000000-0000-0000-0000-000000000322','00000000-0000-0000-0000-000000000410','inpatient',   'CAR-T infusion Day 0. Pre-infusion lymphodepleting chemo completed.', 'A08008001001E001'),
            ('00000000-0000-0000-0000-000000000523','00000000-0000-0000-0000-000000000322','00000000-0000-0000-0000-000000000410','labs',        'Day 7 cytokine panel: IL-6 42 pg/mL, ferritin 1200 ng/mL — CRS Grade 1.', 'A08008001001C001'),
            ('00000000-0000-0000-0000-000000000524','00000000-0000-0000-0000-000000000322','00000000-0000-0000-0000-000000000410','imaging',     'PET-CT Day 30: CMR in 4/5 target lesions — Deauville 2.', 'A08008001001B001'),
            -- Patient 325 (SUBJ-I01) — Alzheimer
            ('00000000-0000-0000-0000-000000000525','00000000-0000-0000-0000-000000000325','00000000-0000-0000-0000-000000000411','outpatient',  'Baseline CDR 0.5. Plasma Ab42/40 ratio 0.061 — amyloid positive.', 'A09009001001A001'),
            ('00000000-0000-0000-0000-000000000526','00000000-0000-0000-0000-000000000325','00000000-0000-0000-0000-000000000411','imaging',     'MRI volumetrics: hippocampal volume 3.1 cm3 (78th percentile).', 'A09009001001B001'),
            ('00000000-0000-0000-0000-000000000527','00000000-0000-0000-0000-000000000325','00000000-0000-0000-0000-000000000411','labs',        'CSF Ab42 620 pg/mL, p-tau181 32 pg/mL — AD biomarker profile.', 'A09009001001C001'),
            -- Patient 328 (SUBJ-J01) — Rheumatoid
            ('00000000-0000-0000-0000-000000000528','00000000-0000-0000-0000-000000000328','00000000-0000-0000-0000-000000000412','outpatient',  'DMARD-naive. DAS28-CRP 5.4. Upadacitinib 15 mg OD initiated.', 'A0A00A001001A001'),
            ('00000000-0000-0000-0000-000000000529','00000000-0000-0000-0000-000000000328','00000000-0000-0000-0000-000000000412','labs',        'Anti-CCP 180 IU/mL, RF 85 IU/mL, CRP 22 mg/L — high disease activity.', 'A0A00A001001C001'),
            ('00000000-0000-0000-0000-000000000530','00000000-0000-0000-0000-000000000328','00000000-0000-0000-0000-000000000412','outpatient',  '12-week EULAR: DAS28-CRP 2.8 — low disease activity achieved.', 'A0A00A001001A002')
        "#, &[]).await?;

        // ── 8. Media Upload Tickets (20 — across all entity types) ───────────
        // We insert with entity_type + entity_id; upload URLs are demo stubs
        let _ = tx.execute(r#"
            INSERT INTO media_upload_tickets (id, organization_id, project_id, patient_id, mime_type, upload_url, expires_at, entity_type, entity_id, file_name, description) VALUES
            -- Organization-level documents
            ('00000000-0000-0000-0000-000000000601','00000000-0000-0000-0000-000000000002','00000000-0000-0000-0000-000000000101','system','application/pdf',
             'https://upload.virivu.example/v1/media/601?content_type=application%2Fpdf',NOW()+INTERVAL '10 minutes',
             'organization','00000000-0000-0000-0000-000000000002','IRB_Approval_AlphaHealth_2026.pdf','Institutional Review Board approval letter'),
            ('00000000-0000-0000-0000-000000000602','00000000-0000-0000-0000-000000000003','00000000-0000-0000-0000-000000000102','system','application/pdf',
             'https://upload.virivu.example/v1/media/602?content_type=application%2Fpdf',NOW()+INTERVAL '10 minutes',
             'organization','00000000-0000-0000-0000-000000000003','SponsorAgreement_BetaMedical.pdf','Sponsor agreement and indemnity'),
            -- Study-level documents
            ('00000000-0000-0000-0000-000000000603','00000000-0000-0000-0000-000000000002','00000000-0000-0000-0000-000000000101','system','application/pdf',
             'https://upload.virivu.example/v1/media/603?content_type=application%2Fpdf',NOW()+INTERVAL '10 minutes',
             'study','00000000-0000-0000-0000-000000000101','Protocol_AlphaStroke_v2.1.pdf','Study protocol version 2.1'),
            ('00000000-0000-0000-0000-000000000604','00000000-0000-0000-0000-000000000003','00000000-0000-0000-0000-000000000102','system','application/pdf',
             'https://upload.virivu.example/v1/media/604?content_type=application%2Fpdf',NOW()+INTERVAL '10 minutes',
             'study','00000000-0000-0000-0000-000000000102','ImagingManual_BetaDiabetes.pdf','Imaging acquisition and reading manual'),
            -- Site-level documents
            ('00000000-0000-0000-0000-000000000605','00000000-0000-0000-0000-000000000002','00000000-0000-0000-0000-000000000101','system','application/pdf',
             'https://upload.virivu.example/v1/media/605?content_type=application%2Fpdf',NOW()+INTERVAL '10 minutes',
             'site','00000000-0000-0000-0000-000000000201','SiteActivation_AlphaNeuro.pdf','Site qualification and activation checklist'),
            ('00000000-0000-0000-0000-000000000606','00000000-0000-0000-0000-000000000004','00000000-0000-0000-0000-000000000103','system','application/pdf',
             'https://upload.virivu.example/v1/media/606?content_type=application%2Fpdf',NOW()+INTERVAL '10 minutes',
             'site','00000000-0000-0000-0000-000000000203','CVandLicence_DrCharlieSong.pdf','PI curriculum vitae and medical licence'),
            -- Patient-level: video
            ('00000000-0000-0000-0000-000000000607','00000000-0000-0000-0000-000000000002','00000000-0000-0000-0000-000000000101','SUBJ-A01','video/mp4',
             'https://upload.virivu.example/v1/media/607?content_type=video%2Fmp4',NOW()+INTERVAL '10 minutes',
             'patient','00000000-0000-0000-0000-000000000301','SUBJ-A01_gait_baseline.mp4','Baseline gait video assessment'),
            ('00000000-0000-0000-0000-000000000608','00000000-0000-0000-0000-000000000008','00000000-0000-0000-0000-000000000107','SUBJ-G01','video/mp4',
             'https://upload.virivu.example/v1/media/608?content_type=video%2Fmp4',NOW()+INTERVAL '10 minutes',
             'patient','00000000-0000-0000-0000-000000000319','SUBJ-G01_tremor_video_month3.mp4','Month 3 tremor assessment video'),
            -- Patient-level: audio
            ('00000000-0000-0000-0000-000000000609','00000000-0000-0000-0000-000000000008','00000000-0000-0000-0000-000000000107','SUBJ-G02','audio/wav',
             'https://upload.virivu.example/v1/media/609?content_type=audio%2Fwav',NOW()+INTERVAL '10 minutes',
             'patient','00000000-0000-0000-0000-000000000320','SUBJ-G02_speech_recording_baseline.wav','Baseline speech fluency recording'),
            ('00000000-0000-0000-0000-000000000610','00000000-0000-0000-0000-000000000007','00000000-0000-0000-0000-000000000106','SUBJ-F01','audio/wav',
             'https://upload.virivu.example/v1/media/610?content_type=audio%2Fwav',NOW()+INTERVAL '10 minutes',
             'patient','00000000-0000-0000-0000-000000000316','SUBJ-F01_breath_sounds_before.wav','Pre-treatment auscultation recording'),
            -- Patient-level: DICOM
            ('00000000-0000-0000-0000-000000000611','00000000-0000-0000-0000-000000000002','00000000-0000-0000-0000-000000000101','SUBJ-A01','application/dicom',
             'https://upload.virivu.example/v1/media/611?content_type=application%2Fdicom',NOW()+INTERVAL '10 minutes',
             'patient','00000000-0000-0000-0000-000000000301','SUBJ-A01_MRI_brain_day0.dcm','Baseline brain MRI DICOM series'),
            ('00000000-0000-0000-0000-000000000612','00000000-0000-0000-0000-000000000010','00000000-0000-0000-0000-000000000109','SUBJ-I01','application/dicom',
             'https://upload.virivu.example/v1/media/612?content_type=application%2Fdicom',NOW()+INTERVAL '10 minutes',
             'patient','00000000-0000-0000-0000-000000000325','SUBJ-I01_MRI_hippocampus_month0.dcm','Hippocampal volumetric MRI baseline'),
            -- Patient-level: images
            ('00000000-0000-0000-0000-000000000613','00000000-0000-0000-0000-000000000006','00000000-0000-0000-0000-000000000105','SUBJ-E01','image/jpeg',
             'https://upload.virivu.example/v1/media/613?content_type=image%2Fjpeg',NOW()+INTERVAL '10 minutes',
             'patient','00000000-0000-0000-0000-000000000313','SUBJ-E01_rash_photo_day0.jpg','Malar rash clinical photograph at baseline'),
            ('00000000-0000-0000-0000-000000000614','00000000-0000-0000-0000-000000000004','00000000-0000-0000-0000-000000000103','SUBJ-C01','image/jpeg',
             'https://upload.virivu.example/v1/media/614?content_type=image%2Fjpeg',NOW()+INTERVAL '10 minutes',
             'patient','00000000-0000-0000-0000-000000000307','SUBJ-C01_echo_screenshot.jpg','Echocardiogram parasternal long axis screenshot'),
            -- Encounter-level media
            ('00000000-0000-0000-0000-000000000615','00000000-0000-0000-0000-000000000002','00000000-0000-0000-0000-000000000101','SUBJ-A01','video/mp4',
             'https://upload.virivu.example/v1/media/615?content_type=video%2Fmp4',NOW()+INTERVAL '10 minutes',
             'encounter','00000000-0000-0000-0000-000000000501','Enc501_consultation_recording.mp4','Consultation recording for audit'),
            ('00000000-0000-0000-0000-000000000616','00000000-0000-0000-0000-000000000002','00000000-0000-0000-0000-000000000101','SUBJ-A01','application/dicom',
             'https://upload.virivu.example/v1/media/616?content_type=application%2Fdicom',NOW()+INTERVAL '10 minutes',
             'encounter','00000000-0000-0000-0000-000000000502','Enc502_MRI_series.dcm','MRI series from encounter 502'),
            ('00000000-0000-0000-0000-000000000617','00000000-0000-0000-0000-000000000009','00000000-0000-0000-0000-000000000108','SUBJ-H01','video/mp4',
             'https://upload.virivu.example/v1/media/617?content_type=video%2Fmp4',NOW()+INTERVAL '10 minutes',
             'encounter','00000000-0000-0000-0000-000000000522','Enc522_infusion_monitoring.mp4','CAR-T infusion day monitoring video'),
            ('00000000-0000-0000-0000-000000000618','00000000-0000-0000-0000-000000000008','00000000-0000-0000-0000-000000000107','SUBJ-G01','audio/wav',
             'https://upload.virivu.example/v1/media/618?content_type=audio%2Fwav',NOW()+INTERVAL '10 minutes',
             'encounter','00000000-0000-0000-0000-000000000519','Enc519_motor_exam_audio.wav','Motor examination verbal notes'),
            ('00000000-0000-0000-0000-000000000619','00000000-0000-0000-0000-000000000011','00000000-0000-0000-0000-000000000110','SUBJ-J01','image/jpeg',
             'https://upload.virivu.example/v1/media/619?content_type=image%2Fjpeg',NOW()+INTERVAL '10 minutes',
             'encounter','00000000-0000-0000-0000-000000000528','Enc528_joint_xray.jpg','Bilateral hand X-ray at baseline visit'),
            ('00000000-0000-0000-0000-000000000620','00000000-0000-0000-0000-000000000003','00000000-0000-0000-0000-000000000102','SUBJ-B01','application/pdf',
             'https://upload.virivu.example/v1/media/620?content_type=application%2Fpdf',NOW()+INTERVAL '10 minutes',
             'encounter','00000000-0000-0000-0000-000000000504','Enc504_lab_report.pdf','Signed laboratory report for baseline encounter')
        "#, &[]).await;
        // Note: entity_type columns may fail if migration didn't apply cleanly — that's OK for the seed

        tx.commit().await?;
        Ok(())
    }

    pub async fn list_all_projects(&self) -> anyhow::Result<Vec<Project>> {
        let client = self.pool.get().await?;
        let rows = client
            .query(
                r#"
                SELECT
                    id, organization_id, name, therapeutic_area, protocol_code, lifecycle_phase, planned_enrollment, clinicaltrials_gov_id, study_summary, phase_changed_at, hex_code, status, last_activity_at, created_at
                FROM projects
                ORDER BY name ASC
                "#,
                &[],
            )
            .await?;
        Ok(rows.iter().map(row_to_project).collect())
    }

    pub async fn list_all_sites(&self) -> anyhow::Result<Vec<Site>> {
        let client = self.pool.get().await?;
        let rows = client
            .query(
                r#"
                SELECT id, project_id, name, principal_investigator, co_principal_investigator, sub_investigator, hex_code, status, last_activity_at, created_at
                FROM sites
                ORDER BY name ASC
                "#,
                &[],
            )
            .await?;
        Ok(rows.iter().map(row_to_site).collect())
    }

    pub async fn list_all_patients(&self) -> anyhow::Result<Vec<Patient>> {
        let client = self.pool.get().await?;
        let rows = client
            .query(
                r#"
                SELECT id, organization_id, project_id, site_id, external_subject_id, email, date_of_birth, hex_code, created_at
                FROM patients
                ORDER BY hex_code ASC
                "#,
                &[],
            )
            .await?;
        Ok(rows.iter().map(row_to_patient).collect())
    }

    pub async fn set_site_status(&self, site_id: Uuid, status: &str) -> anyhow::Result<()> {
        let client = self.pool.get().await?;
        client
            .execute(
                "UPDATE sites SET status = $1, last_activity_at = NOW() WHERE id = $2",
                &[&status, &site_id],
            )
            .await?;
        Ok(())
    }

    /// Attach an existing site (typically org-level) to a specific project/study.
    pub async fn attach_site_to_project(
        &self,
        site_id: Uuid,
        project_id: Uuid,
    ) -> anyhow::Result<()> {
        let client = self.pool.get().await?;
        client
            .execute(
                "UPDATE sites SET project_id = $1, last_activity_at = NOW() WHERE id = $2",
                &[&project_id, &site_id],
            )
            .await?;
        Ok(())
    }

    /// Detach a site from its current study (makes it org-level again).
    pub async fn detach_site_from_project(&self, site_id: Uuid) -> anyhow::Result<()> {
        let client = self.pool.get().await?;
        client
            .execute(
                "UPDATE sites SET project_id = NULL, last_activity_at = NOW() WHERE id = $1",
                &[&site_id],
            )
            .await?;
        Ok(())
    }

    pub async fn set_project_status(&self, project_id: Uuid, status: &str) -> anyhow::Result<()> {
        let client = self.pool.get().await?;
        client
            .execute(
                "UPDATE projects SET status = $1, last_activity_at = NOW() WHERE id = $2",
                &[&status, &project_id],
            )
            .await?;
        Ok(())
    }

    pub async fn touch_site_activity(&self, site_id: Uuid) -> anyhow::Result<()> {
        let client = self.pool.get().await?;
        client
            .execute(
                "UPDATE sites SET last_activity_at = NOW() WHERE id = $1",
                &[&site_id],
            )
            .await?;
        client
            .execute(
                "UPDATE projects SET last_activity_at = NOW() WHERE id = (SELECT project_id FROM sites WHERE id = $1)",
                &[&site_id],
            )
            .await?;
        Ok(())
    }

    pub async fn touch_project_activity(&self, project_id: Uuid) -> anyhow::Result<()> {
        let client = self.pool.get().await?;
        client
            .execute(
                "UPDATE projects SET last_activity_at = NOW() WHERE id = $1",
                &[&project_id],
            )
            .await?;
        Ok(())
    }

    pub async fn auto_archive_dormant_entities(
        &self,
        days_threshold: f64,
    ) -> anyhow::Result<(u64, u64)> {
        let client = self.pool.get().await?;

        let updated_sites = client
            .execute(
                "UPDATE sites 
                 SET status = 'dormant' 
                 WHERE status = 'active' AND last_activity_at < NOW() - ($1 * INTERVAL '1 day')",
                &[&days_threshold],
            )
            .await?;

        let updated_projects = client
            .execute(
                "UPDATE projects 
                 SET status = 'dormant' 
                 WHERE status = 'active' AND last_activity_at < NOW() - ($1 * INTERVAL '1 day')",
                &[&days_threshold],
            )
            .await?;

        Ok((updated_sites, updated_projects))
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
            entity_type: None,
            entity_id: None,
            file_name: None,
            description: None,
            upload_status: Some("pending".to_string()),
        })
    }

    /// Create a media ticket scoped to a specific entity (org/study/site/patient/encounter)
    pub async fn create_media_upload_ticket_for_entity(
        &self,
        organization_id: Uuid,
        project_id: Uuid,
        patient_id: &str,
        mime_type: &str,
        entity_type: &str,
        entity_id: Option<Uuid>,
        file_name: Option<&str>,
        description: Option<&str>,
    ) -> anyhow::Result<MediaUploadTicket> {
        let id = Uuid::new_v4();
        let expires_at = Utc::now() + Duration::minutes(60);
        let upload_url = format!(
            "https://upload.virivu.example/v1/media/{id}?content_type={}&entity={entity_type}",
            mime_type
        );

        let client = self.pool.get().await?;
        let row = client
            .query_one(
                r#"
                INSERT INTO media_upload_tickets
                    (id, organization_id, project_id, patient_id, mime_type, upload_url, expires_at,
                     entity_type, entity_id, file_name, description, upload_status)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, 'pending')
                RETURNING id, organization_id, project_id, patient_id, mime_type, upload_url, expires_at,
                          entity_type, entity_id, file_name, description, upload_status
                "#,
                &[
                    &id, &organization_id, &project_id, &patient_id,
                    &mime_type, &upload_url, &expires_at,
                    &entity_type, &entity_id, &file_name, &description,
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
            entity_type: row.get("entity_type"),
            entity_id: row.get("entity_id"),
            file_name: row.get("file_name"),
            description: row.get("description"),
            upload_status: row.get("upload_status"),
        })
    }

    /// Get all media tickets for a specific entity
    pub async fn list_media_tickets_for_entity(
        &self,
        entity_type: &str,
        entity_id: Uuid,
    ) -> anyhow::Result<Vec<MediaUploadTicket>> {
        let client = self.pool.get().await?;
        let rows = client
            .query(
                r#"
                SELECT id, organization_id, project_id, patient_id, mime_type, upload_url, expires_at,
                       entity_type, entity_id, file_name, description, upload_status
                FROM media_upload_tickets
                WHERE entity_type = $1 AND entity_id = $2
                ORDER BY expires_at DESC
                LIMIT 50
                "#,
                &[&entity_type, &entity_id],
            )
            .await?;
        Ok(rows
            .into_iter()
            .map(|row| MediaUploadTicket {
                id: row.get("id"),
                organization_id: row.get("organization_id"),
                project_id: row.get("project_id"),
                patient_id: row.get("patient_id"),
                mime_type: row.get("mime_type"),
                upload_url: row.get("upload_url"),
                expires_at: row.get("expires_at"),
                entity_type: row.get("entity_type"),
                entity_id: row.get("entity_id"),
                file_name: row.get("file_name"),
                description: row.get("description"),
                upload_status: row.get("upload_status"),
            })
            .collect())
    }

    /// Get a patient by their ID with associated org, project, site names
    pub async fn get_patient_with_context(
        &self,
        patient_id: Uuid,
    ) -> anyhow::Result<Option<(Patient, String, String, String)>> {
        let client = self.pool.get().await?;
        let row = client
            .query_opt(
                r#"
                SELECT
                    p.id, p.organization_id, p.project_id, p.site_id,
                    p.external_subject_id, p.email, p.date_of_birth, p.hex_code, p.created_at,
                    o.name as org_name,
                    proj.name as study_name,
                    COALESCE(s.name, 'Unknown Site') as site_name
                FROM patients p
                JOIN organizations o ON o.id = p.organization_id
                JOIN projects proj ON proj.id = p.project_id
                LEFT JOIN sites s ON s.id = p.site_id
                WHERE p.id = $1
                "#,
                &[&patient_id],
            )
            .await?;
        Ok(row.map(|r| {
            let patient = Patient {
                id: r.get("id"),
                organization_id: r.get("organization_id"),
                project_id: r.get("project_id"),
                site_id: r.get("site_id"),
                external_subject_id: r.get("external_subject_id"),
                email: r.get("email"),
                date_of_birth: r.get("date_of_birth"),
                hex_code: r.get("hex_code"),
                created_at: r.get("created_at"),
            };
            let org_name: String = r.get("org_name");
            let study_name: String = r.get("study_name");
            let site_name: String = r.get("site_name");
            (patient, org_name, study_name, site_name)
        }))
    }

    /// Get all encounters for a patient
    pub async fn list_encounters_for_patient(
        &self,
        patient_id: Uuid,
    ) -> anyhow::Result<Vec<Encounter>> {
        let client = self.pool.get().await?;
        let rows = client
            .query(
                r#"
                SELECT id, patient_id, provider_id, encounter_type, notes, hex_code, created_at
                FROM encounters
                WHERE patient_id = $1
                ORDER BY created_at DESC
                "#,
                &[&patient_id],
            )
            .await?;
        Ok(rows
            .into_iter()
            .map(|row| Encounter {
                id: row.get("id"),
                patient_id: row.get("patient_id"),
                provider_id: row.get("provider_id"),
                encounter_type: row.get("encounter_type"),
                notes: row.get("notes"),
                hex_code: row.get("hex_code"),
                created_at: row.get("created_at"),
            })
            .collect())
    }

    /// Create a patient portal magic-link session
    pub async fn create_patient_session(&self, patient_id: Uuid) -> anyhow::Result<PatientSession> {
        // Use two UUIDs concatenated (no dashes) as a 256-bit secure token
        let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        let expires_at = Utc::now() + Duration::days(7);

        let client = self.pool.get().await?;
        let row = client
            .query_one(
                r#"
                INSERT INTO patient_sessions (patient_id, token, expires_at)
                VALUES ($1, $2, $3)
                RETURNING id, patient_id, token, expires_at, used_at, created_at
                "#,
                &[&patient_id, &token, &expires_at],
            )
            .await?;
        Ok(PatientSession {
            id: row.get("id"),
            patient_id: row.get("patient_id"),
            token: row.get("token"),
            expires_at: row.get("expires_at"),
            used_at: row.get("used_at"),
            created_at: row.get("created_at"),
        })
    }

    /// Look up a patient by their portal token
    pub async fn get_patient_by_portal_token(
        &self,
        token: &str,
    ) -> anyhow::Result<Option<Patient>> {
        let client = self.pool.get().await?;
        let row = client
            .query_opt(
                r#"
                SELECT p.id, p.organization_id, p.project_id, p.site_id,
                       p.external_subject_id, p.email, p.date_of_birth, p.hex_code, p.created_at
                FROM patients p
                JOIN patient_sessions ps ON ps.patient_id = p.id
                WHERE ps.token = $1
                  AND ps.expires_at > NOW()
                  AND ps.used_at IS NULL
                "#,
                &[&token],
            )
            .await?;
        Ok(row.map(|r| Patient {
            id: r.get("id"),
            organization_id: r.get("organization_id"),
            project_id: r.get("project_id"),
            site_id: r.get("site_id"),
            external_subject_id: r.get("external_subject_id"),
            email: r.get("email"),
            date_of_birth: r.get("date_of_birth"),
            hex_code: r.get("hex_code"),
            created_at: r.get("created_at"),
        }))
    }

    /// Look up a patient by portal token (multi-use within expiry window - practical for patient portal)
    pub async fn get_patient_by_portal_token_multiuse(
        &self,
        token: &str,
    ) -> anyhow::Result<Option<Patient>> {
        let client = self.pool.get().await?;
        let row = client
            .query_opt(
                r#"
                SELECT p.id, p.organization_id, p.project_id, p.site_id,
                       p.external_subject_id, p.email, p.date_of_birth, p.hex_code, p.created_at
                FROM patients p
                JOIN patient_sessions ps ON ps.patient_id = p.id
                WHERE ps.token = $1
                  AND ps.expires_at > NOW()
                "#,
                &[&token],
            )
            .await?;
        Ok(row.map(|r| Patient {
            id: r.get("id"),
            organization_id: r.get("organization_id"),
            project_id: r.get("project_id"),
            site_id: r.get("site_id"),
            external_subject_id: r.get("external_subject_id"),
            email: r.get("email"),
            date_of_birth: r.get("date_of_birth"),
            hex_code: r.get("hex_code"),
            created_at: r.get("created_at"),
        }))
    }

    /// Submit a PRO form from patient portal
    pub async fn create_pro_submission(
        &self,
        patient_id: Uuid,
        form_type: &str,
        answers_json: &str,
        total_score: Option<i32>,
    ) -> anyhow::Result<ProSubmission> {
        let client = self.pool.get().await?;
        let row = client
            .query_one(
                r#"
                INSERT INTO pro_submissions (patient_id, form_type, answers, total_score, submitted_at)
                VALUES ($1, $2, $3::jsonb, $4, NOW())
                RETURNING id, patient_id, form_type, answers::text as answers_text, total_score, submitted_at, created_at
                "#,
                &[&patient_id, &form_type, &answers_json, &total_score],
            )
            .await?;
        let answers_str: String = row.get("answers_text");
        Ok(ProSubmission {
            id: row.get("id"),
            patient_id: row.get("patient_id"),
            form_type: row.get("form_type"),
            answers: serde_json::from_str(&answers_str).unwrap_or(serde_json::Value::Null),
            total_score: row.get("total_score"),
            submitted_at: row.get("submitted_at"),
            created_at: row.get("created_at"),
        })
    }

    /// List PRO submissions for a patient
    pub async fn list_pro_submissions(
        &self,
        patient_id: Uuid,
    ) -> anyhow::Result<Vec<ProSubmission>> {
        let client = self.pool.get().await?;
        let rows = client
            .query(
                r#"
                SELECT id, patient_id, form_type, answers::text as answers_text, total_score, submitted_at, created_at
                FROM pro_submissions
                WHERE patient_id = $1
                ORDER BY created_at DESC
                "#,
                &[&patient_id],
            )
            .await?;
        Ok(rows
            .into_iter()
            .map(|row| {
                let answers_str: String = row.get("answers_text");
                ProSubmission {
                    id: row.get("id"),
                    patient_id: row.get("patient_id"),
                    form_type: row.get("form_type"),
                    answers: serde_json::from_str(&answers_str).unwrap_or(serde_json::Value::Null),
                    total_score: row.get("total_score"),
                    submitted_at: row.get("submitted_at"),
                    created_at: row.get("created_at"),
                }
            })
            .collect())
    }

    /// List the most recent PRO submissions across all patients (for coordinator dashboard visibility of patient portal activity)
    pub async fn list_recent_pro_submissions(
        &self,
        limit: i64,
    ) -> anyhow::Result<Vec<ProSubmission>> {
        let client = self.pool.get().await?;
        let rows = client
            .query(
                r#"
                SELECT id, patient_id, form_type, answers::text as answers_text, total_score, submitted_at, created_at
                FROM pro_submissions
                ORDER BY created_at DESC
                LIMIT $1
                "#,
                &[&limit],
            )
            .await?;
        Ok(rows
            .into_iter()
            .map(|row| {
                let answers_str: String = row.get("answers_text");
                ProSubmission {
                    id: row.get("id"),
                    patient_id: row.get("patient_id"),
                    form_type: row.get("form_type"),
                    answers: serde_json::from_str(&answers_str).unwrap_or(serde_json::Value::Null),
                    total_score: row.get("total_score"),
                    submitted_at: row.get("submitted_at"),
                    created_at: row.get("created_at"),
                }
            })
            .collect())
    }

    /// Find a patient by email for portal login
    pub async fn get_patient_by_email(&self, email: &str) -> anyhow::Result<Option<Patient>> {
        let client = self.pool.get().await?;
        let row = client
            .query_opt(
                r#"
                SELECT id, organization_id, project_id, site_id,
                       external_subject_id, email, date_of_birth, hex_code, created_at
                FROM patients
                WHERE LOWER(email) = LOWER($1)
                LIMIT 1
                "#,
                &[&email],
            )
            .await?;
        Ok(row.map(|r| Patient {
            id: r.get("id"),
            organization_id: r.get("organization_id"),
            project_id: r.get("project_id"),
            site_id: r.get("site_id"),
            external_subject_id: r.get("external_subject_id"),
            email: r.get("email"),
            date_of_birth: r.get("date_of_birth"),
            hex_code: r.get("hex_code"),
            created_at: r.get("created_at"),
        }))
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
                    status,
                    last_activity_at,
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
            status: r.get("status"),
            last_activity_at: r.get("last_activity_at"),
            created_at: r.get("created_at"),
        }))
    }

    pub async fn get_site(&self, site_id: Uuid) -> anyhow::Result<Option<Site>> {
        let client = self.pool.get().await?;
        let row = client
            .query_opt(
                r#"
                SELECT id, organization_id, project_id, name, principal_investigator, co_principal_investigator, sub_investigator, hex_code, status, last_activity_at, created_at
                FROM sites
                WHERE id = $1
                "#,
                &[&site_id],
            )
            .await?;
        Ok(row.map(|r| Site {
            id: r.get("id"),
            organization_id: r.get("organization_id"),
            project_id: r.get("project_id"),
            name: r.get("name"),
            principal_investigator: r.get("principal_investigator"),
            co_principal_investigator: r.get("co_principal_investigator"),
            sub_investigator: r.get("sub_investigator"),
            hex_code: r.get("hex_code"),
            status: r.get("status"),
            last_activity_at: r.get("last_activity_at"),
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

    /// For dev bypass users: ensure they have a platform_admin membership so they can see
    /// all organizations even if no explicit seed memberships exist.
    pub async fn ensure_dev_platform_admin(
        &self,
        user_id: &Uuid,
        email: &str,
    ) -> anyhow::Result<()> {
        let client = self.pool.get().await?;
        // Insert a platform-level admin membership (NULL org + NULL project)
        let _ = client
            .execute(
                r#"
                INSERT INTO user_memberships (user_id, organization_id, project_id, role, status)
                VALUES ($1, NULL, NULL, 'platform_admin', 'active')
                ON CONFLICT DO NOTHING
                "#,
                &[user_id],
            )
            .await;

        // Also give them org_admin on the first (usually platform_root) organization so classic flows work
        let _ = client
            .execute(
                r#"
                INSERT INTO user_memberships (user_id, organization_id, project_id, role, status)
                SELECT $1, o.id, NULL, 'org_admin', 'active'
                FROM organizations o
                ORDER BY o.created_at ASC
                LIMIT 1
                ON CONFLICT DO NOTHING
                "#,
                &[user_id],
            )
            .await;

        Ok(())
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
        project_id: Uuid,
        site_id: Option<Uuid>,
        external_subject_id: Option<&str>,
        email: Option<&str>,
        date_of_birth: Option<NaiveDate>,
    ) -> anyhow::Result<Patient> {
        let client = self.pool.get().await?;
        let project = self
            .get_project(project_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("project not found for patient creation"))?;
        let organization_id = project.organization_id;

        let hex_code = if let Some(sid) = site_id {
            let site_hex = self.ensure_site_hex_code(&client, sid).await?;
            self.generate_unique_patient_hex(&client, &site_hex).await?
        } else {
            // Fall back to project-based hex when no site is assigned yet
            let proj_hex = self.ensure_project_hex_code(&client, project_id).await?;
            self.generate_unique_patient_hex(&client, &proj_hex).await?
        };

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

        if let Some(sid) = site_id {
            self.touch_site_activity(sid).await?;
        }
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
        email: Option<&str>,
        phone_number: Option<&str>,
        npi_number: Option<&str>,
        address: Option<&str>,
        notes: Option<&str>,
    ) -> anyhow::Result<Provider> {
        let client = self.pool.get().await?;
        let org_hex = self
            .ensure_organization_hex_code(&client, organization_id)
            .await?;
        let hex_code = self.generate_unique_provider_hex(&client, &org_hex).await?;
        let row = client
            .query_one(
                r#"
                INSERT INTO providers (organization_id, name, title, referral_source, hex_code, email, phone_number, npi_number, address, notes)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
                RETURNING id, organization_id, name, title, referral_source, hex_code, email, phone_number, npi_number, address, notes, created_at
                "#,
                &[&organization_id, &name, &title, &referral_source, &hex_code, &email, &phone_number, &npi_number, &address, &notes],
            )
            .await?;
        Ok(row_to_provider(&row))
    }
    pub async fn get_provider(&self, provider_id: Uuid) -> anyhow::Result<Option<Provider>> {
        let client = self.pool.get().await?;
        let row = client
            .query_opt(
                r#"
                SELECT id, organization_id, name, title, referral_source, hex_code, email, phone_number, npi_number, address, notes, created_at
                FROM providers
                WHERE id = $1
                "#,
                &[&provider_id],
            )
            .await?;
        Ok(row.as_ref().map(row_to_provider))
    }
    pub async fn list_providers_by_organization(
        &self,
        organization_id: Uuid,
    ) -> anyhow::Result<Vec<Provider>> {
        let client = self.pool.get().await?;
        let rows = client
            .query(
                r#"
                SELECT id, organization_id, name, title, referral_source, hex_code, email, phone_number, npi_number, address, notes, created_at
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

    /// For org-level sites (not attached to any project/study), we need a full 9-character
    /// hex code to satisfy the sites_hex_code_format CHECK constraint.
    /// We use the organization's 3-char prefix + 6 random chars (with enough letters).
    async fn generate_unique_org_level_site_hex(
        &self,
        client: &deadpool_postgres::Client,
        org_hex: &str,
    ) -> anyhow::Result<String> {
        for _ in 0..4096 {
            let candidate = format!("{}{}", org_hex, random_hex_segment(6, 2));
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
        Err(anyhow!("unable to allocate unique org-level site hex code"))
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
    pub async fn insert_audit_log(
        &self,
        entity_table: &str,
        entity_id: Uuid,
        action: &str,
        changed_by_user_id: Option<Uuid>,
        old_data: Option<&str>,
        new_data: Option<&str>,
        reason: Option<&str>,
    ) -> anyhow::Result<AuditLog> {
        let client = self.pool.get().await?;
        let row = client
            .query_one(
                r#"
                INSERT INTO audit_logs (
                    entity_table,
                    entity_id,
                    action,
                    changed_by_user_id,
                    old_data,
                    new_data,
                    reason
                )
                VALUES ($1, $2, $3, $4, $5::TEXT::JSONB, $6::TEXT::JSONB, $7)
                RETURNING
                    id,
                    entity_table,
                    entity_id,
                    action,
                    changed_by_user_id,
                    old_data::TEXT AS old_data,
                    new_data::TEXT AS new_data,
                    reason,
                    created_at
                "#,
                &[
                    &entity_table,
                    &entity_id,
                    &action,
                    &changed_by_user_id,
                    &old_data,
                    &new_data,
                    &reason,
                ],
            )
            .await?;
        Ok(row_to_audit_log(&row))
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

#[allow(dead_code)]
fn row_to_audit_log(row: &Row) -> AuditLog {
    AuditLog {
        id: row.get("id"),
        entity_table: row.get("entity_table"),
        entity_id: row.get("entity_id"),
        action: row.get("action"),
        changed_by_user_id: row.get("changed_by_user_id"),
        old_data: row.get("old_data"),
        new_data: row.get("new_data"),
        reason: row.get("reason"),
        created_at: row.get("created_at"),
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
        status: row.get("status"),
        last_activity_at: row.get("last_activity_at"),
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
        branching_logic_json: row.get("branching_logic_json"),
        edit_checks_json: row.get("edit_checks_json"),
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
        sdv_status: row.get("sdv_status"),
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

fn row_to_site_startup_checklist_item(row: &Row) -> SiteStartupChecklistItem {
    SiteStartupChecklistItem {
        id: row.get("id"),
        site_id: row.get("site_id"),
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
        email: row.get("email"),
        phone_number: row.get("phone_number"),
        npi_number: row.get("npi_number"),
        address: row.get("address"),
        notes: row.get("notes"),
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

fn row_to_site(row: &Row) -> Site {
    Site {
        id: row.get("id"),
        organization_id: row.get("organization_id"),
        project_id: row.get("project_id"),
        name: row.get("name"),
        principal_investigator: row.get("principal_investigator"),
        co_principal_investigator: row.get("co_principal_investigator"),
        sub_investigator: row.get("sub_investigator"),
        hex_code: row.get("hex_code"),
        status: row.get("status"),
        last_activity_at: row.get("last_activity_at"),
        created_at: row.get("created_at"),
    }
}
