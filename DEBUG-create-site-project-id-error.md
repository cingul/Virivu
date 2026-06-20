# Debug Report: Persistent "project_id must be a valid UUID" Error on Create Site (Sites View)

**Date**: 2026-05-30  
**Branch**: `cursor/research-data-platform-d3b4`  
**Repo**: https://github.com/cingul/Virivu  
**Symptom**: When using the clean dev URL `http://localhost:8080/ui/app?admin_email=arcot@cingulum.org&view=sites`, clicking the "Create New Site" card and submitting the modal produces the error:  
**"Error: project_id must be a valid UUID"**

The actual network request shows a payload containing `project_id=63aaa2ed` (a short 8-character hex string, not a valid UUID) being sent to `/ui/app/create-site`, which the server rejects with 400 JSON.

---

## Executive Summary

The root cause is a combination of:

1. A **global form submit interceptor** in the client-side JavaScript that turns every `<form>` on the dashboard into a `fetch()` call and surfaces server errors via `alert("Error: " + data.error)`.

2. **Context leakage** in the massive server-rendered HTML: the hidden `project_id` field in the Create New Site modal (when the page is rendered for `?view=sites`) was being populated at render time with non-UUID values coming from the `selected_project_id` / `selected_project_value` resolution logic (short hex codes from the old seed data).

3. Extremely heavy reliance on query parameters + inline template variables for multi-tenant context across dozens of forms in one giant HTML blob.

Multiple server-side defensive changes and template fixes were made, but the user's browser frequently retained stale HTML, so the bad value continued to be sent.

---

## Detailed Symptoms (Observed by User)

- Clean URL with only `admin_email` + `view=sites` still produced the error.
- Payload captured in Network tab: `project_id=63aaa2ed&site_name=...`
- Response: `{"error":"project_id must be a valid UUID"}`
- Only GET requests visible in some sessions (due to `preventDefault()` + manual `fetch`).
- Error appeared even after full Firefox restart + new tab.
- Docker container often showed old build marker (`VIRIVU-BUILD-2026-05-28-SITE-FIX-4`).

---

## Key Code Locations

### 1. Global Form Submit Handler (Client-Side)

**File**: `apps/api/src/routes.rs` (inside the giant dashboard HTML, ~lines 4966-5013)

```js
document.querySelectorAll('form').forEach(form => {
    form.addEventListener('submit', async (e) => {
        e.preventDefault();
        // ... datalist handling ...
        const formData = new FormData(form);
        const res = await fetch(form.action, {
            method: form.method || 'POST',
            body: new URLSearchParams(formData)
        });
        if (!res.ok) {
            const data = await res.json();
            alert("Error: " + (data.error || "Validation failed"));
        } else {
            // reload or follow redirect
        }
    });
});
```

This is the mechanism that turns a 400 into the exact user-facing alert the user sees.

### 2. Create Site Form in Sites View

The modal for `?view=sites` had (at various points) a hidden field populated from `create_site_project_id_value` / `safe_project_value` / `selected_project_value`.

After several edits, the template was changed to hard-code:

```html
<input type="hidden" name="project_id" value="" />
```

(with a comment explaining the intent).

### 3. Server Handler (submit_app_create_site)

The handler was made defensive:

```rust
let project_id = if form.project_id.trim().is_empty() {
    None
} else {
    match form.project_id.trim().parse::<Uuid>() {
        Ok(id) => Some(id),
        Err(_) => {
            tracing::warn!(... "non-UUID project_id received...");
            None
        }
    }
};
```

However, the running containers the user tested against were frequently older images that still had the strict path.

### 4. Resolution Logic

`selected_project_id` resolution (with fallbacks to `projects.first()` and short-hex prefix matching) fed values into many hidden fields across the dashboard.

---

## Chronological Attempts (High Level)

1. **Initial defensive parsing** in `submit_app_create_site` (and later in send-invite, media-ticket, patient creation, and attach handlers).

2. **Template hardening** — forcing `value=""` specifically in the sites-view create modal (multiple iterations because the giant `format!` strings made changes fragile).

3. **Safe context variables** — introduction of `safe_project_value` with explicit filtering against the current `projects` list.

4. **Docker hygiene** — dozens of `docker compose build --no-cache api` + `up -d --force-recreate api`, full `down --rmi all` attempts.

5. **Browser cache nuking** — repeated instructions for new tabs, hard refreshes (Cmd+Shift+R), full Firefox restarts.

6. **Logging improvements** — explicit `tracing::warn!` at the top of the create-site handler logging the raw `project_id` received.

7. **Aggressive down + rebuilds** performed via the agent.

At every step the server eventually served a safe form, but the user's observed behavior was dominated by stale browser HTML + the global JS interceptor.

---

## Why This Was So Hard to Kill

- The entire dashboard is one enormous server-rendered HTML string (15k+ line `routes.rs`).
- A single global JS listener applies to every form.
- Context (`project_id`, `organization_id`) is threaded through dozens of hidden fields via ad-hoc template variables.
- Docker image updates on Apple Silicon + aggressive caching made "is the new code running?" a constant question.
- The error surfaced via a client-side `alert()` rather than a normal server error page, making it feel like a backend validation failure even when the root was stale client state.

---

## Recommendations for External Reviewer

1. **Kill or heavily scope the global form submit listener.** It is a major source of magic and fragility.

2. **Stop using giant `format!` strings** for the entire dashboard UI. Consider proper templating (Askama, minijinja, etc.) or extract reusable components.

3. **Introduce a single, strongly-typed `DashboardContext`** struct that carries only sanitized `organization_id` and `Option<project_id>`. Use it everywhere instead of many parallel `selected_*_value` variables.

4. **Make the sites tab create flow deliberately project-agnostic** at the template level (already partially done) and document the intent clearly.

5. **Consider moving more dashboard forms to HTMX or a small frontend** so context is managed in one place on the client instead of re-rendered into every hidden field on every navigation.

6. **Add integration-level tests** (even Playwright + the dev bypass) that exercise the "create org-level site from the sites tab" flow with a clean context.

7. **Audit every place** that calls `parse_uuid_field(..., "project_id")` inside the app dashboard handlers and decide whether it should be defensive or whether the form should simply not render a project_id field at all for that action.

---

## Files Most Relevant to the Bug

- `apps/api/src/routes.rs` (especially the global JS block and the "sites" arm of the view match)
- The `submit_app_create_site` handler and `AppCreateSiteForm`
- Resolution logic for `selected_project_id` / `safe_project_value` (~lines 3036-3114 range in recent versions)

---

**Prepared for external expert review**  
This document attempts to be a neutral, chronological record of symptoms, hypotheses, and attempted fixes rather than a defense of any particular approach.
