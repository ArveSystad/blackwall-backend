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

The service isn't implemented yet. The build, test and run commands are added
with the first implementation.
