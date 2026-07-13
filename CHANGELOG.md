# Changelog

All notable changes are documented here. This project follows Semantic Versioning for published
interfaces; prerelease compatibility may still change between alpha versions.

## [0.1.0-alpha.1] - 2026-07-13

### Added

- Local Codex Hook capture with commit-aware daemon acknowledgements and crash-safe replay.
- Conservative Git lineage for file and diff-hunk changes.
- Deterministic checkpoints and Context Packs with evidence, coverage, and freshness.
- Read-only MCP resume tools and a loopback review UI.
- Fact confirmation, pinning, invalidation, replacement, JSON export, and repository purge.
- Reproducible Apple Silicon release archives, checksums, license inventory, and CycloneDX SBOM.
- Public `previously run codex` and `previously import codex` fallback commands while transparent
  Hook/App Server linkage remains unverified.

### Security

- Redaction is applied before canonical storage and fallback queueing.
- Historical evidence is wrapped as untrusted data.
- The default mode contains no telemetry or outbound network integration.

### Known limitations

- macOS is the only supported operating system; binary archives are Apple Silicon only.
- Release binaries are unsigned and not notarized.
- One local Git repository and Codex are supported at a time.
- No code graph, cloud sync, team access, AI fact extraction, or automatic Pack injection.
- Independently launched transparent capture is experimental; the supported alpha path is the
  explicit run wrapper or explicit import command.

[0.1.0-alpha.1]: https://github.com/hongsDream/previously-on/releases/tag/v0.1.0-alpha.1
