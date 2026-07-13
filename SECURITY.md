# Security policy

## Supported versions

| Version | Supported |
| --- | --- |
| `0.1.0-alpha.1` | Yes |
| unpublished development builds | No |

Security fixes for the alpha are released as a new immutable prerelease. Published crates and
release assets are never replaced in place.

## Report a vulnerability

Use the repository's **Security → Report a vulnerability** form:

<https://github.com/hongsDream/previously-on/security/advisories/new>

Do not open a public issue. Include the affected version, platform, reproduction steps, impact,
and a sanitized proof of concept. Never include real credentials or private repository data.
Maintainers will acknowledge a complete report within seven days and coordinate disclosure after
a fix is available. If GitHub private vulnerability reporting is unavailable, do not publish the
details; open a public issue containing only a request for a private contact channel.

## Security boundary

PreviouslyOn processes source code, prompts, shell commands, and tool output. The alpha boundary
covers redaction before persistence, loopback-only UI serving, read-only MCP tools, bounded hook
payloads, conservative Git attribution, and repository purge. Historical evidence is always
returned in an untrusted-data envelope.

PreviouslyOn does not protect copies made by backup software, another process running as the same
macOS user, or data a user intentionally exports. The Apple Silicon alpha artifact is unsigned and
not notarized; verify `SHA256SUMS` and the GitHub artifact attestation before use.
