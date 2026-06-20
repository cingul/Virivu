# Virivu Research Cloud

Virivu is a modern, HIPAA-oriented, REDCap-class research platform for:

- Academic institutions and hospital systems
- Device and pharmaceutical sponsors
- Private practices and multi-site collaborators

It is designed to reduce operational friction in clinical and translational research with:

- Multi-tenant organizations, studies, and sites
- Patient-facing forms (demographics, intake, eConsent, questionnaires)
- Secure media capture and upload (photos/videos from patients/families)
- Trial/site dashboards, milestones, and progress reporting
- Cross-project analytics and statistical analysis surfaces
- AI-assisted workflows (patient guidance, documentation support, transcript-to-note)
- Subscription-ready billing model
- Electronic Data Use Agreement (DUA) workflows for hospitals and Cingulum Foundation Inc.
- Browser-accessible DUA drafting/signing pages and PDF export endpoints
- In-browser organization setup for end-to-end DUA onboarding
- Unified web app dashboard at `/ui/app` that ties operations, patient workflows, analytics, and legal flows together
- Study lifecycle + CRF workbench at `/ui/studies` for pre-study setup, phase transitions, and structured case report form design
- Phase-gated study activation logic (requires site + published CRF before initiation, and enrolled patient before active phase)
- Operational study execution modules: visit templates/scheduling, CRF submission workflow (draft/submitted/locked), monitor query management, and close-checklist gating before closure
- Startup checklist gating before study initiation and operational summary signals (enrollment gap, open queries, pending startup/close items)
- UI datalist selectors in `/ui/app` and `/ui/studies` to reduce UUID copy/paste friction during daily operations
- Organization hierarchy with Cingulum Foundation root tenancy (`platform_root` + child org workspaces), org-scoped DUA consoles, and card-based animated UI refinements

## Core Stack (v1 scaffold)

- **Backend:** Rust (Axum, Tokio)
- **Database:** PostgreSQL (schema + migration seed included)
- **Infra:** Docker Compose (Postgres + API)
- **Identity:** Google Workspace-aligned token claim checks + RBAC model

## Repository Layout

- `docs/`
  - `PRODUCT_BLUEPRINT.md` - product strategy, modules, roadmap, pricing model
  - `ARCHITECTURE.md` - system architecture and compliance-oriented design
  - `COMPETITIVE_FEATURES.md` - benchmark against leading research platforms
- `apps/api/`
  - Rust API service scaffold
  - Domain models and REST endpoints for key workflows
  - SQL migration for core multi-tenant clinical research entities
- `virival/`
  - New greenfield Rust-first rebuild for next-generation end-to-end research workflow orchestration
  - Includes phase-gated lifecycle API baseline and architect handoff prompt (`ARCHITECT_HANDOFF.md`)
- `deploy/docker-compose.yml`
  - Local stack bootstrap for Postgres + API

## Quick Start (local)

1. Start infrastructure:
   ```bash
   docker compose -f deploy/docker-compose.yml up -d postgres
   ```
2. Apply migrations and run API:
   ```bash
   cd apps/api
   cp .env.example .env
   export DATABASE_URL=postgres://postgres:postgres@localhost:5432/virivu
   ./scripts/apply_migrations.sh
   cargo run
   ```
3. Health check:
   ```bash
   curl http://localhost:8080/health
   ```

## Next Build Steps

1. Replace JWT claim-only validation with full Google JWKS signature verification.
2. Implement signed object storage uploads for media (S3/GCS adapters).
3. Build web portal and cross-platform mobile app (patient + coordinator flows).
4. Add background jobs (notifications, reminders, report generation).
5. Implement statistics workspace and configurable analysis templates.

---

Virivu aims to be "research operations + patient engagement + analytics + AI assistance" in one cohesive platform.