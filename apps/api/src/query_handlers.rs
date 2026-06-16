use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::{
    auth::AuthenticatedUser,
    authz::{require_org_role, ROLE_ANALYTICS, ROLE_COORDINATOR_OR_BETTER, ROLE_ORG_MANAGERS},
    error::{map_db_error, ApiError},
    routes::AppContext,
};

#[derive(Debug, Deserialize)]
pub(crate) struct CreateStudyDataQueryRequest {
    submission_id: Uuid,
    field_key: String,
    query_text: String,
}

pub(crate) async fn create_study_data_query(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
    Path(project_id): Path<Uuid>,
    Json(payload): Json<CreateStudyDataQueryRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let project = ctx
        .db
        .get_project(project_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("project not found".to_string()))?;
    require_org_role(&user, project.organization_id, ROLE_COORDINATOR_OR_BETTER)?;
    if payload.field_key.trim().is_empty() || payload.query_text.trim().is_empty() {
        return Err(ApiError::Validation(
            "field_key and query_text are required".to_string(),
        ));
    }
    let submission = ctx
        .db
        .get_study_crf_submission(payload.submission_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("submission not found".to_string()))?;
    if submission.project_id != project_id {
        return Err(ApiError::Validation(
            "submission does not belong to project".to_string(),
        ));
    }
    if submission.status.trim().eq_ignore_ascii_case("draft") {
        return Err(ApiError::Conflict(
            "cannot raise data query on a draft submission".to_string(),
        ));
    }
    let query = ctx
        .db
        .create_study_data_query(
            project_id,
            payload.submission_id,
            payload.field_key.trim(),
            payload.query_text.trim(),
            Some(user.user_id),
        )
        .await
        .map_err(map_db_error)?;
    Ok((StatusCode::CREATED, Json(query)))
}

pub(crate) async fn list_study_data_queries(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
    Path(project_id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let project = ctx
        .db
        .get_project(project_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("project not found".to_string()))?;
    require_org_role(&user, project.organization_id, ROLE_ANALYTICS)?;
    let queries = ctx
        .db
        .list_study_data_queries(project_id)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(queries))
}

#[derive(Debug, Deserialize)]
pub(crate) struct RespondStudyDataQueryRequest {
    response_text: String,
}

pub(crate) async fn respond_study_data_query(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
    Path(query_id): Path<Uuid>,
    Json(payload): Json<RespondStudyDataQueryRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let query = ctx
        .db
        .get_study_data_query(query_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("data query not found".to_string()))?;
    let project = ctx
        .db
        .get_project(query.project_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("project not found".to_string()))?;
    require_org_role(&user, project.organization_id, ROLE_COORDINATOR_OR_BETTER)?;
    if payload.response_text.trim().is_empty() {
        return Err(ApiError::Validation(
            "response_text is required".to_string(),
        ));
    }
    let updated = ctx
        .db
        .respond_study_data_query(query_id, payload.response_text.trim())
        .await
        .map_err(map_db_error)?;
    Ok(Json(updated))
}

pub(crate) async fn close_study_data_query(
    State(ctx): State<AppContext>,
    user: AuthenticatedUser,
    Path(query_id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let query = ctx
        .db
        .get_study_data_query(query_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("data query not found".to_string()))?;
    let project = ctx
        .db
        .get_project(query.project_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::NotFound("project not found".to_string()))?;
    require_org_role(&user, project.organization_id, ROLE_ORG_MANAGERS)?;
    let updated = ctx
        .db
        .close_study_data_query(query_id, Some(user.user_id))
        .await
        .map_err(map_db_error)?;
    Ok(Json(updated))
}
