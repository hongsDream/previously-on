# PreviouslyOn

PreviouslyOn is a local-first, verifiable handoff layer for Codex. It connects captured session
events to Git state and test evidence, then returns a deterministic Context Pack only after the
user approves a resume suggestion.

Version `0.1.0-alpha.1` is an early public alpha. Historical evidence is always untrusted data,
not an instruction and not a replacement for checking the current source.

## What the alpha includes

- one Rust `previously` binary for collection, storage, Git correlation, MCP, and the review UI;
- an embedded React evidence inspector;
- deterministic context packs with provenance, freshness, and capture-coverage warnings;
- no API key, telemetry, or outbound network access in the default mode;
- repository-scoped JSON export and complete local purge.

PreviouslyOn deliberately does not include a code graph, chat replay, cloud sync, multi-agent
integration, or automatic Context Pack injection.

## Install

### Apple Silicon release archive

Download `previously-on-v0.1.0-alpha.1-macos-arm64.tar.gz` and `SHA256SUMS` from the
[`v0.1.0-alpha.1` release](https://github.com/hongsDream/previously-on/releases/tag/v0.1.0-alpha.1),
then verify and install the unsigned alpha binary:

```bash
grep ' previously-on-v0.1.0-alpha.1-macos-arm64.tar.gz$' SHA256SUMS | shasum -a 256 -c -
tar -xzf previously-on-v0.1.0-alpha.1-macos-arm64.tar.gz
install -m 0755 previously-on-v0.1.0-alpha.1-macos-arm64/previously ~/.local/bin/previously
previously --version
```

The Apple Silicon artifact is not signed or notarized. Review the checksum and release
attestation before allowing it through macOS security controls.

### crates.io or source

Intel Mac users should install from crates.io or source; no Intel binary archive is published.

```bash
cargo install previously-on --version 0.1.0-alpha.1 --locked
```

Building the repository requires Rust 1.90+ and Node.js 22+:

```bash
npm --prefix ui ci
npm --prefix ui run build
cargo install --path . --locked
```

## Quick start

```bash
previously setup codex --repo /absolute/path/to/your/repository
previously run codex --repo /absolute/path/to/your/repository -- <codex arguments>
previously status
previously doctor
previously ui
```

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
6. The localhost inspector shows why each item was selected, its evidence, and freshness. Facts
   can be confirmed, pinned, invalidated, or superseded.

Transparent capture from an independently launched Codex process is experimental in this alpha.
The mapped 30-row regression driver and live App Server schema probe do not prove 30 real Codex
workflows or stable Hook/App Server ID linkage, so releases must not advertise that path as
supported yet.

The MCP server is read-only and exposes `suggest_resume`, `resume_task`, `search_tasks`,
`explain_fact`, and `get_task_timeline`.

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
