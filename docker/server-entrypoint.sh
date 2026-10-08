#!/bin/sh
set -eu

SERVER_BIN="${CCP_SERVER_BIN:-/usr/local/bin/server}"
DATA_DIR="${CCP_SERVER_DATA_DIR:-/var/lib/ccp/server}"
HTTP_PORT="${CCP_HTTP_PORT:-1338}"
ADVERTISE_HOST="${CCP_ADVERTISE_HOST:-127.0.0.1}"

mkdir -p "$DATA_DIR"
export CCP_SERVER_DATA_DIR="$DATA_DIR"
export CCP_HTTP_LISTENER_ADDR="${CCP_HTTP_LISTENER_ADDR:-0.0.0.0:${HTTP_PORT}}"
export CCP_HTTP_BASE_URL="${CCP_HTTP_BASE_URL:-http://${ADVERTISE_HOST}:${HTTP_PORT}}"

if [ "${1:-}" = "server" ]; then
    shift
    exec "$SERVER_BIN" "$@"
fi
if [ "$#" -gt 0 ]; then
    exec "$SERVER_BIN" "$@"
fi
if [ -n "${CCP_SESSION_NAME:-}" ]; then
    exec "$SERVER_BIN" "$CCP_SESSION_NAME"
fi
exec "$SERVER_BIN"
