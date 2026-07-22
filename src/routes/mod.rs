mod auth;
mod endpoints;
mod events;

use axum::extract::DefaultBodyLimit;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Serialize;
use tower_http::trace::TraceLayer;

use crate::state::AppState;

const BODY_LIMIT: usize = 256 * 1024;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/auth/register", post(auth::register))
        .route("/auth/login", post(auth::login))
        .route(
            "/endpoints",
            get(endpoints::list_endpoints).post(endpoints::create_endpoint),
        )
        .route(
            "/endpoints/{id}",
            axum::routing::delete(endpoints::delete_endpoint),
        )
        .route("/events", post(events::create_event))
        .route("/events/{id}", get(events::get_event))
        .route("/events/{id}/replay", post(events::replay_event))
        .layer(DefaultBodyLimit::max(BODY_LIMIT))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

#[derive(Serialize)]
struct Health {
    status: &'static str,
}

async fn health() -> Json<Health> {
    Json(Health { status: "ok" })
}
