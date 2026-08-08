#!/bin/sh
set -eu

SERVER_BIN="/usr/local/bin/server"
DATA_DIR="${CCP_SERVER_DATA_DIR:-/var/lib/ccp/server}"
HTTP_PORT="${CCP_HTTP_PORT:-1338}"
ADVERTISE_HOST="${CCP_ADVERTISE_HOST:-192.168.130.34}"

mkdir -p "$DATA_DIR"

if [ "${1:-}" = "server" ]; then
    shift
    exec "$SERVER_BIN" "$@"
fi

if [ "${1:-}" = "create-session" ]; then
    exec "$SERVER_BIN" "$@"
fi

SESSION_NAME="${1:-${CCP_SESSION_NAME:-}}"
if [ -z "$SESSION_NAME" ]; then
    echo "Provide a session name as the first argument or set CCP_SESSION_NAME." >&2
    exit 64
fi

HTTP_LISTENER_ADDR="${CCP_HTTP_LISTENER_ADDR:-0.0.0.0:${HTTP_PORT}}"
HTTP_BASE_URL="${CCP_HTTP_BASE_URL:-http://${ADVERTISE_HOST}:${HTTP_PORT}}"

export CCP_SERVER_DATA_DIR="$DATA_DIR"
export CCP_HTTP_LISTENER_ADDR="$HTTP_LISTENER_ADDR"
export CCP_HTTP_BASE_URL="$HTTP_BASE_URL"

exec "$SERVER_BIN" "$SESSION_NAME"
