#!/bin/sh
set -eu

SERVER_BIN="/usr/local/bin/server"
DATA_DIR="${CCP_SERVER_DATA_DIR:-/var/lib/ccp/server}"
AUTH_PORT="${CCP_AUTH_PORT:-1337}"
MTLS_PORT="${CCP_MTLS_PORT:-1338}"
ADVERTISE_HOST="${CCP_ADVERTISE_HOST:-127.0.0.1}"

mkdir -p "$DATA_DIR"

add_tls_name() {
    name="$1"
    case ",$TLS_SERVER_NAMES," in
        *,"$name",*) ;;
        *) TLS_SERVER_NAMES="${TLS_SERVER_NAMES},${name}" ;;
    esac
}

if [ "${1:-}" = "server" ]; then
    shift
    exec "$SERVER_BIN" "$@"
fi

if [ "${1:-}" = "issue-token" ] || [ "${1:-}" = "health" ]; then
    exec "$SERVER_BIN" "$@"
fi

SESSION_NAME="${1:-${CCP_SESSION_NAME:-}}"
if [ -z "$SESSION_NAME" ]; then
    echo "Provide a session name as the first argument or set CCP_SESSION_NAME." >&2
    exit 64
fi

AUTH_LISTENER_ADDR="${CCP_AUTH_LISTENER_ADDR:-0.0.0.0:${AUTH_PORT}}"
MTLS_LISTENER_ADDR="${CCP_MTLS_LISTENER_ADDR:-0.0.0.0:${MTLS_PORT}}"
AUTH_BASE_URL="${CCP_AUTH_BASE_URL:-http://${ADVERTISE_HOST}:${AUTH_PORT}}"
MTLS_BASE_URL="${CCP_MTLS_BASE_URL:-https://${ADVERTISE_HOST}:${MTLS_PORT}}"
TLS_SERVER_NAMES="${CCP_TLS_SERVER_NAMES:-localhost,127.0.0.1}"

add_tls_name "$ADVERTISE_HOST"

export CCP_SERVER_DATA_DIR="$DATA_DIR"
export CCP_AUTH_LISTENER_ADDR="$AUTH_LISTENER_ADDR"
export CCP_MTLS_LISTENER_ADDR="$MTLS_LISTENER_ADDR"
export CCP_AUTH_BASE_URL="$AUTH_BASE_URL"
export CCP_MTLS_BASE_URL="$MTLS_BASE_URL"
export CCP_TLS_SERVER_NAMES="$TLS_SERVER_NAMES"
export CCP_ALLOW_NON_LOOPBACK_AUTH_LISTENER="${CCP_ALLOW_NON_LOOPBACK_AUTH_LISTENER:-1}"

exec "$SERVER_BIN" "$SESSION_NAME"
