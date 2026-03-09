# Virivu Handoff

This file is the continuity bridge for new chat sessions and new machines.

## Repository

- GitHub: `cingul/Virivu`
- Active branch: `cursor/research-data-platform-d3b4`

## Key commits on this branch

1. `a359a18` - Scaffold Virivu research platform foundation
2. `6751537` - Add Postgres persistence and RBAC auth scaffolding
3. `d264794` - Set default Google Workspace domain to `cingulum.org`
4. `146e442` - Seed `arcot@cingulum.org` as `platform_admin`

## Current implementation status

### Implemented

- Product and architecture docs:
  - `docs/PRODUCT_BLUEPRINT.md`
  - `docs/ARCHITECTURE.md`
  - `docs/COMPETITIVE_FEATURES.md`
- Rust API scaffold (`apps/api`) with:
  - Multi-tenant entities (org/project/site)
  - Form invite endpoint
  - Media upload-ticket endpoint
  - Analytics summary endpoints
  - AI transcript-to-note placeholder endpoint
- Postgres persistence integrated (via `deadpool-postgres` + `tokio-postgres`)
- RBAC scaffold with authenticated user context and role checks
- Domain policy set to `cingulum.org`
- Local migration helper script:
  - `apps/api/scripts/apply_migrations.sh`

### Important security note

Google ID token handling currently validates JWT claims (issuer/audience/domain/exp) but **does not yet cryptographically verify JWT signatures via Google JWKS**.  
This is the highest-priority production hardening task.

## Database migrations

- `apps/api/migrations/0001_init.sql` - core schema
- `apps/api/migrations/0002_dev_seed.sql` - local dev seed users/roles
  - Includes:
    - `admin@cingulum.org` (`platform_admin`)
    - `arcot@cingulum.org` (`platform_admin`)

## Local run (quick)

```bash
docker compose -f deploy/docker-compose.yml up -d postgres
cd apps/api
cp .env.example .env
export DATABASE_URL=postgres://postgres:postgres@localhost:5432/virivu
./scripts/apply_migrations.sh
cargo run
```

Health check:

```bash
curl http://localhost:8080/health
```

## What is needed from owner (Arcot)

- Google OAuth Web Client ID (for production auth validation)
- (Optional next) redirect URIs and JS origins for frontend auth flows

## Highest-priority next task

Implement full Google OIDC signature verification:

1. Fetch and cache Google JWKS keys.
2. Verify JWT signature (kid + alg + key).
3. Enforce `iss`, `aud`, `exp`, `hd`.
4. Keep domain restricted to `cingulum.org`.
5. Remove/disable non-production token parsing path in production mode.

## Suggested prompt for new Cursor session

> Continue work on `cingul/Virivu` branch `cursor/research-data-platform-d3b4`.  
> Read `HANDOFF.md` first, then implement production-grade Google JWKS verification in `apps/api/src/auth.rs`, keeping RBAC behavior unchanged and preserving `cargo check` passing.
