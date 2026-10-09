#!/bin/sh
set -eu

DEFAULT_SERVER_URL="http://127.0.0.1:1338"
SERVER_URL="${CCP_SERVER_URL:-$DEFAULT_SERVER_URL}"
SERVER_URL="${SERVER_URL%/}"
: "${CCP_CLIENT_KEY:?Set CCP_CLIENT_KEY to the server client key}"
CLIENT_KEY="$CCP_CLIENT_KEY"
export CCP_CLIENT_KEY
export CCP_SERVER_URL="$SERVER_URL"
INSTALL_DIR="${CCP_INSTALL_DIR:-$HOME/.local/bin}"

case "$(uname -s)" in
    Linux) os="linux" ;;
    Darwin) os="darwin" ;;
    *) echo "Unsupported operating system" >&2; exit 1 ;;
esac
case "$(uname -m)" in
    x86_64|amd64) arch="x86_64" ;;
    arm64|aarch64) arch="aarch64" ;;
    *) echo "Unsupported CPU architecture" >&2; exit 1 ;;
esac

command -v curl >/dev/null 2>&1 || { echo "curl is required" >&2; exit 1; }
command -v python3 >/dev/null 2>&1 || { echo "python3 is required" >&2; exit 1; }

mkdir -p "$INSTALL_DIR"
staging=$(mktemp -d "$INSTALL_DIR/.ccp-download.XXXXXX")
trap 'rm -rf "$staging"' EXIT HUP INT TERM
curl -fsSL "$SERVER_URL/downloads/ccp-client-${os}-${arch}" -o "$staging/ccp-client"
curl -fsSL "$SERVER_URL/ccp-update" -o "$staging/ccp-update"
chmod 0755 "$staging/ccp-client" "$staging/ccp-update"
mv "$staging/ccp-client" "$INSTALL_DIR/ccp-client"
mv "$staging/ccp-update" "$INSTALL_DIR/ccp-update"
"$INSTALL_DIR/ccp-client" subscribe-all --server "$SERVER_URL"

MCP_VENV="$HOME/.ccp-client/mcp-venv"
python3 -m venv "$MCP_VENV"
"$MCP_VENV/bin/pip" install --upgrade --force-reinstall --no-cache-dir "$SERVER_URL/downloads/ccp-mcp.tar.gz"
"$MCP_VENV/bin/python" -c 'from ccp_mcp_server.server import master_instructions'
MCP_CMD="$MCP_VENV/bin/ccp-mcp-server"

if command -v codex >/dev/null 2>&1; then
    codex mcp remove ccp >/dev/null 2>&1 || true
    codex mcp add ccp \
        --env "CCP_SERVER_URL=$SERVER_URL" \
        --env "CCP_CLIENT_KEY=$CLIENT_KEY" \
        --env "CCP_CLIENT_BIN=$INSTALL_DIR/ccp-client" \
        -- "$MCP_CMD"
    echo "Configured Codex MCP."
fi

if command -v claude >/dev/null 2>&1; then
    claude mcp remove ccp --scope user >/dev/null 2>&1 || true
    claude mcp add ccp --scope user \
        --env "CCP_SERVER_URL=$SERVER_URL" \
        --env "CCP_CLIENT_KEY=$CLIENT_KEY" \
        --env "CCP_CLIENT_BIN=$INSTALL_DIR/ccp-client" \
        -- "$MCP_CMD"
    echo "Configured Claude Code MCP."
fi

echo "Installed $INSTALL_DIR/ccp-client"
echo "All open topics are connected automatically."
echo "Update anytime:  ccp-update"
echo "Restart Codex or Claude Code after updating so it reloads the MCP tool list."
echo "Discover topics: ccp-client remote-sessions"
