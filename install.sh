#!/usr/bin/env bash
# Cephalopod Coordination Protocol installer.
#
# curl -fsSL https://raw.githubusercontent.com/squid-proxy-lovers/ccp/main/install.sh | bash
#
# Copyright (C) 2026 Squid Proxy Lovers
# SPDX-License-Identifier: AGPL-3.0-or-later

set -euo pipefail

REPO="squid-proxy-lovers/ccp"
REPO_RAW="https://raw.githubusercontent.com/squid-proxy-lovers/ccp/main"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}" 2>/dev/null)" 2>/dev/null && pwd || echo "")"
INSTALL_DIR="${HOME}/.local/bin"
SESSION_NAME="my-session"
MODE="both"
FROM_SOURCE=false
INSTALL_TMP_DIR=""
cleanup_install() {
    if [ -n "$INSTALL_TMP_DIR" ]; then rm -rf "$INSTALL_TMP_DIR"; fi
}
trap cleanup_install EXIT

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
        --client)      MODE="client"; shift ;;
        --docker)      MODE="docker"; shift ;;
        --install-dir) [[ $# -ge 2 && -n "$2" ]] || { err "--install-dir needs a path"; exit 64; }; INSTALL_DIR="$2"; shift 2 ;;
        --session)     [[ $# -ge 2 && -n "$2" ]] || { err "--session needs a name"; exit 64; }; SESSION_NAME="$2"; shift 2 ;;
        --from-source) FROM_SOURCE=true; shift ;;
        -h|--help)
            cat <<EOF
Usage: bash install.sh [--client | --docker] [OPTIONS]

  (default)        Install server + client
  --client         Client only + auto-configure MCP for Claude/Cursor/Codex
  --docker         Pull and start the server container

Options:
  --install-dir <path>   Binary install directory (default: ~/.local/bin)
  --session <name>       Docker session name (default: my-session)
  --from-source          Build from repo instead of downloading release binaries
EOF
            exit 0
            ;;
        *) err "Unknown option: $1"; exit 1 ;;
    esac
done

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

download_binary() {
    local name="$1" dest="$2"
    local platform
    platform="$(detect_platform)"
    local url="https://github.com/${REPO}/releases/latest/download/${name}-${platform}"

    step "Downloading ${BOLD}$name${RESET}${CYAN} for $platform${RESET}"
    local ok=false
    local temporary
    temporary="$(mktemp "${dest}.download.XXXXXX")"
    if command -v curl &>/dev/null; then
        curl -fsSL "$url" -o "$temporary" && ok=true
    elif command -v wget &>/dev/null; then
        wget -q "$url" -O "$temporary" && ok=true
    else
        rm -f "$temporary"
        err "Need curl or wget to download binaries."
        exit 1
    fi

    if [ "$ok" = false ]; then
        rm -f "$temporary"
        echo ""
        err "Download failed. No release binaries found for ${BOLD}$platform${RESET}."
        warn "This usually means there's no published release yet."
        echo ""
        info "Try installing from source instead:"
        echo -e "  ${DIM}curl -fsSL ${REPO_RAW}/install.sh | bash -s -- --from-source${RESET}"
        exit 1
    fi
    chmod +x "$temporary"
    mv "$temporary" "$dest"
}

build_from_source() {
    if ! command -v cargo &>/dev/null; then
        err "Rust is not installed."
        info "Get it at ${BOLD}https://rustup.rs${RESET}"
        exit 1
    fi

    local build_dir="$REPO_ROOT"

    if [ -z "$build_dir" ] || [ ! -f "$build_dir/Cargo.toml" ]; then
        INSTALL_TMP_DIR="$(mktemp -d)"
        build_dir="$INSTALL_TMP_DIR/repo"
        step "Cloning repo..."
        if ! git clone --depth 1 "https://github.com/${REPO}.git" "$build_dir" 2>&1; then
            err "Clone failed. Run this from inside the repo instead."
            exit 1
        fi
    fi

    step "Building ${BOLD}release${RESET}${CYAN} binaries...${RESET}"
    if ! (cd "$build_dir" && cargo build --locked --release -p client -p server); then
        err "Build failed."
        exit 1
    fi

    mkdir -p "$INSTALL_DIR"
    if [ "$MODE" = "client" ]; then
        cp "$build_dir/target/release/client" "$INSTALL_DIR/ccp-client"
        chmod +x "$INSTALL_DIR/ccp-client"
    else
        cp "$build_dir/target/release/server" "$INSTALL_DIR/ccp-server"
        cp "$build_dir/target/release/client" "$INSTALL_DIR/ccp-client"
        chmod +x "$INSTALL_DIR/ccp-server" "$INSTALL_DIR/ccp-client"
    fi

    if [ "$build_dir" != "$REPO_ROOT" ]; then
        cleanup_install
        INSTALL_TMP_DIR=""
    fi
}

ensure_path() {
    if ! echo "$PATH" | tr ':' '\n' | grep -qx "$INSTALL_DIR"; then
        echo ""
        warn "Add to your shell profile:"
        printf '  export PATH=%q:"$PATH"\n' "$INSTALL_DIR"
    fi
}

install_mcp_bridge() {
    if ! command -v python3 &>/dev/null; then
        warn "Python 3 is required for the MCP bridge. Skipping."
        return
    fi

    local venv_dir="$HOME/.ccp-mcp/venv"
    local mcp_src
    if [ -n "$REPO_ROOT" ] && [ -d "$REPO_ROOT/mcp" ]; then
        mcp_src="$REPO_ROOT/mcp"
    else
        INSTALL_TMP_DIR="$(mktemp -d)"
        step "Downloading current MCP bridge source..."
        git clone --depth 1 "https://github.com/${REPO}.git" "$INSTALL_TMP_DIR/repo"
        mcp_src="$INSTALL_TMP_DIR/repo/mcp"
    fi
    step "Installing MCP bridge..."
    python3 -m venv "$venv_dir"
    "$venv_dir/bin/python" -m pip install --quiet --upgrade "$mcp_src"
    "$venv_dir/bin/python" -c 'from ccp_mcp_server.server import master_instructions'
    cleanup_install
    INSTALL_TMP_DIR=""
    ok "MCP bridge installed in $venv_dir"
}

configure_mcp() {
    local mcp_cmd="$HOME/.ccp-mcp/venv/bin/ccp-mcp-server"
    [ -x "$mcp_cmd" ] || { warn "MCP bridge is unavailable; skipping configuration"; return; }
    step "Configuring MCP hosts..."
    python3 - "$INSTALL_DIR" "$mcp_cmd" <<'PYCONFIG'
import json
import os
from pathlib import Path
import stat
import sys
import tempfile

install_dir = Path(sys.argv[1]).expanduser().resolve()
mcp_command = sys.argv[2]
config_env = {"CCP_CLIENT_BIN": str(install_dir / "ccp-client")}
if (install_dir / "ccp-server").is_file():
    config_env["CCP_SERVER_BIN"] = str(install_dir / "ccp-server")
for name in ("CCP_SERVER_URL", "CCP_CLIENT_KEY"):
    if name in os.environ:
        config_env[name] = os.environ[name]
config = {"command": mcp_command, "env": config_env}
home = Path.home()
configured = False

# Pass paths and values as data, including quotes, backslashes, and newlines.
# Atomic JSON replacement preserves existing config entries and permissions.
def write_json(path):
    existing = json.loads(path.read_text()) if path.exists() else {}
    entry = existing.setdefault("mcpServers", {}).setdefault("ccp", {})
    entry["command"] = mcp_command
    entry.setdefault("env", {}).update(config_env)
    path.parent.mkdir(parents=True, exist_ok=True)
    permissions = stat.S_IMODE(path.stat().st_mode) if path.exists() else 0o600
    descriptor, temporary = tempfile.mkstemp(dir=path.parent)
    try:
        with os.fdopen(descriptor, "w") as stream:
            json.dump(existing, stream, indent=2)
            stream.write("\n")
        os.chmod(temporary, permissions)
        os.replace(temporary, path)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)
    print(f"Configured {path}")

codex_path = home / ".codex/config.toml"
if codex_path.exists():
    text = codex_path.read_text()
    if "[mcp_servers.ccp]" not in text:
        with codex_path.open("a") as stream:
            stream.write("\n[mcp_servers.ccp]\ncommand = " + json.dumps(mcp_command) + "\n")
            stream.write("\n[mcp_servers.ccp.env]\n")
            for key, value in config_env.items():
                stream.write(key + " = " + json.dumps(value) + "\n")
        print(f"Configured {codex_path}")
    else:
        print(f"Existing CCP config retained in {codex_path}; update its env if your endpoint/key changed")
    configured = True
claude_path = home / ".claude.json"
if claude_path.exists():
    write_json(claude_path)
    configured = True
cursor_path = home / ".cursor/mcp.json"
if cursor_path.parent.exists() or (home / "Library/Application Support/Cursor").exists():
    write_json(cursor_path)
    configured = True
if not configured:
    print("Add the following to your MCP host configuration:")
    print(json.dumps({"mcpServers": {"ccp": config}}, indent=2))
PYCONFIG
}

# ── Main ─────────────────────────────────────────────────────────────────────

banner

# ── Docker mode ──────────────────────────────────────────────────────────────

if [ "$MODE" = "docker" ]; then
    if ! command -v docker &>/dev/null; then
        err "Docker is not installed."
        info "Get it at ${BOLD}https://docs.docker.com/get-docker/${RESET}"
        exit 1
    fi

    if [ ! -f "$REPO_ROOT/docker-compose.yml" ]; then
        err "Docker mode requires a repository checkout. Run install.sh from that checkout."
        exit 1
    fi
    local_image="cephalopod-coordination-protocol-server:latest"
    compose_file="$REPO_ROOT/docker-compose.yml"
    step "Pulling CCP server image..."
    if docker pull "ghcr.io/${REPO}:latest"; then
        local_image="ghcr.io/${REPO}:latest"
    else
        warn "No prebuilt image found. Building from Dockerfile..."
        CCP_IMAGE="$local_image" docker compose -f "$compose_file" build
    fi
    step "Starting CCP server (session: $SESSION_NAME)"
    CCP_IMAGE="$local_image" CCP_SESSION_NAME="$SESSION_NAME" docker compose -f "$compose_file" up -d --no-build
    ok "Server running."
    info "Dashboard: ${CCP_HTTP_BASE_URL:-http://127.0.0.1:${CCP_HTTP_PORT:-1338}}/admin"
    info "View logs: docker compose -f $compose_file logs -f ccp-server"
    info "Stop: docker compose -f $compose_file down"
    exit 0
fi

# ── Binary install ───────────────────────────────────────────────────────────

step "Installing CCP ${BOLD}($MODE)${RESET}"
mkdir -p "$INSTALL_DIR"

if [ "$FROM_SOURCE" = true ]; then
    build_from_source
else
    if [ "$MODE" = "client" ]; then
        download_binary "ccp-client" "$INSTALL_DIR/ccp-client"
    else
        download_binary "ccp-server" "$INSTALL_DIR/ccp-server"
        download_binary "ccp-client" "$INSTALL_DIR/ccp-client"
    fi
fi

echo ""
if [ "$MODE" = "client" ]; then
    ok "Installed ${BOLD}ccp-client${RESET}${GREEN} -> $INSTALL_DIR/ccp-client${RESET}"
else
    ok "Installed ${BOLD}ccp-server${RESET}${GREEN} -> $INSTALL_DIR/ccp-server${RESET}"
    ok "Installed ${BOLD}ccp-client${RESET}${GREEN} -> $INSTALL_DIR/ccp-client${RESET}"
fi

# ── MCP bridge ───────────────────────────────────────────────────────────────

if [ "$MODE" = "client" ]; then
    install_mcp_bridge
    configure_mcp
elif [ "$MODE" = "both" ]; then
    echo ""
    printf "${CYAN}>>>${RESET} Install the MCP bridge for Claude/Cursor/Codex? [y/N] "
    read -r INSTALL_MCP_ANSWER </dev/tty 2>/dev/null || INSTALL_MCP_ANSWER="n"
    case "$INSTALL_MCP_ANSWER" in
        [yY]|[yY][eE][sS])
            install_mcp_bridge
            configure_mcp
            ;;
        *)
            info "Skipped. Run with ${DIM}--client${RESET} later to set up MCP."
            ;;
    esac
fi

ensure_path

echo ""
echo -e "${BOLD}${GREEN}Done.${RESET}"
echo ""
if [ "$MODE" = "client" ]; then
    info "Set CCP_CLIENT_KEY for your server, then subscribe:"
    echo -e "  ${DIM}ccp-client subscribe-all --server <http-url>${RESET}"
else
    info "Start a server:"
    echo -e "  ${DIM}ccp-server <session-name>${RESET}"
    echo ""
    info "Set CCP_CLIENT_KEY for your server, then subscribe a client:"
    echo -e "  ${DIM}ccp-client subscribe-all --server <http-url>${RESET}"
fi
echo ""
