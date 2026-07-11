use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;
use sqlx::error::ErrorKind;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("{0}")]
    BadRequest(String),
    #[error("{0}")]
    Unauthorized(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Conflict(String),
    #[error(transparent)]
    Internal(InternalError),
}

#[derive(Debug)]
pub struct InternalError(String);

impl std::fmt::Display for InternalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for InternalError {}

impl AppError {
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::BadRequest(message.into())
    }

    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::Unauthorized(message.into())
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::NotFound(message.into())
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::Conflict(message.into())
    }

    pub fn internal(err: impl ToString) -> Self {
        Self::Internal(InternalError(err.to_string()))
    }

    pub fn from_sqlx(err: sqlx::Error, duplicate_message: &str) -> Self {
        if let sqlx::Error::Database(db_err) = &err
            && db_err.kind() == ErrorKind::UniqueViolation
        {
            return Self::conflict(duplicate_message);
        }
        Self::internal(err)
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            Self::BadRequest(message) => (StatusCode::BAD_REQUEST, message.clone()),
            Self::Unauthorized(message) => (StatusCode::UNAUTHORIZED, message.clone()),
            Self::NotFound(message) => (StatusCode::NOT_FOUND, message.clone()),
            Self::Conflict(message) => (StatusCode::CONFLICT, message.clone()),
            Self::Internal(err) => {
                tracing::error!(error = %err, "internal server error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal server error".to_string(),
                )
            }
        };

        (status, Json(json!({ "error": message }))).into_response()
    }
}
