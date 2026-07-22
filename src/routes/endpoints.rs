use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::auth::AuthUser;
use crate::db::{self, EndpointRecord};
use crate::error::AppError;
use crate::signing;
use crate::state::AppState;
use crate::validation::{normalize_event_types, validate_url};

#[derive(Debug, Deserialize)]
pub(super) struct CreateEndpointRequest {
    url: String,
    event_types: Vec<String>,
}

#[derive(Serialize)]
pub(super) struct EndpointResponse {
    id: Uuid,
    url: String,
    event_types: Vec<String>,
    active: bool,
    created_at: DateTime<Utc>,
}

#[derive(Serialize)]
pub(super) struct CreatedEndpointResponse {
    id: Uuid,
    url: String,
    event_types: Vec<String>,
    secret: String,
    active: bool,
    created_at: DateTime<Utc>,
}

pub(super) async fn create_endpoint(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<CreateEndpointRequest>,
) -> Result<(StatusCode, Json<CreatedEndpointResponse>), AppError> {
    let url = validate_url(&body.url)?;
    let event_types = normalize_event_types(body.event_types)?;
    let id = Uuid::new_v4();
    let secret = signing::generate_secret();
    let created_at = Utc::now();
    db::insert_endpoint(
        &state.pool,
        id,
        auth.user_id,
        &url,
        &event_types,
        &secret,
        created_at.naive_utc(),
    )
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(CreatedEndpointResponse {
            id,
            url,
            event_types,
            secret,
            active: true,
            created_at,
        }),
    ))
}

pub(super) async fn list_endpoints(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Vec<EndpointResponse>>, AppError> {
    let endpoints = db::list_endpoints(&state.pool, auth.user_id).await?;
    Ok(Json(endpoints.into_iter().map(to_response).collect()))
}

pub(super) async fn delete_endpoint(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let deleted = db::deactivate_endpoint(&state.pool, auth.user_id, id).await?;
    if !deleted {
        return Err(AppError::not_found("endpoint not found"));
    }
    Ok(StatusCode::NO_CONTENT)
}

fn to_response(endpoint: EndpointRecord) -> EndpointResponse {
    EndpointResponse {
        id: endpoint.id,
        url: endpoint.url,
        event_types: endpoint.event_types,
        active: true,
        created_at: endpoint.created_at,
    }
}
