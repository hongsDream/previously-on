# Changelog

All notable changes are documented here. This project follows Semantic Versioning for published
interfaces; prerelease compatibility may still change between alpha versions.

## [Unreleased]

### Added

- Git-backed Regression Contracts with strict camelCase schemas, path and literal-symbol impact
  matching, argv-only required tests, content-fingerprint freshness, and fail-closed validation.
- Evidence-only automatic candidates plus manual invariant candidates in canonical events and
  SQLite projections, with local review, approval, and supersede workflows.
- Public `previously contracts init`, `validate`, and merge-base-aware `check` commands, including
  a version-pinned macOS GitHub Actions hard gate.
- Non-blocking PreToolUse contract context and one-shot Stop blocking for missing, stale, or failed
  required tests.

### Security

- Git Contract files exclude raw prompts, tool output, source code, environment values, and
  secrets; local candidate and evaluation data remains redacted before persistence.

## [0.1.0-alpha.1] - 2026-07-13

### Added

- Local Codex Hook capture with commit-aware daemon acknowledgements and crash-safe replay.
- Conservative Git lineage for file and diff-hunk changes.
- Deterministic checkpoints and Context Packs with evidence, coverage, and freshness.
- Session timelines with source-thread identity, relative age, turns, compactions, observed
  context usage, and current-Git revalidation.
- One-time new-thread advice on the next prompt after six compactions, 80% observed context usage,
  or an old session's relevant code changes.
- Read-only MCP resume tools and a loopback review UI.
- Fact confirmation, pinning, invalidation, replacement, JSON export, and repository purge.
- Reproducible Apple Silicon release archives, checksums, license inventory, and CycloneDX SBOM.
- Public `previously run codex` and `previously import codex` fallback commands while transparent
  Hook/App Server linkage remains unverified.

### Security

- Redaction is applied before canonical storage and fallback queueing.
- Historical evidence is wrapped as untrusted data.
- The default mode contains no telemetry or outbound network integration.
- Context Packs are never injected automatically, and the UI never invokes an AI model.

### Known limitations

- macOS is the only supported operating system; binary archives are Apple Silicon only.
- Release binaries are unsigned and not notarized.
- One local Git repository and Codex are supported at a time.
- No code graph, cloud sync, team access, automatic Pack injection, or UI-triggered AI refresh.
- AI-assisted fact refresh is deferred to v0.1.1 until a deny-read execution boundary can be
  verified against prompt-injection fixtures.
- Independently launched transparent capture is experimental; the supported alpha path is the
  explicit run wrapper or explicit import command.

[0.1.0-alpha.1]: https://github.com/hongsDream/previously-on/releases/tag/v0.1.0-alpha.1
