# Deploying the sync service

`deploy/compose.yml` is the production stack: the service image built from the
repo's `Dockerfile`, PostgreSQL 17 and a `backup` sidecar that dumps the
database every night. It is meant to sit behind a reverse proxy that terminates
TLS. The root `docker-compose.yml` stays the local development setup.

All commands below run from the repo root on the server.

## 1. Status

The hosting environment and the public URL are not decided yet. This directory
is the host-agnostic part; choosing and configuring the reverse proxy, DNS and
the actual rollout is a follow-up task. Until then nothing is live and the
blackwall client has no default endpoint (contract §9 decision 1).

## 2. Prerequisites

- A Linux host with Docker Engine and the Docker Compose v2 plugin
  (`docker compose version`).
- A reverse proxy that meets the requirements in section 3.

## 3. Reverse proxy requirements

Whichever proxy is chosen must:

- **Terminate TLS** and forward plain HTTP to `SYNC_BIND_ADDR:SYNC_PORT`
  (default `127.0.0.1:8080`).
- **Impose no request body cap.** The contract has no body size limit
  ([`docs/sync-protocol.md`](../docs/sync-protocol.md) §3.1, §8 and §9
  decision 8). A `PUT` body is base64url JSON, about 4/3 the size of the stored
  `bytea`, so even a 1 GB cap rejects blobs Postgres accepts. A proxy that
  rejects a large `PUT` shows up in the client as `server-error`.
  - nginx: `client_max_body_size 0;` (the default is 1 MB).
  - Caddy: no `request_body` directive.
  - Traefik: no buffering middleware with `maxRequestBodyBytes`.
- **Not log request headers or bodies.** `X-Write-Token` is a secret. The
  service already logs method, path, status, latency and size.

Keep `SYNC_BIND_ADDR=127.0.0.1` when the proxy runs on the same host.
`SYNC_BIND_ADDR=0.0.0.0` publishes the service over **plain HTTP** on every
interface, and Docker-published ports bypass ufw/iptables `INPUT` rules, so a
host firewall does not protect it. Only change it if the proxy runs on another
machine and the port is firewalled upstream of the host.

## 4. First deploy

```sh
git clone <repo-url> blackwall-backend && cd blackwall-backend
cp deploy/.env.example deploy/.env
# Edit deploy/.env: set POSTGRES_PASSWORD (openssl rand -hex 32) and review the rest.
mkdir -m 700 deploy/backups
docker compose -f deploy/compose.yml up -d --build
```

The service applies the database migrations on startup. Compose refuses to
start if `POSTGRES_PASSWORD` is unset.

Then configure the reverse proxy to the requirements in section 3.

## 5. Smoke test

On the host:

```sh
curl -i http://127.0.0.1:8080/healthz   # 200
```

Through the proxy, from another machine:

```sh
URL=https://<host>
curl -i "$URL/healthz"   # 200, with a valid certificate

# A large PUT catches a proxy body cap. Use a throwaway syncId.
ID=$(openssl rand -hex 32)
{ printf '{"blob":"'; head -c 5000000 /dev/zero | tr '\0' 'A'; printf '"}'; } > /tmp/blob.json
curl -i -X PUT "$URL/v1/sync/$ID" -H 'Content-Type: application/json' \
  -H 'X-Write-Token: smoke' --data-binary @/tmp/blob.json   # 200 {"version":1}

# Clean up: 204, and a later GET returns 410.
curl -i -X DELETE "$URL/v1/sync/$ID" -H 'X-Write-Token: smoke'
curl -i "$URL/v1/sync/$ID"
```

## 6. Update

```sh
git pull && docker compose -f deploy/compose.yml up -d --build
```

Building compiles the Rust binary on the server, which takes a few minutes of
CPU. New migrations run when the service restarts.

## 7. Backups

- The `backup` container writes `deploy/backups/blackwall-<UTC timestamp>.dump`
  (`pg_dump -Fc`) once when it starts and then every `BACKUP_INTERVAL_SECONDS`
  (default 86400, daily).
- Dumps older than `BACKUP_RETENTION_DAYS` (default 14) are deleted.
- Each dump is written to a `.dump.tmp` file and renamed when complete. A
  leftover `.dump.tmp` from an interrupted run is removed on the next pass.
- A failed dump is logged as `backup: pg_dump failed` and retried on the next
  pass.
- The dumps contain write-token hashes; keep `deploy/backups/` at mode `700`.
- Copying `deploy/backups/` off the server is the host's job. Until that is set
  up, a disk loss takes the backups with it.
- Run only one `backup` container per backup directory: the `.tmp` cleanup
  assumes it is the only writer.
- `pg_dump` and the server both come from `postgres:17`. A major PostgreSQL
  upgrade needs a dump and restore.

## 8. Restore

```sh
docker compose -f deploy/compose.yml stop service
docker compose -f deploy/compose.yml exec -T postgres \
  pg_restore -U blackwall -d blackwall --clean --if-exists < deploy/backups/<file>.dump
docker compose -f deploy/compose.yml start service
```

## 9. Logs

```sh
docker compose -f deploy/compose.yml logs service backup
```

## 10. Note on the database volume

The `pgdata` volume is initialised only on first start. If it already exists
from a run with a different `POSTGRES_USER` or `POSTGRES_PASSWORD`, changing
`deploy/.env` does not re-initialise it; change the role in Postgres or remove
the volume (losing its data).
