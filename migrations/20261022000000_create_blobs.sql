CREATE TABLE blobs (
    sync_id          text        PRIMARY KEY,
    version          bigint      NOT NULL,
    blob             bytea,
    write_token_hash bytea,
    deleted_at       timestamptz,
    created_at       timestamptz NOT NULL DEFAULT now(),
    updated_at       timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT blobs_live_or_deleted CHECK (
        (deleted_at IS NULL AND blob IS NOT NULL AND write_token_hash IS NOT NULL)
        OR (deleted_at IS NOT NULL AND blob IS NULL AND write_token_hash IS NULL)
    )
);
