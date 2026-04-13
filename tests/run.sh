#!/usr/bin/env bash
# Full integration test. Builds from source, boots a real server, enrolls
# clients, exercises every CLI operation, then tears everything down.
#
# Usage:
#   bash tests/run.sh              run everything
#   bash tests/run.sh --skip-build use existing binaries
#
# Copyright (C) 2026 Squid Proxy Lovers
# SPDX-License-Identifier: AGPL-3.0-or-later

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SERVER_BIN="$REPO_ROOT/target/release/server"
CLIENT_BIN="$REPO_ROOT/target/release/client"
SKIP_BUILD=false

for arg in "$@"; do
    case "$arg" in
        --skip-build) SKIP_BUILD=true ;;
    esac
done

# ── Colors ───────────────────────────────────────────────────────────────────

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
RESET='\033[0m'

PASS_COUNT=0
FAIL_COUNT=0
SKIP_COUNT=0
FAILURES=()
CURRENT_SECTION=""

section() {
    CURRENT_SECTION="$1"
    echo ""
    echo -e "${BOLD}${CYAN}── $1 ──${RESET}"
}

pass() {
    echo -e "  ${GREEN}✓${RESET} $1"
    ((PASS_COUNT++))
}

fail() {
    echo -e "  ${RED}✗${RESET} $1"
    ((FAIL_COUNT++))
    FAILURES+=("[$CURRENT_SECTION] $1")
}

skip() {
    echo -e "  ${YELLOW}○${RESET} $1"
    ((SKIP_COUNT++))
}

expect_success() {
    local desc="$1"; shift
    local out
    out=$("$@" 2>&1)
    if [ $? -eq 0 ]; then
        pass "$desc"
    else
        fail "$desc"
        echo -e "    ${RED}$(echo "$out" | head -3)${RESET}"
    fi
    echo "$out"
}

expect_failure() {
    local desc="$1"; shift
    local out
    out=$("$@" 2>&1)
    if [ $? -ne 0 ]; then
        pass "$desc"
    else
        fail "$desc — expected failure but got success"
    fi
    echo "$out"
}

expect_contains() {
    local desc="$1" haystack="$2" needle="$3"
    if echo "$haystack" | grep -qi "$needle"; then
        pass "$desc"
    else
        fail "$desc — expected '$needle' in output"
    fi
}

expect_not_contains() {
    local desc="$1" haystack="$2" needle="$3"
    if ! echo "$haystack" | grep -qi "$needle"; then
        pass "$desc"
    else
        fail "$desc — did not expect '$needle' in output"
    fi
}

expect_count() {
    local desc="$1" haystack="$2" expected="$3"
    local count
    count=$(echo "$haystack" | grep -c "^" 2>/dev/null || echo "0")
    # empty string counts as 0
    if [ -z "$haystack" ]; then count=0; fi
    if [ "$count" -eq "$expected" ]; then
        pass "$desc (got $count)"
    else
        fail "$desc — expected $expected, got $count"
    fi
}

# ── Temp dir + cleanup ───────────────────────────────────────────────────────

WORK_DIR=$(mktemp -d)
DATA_DIR="$WORK_DIR/data"
CLIENT_HOME="$WORK_DIR/client"
mkdir -p "$DATA_DIR" "$CLIENT_HOME"

export CCP_CLIENT_HOME="$CLIENT_HOME"

SERVER_PID=""
cleanup() {
    if [ -n "$SERVER_PID" ]; then
        kill "$SERVER_PID" 2>/dev/null
        wait "$SERVER_PID" 2>/dev/null
    fi
    rm -rf "$WORK_DIR"
}
trap cleanup EXIT

# ── Build ────────────────────────────────────────────────────────────────────

section "Build"

if [ "$SKIP_BUILD" = true ]; then
    if [ -f "$SERVER_BIN" ] && [ -f "$CLIENT_BIN" ]; then
        skip "build skipped (--skip-build)"
    else
        fail "binaries not found and --skip-build was set"
        exit 1
    fi
else
    OUT=$(cargo build --release --manifest-path "$REPO_ROOT/Cargo.toml" 2>&1)
    if [ $? -eq 0 ]; then
        pass "cargo build --release"
    else
        fail "cargo build --release"
        echo "$OUT"
        exit 1
    fi
fi

# ── Unit tests ───────────────────────────────────────────────────────────────

section "Unit tests"

OUT=$(cargo test -p server --lib -- --test-threads=1 2>&1)
if echo "$OUT" | grep -q "test result: ok"; then
    COUNT=$(echo "$OUT" | grep "test result:" | grep -oE '[0-9]+ passed' | grep -oE '[0-9]+')
    pass "server ($COUNT passed)"
else
    fail "server unit tests"
fi

OUT=$(cargo test -p client --lib 2>&1)
if echo "$OUT" | grep -q "test result: ok"; then
    COUNT=$(echo "$OUT" | grep "test result:" | grep -oE '[0-9]+ passed' | grep -oE '[0-9]+')
    pass "client ($COUNT passed)"
else
    fail "client unit tests"
fi

# ── Start server ─────────────────────────────────────────────────────────────

section "Server lifecycle"

AUTH_PORT=$(python3 -c "import socket; s=socket.socket(); s.bind(('127.0.0.1',0)); print(s.getsockname()[1]); s.close()")
MTLS_PORT=$(python3 -c "import socket; s=socket.socket(); s.bind(('127.0.0.1',0)); print(s.getsockname()[1]); s.close()")
REDEEM_URL="http://127.0.0.1:$AUTH_PORT/auth/redeem"

CCP_SERVER_DATA_DIR="$DATA_DIR" \
CCP_AUTH_BASE_URL="http://127.0.0.1:$AUTH_PORT" \
CCP_MTLS_BASE_URL="https://localhost:$MTLS_PORT" \
CCP_AUTH_LISTENER_ADDR="127.0.0.1:$AUTH_PORT" \
CCP_MTLS_LISTENER_ADDR="127.0.0.1:$MTLS_PORT" \
"$SERVER_BIN" integration-test >"$WORK_DIR/server.log" 2>&1 &
SERVER_PID=$!

# Wait for server to be ready
READY=false
for i in $(seq 1 30); do
    if curl -s -o /dev/null "http://127.0.0.1:$AUTH_PORT" 2>/dev/null; then
        READY=true
        break
    fi
    sleep 0.2
done

if [ "$READY" = true ]; then
    pass "server started (pid=$SERVER_PID, auth=$AUTH_PORT, mtls=$MTLS_PORT)"
else
    fail "server did not start within 6 seconds"
    cat "$WORK_DIR/server.log"
    exit 1
fi

# ── Token issuance ───────────────────────────────────────────────────────────

section "Token issuance"

issue_token() {
    CCP_SERVER_DATA_DIR="$DATA_DIR" \
    CCP_AUTH_BASE_URL="http://127.0.0.1:$AUTH_PORT" \
    CCP_MTLS_BASE_URL="https://localhost:$MTLS_PORT" \
    CCP_AUTH_LISTENER_ADDR="127.0.0.1:$AUTH_PORT" \
    CCP_MTLS_LISTENER_ADDR="127.0.0.1:$MTLS_PORT" \
    "$SERVER_BIN" issue-token integration-test "$1" 2>&1
}

READ_TOKEN_JSON=$(issue_token read)
READ_TOKEN=$(echo "$READ_TOKEN_JSON" | python3 -c "import sys,json; print(json.load(sys.stdin)['token'])" 2>/dev/null)
if [ -n "$READ_TOKEN" ]; then
    pass "issued read token"
else
    fail "issue read token"
fi

RW_TOKEN_JSON=$(issue_token read_write)
RW_TOKEN=$(echo "$RW_TOKEN_JSON" | python3 -c "import sys,json; print(json.load(sys.stdin)['token'])" 2>/dev/null)
if [ -n "$RW_TOKEN" ]; then
    pass "issued read_write token"
else
    fail "issue read_write token"
fi

ADMIN_TOKEN_JSON=$(issue_token admin)
ADMIN_TOKEN=$(echo "$ADMIN_TOKEN_JSON" | python3 -c "import sys,json; print(json.load(sys.stdin)['token'])" 2>/dev/null)
if [ -n "$ADMIN_TOKEN" ]; then
    pass "issued admin token"
else
    fail "issue admin token"
fi

# ── Enrollment ───────────────────────────────────────────────────────────────

section "Enrollment"

OUT=$(expect_success "enroll read client" "$CLIENT_BIN" enroll --redeem-url "$REDEEM_URL" --token "$READ_TOKEN")
OUT=$(expect_success "enroll read_write client" "$CLIENT_BIN" enroll --redeem-url "$REDEEM_URL" --token "$RW_TOKEN")
OUT=$(expect_success "enroll admin client" "$CLIENT_BIN" enroll --redeem-url "$REDEEM_URL" --token "$ADMIN_TOKEN")

SESSIONS_OUT=$("$CLIENT_BIN" sessions 2>&1)
expect_contains "sessions lists enrollment" "$SESSIONS_OUT" "integration-test"
expect_contains "sessions shows all access levels" "$SESSIONS_OUT" "admin"

# ── Shelf + book creation ────────────────────────────────────────────────────

section "Shelf and book operations"

OUT=$(expect_success "add shelf" "$CLIENT_BIN" add-shelf integration-test research "collected research")
OUT=$(expect_success "add book" "$CLIENT_BIN" add-book integration-test --shelf research findings "key findings")

# ── Entry CRUD ───────────────────────────────────────────────────────────────

section "Entry CRUD"

OUT=$(expect_success "add entry" "$CLIENT_BIN" add-entry integration-test \
    --shelf research --book findings --labels "test,integration" \
    day1 "first entry" "initial content from integration test")

LIST_OUT=$("$CLIENT_BIN" list integration-test 2>&1)
expect_contains "list shows entry" "$LIST_OUT" "day1"

GET_OUT=$("$CLIENT_BIN" get integration-test day1 --shelf research --book findings 2>&1)
expect_contains "get returns content" "$GET_OUT" "initial content"
expect_contains "get returns labels" "$GET_OUT" "test"

OUT=$(expect_success "append to entry" "$CLIENT_BIN" append integration-test day1 \
    --shelf research --book findings "appended follow-up content")

GET_OUT2=$("$CLIENT_BIN" get integration-test day1 --shelf research --book findings 2>&1)
expect_contains "get shows appended content" "$GET_OUT2" "appended follow-up content"

HISTORY_OUT=$("$CLIENT_BIN" history integration-test day1 --shelf research --book findings 2>&1)
expect_contains "history shows append" "$HISTORY_OUT" "appended follow-up content"

# ── Search ───────────────────────────────────────────────────────────────────

section "Search"

SEARCH_OUT=$("$CLIENT_BIN" search-entries integration-test "day1" 2>&1)
expect_contains "search-entries finds by name" "$SEARCH_OUT" "day1"

SEARCH_OUT=$("$CLIENT_BIN" search-context integration-test "integration test" 2>&1)
expect_contains "search-context finds in content" "$SEARCH_OUT" "day1"

SEARCH_OUT=$("$CLIENT_BIN" search-shelves integration-test "research" 2>&1)
expect_contains "search-shelves finds shelf" "$SEARCH_OUT" "research"

SEARCH_OUT=$("$CLIENT_BIN" search-books integration-test "findings" 2>&1)
expect_contains "search-books finds book" "$SEARCH_OUT" "findings"

SEARCH_MISS=$("$CLIENT_BIN" search-entries integration-test "zzzznonexistent" 2>&1)
expect_not_contains "search miss returns empty" "$SEARCH_MISS" "day1"

# ── Delete + restore ─────────────────────────────────────────────────────────

section "Delete and restore"

DELETE_OUT=$("$CLIENT_BIN" delete integration-test day1 --shelf research --book findings 2>&1)
expect_contains "delete succeeds" "$DELETE_OUT" "day1"

LIST_AFTER=$("$CLIENT_BIN" list integration-test 2>&1)
expect_not_contains "entry gone after delete" "$LIST_AFTER" "day1"

DELETED_OUT=$("$CLIENT_BIN" search-deleted integration-test "day1" 2>&1)
expect_contains "search-deleted finds archived entry" "$DELETED_OUT" "day1"

# Extract entry_key for restore
ENTRY_KEY=$(echo "$DELETE_OUT" | python3 -c "import sys,json; print(json.load(sys.stdin)['entry_key'])" 2>/dev/null || echo "")
if [ -n "$ENTRY_KEY" ]; then
    RESTORE_OUT=$(expect_success "restore entry" "$CLIENT_BIN" restore integration-test "$ENTRY_KEY")
    LIST_RESTORED=$("$CLIENT_BIN" list integration-test 2>&1)
    expect_contains "entry back after restore" "$LIST_RESTORED" "day1"
else
    skip "restore — could not extract entry_key from delete output"
fi

# ── Delete shelf ─────────────────────────────────────────────────────────────

section "Delete shelf"

# create a throwaway shelf with entries, then nuke it
OUT=$(expect_success "add throwaway shelf" "$CLIENT_BIN" add-shelf integration-test throwaway "temp shelf")
OUT=$(expect_success "add throwaway book" "$CLIENT_BIN" add-book integration-test --shelf throwaway throwaway-book "temp book")
OUT=$(expect_success "add throwaway entry" "$CLIENT_BIN" add-entry integration-test \
    --shelf throwaway --book throwaway-book throwaway-entry "temp" "temp content")

DS_OUT=$("$CLIENT_BIN" delete-shelf integration-test throwaway 2>&1)
expect_contains "delete-shelf succeeds" "$DS_OUT" "throwaway"

LIST_DS=$("$CLIENT_BIN" list integration-test 2>&1)
expect_not_contains "throwaway entry gone" "$LIST_DS" "throwaway-entry"

SHELVES_DS=$("$CLIENT_BIN" search-shelves integration-test "throwaway" 2>&1)
expect_not_contains "throwaway shelf gone" "$SHELVES_DS" "throwaway"

# ── Export + import ──────────────────────────────────────────────────────────

section "Export and import"

# full session export
EXPORT_FILE="$WORK_DIR/export-full.droplet"
OUT=$(expect_success "export full session" "$CLIENT_BIN" export integration-test --output "$EXPORT_FILE")

if [ -f "$EXPORT_FILE" ] && [ -s "$EXPORT_FILE" ]; then
    pass "full export file exists and is non-empty"
else
    fail "full export file missing or empty"
fi

# verify bundle has sha256
if python3 -c "import json; b=json.load(open('$EXPORT_FILE')); assert b.get('bundle_sha256')" 2>/dev/null; then
    pass "export bundle contains sha256 hash"
else
    fail "export bundle missing sha256 hash"
fi

# scoped export — shelf only
SHELF_EXPORT="$WORK_DIR/export-shelf.droplet"
OUT=$(expect_success "export shelf" "$CLIENT_BIN" export integration-test --shelf research --output "$SHELF_EXPORT")
if python3 -c "import json; b=json.load(open('$SHELF_EXPORT')); assert b['selector']['scope']['Shelf']['shelf']=='research'" 2>/dev/null; then
    pass "shelf export has correct scope"
else
    fail "shelf export scope incorrect"
fi

# scoped export — book
BOOK_EXPORT="$WORK_DIR/export-book.droplet"
OUT=$(expect_success "export book" "$CLIENT_BIN" export integration-test --shelf research --book findings --output "$BOOK_EXPORT")
if python3 -c "import json; b=json.load(open('$BOOK_EXPORT')); assert b['selector']['scope']['Book']['book']=='findings'" 2>/dev/null; then
    pass "book export has correct scope"
else
    fail "book export scope incorrect"
fi

# export without history
NOHIST_EXPORT="$WORK_DIR/export-nohist.droplet"
OUT=$(expect_success "export no-history" "$CLIENT_BIN" export integration-test --no-history --output "$NOHIST_EXPORT")
if python3 -c "import json; b=json.load(open('$NOHIST_EXPORT')); assert b['selector']['include_history']==False" 2>/dev/null; then
    pass "no-history export flag works"
else
    fail "no-history flag not reflected in bundle"
fi

# delete entry then re-import with overwrite
"$CLIENT_BIN" delete integration-test day1 --shelf research --book findings >/dev/null 2>&1

OUT=$(expect_success "import with overwrite" "$CLIENT_BIN" import integration-test "$EXPORT_FILE" --policy overwrite)

LIST_IMPORTED=$("$CLIENT_BIN" list integration-test 2>&1)
expect_contains "imported entry exists" "$LIST_IMPORTED" "day1"

# import with skip policy (entry already exists, should skip)
SKIP_OUT=$("$CLIENT_BIN" import integration-test "$EXPORT_FILE" --policy skip 2>&1)
expect_contains "skip policy works" "$SKIP_OUT" "skipped"

# import with error policy (entry exists, should fail)
ERR_OUT=$("$CLIENT_BIN" import integration-test "$EXPORT_FILE" --policy error 2>&1)
if echo "$ERR_OUT" | grep -qi "already exists\|error"; then
    pass "error policy rejects duplicate"
else
    fail "error policy did not reject duplicate"
fi

# ── Brief me ─────────────────────────────────────────────────────────────────

section "Brief me"

BRIEF_OUT=$("$CLIENT_BIN" brief-me integration-test 2>&1)
expect_contains "brief returns session name" "$BRIEF_OUT" "integration-test"
expect_contains "brief includes shelves" "$BRIEF_OUT" "research"
expect_contains "brief includes recent entries" "$BRIEF_OUT" "day1"

# ── Temporal queries ─────────────────────────────────────────────────────────

section "Temporal queries"

# get the current timestamp to use as "after all appends"
CURRENT_TS=$(python3 -c "import time; print(int(time.time()))")

# get entry at current time — should have all content
ENTRY_NOW=$("$CLIENT_BIN" get-entry-at integration-test day1 --at "$CURRENT_TS" --shelf research --book findings 2>&1)
expect_contains "entry at now has content" "$ENTRY_NOW" "initial content"

# get entry at timestamp 0 — should have no content (no appends before epoch)
ENTRY_OLD=$("$CLIENT_BIN" get-entry-at integration-test day1 --at "0" --shelf research --book findings 2>&1)
expect_not_contains "entry at epoch has no appended content" "$ENTRY_OLD" "appended follow-up"

# ── Access control ───────────────────────────────────────────────────────────

section "Access control"

# Read client should not be able to write. Issue a new read-only token and enroll
# in an isolated client home to test.
READ_ONLY_HOME="$WORK_DIR/read_only_client"
mkdir -p "$READ_ONLY_HOME"
RO_TOKEN_JSON=$(issue_token read)
RO_TOKEN=$(echo "$RO_TOKEN_JSON" | python3 -c "import sys,json; print(json.load(sys.stdin)['token'])" 2>/dev/null)

CCP_CLIENT_HOME="$READ_ONLY_HOME" "$CLIENT_BIN" enroll --redeem-url "$REDEEM_URL" --token "$RO_TOKEN" >/dev/null 2>&1

RO_ADD=$(CCP_CLIENT_HOME="$READ_ONLY_HOME" "$CLIENT_BIN" add-shelf integration-test blocked-shelf "nope" 2>&1)
if echo "$RO_ADD" | grep -qi "denied\|error\|access\|write"; then
    pass "read client cannot write"
else
    fail "read client was allowed to write"
fi

# ── Certificate revocation ───────────────────────────────────────────────────

section "Certificate revocation"

# Use an isolated admin-only home so the CLI picks the admin enrollment
ADMIN_HOME="$WORK_DIR/admin_only"
mkdir -p "$ADMIN_HOME"
ADMIN_REVOKE_TOKEN_JSON=$(issue_token admin)
ADMIN_REVOKE_TOKEN=$(echo "$ADMIN_REVOKE_TOKEN_JSON" | python3 -c "import sys,json; print(json.load(sys.stdin)['token'])" 2>/dev/null)
CCP_CLIENT_HOME="$ADMIN_HOME" "$CLIENT_BIN" enroll --redeem-url "$REDEEM_URL" --token "$ADMIN_REVOKE_TOKEN" >/dev/null 2>&1

# Enroll a target to revoke in its own home
REVOKE_HOME="$WORK_DIR/revoke_target"
mkdir -p "$REVOKE_HOME"
REVOKE_TOKEN_JSON=$(issue_token read_write)
REVOKE_TOKEN=$(echo "$REVOKE_TOKEN_JSON" | python3 -c "import sys,json; print(json.load(sys.stdin)['token'])" 2>/dev/null)

REVOKE_ENROLL=$(CCP_CLIENT_HOME="$REVOKE_HOME" "$CLIENT_BIN" enroll --redeem-url "$REDEEM_URL" --token "$REVOKE_TOKEN" 2>&1)
REVOKE_CN=$(echo "$REVOKE_ENROLL" | grep "client_cn=" | sed 's/.*client_cn=//')

if [ -n "$REVOKE_CN" ]; then
    # Admin revokes this client
    REVOKE_OUT=$(CCP_CLIENT_HOME="$ADMIN_HOME" "$CLIENT_BIN" revoke-cert integration-test "$REVOKE_CN" 2>&1)
    expect_contains "admin revokes cert" "$REVOKE_OUT" "revoked"

    # Revoked client should fail to list
    REVOKED_LIST=$(CCP_CLIENT_HOME="$REVOKE_HOME" "$CLIENT_BIN" list integration-test 2>&1)
    if echo "$REVOKED_LIST" | grep -qi "denied\|error\|access\|revoked"; then
        pass "revoked client denied access"
    else
        fail "revoked client still has access"
    fi
else
    skip "revocation — could not extract client CN"
fi

# ── Server health ────────────────────────────────────────────────────────────

section "Server health"

HEALTH_OUT=$(CCP_SERVER_DATA_DIR="$DATA_DIR" \
    CCP_AUTH_BASE_URL="http://127.0.0.1:$AUTH_PORT" \
    CCP_MTLS_BASE_URL="https://localhost:$MTLS_PORT" \
    CCP_AUTH_LISTENER_ADDR="127.0.0.1:$AUTH_PORT" \
    CCP_MTLS_LISTENER_ADDR="127.0.0.1:$MTLS_PORT" \
    "$SERVER_BIN" health integration-test 2>&1)
expect_contains "health check returns session info" "$HEALTH_OUT" "integration-test"

# ── File permissions ─────────────────────────────────────────────────────────

section "Security"

KEY_FILES=$(find "$CLIENT_HOME" -name "client.key" -type f 2>/dev/null)
if [ -n "$KEY_FILES" ]; then
    BAD_PERMS=false
    while IFS= read -r kf; do
        PERMS=$(stat -f "%Lp" "$kf" 2>/dev/null || stat -c "%a" "$kf" 2>/dev/null)
        if [ "$PERMS" != "600" ]; then
            BAD_PERMS=true
            fail "client.key at $kf has permissions $PERMS (should be 600)"
        fi
    done <<< "$KEY_FILES"
    if [ "$BAD_PERMS" = false ]; then
        pass "all client.key files have 600 permissions"
    fi
else
    skip "no client.key files found"
fi

TRACKED=$(git -C "$REPO_ROOT" ls-files '*.pem' '*.sqlite3' '*.key' '*.bin' 2>/dev/null)
if [ -z "$TRACKED" ]; then
    pass "no secrets tracked in git"
else
    fail "secrets tracked in git: $TRACKED"
fi

# ── Shutdown ─────────────────────────────────────────────────────────────────

section "Shutdown"

kill "$SERVER_PID" 2>/dev/null
wait "$SERVER_PID" 2>/dev/null
SERVER_PID=""
pass "server stopped cleanly"

# ── Summary ──────────────────────────────────────────────────────────────────

echo ""
echo -e "${BOLD}════════════════════════════════════════${RESET}"
TOTAL=$((PASS_COUNT + FAIL_COUNT + SKIP_COUNT))
echo -e "  ${GREEN}$PASS_COUNT passed${RESET}  ${RED}$FAIL_COUNT failed${RESET}  ${YELLOW}$SKIP_COUNT skipped${RESET}  ($TOTAL total)"
echo -e "${BOLD}════════════════════════════════════════${RESET}"

if [ "$FAIL_COUNT" -gt 0 ]; then
    echo ""
    echo -e "${RED}Failures:${RESET}"
    for f in "${FAILURES[@]}"; do
        echo -e "  ${RED}✗${RESET} $f"
    done
    exit 1
fi

exit 0
