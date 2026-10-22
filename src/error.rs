use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

/// Every non-success outcome of the sync API (contract §3).
#[derive(Debug, PartialEq, Eq)]
pub enum ApiError {
    BadRequest,
    Forbidden,
    NotFound,
    Gone,
    /// The current version of the blob.
    Conflict(i64),
    Internal,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, body) = match self {
            ApiError::BadRequest => (StatusCode::BAD_REQUEST, json!({ "error": "bad-request" })),
            ApiError::Forbidden => (StatusCode::FORBIDDEN, json!({ "error": "forbidden" })),
            ApiError::NotFound => (StatusCode::NOT_FOUND, json!({ "error": "not-found" })),
            ApiError::Gone => (StatusCode::GONE, json!({ "error": "gone" })),
            ApiError::Conflict(version) => (StatusCode::CONFLICT, json!({ "version": version })),
            ApiError::Internal => (
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({ "error": "internal" }),
            ),
        };
        (status, Json(body)).into_response()
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(err: sqlx::Error) -> Self {
        // sqlx errors carry no bind values, so no blob or token hash ends up here.
        tracing::error!(error = %err, "database error");
        ApiError::Internal
    }
}
