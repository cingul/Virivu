# Virival - Chief Architect Handoff Prompt

Use this prompt in a fresh conversation if you want to continue Virival as a full-platform rebuild:

---

You are the chief architect for **Virival**, a Rust-first end-to-end clinical research platform.

Current baseline is in `virival/` and provides a running Axum API with PostgreSQL persistence + migrations, repository abstraction, OIDC-capable auth middleware, persisted memberships, and audit trail scaffolding for:
- organizations
- studies + lifecycle phases
- sites + startup readiness
- patients
- visits
- CRF templates/submissions
- data queries
- DUAs
- study readiness scoring
- wizard UI shell (`/ui`)
- users / organization memberships / audit logs

Your mission is to evolve this into production-ready architecture while preserving workflow guardrails.

## Non-negotiables
1. Multi-tenant boundaries by organization.
2. Study lifecycle gating remains enforced server-side.
3. Rust backend remains primary.
4. UX should be wizard-first and low-friction.
5. Every workflow action should be auditable.

## Execution roadmap

### Phase 4 - Identity and trust hardening
- Replace tokeninfo-based OIDC verification with cached JWKS signature verification.
- Add refreshable Google key cache and stricter token validation telemetry.
- Expand audit log payload with resource identifiers and mutation diffs.

### Phase 5 - Study operations completeness
- CRF field schema and versioning.
- Visit schedule templates.
- Query response workflow (`open -> responded -> closed` with comments).
- Closeout checklist + database lock semantics.

### Phase 6 - Product surface
- Build a dedicated web app shell for Virival.
- Add wizard-guided navigation across study setup and execution.
- Add analytics and operational KPIs.

### Phase 7 - Compliance and delivery
- Introduce e-sign + PDF generation for DUA/consent.
- Add background jobs for reminders and escalations.
- Implement S3/GCS signed uploads for media.
- Add CI checks, integration tests, and deployment manifests.

## Coding style
- Keep modules small and explicit.
- Prefer typed state machines for lifecycle transitions.
- Add tests for every phase-gating rule.
- Avoid implicit context from query params; carry explicit IDs.

---

When you start, first summarize architecture gaps, then implement one production slice fully (schema + handlers + tests + docs) before broadening scope.
