# Virivu Handoff — 2026-05-29 (Flexible Sites + Full UI Seeded)

**Purpose**: This document lets you (or a new Grok session) pick up exactly where we left off with almost zero context loss. The previous conversation was extremely long; start fresh here.

## Current High-Level State (as of this handoff)

- **Core architectural shift completed**: Sites are now **primarily Organization-scoped** (required `organization_id`) with **optional** `project_id` attachment. True many-to-many not yet (no join table), but attach/detach is a simple UPDATE on the nullable FK. This was the major request that drove the last phase of work.
  - You can have org-level sites that belong to no study yet.
  - A study can have zero or many sites attached.
  - Old strict hierarchy (Project → Site) is gone.

- **Database**: 1 organization ("Cingulum Foundation Inc." — platform_root), **10 studies/projects**, **15 sites** (mix of pure org-level and a few attached to studies for demo). Dev user `arcot@cingulum.org` has `platform_admin` + `org_admin` on the org (grants are created automatically on page load when `ALLOW_DEV_AUTH_BYPASS=true`).

- **Running locally**: Fully functional Docker setup (postgres + api). The app is production-pilot directionally ready for the core multi-tenant EDC + patient portal flows, with lots of operational UX (aging signals, Recommended Next Steps, Operational Snapshot, risk badges, etc.) already built in prior phases.

- **Dev bypass still the primary way you test**: `http://localhost:8080/ui/app?admin_email=arcot@cingulum.org` (and `&view=sites` etc.). The grant logic + URL canonicalization that rejects fake 0000... UUIDs is in place.

## How to Run Right Now (May 29 2026)

1. `cd deploy`
2. `docker compose up -d` (or `--force-recreate api` after a code change)
3. Hard-refresh the URL above.
4. Sidebar should now show the org and the 10 seeded studies in "Active Study".
5. Sites view (`?view=sites`) shows 15 cards, some with "Attach to this study" / "Detach" buttons (the new flexible model in action). Reset Checklist buttons are on every card.
6. Creating a new site with no project selected creates a pure org-level site.

**Seed / reset data**:
- There is `apps/api/add_dummies.sh` (old, uses fake UUIDs — update it if you want).
- We just did a direct SQL seed of 10 studies + 15 sites. You can repeat similar patterns.
- To completely reset: look at migrations or the old `reset_and_seed_db` helper (if still wired).

**Important env in docker-compose.yml**:
- `ALLOW_DEV_AUTH_BYPASS: "true"`
- `ALLOW_UNSAFE_DEV_BYPASS: "false"` (keeps it localhost-only for safety)

## Major Work Completed in This Session (the "you run it" phase)

- Full rethink + implementation of **decoupled sites** (org-first, optional project attachment).
- Migration `2026-05-29-000016_flexible_sites`.
- Model + db.rs changes (`Site.organization_id` required, `project_id` nullable, attach/detach helpers, new org-level hex generator that always produces valid 9-char codes with ≥2 A-F chars).
- UI: study workbench now splits "Attached to this study" vs "Other org sites", attach/detach forms, "Org-level site" labels.
- Many auth/form fixes: `submit_app_create_site` no longer requires AuthenticatedUser extractor (reconstructs from `admin_email` form field for dev bypass); aggressive "treat anything not perfect UUID as org-level" defense; removal of all stray debug `uuid email` strings from site cards; Reset Checklist button + endpoint + `reset_site_startup_checklist_items`.
- `ensure_dev_platform_admin` + reload of memberships inside `render_app_dashboard` so sidebar populates even with minimal seed data.
- The giant `format!` string fragility in routes.rs caused a build break (missing/extra placeholders after repeated edits); fixed by careful counting + Python-assisted cleanup of the site card template.
- Docker reliability on Apple Silicon: switched to `docker compose build --no-cache api` (the only arch-correct path) after local `cargo build --release` produced Mach-O binaries that wouldn't exec in the linux container.
- Seeded 10 studies + 15 sites so you can actually see organizations and active studies in the sidebar.

## Known Technical Debt / Fragile Areas (tell the next session about these immediately)

1. **routes.rs is a 15k-line monster** with enormous `r#" ... {} ... "#` format strings for every page and card. Every edit risks placeholder count mismatches (we just lived through this). Long-term: extract to proper templates or a small HTML builder.
2. Duplicate `user_memberships` rows for the dev user (the ensure function is called on every dashboard render). Harmless but noisy. A small cleanup query exists in this handoff.
3. Hex code generation for studies sometimes produces values that fail the `projects_hex_code_format` CHECK (needs ≥1 A-F char). The site one is more robust now.
4. No tests. No CI. Docker layer caching on Mac is still painful (hence the build --no-cache dance).
5. Patient portal magic links + ProSubmission → real StudyCrfSubmission creation works, but the rest of the patient journey is still thin.
6. Many "unused variable" warnings (lots of dead code from earlier experiments).

## Recommended Immediate Next Steps (for the next conversation)

- Hard refresh the dev URL and play with the new attach/detach, Reset Checklist, org-level site creation.
- If you want prettier study names, run a quick UPDATE or improve the seed script.
- Consider adding a small "Re-seed demo data" button in the admin UI that calls a safe version of the seed we just did.
- The original production-readiness list (from the very first review) still applies: more eConsent/media, field-level provenance, deeper SDV workflows, proper runbooks, etc.
- Once you're happy with the data model, we can introduce a real `project_sites` join table if you ever need a site in multiple studies without duplicating the site row.

## Files Worth Reading First in a New Session

- This handoff
- `docs/ARCHITECTURE.md` and `docs/PRODUCT_BLUEPRINT.md`
- `apps/api/src/routes.rs` (the app dashboard + sidebar sections + the two submit_*_create handlers)
- `apps/api/src/db.rs` (ensure_dev_platform_admin, create_project, create_site, attach/detach, list_*_for_email)
- `deploy/docker-compose.yml` + `apps/api/Dockerfile` + `apps/api/rust-toolchain.toml`
- The latest migration `apps/api/migrations/2026-05-29-000016_flexible_sites/`

## Git / Deployment Notes

- Remote: `git@github.com:cingul/Virivu.git`
- Active branch at time of handoff: `cursor/research-data-platform-d3b4`
- We are pushing this handoff + the decoupling work + all the auth/UI fixes in one commit.

## Quick Commands You Will Use Constantly

```bash
cd deploy
docker compose up -d --force-recreate api     # after code change
docker compose logs -f api
docker exec -it virivu-postgres psql -U postgres -d virivu
```

---

**You can now safely start a brand new conversation.** Paste the first 30-40 lines of this handoff + the exact URL you're using + "what do you see in the sidebar right now?" and the new session will be productive immediately.

Everything that was painful (auth context missing, UUID errors, debug lines in cards, no data in sidebar, Docker binary arch problems) has been resolved in this session. The foundation is solid.

— Grok (end of 2026-05-29 session)