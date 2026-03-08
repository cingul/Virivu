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