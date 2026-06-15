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

## Current scope (v0 foundation)

The initial version includes a runnable API with domain slices for:

- Organizations
- Studies + phase transitions
- Sites + startup readiness
- Patients + enrollment
- Visits
- CRF templates + submissions
- Data queries
- DUAs
- Study readiness summary

Data is in-memory for now (fast iteration baseline).  
Next step is persistence (Postgres + migrations), auth, audit logs, and UI.

## Run locally

```bash
cd virival
cargo run
```

Defaults:
- `APP_NAME=virival-api`
- `BIND_ADDRESS=0.0.0.0:8090`

Health:

```bash
curl http://localhost:8090/health
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

## Phase transition rules (opinionated)

- `pre_study -> initiation` requires:
  - active DUA
  - at least one startup-complete site
  - at least one published CRF template
- `initiation -> active` requires at least one enrolled patient
- `active -> monitoring` requires at least one locked CRF submission
- `monitoring -> closed` requires zero unresolved data queries

This is the first layer of enforcement for a frictionless, compliant study workflow.
