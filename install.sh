#!/usr/bin/env bash
# Cephalopod Coordination Protocol installer.
#
# curl -fsSL https://raw.githubusercontent.com/squid-proxy-lovers/ccp/main/install.sh | bash
#
# Copyright (C) 2026 Squid Proxy Lovers
# SPDX-License-Identifier: AGPL-3.0-or-later

set -uo pipefail

REPO="squid-proxy-lovers/ccp"
REPO_RAW="https://raw.githubusercontent.com/squid-proxy-lovers/ccp/main"
REPO_ROOT="$(CDPATH='' cd -P -- "$(dirname "${BASH_SOURCE[0]:-$0}" 2>/dev/null)" 2>/dev/null && pwd || echo "")"
INSTALL_DIR="${HOME:-}/.local/bin"
SESSION_NAME="my-session"
MODE="both"
FROM_SOURCE=false
CHANNEL="stable"
RELEASE_VERSION=""
RELEASE_STAGE=""
PUBLISH_STAGE=""
MCP_CONFIG_WARNINGS=""
UNINSTALL=false
INSTALL_ONLY_OPTIONS=""
UNINSTALL_REMOVED=0
UNINSTALL_FAILED=0

# ── Colors ───────────────────────────────────────────────────────────────────

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
CYAN='\033[0;36m'
MAGENTA='\033[0;35m'
BOLD='\033[1m'
DIM='\033[2m'
RESET='\033[0m'

info()  { echo -e "${CYAN}>>>${RESET} $1"; }
ok()    { echo -e "${GREEN} ✓${RESET} $1"; }
warn()  { echo -e "${YELLOW} !${RESET} $1"; }
err()   { echo -e "${RED} ✗${RESET} $1"; }
step()  { echo -e "${BOLD}${CYAN}>>>${RESET} $1"; }

banner() {
    echo ""
    echo -e "${MAGENTA}${BOLD}"
    cat <<'SQUID'
                                        ██████████
                                    ████░░░░░░░░░░████
                                  ██░░░░░░░░░░░░░░░░░░██
                                ██░░░░░░░░░░░░░░░░░░░░░░██
                                ██░░░░░░░░░░░░░░░░░░░░░░██
                              ██░░░░░░░░░░░░░░░░░░░░░░░░░░██
                              ██░░        ░░░░░░        ░░██
                              ██░░          ░░          ░░██
                              ██░░    ████  ░░  ████    ░░██
                              ██░░    ██████████████    ░░██
                                ██░░  ░░██░░░░░░██░░  ░░██
                              ██░░██░░██░░██████░░██░░██░░██
                            ██░░░░██████░░██████░░██████░░░░██
                            ██░░██░░░░████░░░░░░████░░░░██░░██
                              ████░░░░██░░██████░░██░░░░░░██
                              ██░░░░████░░░░██░░░░████░░░░██
                                ██████░░░░██  ██░░░░██████
                                      ████      ████

                    ╔════════════════════════════════════════════════╗
                    ║     Cephalopod Coordination Protocol (CCP)     ║
                    ╚════════════════════════════════════════════════╝
SQUID
    echo -e "${RESET}"
}

# ── Args ─────────────────────────────────────────────────────────────────────

while [[ $# -gt 0 ]]; do
    case "$1" in
        --client)      MODE="client"; INSTALL_ONLY_OPTIONS+=" --client"; shift ;;
        --docker)      MODE="docker"; INSTALL_ONLY_OPTIONS+=" --docker"; shift ;;
        --install-dir)
            if [ $# -lt 2 ] || [[ "$2" == --* ]]; then err "--install-dir requires a value."; exit 1; fi
            INSTALL_DIR="$2"; shift 2
            ;;
        --session)
            if [ $# -lt 2 ] || [[ "$2" == --* ]]; then err "--session requires a value."; exit 1; fi
            SESSION_NAME="$2"; INSTALL_ONLY_OPTIONS+=" --session"; shift 2
            ;;
        --from-source) FROM_SOURCE=true; INSTALL_ONLY_OPTIONS+=" --from-source"; shift ;;
        --uninstall)   UNINSTALL=true; shift ;;
        --channel|--version)
            if [ $# -lt 2 ] || [[ "$2" == --* ]] || [ -z "$2" ]; then
                err "$1 requires a value."; exit 1
            fi
            if [ "$1" = "--channel" ]; then CHANNEL="$2"; else RELEASE_VERSION="$2"; fi
            INSTALL_ONLY_OPTIONS+=" $1"
            shift 2
            ;;
        -h|--help)
            cat <<EOF
Usage: bash install.sh [--client | --docker] [OPTIONS]

  (default)        Install server + client
  --client         Client only + auto-configure MCP for Claude/Cursor/Codex
  --docker         Pull and start the server container
  --uninstall      Remove what this installer installed (see below)

Options:
  --install-dir <path>   Binary install directory (default: ~/.local/bin)
                         With --uninstall: the directory to remove from
  --session <name>       Docker session name (default: my-session)
  --from-source          Build from repo instead of downloading release binaries
  --channel <channel>    stable (default) or nightly
  --version <tag>        Pin an exact nightly release (requires --channel nightly)

Uninstall (bash install.sh --uninstall [--install-dir <path>]) removes
ccp-client, ccp-server and ccp-nightly.json from the install directory, the
ccp alias if it links to ccp-client, the MCP bridge environment in
~/.ccp-mcp/venv (never ~/.ccp-mcp/src), and the
"ccp" entry in the Claude and Cursor MCP configs when it points at that
bridge. It never deletes enrollment profiles, client state, server data or
Docker volumes. It prints where they are and how to remove them. It does not
edit Codex config.toml; remove the [mcp_servers.ccp] table by hand.
EOF
            exit 0
            ;;
        *) err "Unknown option: $1"; exit 1 ;;
    esac
done

# Print the physical (symlink-free) absolute path of an existing path.
physical_path() {
    local path="$1" out dir
    if [ -d "$path" ]; then
        out="$(CDPATH='' cd -P -- "$path" && pwd -P)" || return 1
    else
        dir="$(CDPATH='' cd -P -- "$(dirname -- "$path")" && pwd -P)" || return 1
        out="${dir%/}/$(basename -- "$path")"
    fi
    # Some shells print // for the root; collapse any leading run of slashes.
    while [ "${out#//}" != "$out" ]; do out="${out#/}"; done
    printf '%s\n' "$out"
}

# Validate the environment and paths before anything is touched.
if [ -z "${HOME:-}" ]; then
    err "HOME is not set; it locates the install directory, MCP bridge and host configs."
    exit 1
fi
if [ -z "${INSTALL_DIR//\//}" ]; then
    err "--install-dir must be a directory path other than empty or /."
    exit 1
fi
if [ -d "$INSTALL_DIR" ] && [ "$(physical_path "$INSTALL_DIR")" = "/" ]; then
    err "--install-dir resolves to /; choose a specific directory."
    exit 1
fi
if [ "$UNINSTALL" = true ] && [ -n "$INSTALL_ONLY_OPTIONS" ]; then
    err "--uninstall only accepts --install-dir; it cannot be combined with:$INSTALL_ONLY_OPTIONS"
    exit 1
fi

case "$CHANNEL" in
    stable|nightly) ;;
    *) err "Unknown channel: $CHANNEL (choose stable or nightly)."; exit 1 ;;
esac
if [ -n "$RELEASE_VERSION" ]; then
    if [ "$CHANNEL" != "nightly" ] || ! [[ "$RELEASE_VERSION" =~ ^nightly-[0-9]{4}-[0-9]{2}-[0-9]{2}-[0-9a-f]{12}$ ]]; then
        err "--version requires --channel nightly and a nightly-YYYY-MM-DD-<12-character SHA> tag."
        exit 1
    fi
fi
if [ "$CHANNEL" = "nightly" ] && [ "$FROM_SOURCE" = true ]; then
    err "Choose --channel nightly for published builds or --from-source for your checkout."
    exit 1
fi

cleanup_release() {
    if [ -n "$RELEASE_STAGE" ]; then rm -rf "$RELEASE_STAGE"; fi
    if [ -n "$PUBLISH_STAGE" ]; then rm -f "$PUBLISH_STAGE"; fi
}
trap cleanup_release EXIT

# ── Helpers ──────────────────────────────────────────────────────────────────

detect_platform() {
    local os arch
    os="$(uname -s)"
    arch="$(uname -m)"

    case "$os" in
        Linux)  os="linux" ;;
        Darwin) os="darwin" ;;
        *)      err "Unsupported OS: $os"; exit 1 ;;
    esac

    case "$arch" in
        x86_64|amd64)  arch="x86_64" ;;
        arm64|aarch64) arch="aarch64" ;;
        *)             err "Unsupported architecture: $arch"; exit 1 ;;
    esac

    echo "${os}-${arch}"
}

resolve_destination() {
    local dest="$1" link hops=0 parent
    while true; do
        parent="$(CDPATH='' cd -P -- "${dest%/*}" && pwd -P)" || return 1
        dest="$parent/${dest##*/}"
        if [ ! -L "$dest" ]; then break; fi
        if [ "$hops" -ge 40 ] || [ ! -f "$dest" ]; then
            err "$dest is not a symlink to a regular file." >&2
            return 1
        fi
        link="$(readlink "$dest")" || return 1
        case "$link" in
            /*) dest="$link" ;;
            *) dest="$parent/$link" ;;
        esac
        hops=$((hops + 1))
    done
    if [ -e "$dest" ] && [ ! -f "$dest" ]; then
        err "$dest exists and is not a regular file; refusing to overwrite it." >&2
        return 1
    fi
    printf '%s\n' "$dest"
}

finish_publish() {
    local dest="$1"
    # mv can succeed by moving the stage *inside* a concurrently created
    # directory. Verify the postcondition before claiming publication.
    if ! mv -f "$PUBLISH_STAGE" "$dest" || [ ! -f "$dest" ] || [ -e "$dest/${PUBLISH_STAGE##*/}" ]; then
        err "Could not publish $dest."
        if [ -d "$dest" ]; then rm -f "$dest/${PUBLISH_STAGE##*/}"; fi
        rm -f "$PUBLISH_STAGE"
        PUBLISH_STAGE=""
        return 1
    fi
    PUBLISH_STAGE=""
}

publish_file() {
    local source="$1" dest mode="$3"
    dest="$(resolve_destination "$2")" || return 1
    PUBLISH_STAGE="$(mktemp "${dest}.stage.XXXXXX")" || return 1
    if ! cp "$source" "$PUBLISH_STAGE" || ! chmod "$mode" "$PUBLISH_STAGE"; then
        err "Could not stage $dest."
        return 1
    fi
    finish_publish "$dest"
}

publish_binary() {
    publish_file "$1" "$2" 0755
}

download_binary() {
    local name="$1" dest
    local platform
    dest="$(resolve_destination "$2")" || return 1
    platform="$(detect_platform)"
    publish_binary "$RELEASE_STAGE/$name-$platform" "$dest"
}

prepare_stable_release() {
    if ! command -v python3 &>/dev/null; then
        err "Python 3 is required to select and verify stable releases."
        return 1
    fi
    RELEASE_STAGE="$(mktemp -d)" || return 1
    local api="https://api.github.com/repos/${REPO}/releases/latest"
    if ! download_release_file "$api" "$RELEASE_STAGE/release.json"; then
        stable_release_error "Stable release discovery failed. No binaries installed."
        return 1
    fi
    RELEASE_VERSION="$(python3 - "$RELEASE_STAGE/release.json" <<'PY'
import json, re, sys
release = json.load(open(sys.argv[1]))
tag = release.get("tag_name", "")
if release.get("draft") or release.get("prerelease") or not re.fullmatch(r"v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)", tag):
    sys.exit("Invalid stable release")
print(tag)
PY
    )" || {
        stable_release_error "Stable release selection failed. No binaries installed."
        return 1
    }
    local platform base asset
    platform="$(detect_platform)" || return 1
    base="https://github.com/${REPO}/releases/download/${RELEASE_VERSION}"
    local assets=("ccp-client-$platform")
    if [ "$INSTALL_MCP" = true ]; then assets+=("ccp-mcp.tar.gz"); fi
    if [ "$MODE" = both ]; then assets+=("ccp-server-$platform"); fi
    for asset in SHA256SUMS "${assets[@]}"; do
        if ! download_release_file "$base/$asset" "$RELEASE_STAGE/$asset"; then
            stable_release_error "Stable download failed: $asset. No binaries installed."
            return 1
        fi
    done
    if ! python3 - "$RELEASE_STAGE" "${assets[@]}" <<'PY'
import hashlib, pathlib, re, sys
stage = pathlib.Path(sys.argv[1])
checksums = {}
for line in (stage / "SHA256SUMS").read_text().splitlines():
    match = re.fullmatch(r"([0-9a-f]{64})  ([a-zA-Z0-9_.-]+)", line)
    if not match or match[2] in checksums:
        sys.exit("Invalid stable checksum manifest")
    checksums[match[2]] = match[1]
for name in sys.argv[2:]:
    if hashlib.sha256((stage / name).read_bytes()).hexdigest() != checksums.get(name):
        sys.exit(f"Stable checksum mismatch: {name}")
PY
    then
        stable_release_error "Stable verification failed. No binaries installed."
        return 1
    fi
}

stable_release_error() {
    err "$1"
    info "Try installing from source instead:"
    echo -e "  ${DIM}curl -fsSL ${REPO_RAW}/install.sh | bash -s -- --from-source${RESET}"
}

download_release_file() {
    local url="$1" dest="$2"
    if command -v curl &>/dev/null; then
        curl -fsSL "$url" -o "$dest"
    elif command -v wget &>/dev/null; then
        wget -q "$url" -O "$dest"
    else
        err "Need curl or wget to download nightlies."
        return 1
    fi
}

install_nightly_binaries() {
    if ! command -v python3 &>/dev/null; then
        err "Python 3 is required to select and verify nightly builds."
        exit 1
    fi
    RELEASE_STAGE="$(mktemp -d)" || exit 1
    local page=1 selection platform asset base
    if [ -z "$RELEASE_VERSION" ]; then
        while true; do
            if ! download_release_file "https://api.github.com/repos/${REPO}/releases?per_page=100&page=$page" "$RELEASE_STAGE/releases.json"; then
                err "Could not discover nightly releases."
                exit 1
            fi
            selection="$(python3 - "$RELEASE_STAGE/releases.json" <<'PY'
import json, re, sys
releases = json.load(open(sys.argv[1]))
if not isinstance(releases, list):
    sys.exit("Invalid GitHub release response")
candidates = [item for item in releases if item.get("prerelease") and not item.get("draft")
              and re.fullmatch(r"nightly-\d{4}-\d{2}-\d{2}-[0-9a-f]{12}", item.get("tag_name", ""))]
print(max(candidates, key=lambda item: item["published_at"])["tag_name"] if candidates
      else ("more" if releases else "none"))
PY
            )" || exit 1
            case "$selection" in
                more) page=$((page + 1)) ;;
                none) err "No published nightly is available yet."; exit 1 ;;
                *) RELEASE_VERSION="$selection"; break ;;
            esac
        done
    fi
    platform="$(detect_platform)" || exit 1
    base="https://github.com/${REPO}/releases/download/${RELEASE_VERSION}"
    step "Downloading nightly ${BOLD}$RELEASE_VERSION${RESET} for $platform"
    local assets=("ccp-client-$platform" "build-info.json")
    if [ "$INSTALL_MCP" = true ]; then assets+=("ccp-mcp.tar.gz"); fi
    if [ "$MODE" = "both" ]; then assets+=("ccp-server-$platform"); fi
    for asset in SHA256SUMS "${assets[@]}"; do
        if ! download_release_file "$base/$asset" "$RELEASE_STAGE/$asset"; then
            err "Nightly download failed: $asset. No binaries installed."
            exit 1
        fi
    done
    if ! python3 - "$RELEASE_STAGE" "$RELEASE_VERSION" "${assets[@]}" <<'PY'
import hashlib, json, pathlib, re, sys
stage, tag = pathlib.Path(sys.argv[1]), sys.argv[2]
checksums = {}
for line in (stage / "SHA256SUMS").read_text().splitlines():
    match = re.fullmatch(r"([0-9a-f]{64})  ([a-zA-Z0-9_.-]+)", line)
    if not match or match[2] in checksums:
        sys.exit("Invalid nightly checksum manifest")
    checksums[match[2]] = match[1]
for name in sys.argv[3:]:
    if hashlib.sha256((stage / name).read_bytes()).hexdigest() != checksums.get(name):
        sys.exit(f"Nightly checksum mismatch: {name}")
source = json.loads((stage / "build-info.json").read_text())["source_sha"]
if not re.fullmatch(r"[0-9a-f]{40}", source) or not tag.endswith("-" + source[:12]):
    sys.exit("Nightly metadata does not match the selected release")
PY
    then
        err "Nightly verification failed. No binaries installed."
        exit 1
    fi
    # Verify every component before replacing any installed binary.
    for asset in "ccp-client" "ccp-server"; do
        if [ "$asset" = "ccp-server" ] && [ "$MODE" = "client" ]; then continue; fi
        publish_binary "$RELEASE_STAGE/$asset-$platform" "$INSTALL_DIR/$asset" || exit 1
    done
    publish_file "$RELEASE_STAGE/build-info.json" "$INSTALL_DIR/ccp-nightly.json" 0644 || exit 1
}

build_from_source() {
    if ! command -v cargo &>/dev/null; then
        err "Rust is not installed."
        info "Get it at ${BOLD}https://rustup.rs${RESET}"
        exit 1
    fi

    if [ "$MODE" != "client" ]; then
        if ! command -v npm &>/dev/null || ! command -v node &>/dev/null; then
            err "Server source builds require Node.js 24 or newer and npm."
            exit 1
        fi
        if ! node -e 'process.exit(Number(process.versions.node.split(".")[0]) >= 24 ? 0 : 1)'; then
            err "Server source builds require Node.js 24 or newer."
            exit 1
        fi
    fi

    local build_dir="$REPO_ROOT"

    if [ -z "$build_dir" ] || [ ! -f "$build_dir/Cargo.toml" ]; then
        build_dir="$(mktemp -d)"
        step "Cloning repo..."
        if ! git clone --depth 1 "https://github.com/${REPO}.git" "$build_dir" 2>&1; then
            err "Clone failed. Run this from inside the repo instead."
            exit 1
        fi
    fi

    local packages=(-p client)
    if [ "$MODE" != "client" ]; then
        step "Installing locked frontend build dependencies..."
        if ! npm ci --prefix "$build_dir/crates/server/web" --ignore-scripts; then
            err "Frontend dependency installation failed. No binaries installed."
            exit 1
        fi
        packages+=(-p server)
    fi
    step "Building ${BOLD}release${RESET}${CYAN} binaries...${RESET}"
    if ! (CDPATH='' cd -P -- "$build_dir" && cargo build --release --locked "${packages[@]}"); then
        err "Build failed."
        exit 1
    fi

    mkdir -p "$INSTALL_DIR" || return 1
    if [ "$MODE" != "client" ]; then
        publish_binary "$build_dir/target/release/server" "$INSTALL_DIR/ccp-server" || return 1
    fi
    publish_binary "$build_dir/target/release/client" "$INSTALL_DIR/ccp-client" || return 1

    if [ "$build_dir" != "$REPO_ROOT" ]; then
        rm -rf "$build_dir" || return 1
    fi
}

install_ccp_alias() {
    local alias_path="$INSTALL_DIR/ccp"
    if [ -e "$alias_path" ] || [ -L "$alias_path" ]; then
        if [ -L "$alias_path" ] && [ "$(readlink "$alias_path")" = "ccp-client" ]; then
            return
        fi
        warn "An existing ccp path was retained. Use ccp-client for CCP."
        return
    fi
    if command -v ccp &>/dev/null; then
        warn "An existing ccp command was retained. Use ccp-client for CCP."
        return
    fi
    if ! ln -s ccp-client "$alias_path"; then
        warn "The ccp alias could not be created. Use ccp-client for CCP."
    fi
}

ensure_path() {
    if ! echo "$PATH" | tr ':' '\n' | grep -qx "$INSTALL_DIR"; then
        echo ""
        warn "Add to your shell profile:"
        echo -e "  ${DIM}export PATH=\"$INSTALL_DIR:\$PATH\"${RESET}"
    fi
}

install_mcp_bridge() {
    if ! command -v python3 &>/dev/null; then
        err "Python 3 is required for the MCP bridge."
        return 1
    fi

    local mcp_home="$HOME/.ccp-mcp"
    local venv_dir="$mcp_home/venv"
    local mcp_src

    if [ "$FROM_SOURCE" = false ]; then
        step "Installing MCP bridge from $RELEASE_VERSION"
        if [ ! -d "$venv_dir" ] && ! python3 -m venv "$venv_dir"; then
            err "Could not create the MCP environment."; exit 1
        fi
        if ! "$venv_dir/bin/pip" install --quiet --upgrade --force-reinstall "$RELEASE_STAGE/ccp-mcp.tar.gz"; then
            err "Release MCP installation failed."; exit 1
        fi
        ok "MCP bridge installed in $venv_dir"
        return
    fi

    if [ "$FROM_SOURCE" = true ] && [ -n "$REPO_ROOT" ] && [ -d "$REPO_ROOT/mcp" ]; then
        mcp_src="$REPO_ROOT/mcp"
    else
        mcp_src="$mcp_home/src"
        if [ ! -d "$mcp_src" ]; then
            mkdir -p "$mcp_home" || return 1
            git clone --depth 1 "https://github.com/${REPO}.git" "$mcp_home/repo" || return 1
            mv "$mcp_home/repo/mcp" "$mcp_src" || return 1
            rm -rf "$mcp_home/repo" || return 1
        fi
    fi

    step "Installing MCP bridge..."
    if [ ! -d "$venv_dir" ]; then
        python3 -m venv "$venv_dir" || return 1
    fi
    "$venv_dir/bin/pip" install --quiet --upgrade pip || return 1
    "$venv_dir/bin/pip" install --quiet -e "$mcp_src" || return 1
    ok "MCP bridge installed in $venv_dir"
}

mcp_config_warning() {
    warn "$2"
    if [ -n "$MCP_CONFIG_WARNINGS" ]; then MCP_CONFIG_WARNINGS+="; "; fi
    MCP_CONFIG_WARNINGS+="the $1 MCP config was not written"
}

# Inspect or update the Codex config with one python3 process. Mode "check"
# prints the state of [mcp_servers.ccp]: present, absent, unsafe (a value we
# must not touch or extend), unsupported (no tomllib) or invalid (unparseable).
# Mode "append CONFIG COMMAND CLIENT SERVER" builds the entry with escaped TOML
# strings, takes an flock on the sidecar file CONFIG.ccp-lock, retrying for
# about two seconds (CCP_CODEX_LOCK_RETRIES tenths of a second, for tests),
# then reads, validates and appends the entry through one descriptor on the
# config, in place, all in that one process, so the kernel drops the lock
# however the process ends. If the write fails, or the path stops naming the
# file that was written, or the process gets HUP, INT or TERM, the config is
# truncated back to its original size, but only when the file is exactly the
# validated bytes plus the bytes this process wrote; another process's
# in-place edits are never cut off, and nothing is cut when nothing was
# written. SIGKILL or power loss mid-write can still leave a partial append,
# as the old "cat >>" could; nothing rolls that back. A read-only config is never replaced. It prints added, locked
# (contention), failed (lock or write error), changed (the file differed
# from what was validated, nothing written), damaged (a write failed and
# someone else's bytes made a rollback unsafe), badentry (the
# entry itself is not valid TOML) or a check state. The lock file itself is
# left in place; only the lock on it matters.
codex_config_tool() {
    local out
    out="$(python3 -I - "$@" <<'PY'
import os, signal, sys, time
try:
    import tomllib
except ModuleNotFoundError:
    print("unsupported")
    sys.exit(0)


def classify(config):
    servers = config.get("mcp_servers", {})
    if not isinstance(servers, dict):
        return "unsafe"
    if "ccp" not in servers:
        return "absent"
    return "present" if isinstance(servers["ccp"], dict) else "unsafe"


def inspect(raw):
    try:
        return classify(tomllib.loads(raw.decode("utf-8")))
    except (ValueError, RecursionError, MemoryError):
        return "invalid"


def check(path):
    try:
        with open(path, "rb") as stream:
            return inspect(stream.read())
    except OSError:
        return "invalid"


def quote(value):
    named = {
        "\\": "\\\\", '"': '\\"', "\b": "\\b", "\t": "\\t",
        "\n": "\\n", "\f": "\\f", "\r": "\\r",
    }
    chars = []
    for char in value:
        if char in named:
            chars.append(named[char])
        elif ord(char) < 0x20 or ord(char) == 0x7F:
            chars.append("\\u%04x" % ord(char))
        else:
            chars.append(char)
    return '"' + "".join(chars) + '"'


def build_block(command, client, server):
    lines = ["", "[mcp_servers.ccp]", "command = " + quote(command), ""]
    lines += ["[mcp_servers.ccp.env]", "CCP_CLIENT_BIN = " + quote(client)]
    if server:
        lines.append("CCP_SERVER_BIN = " + quote(server))
    block = "\n".join(lines)
    block.encode("utf-8")
    tomllib.loads(block)
    return block


def retries():
    try:
        return max(1, int(os.environ.get("CCP_CODEX_LOCK_RETRIES", "20")))
    except ValueError:
        return 20


def acquire(path):
    import fcntl
    fd = os.open(path + ".ccp-lock", os.O_RDWR | os.O_CREAT, 0o600)
    for _ in range(retries()):
        try:
            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
            return True
        except BlockingIOError:
            time.sleep(0.1)
    return False


def read_all(fd, size):
    chunks = []
    offset = 0
    while offset < size:
        chunk = os.pread(fd, size - offset, offset)
        if not chunk:
            break
        chunks.append(chunk)
        offset += len(chunk)
    return b"".join(chunks)


class Damaged(Exception):
    """The config changed under a failed write, so it was not rolled back."""


def write_tail(fd, before, path, tail, written):
    view = memoryview(tail)
    while view:
        count = os.write(fd, view)
        written[0] += count
        view = view[count:]
    os.fsync(fd)
    current = os.stat(path)
    if (current.st_dev, current.st_ino) != (before.st_dev, before.st_ino):
        raise OSError("config was replaced during the update")


def roll_back(fd, original, tail, written):
    # Undo only what this process appended. With nothing written there is
    # nothing of ours to remove. Otherwise the file must be exactly the
    # validated bytes plus the bytes os.write reported; anything else includes
    # edits by another writer, and cutting it off would destroy that work.
    if written == 0:
        return
    if read_all(fd, os.fstat(fd).st_size) != original + tail[:written]:
        raise Damaged
    os.ftruncate(fd, len(original))
    os.fsync(fd)


def append(path, block):
    # One descriptor does the read, the validation and the write. Right before
    # writing, the file is read again and must still equal the validated bytes.
    # Appending in place keeps the inode, mode, ACLs, xattrs and owner; opening
    # the path follows a symlink and fails on a read-only file. An editor that
    # writes without taking our lock can still race in the tiny window between
    # that last comparison and the write; that is accepted.
    tail = (block + "\n").encode("utf-8")
    fd = os.open(path, os.O_RDWR | os.O_APPEND)
    try:
        before = os.fstat(fd)
        original = read_all(fd, before.st_size)
        state = inspect(original)
        if state != "absent":
            return state
        try:
            candidate = tomllib.loads((original + tail).decode("utf-8"))
        except (UnicodeDecodeError, RecursionError, MemoryError):
            return "invalid"
        except tomllib.TOMLDecodeError:
            return "unsafe"
        if classify(candidate) != "present":
            return "unsafe"
        if read_all(fd, os.fstat(fd).st_size) != original:
            return "changed"
        written = [0]
        try:
            write_tail(fd, before, path, tail, written)
        except BaseException:
            roll_back(fd, original, tail, written[0])
            raise
        return "added"
    finally:
        os.close(fd)


mode, path = sys.argv[1], sys.argv[2]
if mode == "check":
    print(check(path))
    sys.exit(0)
try:
    block = build_block(*sys.argv[3:6])
except ValueError:
    print("badentry")
    sys.exit(0)
for name in ("SIGHUP", "SIGINT", "SIGTERM"):
    signal.signal(getattr(signal, name), lambda *_: sys.exit(1))
try:
    print(append(path, block) if acquire(path) else "locked")
except Damaged:
    print("damaged")
except (ImportError, OSError):
    print("failed")
PY
    )" || out=""
    case "$out" in
        present|absent|unsafe|unsupported|invalid|added|locked|failed|badentry|changed|damaged) echo "$out" ;;
        *) if [ "$1" = append ]; then echo failed; else echo invalid; fi ;;
    esac
}

configure_codex_mcp() {
    local codex_config="$1" mcp_cmd="$2" client_bin="$3" server_bin=""
    local codex_state
    codex_state="$(codex_config_tool check "$codex_config")"
    if [ "$codex_state" = absent ]; then
        # only include the server binary if it's installed (full install mode)
        if [ -f "$INSTALL_DIR/ccp-server" ]; then server_bin="$INSTALL_DIR/ccp-server"; fi
        codex_state="$(codex_config_tool append "$codex_config" "$mcp_cmd" "$client_bin" "$server_bin")"
        if [ "$codex_state" = added ]; then
            ok "Added to $codex_config"
            return
        fi
    fi
    case "$codex_state" in
        present) ok "$codex_config already configured" ;;
        unsupported) mcp_config_warning "Codex" "Could not check $codex_config: Python 3.11+ is required; add [mcp_servers.ccp] manually" ;;
        invalid) mcp_config_warning "Codex" "Could not parse $codex_config; repair the TOML, then add [mcp_servers.ccp]" ;;
        locked) mcp_config_warning "Codex" "Another installer is updating $codex_config (lock file $codex_config.ccp-lock is held); retry later or add [mcp_servers.ccp] manually" ;;
        failed) mcp_config_warning "Codex" "Could not update $codex_config" ;;
        changed) mcp_config_warning "Codex" "Could not update $codex_config: it changed during the update and was left as it is; retry or add [mcp_servers.ccp] manually" ;;
        damaged) mcp_config_warning "Codex" "Could not update $codex_config cleanly: it changed during the update and may need manual repair; check it, then add [mcp_servers.ccp] if it is missing" ;;
        badentry) mcp_config_warning "Codex" "Could not build a valid [mcp_servers.ccp] entry for $codex_config; add it manually" ;;
        *) mcp_config_warning "Codex" "Cannot add [mcp_servers.ccp] to $codex_config safely: mcp_servers or mcp_servers.ccp is not a table that can be extended (a scalar, array or inline table); replace it, then add the entry manually" ;;
    esac
}

configure_mcp() {
    local client_bin="$INSTALL_DIR/ccp-client"
    local venv_dir="$HOME/.ccp-mcp/venv"
    local mcp_cmd="$venv_dir/bin/ccp-mcp-server"

    if [ ! -f "$mcp_cmd" ]; then
        warn "MCP bridge not found at $mcp_cmd. Skipping configuration."
        return
    fi

    local mcp_env
    mcp_env="{\"CCP_CLIENT_BIN\": \"$client_bin\"}"
    # only include server binary if it's installed (full install mode)
    if [ -f "$INSTALL_DIR/ccp-server" ]; then
        mcp_env="{\"CCP_CLIENT_BIN\": \"$client_bin\", \"CCP_SERVER_BIN\": \"$INSTALL_DIR/ccp-server\"}"
    fi

    local mcp_block
    mcp_block=$(cat <<MCPEOF
{
  "ccp": {
    "command": "$mcp_cmd",
    "env": $mcp_env
  }
}
MCPEOF
)

    echo ""
    step "Configuring MCP hosts..."

    # Codex: ~/.codex/config.toml
    local codex_config="$HOME/.codex/config.toml"
    if [ -f "$codex_config" ]; then
        configure_codex_mcp "$codex_config" "$mcp_cmd" "$client_bin"
    fi

    # Claude: ~/.claude.json
    local claude_config="$HOME/.claude.json"
    if [ -f "$claude_config" ]; then
        if ! python3 -c "import json; cfg=json.load(open('$claude_config')); exit(0 if 'ccp' in cfg.get('mcpServers',{}) else 1)" 2>/dev/null; then
            local tmp
            tmp="$(mktemp)"
            if python3 -c "
import json, os
with open('$claude_config') as f:
    cfg = json.load(f)
env = {'CCP_CLIENT_BIN': '$client_bin'}
if os.path.isfile('$INSTALL_DIR/ccp-server'):
    env['CCP_SERVER_BIN'] = '$INSTALL_DIR/ccp-server'
cfg.setdefault('mcpServers', {})['ccp'] = {'command': '$mcp_cmd', 'env': env}
with open('$tmp', 'w') as f:
    json.dump(cfg, f, indent=2)
" 2>/dev/null && mv "$tmp" "$claude_config"; then
                ok "Added to $claude_config"
            else
                mcp_config_warning "Claude" "Could not update $claude_config"
            fi
        else
            ok "$claude_config already configured"
        fi
    fi

    # Cursor: ~/.cursor/mcp.json
    local cursor_config="$HOME/.cursor/mcp.json"
    if [ -d "$HOME/.cursor" ] || [ -d "$HOME/Library/Application Support/Cursor" ]; then
        mkdir -p "$HOME/.cursor"
        if [ ! -f "$cursor_config" ]; then
            if echo "{\"mcpServers\": $mcp_block}" > "$cursor_config"; then
                ok "Created $cursor_config"
            else
                mcp_config_warning "Cursor" "Could not create $cursor_config"
            fi
        elif ! python3 -c "import json; cfg=json.load(open('$cursor_config')); exit(0 if 'ccp' in cfg.get('mcpServers',{}) else 1)" 2>/dev/null; then
            local tmp
            tmp="$(mktemp)"
            if python3 -c "
import json, os
with open('$cursor_config') as f:
    cfg = json.load(f)
env = {'CCP_CLIENT_BIN': '$client_bin'}
if os.path.isfile('$INSTALL_DIR/ccp-server'):
    env['CCP_SERVER_BIN'] = '$INSTALL_DIR/ccp-server'
cfg.setdefault('mcpServers', {})['ccp'] = {'command': '$mcp_cmd', 'env': env}
with open('$tmp', 'w') as f:
    json.dump(cfg, f, indent=2)
" 2>/dev/null && mv "$tmp" "$cursor_config"; then
                ok "Added to $cursor_config"
            else
                mcp_config_warning "Cursor" "Could not update $cursor_config"
            fi
        else
            ok "$cursor_config already configured"
        fi
    fi

    if [ ! -f "$codex_config" ] && [ ! -f "$claude_config" ] && [ ! -d "$HOME/.cursor" ]; then
        warn "No MCP host configs found. Add this to your agent's MCP config:"
        echo ""
        echo "$mcp_block"
    fi
}

# -- Uninstall ----------------------------------------------------------------

uninstall_removed() {
    ok "Removed $1"
    UNINSTALL_REMOVED=$((UNINSTALL_REMOVED + 1))
}

uninstall_failed() {
    err "$1"
    UNINSTALL_FAILED=$((UNINSTALL_FAILED + 1))
}

# Remove one installer-created regular file. A symlink was made by the user
# (the installer writes through it), so it is kept along with its target.
uninstall_file() {
    local path="$1"
    if [ -L "$path" ]; then
        warn "$path is a symlink and was kept. Remove it, and the file it points to, yourself if CCP installed it."
    elif [ -f "$path" ]; then
        if rm -f -- "$path"; then uninstall_removed "$path"; else uninstall_failed "Could not remove $path"; fi
    elif [ -e "$path" ]; then
        warn "$path is not a regular file and was kept."
    fi
}

# Mirror install_ccp_alias: the alias is ours only as a link to ccp-client.
uninstall_alias() {
    local alias_path="$INSTALL_DIR/ccp"
    if [ -L "$alias_path" ] && [ "$(readlink "$alias_path")" = "ccp-client" ]; then
        if rm -f -- "$alias_path"; then uninstall_removed "$alias_path"; else uninstall_failed "Could not remove $alias_path"; fi
    elif [ -e "$alias_path" ] || [ -L "$alias_path" ]; then
        warn "$alias_path was not created by this installer and was kept."
    fi
}

# Directories that hold user data and must survive uninstall: the client state
# (CCP_CLIENT_HOME or ~/.ccp-client) and the server data locations. Only existing
# ones matter, because a missing directory cannot lose anything.
UNINSTALL_STATE_DIRS=()
UNINSTALL_STATE_UNKNOWN=false

uninstall_collect_state() {
    local candidate resolved
    for candidate in "${CCP_CLIENT_HOME:-$HOME/.ccp-client}" "${CCP_SERVER_DATA_DIR:-}" \
        "${CCP_SERVER_HOME:-sessions}" "data"; do
        if [ -z "$candidate" ] || { [ ! -e "$candidate" ] && [ ! -L "$candidate" ]; }; then continue; fi
        if resolved="$(physical_path "$candidate")" && [ -n "$resolved" ]; then
            UNINSTALL_STATE_DIRS+=("$resolved")
        else
            UNINSTALL_STATE_UNKNOWN=true
        fi
    done
}

# Succeeds, after warning, when removing $1 could delete user data.
uninstall_holds_state() {
    local path="$1" resolved state
    if ! resolved="$(physical_path "$path")" || [ -z "$resolved" ] || [ "$UNINSTALL_STATE_UNKNOWN" = true ]; then
        warn "$path was kept because CCP data locations could not be resolved."
        return 0
    fi
    for state in ${UNINSTALL_STATE_DIRS[@]+"${UNINSTALL_STATE_DIRS[@]}"}; do
        case "$state/" in
            "$resolved/"*)
                warn "$path was kept because it holds $state, which is CCP data. Move that data elsewhere, then re-run."
                return 0
                ;;
        esac
    done
    return 1
}

# Remove only the virtual environment install_mcp_bridge creates in ~/.ccp-mcp.
# ~/.ccp-mcp/src is never removed: a release install does not create it, and it
# may be your own checkout or hold server data. The directory itself goes only
# if nothing else is left in it.
uninstall_mcp_bridge() {
    local mcp_home="$HOME/.ccp-mcp" venv src remaining
    venv="$mcp_home/venv"
    src="$mcp_home/src"
    if [ -L "$mcp_home" ]; then
        warn "$mcp_home is a symlink and was kept."
        return
    fi
    if [ ! -e "$mcp_home" ]; then return; fi
    if [ ! -d "$mcp_home" ]; then
        warn "$mcp_home is not a directory and was kept."
        return
    fi
    if [ -L "$venv" ]; then
        warn "$venv is a symlink and was kept."
    elif [ -d "$venv" ]; then
        if [ ! -f "$venv/pyvenv.cfg" ]; then
            warn "$venv was kept because it has no pyvenv.cfg, so it is not a Python virtual environment CCP can prove it made."
        elif [ -d "$venv/sessions" ] || [ -d "$venv/data" ]; then
            warn "$venv was kept because it holds a sessions or data directory, which may be server data. Move it elsewhere, then re-run."
        elif ! uninstall_holds_state "$venv"; then
            if rm -rf -- "$venv"; then uninstall_removed "$venv"; else uninstall_failed "Could not remove $venv"; fi
        fi
    elif [ -e "$venv" ]; then
        warn "$venv is not a directory and was kept."
    fi
    if [ -L "$src" ] || [ -e "$src" ]; then
        warn "$src was kept. A release install does not create it, and it may be your own checkout or hold server data. Delete it yourself if CCP made it and you no longer need it."
    fi
    remaining="$(find "$mcp_home" -mindepth 1 -maxdepth 1 -exec basename {} \; 2>/dev/null | tr '\n' ' ')"
    if [ -z "$remaining" ]; then
        if rmdir -- "$mcp_home"; then uninstall_removed "$mcp_home"; else uninstall_failed "Could not remove $mcp_home"; fi
    else
        warn "$mcp_home was kept because it holds files CCP did not create: $remaining"
    fi
}

# Drop mcpServers.ccp from a Claude or Cursor JSON config, but only when its
# command is this installer's bridge. The file is rewritten through a temporary
# file in the same directory and an atomic rename.
# Succeeds when HOME, or any component from HOME down to $1, is a symlink.
uninstall_path_has_symlink() {
    local path="$HOME" part
    local -a parts
    if [ -L "$path" ]; then return 0; fi
    case "$1" in "$HOME"/*) ;; *) return 0 ;; esac
    IFS=/ read -r -a parts <<< "${1#"$HOME"/}"
    for part in "${parts[@]}"; do
        path="$path/$part"
        if [ -L "$path" ]; then return 0; fi
    done
    return 1
}

uninstall_json_host() {
    local label="$1" config="$2" bridge="$HOME/.ccp-mcp/venv/bin/ccp-mcp-server" status mode=edit
    if [ ! -e "$config" ] && [ ! -L "$config" ]; then return; fi
    # A symlink anywhere on the path is managed elsewhere; never rewrite through it.
    if uninstall_path_has_symlink "$config"; then
        mode=check
    elif [ ! -f "$config" ]; then
        uninstall_failed "$config is not a regular file and was left unchanged. Remove the \"ccp\" entry under mcpServers by hand if present."
        return
    fi
    if ! command -v python3 &>/dev/null; then
        if grep -q '"ccp"' "$config" 2>/dev/null; then
            warn "python3 was not found, so $config was not edited. Remove the \"ccp\" entry under mcpServers by hand."
        fi
        return
    fi
    status="$(python3 - "$config" "$bridge" "$mode" <<'PY'
import json, os, sys, tempfile
path, bridge, mode = sys.argv[1], sys.argv[2], sys.argv[3]
try:
    with open(path, encoding="utf-8") as handle:
        raw = handle.read()
except OSError:
    print("ioerror")
    sys.exit(0)
try:
    cfg = json.loads(raw)
except ValueError:
    print("invalid")
    sys.exit(0)
servers = cfg.get("mcpServers") if isinstance(cfg, dict) else None
if not isinstance(servers, dict) or "ccp" not in servers:
    print("absent")
    sys.exit(0)
entry = servers["ccp"]
if not isinstance(entry, dict) or entry.get("command") != bridge:
    print("foreign")
    sys.exit(0)
if mode != "edit":
    print("manual")
    sys.exit(0)
del servers["ccp"]
text = json.dumps(cfg, indent=2, ensure_ascii=False)
if raw.endswith("\n"):
    text += "\n"
temp = ""
try:
    fd, temp = tempfile.mkstemp(dir=os.path.dirname(path), prefix=os.path.basename(path) + ".ccp-")
    with os.fdopen(fd, "w", encoding="utf-8") as handle:
        handle.write(text)
        handle.flush()
        os.fsync(handle.fileno())
    os.chmod(temp, os.stat(path).st_mode & 0o7777)
    os.replace(temp, path)
except OSError:
    if temp:
        try:
            os.unlink(temp)
        except OSError:
            pass
    print("ioerror")
    sys.exit(0)
print("removed")
PY
    )" || status="ioerror"
    case "$status" in
        removed) uninstall_removed "the ccp entry from $label config $config" ;;
        absent) ;;
        foreign) warn "The ccp entry in $config does not point at $bridge and was kept." ;;
        manual) warn "$config is, or sits under, a symlink, so it was not edited. Remove the \"ccp\" entry under mcpServers by hand." ;;
        invalid) warn "$config is not valid JSON and was left unchanged. Remove the \"ccp\" entry under mcpServers by hand if present." ;;
        ioerror) if [ "$mode" = check ]; then
                warn "$config could not be read, and it is, or sits under, a symlink, so it was not edited. Remove the \"ccp\" entry under mcpServers by hand if present."
            else
                uninstall_failed "Could not read or update $config; it was left unchanged. Fix the problem and re-run, or remove the \"ccp\" entry under mcpServers by hand."
            fi ;;
        *) uninstall_failed "Could not read or update $config; it was left unchanged. Fix the problem and re-run, or remove the \"ccp\" entry under mcpServers by hand." ;;
    esac
}

# The Codex config is TOML; this installer never rewrites it.
uninstall_codex_notice() {
    local codex_config="$HOME/.codex/config.toml"
    if [ -f "$codex_config" ] && grep -q "mcp_servers.ccp" "$codex_config" 2>/dev/null; then
        warn "Codex: $codex_config still has a CCP entry. Remove the [mcp_servers.ccp] table, and the [mcp_servers.ccp.env] table under it, by hand. This installer does not edit TOML."
    fi
}

uninstall_data_notice() {
    local client_home="${CCP_CLIENT_HOME:-$HOME/.ccp-client}" quoted
    printf -v quoted '%q' "$client_home"
    echo ""
    info "Not removed. These hold your enrollments, keys and server data:"
    echo "  Client state (enrollment profiles, certificates, keys, workspaces):"
    printf '    %s\n' "$client_home"
    printf '    %brm -rf %s%b\n' "$DIM" "$quoted" "$RESET"
    echo "  Server data (database, CA and server keys): ccp-server uses"
    echo "  sessions/<session-slug> under the directory it was started from."
    echo "  CCP_SERVER_HOME replaces sessions, and CCP_SERVER_DATA_DIR overrides both."
    echo "  An existing ./data (older versions) or matching session directory is reused."
    if [ -n "${CCP_SERVER_DATA_DIR:-}" ]; then printf '    CCP_SERVER_DATA_DIR=%s\n' "$CCP_SERVER_DATA_DIR"; fi
    if [ -n "${CCP_SERVER_HOME:-}" ]; then printf '    CCP_SERVER_HOME=%s\n' "$CCP_SERVER_HOME"; fi
    echo "  Docker server (started by --docker): container ccp-server, volume ccp-server-data."
    echo -e "    ${DIM}docker stop ccp-server && docker rm ccp-server${RESET}"
    echo -e "    ${DIM}docker volume rm ccp-server-data${RESET}  (deletes the server database and keys)"
}

uninstall_ccp() {
    step "Uninstalling CCP from ${BOLD}$INSTALL_DIR${RESET}"
    uninstall_collect_state
    uninstall_file "$INSTALL_DIR/ccp-client"
    uninstall_file "$INSTALL_DIR/ccp-server"
    uninstall_file "$INSTALL_DIR/ccp-nightly.json"
    uninstall_alias
    uninstall_json_host "Claude" "$HOME/.claude.json"
    uninstall_json_host "Cursor" "$HOME/.cursor/mcp.json"
    uninstall_codex_notice
    uninstall_mcp_bridge
    echo ""
    if [ "$UNINSTALL_FAILED" -gt 0 ]; then
        err "Uninstall finished with errors; see above. Re-run it after fixing them."
    elif [ "$UNINSTALL_REMOVED" -eq 0 ]; then
        ok "Nothing left to remove."
    else
        ok "CCP uninstalled."
    fi
    uninstall_data_notice
    [ "$UNINSTALL_FAILED" -eq 0 ]
}

# ── Main ─────────────────────────────────────────────────────────────────────

banner

if [ "$UNINSTALL" = true ]; then
    uninstall_ccp
    exit $?
fi

# ── Docker mode ──────────────────────────────────────────────────────────────

if [ "$MODE" = "docker" ]; then
    if ! command -v docker &>/dev/null; then
        err "Docker is not installed."
        info "Get it at ${BOLD}https://docs.docker.com/get-docker/${RESET}"
        exit 1
    fi

    if ! docker info >/dev/null 2>&1; then
        err "Docker is not running or the current user cannot access it."
        exit 1
    fi

    image="ghcr.io/${REPO}:latest"
    if [ "$CHANNEL" = "nightly" ]; then
        image="ghcr.io/${REPO}:${RELEASE_VERSION:-nightly}"
    fi
    if [ "$FROM_SOURCE" = false ]; then
        step "Pulling CCP server image..."
        if ! docker pull "$image"; then
            err "Image pull failed. Check the registry, network, or platform error above."
            info "To build from source explicitly, run:"
            printf '  curl -fsSL %s/install.sh | bash -s -- --docker --from-source --session %q\n' "$REPO_RAW" "$SESSION_NAME"
            exit 1
        fi
    fi

    if [ "$FROM_SOURCE" = true ]; then
        build_dir="$REPO_ROOT"
        temporary_build=false
        if [ ! -f "$build_dir/Dockerfile" ] || [ ! -f "$build_dir/Cargo.toml" ]; then
            if ! command -v git &>/dev/null; then
                err "Git is required to build outside a checkout. Install Git or run from the repo root."
                exit 1
            fi
            build_dir="$(mktemp -d)" || exit 1
            temporary_build=true
            if ! git clone --depth 1 "https://github.com/${REPO}.git" "$build_dir"; then
                rm -rf "$build_dir"
                err "Could not download the source."
                exit 1
            fi
        fi
        image="cephalopod-coordination-protocol-server:local"
        step "Building CCP server image..."
        if ! docker build -t "$image" "$build_dir"; then
            if [ "$temporary_build" = true ]; then rm -rf "$build_dir"; fi
            err "Container build failed."
            exit 1
        fi
        if [ "$temporary_build" = true ]; then rm -rf "$build_dir"; fi
    fi

    step "Starting CCP server ${BOLD}(session: $SESSION_NAME)${RESET}"
    if ! docker run -d --name ccp-server --restart unless-stopped \
        -e "CCP_SESSION_NAME=$SESSION_NAME" \
        -e "CCP_AUTH_PORT=${CCP_AUTH_PORT:-1337}" \
        -e "CCP_MTLS_PORT=${CCP_MTLS_PORT:-1338}" \
        -e "CCP_ADVERTISE_HOST=${CCP_ADVERTISE_HOST:-127.0.0.1}" \
        -p "${CCP_PUBLISH_HOST:-127.0.0.1}:${CCP_AUTH_PORT:-1337}:${CCP_AUTH_PORT:-1337}" \
        -p "${CCP_PUBLISH_HOST:-127.0.0.1}:${CCP_MTLS_PORT:-1338}:${CCP_MTLS_PORT:-1338}" \
        -v ccp-server-data:/var/lib/ccp/server "$image"; then
        err "Container startup failed. Check Docker's error above; stop and remove an existing ccp-server container before retrying."
        exit 1
    fi

    echo ""
    ok "Server container started."
    echo ""
    info "Check logs for enrollment tokens:"
    echo -e "  ${DIM}docker logs -f ccp-server${RESET}"
    echo ""
    info "Issue tokens:"
    echo -e "  ${DIM}docker exec ccp-server server issue-token \"$SESSION_NAME\" read${RESET}"
    echo -e "  ${DIM}docker exec ccp-server server issue-token \"$SESSION_NAME\" read_write${RESET}"
    echo ""
    info "Stop and remove the container (the data volume is retained):"
    echo -e "  ${DIM}docker stop ccp-server && docker rm ccp-server${RESET}"
    exit 0
fi

# ── Binary install ───────────────────────────────────────────────────────────

# Decide on the MCP bridge first, so release downloads fetch and verify its
# archive only when it will be installed.
INSTALL_MCP=false
if [ "$MODE" = "client" ]; then
    INSTALL_MCP=true
elif [ "$MODE" = "both" ]; then
    printf '%b>>>%b Install the MCP bridge for Claude/Cursor/Codex? [y/N] ' "$CYAN" "$RESET"
    read -r INSTALL_MCP_ANSWER </dev/tty 2>/dev/null || INSTALL_MCP_ANSWER="n"
    case "$INSTALL_MCP_ANSWER" in
        [yY]|[yY][eE][sS]) INSTALL_MCP=true ;;
    esac
    echo ""
fi

step "Installing CCP ${BOLD}($MODE)${RESET}"
mkdir -p "$INSTALL_DIR" || exit 1

# Validate every destination before publishing the first component, for all
# channels and source builds. Publication rechecks each resolved destination.
resolve_destination "$INSTALL_DIR/ccp-client" >/dev/null || exit 1
if [ "$MODE" = "both" ]; then
    resolve_destination "$INSTALL_DIR/ccp-server" >/dev/null || exit 1
fi
resolve_destination "$INSTALL_DIR/ccp-nightly.json" >/dev/null || exit 1

if [ "$FROM_SOURCE" = true ]; then
    build_from_source || exit 1
elif [ "$CHANNEL" = "nightly" ]; then
    install_nightly_binaries || exit 1
else
    prepare_stable_release || exit 1
    if [ "$MODE" = "client" ]; then
        download_binary "ccp-client" "$INSTALL_DIR/ccp-client" || exit 1
    else
        download_binary "ccp-server" "$INSTALL_DIR/ccp-server" || exit 1
        download_binary "ccp-client" "$INSTALL_DIR/ccp-client" || exit 1
    fi
fi

if [ "$CHANNEL" != "nightly" ]; then
    rm -f "$INSTALL_DIR/ccp-nightly.json" || exit 1
fi

echo ""
if [ "$MODE" = "client" ]; then
    ok "Installed ${BOLD}ccp-client${RESET}${GREEN} -> $INSTALL_DIR/ccp-client${RESET}"
else
    ok "Installed ${BOLD}ccp-server${RESET}${GREEN} -> $INSTALL_DIR/ccp-server${RESET}"
    ok "Installed ${BOLD}ccp-client${RESET}${GREEN} -> $INSTALL_DIR/ccp-client${RESET}"
fi

# The alias always executes the same installed client and follows its updates.
install_ccp_alias

# ── MCP bridge ───────────────────────────────────────────────────────────────

if [ "$INSTALL_MCP" = true ]; then
    install_mcp_bridge || exit 1
    configure_mcp
elif [ "$MODE" = "both" ]; then
    info "MCP bridge skipped. Run with ${DIM}--client${RESET} later to set up MCP."
fi

ensure_path

echo ""
if [ "$MODE" = "client" ]; then
    info "Enroll with a server:"
    echo -e "  ${DIM}ccp-client enroll --redeem-url <url> --token <token>${RESET}"
else
    info "Start a server:"
    echo -e "  ${DIM}ccp-server <session-name>${RESET}"
    echo ""
    info "Enroll a client:"
    echo -e "  ${DIM}ccp-client enroll --redeem-url <url> --token <token>${RESET}"
fi
echo ""
if [ -n "$MCP_CONFIG_WARNINGS" ]; then
    echo -e "${BOLD}${GREEN}Done, with warnings: $MCP_CONFIG_WARNINGS; see above.${RESET}"
else
    echo -e "${BOLD}${GREEN}Done.${RESET}"
fi
