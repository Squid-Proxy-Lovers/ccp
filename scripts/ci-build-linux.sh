#!/usr/bin/env bash
# Build distributable GNU/Linux binaries against Debian Bookworm's libc.
# Copyright (C) 2026 Squid Proxy Lovers
# SPDX-License-Identifier: AGPL-3.0-or-later
set -euo pipefail

target="${1:?usage: ci-build-linux.sh TARGET [client|server ...]}"
shift
case "$target" in
    x86_64-unknown-linux-gnu) platform=linux/amd64 ;;
    aarch64-unknown-linux-gnu) platform=linux/arm64 ;;
    *) echo "Unsupported Linux target: $target" >&2; exit 64 ;;
esac
if [ "$#" -eq 0 ]; then set -- server client; fi
package_args=()
packages=("$@")
for package in "${packages[@]}"; do
    case "$package" in client|server) package_args+=(-p "$package") ;;
        *) echo "Unsupported package: $package" >&2; exit 64 ;;
    esac
done

project_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
build_dir="$project_dir/target/bookworm"
registry_dir="${CCP_CI_CARGO_CACHE:-${CARGO_HOME:-$HOME/.cargo}}/registry"
mkdir -p "$build_dir" "$registry_dir"

# Isolate this target from host-built objects and test-only feature unification.
docker run --rm --platform "$platform" \
    --mount "type=bind,source=$project_dir,target=/source,readonly" \
    --mount "type=bind,source=$build_dir,target=/build" \
    --mount "type=bind,source=$registry_dir,target=/usr/local/cargo/registry" \
    -e CARGO_TARGET_DIR=/build -w /source \
    rust:1.88.0-bookworm \
    cargo build --locked --release --target "$target" "${package_args[@]}"

for package in "${packages[@]}"; do
    docker run --rm --platform "$platform" \
        --mount "type=bind,source=$build_dir,target=/artifacts,readonly" \
        --entrypoint "/artifacts/$target/release/$package" \
        debian:bookworm-slim --version
done
