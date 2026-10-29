# Enrollment link format

Unreleased version 1. This reference defines the shared Rust/browser parser.
Issuance, the join page and client-command integration are separate work until
their implementation and end-to-end checks are complete.

[Documentation index](README.md) · [Workspace client](workspace-client.md)

One link contains public routing and pinning metadata plus an invitation secret:

```text
https://ccp.example:8443/join#v=1&i=INSTANCE_UUID&m=8444&ca=sha256:BASE64URL&t=INVITATION&w=WORKSPACE_UUID&n=WORKSPACE_NAME
```

Values use UTF-8 form encoding. Spaces in names may be encoded as `+` or `%20`.
The canonical encoder emits a full HTTPS origin for `m`; the parser also accepts
a port on the link's HTTPS host. For example, a different client endpoint can be
encoded as `m=https%3A%2F%2Fclients.example%3A9444%2F`.

| Field | Meaning |
| --- | --- |
| `v` | Exactly `1`; other versions are rejected |
| `i` | Instance UUID, canonical lowercase hyphenated form |
| `m` | mTLS port from 1–65535, or a complete HTTPS origin |
| `ca` | `sha256:` followed by the unpadded base64url encoding of 32 bytes |
| `t` | The exact `ccp_join_` invitation secret issued by the server |
| `w` | Workspace UUID, canonical lowercase hyphenated form |
| `n` | Workspace display-name hint, 1–256 UTF-8 bytes |

Every field is required exactly once. Unknown and duplicate fields, including
duplicates disguised with percent encoding, are rejected. Malformed percent
escapes, invalid UTF-8, control characters and unsupported versions are rejected
without echoing the link or secret in an error. The complete link is limited to
8192 UTF-8 bytes. Names cannot start or end with Unicode whitespace; the parser
preserves other valid Unicode, including U+FEFF. Names never authorize access or
choose an unchecked filesystem path.

Origins require HTTPS, an ASCII DNS name or IP address, and an optional port.
Version 1 excludes internationalised hostnames and reserved DNS labels with `--`
in their third and fourth positions. This keeps hostname acceptance independent
of differing platform IDNA implementations; Unicode workspace names are supported.
Hosts must already have canonical spelling/compression apart from ASCII letter
case. An explicit default port is accepted; empty or leading-zero ports are not.
Credentials, queries, extra routes, backslashes, unspecified IP addresses and
IPv4-mapped IPv6 aliases are rejected. The public route is exactly `/join`. UUID
hints cannot be nil. Encoders percent-encode metadata and quote the optional
**POSIX shell** command for sh/bash/zsh. Other shells should use hidden paste.

## Trust and secret handling

The CA pin hashes the **DER certificate bytes**, matching CCP's existing
`ca_sha256`. It does not hash the public-key/SPKI representation used by some
other enrollment systems. The link must arrive through a channel the recipient
trusts independently; fetching public CA bytes does not establish trust by itself.

Parsing performs no network request. A client must compare the discovered CA's
DER digest against the pin, use normal CA/name verification, and check the
instance UUID through health before sending the invitation or CSR. Profile
publication and enrollment must retain the existing durable-request and retry
rules. A pin or UUID mismatch must not transmit the invitation. The mTLS endpoint
may have a different hostname, and must pass the same CA/name/instance checks.

Browser consumers remove the fragment before validating it and keep the secret in
memory only. The token is excluded from ordinary object serialization and spread;
explicit access is required to copy it. Developer tools can still inspect browser
memory. Consumers never redeem or submit it. Hidden paste is the primary flow;
an optional copyable command must identify its supported shell.
The fragment stays out of the initial HTTP request; browser JavaScript still has
access to it. A hosted browser UI therefore has its previously agreed proxy trust
boundary. Passing a secret link on argv can expose it through shell history and
process listings; hidden paste and stdin are required command alternatives. POSIX
single quoting is not safe in Windows cmd.exe, so do not paste that command there.

Lifetime and use limits belong to the server's invitation record. Editing a link
cannot extend either. Short-lived, single-use issuance is the default; an explicit
no-expiry choice requires warning/confirmation and retains a bounded use limit.
The existing one-time initial group window and later individual-admission rules
remain applicable. Exact committed retries are separate from a fresh redemption.

This version describes direct HTTPS/mTLS endpoints. It does not imply that the
client WSS connector or real Cloudflare deployment acceptance is complete.
