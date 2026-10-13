# Nightly builds

[Documentation index](README.md) · [Stable install](../README.md#install)

Nightlies preview changes merged into `main` since the latest stable release.
CI runs daily at **06:37 UTC** using the pinned Rust compiler and locked Cargo
dependencies. A new source commit gets a dated GitHub prerelease and Docker image
after all required checks pass. Unchanged main is still checked daily; its
existing nightly release is reused.

## Install

The installer defaults to stable releases. Choose nightlies explicitly:

```bash
# Server and client; optionally install the matching MCP bridge
curl -fsSL https://raw.githubusercontent.com/squid-proxy-lovers/ccp/main/install.sh | bash -s -- --channel nightly

# Client plus the matching MCP bridge
curl -fsSL https://raw.githubusercontent.com/squid-proxy-lovers/ccp/main/install.sh | bash -s -- --client --channel nightly

# Docker server
curl -fsSL https://raw.githubusercontent.com/squid-proxy-lovers/ccp/main/install.sh | bash -s -- --docker --channel nightly --session my-session
```

Binary installation requires Python 3 and curl or wget. It discovers published
nightly prereleases through GitHub's API, downloads all selected components from
one release, and checks SHA-256 before replacing binaries. The MCP bridge is
installed from that release's archive, including when a cached editable checkout
already exists. `~/.local/bin/ccp-nightly.json` records the source commit and
compiler; `--install-dir` changes its directory along with the binaries.
Switching to a stable or source installation clears this nightly stamp.

Checksums detect corrupted downloads. Authenticity depends on the release source.
Nightly tags identify a source commit independently of the package's semver.
Use the build metadata to identify the exact nightly commit.

Use an exact tag from the [release list](https://github.com/Squid-Proxy-Lovers/ccp/releases)
to pin either installation mode:

```bash
NIGHTLY_TAG="nightly-2026-10-12-0123456789ab" # Replace with a published tag
bash install.sh --client --channel nightly --version "$NIGHTLY_TAG"
bash install.sh --docker --channel nightly --version "$NIGHTLY_TAG"
```

Replace the example tag with an actual release tag. `--version` requires
`--channel nightly`. Source builds use `--from-source` separately and build the
selected local checkout, or main when cloning outside a checkout.

## Download and container tags

Each `nightly-YYYY-MM-DD-<12-character-SHA>` prerelease contains:

- Client and server binaries for Linux and macOS, x86_64 and ARM64.
- `ccp-mcp.tar.gz`: MCP source from the same commit.
- `build-info.json`: full source SHA, pinned compiler, and artifact/platform list.
- `SHA256SUMS`: hashes of the binaries, MCP archive, and metadata.

Docker images use `ghcr.io/squid-proxy-lovers/ccp:nightly` for the current nightly,
and the matching dated release tag for a pinned image. The Docker build currently
targets Linux amd64. Nightly runs publish only these nightly tags; the existing
main/tag push workflow continues publishing `:latest` and commit-SHA images.

## Publication and recovery

The existing tests, Clippy, formatting, source lint, MSRV, dependency audits,
compatibility smoke, and Python/FastMCP matrix gate the binary builds. CI starts
the container and checks its health before saving the exact tested image for
publication. PRs build and validate packages but cannot publish nightlies.

The publisher validates the artifacts and image source SHA, stages release assets
in a draft, and uploads the dated image before exposing the prerelease. Failed
checks, asset uploads, or dated-image uploads leave the previous public nightly
available. Rerunning a partial draft resumes its upload. Existing published
releases are preserved, and older queued commits cannot replace a newer nightly.

GitHub release publication and the moving Docker tag are separate operations.
If updating `:nightly` fails after the prerelease is published, its dated image
remains available; rerunning the workflow repairs the alias from that image.
Use a dated tag when matching a specific client/server/MCP build.

Maintainers can run CI manually on **main** with the **nightly** input enabled.
Manual runs without that input perform checks only. The schedule runs on main;
GitHub may delay scheduled starts. Dated releases/images are retained without
automatic deletion.
Intermediate CI artifacts are retained for one day; distribution uses GitHub
Releases and GHCR. After intermediates expire, rerun the complete workflow.

Local checks for this plumbing:

```bash
make nightly-tests installer-tests
make lint-python lint-shell lint-workflows
```
