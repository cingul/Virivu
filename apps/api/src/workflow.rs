pub const QUERY_STATUS_OPEN: &str = "open";
pub const QUERY_STATUS_RESPONDED: &str = "responded";
pub const QUERY_STATUS_CLOSED: &str = "closed";

pub fn normalize_query_status(status: &str) -> String {
    status.trim().to_ascii_lowercase()
}

pub fn query_is_open(status: &str) -> bool {
    normalize_query_status(status) == QUERY_STATUS_OPEN
}

pub fn query_is_responded(status: &str) -> bool {
    normalize_query_status(status) == QUERY_STATUS_RESPONDED
}

pub fn query_is_closed(status: &str) -> bool {
    normalize_query_status(status) == QUERY_STATUS_CLOSED
}

pub fn query_can_respond(status: &str) -> bool {
    query_is_open(status)
}

pub fn query_can_close(status: &str) -> bool {
    query_is_responded(status)
}

#[cfg(test)]
mod tests {
    use super::{
        normalize_query_status, query_can_close, query_can_respond, query_is_closed, query_is_open,
        query_is_responded,
    };

    #[test]
    fn query_status_helpers_are_case_and_whitespace_tolerant() {
        assert!(query_is_open(" Open "));
        assert!(query_is_responded("RESPONDED"));
        assert!(query_is_closed(" closed "));
        assert_eq!(normalize_query_status("  Open "), "open");
    }

    #[test]
    fn query_transition_guards_match_expected_workflow() {
        assert!(query_can_respond("open"));
        assert!(!query_can_respond("responded"));
        assert!(!query_can_respond("closed"));
        assert!(query_can_close("responded"));
        assert!(!query_can_close("open"));
        assert!(!query_can_close("closed"));
    }
}
