# Hosted downloads

These files serve the HTTP client installers. `provenance.json` records each artifact's source commit, CI run, target, size and SHA-256. The package version is 0.2.0; the application source revision is the same across all native files.

The native files were promoted from successful CI builds of the reviewed source. Linux release artifacts are built in Rust 1.88.0 Bookworm containers on native runners and invoked in Debian Bookworm runtime containers to check libc compatibility; they are built without test-only workspace features. The MCP archive's source files are checked against that revision. Updating source does not automatically update these files: regenerate the relevant platforms or promote matching CI artifacts, then refresh provenance. Do not treat a mixed or stale downloads directory as a current build.

`scripts/build-downloads.sh` can build the local platform and MCP package; other platforms require their corresponding CI jobs. Releases include all supported client platforms and the Python source archive. Copy these filenames into the running server's `CCP_DOWNLOAD_DIR`; CI uploads alone do not modify that directory.

Only one macOS arm64 server binary is included here for parity with the existing repository artifact layout. Other server platforms are available in CI/release artifacts. Windows installer runtime is not verified by the Linux/macOS smoke tests; CI runs native client regressions and PowerShell syntax checks.
