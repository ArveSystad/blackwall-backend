# Deploying the sync service

`deploy/compose.yml` is the production stack: only the sync service, built from
the repo's `Dockerfile`. PostgreSQL is provided externally (section 3). The
service is meant to sit behind a reverse proxy that terminates TLS. The root
`docker-compose.yml` stays the local development setup.

All commands below run from the repo root on the server.

## 1. Status

The hosting environment and the public URL are not decided yet. This directory
is the host-agnostic part; choosing and configuring the reverse proxy, DNS and
the actual rollout is a follow-up task. Until then nothing is live and the
blackwall client has no default endpoint (contract §9 decision 1).

## 2. Prerequisites

- A Linux host with Docker Engine and the Docker Compose v2 plugin
  (`docker compose version`).
- An external PostgreSQL database that meets the requirements in section 3.
- A reverse proxy that meets the requirements in section 4.

## 3. External database requirements

- **PostgreSQL 17.**
- **A dedicated database** for the sync service, e.g. `blackwall`.
- **A non-superuser role that owns that database.** The service runs its sqlx
  migrations on startup, so the role must be able to create tables in the
  database. It needs no other privileges.
- **Network reachability from the container.** The service connects from inside
  Docker, so `localhost` in `DATABASE_URL` means the container, not the host.
  Use a hostname or address reachable from the Docker network, and allow the
  host in the database's firewall and `pg_hba.conf`.
- **TLS for a remote database.** Append `?sslmode=require` (or `verify-full` to
  also check the certificate) to `DATABASE_URL`.

Backups of the database are the database provider's responsibility; this stack
takes none.

## 4. Reverse proxy requirements

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

## 5. First deploy

Create the database and role first (section 3), then:

```sh
git clone <repo-url> blackwall-backend && cd blackwall-backend
cp deploy/.env.example deploy/.env
# Edit deploy/.env: set DATABASE_URL and review the rest.
chmod 600 deploy/.env
docker compose -f deploy/compose.yml up -d --build
```

The service applies the database migrations on startup and exits if it cannot
connect or migrate; check the logs (section 8). Compose refuses to start if
`DATABASE_URL` is unset.

Then configure the reverse proxy to the requirements in section 4.

## 6. Smoke test

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

## 7. Update

```sh
git pull && docker compose -f deploy/compose.yml up -d --build
```

Building compiles the Rust binary on the server, which takes a few minutes of
CPU. New migrations run against the external database when the service
restarts, so take or confirm a database backup with the provider before an
update that adds migrations.

## 8. Logs

```sh
docker compose -f deploy/compose.yml logs service
```
