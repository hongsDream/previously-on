# PreviouslyOn

PreviouslyOn is a local-first, verifiable project-memory and continuation layer for Codex. It
connects captured sessions to the codebase, Git state, test evidence, decisions, and open work. At
a continuation boundary it can start the current request in a fresh Codex task with a verified,
bounded Context Pack.

Version `0.1.0-alpha.3` is a verified source preview. This change does not create a tag, GitHub
Release, or crates.io publication. Historical evidence is always untrusted data, not an
instruction and not a replacement for checking the current source.

## What the alpha includes

- one Rust `previously` binary for collection, storage, Git correlation, MCP, and the review UI;
- an embedded React project overview and evidence inspector;
- a task timeline with source-thread identity, relative activity age, turn and compaction counts,
  and observed context usage;
- deterministic context packs with provenance, freshness, and capture-coverage warnings;
- Git-shared regression contracts that connect changed paths and literal symbols to required tests;
- append-only task editing and move, merge, split, and undo operations with deterministic replay;
- a provenance-bearing relationship graph derived from canonical events, projections, and
  Regression Contracts rather than inferred dependencies;
- optional user-triggered AI fact refresh that produces reviewable candidates only after an
  input-only named permission profile is verified;
- same-device, read-only Codex agent lineage with explicit parentage when the App Server reports it;
- consent-gated fresh-task continuation after a session crosses the provisional boundary, with
  idempotent recovery and automatic navigation to the started Codex task;
- no PreviouslyOn API key, telemetry, cloud service, or independent outbound integration; model
  execution is delegated to the user's configured local Codex App Server;
- repository-scoped JSON export and complete local purge.

PreviouslyOn deliberately does not infer dependencies, replay chats, sync to a cloud or team
account, or orchestrate agents. The only Codex write is the explicitly approved fresh-task
continuation; local agent lineage remains observation only.

## Install

### CI/PR source-preview artifacts

The `package-release` job builds `previously-on-v0.1.0-alpha.3-macos-arm64.tar.gz`,
`previously-on-v0.1.0-alpha.3-source.tar.gz`, and `SHA256SUMS` as retained CI/PR artifacts. They
are not public release assets. After downloading the artifact from the reviewed pull request,
verify and install the unsigned Apple Silicon binary:

```bash
shasum -a 256 -c SHA256SUMS
tar -xzf previously-on-v0.1.0-alpha.3-macos-arm64.tar.gz
install -m 0755 previously-on-v0.1.0-alpha.3-macos-arm64/previously ~/.local/bin/previously
previously --version
```

The Apple Silicon preview artifact is not signed or notarized. Review its PR, commit, checksum,
and CI result before allowing it through macOS security controls.

### Source checkout

No `0.1.0-alpha.3` crate is published by this preview. Build the reviewed checkout with Rust
1.90+ and Node.js 22+:

```bash
npm --prefix ui ci
npm --prefix ui run build
cargo install --path . --locked
```

No Intel binary archive is produced. Regression Contracts are included in `0.1.0-alpha.3`.
`previously contracts init --github-actions` pins the same PreviouslyOn package version that
generated the workflow and installs it outside the consumer repository, so the gate never assumes
that repository is a Rust project. Because `0.1.0-alpha.3` is not published to crates.io, do not
merge that generated consumer workflow until a later public release supplies the pinned crate.

## Quick start

```bash
previously setup codex --repo /absolute/path/to/your/repository
previously run codex --repo /absolute/path/to/your/repository -- <codex arguments>
previously status
previously doctor
previously ui
previously contracts validate
previously contracts check --base origin/main --execute --json
```

`previously doctor` is read-only: it generates the installed App Server's official JSON schema,
performs `initialize` and a bounded `thread/list`, and reports readiness separately for core
import, continuation, and optional experimental refresh. The detected Codex version is provenance,
not a feature gate; doctor never creates a task or starts a model turn.

AI fact refresh is beta and disabled by default. To install the managed input-only profile, opt in
explicitly, restart Codex, and use the review UI's **Refresh** button:

```bash
previously setup codex --repo /absolute/path/to/your/repository --enable-ai-refresh
```

The button remains disabled unless the experimental App Server verifies the named profile and its
effective requirements. A refresh is never scheduled or started in the background. Its output is
an AI candidate, not Evidence, and becomes a Fact Candidate only after explicit accept or edit.

Restart Codex after setup so it loads the managed Hooks and `previously_on` MCP server. Other
public commands are:

```text
previously export --format json
previously import codex --repo <path>
previously purge --repo <path>
previously uninstall codex
```

The official executable is `previously`. If the shorter spelling does not conflict with another
tool already installed on your machine, add an optional shell alias yourself:

```bash
alias prev=previously
```

PreviouslyOn does not install a `prev` binary or symlink because an existing development tool
already uses that command name.

## Codex workflow

1. Register the repository, then start supported alpha sessions with `previously run codex
   --repo <path> -- <codex arguments>`.
2. The wrapper runs Codex in that repository with inherited terminal I/O, preserves Codex's exit
   status, replays any redacted crash-safe queue, and attempts an App Server repair import.
3. If Codex was started separately, run `previously import codex --repo <path>` explicitly. The
   import reports skipped/degraded threads and never treats unknown items as trusted evidence.
4. A completed session creates a deterministic checkpoint from observed events, Git state, and
   verification results.
5. On a later first prompt, Codex may show a small resume candidate. Nothing is loaded until the
   user approves it and `resume_task` is called through MCP.
6. The project overview shows active tasks, recent source task IDs, decisions, open items, code
   areas, and an evidence-backed relationship graph with an equivalent table. Task sessions can be moved,
   merged, split, or undone through previewed append-only operations. The task inspector shows
   Codebase Lineage, local agent lineage, checkpoints, and why facts were selected. Facts can be
   edited, deprecated after a Git commit, confirmed, pinned, invalidated, or superseded; a
   captured session can be excluded from future packs.
7. When a session reaches seven observed compactions or at least 80% observed context usage, the
   next user prompt offers **Continue in a fresh task?** Codex must show its approval UI before the
   `continue_task` MCP write can run. Approval revalidates Git and Regression Contracts, creates a
   persisted task with the official Codex App Server, starts the exact current request with a
   verified Context Pack plus a current-worktree Contract evaluation, opens
   `codex://threads/<thread-id>`, and stops the source turn after the successful tool result to
   prevent duplicate work. Required tests are reported as passed, failed, stale, or missing from
   existing same-fingerprint evidence and are never auto-run during handoff. Decline, cancel, or
   failure keeps the request in the source task. A session inactive for at least 72 hours also
   offers this flow only when relevant code changed.

The seven-compaction/80% rule is an explicit provisional alpha policy, not a benchmark-derived or
model-general threshold. It will be replaced only after the continuation benchmark described in
[the product roadmap](docs/product-roadmap.md) produces a model-specific recommendation. Codex's
documented desktop deep link opens the persisted task after `turn/start`; the review UI keeps an
**Open in Codex** fallback when automatic navigation is unavailable.

Context usage is recorded only when the App Server actually emits a token-usage notification.
PreviouslyOn does not infer a percentage from prompt size or other incomplete observations.

Transparent capture from an independently launched Codex process is experimental in this alpha.
The mapped 30-row regression driver and live App Server schema probe do not prove 30 real Codex
workflows or stable Hook/App Server ID linkage, so releases must not advertise that path as
supported yet.

The MCP server exposes five read-only tools (`suggest_resume`, `resume_task`, `search_tasks`,
`explain_fact`, and `get_task_timeline`) plus the idempotent local write `continue_task`. Setup
forces `continue_task` to `approval_mode = "prompt"`; it cannot create or start a task before the
user approves Codex's confirmation UI.

## AI refresh and local agent lineage

AI refresh sends only a bounded, redacted verified pack from an isolated empty `0700` working
directory. The managed `previously-input-only` profile denies root, temporary-directory, and
network access, permits only minimal read, and uses approval `never`. PreviouslyOn verifies the
profile through `permissionProfile/list`, sends named permissions without legacy `sandbox`, uses a
strict `turn/start` output schema, and fails closed when any required capability is unavailable.
The configured Codex default model is inherited; model ID, tokens, and latency are recorded only
when the App Server exposes them, otherwise they remain unavailable.

The experimental App Server child starts from a cleared environment and receives only `PATH`,
`HOME`, `CODEX_HOME`, `TMPDIR`, locale, and terminal variables. The same initialized client that
verifies the allowed profile performs the refresh start, closing the verification/execution gap.

Agent lineage imports paginated interactive and sub-agent thread metadata from the same local
Codex App Server. It includes a thread only when it belongs to the registered concrete worktree and
links a parent only when `parentThreadId` is present. Missing parentage stays unlinked/degraded;
every `thread/read` ID and concrete worktree is revalidated, and unsafe or sensitive file paths are
discarded. Names, paths, and similarity are never used to guess a relationship. This is
same-device local observation, not cloud sync, team access, orchestration, or write-back.

## Regression Contracts

Regression Contracts keep a repository's proven bug fixes and service invariants in Git. Each
approved contract is one camelCase JSON file at `.previously-on/contracts/<uuid>.json`. A file is
active in the current checkout as soon as it is written; the normal commit and pull-request flow
shares it with the team.

Contracts use case-sensitive Git path selectors (`exact` or `prefix`) and optional literal symbol
identifiers. Selector groups are ORed. A group always requires a path match, and when it lists
symbols at least one must occur in a changed hunk. Renames inspect both paths. If a binary,
unreadable, or oversized diff prevents symbol inspection, PreviouslyOn conservatively treats the
path match as relevant and reports a warning.

Required tests are argv records, never shell strings:

```json
{
  "id": "tenant-isolation",
  "name": "Tenant isolation integration",
  "program": "./scripts/test-tenant-isolation",
  "args": [],
  "workingDirectory": ".",
  "timeoutSeconds": 900
}
```

The Codex `PreToolUse` hook supplies relevant contract metadata as non-blocking context. The Stop
hook blocks completion once when a relevant required test is missing or failed, records
`contract_blocked` readiness, and avoids an automatic stop loop. Hooks improve the Codex workflow,
but the generated GitHub Actions check is the hard gate: it computes the base/HEAD merge base,
deduplicates identical argv tests, executes them directly, and fails closed on invalid contracts,
missing executables, timeouts, or nonzero exits.

Automatic candidates are evidence-only: the same normalized test command must fail, code must
change, and the command must then pass; alternatively, a test-file change and a code change may be
followed by a passing test. Ordinary pass-only tasks do not create candidates. Manual candidates
cover service invariants. Candidates and evaluations stay in the local canonical event log and
SQLite projection until approval writes the Git JSON; no raw prompt, tool output, or source code is
written to a contract.

## Local data and privacy

Data lives under `~/.previously-on` by default. Set `PREVIOUSLY_ON_DATA_DIR` to use a different
location only under a trusted parent and when the directory is owned by the current user and not
writable by group or others. PreviouslyOn tightens safe read/execute-only excess permissions to
`0700`, but fails closed on symlinked, foreign-owned, group/world-writable, or unsafe data files
before opening them. Full transcripts are disabled by default; evidence excerpts are redacted
and limited to 500 characters. Run `previously purge --repo <path>` to delete a repository's
canonical events, projections, queues, cached packs, WAL, and related recovery data.

The unpublished `~/.lineage` development directory is never migrated or deleted. `previously
doctor` reports it as ignored.

See the [privacy model](docs/privacy.md), [architecture](docs/architecture.md), and
[compatibility matrix](docs/compatibility.md).

## Development

```bash
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all
npm --prefix ui ci
npm --prefix ui run typecheck
npm --prefix ui run lint
npm --prefix ui test -- --run
npm --prefix ui run build
npm --prefix ui audit --audit-level=high
```

`./scripts/build-release.sh` runs the local quality, adversarial, packaging, offline-smoke, and
reproducibility gates. It writes Apple Silicon and source archives, an SBOM, license inventory,
NOTICE, and checksums to `outputs/`; CI retains the same files as PR artifacts. It does **not** run
or certify the separate live Codex compatibility matrix. `0.1.0-alpha.3` remains a source preview:
tagging, GitHub Release creation, and crates.io publication are blocked. Any later public release
also requires a SHA-pinned, separately supplied live evidence artifact to pass
`scripts/validate-live-compatibility.mjs` in the protected `release-compatibility` environment.

## Security and license

Report vulnerabilities through GitHub's private vulnerability reporting flow; see
[SECURITY.md](SECURITY.md). PreviouslyOn is licensed under Apache-2.0. Third-party notices are in
[NOTICE](NOTICE) and [THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md).
