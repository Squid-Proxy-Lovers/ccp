# Verify a matching client/MCP release bundle

This unreleased distribution contract identifies matching official client/MCP
bytes and their build provenance. The [managed bundle commands](managed-client.md) now support manual staging,
activation and pinned launch. Opt-in automatic checks/activation remain subsequent work. Existing manual installation and GHCR
server deployment keep their current behavior.

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
`darwin-aarch64`. For a dated nightly, use its exact
`nightly-YYYY-MM-DD-<12-character-SHA>` tag and `--channel nightly`. The source
commit in the manifest must match that tag's SHA prefix. A stable tag must equal
`v` followed by the matching package version. No moving `latest` or `nightly`
selector is accepted by this verification command.

Successful output is public JSON metadata. A failed check exits nonzero without
installing software. Keep the staged client, manifest, provenance and MCP archive
together. This command verifies files; it does not activate a bundle.

## Understand publisher verification

`ccp-release.json` contains format 1, the official repository identity, full source
commit, matching client/server/MCP package version, fixed platforms and each
artifact's exact byte count and SHA-256. Contract numbers identify native wire,
workspace API, client profile read minimum/write format and server schema write
format from the producing source. The MCP archive and `build-info.json` are bound
to the same manifest as the client binaries.

`ccp-provenance.json` is the GitHub Actions attestation bundle for that manifest.
The verifier invokes GitHub CLI using private copies of the exact parsed manifest
and bundle. Policy fixes the GitHub hostname, canonical repository spelling,
exact workflow certificate identity and OIDC issuer, source ref/commit, signer
commit and GitHub-hosted runner requirement. Stable builds must come from their
exact version tag; nightly builds must come from `refs/heads/main`.
GitHub documents the [attestation mechanism](https://docs.github.com/en/actions/how-tos/secure-your-work/use-artifact-attestations/use-artifact-attestations)
and [CLI policy flags](https://cli.github.com/manual/gh_attestation_verify).

After publisher verification, the tool checks the chosen client, MCP archive and
build metadata against the authenticated sizes/hashes. Malformed, duplicate,
future-format, foreign-repository or unexpected artifact fields fail. Symlink
inputs, excessive sizes, missing provenance, unavailable verification tools,
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

Packaging produces the manifest beside the existing binaries, matching MCP
archive, build metadata and checksums. A separate provenance job runs only for
the official repository's version-tag pushes or approved main nightly triggers.
PR packaging retains read-only permissions; only the event/ref-gated provenance job
receives OIDC/attestation write permissions. The pinned official attestation
action signs the manifest, and the job verifies publisher policy before making
the bundle available to the publishers. Stable and nightly publication both wait
for that job and attach its bundle. Failed verification leaves publication blocked.

This workflow configuration has local regression/lint coverage. A successful
hosted attestation and publication must still be demonstrated when the approved
workflow runs. No live release or image was published by this implementation.
