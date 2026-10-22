#!/bin/sh
# Dumps the database to /backups now and then every BACKUP_INTERVAL_SECONDS,
# and prunes dumps older than BACKUP_RETENTION_DAYS. Connection settings come
# from the PG* environment variables. A failed pass is logged and the loop
# carries on.
set -eu

: "${BACKUP_RETENTION_DAYS:=14}"
: "${BACKUP_INTERVAL_SECONDS:=86400}"

while true; do
    # Only this container writes to /backups and passes never overlap, so any
    # .tmp file here is left over from a run that was killed mid-dump.
    rm -f /backups/blackwall-*.dump.tmp

    ts=$(date -u +%Y%m%dT%H%M%SZ)
    tmp=/backups/blackwall-$ts.dump.tmp
    final=/backups/blackwall-$ts.dump

    if pg_dump -Fc -f "$tmp"; then
        mv "$tmp" "$final"
        echo "backup: wrote $final"
    else
        rm -f "$tmp"
        echo "backup: pg_dump failed" >&2
    fi

    find /backups -name 'blackwall-*.dump' -mtime +"$BACKUP_RETENTION_DAYS" -delete \
        || echo "backup: prune failed" >&2

    sleep "$BACKUP_INTERVAL_SECONDS"
done
