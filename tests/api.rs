use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use blackwall_backend::{app, db, token};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;

const TOKEN: &str = "c2VjcmV0LXdyaXRlLXRva2VuLWZvci10ZXN0cy1vbmx5";
const OTHER_TOKEN: &str = "b3RoZXItd3JpdGUtdG9rZW4tZm9yLXRlc3RzLW9ubHkx";

fn id(n: u64) -> String {
    format!("{n:064x}")
}

fn url(id: &str) -> String {
    format!("/v1/sync/{id}")
}

fn put_raw(
    id: &str,
    content_type: &str,
    body: impl Into<Body>,
    if_match: Option<&str>,
    token: Option<&str>,
) -> Request<Body> {
    let mut req = Request::put(url(id)).header("content-type", content_type);
    if let Some(v) = if_match {
        req = req.header("if-match", v);
    }
    if let Some(t) = token {
        req = req.header("x-write-token", t);
    }
    req.body(body.into()).unwrap()
}

fn put(id: &str, blob: &[u8], if_match: Option<i64>, token: Option<&str>) -> Request<Body> {
    let body = json!({ "blob": URL_SAFE_NO_PAD.encode(blob) }).to_string();
    let if_match = if_match.map(|v| v.to_string());
    put_raw(id, "application/json", body, if_match.as_deref(), token)
}

fn get(id: &str) -> Request<Body> {
    Request::get(url(id)).body(Body::empty()).unwrap()
}

fn delete(id: &str, token: Option<&str>) -> Request<Body> {
    let mut req = Request::delete(url(id));
    if let Some(t) = token {
        req = req.header("x-write-token", t);
    }
    req.body(Body::empty()).unwrap()
}

async fn send(app: &Router, req: Request<Body>) -> (StatusCode, Value) {
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    (status, body)
}

async fn create(app: &Router, id: &str, blob: &[u8]) {
    let (status, body) = send(app, put(id, blob, None, Some(TOKEN))).await;
    assert_eq!((status, body), (StatusCode::OK, json!({ "version": 1 })));
}

fn bad_ids() -> Vec<String> {
    vec![
        "A".repeat(64),
        "a".repeat(63),
        "a".repeat(65),
        format!("{}g", "a".repeat(63)),
    ]
}

// --- syncId -----------------------------------------------------------------

#[sqlx::test]
async fn malformed_sync_id_is_400_on_every_endpoint(pool: PgPool) {
    let app = app(pool);
    for bad in bad_ids() {
        // Otherwise-valid requests, so only the id can cause the 400.
        assert_eq!(
            send(&app, get(&bad)).await.0,
            StatusCode::BAD_REQUEST,
            "{bad}"
        );
        let (status, _) = send(&app, put(&bad, b"x", None, Some(TOKEN))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
        let (status, _) = send(&app, delete(&bad, Some(TOKEN))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
        // And with everything else broken too.
        let req = put_raw(&bad, "text/plain", "nope", Some("x"), None);
        assert_eq!(send(&app, req).await.0, StatusCode::BAD_REQUEST, "{bad}");
        assert_eq!(
            send(&app, delete(&bad, None)).await.0,
            StatusCode::BAD_REQUEST
        );
    }
}

#[sqlx::test]
async fn uppercase_form_of_existing_id_is_400(pool: PgPool) {
    let app = app(pool);
    let lower = "ab".repeat(32);
    create(&app, &lower, b"x").await;
    let upper = lower.to_uppercase();
    assert_eq!(send(&app, get(&upper)).await.0, StatusCode::BAD_REQUEST);
    let (status, _) = send(&app, put(&upper, b"y", Some(1), Some(TOKEN))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        send(&app, delete(&upper, Some(TOKEN))).await.0,
        StatusCode::BAD_REQUEST
    );
}

// --- GET --------------------------------------------------------------------

#[sqlx::test]
async fn get_unknown_is_404(pool: PgPool) {
    let app = app(pool);
    let (status, body) = send(&app, get(&id(1))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body, json!({ "error": "not-found" }));
}

#[sqlx::test]
async fn get_returns_created_blob(pool: PgPool) {
    let app = app(pool);
    let blob = b"\x01\xff\xfe opaque bytes";
    create(&app, &id(1), blob).await;
    let (status, body) = send(&app, get(&id(1))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body,
        json!({ "version": 1, "blob": URL_SAFE_NO_PAD.encode(blob) })
    );
}

// --- PUT create ---------------------------------------------------------------

#[sqlx::test]
async fn create_stores_token_hash(pool: PgPool) {
    let app = app(pool.clone());
    create(&app, &id(1), b"blob").await;
    let row = db::get(&pool, &id(1)).await.unwrap().unwrap();
    assert_eq!(row.version, 1);
    assert_eq!(row.blob.as_deref(), Some(&b"blob"[..]));
    assert_eq!(
        row.write_token_hash.as_deref(),
        Some(&token::hash(TOKEN.as_bytes())[..])
    );
    assert!(row.deleted_at.is_none());
}

#[sqlx::test]
async fn create_accepts_empty_blob(pool: PgPool) {
    let app = app(pool);
    create(&app, &id(1), b"").await;
    let (_, body) = send(&app, get(&id(1))).await;
    assert_eq!(body, json!({ "version": 1, "blob": "" }));
}

#[sqlx::test]
async fn create_without_token_is_400(pool: PgPool) {
    let app = app(pool.clone());
    let (status, _) = send(&app, put(&id(1), b"x", None, None)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = send(&app, put(&id(1), b"x", None, Some(""))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(db::get(&pool, &id(1)).await.unwrap().is_none());
}

#[sqlx::test]
async fn malformed_body_is_400(pool: PgPool) {
    let app = app(pool.clone());
    let padded = json!({ "blob": URL_SAFE_NO_PAD.encode(b"x") + "==" }).to_string();
    let standard = json!({ "blob": STANDARD.encode([0xfb, 0xff, 0xfe]) }).to_string();
    let bodies = [
        "not json".to_owned(),
        "".to_owned(),
        "{}".to_owned(),
        r#"{"blob":42}"#.to_owned(),
        r#"{"blob":null}"#.to_owned(),
        r#"{"blob":"not base64!"}"#.to_owned(),
        padded,
        standard,
    ];
    for body in bodies {
        let req = put_raw(&id(1), "application/json", body.clone(), None, Some(TOKEN));
        assert_eq!(send(&app, req).await.0, StatusCode::BAD_REQUEST, "{body}");
    }
    assert!(db::get(&pool, &id(1)).await.unwrap().is_none());
}

#[sqlx::test]
async fn content_type_is_not_checked(pool: PgPool) {
    let app = app(pool);
    let body = json!({ "blob": "AQ" }).to_string();
    let req = put_raw(&id(1), "text/plain", body, None, Some(TOKEN));
    assert_eq!(
        send(&app, req).await,
        (StatusCode::OK, json!({ "version": 1 }))
    );
}

// --- If-Match -----------------------------------------------------------------

#[sqlx::test]
async fn malformed_if_match_is_400(pool: PgPool) {
    let app = app(pool);
    create(&app, &id(1), b"x").await;
    let body = json!({ "blob": "AQ" }).to_string();
    for bad in [
        "abc",
        "\"1\"",
        "-1",
        "+1",
        "1.0",
        "",
        "W/\"1\"",
        "9223372036854775808",
    ] {
        let req = put_raw(
            &id(1),
            "application/json",
            body.clone(),
            Some(bad),
            Some(TOKEN),
        );
        assert_eq!(send(&app, req).await.0, StatusCode::BAD_REQUEST, "{bad:?}");
    }
}

#[sqlx::test]
async fn if_match_on_unknown_id_is_404(pool: PgPool) {
    let app = app(pool.clone());
    let (status, _) = send(&app, put(&id(1), b"x", Some(1), Some(TOKEN))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(db::get(&pool, &id(1)).await.unwrap().is_none());
}

// --- PUT update ---------------------------------------------------------------

#[sqlx::test]
async fn put_without_if_match_on_existing_is_409(pool: PgPool) {
    let app = app(pool);
    create(&app, &id(1), b"x").await;
    for token in [Some(TOKEN), Some(OTHER_TOKEN), None] {
        let res = send(&app, put(&id(1), b"y", None, token)).await;
        assert_eq!(
            res,
            (StatusCode::CONFLICT, json!({ "version": 1 })),
            "{token:?}"
        );
    }
}

#[sqlx::test]
async fn put_with_wrong_or_missing_token_is_403(pool: PgPool) {
    let app = app(pool.clone());
    create(&app, &id(1), b"x").await;
    for token in [Some(OTHER_TOKEN), Some(""), None] {
        let (status, _) = send(&app, put(&id(1), b"y", Some(1), token)).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{token:?}");
    }
    let row = db::get(&pool, &id(1)).await.unwrap().unwrap();
    assert_eq!((row.version, row.blob.as_deref()), (1, Some(&b"x"[..])));
}

#[sqlx::test]
async fn token_check_comes_before_version_check(pool: PgPool) {
    let app = app(pool);
    create(&app, &id(1), b"x").await;
    let (status, _) = send(&app, put(&id(1), b"y", Some(7), Some(OTHER_TOKEN))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[sqlx::test]
async fn update_bumps_version_and_rejects_stale(pool: PgPool) {
    let app = app(pool);
    create(&app, &id(1), b"one").await;

    let res = send(&app, put(&id(1), b"two", Some(1), Some(TOKEN))).await;
    assert_eq!(res, (StatusCode::OK, json!({ "version": 2 })));

    let res = send(&app, put(&id(1), b"stale", Some(1), Some(TOKEN))).await;
    assert_eq!(res, (StatusCode::CONFLICT, json!({ "version": 2 })));

    let res = send(&app, put(&id(1), b"three", Some(2), Some(TOKEN))).await;
    assert_eq!(res, (StatusCode::OK, json!({ "version": 3 })));

    let (_, body) = send(&app, get(&id(1))).await;
    assert_eq!(
        body,
        json!({ "version": 3, "blob": URL_SAFE_NO_PAD.encode(b"three") })
    );
}

#[sqlx::test]
async fn large_blob_round_trips(pool: PgPool) {
    let app = app(pool);
    // ~3 MB of xorshift noise: well above typical framework default limits.
    let mut state: u64 = 0x9e37_79b9_7f4a_7c15;
    let blob: Vec<u8> = (0..3 * 1024 * 1024)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state as u8
        })
        .collect();
    create(&app, &id(1), &blob).await;
    let (status, body) = send(&app, get(&id(1))).await;
    assert_eq!(status, StatusCode::OK);
    let got = URL_SAFE_NO_PAD
        .decode(body["blob"].as_str().unwrap())
        .unwrap();
    assert!(got == blob, "blob differs after round trip");
}

// --- DELETE -------------------------------------------------------------------

#[sqlx::test]
async fn delete_unknown_is_404(pool: PgPool) {
    let app = app(pool);
    let (status, _) = send(&app, delete(&id(1), Some(TOKEN))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[sqlx::test]
async fn delete_with_wrong_or_missing_token_is_403(pool: PgPool) {
    let app = app(pool.clone());
    create(&app, &id(1), b"x").await;
    for token in [Some(OTHER_TOKEN), Some(""), None] {
        let (status, _) = send(&app, delete(&id(1), token)).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{token:?}");
    }
    let row = db::get(&pool, &id(1)).await.unwrap().unwrap();
    assert_eq!((row.version, row.blob.as_deref()), (1, Some(&b"x"[..])));
    assert!(row.write_token_hash.is_some());
    assert!(row.deleted_at.is_none());
}

#[sqlx::test]
async fn delete_leaves_marker_and_everything_is_410_after(pool: PgPool) {
    let app = app(pool.clone());
    create(&app, &id(1), b"x").await;
    send(&app, put(&id(1), b"y", Some(1), Some(TOKEN))).await;

    let res = app
        .clone()
        .oneshot(delete(&id(1), Some(TOKEN)))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NO_CONTENT);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    assert!(bytes.is_empty());

    let row = db::get(&pool, &id(1)).await.unwrap().unwrap();
    assert_eq!(row.version, 2);
    assert!(row.blob.is_none());
    assert!(row.write_token_hash.is_none());
    assert!(row.deleted_at.is_some());

    let gone = (StatusCode::GONE, json!({ "error": "gone" }));
    assert_eq!(send(&app, get(&id(1))).await, gone);
    assert_eq!(send(&app, put(&id(1), b"z", None, Some(TOKEN))).await, gone);
    assert_eq!(
        send(&app, put(&id(1), b"z", Some(2), Some(TOKEN))).await,
        gone
    );
    assert_eq!(send(&app, put(&id(1), b"z", None, None)).await, gone);
    assert_eq!(send(&app, delete(&id(1), Some(TOKEN))).await, gone);
    assert_eq!(send(&app, delete(&id(1), None)).await, gone);
}

// --- Races ------------------------------------------------------------------

#[sqlx::test]
async fn concurrent_creates_have_one_winner(pool: PgPool) {
    let app = app(pool);
    let tasks: Vec<_> = (0..10)
        .map(|i| {
            let app = app.clone();
            tokio::spawn(async move {
                send(
                    &app,
                    put(&id(1), format!("blob {i}").as_bytes(), None, Some(TOKEN)),
                )
                .await
            })
        })
        .collect();
    let mut ok = 0;
    for task in tasks {
        let (status, body) = task.await.unwrap();
        assert_eq!(body, json!({ "version": 1 }));
        match status {
            StatusCode::OK => ok += 1,
            StatusCode::CONFLICT => {}
            other => panic!("unexpected status {other}"),
        }
    }
    assert_eq!(ok, 1);
}

#[sqlx::test]
async fn try_create_on_existing_row_returns_none(pool: PgPool) {
    let app = app(pool.clone());
    create(&app, &id(1), b"x").await;
    let mut tx = pool.begin().await.unwrap();
    let hash = token::hash(TOKEN.as_bytes());
    let res = db::try_create(&mut tx, &id(1), b"y", &hash).await.unwrap();
    assert_eq!(res, None);
    assert_eq!(db::current(&mut tx, &id(1)).await.unwrap(), Some((1, None)));
}

#[sqlx::test]
async fn concurrent_updates_have_one_winner(pool: PgPool) {
    let app = app(pool);
    create(&app, &id(1), b"x").await;
    let tasks: Vec<_> = (0..2)
        .map(|i| {
            let app = app.clone();
            tokio::spawn(async move {
                send(
                    &app,
                    put(&id(1), format!("v{i}").as_bytes(), Some(1), Some(TOKEN)),
                )
                .await
            })
        })
        .collect();
    let mut results = Vec::new();
    for task in tasks {
        results.push(task.await.unwrap());
    }
    results.sort_by_key(|(status, _)| status.as_u16());
    assert_eq!(
        results,
        vec![
            (StatusCode::OK, json!({ "version": 2 })),
            (StatusCode::CONFLICT, json!({ "version": 2 })),
        ]
    );
}

// --- Healthz ------------------------------------------------------------------

#[sqlx::test]
async fn healthz_reflects_database(pool: PgPool) {
    let app = app(pool.clone());
    let req = || Request::get("/healthz").body(Body::empty()).unwrap();
    assert_eq!(
        send(&app, req()).await,
        (StatusCode::OK, json!({ "status": "ok" }))
    );
    pool.close().await;
    assert_eq!(
        send(&app, req()).await,
        (
            StatusCode::SERVICE_UNAVAILABLE,
            json!({ "error": "database-unavailable" })
        )
    );
}
