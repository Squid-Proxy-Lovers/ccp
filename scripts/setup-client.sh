#!/bin/sh
set -eu

SERVER_URL="http://192.168.130.34:1338"
CLIENT_KEY="ccp-client-7b6c2f915e4a8d30"
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

mkdir -p "$INSTALL_DIR"
curl -fsSL "$SERVER_URL/downloads/ccp-client-${os}-${arch}" -o "$INSTALL_DIR/ccp-client"
chmod 0755 "$INSTALL_DIR/ccp-client"
curl -fsSL "$SERVER_URL/ccp-update" -o "$INSTALL_DIR/ccp-update"
chmod 0755 "$INSTALL_DIR/ccp-update"
"$INSTALL_DIR/ccp-client" subscribe-all

MCP_VENV="$HOME/.ccp-client/mcp-venv"
python3 -m venv "$MCP_VENV"
"$MCP_VENV/bin/pip" install --upgrade "$SERVER_URL/downloads/ccp-mcp.tar.gz"
MCP_CMD="$MCP_VENV/bin/ccp-mcp-server"

if command -v codex >/dev/null 2>&1; then
    codex mcp remove ccp >/dev/null 2>&1 || true
    codex mcp add ccp \
        --env "CCP_SERVER_URL=$SERVER_URL" \
        --env "CCP_CLIENT_KEY=$CLIENT_KEY" \
        -- "$MCP_CMD"
    echo "Configured Codex MCP."
fi

if command -v claude >/dev/null 2>&1; then
    claude mcp remove ccp --scope user >/dev/null 2>&1 || true
    claude mcp add ccp --scope user \
        --env "CCP_SERVER_URL=$SERVER_URL" \
        --env "CCP_CLIENT_KEY=$CLIENT_KEY" \
        -- "$MCP_CMD"
    echo "Configured Claude Code MCP."
fi

echo "Installed $INSTALL_DIR/ccp-client"
echo "All open topics are connected automatically."
echo "Update anytime:  ccp-update"
echo "Discover topics: ccp-client remote-sessions"
