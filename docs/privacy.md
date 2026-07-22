# Privacy and data handling

PreviouslyOn is local-first and requires no PreviouslyOn API key. It has no telemetry, cloud
storage, or independent network integration. User-approved continuation asks the user's already
configured local Codex App Server to start a model turn; Codex's own provider connection and
credentials remain under Codex configuration. Beta AI fact refresh uses that same configured App
Server only after explicit setup opt-in and an explicit Refresh action; it is never automatic.

## Before persistence

Every hook payload, App Server item, Git path, command, and fallback record passes through the
same redaction and size-limit pipeline before a durable write. The default filters cover:

- bearer and authorization headers;
- common API key and token prefixes;
- password, secret, token, and key assignments;
- `.env` files and common credential/key basenames;
- oversized prompts, tool input, tool output, and evidence excerpts.

Regression Contract JSON has a deliberately narrow schema: redacted title and invariant, impact
selectors, argv test metadata, the source commit and timestamp, and a SHA-256 evidence digest. It
does not contain raw prompts, tool output, raw source code, environment values, or secrets.
Automatic candidate evidence is reduced to normalized structural metadata before hashing.

At a continuation boundary, the managed hook provides routing IDs but not the full prompt. After
the user approves the `continue_task` MCP confirmation, the exact current request is redacted,
capped at 12,000 characters, and passed to a short-lived local worker over stdin. The full
transient value is not written to PreviouslyOn canonical events, SQLite projections, fallback
queues, or the review UI. The ordinary stored UserPrompt event remains redacted and capped at 500
characters. Codex already holds the request in its source-task transcript.

Redaction is defense in depth, not a guarantee that arbitrary secrets can never appear. Review
the local inspector before sharing an export.

## Pilot diagnostics

`previously diagnostics --repo <path>` opens the existing database in SQLite read-only/query-only
mode and prints one `schemaVersion: 1` JSON object to stdout. It does not save a report, upload
data, enable telemetry, create a Codex task, or invoke a model. The only Codex process call is the
read-only `codex --version` check.

The report is built from an allowlist: normalized app/Codex version, OS/architecture, relative
setup-to-first-checkpoint seconds, and aggregate session, checkpoint, coverage, continuation, and
Contract/test-state counts. Repository names and paths, prompts, source text, file names, commands,
task/session/thread/event IDs, secrets, and absolute timestamps are never fields in the diagnostic
DTO. Review the JSON before choosing to share it manually.

## Retention

- Unpinned evidence: 90 days by default.
- Evidence required by a pinned fact: retained until the fact is unpinned, invalidated, or the
  repository is purged.
- Full transcripts: disabled by default.
- External backups: outside the deletion guarantee; users must manage their own backup copies.

`previously purge --repo <path>` removes the repository from the canonical event log, projections,
FTS indexes, fallback queues, cached packs, and database WAL through an atomic compaction.
Git-owned `.previously-on/contracts/*.json` files are not local projection data and are never
deleted by repository purge.

The 90-day compaction runs when the daemon or review UI starts. Setup also keeps `0600` backups
of the pre-install `hooks.json` and `config.toml` under `~/.previously-on/setup-backups` so
uninstall can preserve later user edits. Those backups can contain the same sensitive values as
the original Codex configuration and are deleted only when the user removes the local
PreviouslyOn data directory.

The first-run local UI can perform the normal Codex setup without a terminal command. The setup
endpoint is available only to the loopback UI session, requires a matching same-origin request and
an explicit confirmation flag, accepts only an absolute Git worktree path, and refuses to replace
an existing registration. It calls the same journaled setup implementation used by the CLI, so
existing Codex configuration is backed up and interrupted writes remain recoverable. The doctor
checks run after setup without creating a Codex task, starting a model turn, or uploading data.

## Local Codex Desktop import

Registered projects remain separate in the local manifest and UI. Choosing **Sync Codex app
history** explicitly starts a same-device App Server import for the selected project only. The UI
may start this action once when that project is opened during a browser session, and the user can
request it again with the button; PreviouslyOn does not run a cloud sync service or continuously
watch a Codex account. **All projects** is a read-only local summary and does not merge project
histories.

The import keeps only bounded, redacted, allowlisted semantic events and metadata. Raw transcripts
are not written to the canonical store. Stable `reasonCode` values drive user-facing status copy;
redacted App Server failures and compatibility warnings remain separate `technicalDetails` and are
shown only under **Technical details**. Nothing from this flow is uploaded by PreviouslyOn.

## Beta AI fact refresh

AI refresh is disabled by default. `previously setup codex --enable-ai-refresh` installs a managed
`previously-input-only` profile with root and temporary-directory access denied, minimal read
only, network disabled, and approval `never`. PreviouslyOn refuses to run unless the App Server's
experimental permission-profile API verifies the profile and effective requirements. It never
sends both legacy sandbox and named permissions.

Each user-triggered operation starts from an isolated empty `0700` directory. The input is limited
to a bounded, redacted verified pack containing goal, current facts, open items, file paths/status,
tests, and contracts. Repository cwd, source contents, raw prompts, raw tool output, and secret
environment variables are not forwarded. Output must match a strict add/update/deprecate schema;
malformed, oversized, timed-out, or interrupted runs fail closed. Model output is stored only as
an AI candidate. It is not Evidence and becomes a Fact Candidate only after explicit user accept
or edit. Model ID, token, and latency values remain `unavailable` when the App Server does not
expose them.

The experimental App Server child begins with an empty environment and receives only `PATH`,
`HOME`, `CODEX_HOME`, `TMPDIR`, locale, and terminal variables. Database URLs, auth values,
passwords, cookies, credentials, API keys, secrets, and tokens are neither inherited nor accepted
unredacted in the verified pack. Profile verification and execution use the same initialized App
Server client.

The setup ownership journal covers the managed profile. Uninstall removes it only when the entry
is still owned and unchanged; a user-modified or unowned profile is preserved.
All PreviouslyOn data and backup directories, including an alternate
`PREVIOUSLY_ON_DATA_DIR`, must be under a trusted parent, current-user-owned, and not writable by
group or others. Runtime safely tightens read/execute-only excess permissions to `0700` before
access. Databases, sidecars, queues, locks, the recovery journal, manifest, and backup files must
be regular, current-user-owned files no broader than `0600`. Setup, normal runtime, and uninstall
reject symlinks, foreign ownership, group/world-writable directories, unexpected journal targets,
and hash mismatches before mutating managed files.

## Local agent observation

Agent lineage is a read-only projection of same-device Codex App Server metadata. PreviouslyOn
stores bounded redacted output summaries plus observed file/test metadata, includes only threads
from the registered concrete worktree, and links parents only from an explicit `parentThreadId`.
The ID and concrete worktree returned by `thread/read` are checked again, and unsafe or sensitive
file paths are discarded. Missing parentage is shown as unlinked/degraded rather than guessed.
Nothing is synced to a cloud or team account, and PreviouslyOn does not orchestrate or write back
to an agent.

## Prompt injection

Stored prompts and tool output can contain malicious instructions. PreviouslyOn wraps them
as historical evidence, labels them untrusted, and never maps their text to developer or system
instructions. MCP tools return data; they do not execute commands from history.

Ordinary resume suggestions contain metadata only, and an approved manual resume still uses the
read-only `resume_task` MCP tool. Consent-gated fresh-task continuation is the narrow write
exception: only after the deterministic boundary and fresh Codex tool approval, PreviouslyOn
generates the same verified pack, places it in an explicitly data-only untrusted block after the
current request, and starts a fresh Codex turn. Captured text inside the block is never promoted to
system or developer instructions.

## Legacy development data

The unpublished development directory `~/.lineage` is not migrated or deleted. `previously
doctor` reports it as ignored so users can remove it manually after reviewing its contents.
