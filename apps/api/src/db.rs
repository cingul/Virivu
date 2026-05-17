use anyhow::anyhow;
use chrono::{Duration, Utc};
use deadpool_postgres::{Manager, ManagerConfig, Pool, RecyclingMethod, Runtime};
use std::net::IpAddr;
use tokio_postgres::{NoTls, Row};
use uuid::Uuid;

use crate::models::{
    DataUseAgreement, DataUseAgreementSignature, FormInvite, MediaUploadTicket, Organization,
    OrganizationSummaryRow, OutboundEmail, Project, ProjectProgressRow, Site, User, UserMembership,
};

#[derive(Clone)]
pub struct Db {
    pool: Pool,
}

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

    pub async fn create_organization(&self, name: &str) -> anyhow::Result<Organization> {
        let client = self.pool.get().await?;
        let row = client
            .query_one(
                r#"
                INSERT INTO organizations (name)
                VALUES ($1)
                RETURNING id, name, created_at
                "#,
                &[&name],
            )
            .await?;
        Ok(Organization {
            id: row.get("id"),
            name: row.get("name"),
            created_at: row.get("created_at"),
        })
    }

    pub async fn create_project(
        &self,
        organization_id: Uuid,
        name: &str,
        therapeutic_area: &str,
    ) -> anyhow::Result<Project> {
        let client = self.pool.get().await?;
        let row = client
            .query_one(
                r#"
                INSERT INTO projects (organization_id, name, therapeutic_area)
                VALUES ($1, $2, $3)
                RETURNING id, organization_id, name, therapeutic_area, created_at
                "#,
                &[&organization_id, &name, &therapeutic_area],
            )
            .await?;
        Ok(Project {
            id: row.get("id"),
            organization_id: row.get("organization_id"),
            name: row.get("name"),
            therapeutic_area: row.get("therapeutic_area"),
            created_at: row.get("created_at"),
        })
    }

    pub async fn create_site(
        &self,
        project_id: Uuid,
        name: &str,
        principal_investigator: &str,
    ) -> anyhow::Result<Site> {
        let client = self.pool.get().await?;
        let row = client
            .query_one(
                r#"
                INSERT INTO sites (project_id, name, principal_investigator)
                VALUES ($1, $2, $3)
                RETURNING id, project_id, name, principal_investigator, created_at
                "#,
                &[&project_id, &name, &principal_investigator],
            )
            .await?;
        Ok(Site {
            id: row.get("id"),
            project_id: row.get("project_id"),
            name: row.get("name"),
            principal_investigator: row.get("principal_investigator"),
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
                SELECT id, organization_id, name, therapeutic_area, created_at
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
