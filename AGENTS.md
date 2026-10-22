# blackwall-backend

The sync blob service for blackwall: a Rust service on PostgreSQL
that stores one end-to-end encrypted blob per sync key, so blackwall's config and
backlog can sync between machines without logins.

## Source of truth

- [`docs/sync-protocol.md`](docs/sync-protocol.md) is the contract between this
  server and the blackwall client: endpoints, status codes, crypto, document
  format and merge rules. Change it first, then the code.
- blackwall's sync tasks use this repo as a reference repo and read the contract
  from here. blackwall keeps no copy of it.

## Rules

- Never log request or response bodies (blobs, write tokens). Method, path,
  status, duration and size are fine.
- Compare write tokens in constant time.
- The server never sees the key or plaintext; don't add anything that needs them.

## Commands

- Start the database: `docker compose up -d postgres`
- Check: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`
- Run: `cargo run` with `DATABASE_URL` set, or `docker compose up --build`.
- Deploy: see `deploy/README.md` (`docker compose -f deploy/compose.yml up -d --build` on the server).
- After changing a query or migration: install sqlx-cli once with
  `cargo install sqlx-cli --no-default-features --features rustls,postgres`, then
  run `sqlx migrate run` and `cargo sqlx prepare -- --all-targets`, and commit
  `.sqlx/`. Check freshness with `cargo sqlx prepare --check -- --all-targets`.
- CI (`.github/workflows/ci.yml`) runs all of the above plus `cargo sqlx prepare --check`
  and `docker build`; keep them in sync.
