# Architecture

## Trust model

PreviouslyOn distinguishes three independent states:

1. **Capture coverage** — whether the expected source events were observed.
2. **Fact lifecycle** — whether a semantic claim is a candidate, confirmed, pinned, invalid,
   or superseded.
3. **Code freshness** — whether the linked file evidence is fresh, likely fresh, stale, or
   broken at the current Git state.

Missing capture cannot be repaired by increasing model confidence. AI-derived facts are
untrusted projections and never become evidence themselves.

## Process model

The alpha's supported entry point is the explicit `previously run codex` wrapper. It provides a
bounded session boundary, preserves Codex's exit status, replays the redacted fallback queue, and
then attempts App Server reconciliation. `previously import codex` exposes reconciliation as a
separate public command. Independently launched transparent capture remains experimental until the
live compatibility gate proves stable Hook/App Server linkage.

```text
Codex hooks ──> redaction/caps ──> Unix socket ──> insert-only event log
                         └──────> redacted fallback queue
                                                  │
Codex App Server reconciliation ──────────────────┤
Git snapshots and diffs ──────────────────────────┤
                                                  ▼
                                      SQLite deterministic projections
                                      tasks / checkpoints / facts / FTS
                                                  │
                               ┌──────────────────┴──────────────────┐
                               ▼                                     ▼
                        read-only MCP                         loopback review UI
```

The database runs in WAL mode. Canonical events are insert-only during ingestion. Retention
and repository purge use maintenance compaction: surviving rows are copied to a new database,
validated, fsynced, and atomically swapped so deleted evidence cannot return during rebuild.

## Attribution

`modified_by` is emitted only when a supported tool event and before/after Git snapshots prove
the causal interval. Dirty worktrees, external editors, concurrent Codex sessions, ambiguous
renames, and unobserved tool paths are recorded as `observed_changed_in`.

## Context packs

`ContextPackV1` uses stable ordering and a fixed `o200k_base` tokenizer. The default budget is
1,200 tokens and the hard limit is 2,000 tokens including the JSON/MCP envelope. Mandatory
source, coverage, and freshness fields are never truncated; lower-ranked whole items are
removed first.

Historical evidence is enclosed and labelled as untrusted data. A context pack is an index for
live verification, not authority over the current repository.

## Local surfaces

- Hook commands send bounded JSON over a permission-restricted Unix domain socket inside the
  `0700` data directory. The hook starts the daemon on demand and queues an already-redacted
  event if startup or delivery fails.
- The MCP server uses JSON-RPC over stdio and has no write tools.
- The review server binds only to loopback and requires a per-launch bearer/CSRF token for
  state-changing requests.
- The embedded UI is static and does not load remote fonts, scripts, analytics, or assets.
