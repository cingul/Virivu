# Virivu Research Cloud - New Conversation Continuation Summary

_Last updated: 2026-05-17 (UTC)_
_Branch: `cursor/research-data-platform-d3b4`_
_Latest commit at handoff: `f32d677`_

---

## 1) What this product is

Virivu is being built as a REDCap-equivalent, end-to-end clinical research platform for:

- Cingulum Foundation Inc. (top-level platform org)
- Cingulum Research and partner institutions
- Hospitals (e.g., Wyckoff Heights), sponsors, private practices, and collaborators

Primary goals:

- Low-friction study startup and execution
- Strong multi-tenant separation
- Patient data capture (forms, consent, media)
- Site/study operations and monitoring workflows
- DUA/legal workflows in-app
- Future analytics + AI-assisted workflows

---

## 2) Current user feedback (most recent)

The UI updates are visible now, but the user says:

> "the workflow is confusing"

This is the immediate product priority for the next conversation:

1. Simplify navigation and reduce context switching
2. Provide guided "next step" progression
3. Make onboarding/startup flow linear and obvious
4. Keep strong org isolation and DUA scoping behavior

---

## 3) What has already been implemented

### 3.1 Core backend/platform

- Rust API with Axum/Tokio
- Postgres persistence (`deadpool-postgres`, `tokio-postgres`)
- Diesel migrations and migration runner
- Auto-run pending migrations on API startup
- RBAC scaffolding + Google Workspace domain alignment (`cingulum.org`)
- Dev auth bypass support via headers for local workflows

### 3.2 Multi-tenant org hierarchy (newly added)

Migration:

- `apps/api/migrations/2026-05-17-000008_organization_hierarchy`

New `organizations` fields:

- `parent_organization_id`
- `organization_kind` (`platform_root`, `research_network`, `hospital`, `tenant`, `sponsor`)
- `workspace_slug`

Behavior:

- Seeds/normalizes `Cingulum Foundation Inc.` as `platform_root`
- Backfills existing orgs under root if missing parent
- Org creation supports parent + kind inputs
- Non-root orgs default to root parent if parent omitted

### 3.3 DUA workflow (implemented and tightened)

- DUA drafting, hospital token signing, Cingulum signing, outbound email queue, PDF export
- DUA admin UI: `/ui/dua`
- Agreement workspace UI: `/ui/dua/{agreement_id}`
- Added org-scoped display + permissions:
  - DUA console list is scoped to selected organization
  - Agreement workspace requires `admin_email` context and org-manager authorization
  - DUA links preserve `admin_email` context so user remains in tenant scope

### 3.4 Unified operations web UI

- `/ui/app`: tabbed workspace for org/project/site/patient/provider/encounter/legal/analytics
- `/ui/studies`: tabbed study lifecycle + CRF workbench
- Datalist selectors added broadly to reduce UUID copy/paste friction
- Card-based styling + animations, visible "UI Refresh v3" badge

### 3.5 Study lifecycle and operations

- Study phases and transitions with readiness gating
- CRF templates + CRF fields
- Visit templates + patient visit scheduling
- CRF submissions (draft/submitted/locked)
- Data query tracking (open/responded/closed)
- Startup checklist and close checklist gating logic

### 3.6 Hex identifier workflow

Implemented hierarchical hex code generation across org/study/site/patient/provider/encounter domains with uniqueness checks and encounter-range logic.

---

## 4) Key files touched recently (high value for next agent)

- `apps/api/src/routes.rs`
  - Main SSR UI rendering + forms + handlers
  - DUA UI scoping rules + workflow endpoints
  - Theme/CSS and tab behavior
- `apps/api/src/db.rs`
  - Organization creation + hierarchy helpers
  - DUA and study operational data access logic
- `apps/api/src/models.rs`
  - `Organization` model includes hierarchy fields
- `apps/api/migrations/2026-05-17-000008_organization_hierarchy/*`
  - Schema for parent-child org tree and root seeding
- `README.md`
- `HANDOFF.md`

---

## 5) Current workflow pain (why user still feels confused)

Likely causes:

1. Too many forms on single pages (high cognitive load)
2. UUID-centric inputs still visible despite datalists
3. No clear "Start here -> then this -> then this" wizard
4. DUA and study flows are functional but not arranged as guided journeys
5. Workspace context (`admin_email`, organization, project) is not always obvious as persistent state

---

## 6) Recommended immediate redesign plan (next conversation)

### P0 UX objective

Transform from "feature hub" to "guided workflow app".

### Phase A: Guided navigation shell

- Add persistent left sidebar:
  - Onboarding
  - Organizations
  - Studies
  - Sites
  - Patients
  - Visits/CRFs
  - Monitoring
  - DUA/Legal
- Add global context bar (sticky):
  - Current user/admin
  - Current org
  - Current study
  - Quick switchers

### Phase B: Convert forms into step-based wizards

1. **Start Study Wizard**
   - Select org
   - Create study metadata
   - Add site(s)
   - Define CRF template(s)
   - Setup startup checklist
   - "Initiate study" gate summary
2. **Patient Enrollment Wizard**
   - Select study/site
   - Create patient
   - Schedule baseline visit
   - Create first CRF submission
3. **DUA Wizard**
   - Select org workspace
   - Draft agreement
   - Send signing link
   - Track signatures
   - Export PDF

### Phase C: Reduce UUID exposure

- Prefer dropdowns by human labels + hex IDs
- Hide raw UUID fields in advanced sections only

### Phase D: Dashboard cards with action cues

- "Next actions" card (top 5 tasks)
- "Blocked by" card (gating failures)
- "At risk" studies card (open queries, overdue visits)

---

## 7) Security and production hardening still pending

1. Full Google JWKS signature verification (highest priority hardening)
2. Production session auth for UI
3. Stronger authorization checks on every write path
4. Expanded audit logging
5. Sensitive-field encryption + secure media URL strategy
6. Email queue worker with retries/dead-letter strategy
7. Metrics/tracing dashboards + backup/restore runbooks

---

## 8) Runbook for local execution

From repo root:

```bash
docker compose -f deploy/docker-compose.yml up -d postgres
cd apps/api
cp .env.example .env
export DATABASE_URL=postgres://postgres:postgres@localhost:5432/virivu
./scripts/apply_migrations.sh
cargo run --bin virivu-api
```

UI entry points:

- `http://localhost:8080/ui/app?admin_email=arcot@cingulum.org`
- `http://localhost:8080/ui/studies?admin_email=arcot@cingulum.org`
- `http://localhost:8080/ui/dua?admin_email=arcot@cingulum.org`

If UI seems stale:

1. `git pull --ff-only`
2. Restart API process
3. Hard refresh browser (`Cmd+Shift+R` on Mac)

Expected visual proof of latest UI:

- Header text includes: `UI Refresh v3`

---

## 9) Known environment caveat (cloud CI/agent context)

- `cargo check` passes
- `cargo test` may fail in some environments due to missing system `libpq` linker dependency (`cannot find -lpq`)

---

## 10) Suggested prompt for the next conversation

Use this exact starter prompt:

> Continue from `NEW_CONVERSATION_SUMMARY.md` on branch `cursor/research-data-platform-d3b4`.  
> The app works but the workflow is confusing.  
> Redesign `/ui/app`, `/ui/studies`, and `/ui/dua` into guided, step-by-step wizards with persistent org/study context, minimal UUID exposure, and clear "next action" cards.  
> Keep existing hierarchy + DUA org isolation behavior intact and preserve compile success (`cargo check`).

