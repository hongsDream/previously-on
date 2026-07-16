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
Git regression contracts ─────────────────────────┤
                                                  ▼
                                      SQLite deterministic projections
                                      tasks / sessions / checkpoints / facts
                                      contract candidates / evaluations / FTS
                                                  │
                               ┌──────────────────┴──────────────────┐
                               ▼                                     ▼
                        read-only MCP                         loopback review UI
```

The database runs in WAL mode. Canonical events are insert-only during ingestion. Retention
and repository purge use maintenance compaction: surviving rows are copied to a new database,
validated, fsynced, and atomically swapped so deleted evidence cannot return during rebuild.

Approved Regression Contracts are intentionally outside SQLite's source-of-truth boundary. They
live as one file per contract under `.previously-on/contracts/`, become active from the current
working tree immediately, and are shared by ordinary Git workflows. Local candidate and readiness
projections remain canonical-event-backed so rebuild, export, retention, and repository purge are
deterministic. Purging local repository data never deletes Git-owned contract files.

## Regression contract evaluation

The v1 impact engine compares the merge base with `HEAD` and includes dirty working-tree changes.
It matches case-sensitive exact or prefix paths, inspects old and new rename paths, and performs
literal identifier-token matching only in changed hunks. It does not infer dependencies or use a
model. When symbol inspection cannot be completed safely for a path-matched binary, unreadable, or
oversized diff, the result is conservatively relevant and carries a warning.

Test freshness binds a successful argv execution to the content fingerprint of the relevant files
at that time. A later related content change makes the test stale. CLI execution deduplicates the
tuple `(program, args, workingDirectory)`, enforces a 1–3600 second timeout (900 seconds by
default), and treats invalid schemas, conflicting active contracts, missing executables, timeouts,
and nonzero exits as failures.

The PreToolUse hook is advisory and never blocks editing. The Stop hook may issue one continuation
with the exact required argv when readiness is blocked; persisted evaluation state and Codex's
`stop_hook_active` flag prevent an automatic loop. GitHub Actions remains the enforcement boundary.
No dependency graph is part of v1.

## Session timeline and automatic continuation

The session projection records the source App Server thread ID when available, last observed
activity, turn count, compaction count, and observed context usage. The UI renders relative age
from the recorded timestamp and re-evaluates that display locally as time passes. It does not
store or replay a full transcript.

App Server token usage is projected only from an actually observed
`thread/tokenUsage/updated` notification. If no notification is observed, context usage remains
unknown; prompt length and other partial signals are not treated as substitutes.

Imported historical App Server events never receive the Git snapshot observed at import time.
Without a historical snapshot their coverage is degraded and they cannot create a deterministic
checkpoint baseline. For linked worktrees, later revalidation runs against the concrete worktree
root stored in the checkpoint while confirming that it still belongs to the registered logical
repository.

Continuation eligibility is deterministic and session-scoped:

- seven observed compactions make the session eligible;
- at least 80% observed context-window usage makes the session eligible;
- after 72 hours of inactivity, a relevant Git change makes the session eligible.

The seven/80 rule is a provisional alpha policy rather than a model-general threshold. Eligibility
is checked before each user prompt. At a boundary, the redacted current prompt is carried over a
bounded child-process stdin channel and is not written to the canonical event log. The worker then:

1. validates the source event and repository identity, then writes a deterministic `pending`
   operation event before any external task can be created;
2. revalidates current Git state, fact freshness, excluded sessions, fact deprecation commits, and
   relevant Regression Contracts into a bounded Context Pack;
3. calls the documented Codex App Server `thread/start` method for a persisted task and durably
   records its task ID before doing anything else;
4. links the new App Server session to the existing PreviouslyOn task, sets a display name on a
   best-effort basis, and calls `turn/start` with the current request plus a bounded verified pack;
5. records `started`, then returns a UserPromptSubmit `block` decision for only the source prompt.

The operation ID and all transition event IDs are deterministic. A repeated hook invocation reuses
the recorded result. If a task ID was durably recorded, recovery resumes that task; if an attempt
stopped before the ID was recorded, PreviouslyOn refuses to create a possible duplicate. Any App
Server or validation failure records `failed` and leaves the source request unblocked.

The public App Server interface creates a persisted task but does not expose a way for PreviouslyOn
to force the Codex desktop UI to focus it. The new ID and rollover state are shown in the review UI.

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

Before a pack is returned, relevant files are revalidated against the current Git state. The UI
separates the checkpoint baseline (Then), intervening file changes (Since), current validation
(Now), and items that need review. Unrelated repository changes do not make scoped evidence stale;
renames, deletions, divergence, and relevant edits are surfaced explicitly. Invalid, superseded,
stale, broken, or unsupported facts remain excluded by default.

Ordinary user-approved resume remains behind the read-only MCP call. The only automatic pack
delivery is the boundary-triggered fresh-task flow above; it uses the same verified builder and
labels the pack as data-only untrusted history.

## AI fact refresh is deferred

v0.1 does not invoke Codex from the review UI. A read-only tool sandbox still permits filesystem
reads, so it is not a sufficient boundary when untrusted historical evidence may contain prompt
injection. AI-assisted candidate generation is deferred to v0.1.1 until it can run with a verified
deny-read profile or an equivalently isolated input-only execution path. Model output will remain
candidate-only and will never count as evidence when that feature is introduced.

## Local surfaces

- Hook commands send bounded JSON over a permission-restricted Unix domain socket inside the
  `0700` data directory. The hook starts the daemon on demand and queues an already-redacted
  event if startup or delivery fails.
- The MCP server uses JSON-RPC over stdio and has no write tools.
- The review server binds only to loopback and requires a per-launch bearer/CSRF token for
  state-changing requests.
- The embedded UI is static and does not load remote fonts, scripts, analytics, or assets.
