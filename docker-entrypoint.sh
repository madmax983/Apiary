#!/bin/sh
# A storm briefly holds ~600 hive files open (max_open is a soft ceiling, see
# docs/FINDINGS.md), each with its WAL sidecars and pooled connections. Raise
# the soft open-file limit as far as the hard limit allows.
set -e
ulimit -n "$(ulimit -Hn)" 2>/dev/null || true
echo "apiary: open-file limit $(ulimit -n)"
mkdir -p /data/apiary-fleet /data/apiary-archive
exec "$@"
