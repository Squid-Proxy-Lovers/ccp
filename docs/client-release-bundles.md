# Verify a matching client/MCP release bundle

This unreleased distribution contract identifies matching official client/MCP
bytes and their build provenance. The [managed bundle commands](managed-client.md)
support manual staging, pinned launch and updates, plus local opt-in automatic
checks and guarded activation. Existing manual installation and GHCR server
deployment keep their current behavior.

[Documentation index](README.md) · [Nightly builds](nightly-builds.md) ·
[Workspace client](workspace-client.md)

## Select and download a release

Use an immutable tag from the official repository. The new files become available
only after the approved publication workflow runs; older releases without them
cannot pass this verifier. A CCP instance or workspace invitation supplies no
software trust authority.

From a matching checkout, with Python 3.10+ and GitHub CLI installed, download one
client platform and its matching MCP archive. Replace `YOUR_PUBLISHED_TAG` with an
actual published stable tag, and choose the platform you will run:

```bash
RELEASE_TAG="YOUR_PUBLISHED_TAG"
mkdir release-stage
gh release download "$RELEASE_TAG" --repo github.com/Squid-Proxy-Lovers/ccp \
  --dir release-stage --pattern ccp-release.json --pattern ccp-provenance.json \
  --pattern ccp-client-darwin-aarch64 --pattern ccp-mcp.tar.gz \
  --pattern build-info.json
python3 scripts/release_contract.py --directory release-stage \
  --channel stable --tag "$RELEASE_TAG" --platform darwin-aarch64
```

Supported platform names are `linux-x86_64`, `linux-aarch64`, `darwin-x86_64` and
`darwin-aarch64`, plus client-only `windows-x86_64`
(`ccp-client-windows-x86_64.exe`). For a dated nightly, use its exact
`nightly-YYYY-MM-DD-<12-character-SHA>` tag and `--channel nightly`. The source
commit in the manifest must match that tag's SHA prefix. Stable and RC tags must
equal `v` followed by the matching Rust/manifest version, with `--channel rc`
required for an RC. No moving `latest` or `nightly` selector is accepted by this
verification command.

Successful output is public JSON metadata. A failed check exits nonzero without
installing software. Keep the staged client, manifest, provenance and MCP archive
together. This command verifies files; it does not activate a bundle.

## Install a release candidate

RCs require an explicit tag. Once `v0.2.0-rc.1` is published, run these commands
from a checkout containing this verifier, with Python 3.10+ and GitHub CLI
installed. This example selects macOS ARM64; change both platform occurrences
for another supported client platform:

```bash
mkdir ccp-rc-stage
gh release download v0.2.0-rc.1 --repo github.com/Squid-Proxy-Lovers/ccp \
  --dir ccp-rc-stage --pattern ccp-release.json --pattern ccp-provenance.json \
  --pattern ccp-client-darwin-aarch64 --pattern ccp-mcp.tar.gz \
  --pattern build-info.json
python3 scripts/release_contract.py --directory ccp-rc-stage \
  --channel rc --tag v0.2.0-rc.1 --platform darwin-aarch64
```

Continue only if verification succeeds. Install the verified client and the MCP
archive from that same release in a separate test directory:

```bash
mkdir -p "$HOME/.local/share/ccp-rc/v0.2.0-rc.1/bin"
install -m 755 ccp-rc-stage/ccp-client-darwin-aarch64 \
  "$HOME/.local/share/ccp-rc/v0.2.0-rc.1/bin/ccp"
python3 -m venv "$HOME/.local/share/ccp-rc/v0.2.0-rc.1/venv"
"$HOME/.local/share/ccp-rc/v0.2.0-rc.1/venv/bin/python" -m pip install \
  ./ccp-rc-stage/ccp-mcp.tar.gz
```

Point the MCP host at
`~/.local/share/ccp-rc/v0.2.0-rc.1/venv/bin/ccp-workspace-mcp` and set its
`CCP_CLIENT_BIN` to the absolute path of the matching `bin/ccp`. Keep the host's
workspace configuration as described in the [MCP guide](workspace-mcp.md).
Rust binaries, the manifest and tag identify `0.2.0-rc.1`; Python package metadata
identifies the exact corresponding version as `0.2.0rc1`.

For a Docker server, pin the exact RC image:

```bash
docker pull ghcr.io/squid-proxy-lovers/ccp:v0.2.0-rc.1
docker run --rm -it -p 127.0.0.1:8443:8443 -p 127.0.0.1:8444:8444 \
  -v ccp-rc-data:/var/lib/ccp/server \
  ghcr.io/squid-proxy-lovers/ccp:v0.2.0-rc.1
```

Follow the [first-boot setup](workspace-foundation.md#start-an-instance-in-one-command)
using the setup URL printed by the container.

RC publication creates a GitHub prerelease excluded from the default download,
and publishes only the full commit-SHA image and exact RC image tag. Stable
installers and managed stable discovery do not select RCs. RCs never move
`:latest` or `:nightly`. The normal installer has no RC channel; use the explicit
verified download above.

## Understand publisher verification

`ccp-release.json` contains format 1, the official repository identity, full source
commit, matching client/server/MCP package version, declared platforms and each
artifact's exact byte count and SHA-256. Contract numbers identify native wire,
workspace API, client profile read minimum/write format and server schema write
format from the producing source. The MCP archive and `build-info.json` are bound
to the same manifest as the client binaries.

`ccp-provenance.json` is the GitHub Actions attestation bundle for that manifest.
The verifier invokes GitHub CLI using private copies of the exact parsed manifest
and bundle. Policy fixes the GitHub hostname, canonical repository spelling,
exact workflow certificate identity and OIDC issuer, source ref/commit, signer
commit and GitHub-hosted runner requirement. Stable and RC builds must come from
their exact version tag; nightly builds must come from `refs/heads/main`.
GitHub documents the [attestation mechanism](https://docs.github.com/en/actions/how-tos/secure-your-work/use-artifact-attestations/use-artifact-attestations)
and [CLI policy flags](https://cli.github.com/manual/gh_attestation_verify).

After publisher verification, the tool checks the chosen client, MCP archive and
build metadata against the authenticated sizes/hashes. Format 1 keeps an exact
top-level key set: `format`, `repository`, `source_sha`, `package_version`,
`platforms`, `contracts` and `artifacts`. The `contracts` object must contain
exactly the five named contracts above, each an integer from 1 through 2^32 - 1;
the profile read minimum must not exceed its write format. Future producers must
keep this envelope and contract set to remain compatible with format-1 clients.
Adding top-level or compatibility-contract fields requires an explicit verifier
upgrade. Unsupported formats, foreign repositories and missing fields fail.

Names are ASCII only. A platform's OS component starts with `[a-z]`, followed by
zero or more `[a-z0-9]` characters and a hyphen. The rest consists of nonempty
`[a-z0-9]+` components separated by single hyphens or underscores. More than two
parts are allowed, for example `futureos-x86_64-v3-musl`. An asset name starts with
`[a-z0-9]` and otherwise uses lowercase letters, digits, underscores and hyphens;
single dots may separate nonempty components that each start with `[a-z0-9]`.
Paths, non-ASCII characters, leading dots and consecutive dots are rejected.
Duplicate platform names and duplicate JSON keys anywhere, including inside
ignored metadata, are rejected.

All recognized artifact names (the current eight client/server binaries,
`ccp-mcp.tar.gz` and `build-info.json`) retain strict entries: exactly `size` and
`sha256`, a positive integer size no greater than 256 MiB, and a lowercase
64-character hexadecimal SHA-256. Malformed recognized entries fail even when
not selected for installation. Unknown artifact names may carry opaque JSON
payloads, including zero or larger declared sizes and extra `url` or `signature`
fields. Those payloads are retained for manifest-integrity comparisons but never
interpreted, fetched or executed; URLs never influence download locations. The
whole manifest remains limited to 64 KiB and covered by publisher attestation.

Additional platforms cannot expand the verifier's supported platform list.
Platforms may provide a client without a server. The selected platform must be
listed and its client, MCP archive and build metadata must all be present and
verified. Current producers still emit exactly the four platforms, in the same
order, and both binaries for each. Their manifest, build metadata and checksum
bytes are identical to the pre-change output for the same source values and
artifact bytes. Publication rejects any other platform order or artifact set.

Symlink inputs, excessive sizes, missing provenance, unavailable verification tools,
timeouts and signature/policy failures also fail. There is no checksum fallback.
GitHub CLI must support the required policy flags; local verification was checked
with version 2.100.0. Trust-root/network prerequisites follow GitHub CLI's
verification behavior; no custom trust-root or instance-origin option is exposed.

Checksums remain useful for corruption detection. `SHA256SUMS` continues covering
existing assets and now also covers the release manifest; the signed manifest is
the publisher trust boundary. Authenticated build declarations do not establish
compatibility with a particular running server or local profile. The managed
updater must check those contracts before activation. Missing or unsupported
contracts require a manual decision.

## Publication boundary

Before attestation or publication, version tags must be exactly `vX.Y.Z` or
`vX.Y.Z-rc.N` and match all source package versions. Docker version-tag
publication also waits for identity validation and provenance; only validated
stable releases publish their exact version tag, commit-SHA tag, and `:latest`.
Main pushes run checks without moving that alias.

Packaging produces the manifest beside the existing binaries, matching MCP
archive, build metadata and checksums. A separate provenance job runs only for
the official repository's version-tag pushes or approved main nightly triggers.
PR packaging retains read-only permissions; only the event/ref-gated provenance job
receives OIDC/attestation write permissions. The pinned official attestation
action signs the manifest, and the job verifies publisher policy before making
the bundle available to the publishers. Stable, RC and nightly publication all
wait for that job and attach its bundle. Failed verification leaves publication
blocked.

This workflow configuration has local regression/lint coverage. A successful
hosted attestation and publication must still be demonstrated when the approved
workflow runs. No live release or image was published by this implementation.
