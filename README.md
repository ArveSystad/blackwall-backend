# blackwall-backend

The sync blob service for blackwall. It stores one opaque, end-to-end encrypted
blob per `syncId`, with a version counter, write-token authorisation and
permanent deletion markers. It never sees the key or the plaintext.

The contract with the client is [`docs/sync-protocol.md`](docs/sync-protocol.md);
section 3 describes the HTTP API in full.

## Endpoints

| Method   | Path               | Summary                                                         |
| -------- | ------------------ | --------------------------------------------------------------- |
| `GET`    | `/v1/sync/:syncId` | `200 { version, blob }`, `404` never stored, `410` deleted.      |
| `PUT`    | `/v1/sync/:syncId` | Create (no `If-Match`) or update (`If-Match: <version>`). Needs `X-Write-Token`. |
| `DELETE` | `/v1/sync/:syncId` | Erase the blob for good, leaving a deletion marker. Needs `X-Write-Token`. |
| `GET`    | `/healthz`         | `200` when the database is reachable, `503` otherwise.          |

`syncId` must match `^[0-9a-f]{64}$`; anything else is `400`. Blobs travel as
base64url without padding.

## Configuration

| Variable       | Default        | Meaning                                    |
| -------------- | -------------- | ------------------------------------------ |
| `DATABASE_URL` | (required)     | PostgreSQL connection string.              |
| `BIND_ADDR`    | `0.0.0.0:8080` | Address and port to listen on.             |
| `RUST_LOG`     | `info`         | `tracing` filter.                          |

The binary reads only the process environment (not `.env`). It applies the
migrations in `migrations/` on startup.

## Running with Docker Compose

```sh
docker compose up --build
curl -i http://localhost:8080/healthz
```

This starts PostgreSQL 17 and the service on port 8080.

## Local development

```sh
docker compose up -d postgres
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
DATABASE_URL=postgres://postgres:postgres@localhost:5432/blackwall cargo run
```

The tests use `#[sqlx::test]`: each test gets a fresh throwaway database with
the migrations applied, so PostgreSQL must be running and `DATABASE_URL` (from
`.env`) must point at a user that may create databases.

### sqlx offline metadata

Queries are checked at compile time with `sqlx::query!`. The committed `.env`
sets `SQLX_OFFLINE=true`, so builds use the metadata in `.sqlx/` instead of a
live database. After changing a query or a migration:

```sh
cargo install sqlx-cli --no-default-features --features rustls,postgres  # once
sqlx migrate run
cargo sqlx prepare -- --all-targets
cargo sqlx prepare --check -- --all-targets  # confirms .sqlx/ is current
```

Commit the updated `.sqlx/`. A stale `.sqlx/` fails the build on purpose.

### CI

[`.github/workflows/ci.yml`](.github/workflows/ci.yml) runs on every push to
`main` and every pull request. It runs the same fmt, clippy and test commands,
plus `cargo sqlx prepare --check -- --all-targets` against PostgreSQL 17 and a
Docker build. A stale `.sqlx/` therefore fails CI.

## Deployment

The production stack (service, PostgreSQL and nightly backups) and its runbook
are in [`deploy/README.md`](deploy/README.md).

There are no body size or rate limits (contract §8). A reverse proxy in front of
the service must raise its own body limit (nginx defaults to 1 MB), or large
`PUT`s fail before they reach the service.
