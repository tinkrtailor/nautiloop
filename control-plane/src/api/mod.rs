pub mod auth;
pub mod handlers;
pub mod sse;

use std::sync::Arc;

use axum::routing::{delete, get, post};
use axum::Router;

use crate::git::GitOperations;
use crate::state::StateStore;

/// Shared application state for all API handlers.
#[derive(Clone)]
pub struct AppState {
    pub store: Arc<dyn StateStore>,
    pub git: Arc<dyn GitOperations>,
}

/// Build the axum router with all endpoints.
pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/submit", post(handlers::submit))
        .route("/status", get(handlers::status))
        .route("/logs/{id}", get(handlers::logs))
        .route("/cancel/{id}", delete(handlers::cancel))
        .route("/approve/{id}", post(handlers::approve))
        .route("/resume/{id}", post(handlers::resume))
        .route("/inspect/{user}/{branch}", get(handlers::inspect))
        .with_state(state)
}
