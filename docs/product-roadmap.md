# Product roadmap

## Product north star

PreviouslyOn is not a transcript archive. It is a verifiable memory layer that answers two product
questions:

1. What codebase, sessions, files, decisions, constraints, tests, and unfinished work belong to
   this task?
2. When the current Codex task becomes an unreliable place to continue, how can the same work move
   to a fresh task without losing state or duplicating execution?

The review UI and Context Pack exist to support that continuation. Automatic fresh-task
continuation is a core product capability, not a benchmark-only feature.

## Included in `0.1.0-alpha.2`

- **Project overview:** active tasks, recent Codex source task IDs, verified decisions, unresolved
  items, and touched code areas.
- **Codebase Lineage:** the registered repository and concrete worktree, branch, baseline/current
  commits, captured source task IDs, code areas, checkpoints, and test state.
- **Memory controls:** edit a fact, explain why it was selected, deprecate it after a Git commit,
  or exclude/re-include an entire source session from future Context Packs.
- **Verified Context Packs:** deterministic ordering, fixed budgets, evidence lineage, Git
  freshness, current validation, coverage warnings, and relevant Regression Contracts.
- **Automatic continuation:** at the provisional boundary, create a persisted Codex task through
  the official App Server, start the current request with a verified pack, and block the source
  prompt only after success. Idempotent operation records prevent blind duplicate creation.
- **Failure visibility:** pending, recovered, started, and failed rollover state plus the new Codex
  task ID are visible in the task workspace.

## Provisional alpha policy

`0.1.0-alpha.2` uses **seven observed compactions OR 80% observed context-window usage**. The
existing 72-hour inactivity plus relevant-code-change trigger remains an independent stale-context
safety rule.

This is a product pilot default, not evidence that every model degrades at that point. Unknown
token usage is never estimated. The policy is deliberately centralized as versioned constants so
it can be replaced without searching UI copy or projection code.

## Pilot before benchmark

The alpha pilot should first verify product behavior that a model benchmark cannot answer:

- users can find the new Codex task and understand why it was created;
- no source prompt is lost, duplicated, or blocked after a failed rollover;
- task/codebase lineage and the included/excluded memory are understandable;
- the verified pack contains the right goal, decisions, files, test state, next work, and relevant
  Regression Contracts;
- latency at the boundary is acceptable with actual local Codex App Server versions;
- redaction and recovery remain intact across app restart or network interruption.

## Final continuation benchmark

After the product flow is stable, run the versioned benchmark under `benchmarks/continuation` for
`gpt-5.5` and `gpt-5.6-sol`, reasoning high and fast mode off. The base matrix remains 864 measured
arms: 2 models × 8 fixed scenarios × 2 strategies × 9 compaction checkpoints × 3 repetitions.
Calibration is excluded. The large set of manually created Codex tasks from the abandoned first
attempt is not measured evidence and must not be counted or repeated.

The current append-only campaign contains 6 of 864 base arms, so 858 remain. Its derived output is
correctly `no_auto_rollover`; that incomplete result is not evidence for the seven/80 pilot policy
and is not silently converted into a model threshold.

The verified Context Pack product arm is added only after a degradation boundary is detected.
Recommendations remain model/version-specific. The provisional seven/80 policy changes only if
the predeclared bootstrap confidence-interval and product-arm gates pass; otherwise the release
records `no_auto_rollover_recommendation` and keeps the pilot policy explicitly provisional.

## Next product work

1. **Task discovery quality:** merge/split controls for incorrectly grouped sessions and clearer
   task naming suggestions.
2. **Richer code map:** symbol and dependency relationships once they can be derived
   deterministically without pretending that path co-occurrence proves a dependency.
3. **Continuation navigation:** focus/open the new Codex task if a documented desktop interface is
   introduced; do not depend on private deep links.
4. **AI-assisted fact refresh:** candidate-only output after an input-only or deny-read execution
   boundary passes prompt-injection fixtures.
5. **Team and multi-agent views:** shared provenance and access controls before any cloud sync or
   cross-agent automation.

Cloud services, full chat replay, automatic dependency inference, and private Codex APIs remain
outside the alpha.
