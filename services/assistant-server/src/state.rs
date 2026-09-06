//! Shared application state.
//!
//! Held in an `Arc` and cloned into every handler. It carries seams (the auth
//! verifier, the event bus), not business logic.

use std::sync::Arc;

use assistant_auth::TokenVerifier;
use assistant_core::EventBus;
use sqlx::PgPool;

pub type SharedState = Arc<AppState>;

pub struct AppState {
    pub verifier: Arc<dyn TokenVerifier>,
    pub events: EventBus,
    /// `None` when no `DATABASE_URL` was configured.
    pub db: Option<PgPool>,
}

impl AppState {
    pub fn is_healthy(&self) -> bool {
        self.db.is_some()
    }
}
