use uuid::Uuid;

use crate::{
    error::ApiError,
    models::{AgreementStatus, QueryStatus, StudyPhase, StudyReadiness, SubmissionStatus},
    state::Store,
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

pub fn compute_readiness(store: &Store, study_id: Uuid) -> Result<StudyReadiness, ApiError> {
    let study = store
        .studies
        .get(&study_id)
        .ok_or_else(|| ApiError::NotFound(format!("study {study_id}")))?;

    let has_site = store
        .sites
        .values()
        .any(|site| site.study_id == Some(study_id) && site.startup_complete);
    let has_published_crf = store
        .crf_templates
        .values()
        .any(|template| template.study_id == study_id && template.published);
    let has_enrolled_patient = store
        .patients
        .values()
        .any(|patient| patient.study_id == study_id);
    let has_locked_submission = store.crf_submissions.values().any(|submission| {
        submission.study_id == study_id && submission.status == SubmissionStatus::Locked
    });
    let open_query_count = store
        .data_queries
        .values()
        .filter(|query| query.study_id == study_id && query.status != QueryStatus::Closed)
        .count();
    let has_active_dua = store.duas.values().any(|agreement| {
        agreement.organization_id == study.organization_id
            && agreement.status == AgreementStatus::Active
    });

    let next_recommended_action = if !has_active_dua {
        "Activate a DUA for the study organization".to_string()
    } else if !has_site {
        "Mark at least one attached site as startup-complete".to_string()
    } else if !has_published_crf {
        "Create and publish at least one CRF template".to_string()
    } else if !has_enrolled_patient {
        "Enroll first patient to unlock active operations".to_string()
    } else if !has_locked_submission {
        "Capture and lock at least one CRF submission".to_string()
    } else if open_query_count > 0 {
        "Resolve all open data queries before closure".to_string()
    } else {
        "Study is ready for operational closeout".to_string()
    };

    Ok(StudyReadiness {
        study_id,
        phase: study.phase.clone(),
        has_site,
        has_published_crf,
        has_enrolled_patient,
        has_locked_submission,
        open_query_count,
        has_active_dua,
        next_recommended_action,
    })
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
            Ok(())
        }
        _ => Err(ApiError::BadRequest(format!(
            "unsupported phase transition from {:?} to {:?}",
            current, target
        ))),
    }
}
