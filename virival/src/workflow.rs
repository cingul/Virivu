use crate::{
    error::ApiError,
    models::{StudyPhase, StudyReadiness},
};

fn phase_rank(phase: &StudyPhase) -> u8 {
    match phase {
        StudyPhase::PreStudy => 0,
        StudyPhase::Initiation => 1,
        StudyPhase::Active => 2,
        StudyPhase::Monitoring => 3,
        StudyPhase::Closed => 4,
    }
}

pub fn validate_phase_transition(
    current: &StudyPhase,
    target: &StudyPhase,
    readiness: &StudyReadiness,
) -> Result<(), ApiError> {
    if phase_rank(target) < phase_rank(current) {
        return Err(ApiError::Conflict(
            "phase regression is not allowed in Virival workflow".to_string(),
        ));
    }

    if phase_rank(target) == phase_rank(current) {
        return Ok(());
    }

    match (current, target) {
        (StudyPhase::PreStudy, StudyPhase::Initiation) => {
            if !readiness.has_active_dua {
                return Err(ApiError::Conflict(
                    "cannot enter initiation without an active DUA".to_string(),
                ));
            }
            if !readiness.has_site || !readiness.has_published_crf {
                return Err(ApiError::Conflict(
                    "cannot enter initiation without startup-complete site and published CRF"
                        .to_string(),
                ));
            }
            Ok(())
        }
        (StudyPhase::Initiation, StudyPhase::Active) => {
            if !readiness.has_enrolled_patient {
                return Err(ApiError::Conflict(
                    "cannot move to active until at least one patient is enrolled".to_string(),
                ));
            }
            Ok(())
        }
        (StudyPhase::Active, StudyPhase::Monitoring) => {
            if !readiness.has_locked_submission {
                return Err(ApiError::Conflict(
                    "cannot move to monitoring before at least one locked CRF submission"
                        .to_string(),
                ));
            }
            Ok(())
        }
        (StudyPhase::Monitoring, StudyPhase::Closed) => {
            if readiness.open_query_count > 0 {
                return Err(ApiError::Conflict(
                    "cannot close study with unresolved data queries".to_string(),
                ));
            }
            if readiness.pending_closeout_items > 0 {
                return Err(ApiError::Conflict(
                    "cannot close study with pending required closeout checklist items".to_string(),
                ));
            }
            Ok(())
        }
        _ => Err(ApiError::BadRequest(format!(
            "unsupported phase transition from {:?} to {:?}",
            current, target
        ))),
    }
}
