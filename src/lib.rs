//! The blackwall sync blob service. See `docs/sync-protocol.md` §3.

pub mod config;
pub mod db;
pub mod error;
pub mod routes;
pub mod token;
pub mod validate;

use axum::Router;
use sqlx::PgPool;

/// Builds the service's router on top of a connection pool.
pub fn app(pool: PgPool) -> Router {
    routes::router(pool)
}
