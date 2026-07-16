# Changelog

All notable changes are documented here. This project follows Semantic Versioning for published
interfaces; prerelease compatibility may still change between alpha versions.

## [Unreleased]

## 0.1.0-alpha.3 - 2026-07-16

This version is a verified source preview only. No `v0.1.0-alpha.3` tag, GitHub Release, or
crates.io publication is created by this change.

### Added

- Explicit task title/goal/status editing and deterministic title suggestions from verified goal,
  branch, and touched-area data without an AI call.
- Append-only task grouping operations for session move, merge, split, preview, history, and undo,
  including lifecycle snapshots, idempotent replay, and mixed-provenance fact handling.
- A deterministic provenance-bearing relationship graph and accessible table over tasks, sessions,
  commits, changed files, Regression Contracts, verified symbols, required tests, and local agents.
- Optional `--enable-ai-refresh` setup for the managed `previously-input-only` profile, explicit
  user-triggered candidate refresh, strict structured output, restart recovery, and candidate
  accept/edit/reject review.
- Same-device read-only Codex agent lineage for interactive and supported sub-agent source kinds,
  explicit parent/child trees, degraded missing-parent state, Copy ID, and Find in Codex guidance.

### Security

- AI refresh fails closed unless the experimental App Server verifies the input-only named
  profile. It uses an isolated empty `0700` cwd, no network, approval `never`, bounded redacted
  input, no raw repository/source/tool payload, and no simultaneous legacy sandbox field.
- Experimental App Server processes inherit only a minimal non-secret environment. Profile
  verification and execution share one client, and transactional claims prevent concurrent
  duplicate refresh calls or contradictory candidate reviews.
- AI output is never Evidence. Model ID, token, and latency fields remain unavailable unless they
  are actually exposed by the App Server.
- Grouping and graph APIs retain loopback bearer/CSRF enforcement, reject cross-repository or stale
  associations, and derive graph edges only from verified sources instead of similarity guesses.
- Agent reads revalidate thread identity and logical repository and reject unsafe or sensitive
  paths before projection.
- Setup and normal runtime validate current-user-owned, non-writable-by-others data directories
  and regular private data, queue, sidecar, lock, and recovery files before access; safe excess
  directory permissions are tightened, unsafe ownership or modes fail closed, and SQLite is
  opened with no-follow semantics.

### Known limitations

- AI refresh is beta, disabled by default, and requires a compatible experimental App Server plus
  an unchanged verified managed profile. No real calibration/model call was run for this preview.
- Agent lineage is local observation, not cloud sync, team access, orchestration, or write-back.
- No documented desktop focus/open interface is available; use Copy ID and Find in Codex.
- The automatic continuation policy remains provisional: seven observed compactions or 80%
  observed context usage, independently 72 hours plus a relevant code change. It is not benchmark
  validated. The continuation campaign remains 6/864 complete with 858 arms remaining and
  `no_auto_rollover`.
- Source and macOS arm64 archives are retained as CI/PR artifacts only. They are unsigned and not
  notarized; public release remains blocked.

## [0.1.0-alpha.2] - 2026-07-16

### Added

- Git-backed Regression Contracts with strict camelCase schemas, path and literal-symbol impact
  matching, argv-only required tests, content-fingerprint freshness, and fail-closed validation.
- Evidence-only automatic candidates plus manual invariant candidates in canonical events and
  SQLite projections, with local review, approval, and supersede workflows.
- Public `previously contracts init`, `validate`, and merge-base-aware `check` commands, including
  a version-pinned macOS GitHub Actions hard gate.
- Non-blocking PreToolUse contract context and one-shot Stop blocking for missing, stale, or failed
  required tests.
- Project overview for active tasks, recent Codex sessions, decisions, open items, and touched code
  areas, plus explicit Codebase Lineage from task to repository/worktree and verification state.
- Fact editing and Git-commit deprecation, session inclusion controls, and selection reasons wired
  into the verified Context Pack builder.
- Idempotent automatic fresh-task continuation through documented Codex App Server methods at the
  provisional seven-compaction or 80% observed-context boundary.

### Security

- Git Contract files exclude raw prompts, tool output, source code, environment values, and
  secrets; local candidate and evaluation data remains redacted before persistence.
- Automatic continuation carries the redacted current prompt only over bounded child stdin,
  records the new task ID before starting its turn, and leaves the source prompt unblocked on any
  failure.

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

[0.1.0-alpha.2]: https://github.com/hongsDream/previously-on/releases/tag/v0.1.0-alpha.2
[0.1.0-alpha.1]: https://github.com/hongsDream/previously-on/releases/tag/v0.1.0-alpha.1
