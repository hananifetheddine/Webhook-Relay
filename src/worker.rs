use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use futures::future;
use tokio::sync::{Semaphore, watch};
use tokio::time::{MissedTickBehavior, interval};
use uuid::Uuid;

use crate::backoff::next_attempt_at;
use crate::db::{self, DeliveryJob};
use crate::domain::DeliveryStatus;
use crate::error::AppError;
use crate::signing;
use crate::state::AppState;

pub async fn run(state: AppState, mut shutdown: watch::Receiver<bool>) {
    let mut ticker = interval(Duration::from_millis(state.config.worker_poll_interval_ms));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            biased;
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    tracing::info!("delivery worker stopped");
                    break;
                }
            }
            _ = ticker.tick() => {
                if let Err(err) = process_due(&state).await {
                    tracing::error!(error = %err, "delivery worker tick failed");
                }
            }
        }
    }
}

pub async fn process_due(state: &AppState) -> Result<usize, AppError> {
    dispatch(state, None).await
}

pub async fn process_event(state: &AppState, event_id: Uuid) -> Result<usize, AppError> {
    dispatch(state, Some(event_id)).await
}

/// Avance l'échéance d'une tentative. Les tests s'en servent pour ne pas attendre le backoff.
pub async fn make_due(state: &AppState, delivery_id: Uuid) -> Result<(), AppError> {
    let when = (Utc::now() - chrono::Duration::seconds(1)).naive_utc();
    db::set_next_attempt_at(&state.pool, delivery_id, when).await
}

async fn dispatch(state: &AppState, only_event: Option<Uuid>) -> Result<usize, AppError> {
    let ids = db::due_delivery_ids(
        &state.pool,
        Utc::now().naive_utc(),
        state.config.worker_batch_size,
        only_event,
    )
    .await?;
    if ids.is_empty() {
        return Ok(0);
    }

    let semaphore = Arc::new(Semaphore::new(state.config.worker_concurrency));
    let mut tasks = Vec::with_capacity(ids.len());
    for id in ids {
        let permit = semaphore
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| AppError::internal("worker semaphore closed"))?;
        let state = state.clone();
        tasks.push(tokio::spawn(async move {
            let _permit = permit;
            match deliver_one(&state, id).await {
                Ok(sent) => sent,
                Err(err) => {
                    tracing::error!(delivery_id = %id, error = %err, "delivery failed unexpectedly");
                    false
                }
            }
        }));
    }

    let results = future::join_all(tasks).await;
    Ok(results
        .into_iter()
        .filter(|result| matches!(result, Ok(true)))
        .count())
}

async fn deliver_one(state: &AppState, id: Uuid) -> Result<bool, AppError> {
    let now = Utc::now();
    let lease_until = now + chrono::Duration::seconds(state.config.claim_lease_secs);
    let claimed =
        db::claim_delivery(&state.pool, id, lease_until.naive_utc(), now.naive_utc()).await?;
    if !claimed {
        return Ok(false);
    }

    let Some(job) = db::load_delivery_job(&state.pool, id).await? else {
        tracing::warn!(delivery_id = %id, "claimed delivery could not be loaded");
        return Ok(false);
    };

    let body = serde_json::to_vec(&job.payload).map_err(AppError::internal)?;
    let timestamp = now.timestamp();
    let signature =
        signing::signature_header(&job.secret, timestamp, &body).map_err(AppError::internal)?;

    let outcome = state
        .http
        .post(&job.endpoint_url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header("x-webhook-id", job.event_id.to_string())
        .header("x-webhook-event", &job.event_type)
        .header("x-webhook-timestamp", timestamp.to_string())
        .header("x-webhook-signature", &signature)
        .body(body)
        .send()
        .await;

    let attempted_at = Utc::now().naive_utc();
    let new_count = job.attempt_count.max(0).saturating_add(1);

    match outcome {
        Ok(response) if response.status().is_success() => {
            let http_status = i32::from(response.status().as_u16());
            db::mark_delivery_success(&state.pool, id, http_status, new_count, attempted_at)
                .await?;
            tracing::info!(
                delivery_id = %id,
                event_id = %job.event_id,
                http_status,
                "delivery succeeded"
            );
        }
        Ok(response) => {
            let http_status = i32::from(response.status().as_u16());
            tracing::warn!(delivery_id = %id, http_status, "delivery returned a non-2xx status");
            record_failure(state, &job, id, new_count, attempted_at, Some(http_status)).await?;
        }
        Err(err) => {
            tracing::warn!(delivery_id = %id, error = %err, "delivery request failed");
            record_failure(state, &job, id, new_count, attempted_at, None).await?;
        }
    }

    Ok(true)
}

async fn record_failure(
    state: &AppState,
    job: &DeliveryJob,
    id: Uuid,
    attempt_count: i32,
    attempted_at: chrono::NaiveDateTime,
    http_status: Option<i32>,
) -> Result<(), AppError> {
    let count =
        u32::try_from(attempt_count).map_err(|_| AppError::internal("attempt count overflow"))?;
    let now = DateTime::from_naive_utc_and_offset(attempted_at, Utc);
    let (status, next) = match next_attempt_at(now, count, state.config.max_delivery_attempts) {
        Some(next) => (DeliveryStatus::Failed, Some(next.naive_utc())),
        None => (DeliveryStatus::Exhausted, None),
    };
    db::mark_delivery_failure(
        &state.pool,
        id,
        status,
        http_status,
        attempt_count,
        attempted_at,
        next,
    )
    .await?;
    tracing::info!(
        delivery_id = %id,
        event_id = %job.event_id,
        status = status.as_str(),
        attempt_count,
        "delivery attempt recorded"
    );
    Ok(())
}
