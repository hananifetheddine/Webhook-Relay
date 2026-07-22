use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::auth::{self, hash_password, verify_password};
use crate::db;
use crate::error::AppError;
use crate::state::AppState;
use crate::validation::{normalize_email, validate_password};

#[derive(Debug, Deserialize)]
pub(super) struct Credentials {
    email: String,
    password: String,
}

#[derive(Serialize)]
pub(super) struct UserResponse {
    id: Uuid,
    email: String,
}

#[derive(Serialize)]
pub(super) struct LoginResponse {
    token: String,
    token_type: &'static str,
    expires_in: i64,
}

pub(super) async fn register(
    State(state): State<AppState>,
    Json(body): Json<Credentials>,
) -> Result<(StatusCode, Json<UserResponse>), AppError> {
    let email = normalize_email(&body.email)?;
    validate_password(&body.password)?;
    let password_hash = hash_password(&body.password)?;
    let id = Uuid::new_v4();
    db::insert_user(
        &state.pool,
        id,
        &email,
        &password_hash,
        Utc::now().naive_utc(),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(UserResponse { id, email })))
}

pub(super) async fn login(
    State(state): State<AppState>,
    Json(body): Json<Credentials>,
) -> Result<Json<LoginResponse>, AppError> {
    let email = normalize_email(&body.email)?;
    validate_password(&body.password)?;
    let user = db::find_user_by_email(&state.pool, &email).await?;
    let Some(user) = user else {
        auth::reject_unknown_user(&body.password);
        return Err(AppError::unauthorized("invalid email or password"));
    };
    if !verify_password(&body.password, &user.password_hash)? {
        return Err(AppError::unauthorized("invalid email or password"));
    }
    let token = auth::issue_token(&state.config.jwt_secret, user.id, state.config.jwt_ttl_secs)?;
    Ok(Json(LoginResponse {
        token,
        token_type: "Bearer",
        expires_in: state.config.jwt_ttl_secs,
    }))
}
