use chrono::{Duration, Utc};
use deadpool_postgres::{Manager, ManagerConfig, Pool, RecyclingMethod, Runtime};
use tokio_postgres::NoTls;
use uuid::Uuid;

use crate::models::{
    FormInvite, MediaUploadTicket, Organization, OrganizationSummaryRow, Project,
    ProjectProgressRow, Site, User, UserMembership,
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
}
