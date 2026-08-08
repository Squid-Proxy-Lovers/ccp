#!/bin/sh
set -eu

project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
download_dir="${CCP_DOWNLOAD_DIR:-$project_dir/downloads}"
mkdir -p "$download_dir"

cd "$project_dir"
cargo build --release -p client -p server

case "$(uname -s)" in Linux) os=linux ;; Darwin) os=darwin ;; *) exit 1 ;; esac
case "$(uname -m)" in x86_64|amd64) arch=x86_64 ;; arm64|aarch64) arch=aarch64 ;; *) exit 1 ;; esac
cp target/release/client "$download_dir/ccp-client-${os}-${arch}"
cp target/release/server "$download_dir/ccp-server-${os}-${arch}"
chmod 0755 "$download_dir/ccp-client-${os}-${arch}" "$download_dir/ccp-server-${os}-${arch}"

build_venv=$(mktemp -d)
python3 -m venv "$build_venv"
"$build_venv/bin/pip" install --quiet build
"$build_venv/bin/python" -m build --sdist --outdir "$download_dir" mcp
archive=$(find "$download_dir" -maxdepth 1 -name 'ccp_mcp_server-*.tar.gz' | sort | tail -1)
cp "$archive" "$download_dir/ccp-mcp.tar.gz"
rm -r "$build_venv"

echo "Artifacts written to $download_dir"
