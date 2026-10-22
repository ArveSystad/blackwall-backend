use std::time::Duration;

use axum::Router;
use axum::body::{Body, Bytes, HttpBody};
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{HeaderMap, HeaderValue, Request, Response, StatusCode};
use axum::response::{IntoResponse, Json};
use axum::routing::get;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use serde_json::json;
use sqlx::PgPool;
use tower_http::trace::TraceLayer;
use tracing::Span;

use crate::db;
use crate::error::ApiError;
use crate::token;
use crate::validate;

const WRITE_TOKEN: &str = "x-write-token";

pub fn router(pool: PgPool) -> Router {
    // Only method, path, status, latency and size are logged: never headers
    // (the write token is one), the query string, or bodies.
    let trace = TraceLayer::new_for_http()
        .make_span_with(|req: &Request<Body>| {
            tracing::info_span!("request", method = %req.method(), path = %req.uri().path())
        })
        .on_request(())
        .on_response(|res: &Response<Body>, latency: Duration, _span: &Span| {
            tracing::info!(
                status = res.status().as_u16(),
                latency_ms = latency.as_millis() as u64,
                size = res.body().size_hint().exact(),
                "response"
            );
        });

    Router::new()
        .route(
            "/v1/sync/{sync_id}",
            get(get_blob).put(put_blob).delete(delete_blob),
        )
        .route("/healthz", get(healthz))
        .layer(DefaultBodyLimit::disable())
        .layer(trace)
        .with_state(pool)
}

#[derive(Deserialize)]
struct PutBody {
    blob: String,
}

/// The `X-Write-Token` header; an empty value counts as no token.
fn write_token(headers: &HeaderMap) -> Option<&HeaderValue> {
    headers.get(WRITE_TOKEN).filter(|v| !v.is_empty())
}

async fn get_blob(
    State(pool): State<PgPool>,
    Path(sync_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    validate::sync_id(&sync_id)?;

    let row = db::get(&pool, &sync_id).await?.ok_or(ApiError::NotFound)?;
    if row.deleted_at.is_some() {
        return Err(ApiError::Gone);
    }
    let blob = row.blob.ok_or(ApiError::Internal)?;
    Ok(Json(json!({
        "version": row.version,
        "blob": URL_SAFE_NO_PAD.encode(blob),
    })))
}

async fn put_blob(
    State(pool): State<PgPool>,
    Path(sync_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<impl IntoResponse, ApiError> {
    validate::sync_id(&sync_id)?;
    let if_match = validate::if_match(&headers)?;
    let body: PutBody = serde_json::from_slice(&body).map_err(|_| ApiError::BadRequest)?;
    let blob = validate::decode_blob(&body.blob)?;
    let token = write_token(&headers);

    // Contract §3.5, order of checks.
    let mut tx = pool.begin().await?;
    let version = match db::lock(&mut tx, &sync_id).await? {
        Some(row) if row.deleted_at.is_some() => return Err(ApiError::Gone),
        None => {
            if if_match.is_some() {
                return Err(ApiError::NotFound);
            }
            let token = token.ok_or(ApiError::BadRequest)?;
            let hash = token::hash(token.as_bytes());
            match db::try_create(&mut tx, &sync_id, &blob, &hash).await? {
                Some(version) => version,
                None => {
                    // Another create committed between our lock and insert.
                    return match db::current(&mut tx, &sync_id).await? {
                        Some((_, Some(_))) => Err(ApiError::Gone),
                        Some((version, None)) => Err(ApiError::Conflict(version)),
                        None => Err(ApiError::Internal),
                    };
                }
            }
        }
        Some(row) => {
            let Some(expected) = if_match else {
                return Err(ApiError::Conflict(row.version));
            };
            let stored = row.write_token_hash.as_deref().ok_or(ApiError::Internal)?;
            if !token::matches(token, stored) {
                return Err(ApiError::Forbidden);
            }
            if expected != row.version {
                return Err(ApiError::Conflict(row.version));
            }
            db::update(&mut tx, &sync_id, &blob).await?
        }
    };
    tx.commit().await?;

    Ok(Json(json!({ "version": version })))
}

async fn delete_blob(
    State(pool): State<PgPool>,
    Path(sync_id): Path<String>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    validate::sync_id(&sync_id)?;

    // Contract §3.6: 410, then 404, then 403.
    let mut tx = pool.begin().await?;
    let row = db::lock(&mut tx, &sync_id).await?;
    match row {
        Some(row) if row.deleted_at.is_some() => return Err(ApiError::Gone),
        None => return Err(ApiError::NotFound),
        Some(row) => {
            let stored = row.write_token_hash.as_deref().ok_or(ApiError::Internal)?;
            if !token::matches(write_token(&headers), stored) {
                return Err(ApiError::Forbidden);
            }
        }
    }
    db::mark_deleted(&mut tx, &sync_id).await?;
    tx.commit().await?;

    Ok(StatusCode::NO_CONTENT)
}

async fn healthz(State(pool): State<PgPool>) -> impl IntoResponse {
    match sqlx::query_scalar!(r#"SELECT 1 AS "one!""#)
        .fetch_one(&pool)
        .await
    {
        Ok(_) => (StatusCode::OK, Json(json!({ "status": "ok" }))),
        Err(err) => {
            tracing::warn!(error = %err, "health check failed");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "error": "database-unavailable" })),
            )
        }
    }
}
