use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::models::{StudyCrfSubmission, StudyDataQuery};
use crate::workflow::{
    query_can_close, query_can_respond, query_is_closed, query_is_open, query_is_responded,
};

pub struct QueryMonitorPanelInput<'a> {
    pub data_queries: &'a [StudyDataQuery],
    pub submissions: &'a [StudyCrfSubmission],
    pub queryable_submissions: &'a [&'a StudyCrfSubmission],
    pub selected_project_id: Option<Uuid>,
    pub selected_query_submission_id: Option<Uuid>,
    pub query_age_filter: &'a str,
    pub patient_related_queries_filter: bool,
    pub now: DateTime<Utc>,
    pub admin_email: &'a str,
    pub queries_tab_url: &'a str,
    pub submissions_tab_url: &'a str,
    pub stale_queries_url: &'a str,
    pub aging_queries_url: &'a str,
    pub patient_queries_url: &'a str,
}

pub struct QueryMonitorPanelModel {
    pub query_submission_options_html: String,
    pub selected_query_submission_hex: String,
    pub create_query_disabled: bool,
    pub no_query_submission_hint: String,
    pub query_header: String,
    pub query_filter_controls_html: String,
    pub data_queries_html: String,
}

pub fn build_query_monitor_panel(input: QueryMonitorPanelInput<'_>) -> QueryMonitorPanelModel {
    let query_submission_options_html = if input.queryable_submissions.is_empty() {
        r#"<option value="" data-id="">No submitted/locked submissions available yet</option>"#
            .to_string()
    } else {
        input
            .queryable_submissions
            .iter()
            .map(|submission| {
                format!(
                    r#"<option value="{}" data-id="{}">{}</option>"#,
                    submission.id.to_string().chars().take(8).collect::<String>(),
                    submission.id,
                    escape_html(&format!("{} ({})", submission.id, submission.status))
                )
            })
            .collect::<Vec<_>>()
            .join("")
    };
    let selected_query_submission_hex = input
        .selected_query_submission_id
        .map(|id| id.to_string().chars().take(8).collect::<String>())
        .unwrap_or_default();
    let create_query_disabled =
        input.selected_project_id.is_none() || input.queryable_submissions.is_empty();
    let no_query_submission_hint = if input.selected_project_id.is_none() {
        "<p class=\"muted\" style=\"margin-top:0.5rem;\">Select a study first, then create at least one non-draft submission.</p>"
            .to_string()
    } else if input.queryable_submissions.is_empty() {
        format!(
            "<p class=\"muted\" style=\"margin-top:0.5rem;\">No eligible submissions yet. Go to <a href=\"{}\" style=\"font-weight:700;color:#02182b;\">Submissions tab</a>, submit/lock a CRF, then return here.</p>",
            input.submissions_tab_url
        )
    } else {
        String::new()
    };
    let query_filter_controls_html = format!(
        r#"<div style="margin:0.55rem 0 0.7rem; display:flex; gap:0.45rem; flex-wrap:wrap;">
  <a href="{stale}" style="font-size:0.75rem; background:#fff1f2; color:#b91c1c; padding:3px 7px; border-radius:4px; text-decoration:none;">Stale queries</a>
  <a href="{aging}" style="font-size:0.75rem; background:#fefce8; color:#a16207; padding:3px 7px; border-radius:4px; text-decoration:none;">Aging queries</a>
  <a href="{patient}" style="font-size:0.75rem; background:#eff6ff; color:#1d4ed8; padding:3px 7px; border-radius:4px; text-decoration:none;">Patient-related</a>
  <a href="{clear}" style="font-size:0.75rem; color:#166534; text-decoration:none; padding-top:3px;">Clear filters</a>
</div>"#,
        stale = input.stale_queries_url,
        aging = input.aging_queries_url,
        patient = input.patient_queries_url,
        clear = input.queries_tab_url
    );

    let mut filtered_queries: Vec<_> = input.data_queries.iter().collect();
    if input.query_age_filter == "stale" {
        filtered_queries.retain(|q| (input.now - q.created_at).num_days() > 30);
    } else if input.query_age_filter == "aging" {
        filtered_queries.retain(|q| {
            let age = (input.now - q.created_at).num_days();
            age > 7 && age <= 30
        });
    }
    if input.patient_related_queries_filter {
        filtered_queries.retain(|q| {
            input
                .submissions
                .iter()
                .any(|s| s.id == q.submission_id && s.entered_by_user_id.is_none())
        });
    }

    let query_open_count = input
        .data_queries
        .iter()
        .filter(|q| query_is_open(&q.status))
        .count();
    let query_responded_count = input
        .data_queries
        .iter()
        .filter(|q| query_is_responded(&q.status))
        .count();
    let query_closed_count = input
        .data_queries
        .iter()
        .filter(|q| query_is_closed(&q.status))
        .count();
    let query_header = if input.query_age_filter == "stale" {
        format!(
            r#"Queries <span style="font-size:0.7rem; color:#c53030; background:#fff1f2; padding:1px 5px; border-radius:3px;">showing STALE only</span> <a href="{}" style="font-size:0.65rem; color:#166534; margin-left:6px;">clear filter</a>"#,
            input.queries_tab_url
        )
    } else if input.query_age_filter == "aging" {
        format!(
            r#"Queries <span style="font-size:0.7rem; color:#b7791f; background:#fefce8; padding:1px 5px; border-radius:3px;">showing AGING only</span> <a href="{}" style="font-size:0.65rem; color:#166534; margin-left:6px;">clear filter</a>"#,
            input.queries_tab_url
        )
    } else if input.patient_related_queries_filter {
        format!(
            r#"Queries <span style="font-size:0.7rem; color:#1d4ed8; background:#eff6ff; padding:1px 5px; border-radius:3px;">showing PATIENT-related only</span> <a href="{}" style="font-size:0.65rem; color:#166534; margin-left:6px;">clear filter</a>"#,
            input.queries_tab_url
        )
    } else {
        format!(
            "Queries <span style=\"font-size:0.7rem; color:#64748b;\">({} open · {} responded · {} closed)</span>",
            query_open_count, query_responded_count, query_closed_count
        )
    };

    let data_queries_html = if filtered_queries.is_empty() {
        if input.query_age_filter == "stale" || input.query_age_filter == "aging" {
            format!(
                "<li style=\"color:#64748b;font-size:0.85rem;\">No {} queries match the current filter.</li>",
                input.query_age_filter
            )
        } else if input.patient_related_queries_filter {
            "<li style=\"color:#64748b;font-size:0.85rem;\">No patient-related queries match the current filter.</li>".to_string()
        } else {
            "<li>No monitor queries yet.</li>".to_string()
        }
    } else {
        filtered_queries
            .iter()
            .take(30)
            .map(|q| {
                let age_days = (input.now - q.created_at).num_days();
                let age_badge = if age_days > 30 {
                    format!(r#"<span style="background:#c53030;color:white;padding:1px 3px;border-radius:2px;font-size:0.58rem;font-weight:600;margin-left:3px;" title="Stale query">STALE {}d</span>"#, age_days)
                } else if age_days > 7 {
                    format!(r#"<span style="background:#b7791f;color:white;padding:1px 3px;border-radius:2px;font-size:0.58rem;font-weight:600;margin-left:3px;" title="Aging query">AGING {}d</span>"#, age_days)
                } else {
                    format!(r#"<span style="background:#047857;color:white;padding:1px 3px;border-radius:2px;font-size:0.58rem;font-weight:600;margin-left:3px;" title="Recent query">{}d</span>"#, age_days)
                };
                let response_form = if query_is_closed(&q.status) {
                    "<small>closed</small>".to_string()
                } else if query_can_close(&q.status) {
                    format!(
                        r#"<form method="post" action="/ui/studies/queries/{}/close" style="margin:0.4rem 0;">
  <input type="hidden" name="admin_email" value="{}" />
  <button type="submit">Close query</button>
</form>"#,
                        q.id,
                        escape_html(input.admin_email)
                    )
                } else if query_can_respond(&q.status) {
                    format!(
                        r#"<form method="post" action="/ui/studies/queries/{}/respond" style="margin:0.4rem 0;">
  <input type="hidden" name="admin_email" value="{}" />
  <input name="response_text" placeholder="Response / correction note" required />
  <button type="submit">Respond</button>
</form>
<small style="color:#64748b;">Respond before closure.</small>"#,
                        q.id,
                        escape_html(input.admin_email)
                    )
                } else {
                    "<small>workflow state unavailable</small>".to_string()
                };
                format!(
                    "<li><strong>{}</strong> <small>submission={} field={} status={} {}</small><div>{}</div>{}</li>",
                    escape_html(&q.query_text),
                    q.submission_id,
                    escape_html(&q.field_key),
                    escape_html(&q.status),
                    age_badge,
                    escape_html(&q.response_text),
                    response_form
                )
            })
            .collect::<Vec<_>>()
            .join("")
    };

    QueryMonitorPanelModel {
        query_submission_options_html,
        selected_query_submission_hex,
        create_query_disabled,
        no_query_submission_hint,
        query_header,
        query_filter_controls_html,
        data_queries_html,
    }
}

fn escape_html(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
