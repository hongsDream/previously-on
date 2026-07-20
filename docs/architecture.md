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
                                      grouping / AI candidates / local agents
                                      contract candidates / evaluations / FTS
                                                  │
                               ┌──────────────────┴──────────────────┐
                               ▼                                     ▼
                  MCP: 5 reads + approved continue_task      loopback review UI
```

The database runs in WAL mode. Canonical events are insert-only during ingestion. Retention
and repository purge use maintenance compaction: surviving rows are copied to a new database,
validated, fsynced, and atomically swapped so deleted evidence cannot return during rebuild.

Approved Regression Contracts are intentionally outside SQLite's source-of-truth boundary. They
live as one file per contract under `.previously-on/contracts/`, become active from the current
working tree immediately, and are shared by ordinary Git workflows. Local candidate and readiness
projections remain canonical-event-backed so rebuild, export, retention, and repository purge are
deterministic. Purging local repository data never deletes Git-owned contract files.

## Append-only task grouping

Task title, goal, and status edits are explicit canonical events. Session move, merge, split, and
undo use one `TaskGroupingChanged` event per operation, including session moves, before/after task
lifecycle snapshots, a stable operation ID, and an inverse link. Preview validates repository and
current associations without mutation; apply rejects missing or duplicate sessions, stale state,
invalid targets, and cross-repository changes. Replaying the canonical event atomically moves the
session-owned checkpoint, evidence, file, and test projections.

A fact moves only when every supporting provenance item belongs to moved sessions. Evidence that
spans moved and unmoved sessions stays on the original task, is marked mixed provenance, and is
never duplicated or guessed. A merge may complete an emptied source task while preserving its
previous lifecycle for undo; a split creates a new active task. Undo appends an inverse canonical
event instead of deleting history. Request and replay IDs make all paths idempotent.

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
The relationship graph is a deterministic view over canonical events, projections, and approved
contracts. It carries provenance for task/session, observed commit, changed file, contract,
literal symbol, required test, and confirmed agent-parent edges. It does not store a second truth
or infer dependencies from path co-occurrence, imports, or name similarity. Edge identity uses the
serialized relationship kind, endpoints, and source kind rather than observation time. Repeated
observations merge sorted provenance and retain the latest observation time. Edges without
provenance or existing endpoint nodes are omitted. The V1 `verified` and `verifiedEdgeCount`
fields remain as deprecated compatibility mirrors only.

## Session timeline and consent-gated continuation

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
is checked before each user prompt. At a boundary, the hook supplies only deterministic routing
IDs and asks Codex to invoke `continue_task`. Setup pins that one write tool to
`approval_mode = "prompt"`, so the Codex client must collect fresh user consent before execution.
Decline or cancel keeps the original turn in the source task. After approval, the exact current
request is redacted, bounded, carried over local MCP and child-process stdin, and never written to
the canonical event log. The worker then:

1. validates the source event and repository identity, then writes a deterministic `pending`
   operation event before any external task can be created;
2. revalidates the exact source worktree, task-observed and current file changes, fact freshness,
   excluded sessions, fact deprecation commits, Contract relevance, the current content
   fingerprint, and previously recorded test evidence;
3. calls the documented Codex App Server `thread/start` method for a persisted task and durably
   records its task ID before doing anything else;
4. links the new App Server session to the existing PreviouslyOn task, sets a display name on a
   best-effort basis, and calls `turn/start` with the current request plus a bounded internal
   `ContinuationHandoffV1` containing the existing `ContextPackV1` and `ContractEvaluationV1`;
5. records `started`; the successful `PostToolUse` hook returns `continue: false` so the source
   turn stops before it can repeat the work.

The operation ID and all transition event IDs are deterministic. A repeated hook invocation reuses
the recorded result. If a task ID was durably recorded, recovery resumes that task; if an attempt
stopped before the ID was recorded, PreviouslyOn refuses to create a possible duplicate. Any App
Server or validation failure records `failed` and leaves the source request available in the
original turn. Invalid Contract JSON, repository/worktree mismatch, fingerprint failure, and an
oversized handoff fail before `thread/start`; a preflight error cannot remain only `pending`.
`contract_blocked` is carried into the new task so it can be resolved there. Required tests are
never auto-executed at this boundary: only a prior pass or failure for the same fingerprint is
reused, an older pass becomes `stale`, and all other evidence stays `missing`.

After `turn/start`, PreviouslyOn opens the documented `codex://threads/<thread-id>` desktop deep
link. If the operating-system opener is unavailable, continuation still succeeds and returns the
same link; the review UI keeps an **Open in Codex** recovery action next to the task ID.

## Attribution

`modified_by` is emitted only when a supported tool event and before/after Git snapshots prove
the causal interval. Dirty worktrees, external editors, concurrent Codex sessions, ambiguous
renames, and unobserved tool paths are recorded as `observed_changed_in`.

## Context packs

`ContextPackV1` uses stable ordering and a fixed `o200k_base` tokenizer. The default budget is
1,200 tokens and the hard limit is 2,000 tokens including the JSON/MCP envelope. Mandatory
source, coverage, and freshness fields are never truncated; lower-ranked whole items are
removed first.

The public `resume_task` result and `ContextPackV1` JSON remain unchanged. The richer Contract
evaluation wrapper exists only inside the consented automatic-continuation turn input.

Historical evidence is enclosed and labelled as untrusted data. A context pack is an index for
live verification, not authority over the current repository.

Before a pack is returned, relevant files are revalidated against the current Git state. The UI
separates the checkpoint baseline (Then), intervening file changes (Since), current validation
(Now), and items that need review. Unrelated repository changes do not make scoped evidence stale;
renames, deletions, divergence, and relevant edits are surfaced explicitly. Invalid, superseded,
stale, broken, or unsupported facts remain excluded by default.

Ordinary user-approved resume remains behind the read-only MCP call. The only write-backed pack
delivery is the boundary-triggered, separately approved fresh-task flow above; it uses the same
verified builder and labels the pack as data-only untrusted history.

## Opt-in AI fact refresh

AI fact refresh is a beta, explicit user action. Setup installs the managed named profile
`previously-input-only` only with `--enable-ai-refresh`. It denies `:root`, `:tmpdir`, and
`:slash_tmp`, permits `:minimal` read, disables network, and uses approval `never`. Existing
unowned profiles are never replaced. Uninstall removes the profile only when PreviouslyOn still
owns the unchanged entry.

Before enabling Refresh, the experimental App Server client calls `permissionProfile/list` and
verifies that the named profile is allowed. `thread/start` receives named `permissions` without a
legacy `sandbox` field. The ephemeral thread starts in a fresh empty `0700` directory and receives
only a bounded, redacted verified pack: goal, current facts, open items, file paths/status, tests,
and contracts. Repository cwd, source contents, raw prompts, tool output, credentials, and network
access are excluded. `turn/start` uses a strict output schema for add, update, or deprecate
candidates, inherits the configured default model, and requests medium reasoning.

The experimental App Server process receives a cleared environment rebuilt from a minimal
allowlist (`PATH`, `HOME`, `CODEX_HOME`, `TMPDIR`, locale, and terminal values). Execution uses one
initialized client for profile listing, allowed-state verification, `thread/start`, and
`turn/start`; the profile ownership hash is checked before and after verification. Operation and
candidate claims are transactional so concurrent identical requests reuse one result while
conflicting request content fails instead of starting another model call.

The durable operation state is `pending`, `thread_created`, `completed`, or `failed`; deterministic
IDs make retries and restart recovery idempotent. Malformed, oversized, timed-out, or
capability-unverified runs fail closed. Model output lives in a separate candidate projection and
never counts as Evidence. Only explicit user accept/edit creates a Fact Candidate. Model ID,
token, and latency fields remain unavailable unless the App Server exposes them.

## Same-device local agent lineage

When the experimental App Server supports it, import/list/read collects paginated interactive,
subAgent, subAgentReview, subAgentCompact, subAgentThreadSpawn, and subAgentOther thread metadata.
Only threads from the registered concrete worktree are projected. A parent/child edge is emitted only
from an explicit `parentThreadId`; missing parents remain unlinked/degraded and fork/name/path
similarity is not used as a substitute. Both list and read results must agree on thread ID and
concrete worktree, and unsafe, absolute, traversal, or sensitive file paths are discarded.
Bounded redacted summaries, observed files, and tests are read-only local observations. There is
no cloud sync, team account, orchestration, or write-back.

## Local surfaces

- Hook commands send bounded JSON over a permission-restricted Unix domain socket inside the
  `0700` data directory. The hook starts the daemon on demand and queues an already-redacted
  event if startup or delivery fails.
- Every runtime surface requires current-user ownership and private permissions before opening
  the data directory, SQLite database/sidecars, locks, recovery files, or fallback queues; it
  rejects symlinks and opens SQLite with no-follow semantics.
- The MCP server uses JSON-RPC over stdio and has no write tools.
- The review server binds only to loopback and requires a per-launch bearer/CSRF token for
  state-changing requests.
- The embedded UI is static and does not load remote fonts, scripts, analytics, or assets.
