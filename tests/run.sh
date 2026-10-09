#!/usr/bin/env bash
# HTTP integration tests for the real server and CLI.
# Copyright (C) 2026 Squid Proxy Lovers
# SPDX-License-Identifier: AGPL-3.0-or-later
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SERVER_BIN="$REPO_ROOT/target/release/server"
CLIENT_BIN="$REPO_ROOT/target/release/client"
SKIP_BUILD=false
for arg in "$@"; do
    case "$arg" in
        --skip-build) SKIP_BUILD=true ;;
        *) echo "Usage: bash tests/run.sh [--skip-build]" >&2; exit 64 ;;
    esac
done
command -v python3 >/dev/null
command -v curl >/dev/null
if [ "$SKIP_BUILD" = false ]; then
    "${CARGO:-cargo}" build --locked --release -p server -p client --manifest-path "$REPO_ROOT/Cargo.toml"
fi
[ -x "$SERVER_BIN" ] && [ -x "$CLIENT_BIN" ] || { echo "Build server and client first" >&2; exit 1; }

WORK_DIR=$(mktemp -d)
SERVER_PID=""
cleanup() {
    if [ -n "$SERVER_PID" ]; then
        kill "$SERVER_PID" 2>/dev/null || true
        wait "$SERVER_PID" 2>/dev/null || true
    fi
    rm -rf "$WORK_DIR"
}
trap cleanup EXIT
PASS_COUNT=0
FAIL_COUNT=0
OUT=""
pass() { echo "PASS $1"; PASS_COUNT=$((PASS_COUNT + 1)); }
fail() { echo "FAIL $1" >&2; FAIL_COUNT=$((FAIL_COUNT + 1)); }
# Capture only command output. Counts are updated in the parent shell, never
# inside command substitution, and both success and failure statuses are tested.
expect_success() {
    local description="$1"; shift
    if OUT=$("$@" 2>&1); then pass "$description";
    else fail "$description"; printf '%s\n' "$OUT" >&2; fi
}
expect_failure() {
    local description="$1"; shift
    if OUT=$("$@" 2>&1); then fail "$description (unexpected success)";
    else pass "$description"; fi
}
assert_json() {
    local description="$1" expression="$2"
    if printf '%s' "$OUT" | python3 -c 'import json, sys; value=json.load(sys.stdin); assert eval(sys.argv[1], {"value": value})' "$expression"; then
        pass "$description"
    else fail "$description"; fi
}

HTTP_PORT=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()')
export CCP_SERVER_URL="http://127.0.0.1:$HTTP_PORT"
export CCP_SERVER_DATA_DIR="$WORK_DIR/data"
export CCP_CLIENT_HOME="$WORK_DIR/client"
export CCP_CLIENT_KEY="integration-client-$HTTP_PORT"
export CCP_ADMIN_KEY="integration-admin-$HTTP_PORT"
export CCP_HTTP_BASE_URL="$CCP_SERVER_URL"
export CCP_HTTP_LISTENER_ADDR="127.0.0.1:$HTTP_PORT"
"$SERVER_BIN" integration-test >"$WORK_DIR/server.log" 2>&1 &
SERVER_PID=$!
READY=false
for _ in $(seq 1 100); do
    if OUT=$(curl -fsS --max-time 1 "$CCP_SERVER_URL/health" 2>/dev/null) &&
        printf '%s' "$OUT" | python3 -c 'import json,sys; assert json.load(sys.stdin)["status"] == "ok"' 2>/dev/null; then
        READY=true; break
    fi
    if ! kill -0 "$SERVER_PID" 2>/dev/null; then break; fi
    sleep 0.1
done
if [ "$READY" = false ]; then cat "$WORK_DIR/server.log" >&2; exit 1; fi
pass "HTTP server ready"

expect_failure "unsubscribed session is rejected" "$CLIENT_BIN" list integration-test
expect_success "discover sessions" "$CLIENT_BIN" remote-sessions
assert_json "initial topic discoverable" 'any(s["session_name"] == "integration-test" for s in value)'
expect_success "subscribe" "$CLIENT_BIN" subscribe integration-test --server "$CCP_SERVER_URL"
expect_success "list local subscriptions" "$CLIENT_BIN" sessions
if [[ "$OUT" == *"session=integration-test "* ]]; then pass "local topic recorded";
else fail "local topic recorded"; fi
expect_success "subscribe all" "$CLIENT_BIN" subscribe-all --server "$CCP_SERVER_URL"
expect_success "empty list" "$CLIENT_BIN" list integration-test
assert_json "list is empty" 'value == []'
expect_success "public topic accepts ordinary client requests" env CCP_CLIENT_KEY=wrong "$CLIENT_BIN" list integration-test

expect_success "add shelf" "$CLIENT_BIN" add-shelf integration-test research "collected research"
expect_success "add book" "$CLIENT_BIN" add-book integration-test --shelf research findings "key findings"
expect_success "add entry" "$CLIENT_BIN" add-entry integration-test --shelf research --book findings --labels test,integration day1 "first entry" "initial integration content"
expect_success "get entry" "$CLIENT_BIN" get integration-test day1 --shelf research --book findings
assert_json "get preserves content and labels" '"initial integration content" in value["context"] and "test" in value["labels"]'
expect_success "append" "$CLIENT_BIN" append integration-test day1 --shelf research --book findings "appended follow-up"
expect_success "history" "$CLIENT_BIN" history integration-test day1 --shelf research --book findings
assert_json "history includes append" 'any("appended follow-up" in h["appended_content"] for h in value)'
expect_success "list entry" "$CLIENT_BIN" list integration-test
assert_json "list names entry" 'any(e["name"] == "day1" for e in value)'
expect_success "search entries" "$CLIENT_BIN" search-entries integration-test day1
assert_json "entry search returns match" 'any(e["name"] == "day1" for e in value)'
expect_success "search context" "$CLIENT_BIN" search-context integration-test "integration content"
assert_json "context search returns match" 'len(value) > 0'
expect_success "search shelves" "$CLIENT_BIN" search-shelves integration-test research
assert_json "shelf search returns match" 'len(value) == 1'
expect_success "search books" "$CLIENT_BIN" search-books integration-test findings
assert_json "book search returns match" 'len(value) == 1'
expect_success "search miss" "$CLIENT_BIN" search-entries integration-test missing-name-zzzz
assert_json "search miss is empty" 'value == []'

expect_success "set status" "$CLIENT_BIN" set-status integration-test --team research --agent scout-one "Investigating parser performance"
assert_json "status reports expiry" 'value["agent_name"] == "scout-one" and value["expires_at"] > value["updated_at"]'
expect_success "set second status" "$CLIENT_BIN" set-status integration-test --team research --agent scout-two "Writing tests"
expect_success "list team status" "$CLIENT_BIN" team-status integration-test --team research
assert_json "both agents visible" '{s["agent_name"] for s in value} == {"scout-one", "scout-two"}'
expect_success "search status" "$CLIENT_BIN" search-team-status integration-test --team research "PARSER PERFORMANCE"
assert_json "status search is case insensitive" 'len(value) == 1 and value[0]["agent_name"] == "scout-one"'
expect_success "clear status" "$CLIENT_BIN" clear-status integration-test --team research --agent scout-two
assert_json "status removed" 'value["cleared"] is True'
expect_success "clear status twice" "$CLIENT_BIN" clear-status integration-test --team research --agent scout-two
assert_json "clear is idempotent" 'value["cleared"] is False'

expect_success "delete entry" "$CLIENT_BIN" delete integration-test day1 --shelf research --book findings
ENTRY_KEY=$(printf '%s' "$OUT" | python3 -c 'import json,sys; print(json.load(sys.stdin)["entry_key"])')
expect_failure "deleted entry cannot be read" "$CLIENT_BIN" get integration-test day1 --shelf research --book findings
expect_success "search archived entries" "$CLIENT_BIN" search-deleted integration-test day1
assert_json "archived entry found" 'len(value) == 1'
expect_success "restore entry" "$CLIENT_BIN" restore integration-test "$ENTRY_KEY"
expect_success "get restored entry" "$CLIENT_BIN" get integration-test day1 --shelf research --book findings
assert_json "restore preserves appended content" '"appended follow-up" in value["context"]'

EXPORT_FILE="$WORK_DIR/export.droplet"
expect_success "export session" "$CLIENT_BIN" export integration-test --output "$EXPORT_FILE"
if python3 - "$EXPORT_FILE" <<'PY'
import json, sys
with open(sys.argv[1]) as stream:
    assert json.load(stream)['bundle_sha256']
PY
then pass "export contains checksum"; else fail "export checksum missing"; fi
expect_success "export shelf" "$CLIENT_BIN" export integration-test --shelf research --output "$WORK_DIR/shelf.droplet"
expect_success "export book" "$CLIENT_BIN" export integration-test --shelf research --book findings --output "$WORK_DIR/book.droplet"
expect_success "export without history" "$CLIENT_BIN" export integration-test --no-history --output "$WORK_DIR/no-history.droplet"
expect_success "import skip" "$CLIENT_BIN" import integration-test "$EXPORT_FILE" --policy skip
expect_failure "import error rejects duplicates" "$CLIENT_BIN" import integration-test "$EXPORT_FILE" --policy error
expect_success "delete before import" "$CLIENT_BIN" delete integration-test day1 --shelf research --book findings
expect_success "import overwrite" "$CLIENT_BIN" import integration-test "$EXPORT_FILE" --policy overwrite
expect_success "get imported entry" "$CLIENT_BIN" get integration-test day1 --shelf research --book findings
assert_json "import restores content" '"appended follow-up" in value["context"]'
expect_success "briefing" "$CLIENT_BIN" brief-me integration-test
CURRENT_TS=$(python3 -c 'import time; print(int(time.time()))')
expect_success "entry at current time" "$CLIENT_BIN" get-entry-at integration-test day1 --at "$CURRENT_TS" --shelf research --book findings
assert_json "temporal read contains current data" '"initial integration content" in value["context"]'
expect_success "master instructions" "$CLIENT_BIN" master-instructions integration-test

expect_success "add temporary shelf" "$CLIENT_BIN" add-shelf integration-test throwaway temp
expect_success "add temporary book" "$CLIENT_BIN" add-book integration-test --shelf throwaway temporary temp
expect_success "add temporary entry" "$CLIENT_BIN" add-entry integration-test --shelf throwaway --book temporary temporary-entry temp temp
expect_success "delete shelf" "$CLIENT_BIN" delete-shelf integration-test throwaway
expect_success "search deleted shelf" "$CLIENT_BIN" search-shelves integration-test throwaway
assert_json "shelf removed" 'value == []'
expect_success "delete local subscription" "$CLIENT_BIN" delete-session integration-test
expect_failure "deleted subscription cannot be used" "$CLIENT_BIN" list integration-test

kill "$SERVER_PID"
for _ in $(seq 1 100); do
    if ! kill -0 "$SERVER_PID" 2>/dev/null; then break; fi
    sleep 0.1
done
if kill -0 "$SERVER_PID" 2>/dev/null; then
    fail "server shutdown timed out"; kill -KILL "$SERVER_PID" 2>/dev/null || true
fi
if wait "$SERVER_PID"; then pass "server shutdown completed";
else fail "server exited with error"; cat "$WORK_DIR/server.log" >&2; fi
SERVER_PID=""
printf '\n%d passed; %d failed\n' "$PASS_COUNT" "$FAIL_COUNT"
[ "$FAIL_COUNT" -eq 0 ]
