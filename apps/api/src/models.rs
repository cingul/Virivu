use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Organization {
    pub id: Uuid,
    pub name: String,
    pub hex_code: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: Uuid,
    pub organization_id: Uuid,
    pub name: String,
    pub therapeutic_area: String,
    pub protocol_code: Option<String>,
    pub lifecycle_phase: String,
    pub planned_enrollment: i32,
    pub clinicaltrials_gov_id: Option<String>,
    pub study_summary: String,
    pub phase_changed_at: DateTime<Utc>,
    pub hex_code: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Site {
    pub id: Uuid,
    pub project_id: Uuid,
    pub name: String,
    pub principal_investigator: String,
    pub hex_code: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Patient {
    pub id: Uuid,
    pub organization_id: Uuid,
    pub project_id: Uuid,
    pub site_id: Option<Uuid>,
    pub external_subject_id: Option<String>,
    pub email: Option<String>,
    pub date_of_birth: Option<NaiveDate>,
    pub hex_code: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provider {
    pub id: Uuid,
    pub organization_id: Uuid,
    pub name: String,
    pub title: String,
    pub referral_source: String,
    pub hex_code: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Encounter {
    pub id: Uuid,
    pub patient_id: Uuid,
    pub provider_id: Option<Uuid>,
    pub encounter_type: String,
    pub notes: String,
    pub hex_code: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StudyReadiness {
    pub lifecycle_phase: String,
    pub total_sites: i64,
    pub total_patients: i64,
    pub total_encounters: i64,
    pub published_crf_templates: i64,
    pub draft_crf_templates: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StudyPhaseEvent {
    pub id: Uuid,
    pub project_id: Uuid,
    pub previous_phase: String,
    pub new_phase: String,
    pub changed_by_user_id: Option<Uuid>,
    pub notes: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StudyCrfTemplate {
    pub id: Uuid,
    pub project_id: Uuid,
    pub name: String,
    pub description: String,
    pub version: i32,
    pub status: String,
    pub applicable_phase: String,
    pub created_by_user_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StudyCrfField {
    pub id: Uuid,
    pub template_id: Uuid,
    pub field_key: String,
    pub field_label: String,
    pub field_type: String,
    pub required: bool,
    pub options_json: String,
    pub display_order: i32,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StudyVisitTemplate {
    pub id: Uuid,
    pub project_id: Uuid,
    pub visit_code: String,
    pub visit_name: String,
    pub target_day: i32,
    pub window_before_days: i32,
    pub window_after_days: i32,
    pub required: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatientStudyVisit {
    pub id: Uuid,
    pub project_id: Uuid,
    pub patient_id: Uuid,
    pub visit_template_id: Uuid,
    pub scheduled_for: Option<NaiveDate>,
    pub status: String,
    pub completed_at: Option<DateTime<Utc>>,
    pub locked: bool,
    pub locked_at: Option<DateTime<Utc>>,
    pub locked_by_user_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StudyCrfSubmission {
    pub id: Uuid,
    pub project_id: Uuid,
    pub template_id: Uuid,
    pub patient_id: Uuid,
    pub patient_visit_id: Option<Uuid>,
    pub answers_json: String,
    pub status: String,
    pub entered_by_user_id: Option<Uuid>,
    pub submitted_at: Option<DateTime<Utc>>,
    pub locked_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StudyDataQuery {
    pub id: Uuid,
    pub project_id: Uuid,
    pub submission_id: Uuid,
    pub field_key: String,
    pub query_text: String,
    pub status: String,
    pub response_text: String,
    pub raised_by_user_id: Option<Uuid>,
    pub resolved_by_user_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StudyCloseChecklistItem {
    pub id: Uuid,
    pub project_id: Uuid,
    pub item_code: String,
    pub item_label: String,
    pub completed: bool,
    pub completed_by_user_id: Option<Uuid>,
    pub completed_at: Option<DateTime<Utc>>,
    pub notes: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FormInvite {
    pub id: Uuid,
    pub organization_id: Uuid,
    pub project_id: Uuid,
    pub patient_email: String,
    pub form_type: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaUploadTicket {
    pub id: Uuid,
    pub organization_id: Uuid,
    pub project_id: Uuid,
    pub patient_id: String,
    pub mime_type: String,
    pub upload_url: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    pub google_subject: String,
    pub display_name: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserMembership {
    pub organization_id: Option<Uuid>,
    pub project_id: Option<Uuid>,
    pub role: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrganizationSummaryRow {
    pub projects: i64,
    pub sites: i64,
    pub sent_form_invites: i64,
    pub generated_media_upload_links: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectProgressRow {
    pub total_sites: i64,
    pub total_form_invites: i64,
    pub total_media_captures_requested: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataUseAgreement {
    pub id: Uuid,
    pub organization_id: Uuid,
    pub hospital_name: String,
    pub hospital_contact_name: String,
    pub hospital_contact_email: String,
    pub counterparty_name: String,
    pub agreement_version: String,
    pub status: String,
    pub effective_date: Option<NaiveDate>,
    pub expiration_date: Option<NaiveDate>,
    pub agreement_text: String,
    pub hospital_signing_token: Uuid,
    pub created_by_user_id: Option<Uuid>,
    pub signed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataUseAgreementSignature {
    pub id: Uuid,
    pub agreement_id: Uuid,
    pub signer_role: String,
    pub signer_name: String,
    pub signer_email: String,
    pub signer_title: String,
    pub signer_organization: String,
    pub signature_method: String,
    pub signature_text: String,
    pub ip_address: Option<String>,
    pub signed_by_user_id: Option<Uuid>,
    pub signed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboundEmail {
    pub id: Uuid,
    pub agreement_id: Option<Uuid>,
    pub recipient_email: String,
    pub subject: String,
    pub body: String,
    pub status: String,
    pub requested_by_user_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}
