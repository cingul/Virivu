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

## Current scope (Phase 2 baseline)

Virival now includes:

- Organizations
- Studies + phase transitions
- Sites + startup readiness
- Patients + enrollment
- Visits
- CRF templates + submissions
- Data queries
- DUAs
- Study readiness summary
- PostgreSQL-backed persistence with boot-time SQL migrations
- Repository interface + PostgreSQL implementation (domain logic decoupled from handlers)
- Auth/RBAC middleware skeleton (`x-virival-user`, `x-virival-role`)
- First wizard UI shell at `/ui`

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
- `POST /api/v1/crf-templates/{template_id}/publish`
- `POST /api/v1/crf-submissions`
- `POST /api/v1/crf-submissions/{submission_id}/lock`
- `POST /api/v1/data-queries`
- `POST /api/v1/data-queries/{query_id}/close`
- `POST /api/v1/duas`
- `POST /api/v1/duas/{dua_id}/activate`
- `GET /api/v1/studies/{study_id}/readiness`
- `POST /api/v1/studies/{study_id}/phase`

## Auth / RBAC skeleton

- Middleware protects `/ui` and all `/api/v1/*` routes.
- Dev default (if `ALLOW_DEV_AUTH_BYPASS=true`): automatic `platform_admin` identity when headers are missing.
- Explicit headers for testing:
  - `x-virival-user: you@org.tld`
  - `x-virival-role: platform_admin|org_admin|investigator|site_coordinator|analyst|monitor`

## Phase transition rules (opinionated)

- `pre_study -> initiation` requires:
  - active DUA
  - at least one startup-complete site
  - at least one published CRF template
- `initiation -> active` requires at least one enrolled patient
- `active -> monitoring` requires at least one locked CRF submission
- `monitoring -> closed` requires zero unresolved data queries

This is the first layer of enforcement for a frictionless, compliant study workflow.
