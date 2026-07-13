# Privacy and data handling

PreviouslyOn is local-first. The default mode does not make network requests and does not
require an API key.

## Before persistence

Every hook payload, App Server item, Git path, command, and fallback record passes through the
same redaction and size-limit pipeline before a durable write. The default filters cover:

- bearer and authorization headers;
- common API key and token prefixes;
- password, secret, token, and key assignments;
- `.env` files and common credential/key basenames;
- oversized prompts, tool input, tool output, and evidence excerpts.

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

The 90-day compaction runs when the daemon or review UI starts. Setup also keeps `0600` backups
of the pre-install `hooks.json` and `config.toml` under `~/.previously-on/setup-backups` so
uninstall can preserve later user edits. Those backups can contain the same sensitive values as
the original Codex configuration and are deleted only when the user removes the local
PreviouslyOn data directory.

## Deferred AI extraction

AI fact extraction is not included in `v0.1.0-alpha.1`. Facts can enter a context pack only after
they have verified evidence and the user confirms or pins them. A later experimental extractor
must preserve that gate; model output will never count as evidence.

## Prompt injection

Stored prompts and tool output can contain malicious instructions. PreviouslyOn wraps them
as historical evidence, labels them untrusted, and never maps their text to developer or system
instructions. MCP tools return data; they do not execute commands from history.

## Legacy development data

The unpublished development directory `~/.lineage` is not migrated or deleted. `previously
doctor` reports it as ignored so users can remove it manually after reviewing its contents.
