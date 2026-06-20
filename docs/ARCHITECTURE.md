# Virivu Architecture (Rust + Postgres)

## High-Level Components

1. **API Layer (Rust / Axum)**
   - AuthN/AuthZ middleware
   - Study operations, form delivery, media endpoints
   - Analytics and reporting endpoints

2. **Data Layer (PostgreSQL)**
   - Multi-tenant schema
   - Study, site, participant, form, consent, submission tables
   - Audit log and milestone tracking

3. **Object Storage**
   - PHI-safe bucket partitioning
   - Signed URL upload/download
   - Lifecycle and retention rules

4. **Async Worker Layer**
   - Reminder notifications
   - Report generation
   - AI processing pipelines (transcription, note drafting)

5. **Client Applications**
   - Web console for researchers/sponsors
   - Mobile/web patient app for intake, media, and consent

## Multi-Tenant Model

- `organizations` own multiple `projects` (trials/studies)
- each `project` has one or more `sites`
- patients belong to sites or are linked by enrollment events
- all PHI-bearing entities include tenant/project ownership metadata

## Recommended Security Controls

- OIDC SSO with Google Workspace
- MFA + conditional access
- Row-level security strategy (at DB and service layers)
- Immutable audit events for:
  - login/auth actions
  - form submissions
  - consent changes
  - media access/download
- Signed URLs with short TTL for patient uploads

## AI Workloads

AI features should be isolated into dedicated services with:
- strict consent checks
- traceable prompts and model outputs
- configurable PHI masking
- human-in-the-loop approval for clinical notes

## Deployment Pattern

- Containerized API services
- Managed Postgres with PITR backups
- Object storage with encryption and access logs
- Queue + worker services for long-running tasks
- Observability: logs, traces, metrics, security events

## API Design Guidance

- Versioned REST paths (`/v1/...`)
- Event-first model for auditability
- Idempotent write operations for external integrations
- Pagination and server-side filtering for operational dashboards
