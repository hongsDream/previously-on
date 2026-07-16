# PreviouslyOn

PreviouslyOn is a local-first, verifiable project-memory and continuation layer for Codex. It
connects captured sessions to the codebase, Git state, test evidence, decisions, and open work. At
a continuation boundary it can start the current request in a fresh Codex task with a verified,
bounded Context Pack.

Version `0.1.0-alpha.2` is an early public alpha. Historical evidence is always untrusted data,
not an instruction and not a replacement for checking the current source.

## What the alpha includes

- one Rust `previously` binary for collection, storage, Git correlation, MCP, and the review UI;
- an embedded React project overview and evidence inspector;
- a task timeline with source-thread identity, relative activity age, turn and compaction counts,
  and observed context usage;
- deterministic context packs with provenance, freshness, and capture-coverage warnings;
- Git-shared regression contracts that connect changed paths and literal symbols to required tests;
- automatic fresh-task continuation after a session crosses the provisional boundary, with
  idempotent recovery and source-prompt blocking only after the new turn starts;
- no PreviouslyOn API key, telemetry, cloud service, or independent outbound integration; model
  execution is delegated to the user's configured local Codex App Server;
- repository-scoped JSON export and complete local purge.

PreviouslyOn deliberately does not include a dependency graph, chat replay, cloud sync,
multi-agent orchestration, or UI-triggered AI fact refresh.

## Install

### Apple Silicon release archive

Download `previously-on-v0.1.0-alpha.2-macos-arm64.tar.gz` and `SHA256SUMS` from the
[`v0.1.0-alpha.2` release](https://github.com/hongsDream/previously-on/releases/tag/v0.1.0-alpha.2),
then verify and install the unsigned alpha binary:

```bash
grep ' previously-on-v0.1.0-alpha.2-macos-arm64.tar.gz$' SHA256SUMS | shasum -a 256 -c -
tar -xzf previously-on-v0.1.0-alpha.2-macos-arm64.tar.gz
install -m 0755 previously-on-v0.1.0-alpha.2-macos-arm64/previously ~/.local/bin/previously
previously --version
```

The Apple Silicon artifact is not signed or notarized. Review the checksum and release
attestation before allowing it through macOS security controls.

### crates.io or source

Intel Mac users should install from crates.io or source; no Intel binary archive is published.

```bash
cargo install previously-on --version 0.1.0-alpha.2 --locked
```

Building the repository requires Rust 1.90+ and Node.js 22+:

```bash
npm --prefix ui ci
npm --prefix ui run build
cargo install --path . --locked
```

Regression Contracts are included in `0.1.0-alpha.2`. `previously contracts init
--github-actions` pins the same PreviouslyOn package version that generated the workflow and
installs it outside the consumer repository, so the gate never assumes that repository is a Rust
project.

## Quick start

```bash
previously setup codex --repo /absolute/path/to/your/repository
previously run codex --repo /absolute/path/to/your/repository -- <codex arguments>
previously status
previously doctor
previously ui
previously contracts init --github-actions
previously contracts validate
previously contracts check --base origin/main --execute --json
```

The review UI is deterministic and local. v0.1 does not invoke an AI model from the UI; facts are
activated only through explicit user review.

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
6. The project overview shows active tasks, recent source task IDs, decisions, open items, and code
   areas. The task inspector shows detailed Codebase Lineage, checkpoints, and why facts were
   selected. Facts can be edited, deprecated after a Git commit, confirmed, pinned, invalidated,
   or superseded; a captured session can be excluded from future packs.
7. When a session reaches seven observed compactions or at least 80% observed context usage, the
   next user prompt triggers automatic continuation. PreviouslyOn revalidates Git and Regression
   Contracts, creates a persisted task with the official Codex App Server, names it, starts the
   current request with a verified Context Pack, and only then blocks the source prompt to prevent
   duplicate work. If any step fails, the source prompt continues normally. A session inactive for
   at least 72 hours also uses this flow only when relevant code changed.

The seven-compaction/80% rule is an explicit provisional alpha policy, not a benchmark-derived or
model-general threshold. It will be replaced only after the continuation benchmark described in
[the product roadmap](docs/product-roadmap.md) produces a model-specific recommendation. The App
Server creates a new persisted task that appears in Codex; the current public interface does not
let PreviouslyOn force the desktop UI to focus that task.

Context usage is recorded only when the App Server actually emits a token-usage notification.
PreviouslyOn does not infer a percentage from prompt size or other incomplete observations.

Transparent capture from an independently launched Codex process is experimental in this alpha.
The mapped 30-row regression driver and live App Server schema probe do not prove 30 real Codex
workflows or stable Hook/App Server ID linkage, so releases must not advertise that path as
supported yet.

The MCP server is read-only and exposes `suggest_resume`, `resume_task`, `search_tasks`,
`explain_fact`, and `get_task_timeline`.

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
location. Full transcripts are disabled by default; evidence excerpts are redacted and limited
to 500 characters. Run `previously purge --repo <path>` to delete a repository's canonical
events, projections, queues, cached packs, WAL, and related recovery data.

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
reproducibility gates. It writes Apple Silicon artifacts, an SBOM, license inventory, NOTICE, and
checksums to `outputs/`. It does **not** run or certify the separate live Codex compatibility
matrix. A tag release remains blocked until a SHA-pinned, separately supplied live evidence
artifact passes `scripts/validate-live-compatibility.mjs` in the protected
`release-compatibility` environment.

## Security and license

Report vulnerabilities through GitHub's private vulnerability reporting flow; see
[SECURITY.md](SECURITY.md). PreviouslyOn is licensed under Apache-2.0. Third-party notices are in
[NOTICE](NOTICE) and [THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md).
