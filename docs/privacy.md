# Privacy and data handling

PreviouslyOn is local-first and requires no PreviouslyOn API key. It has no telemetry, cloud
storage, or independent network integration. Automatic continuation asks the user's already
configured local Codex App Server to start a model turn; Codex's own provider connection and
credentials remain under Codex configuration.

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

At an automatic continuation boundary, the current prompt is redacted, capped at 12,000
characters, and passed to a short-lived local worker over stdin. The full transient value is not
written to canonical events, SQLite projections, fallback queues, or the UI. The ordinary stored
UserPrompt event remains redacted and capped at 500 characters.

Redaction is defense in depth, not a guarantee that arbitrary secrets can never appear. Review
the local inspector before sharing an export.

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

## AI fact refresh is not included in v0.1

The review UI does not invoke Codex or another model. This avoids giving untrusted historical
evidence to an agent that can read repository or credential files. An AI-assisted candidate path
is deferred until a deny-read boundary and adversarial prompt-injection tests are available; model
output will not count as evidence when that path is introduced.

## Prompt injection

Stored prompts and tool output can contain malicious instructions. PreviouslyOn wraps them
as historical evidence, labels them untrusted, and never maps their text to developer or system
instructions. MCP tools return data; they do not execute commands from history.

Ordinary resume suggestions contain metadata only, and an approved manual resume still uses the
read-only `resume_task` MCP tool. Automatic fresh-task continuation is the narrow exception: only
after the deterministic boundary is reached, PreviouslyOn generates the same verified pack,
places it in an explicitly data-only untrusted block after the current request, and starts a fresh
Codex turn. Captured text inside the block is never promoted to system or developer instructions.

## Legacy development data

The unpublished development directory `~/.lineage` is not migrated or deleted. `previously
doctor` reports it as ignored so users can remove it manually after reviewing its contents.
