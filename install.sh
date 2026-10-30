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
CHANNEL="stable"
RELEASE_VERSION=""
RELEASE_STAGE=""

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
        --channel|--version)
            if [ $# -lt 2 ] || [[ "$2" == --* ]] || [ -z "$2" ]; then
                err "$1 requires a value."; exit 1
            fi
            if [ "$1" = "--channel" ]; then CHANNEL="$2"; else RELEASE_VERSION="$2"; fi
            shift 2
            ;;
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
  --channel <channel>    stable (default) or nightly
  --version <tag>        Pin an exact nightly release (requires --channel nightly)
EOF
            exit 0
            ;;
        *) err "Unknown option: $1"; exit 1 ;;
    esac
done

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
    local assets=("ccp-client-$platform" "ccp-mcp.tar.gz" "build-info.json")
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
        if ! chmod +x "$RELEASE_STAGE/$asset-$platform" || ! cp "$RELEASE_STAGE/$asset-$platform" "$INSTALL_DIR/$asset"; then
            err "Could not install $asset."
            exit 1
        fi
    done
    cp "$RELEASE_STAGE/build-info.json" "$INSTALL_DIR/ccp-nightly.json" || exit 1
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
    if ! (cd "$build_dir" && cargo build --release --locked "${packages[@]}"); then
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
        warn "Python 3 is required for the MCP bridge. Skipping."
        return
    fi

    local mcp_home="$HOME/.ccp-mcp"
    local venv_dir="$mcp_home/venv"
    local mcp_src

    if [ "$CHANNEL" = "nightly" ]; then
        step "Installing MCP bridge from $RELEASE_VERSION"
        if [ ! -d "$venv_dir" ] && ! python3 -m venv "$venv_dir"; then
            err "Could not create the MCP environment."; exit 1
        fi
        if ! "$venv_dir/bin/pip" install --quiet --upgrade --force-reinstall "$RELEASE_STAGE/ccp-mcp.tar.gz"; then
            err "Nightly MCP installation failed."; exit 1
        fi
        ok "MCP bridge installed in $venv_dir"
        return
    fi

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
                warn "Could not update $claude_config"
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
            echo "{\"mcpServers\": $mcp_block}" > "$cursor_config"
            ok "Created $cursor_config"
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
                warn "Could not update $cursor_config"
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

# ── Main ─────────────────────────────────────────────────────────────────────

banner

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

step "Installing CCP ${BOLD}($MODE)${RESET}"
mkdir -p "$INSTALL_DIR"

if [ "$FROM_SOURCE" = true ]; then
    build_from_source
elif [ "$CHANNEL" = "nightly" ]; then
    install_nightly_binaries
else
    if [ "$MODE" = "client" ]; then
        download_binary "ccp-client" "$INSTALL_DIR/ccp-client"
    else
        download_binary "ccp-server" "$INSTALL_DIR/ccp-server"
        download_binary "ccp-client" "$INSTALL_DIR/ccp-client"
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

if [ "$MODE" = "client" ]; then
    install_mcp_bridge
    configure_mcp
elif [ "$MODE" = "both" ]; then
    echo ""
    printf '%b>>>%b Install the MCP bridge for Claude/Cursor/Codex? [y/N] ' "$CYAN" "$RESET"
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
