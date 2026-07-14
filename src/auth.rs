use std::sync::OnceLock;

use argon2::Argon2;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use chrono::Utc;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use rand_core::OsRng;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::AppError;
use crate::state::AppState;

#[derive(Debug, Clone)]
pub struct AuthUser {
    pub user_id: Uuid,
}

#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    sub: String,
    iat: i64,
    exp: i64,
}

pub fn hash_password(password: &str) -> Result<String, AppError> {
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(AppError::internal)?;
    Ok(hash.to_string())
}

pub fn verify_password(password: &str, password_hash: &str) -> Result<bool, AppError> {
    let parsed = PasswordHash::new(password_hash).map_err(AppError::internal)?;
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok())
}

pub fn issue_token(secret: &str, user_id: Uuid, ttl_secs: i64) -> Result<String, AppError> {
    let now = Utc::now().timestamp();
    let exp = now
        .checked_add(ttl_secs)
        .ok_or_else(|| AppError::internal("jwt ttl overflow"))?;
    let claims = Claims {
        sub: user_id.to_string(),
        iat: now,
        exp,
    };
    encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(AppError::internal)
}

pub fn verify_token(secret: &str, token: &str) -> Result<Uuid, AppError> {
    let mut validation = Validation::new(Algorithm::HS256);
    validation.validate_exp = true;
    let data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )
    .map_err(|err| {
        tracing::debug!(error = %err, "jwt rejected");
        AppError::unauthorized("invalid token")
    })?;
    Uuid::parse_str(&data.claims.sub).map_err(|_| AppError::unauthorized("invalid token"))
}

fn dummy_password_hash() -> &'static str {
    static DUMMY: OnceLock<String> = OnceLock::new();
    DUMMY.get_or_init(|| hash_password("dummy-password-not-a-user").unwrap_or_default())
}

pub fn reject_unknown_user(password: &str) {
    let hash = dummy_password_hash();
    if hash.is_empty() {
        return;
    }
    let _ = verify_password(password, hash);
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let header = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .ok_or_else(|| AppError::unauthorized("missing authorization header"))?
            .to_str()
            .map_err(|_| AppError::unauthorized("invalid authorization header"))?;
        let token =
            bearer_token(header).ok_or_else(|| AppError::unauthorized("expected bearer token"))?;
        let user_id = verify_token(&state.config.jwt_secret, token)?;
        Ok(AuthUser { user_id })
    }
}

fn bearer_token(value: &str) -> Option<&str> {
    let (scheme, token) = value.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    let token = token.trim();
    if token.is_empty() { None } else { Some(token) }
}
