use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::str::FromStr;
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

impl StudyPhase {
    pub fn as_db(&self) -> &'static str {
        match self {
            StudyPhase::PreStudy => "pre_study",
            StudyPhase::Initiation => "initiation",
            StudyPhase::Active => "active",
            StudyPhase::Monitoring => "monitoring",
            StudyPhase::Closed => "closed",
        }
    }
}

impl FromStr for StudyPhase {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "pre_study" => Ok(StudyPhase::PreStudy),
            "initiation" => Ok(StudyPhase::Initiation),
            "active" => Ok(StudyPhase::Active),
            "monitoring" => Ok(StudyPhase::Monitoring),
            "closed" => Ok(StudyPhase::Closed),
            other => Err(format!("invalid study phase value: {other}")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VisitStatus {
    Planned,
    Completed,
    Missed,
}

impl VisitStatus {
    pub fn as_db(&self) -> &'static str {
        match self {
            VisitStatus::Planned => "planned",
            VisitStatus::Completed => "completed",
            VisitStatus::Missed => "missed",
        }
    }
}

impl FromStr for VisitStatus {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "planned" => Ok(VisitStatus::Planned),
            "completed" => Ok(VisitStatus::Completed),
            "missed" => Ok(VisitStatus::Missed),
            other => Err(format!("invalid visit status value: {other}")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SubmissionStatus {
    Draft,
    Submitted,
    Locked,
}

impl SubmissionStatus {
    pub fn as_db(&self) -> &'static str {
        match self {
            SubmissionStatus::Draft => "draft",
            SubmissionStatus::Submitted => "submitted",
            SubmissionStatus::Locked => "locked",
        }
    }
}

impl FromStr for SubmissionStatus {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "draft" => Ok(SubmissionStatus::Draft),
            "submitted" => Ok(SubmissionStatus::Submitted),
            "locked" => Ok(SubmissionStatus::Locked),
            other => Err(format!("invalid submission status value: {other}")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum QueryStatus {
    Open,
    Responded,
    Closed,
}

impl QueryStatus {
    pub fn as_db(&self) -> &'static str {
        match self {
            QueryStatus::Open => "open",
            QueryStatus::Responded => "responded",
            QueryStatus::Closed => "closed",
        }
    }
}

impl FromStr for QueryStatus {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "open" => Ok(QueryStatus::Open),
            "responded" => Ok(QueryStatus::Responded),
            "closed" => Ok(QueryStatus::Closed),
            other => Err(format!("invalid data query status value: {other}")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgreementStatus {
    Draft,
    PendingSignatures,
    Active,
}

impl AgreementStatus {
    pub fn as_db(&self) -> &'static str {
        match self {
            AgreementStatus::Draft => "draft",
            AgreementStatus::PendingSignatures => "pending_signatures",
            AgreementStatus::Active => "active",
        }
    }
}

impl FromStr for AgreementStatus {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "draft" => Ok(AgreementStatus::Draft),
            "pending_signatures" => Ok(AgreementStatus::PendingSignatures),
            "active" => Ok(AgreementStatus::Active),
            other => Err(format!("invalid agreement status value: {other}")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AppRole {
    PlatformAdmin,
    OrgAdmin,
    Investigator,
    SiteCoordinator,
    Analyst,
    Monitor,
}

impl AppRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            AppRole::PlatformAdmin => "platform_admin",
            AppRole::OrgAdmin => "org_admin",
            AppRole::Investigator => "investigator",
            AppRole::SiteCoordinator => "site_coordinator",
            AppRole::Analyst => "analyst",
            AppRole::Monitor => "monitor",
        }
    }
}

impl FromStr for AppRole {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "platform_admin" => Ok(AppRole::PlatformAdmin),
            "org_admin" => Ok(AppRole::OrgAdmin),
            "investigator" => Ok(AppRole::Investigator),
            "site_coordinator" => Ok(AppRole::SiteCoordinator),
            "analyst" => Ok(AppRole::Analyst),
            "monitor" => Ok(AppRole::Monitor),
            other => Err(format!("invalid role: {other}")),
        }
    }
}

#[derive(Debug, Clone)]
pub struct AuthenticatedUser {
    pub user_id: Uuid,
    pub subject: String,
    pub email: Option<String>,
    pub role: AppRole,
    pub organization_scope: Option<Uuid>,
    pub auth_source: String,
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
pub struct CrfTemplateVersion {
    pub id: Uuid,
    pub template_id: Uuid,
    pub version_number: i32,
    pub schema_json: serde_json::Value,
    pub is_published: bool,
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
pub struct DataQueryComment {
    pub id: Uuid,
    pub query_id: Uuid,
    pub author_user_id: Option<Uuid>,
    pub comment_text: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisitScheduleTemplate {
    pub id: Uuid,
    pub study_id: Uuid,
    pub name: String,
    pub day_offset: i32,
    pub window_before_days: i32,
    pub window_after_days: i32,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloseoutChecklistItem {
    pub id: Uuid,
    pub study_id: Uuid,
    pub item_key: String,
    pub item_label: String,
    pub is_required: bool,
    pub is_complete: bool,
    pub completed_at: Option<DateTime<Utc>>,
    pub completed_by_user_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
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
    pub pending_closeout_items: usize,
    pub has_active_dua: bool,
    pub next_recommended_action: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: Uuid,
    pub subject: String,
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub platform_role: AppRole,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrganizationMembership {
    pub id: Uuid,
    pub user_id: Uuid,
    pub organization_id: Uuid,
    pub role: AppRole,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewAuditLogEntry {
    pub actor_user_id: Option<Uuid>,
    pub actor_subject: String,
    pub actor_role: Option<AppRole>,
    pub actor_email: Option<String>,
    pub auth_source: Option<String>,
    pub method: String,
    pub path: String,
    pub action: Option<String>,
    pub resource_type: Option<String>,
    pub resource_id: Option<Uuid>,
    pub metadata_json: Option<serde_json::Value>,
    pub status_code: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditLogRecord {
    pub id: Uuid,
    pub actor_user_id: Option<Uuid>,
    pub actor_subject: String,
    pub actor_role: Option<AppRole>,
    pub actor_email: Option<String>,
    pub auth_source: Option<String>,
    pub method: String,
    pub path: String,
    pub action: Option<String>,
    pub resource_type: Option<String>,
    pub resource_id: Option<Uuid>,
    pub metadata_json: Option<serde_json::Value>,
    pub status_code: i32,
    pub happened_at: DateTime<Utc>,
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
pub struct CreateCrfTemplateVersionRequest {
    pub schema_json: serde_json::Value,
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
pub struct CreateDataQueryCommentRequest {
    pub comment_text: String,
}

#[derive(Debug, Deserialize)]
pub struct RespondDataQueryRequest {
    pub comment_text: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateVisitScheduleTemplateRequest {
    pub name: String,
    pub day_offset: i32,
    pub window_before_days: i32,
    pub window_after_days: i32,
}

#[derive(Debug, Deserialize)]
pub struct CreateCloseoutChecklistItemRequest {
    pub item_key: String,
    pub item_label: String,
    pub is_required: bool,
}

#[derive(Debug, Deserialize)]
pub struct CompleteCloseoutChecklistItemRequest {
    pub is_complete: bool,
}

#[derive(Debug, Deserialize)]
pub struct CreateDuaRequest {
    pub organization_id: Uuid,
    pub counterparty: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateMembershipRequest {
    pub subject: String,
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub organization_id: Uuid,
    pub role: AppRole,
}

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub service: String,
    pub timestamp_utc: DateTime<Utc>,
}
