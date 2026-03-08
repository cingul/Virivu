use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Organization {
    pub id: Uuid,
    pub name: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: Uuid,
    pub organization_id: Uuid,
    pub name: String,
    pub therapeutic_area: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Site {
    pub id: Uuid,
    pub project_id: Uuid,
    pub name: String,
    pub principal_investigator: String,
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
