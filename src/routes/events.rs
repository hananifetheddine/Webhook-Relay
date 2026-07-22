use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::auth::AuthUser;
use crate::db::{self, StoredAttempt, StoredEvent};
use crate::error::AppError;
use crate::state::AppState;
use crate::validation::validate_event_type;

#[derive(Debug, Deserialize)]
pub(super) struct CreateEventRequest {
    #[serde(rename = "type")]
    event_type: String,
    payload: Value,
}

#[derive(Serialize)]
pub(super) struct DeliverySummary {
    id: Uuid,
    endpoint_id: Uuid,
    status: String,
}

#[derive(Serialize)]
pub(super) struct CreatedEventResponse {
    id: Uuid,
    #[serde(rename = "type")]
    event_type: String,
    received_at: DateTime<Utc>,
    deliveries: Vec<DeliverySummary>,
}

#[derive(Serialize)]
pub(super) struct AttemptResponse {
    id: Uuid,
    endpoint_id: Uuid,
    endpoint_url: String,
    status: String,
    http_status: Option<i32>,
    attempt_count: i32,
    next_attempt_at: Option<DateTime<Utc>>,
    last_attempt_at: Option<DateTime<Utc>>,
}

#[derive(Serialize)]
pub(super) struct EventResponse {
    id: Uuid,
    #[serde(rename = "type")]
    event_type: String,
    payload: Value,
    received_at: DateTime<Utc>,
    attempts: Vec<AttemptResponse>,
}

#[derive(Serialize)]
pub(super) struct ReplayResponse {
    requeued: u64,
    event: EventResponse,
}

pub(super) async fn create_event(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<CreateEventRequest>,
) -> Result<(StatusCode, Json<CreatedEventResponse>), AppError> {
    let event_type = validate_event_type(&body.event_type)?;
    let received_at = Utc::now();
    let event = db::create_event(
        &state.pool,
        auth.user_id,
        &event_type,
        &body.payload,
        received_at,
    )
    .await?;
    Ok((StatusCode::ACCEPTED, Json(created_response(event))))
}

pub(super) async fn get_event(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<Json<EventResponse>, AppError> {
    let event = load_owned(&state, auth.user_id, id).await?;
    Ok(Json(event_response(event)))
}

pub(super) async fn replay_event(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> Result<(StatusCode, Json<ReplayResponse>), AppError> {
    let now = Utc::now();
    let Some(requeued) = db::replay_event(&state.pool, auth.user_id, id, now).await? else {
        return Err(AppError::not_found("event not found"));
    };
    let event = load_owned(&state, auth.user_id, id).await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(ReplayResponse {
            requeued,
            event: event_response(event),
        }),
    ))
}

async fn load_owned(
    state: &AppState,
    user_id: Uuid,
    event_id: Uuid,
) -> Result<StoredEvent, AppError> {
    db::get_event(&state.pool, user_id, event_id)
        .await?
        .ok_or_else(|| AppError::not_found("event not found"))
}

fn created_response(event: StoredEvent) -> CreatedEventResponse {
    CreatedEventResponse {
        id: event.id,
        event_type: event.event_type,
        received_at: event.received_at,
        deliveries: event
            .attempts
            .into_iter()
            .map(|attempt| DeliverySummary {
                id: attempt.id,
                endpoint_id: attempt.endpoint_id,
                status: attempt.status,
            })
            .collect(),
    }
}

fn event_response(event: StoredEvent) -> EventResponse {
    EventResponse {
        id: event.id,
        event_type: event.event_type,
        payload: event.payload,
        received_at: event.received_at,
        attempts: event.attempts.into_iter().map(attempt_response).collect(),
    }
}

fn attempt_response(attempt: StoredAttempt) -> AttemptResponse {
    AttemptResponse {
        id: attempt.id,
        endpoint_id: attempt.endpoint_id,
        endpoint_url: attempt.endpoint_url,
        status: attempt.status,
        http_status: attempt.http_status,
        attempt_count: attempt.attempt_count,
        next_attempt_at: attempt.next_attempt_at,
        last_attempt_at: attempt.last_attempt_at,
    }
}
