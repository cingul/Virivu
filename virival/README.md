# Virival (Greenfield Rebuild)

Virival is a new, architected-from-scratch baseline for end-to-end research management.

It lives in a top-level isolated folder (`virival/`) so the legacy Virivu code can continue to run while this next-generation platform evolves in parallel.

## Architectural stance

This rebuild prioritizes:

1. **Workflow integrity over ad-hoc forms**
   - Study lifecycle is phase-gated.
   - Preconditions are enforced server-side.
2. **Multi-tenant research operations**
   - Organization-scoped studies, sites, patients, DUAs.
3. **Operational traceability**
   - Every core entity has explicit state/status.
4. **Rust-first backend**
   - Axum + Tokio API foundation.

## Current scope (Phase 7 baseline)

Virival now includes:

- Organizations
- Studies + phase transitions
- Sites + startup readiness
- Patients + enrollment
- Visits
- CRF templates + versioned schemas
- CRF submissions
- Visit schedule templates
- Data queries with response/comment workflow
- Study closeout checklist tracking
- DUAs
- Study readiness summary
- Dedicated tabbed web workbench (`/ui/workbench`) for setup, design, execute, monitor, closeout, and analytics
- Wizard-guided stage cards with context-aware workflow coaching
- Direct UI form actions that orchestrate core operations (org/study/site/DUA/CRF/versioning/patient/visit/submission/query/closeout/phase)
- DUA signature capture + PDF document export
- Reminder jobs queue with processing endpoint for due reminders
- PostgreSQL-backed persistence with boot-time SQL migrations
- Repository interface + PostgreSQL implementation (domain logic decoupled from handlers)
- Auth/RBAC middleware with Google OIDC RS256 signature verification via cached JWKS + dev fallback
- Persisted users + organization memberships (role assignments in DB)
- Enriched audit log trail (action, resource type/id, metadata JSON)
- Wizard shell + membership admin UI + audit console UI

## Run locally

```bash
cd virival
cp .env.example .env
cargo run
```

Defaults:
- `APP_NAME=virival-api`
- `BIND_ADDRESS=0.0.0.0:8090`
- `DATABASE_URL=postgres://postgres:postgres@localhost:5432/virival`
- `ALLOW_DEV_AUTH_BYPASS=true`
- `GOOGLE_WORKSPACE_DOMAIN=cingulum.org`
- `GOOGLE_CLIENT_ID=` (set for strict audience validation)
- `GOOGLE_JWKS_URL=https://www.googleapis.com/oauth2/v3/certs`
- `OIDC_JWKS_CACHE_SECONDS=3600`

Health:

```bash
curl http://localhost:8090/health
```

Wizard shell:

```bash
curl -H "x-virival-user: architect@cingulum.org" -H "x-virival-role: platform_admin" http://localhost:8090/ui
```

## High-value endpoints

- `POST /api/v1/organizations`
- `POST /api/v1/studies`
- `POST /api/v1/sites`
- `POST /api/v1/sites/{site_id}/startup`
- `POST /api/v1/patients`
- `POST /api/v1/crf-templates`
- `POST /api/v1/crf-templates/{template_id}/versions`
- `GET /api/v1/crf-templates/{template_id}/versions`
- `POST /api/v1/crf-template-versions/{version_id}/publish`
- `POST /api/v1/crf-templates/{template_id}/publish`
- `POST /api/v1/crf-submissions`
- `POST /api/v1/crf-submissions/{submission_id}/lock`
- `POST /api/v1/data-queries`
- `POST /api/v1/data-queries/{query_id}/respond`
- `GET /api/v1/data-queries/{query_id}/comments`
- `POST /api/v1/data-queries/{query_id}/comments`
- `POST /api/v1/data-queries/{query_id}/close`
- `POST /api/v1/studies/{study_id}/visit-schedule-templates`
- `GET /api/v1/studies/{study_id}/visit-schedule-templates`
- `POST /api/v1/studies/{study_id}/closeout-checklist`
- `GET /api/v1/studies/{study_id}/closeout-checklist`
- `POST /api/v1/closeout-checklist/{item_id}/complete`
- `POST /api/v1/duas`
- `POST /api/v1/duas/{dua_id}/activate`
- `POST /api/v1/duas/{dua_id}/signatures`
- `GET /api/v1/duas/{dua_id}/signatures`
- `GET /api/v1/duas/{dua_id}/pdf`
- `GET /api/v1/studies/{study_id}/readiness`
- `POST /api/v1/studies/{study_id}/phase`
- `POST /api/v1/admin/memberships`
- `GET /api/v1/admin/organizations/{organization_id}/memberships`
- `GET /api/v1/admin/audit-logs?limit=50`
- `POST /api/v1/admin/reminder-jobs`
- `GET /api/v1/admin/reminder-jobs?limit=50`
- `POST /api/v1/admin/reminder-jobs/process`

## Admin UI routes

- `GET /ui`
- `GET /ui/workbench`
- `POST /ui/workbench/organizations`
- `POST /ui/workbench/studies`
- `POST /ui/workbench/sites`
- `POST /ui/workbench/duas`
- `POST /ui/workbench/dua-signatures`
- `POST /ui/workbench/crf-design`
- `POST /ui/workbench/visit-schedules`
- `POST /ui/workbench/patients`
- `POST /ui/workbench/visits`
- `POST /ui/workbench/submissions`
- `POST /ui/workbench/queries`
- `POST /ui/workbench/reminders`
- `POST /ui/workbench/reminders/process`
- `POST /ui/workbench/closeout-items`
- `POST /ui/workbench/studies/{study_id}/phase`
- `GET /ui/admin/memberships`
- `POST /ui/admin/memberships`
- `GET /ui/admin/audit`

## Auth / RBAC

- Middleware protects `/ui` and all `/api/v1/*` routes.
- Preferred path: Google OIDC `Authorization: Bearer <id_token>` with RS256 signature verification against Google JWKS.
- JWKS are cached in-process and refreshed when cache is stale or key id is missing.
- Current fetch path uses `curl` under the hood to retrieve Google JWKS endpoint data.
- Dev default (if `ALLOW_DEV_AUTH_BYPASS=true`): automatic `platform_admin` identity when headers are missing.
- Explicit headers for testing:
  - `x-virival-user: you@org.tld`
  - `x-virival-role: platform_admin|org_admin|investigator|site_coordinator|analyst|monitor`
  - `x-virival-organization-id: <org_uuid>` (optional scope to resolve membership role)

## Phase transition rules (opinionated)

- `pre_study -> initiation` requires:
  - active DUA
  - at least one startup-complete site
  - at least one published CRF template
- `initiation -> active` requires at least one enrolled patient
- `active -> monitoring` requires at least one locked CRF submission
- `monitoring -> closed` requires:
  - zero unresolved data queries
  - zero pending required closeout checklist items

This is the first layer of enforcement for a frictionless, compliant study workflow.
