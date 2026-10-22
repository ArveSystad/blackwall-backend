//! Queries on the `blobs` table. Functions taking a `PgConnection` are meant
//! to run inside the request's transaction.

use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgPool};

/// A `blobs` row. A live row has `blob` and `write_token_hash` set; a deletion
/// marker has `deleted_at` set and both of them `NULL`.
#[derive(Debug)]
pub struct Row {
    pub version: i64,
    pub blob: Option<Vec<u8>>,
    pub write_token_hash: Option<Vec<u8>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

/// Reads the row and locks it until the transaction ends.
pub async fn lock(conn: &mut PgConnection, sync_id: &str) -> sqlx::Result<Option<Row>> {
    sqlx::query_as!(
        Row,
        "SELECT version, blob, write_token_hash, deleted_at
         FROM blobs WHERE sync_id = $1 FOR UPDATE",
        sync_id
    )
    .fetch_optional(conn)
    .await
}

/// Reads the row without a lock.
pub async fn get(pool: &PgPool, sync_id: &str) -> sqlx::Result<Option<Row>> {
    sqlx::query_as!(
        Row,
        "SELECT version, blob, write_token_hash, deleted_at
         FROM blobs WHERE sync_id = $1",
        sync_id
    )
    .fetch_optional(pool)
    .await
}

/// Inserts a new blob at version 1. `None` means another create won the race.
pub async fn try_create(
    conn: &mut PgConnection,
    sync_id: &str,
    blob: &[u8],
    write_token_hash: &[u8],
) -> sqlx::Result<Option<i64>> {
    sqlx::query_scalar!(
        "INSERT INTO blobs (sync_id, version, blob, write_token_hash)
         VALUES ($1, 1, $2, $3)
         ON CONFLICT (sync_id) DO NOTHING
         RETURNING version",
        sync_id,
        blob,
        write_token_hash
    )
    .fetch_optional(conn)
    .await
}

/// Re-reads the version and deletion time after a lost create race.
pub async fn current(
    conn: &mut PgConnection,
    sync_id: &str,
) -> sqlx::Result<Option<(i64, Option<DateTime<Utc>>)>> {
    let row = sqlx::query!(
        "SELECT version, deleted_at FROM blobs WHERE sync_id = $1",
        sync_id
    )
    .fetch_optional(conn)
    .await?;
    Ok(row.map(|r| (r.version, r.deleted_at)))
}

/// Replaces the blob and bumps the version. Returns the new version.
pub async fn update(conn: &mut PgConnection, sync_id: &str, blob: &[u8]) -> sqlx::Result<i64> {
    sqlx::query_scalar!(
        "UPDATE blobs SET blob = $2, version = version + 1, updated_at = now()
         WHERE sync_id = $1
         RETURNING version",
        sync_id,
        blob
    )
    .fetch_one(conn)
    .await
}

/// Erases the blob and token hash, leaving a deletion marker.
pub async fn mark_deleted(conn: &mut PgConnection, sync_id: &str) -> sqlx::Result<()> {
    sqlx::query!(
        "UPDATE blobs
         SET blob = NULL, write_token_hash = NULL, deleted_at = now(), updated_at = now()
         WHERE sync_id = $1",
        sync_id
    )
    .execute(conn)
    .await?;
    Ok(())
}
