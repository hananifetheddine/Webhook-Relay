use chrono::{DateTime, NaiveDateTime, Utc};
use serde_json::Value;
use sqlx::MySqlPool;
use uuid::Uuid;

use crate::domain::{DeliveryStatus, endpoint_accepts};
use crate::error::AppError;

#[derive(Debug, Clone)]
pub struct UserRecord {
    pub id: Uuid,
    pub password_hash: String,
}

#[derive(Debug, Clone)]
pub struct EndpointRecord {
    pub id: Uuid,
    pub url: String,
    pub event_types: Vec<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct ActiveEndpoint {
    pub id: Uuid,
    pub url: String,
    pub secret: String,
    pub event_types: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct StoredAttempt {
    pub id: Uuid,
    pub endpoint_id: Uuid,
    pub endpoint_url: String,
    pub status: String,
    pub http_status: Option<i32>,
    pub attempt_count: i32,
    pub next_attempt_at: Option<DateTime<Utc>>,
    pub last_attempt_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct StoredEvent {
    pub id: Uuid,
    pub event_type: String,
    pub payload: Value,
    pub received_at: DateTime<Utc>,
    pub attempts: Vec<StoredAttempt>,
}

#[derive(Debug, Clone)]
pub struct DeliveryJob {
    pub id: Uuid,
    pub event_id: Uuid,
    pub event_type: String,
    pub payload: Value,
    pub endpoint_url: String,
    pub secret: String,
    pub attempt_count: i32,
}

pub async fn insert_user(
    pool: &MySqlPool,
    id: Uuid,
    email: &str,
    password_hash: &str,
    created_at: NaiveDateTime,
) -> Result<(), AppError> {
    let id = id.to_string();
    sqlx::query!(
        r#"
        INSERT INTO users (id, email, password_hash, created_at)
        VALUES (?, ?, ?, ?)
        "#,
        id,
        email,
        password_hash,
        created_at,
    )
    .execute(pool)
    .await
    .map_err(|err| AppError::from_sqlx(err, "email already registered"))?;
    Ok(())
}

pub async fn find_user_by_email(
    pool: &MySqlPool,
    email: &str,
) -> Result<Option<UserRecord>, AppError> {
    let row = sqlx::query!(
        r#"
        SELECT id, password_hash
        FROM users
        WHERE email = ?
        "#,
        email,
    )
    .fetch_optional(pool)
    .await
    .map_err(AppError::internal)?;

    row.map(|row| {
        Ok(UserRecord {
            id: parse_uuid(&row.id)?,
            password_hash: row.password_hash,
        })
    })
    .transpose()
}

pub async fn insert_endpoint(
    pool: &MySqlPool,
    id: Uuid,
    user_id: Uuid,
    url: &str,
    event_types: &[String],
    secret: &str,
    created_at: NaiveDateTime,
) -> Result<(), AppError> {
    let id = id.to_string();
    let user_id = user_id.to_string();
    let event_types = serde_json::to_string(event_types).map_err(AppError::internal)?;
    sqlx::query!(
        r#"
        INSERT INTO endpoints (id, user_id, url, event_types, secret, active, created_at)
        VALUES (?, ?, ?, ?, ?, 1, ?)
        "#,
        id,
        user_id,
        url,
        event_types,
        secret,
        created_at,
    )
    .execute(pool)
    .await
    .map_err(AppError::internal)?;
    Ok(())
}

pub async fn list_endpoints(
    pool: &MySqlPool,
    user_id: Uuid,
) -> Result<Vec<EndpointRecord>, AppError> {
    let user_id = user_id.to_string();
    let rows = sqlx::query!(
        r#"
        SELECT id, url, event_types, created_at
        FROM endpoints
        WHERE user_id = ? AND active = 1
        ORDER BY created_at
        "#,
        user_id,
    )
    .fetch_all(pool)
    .await
    .map_err(AppError::internal)?;

    rows.into_iter()
        .map(|row| {
            Ok(EndpointRecord {
                id: parse_uuid(&row.id)?,
                url: row.url,
                event_types: parse_string_list(row.event_types)?,
                created_at: to_utc(row.created_at),
            })
        })
        .collect()
}

pub async fn deactivate_endpoint(
    pool: &MySqlPool,
    user_id: Uuid,
    endpoint_id: Uuid,
) -> Result<bool, AppError> {
    let user_id = user_id.to_string();
    let endpoint_id = endpoint_id.to_string();
    let result = sqlx::query!(
        r#"
        UPDATE endpoints
        SET active = 0
        WHERE id = ? AND user_id = ? AND active = 1
        "#,
        endpoint_id,
        user_id,
    )
    .execute(pool)
    .await
    .map_err(AppError::internal)?;
    Ok(result.rows_affected() == 1)
}

pub async fn list_active_endpoints(
    pool: &MySqlPool,
    user_id: Uuid,
) -> Result<Vec<ActiveEndpoint>, AppError> {
    let user_id = user_id.to_string();
    let rows = sqlx::query!(
        r#"
        SELECT id, url, secret, event_types
        FROM endpoints
        WHERE user_id = ? AND active = 1
        "#,
        user_id,
    )
    .fetch_all(pool)
    .await
    .map_err(AppError::internal)?;

    rows.into_iter()
        .map(|row| {
            Ok(ActiveEndpoint {
                id: parse_uuid(&row.id)?,
                url: row.url,
                secret: row.secret,
                event_types: parse_string_list(row.event_types)?,
            })
        })
        .collect()
}

pub async fn create_event(
    pool: &MySqlPool,
    user_id: Uuid,
    event_type: &str,
    payload: &Value,
    received_at: DateTime<Utc>,
) -> Result<StoredEvent, AppError> {
    let endpoints = list_active_endpoints(pool, user_id).await?;
    let matched: Vec<&ActiveEndpoint> = endpoints
        .iter()
        .filter(|endpoint| endpoint_accepts(&endpoint.event_types, event_type))
        .collect();

    let event_id = Uuid::new_v4();
    let payload_json = serde_json::to_string(payload).map_err(AppError::internal)?;
    let received_naive = received_at.naive_utc();
    let mut tx = pool.begin().await.map_err(AppError::internal)?;

    let event_id_str = event_id.to_string();
    let user_id_str = user_id.to_string();
    sqlx::query!(
        r#"
        INSERT INTO events (id, user_id, event_type, payload, received_at)
        VALUES (?, ?, ?, ?, ?)
        "#,
        event_id_str,
        user_id_str,
        event_type,
        payload_json,
        received_naive,
    )
    .execute(&mut *tx)
    .await
    .map_err(AppError::internal)?;

    let mut attempts = Vec::with_capacity(matched.len());
    for endpoint in matched {
        let attempt_id = Uuid::new_v4();
        insert_delivery(
            &mut *tx,
            NewDelivery {
                id: attempt_id,
                event_id,
                endpoint_id: endpoint.id,
                endpoint_url: &endpoint.url,
                status: DeliveryStatus::Pending,
                attempt_count: 0,
                next_attempt_at: Some(received_naive),
                created_at: received_naive,
            },
        )
        .await?;
        attempts.push(StoredAttempt {
            id: attempt_id,
            endpoint_id: endpoint.id,
            endpoint_url: endpoint.url.clone(),
            status: DeliveryStatus::Pending.as_str().to_string(),
            http_status: None,
            attempt_count: 0,
            next_attempt_at: Some(received_at),
            last_attempt_at: None,
        });
    }

    tx.commit().await.map_err(AppError::internal)?;

    Ok(StoredEvent {
        id: event_id,
        event_type: event_type.to_string(),
        payload: payload.clone(),
        received_at,
        attempts,
    })
}

pub async fn get_event(
    pool: &MySqlPool,
    user_id: Uuid,
    event_id: Uuid,
) -> Result<Option<StoredEvent>, AppError> {
    let user_id = user_id.to_string();
    let event_id_str = event_id.to_string();
    let row = sqlx::query!(
        r#"
        SELECT id, event_type, payload, received_at
        FROM events
        WHERE id = ? AND user_id = ?
        "#,
        event_id_str,
        user_id,
    )
    .fetch_optional(pool)
    .await
    .map_err(AppError::internal)?;

    let Some(row) = row else {
        return Ok(None);
    };

    let attempts = list_attempts(pool, event_id).await?;
    Ok(Some(StoredEvent {
        id: parse_uuid(&row.id)?,
        event_type: row.event_type,
        payload: row.payload,
        received_at: to_utc(row.received_at),
        attempts,
    }))
}

pub async fn replay_event(
    pool: &MySqlPool,
    user_id: Uuid,
    event_id: Uuid,
    now: DateTime<Utc>,
) -> Result<Option<u64>, AppError> {
    let Some(existing) = get_event(pool, user_id, event_id).await? else {
        return Ok(None);
    };
    let event_type = existing.event_type;
    let endpoints = list_active_endpoints(pool, user_id).await?;
    let now_naive = now.naive_utc();
    let mut requeued = 0u64;

    for endpoint in endpoints
        .into_iter()
        .filter(|endpoint| endpoint_accepts(&endpoint.event_types, &event_type))
    {
        let updated = sqlx::query!(
            r#"
            UPDATE delivery_attempts
            SET status = 'pending',
                attempt_count = 0,
                http_status = NULL,
                next_attempt_at = ?,
                last_attempt_at = NULL,
                endpoint_url = ?
            WHERE event_id = ? AND endpoint_id = ?
            "#,
            now_naive,
            endpoint.url,
            event_id.to_string(),
            endpoint.id.to_string(),
        )
        .execute(pool)
        .await
        .map_err(AppError::internal)?;

        if updated.rows_affected() > 0 {
            requeued += updated.rows_affected();
            continue;
        }

        let inserted = insert_delivery(
            pool,
            NewDelivery {
                id: Uuid::new_v4(),
                event_id,
                endpoint_id: endpoint.id,
                endpoint_url: &endpoint.url,
                status: DeliveryStatus::Pending,
                attempt_count: 0,
                next_attempt_at: Some(now_naive),
                created_at: now_naive,
            },
        )
        .await;
        match inserted {
            Ok(()) => requeued += 1,
            Err(AppError::Conflict(_)) => requeued += 1,
            Err(err) => return Err(err),
        }
    }

    Ok(Some(requeued))
}

pub async fn due_delivery_ids(
    pool: &MySqlPool,
    now: NaiveDateTime,
    limit: i64,
    only_event: Option<Uuid>,
) -> Result<Vec<Uuid>, AppError> {
    let ids = if let Some(event_id) = only_event {
        let event_id = event_id.to_string();
        sqlx::query!(
            r#"
            SELECT id
            FROM delivery_attempts
            WHERE status IN ('pending', 'failed')
              AND next_attempt_at <= ?
              AND event_id = ?
            ORDER BY next_attempt_at
            LIMIT ?
            "#,
            now,
            event_id,
            limit,
        )
        .fetch_all(pool)
        .await
        .map_err(AppError::internal)?
        .into_iter()
        .map(|row| parse_uuid(&row.id))
        .collect::<Result<Vec<_>, _>>()?
    } else {
        sqlx::query!(
            r#"
            SELECT id
            FROM delivery_attempts
            WHERE status IN ('pending', 'failed')
              AND next_attempt_at <= ?
            ORDER BY next_attempt_at
            LIMIT ?
            "#,
            now,
            limit,
        )
        .fetch_all(pool)
        .await
        .map_err(AppError::internal)?
        .into_iter()
        .map(|row| parse_uuid(&row.id))
        .collect::<Result<Vec<_>, _>>()?
    };
    Ok(ids)
}

pub async fn claim_delivery(
    pool: &MySqlPool,
    id: Uuid,
    lease_until: NaiveDateTime,
    now: NaiveDateTime,
) -> Result<bool, AppError> {
    let id = id.to_string();
    let result = sqlx::query!(
        r#"
        UPDATE delivery_attempts
        SET next_attempt_at = ?
        WHERE id = ?
          AND status IN ('pending', 'failed')
          AND next_attempt_at <= ?
        "#,
        lease_until,
        id,
        now,
    )
    .execute(pool)
    .await
    .map_err(AppError::internal)?;
    Ok(result.rows_affected() == 1)
}

pub async fn load_delivery_job(
    pool: &MySqlPool,
    id: Uuid,
) -> Result<Option<DeliveryJob>, AppError> {
    let id_str = id.to_string();
    let row = sqlx::query!(
        r#"
        SELECT
            d.event_id,
            d.attempt_count,
            d.endpoint_url,
            e.secret,
            ev.event_type,
            ev.payload
        FROM delivery_attempts d
        INNER JOIN endpoints e ON e.id = d.endpoint_id
        INNER JOIN events ev ON ev.id = d.event_id
        WHERE d.id = ?
        "#,
        id_str,
    )
    .fetch_optional(pool)
    .await
    .map_err(AppError::internal)?;

    let Some(row) = row else {
        return Ok(None);
    };

    Ok(Some(DeliveryJob {
        id,
        event_id: parse_uuid(&row.event_id)?,
        event_type: row.event_type,
        payload: row.payload,
        endpoint_url: row.endpoint_url,
        secret: row.secret,
        attempt_count: row.attempt_count,
    }))
}

pub async fn mark_delivery_success(
    pool: &MySqlPool,
    id: Uuid,
    http_status: i32,
    attempt_count: i32,
    attempted_at: NaiveDateTime,
) -> Result<(), AppError> {
    let id = id.to_string();
    sqlx::query!(
        r#"
        UPDATE delivery_attempts
        SET status = 'success',
            http_status = ?,
            attempt_count = ?,
            last_attempt_at = ?,
            next_attempt_at = NULL
        WHERE id = ?
        "#,
        http_status,
        attempt_count,
        attempted_at,
        id,
    )
    .execute(pool)
    .await
    .map_err(AppError::internal)?;
    Ok(())
}

pub async fn mark_delivery_failure(
    pool: &MySqlPool,
    id: Uuid,
    status: DeliveryStatus,
    http_status: Option<i32>,
    attempt_count: i32,
    attempted_at: NaiveDateTime,
    next_attempt_at: Option<NaiveDateTime>,
) -> Result<(), AppError> {
    let id = id.to_string();
    let status = status.as_str();
    sqlx::query!(
        r#"
        UPDATE delivery_attempts
        SET status = ?,
            http_status = ?,
            attempt_count = ?,
            last_attempt_at = ?,
            next_attempt_at = ?
        WHERE id = ?
        "#,
        status,
        http_status,
        attempt_count,
        attempted_at,
        next_attempt_at,
        id,
    )
    .execute(pool)
    .await
    .map_err(AppError::internal)?;
    Ok(())
}

pub async fn set_next_attempt_at(
    pool: &MySqlPool,
    id: Uuid,
    when: NaiveDateTime,
) -> Result<(), AppError> {
    let id = id.to_string();
    sqlx::query!(
        r#"
        UPDATE delivery_attempts
        SET next_attempt_at = ?
        WHERE id = ?
        "#,
        when,
        id,
    )
    .execute(pool)
    .await
    .map_err(AppError::internal)?;
    Ok(())
}

async fn list_attempts(pool: &MySqlPool, event_id: Uuid) -> Result<Vec<StoredAttempt>, AppError> {
    let event_id = event_id.to_string();
    let rows = sqlx::query!(
        r#"
        SELECT id, endpoint_id, endpoint_url, status, http_status, attempt_count, next_attempt_at, last_attempt_at
        FROM delivery_attempts
        WHERE event_id = ?
        ORDER BY created_at, id
        "#,
        event_id,
    )
    .fetch_all(pool)
    .await
    .map_err(AppError::internal)?;

    rows.into_iter()
        .map(|row| {
            Ok(StoredAttempt {
                id: parse_uuid(&row.id)?,
                endpoint_id: parse_uuid(&row.endpoint_id)?,
                endpoint_url: row.endpoint_url,
                status: row.status,
                http_status: row.http_status,
                attempt_count: row.attempt_count,
                next_attempt_at: row.next_attempt_at.map(to_utc),
                last_attempt_at: row.last_attempt_at.map(to_utc),
            })
        })
        .collect()
}

struct NewDelivery<'a> {
    id: Uuid,
    event_id: Uuid,
    endpoint_id: Uuid,
    endpoint_url: &'a str,
    status: DeliveryStatus,
    attempt_count: i32,
    next_attempt_at: Option<NaiveDateTime>,
    created_at: NaiveDateTime,
}

async fn insert_delivery(
    executor: impl sqlx::MySqlExecutor<'_>,
    delivery: NewDelivery<'_>,
) -> Result<(), AppError> {
    let id = delivery.id.to_string();
    let event_id = delivery.event_id.to_string();
    let endpoint_id = delivery.endpoint_id.to_string();
    let endpoint_url = delivery.endpoint_url;
    let status = delivery.status.as_str();
    let attempt_count = delivery.attempt_count;
    let next_attempt_at = delivery.next_attempt_at;
    let created_at = delivery.created_at;
    sqlx::query!(
        r#"
        INSERT INTO delivery_attempts (
            id, event_id, endpoint_id, endpoint_url, status, attempt_count, next_attempt_at, created_at
        )
        VALUES (?, ?, ?, ?, ?, ?, ?, ?)
        "#,
        id,
        event_id,
        endpoint_id,
        endpoint_url,
        status,
        attempt_count,
        next_attempt_at,
        created_at,
    )
    .execute(executor)
    .await
    .map_err(|err| AppError::from_sqlx(err, "delivery already exists"))?;
    Ok(())
}

fn parse_uuid(value: &str) -> Result<Uuid, AppError> {
    Uuid::parse_str(value)
        .map_err(|_| AppError::internal(format!("invalid uuid in database: {value}")))
}

fn parse_string_list(value: Value) -> Result<Vec<String>, AppError> {
    serde_json::from_value(value).map_err(AppError::internal)
}

fn to_utc(value: NaiveDateTime) -> DateTime<Utc> {
    DateTime::from_naive_utc_and_offset(value, Utc)
}
