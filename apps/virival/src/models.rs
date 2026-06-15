use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StudyPhase {
    PreStudy,
    Initiation,
    Active,
    Monitoring,
    Closed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VisitStatus {
    Planned,
    Completed,
    Missed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SubmissionStatus {
    Draft,
    Submitted,
    Locked,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum QueryStatus {
    Open,
    Responded,
    Closed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgreementStatus {
    Draft,
    PendingSignatures,
    Active,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Organization {
    pub id: Uuid,
    pub name: String,
    pub workspace_slug: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Study {
    pub id: Uuid,
    pub organization_id: Uuid,
    pub short_code: String,
    pub title: String,
    pub phase: StudyPhase,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Site {
    pub id: Uuid,
    pub organization_id: Uuid,
    pub study_id: Option<Uuid>,
    pub name: String,
    pub principal_investigator: String,
    pub startup_complete: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Patient {
    pub id: Uuid,
    pub organization_id: Uuid,
    pub study_id: Uuid,
    pub site_id: Option<Uuid>,
    pub external_id: String,
    pub enrolled_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Visit {
    pub id: Uuid,
    pub study_id: Uuid,
    pub patient_id: Uuid,
    pub visit_name: String,
    pub scheduled_for: DateTime<Utc>,
    pub status: VisitStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrfTemplate {
    pub id: Uuid,
    pub study_id: Uuid,
    pub name: String,
    pub published: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrfSubmission {
    pub id: Uuid,
    pub study_id: Uuid,
    pub patient_id: Uuid,
    pub visit_id: Uuid,
    pub template_id: Uuid,
    pub status: SubmissionStatus,
    pub captured_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataQuery {
    pub id: Uuid,
    pub study_id: Uuid,
    pub submission_id: Uuid,
    pub summary: String,
    pub status: QueryStatus,
    pub raised_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DuaAgreement {
    pub id: Uuid,
    pub organization_id: Uuid,
    pub counterparty: String,
    pub status: AgreementStatus,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StudyReadiness {
    pub study_id: Uuid,
    pub phase: StudyPhase,
    pub has_site: bool,
    pub has_published_crf: bool,
    pub has_enrolled_patient: bool,
    pub has_locked_submission: bool,
    pub open_query_count: usize,
    pub has_active_dua: bool,
    pub next_recommended_action: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateOrganizationRequest {
    pub name: String,
    pub workspace_slug: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateStudyRequest {
    pub organization_id: Uuid,
    pub short_code: String,
    pub title: String,
}

#[derive(Debug, Deserialize)]
pub struct TransitionStudyPhaseRequest {
    pub phase: StudyPhase,
}

#[derive(Debug, Deserialize)]
pub struct CreateSiteRequest {
    pub organization_id: Uuid,
    pub study_id: Option<Uuid>,
    pub name: String,
    pub principal_investigator: String,
}

#[derive(Debug, Deserialize)]
pub struct MarkSiteStartupRequest {
    pub startup_complete: bool,
}

#[derive(Debug, Deserialize)]
pub struct EnrollPatientRequest {
    pub organization_id: Uuid,
    pub study_id: Uuid,
    pub site_id: Option<Uuid>,
    pub external_id: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateVisitRequest {
    pub study_id: Uuid,
    pub patient_id: Uuid,
    pub visit_name: String,
    pub scheduled_for: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct CreateCrfTemplateRequest {
    pub study_id: Uuid,
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateCrfSubmissionRequest {
    pub study_id: Uuid,
    pub patient_id: Uuid,
    pub visit_id: Uuid,
    pub template_id: Uuid,
}

#[derive(Debug, Deserialize)]
pub struct CreateDataQueryRequest {
    pub study_id: Uuid,
    pub submission_id: Uuid,
    pub summary: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateDuaRequest {
    pub organization_id: Uuid,
    pub counterparty: String,
}

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub service: String,
    pub timestamp_utc: DateTime<Utc>,
}
