# Blackwall sync protocol v1

This is the contract between the blackwall client and the blackwall sync
server in this repository. It is the single source of truth: the server and the
client are built against it separately, and neither keeps its own copy. A change
to anything here (a constant, a status code, a synced field or settings key)
means changing this file first.

Sync is login-free and end-to-end encrypted. One key, generated on one machine
and pasted into the others, gives the same blackwall config and backlog on
every machine. The server stores one opaque encrypted blob per key and never
sees the key or the plaintext.

Contents:

1. [Overview](#1-overview)
2. [Key and crypto](#2-key-and-crypto)
3. [HTTP API](#3-http-api)
4. [Plaintext document](#4-plaintext-document)
5. [Run confirmation](#5-run-confirmation)
6. [Merge and apply](#6-merge-and-apply)
7. [Client behaviour](#7-client-behaviour)
8. [Threat model](#8-threat-model)
9. [Decisions](#9-decisions)

## 1. Overview

- The client generates a random **key**. From it, it derives a public
  **`syncId`** (the blob's address on the server), an **`encKey`** (encrypts the
  blob) and a **write token** (proves the right to overwrite or delete it).
- The client builds a **document** from its local database (section 4),
  encrypts it into a **blob** and `PUT`s it under the `syncId`.
- Other machines with the same key `GET` the blob, decrypt it, merge it with
  their local data row by row (last writer wins, section 6) and push the result.
- The server only stores bytes, a version counter and a hash of the write token.
  It does no merging and has no accounts.

## 2. Key and crypto

The client uses only Node built-ins (`node:crypto`); no extra dependencies.

### 2.1 Key

- The key is 32 bytes from `crypto.randomBytes(32)`.
- It is stored only in the OS keyring, under account `sync-key` (through
  blackwall's `SecretStore`). It is never stored in settings, the database, the
  document or a log, and never sent to the server.

### 2.2 Key display format

The key is shown to and pasted by the user as **base58** text:

```
display = base58( key || SHA-256(key)[0..4] )
```

- Alphabet: Bitcoin
  `123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz`.
- Input: the 32 key bytes followed by the first 4 bytes of `SHA-256(key)`,
  36 bytes in all, encoded as one big-endian number. Each leading zero byte is
  encoded as a leading `1`, as in Bitcoin base58.
- The result is about 49–50 characters.
- Decoding:
  1. Trim leading and trailing whitespace (users copy and paste the key).
  2. Any character outside the alphabet, a decoded length other than 36 bytes,
     or a checksum that doesn't match the first 4 bytes of `SHA-256` of the
     first 32 bytes gives the error **`bad-key`**.

### 2.3 Derived values

All derivations use HKDF-SHA256 with an empty salt and a 32-byte output
(`crypto.hkdfSync("sha256", key, Buffer.alloc(0), info, 32)`).

| Value        | Derivation                                                    | Form                                    |
| ------------ | ------------------------------------------------------------- | --------------------------------------- |
| `syncId`     | `hex(HKDF-SHA256(key, salt="", info="blackwall-sync-id", 32))` | 64 lowercase hex characters             |
| `encKey`     | `HKDF-SHA256(key, salt="", info="blackwall-sync-enc", 32)`     | 32 bytes, never leaves the client       |
| `writeToken` | `base64url(HMAC-SHA256(encKey, "write"))`                      | 43 characters, base64url without padding |

- `info` strings and `"write"` are ASCII bytes.
- `syncId` is not secret: the server sees it. `encKey` and `writeToken` are
  independent of it, so knowing `syncId` gives neither.

### 2.4 Blob

- Cipher: AES-256-GCM with `encKey`, a fresh random 12-byte nonce on **every**
  push, and a 16-byte tag.
- AAD: the `syncId` as its 64 ASCII hex characters. A blob copied to another
  `syncId` fails to decrypt.
- Plaintext: the document (section 4) as UTF-8 JSON. No compression in v1.
- Layout (bytes):

  ```
  0x01 | nonce (12) | ciphertext (n) | tag (16)
  ```

  The first byte is the blob layout version. A client that sees any other first
  byte, a blob shorter than 29 bytes, or a failed tag check reports
  **`decrypt-failed`** and doesn't merge or push.
- On the wire the blob is base64url without padding.

## 3. HTTP API

### 3.1 General

- Base URL: the server URL the user enters in Settings → Sync. There is **no
  built-in default endpoint** in v1 (section 9, decision 1).
- All paths start with `/v1`, except `/healthz`.
- Request and response bodies are JSON (`Content-Type: application/json`),
  except `204` responses, which have no body.
- Error responses carry `{ "error": "<code>" }` where useful; clients rely only on
  the status code, plus `version` on `409`.
- `:syncId` must match `^[0-9a-f]{64}$`. Anything else gets **`400`** before any
  other check, on every endpoint.
- **No limits.** The contract has no body size cap, no rate limits, and no `413`
  or `429`. The server disables its framework's default body limit. See the
  threat model (section 8).
- The server **never logs request or response bodies** (blobs or tokens). It may
  log method, path, status, duration and body size.
- Blobs never expire in v1.

### 3.2 Versions

- `version` is a server-assigned integer. The first successful `PUT` (create)
  sets it to `1`; each later successful `PUT` adds `1`.
- `If-Match` carries the version the client last read, as a plain decimal
  integer (for example `If-Match: 7`). It is not an HTTP entity tag and is not
  quoted. A malformed value gets `400`.

### 3.3 Write token

- Header: `X-Write-Token: <writeToken>`.
- The write token is **mandatory** on every `PUT` and `DELETE`.
- On create (a `PUT` with no `If-Match` that stores a new blob), the server stores
  `SHA-256(writeToken)`, computed over the header value's ASCII bytes. It never
  stores the token itself.
- Every later `PUT` and every `DELETE` must send a token whose `SHA-256` equals
  the stored hash, compared in constant time. Otherwise the server returns `403`.
- A create without the header, or with an empty one, gets `400`. On an existing
  blob, a missing or wrong token gets `403`.

### 3.4 `GET /v1/sync/:syncId`

Reads the blob.

| Status | Body                         | Meaning                                       |
| ------ | ---------------------------- | --------------------------------------------- |
| `200`  | `{ "version": n, "blob": "<base64url>" }` | The current blob.                |
| `400`  |                              | Malformed `syncId`.                           |
| `404`  |                              | Nothing was ever stored under this `syncId`.  |
| `410`  |                              | The blob was deleted (section 3.6).           |

`GET` needs no write token: without the key the blob is unreadable.

### 3.5 `PUT /v1/sync/:syncId`

Creates or replaces the blob.

Request:

- Body: `{ "blob": "<base64url>" }`. A missing or non-decodable `blob` gets `400`.
  The server doesn't look inside the blob.
- `If-Match: <version>`: left out on create, required on every update.
- `X-Write-Token: <writeToken>`: always.

Responses:

| Status | Body               | Meaning                                                                                      |
| ------ | ------------------ | -------------------------------------------------------------------------------------------- |
| `200`  | `{ "version": n }` | Stored. `n` is the new version (`1` on create).                                              |
| `400`  |                    | Malformed `syncId`, body, `If-Match`, or a create without a write token.                     |
| `403`  |                    | The blob exists and the write token is missing or wrong.                                     |
| `404`  |                    | `If-Match` was sent but no blob exists.                                                      |
| `409`  | `{ "version": n }` | `If-Match` doesn't match the current version, or the PUT had no `If-Match` and a blob exists (including losing a create race). `n` is the current version. |
| `410`  |                    | The blob was deleted.                                                                        |

**Order of checks.** After the `400` validation, the server takes the row lock
and checks in this order, returning the first that applies:

1. A deletion marker exists → `410`.
2. No blob exists:
   - `If-Match` was sent → `404`.
   - Otherwise this is a create: insert the blob, the token hash and
     `version = 1`. If another create won the race, return `409 { version }`
     with the winner's version.
3. A blob exists and the PUT has no `If-Match` → `409 { version }`.
4. Token check → `403` on mismatch.
5. Version check: `If-Match` ≠ current version → `409 { version }`.
6. Replace the blob, add `1` to `version`, return `200 { version }`.

Step 3 comes before the token check so a client that joins an existing `syncId`
learns the version instead of a misleading `403`; it reveals nothing beyond what
`GET` already shows. Steps 2–6 run in one transaction with the row locked.

### 3.6 `DELETE /v1/sync/:syncId`

Erases the blob for good ("Delete server data").

Request: header `X-Write-Token: <writeToken>`. No body. No `If-Match`.

| Status | Meaning                                                |
| ------ | ------------------------------------------------------ |
| `204`  | Deleted.                                               |
| `400`  | Malformed `syncId`.                                    |
| `403`  | The write token is missing or wrong.                   |
| `404`  | Nothing was ever stored under this `syncId`.           |
| `410`  | Already deleted.                                       |

Checks run in the order `410`, `404`, `403`.

On success the server erases the blob and the token hash and keeps only a
**deletion marker** `(sync_id, deleted_at)`. From then on, `GET`, `PUT` and
`DELETE` on that `syncId` return **`410 Gone`**, forever. A deleted `syncId` can
never be reused; to sync again the user generates a new key, which gives a new
`syncId`, and pushes.

### 3.7 `GET /healthz`

Checks the database connection.

| Status | Meaning                          |
| ------ | -------------------------------- |
| `200`  | The server and database are up. |
| `503`  | The database can't be reached.   |

### 3.8 Client error mapping

The client maps every outcome of `pull()`, `push()` and `deleteRemote()` to one
of these errors:

| Error            | When                                                                  |
| ---------------- | --------------------------------------------------------------------- |
| `offline`        | Network failure, DNS failure, timeout, connection refused.           |
| `conflict`       | `409` (carries the server's `version`).                               |
| `forbidden`      | `403`.                                                                |
| `not-found`      | `404`.                                                                |
| `gone`           | `410`.                                                                |
| `decrypt-failed` | The blob fails to decrypt or parse (section 2.4).                     |
| `bad-key`        | The pasted key fails to decode (section 2.2).                         |
| `server-error`   | Any other status, including `400` and `5xx`; carries the status code. |

A `GET` that returns `404` is not an error for `pull()`: it means there is no
remote data yet, and the client creates it on its next push.

## 4. Plaintext document

### 4.1 Shape

```jsonc
{
  "format": 1,
  "tables": {
    "settings":     { "<key>":      { "value": <json>, "updatedAt": <ms> } },
    "agents":       { "<name>":     { /* agent fields */ "updatedAt": <ms> } },
    "tags":         { "<name>":     { "position": <int>, "updatedAt": <ms> } },
    "repos":        { "<repoKey>":  { /* repo fields */ "updatedAt": <ms> } },
    "backlogItems": { "<itemId>":   { /* item fields */ "updatedAt": <ms> } }
  },
  "tombstones": {
    "settings":     { "<key>": <ms> },
    "agents":       { "<name>": <ms> },
    "tags":         { "<name>": <ms> },
    "backlogItems": { "<itemId>": <ms> }
  }
}
```

- `format` is the document format version. A client **refuses to merge or push**
  when the remote `format` is greater than the highest it understands, and tells
  the user to update blackwall.
- Timestamps (`createdAt`, `updatedAt`, tombstone times) are integers: ms since
  the Unix epoch, as blackwall stores them.
- Each table is an object keyed by the row's **sync key**:

  | Table          | Sync key                               |
  | -------------- | -------------------------------------- |
  | `settings`     | settings key                           |
  | `agents`       | agent `name` (unique)                  |
  | `tags`         | tag `name`                             |
  | `repos`        | repo key (section 4.3)                 |
  | `backlogItems` | backlog item `id` (a UUID, the same on every machine) |

- **Canonical JSON** (used for hashes in sections 5 and 7): object keys sorted by
  UTF-16 code unit order at every level, no insignificant whitespace, arrays in
  their stored order, numbers in JavaScript's shortest form, strings as
  `JSON.stringify` writes them. The **document hash** is `hex(SHA-256(canonical
  JSON of the document))`.

### 4.2 Rows

Agents are referenced **by name** everywhere in the document (agent ids differ
per machine), and repos by **repo key**. Apply maps both back to local ids.

**settings** — only keys on the allowlist (section 4.5). `value` is the stored
JSON value; references to agents inside `roleDefaults` are agent names.

**agents** — every agent column except `id`:
`name`, `models`, `efforts`, `prompt`, `eligibleStages`, `permissionOverrides`,
`createdAt`, `updatedAt`.

**tags** — `name` (the key), `position`, `updatedAt`. (`tags.updated_at` is added
in blackwall by follow-up 3; until then tags have no `updatedAt`.)

**repos** — every repo column except `id`, `path` and `connectionId`:
`name`, `remoteUrl`, `checkCommands`, `setupCommands`, `defaultCli`,
`defaultOrigin`, `maxBacklogTasks`, `defaultBranch`, `favorite`, `position`,
`watchCi`, `jiraProject`, `referenceRepoIds` (as repo keys), `agentTrust`,
`trustProjectMcp`, `trustProjectHooks`, `createdAt`, `updatedAt`.

**backlogItems** — every backlog item column, with these changes:

- `repoId` is replaced by `repo`: the repo key.
- `referenceRepoIds` is a list of repo keys, or `null`.
- `agents` (`RoleAgents`) and `agentClis` (keyed by agent) use agent names.
- `taskId` and `sourceTaskId` are always `null`.
- `origin.connectionId` is always `null` in the `jira-issue`, `github-pr` and
  `github-issue` variants.

The fields are `id` (the key), `repo`, `title`, `origin`, `mode`, `freeText`,
`agents`, `agentClis`, `referenceRepoIds`, `triageMode`, `position`,
`dependsOn` (backlog item ids), `status`, `taskId`, `sourceTaskId`, `createdAt`,
`updatedAt`.

### 4.3 Repo keys

A repo is identified across machines by its normalised remote URL:

- Form: `host/owner/repo`, lowercase, without a trailing `.git` or `/`.
- SSH and HTTPS forms of the same remote give the same key:
  - `git@github.com:Owner/Repo.git` → `github.com/owner/repo`
  - `ssh://git@github.com/Owner/Repo.git` → `github.com/owner/repo`
  - `https://github.com/Owner/Repo` → `github.com/owner/repo`
- User info and ports are dropped from the host. Paths with more than two
  segments (for example GitLab groups) keep every segment:
  `host/group/sub/repo`.
- A changed remote URL gives a new key and doesn't reconnect to the old synced
  data.

### 4.4 Local-only data

These columns are **never** in the document (or are always `null` in it), and
apply **never** overwrites them:

| Table          | Local-only columns                                                     |
| -------------- | ---------------------------------------------------------------------- |
| `repos`        | `id`, `path`, `connectionId`                                           |
| `agents`       | `id` (agents are matched by name)                                      |
| `backlogItems` | `taskId`, `sourceTaskId`, `origin.connectionId` (written as `null`)    |

- `origin.connectionId`: the snapshot writes `null`, so it plays no part in
  last-writer-wins or the document hash. Apply keeps the local value on an
  existing item and stores `null` on a new one. Apply doesn't re-resolve it. At
  runtime a `null` resolves by URL host for GitHub and to the single Jira
  connection for Jira.
- A repo's `connectionId` is resolved locally: a synced repo becomes a local row
  only when a clone with that remote is registered, and registration resolves
  the connection for the remote URL.

**Repos without a remote are local-only.** `repos.remoteUrl` is nullable, and a
repo with no remote has no repo key, so:

- Neither the repo nor its backlog items are in the document. Deleting such an
  item writes no tombstone.
- The snapshot drops references to such a repo from every `referenceRepoIds`.
- On apply, a merged `referenceRepoIds` is the remote's repo keys mapped to
  local ids (keys of repos not registered here go to the pending store, section
  6.5), **plus** the local row's existing references to repos without a remote.
  A merge never loses a local-only reference, and those references never affect
  the hash.
- If the repo later gains a remote, it and its backlog enter the next snapshot as
  new rows.

**Never-synced tables:** `tasks`, `runs`, `steps`, `step_events`, `findings`,
`grill_questions`, `chat_messages`, `connections`, secrets (the OS keyring,
including the sync key), `run_confirmations`, `sync_tombstones` (its content
travels as `tombstones`, not as a table) and the pending store.

### 4.5 Settings

Only allowlisted keys are synced. A key that is not on the allowlist is never
sent, and is ignored if received. Adding a key means updating this contract.

**Synced (allowlist):**

| Key                          | Notes                                   |
| ---------------------------- | --------------------------------------- |
| `roleDefaults`               | Agent references as names.              |
| `maxTasksPerRepo`            |                                         |
| `maxBacklogTasksPerRepo`     |                                         |
| `stopBacklogOnNeedsYou`      |                                         |
| `githubIssuesRefreshSeconds` |                                         |

**Never synced (machine-local):**

| Key                         | Why                                                         |
| --------------------------- | ----------------------------------------------------------- |
| `backlogRunner`             | Global runner switch; each machine decides whether it runs. |
| `backlogRunner:<repoId>`    | Per-repo runner state, including `autonomous` and halts.    |
| `voiceModel`, `voiceHotkey` | Per-machine voice setup.                                    |
| `quota`                     | Per-machine CLI quota state.                                |
| `seeded.*`                  | Per-machine seeding and migration markers.                  |
| `roleDefaults.copilot`      | Legacy key.                                                 |
| `sync.*`                    | Sync's own local state (server URL, version, last hash).    |

The key itself lives in the keyring, never in settings.

### 4.6 Backlog items and runners

- Per-repo runner state stays local (section 4.5).
- The backlog runner never promotes an item whose status is already `promoted`.
- Apply never overwrites the local `taskId` or `sourceTaskId`.
- A synced item with status `promoted` and no local `taskId` is shown as
  **"running elsewhere"** and can't be promoted on that machine.
- If two machines promote the same item within one sync window, both run it.
  v1 accepts this (section 8).

## 5. Run confirmation

Commands, trust and agent instructions sync like any other field, but a machine
runs nothing that came from another machine until a user there confirms it.

### 5.1 Subjects and hashes

| Subject | Fields in the hash                                                                  |
| ------- | ----------------------------------------------------------------------------------- |
| `repo`  | `checkCommands`, `setupCommands`, `agentTrust`, `trustProjectMcp`, `trustProjectHooks` |
| `agent` | `prompt`, `permissionOverrides`                                                     |

- The subject hash is `hex(SHA-256(canonical JSON of an object holding exactly
  those fields))`, with canonical JSON as in section 4.1.
- A new security-relevant repo or agent field must be added to its hash.

### 5.2 Local table

Each machine keeps a local, **never-synced** table:

```
run_confirmations(
  subject_kind  'repo' | 'agent',
  subject_id    local repo or agent id,
  confirmed_hash,
  confirmed_at,
  PRIMARY KEY (subject_kind, subject_id)
)
```

- Its migration backfills the current hash of every existing repo and agent, so
  nothing prompts after upgrading.

### 5.3 Who writes it

Local edits record the new hash, since the user made them on this machine:

- registering a repo, and editing a repo through the repo service;
- creating and updating an agent, merging per-CLI twin agents, seeding agents,
  and upgrading seed prompts.

Removing a repo or deleting an agent deletes its row.

**Sync apply never writes `run_confirmations`.** An agent or repo that apply
creates or changes stays unconfirmed until a user confirms it on this machine. A
synced agent whose content equals the local content (for example an unedited
seed agent on a new machine) has the same hash, so after the name match it
needs no prompt. A repo restored from the pending store (section 6.5) needs
confirmation, because no local edit recorded its hash.

### 5.4 Gates

When a subject's current hash differs from its confirmed hash (or it has no
row), the task stops as **needs-you**, with a reason that names the subject and
shows the fields concerned. When the user confirms, the hash is recorded and the
task continues.

- **Repo gate:** before running setup commands, before running check commands,
  and before starting an agent with any trust level other than `locked`.
- **Agent gate:** in the pipeline's `runAgent`, before the invocation that
  builds the agent's system prompt and permissions. Every agent step on every
  stage and every CLI passes through it. Any future way to start an agent that
  bypasses `runAgent` must add this gate.

## 6. Merge and apply

### 6.1 Snapshot

The client builds the document from its local database by the rules of
section 4: allowlisted settings, all agents and tags, repos with a remote and
their backlog items, plus pending-store rows (section 6.5) and all tombstones.

### 6.2 Last writer wins

Merging local and remote is per row, keyed by the sync key, on `updatedAt`:

- Present on one side only: it wins, unless a tombstone for the same key on the
  other side is at least as new (`deletedAt >= updatedAt`), in which case the
  row is deleted.
- Present on both: the greater `updatedAt` wins. On a tie, the row whose
  canonical JSON is greater (by UTF-16 code unit order) wins, so every machine
  picks the same row.
- Tombstones from both sides are unioned; for the same key the later `deletedAt`
  is kept. A row whose `updatedAt` is newer than its tombstone wins (it was
  re-created after the delete).

### 6.3 Tombstones

- Deletes are recorded locally in `sync_tombstones(table, key, deleted_at)` and
  kept forever.
- Deleting an agent, a tag, an allowlisted setting, or a backlog item of a repo
  with a remote writes a tombstone.
- Deleting a backlog item of a repo **without** a remote writes none.
- **Removing (unregistering) a repo writes none** (section 6.5). Repos have no
  tombstones in v1.

### 6.4 Apply

Apply writes the merged result into the local database in **one transaction**:

- It writes each merged row with its document `updatedAt`, and each tombstone
  with its `deleted_at`, directly through the database, bypassing helpers that
  stamp the current time (`$onUpdateFn`, `settings.set`) and service methods
  (`settings.set`, `repos.update`, `agents.update`, `tags.replace`).
- It doesn't rewrite rows equal to the local copy.
- It never touches local-only columns (section 4.4) or `run_confirmations`.
- It maps agent names to local agent ids (creating agents that don't exist
  locally) and repo keys to local repo ids, including inside `RoleAgents`,
  `agentClis`, `roleDefaults` and `referenceRepoIds`.
- Rows for repo keys with no local clone go to the pending store.

Apply must be idempotent: pull → apply → snapshot gives the remote's document
hash. Otherwise every apply would trigger a push-back loop.

### 6.5 Unregister and reconnect (local only)

Unregistering a repo on one machine doesn't delete it from sync:

- It drops the local clone registration and writes **no** tombstone. The synced
  data stays in the document.
- In the same transaction, before the repo's backlog items are cascade-deleted,
  the repo's synced row and its backlog items move to a local **pending store**,
  keyed by repo key.
- References to the repo from other repos' `referenceRepoIds` are kept as
  pending repo-key references. Stripping the local id from those repos doesn't
  bump their `updatedAt`.
- The same applies when a stale repo is removed automatically.
- The pending store is part of the snapshot, so unregistering leaves the
  document hash unchanged.
- **Reconnect:** registering a clone whose remote gives the same repo key
  restores the synced fields, backlog items and references with their document
  `updatedAt`. Its `connectionId` is resolved locally. It then needs run
  confirmation (section 5.3).
- Repos without a remote are removed as before; they were never synced.

### 6.6 First join

When a machine with existing data enters a key whose `syncId` already has data,
it **always merges both sides, without a prompt**:

1. Agents are matched by name; remote references are remapped to local ids.
2. Repos are matched by repo key.
3. Normal last-writer-wins (section 6.2) applies.
4. The merged result is pushed.

## 7. Client behaviour

- **Endpoint:** no default. The server URL is a local setting entered in
  Settings → Sync.
- **Pull** on start, on window focus and every 60 s.
- **Push** when the document hash changes, debounced to about 2 s, plus once on
  shutdown. The first push for a new key has no `If-Match`.
- **On `409`:** pull, merge, retry; at most 3 times, then report `conflict`.
- **On `410`:** stop syncing and show "Server data was deleted; sync stopped.
  Generate a new key to sync again."
- **On `403`:** stop pushing and report `forbidden`.
- **Format gate:** a remote `format` newer than the client understands stops
  merge and push (section 4.1).
- **Disconnect** forgets the key and the sync state on this machine only, after
  an "Are you sure?" prompt. Server data and other machines are untouched.
- **Delete server data** (Danger zone) asks for confirmation, calls `DELETE`, and
  then forgets the key locally. Other machines then get `410` (above).

## 8. Threat model

- **The server operator** sees only `syncId`s, blob sizes, versions and request
  timing. Blobs are AES-256-GCM encrypted with a key the server never receives.
- **The key is the only credential.** Anyone holding it can read and change all
  synced data. A lost key can't be recovered.
- **A leaked key** lets an attacker change synced check and setup commands, repo
  trust, agent prompts and agent permissions, but **nothing runs on a machine
  until a user there confirms the change** (section 5). A code path that skips a
  gate would turn a leaked key into remote command execution, so every gate is
  tested.
- Non-security fields (backlog content, tags, allowlisted settings, other repo
  and agent fields) can still be changed by a key holder without a prompt.
- **Without the key** nobody can overwrite or delete a blob: that needs the
  write token, which is derived from `encKey` and never stored by the server.
- **No limits:** anyone who can reach the server can store large blobs or flood
  it with requests, and each made-up `syncId` creates a new row. Accepted for v1.
  Postgres `bytea` caps a value at about 1 GB. A reverse proxy in front of the
  server may reject large `PUT`s (nginx defaults to 1 MB); those surface as
  `server-error`, so the deployment must raise the proxy limit.
- **Deletion markers** are kept forever, as tiny rows.
- **Clock skew** between machines can make an older edit win. Accepted for v1.
- **Two machines promoting the same item** in one sync window both run it.
  Accepted for v1.

## 9. Decisions

| #    | Question                                    | Decision                                                                                                         |
| ---- | ------------------------------------------- | ---------------------------------------------------------------------------------------------------------------- |
| 1    | Backend runtime and hosting                 | Rust on PostgreSQL, self-hosted on the user's own server and domain. Neither exists yet, so the client ships **without a default endpoint**; the user enters the URL in Settings → Sync. A later change adds a default; deployment is a later follow-up. |
| 2    | Key display format                          | base58 (Bitcoin alphabet) over `key || SHA-256(key)[0..4]`, about 50 characters; whitespace trimmed; a wrong checksum gives `bad-key`. |
| 3    | Syncing security-relevant repo fields       | They sync, but each machine confirms them before first use (section 5).                                         |
| 4    | Per-repo runner state                       | Stays local, including `autonomous`.                                                                             |
| 5    | Promoted items                              | Synced with status; shown as "running elsewhere" without a local `taskId`; the runner never promotes a `promoted` item; a double promotion in one sync window is accepted for v1. |
| 6    | First join                                  | Always merge both sides, without a prompt.                                                                       |
| 7    | Deleting server data                        | `DELETE` erases the blob and leaves a deletion marker; every later request on that `syncId` gets `410`; Disconnect is local only. |
| 8    | Server-side limits                          | None: no body cap, no rate limits, no `413`/`429`.                                                               |
| RQ2  | Do synced agents need local confirmation?   | Yes. The agent gate covers both `prompt` and `permissionOverrides`.                                              |
| —    | Unregistering a repo                        | Local only: no tombstone; the synced data stays and a clone with the same remote reconnects it.                  |
