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
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}" 2>/dev/null)" 2>/dev/null && pwd || echo "")"
INSTALL_DIR="${HOME}/.local/bin"
SESSION_NAME="my-session"
MODE="both"
FROM_SOURCE=false

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
        --install-dir) INSTALL_DIR="$2"; shift 2 ;;
        --session)     SESSION_NAME="$2"; shift 2 ;;
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
    if command -v curl &>/dev/null; then
        curl -fsSL "$url" -o "$dest" && ok=true
    elif command -v wget &>/dev/null; then
        wget -q "$url" -O "$dest" && ok=true
    else
        err "Need curl or wget to download binaries."
        exit 1
    fi

    if [ "$ok" = false ]; then
        echo ""
        err "Download failed. No release binaries found for ${BOLD}$platform${RESET}."
        warn "This usually means there's no published release yet."
        echo ""
        info "Try installing from source instead:"
        echo -e "  ${DIM}curl -fsSL ${REPO_RAW}/install.sh | bash -s -- --from-source${RESET}"
        exit 1
    fi
    chmod +x "$dest"
}

build_from_source() {
    if ! command -v cargo &>/dev/null; then
        err "Rust is not installed."
        info "Get it at ${BOLD}https://rustup.rs${RESET}"
        exit 1
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

    step "Building ${BOLD}release${RESET}${CYAN} binaries...${RESET}"
    if ! (cd "$build_dir" && cargo build --release); then
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
        rm -rf "$build_dir"
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
        warn "Python 3 is required for the MCP bridge. Skipping."
        return
    fi

    local mcp_home="$HOME/.ccp-mcp"
    local venv_dir="$mcp_home/venv"
    local mcp_src

    # find the mcp package — either in the repo or clone it
    if [ -n "$REPO_ROOT" ] && [ -d "$REPO_ROOT/mcp" ]; then
        mcp_src="$REPO_ROOT/mcp"
    else
        mcp_src="$mcp_home/src"
        if [ ! -d "$mcp_src" ]; then
            step "Downloading MCP bridge..."
            mkdir -p "$mcp_home"
            git clone --depth 1 "https://github.com/${REPO}.git" "$mcp_home/repo" 2>/dev/null || {
                warn "Could not download MCP bridge. Install from the repo manually."
                return
            }
            mv "$mcp_home/repo/mcp" "$mcp_src"
            rm -rf "$mcp_home/repo"
        fi
    fi

    step "Installing MCP bridge..."
    if [ ! -d "$venv_dir" ]; then
        python3 -m venv "$venv_dir"
    fi
    "$venv_dir/bin/pip" install --quiet --upgrade pip
    "$venv_dir/bin/pip" install --quiet -e "$mcp_src"
    ok "MCP bridge installed in $venv_dir"
}

configure_mcp() {
    local client_bin="$INSTALL_DIR/ccp-client"
    local venv_dir="$HOME/.ccp-mcp/venv"
    local mcp_cmd="$venv_dir/bin/ccp-mcp-server"

    if [ ! -f "$mcp_cmd" ]; then
        warn "MCP bridge not found at $mcp_cmd — skipping config"
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
        if ! grep -q "mcp_servers.ccp" "$codex_config" 2>/dev/null; then
            local codex_env="CCP_CLIENT_BIN = \"$client_bin\""
            if [ -f "$INSTALL_DIR/ccp-server" ]; then
                codex_env="$codex_env
CCP_SERVER_BIN = \"$INSTALL_DIR/ccp-server\""
            fi
            cat >> "$codex_config" <<TOMLEOF

[mcp_servers.ccp]
command = "$mcp_cmd"

[mcp_servers.ccp.env]
$codex_env
TOMLEOF
            ok "Added to $codex_config"
        else
            ok "$codex_config already configured"
        fi
    fi

    # Claude: ~/.claude.json
    local claude_config="$HOME/.claude.json"
    if [ -f "$claude_config" ]; then
        if ! python3 -c "import json; cfg=json.load(open('$claude_config')); exit(0 if 'ccp' in cfg.get('mcpServers',{}) else 1)" 2>/dev/null; then
            local tmp
            tmp="$(mktemp)"
            python3 -c "
import json, os
with open('$claude_config') as f:
    cfg = json.load(f)
env = {'CCP_CLIENT_BIN': '$client_bin'}
if os.path.isfile('$INSTALL_DIR/ccp-server'):
    env['CCP_SERVER_BIN'] = '$INSTALL_DIR/ccp-server'
cfg.setdefault('mcpServers', {})['ccp'] = {'command': '$mcp_cmd', 'env': env}
with open('$tmp', 'w') as f:
    json.dump(cfg, f, indent=2)
" 2>/dev/null && mv "$tmp" "$claude_config" && ok "Added to $claude_config" || warn "Could not update $claude_config"
        else
            ok "$claude_config already configured"
        fi
    fi

    # Cursor: ~/.cursor/mcp.json
    local cursor_config="$HOME/.cursor/mcp.json"
    if [ -d "$HOME/.cursor" ] || [ -d "$HOME/Library/Application Support/Cursor" ]; then
        mkdir -p "$HOME/.cursor"
        if [ ! -f "$cursor_config" ]; then
            echo "{\"mcpServers\": $mcp_block}" > "$cursor_config"
            ok "Created $cursor_config"
        elif ! python3 -c "import json; cfg=json.load(open('$cursor_config')); exit(0 if 'ccp' in cfg.get('mcpServers',{}) else 1)" 2>/dev/null; then
            local tmp
            tmp="$(mktemp)"
            python3 -c "
import json, os
with open('$cursor_config') as f:
    cfg = json.load(f)
env = {'CCP_CLIENT_BIN': '$client_bin'}
if os.path.isfile('$INSTALL_DIR/ccp-server'):
    env['CCP_SERVER_BIN'] = '$INSTALL_DIR/ccp-server'
cfg.setdefault('mcpServers', {})['ccp'] = {'command': '$mcp_cmd', 'env': env}
with open('$tmp', 'w') as f:
    json.dump(cfg, f, indent=2)
" 2>/dev/null && mv "$tmp" "$cursor_config" && ok "Added to $cursor_config" || warn "Could not update $cursor_config"
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

# ── Main ─────────────────────────────────────────────────────────────────────

banner

# ── Docker mode ──────────────────────────────────────────────────────────────

if [ "$MODE" = "docker" ]; then
    if ! command -v docker &>/dev/null; then
        err "Docker is not installed."
        info "Get it at ${BOLD}https://docs.docker.com/get-docker/${RESET}"
        exit 1
    fi

    step "Pulling CCP server image..."
    docker pull "ghcr.io/${REPO}:latest" 2>/dev/null || {
        warn "No prebuilt image found. Building from Dockerfile..."
        if [ ! -f "docker-compose.yml" ]; then
            err "Run this from the repo root, or use the default install mode instead."
            exit 1
        fi
        docker compose build
    }

    step "Starting CCP server ${BOLD}(session: $SESSION_NAME)${RESET}"
    CCP_SESSION_NAME="$SESSION_NAME" docker compose up -d

    echo ""
    ok "Server running."
    echo ""
    info "Check logs for enrollment tokens:"
    echo -e "  ${DIM}docker compose logs -f ccp-server${RESET}"
    echo ""
    info "Issue tokens:"
    echo -e "  ${DIM}docker compose exec ccp-server server issue-token $SESSION_NAME read${RESET}"
    echo -e "  ${DIM}docker compose exec ccp-server server issue-token $SESSION_NAME read_write${RESET}"
    echo ""
    info "Stop:"
    echo -e "  ${DIM}docker compose down${RESET}"
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
