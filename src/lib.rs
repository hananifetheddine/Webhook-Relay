#![forbid(unsafe_code)]

pub mod auth;
pub mod backoff;
pub mod config;
pub mod db;
pub mod domain;
pub mod error;
pub mod routes;
pub mod signing;
pub mod state;
pub mod validation;
pub mod worker;

pub use config::Config;
pub use error::AppError;
pub use state::AppState;

pub fn router(state: AppState) -> axum::Router {
    routes::router(state)
}

pub async fn migrate(pool: &sqlx::MySqlPool) -> Result<(), sqlx::migrate::MigrateError> {
    sqlx::migrate!("./migrations").run(pool).await
}
