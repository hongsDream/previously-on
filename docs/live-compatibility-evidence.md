# Live compatibility evidence contract

This file documents the separately produced artifact accepted by
`scripts/validate-live-compatibility.mjs`. The mapped repository driver cannot generate this
contract. Do not put raw prompts, tool output, source code, credentials, or repository paths in
the artifact; use SHA-256 references to independently retained, sanitized run evidence.

Required top-level fields:

```json
{
  "schemaVersion": 1,
  "evidenceClass": "live_codex_workflow_matrix",
  "product": "PreviouslyOn",
  "productVersion": "0.1.0-alpha.1",
  "gitCommit": "40-character-tagged-commit-sha",
  "supportMode": "explicit_run_and_import",
  "generatedAt": "ISO-8601 timestamp",
  "runner": { "os": "macOS", "arch": "arm64" },
  "releaseEligibility": {
    "eligible": true,
    "dataLossEvents": 0,
    "seriousStaleApplications": {
      "status": "measured",
      "count": 0,
      "scenariosEvaluated": 60,
      "evaluatedAt": "ISO-8601 timestamp",
      "evidenceSha256": "64-character-digest",
      "evidencePath": "live-compatibility.json.evidence/evaluations/serious-stale-applications.json"
    }
  }
}
```

`seriousStaleApplications` is not inferred from the workflow harness. Without a separately
reviewed evaluator artifact it is recorded as `{ "status": "unmeasured" }`, and automatic
release eligibility remains false. The retained evaluator artifact must bind the product version
and Git commit, state how many scenarios were evaluated, record an ISO timestamp, and report the
observed serious-stale count.

`codexCli.runs` contains exactly two entries with roles `latest` and `previous`. Their stable
semantic versions must equal the two versions resolved from npm when the tag runs. Each entry
contains exactly the 30 IDs and categories in `fixtures/compatibility/scenarios.json`:

```json
{
  "role": "latest",
  "version": "0.0.0",
  "scenarios": [
    {
      "id": "fixture-scenario-id",
      "category": "fixture-category",
      "status": "passed",
      "reconstruction": {
        "userPrompt": true,
        "assistantFinal": true,
        "fileChangeTool": true,
        "testCommand": true,
        "stableSourceIds": true
      },
      "evidenceSha256": "64-character-sanitized-evidence-digest"
    }
  ]
}
```

The repository producer is `scripts/compatibility/live-harness.mjs`. It requires a clean Apple
Silicon checkout, absolute paths to both Codex binaries and the PreviouslyOn binary, an explicitly
confirmed authenticated `CODEX_HOME`, and the confirmation phrase documented in
`fixtures/compatibility/live-workflow-contract.json`. It copies only `auth.json` to isolated
temporary homes and never modifies the supplied source home.

It also requires `--mapped-artifact` from `scripts/run-compatibility.sh`, produced at the same
clean commit for the same two CLI versions. The mapped artifact remains a distinct evidence class:
it proves each scenario's named Rust target/filter and expectation, while the authenticated
workflow proves prompt, final, paired file tool, test command, resume, and stable source linkage.
The producer refuses release eligibility unless both parts pass; it does not mislabel the repeated
minimal repository edit as proof that every internal fault mode occurred in the paid Codex turn.

The producer writes `live-compatibility.json`, a sibling
`live-compatibility.json.evidence/` directory, and a `.tar.gz` bundle containing both. It adds
binary, fixture-contract, and scenario-matrix SHA-256 values plus
`liveCodexWorkflowMatrix.status: "complete"` and
`transparentCaptureReleaseGate.eligible: true`. Each scenario includes its mapped test contract
and must include a unique `evidencePath` for its retained structured verdict. Raw
initial/resume Codex JSONL is processed in memory and is not written to that bundle; it retains
only verdicts, stable identifiers, counts, and hashes needed to audit the result. The mapped
regression JSON is copied into the evidence directory and bound by its own SHA-256. The validator
opens every referenced file, rejects reused or escaping paths, verifies its digest and identity,
and rejects raw prompt/tool/source/credential/path fields.

## Resuming an interrupted producer run

The producer checkpoints the JSON artifact and retained scenario verdict after each completed
workflow. To continue an interrupted 60-workflow run, repeat the original live command with
`--resume` and the same `--output` path. Resume never trusts the stored completed-run count: it
revalidates every passed row against its referenced retained evidence file and SHA-256 digest,
recomputes the checkpoint set, and reruns failed or incomplete scenarios. Normal live and resume
modes reject stale-evaluator arguments so the retained matrix stays `unmeasured` until all 60
workflows are available for review.

Resume is permitted only for the same evidence identity. The clean Git commit, product version,
PreviouslyOn binary digest, both Codex binary digests and versions, fixture-contract digest,
scenario-matrix digest, mapped-artifact digest, runner identity, and supplied Codex App evidence
must remain bound to the original artifact. A missing or modified evidence file, changed binding,
or incompatible artifact fails closed. Start a new output/evidence path and generate a new
artifact instead of reusing checkpoints across that boundary.

## Finalizing the stale-application evaluation

The serious-stale evaluator is created after the 60 retained workflows have been reviewed. Attach
it with `--finalize-stale-evaluation` and the same binary, mapped-artifact, and Codex App evidence
arguments used for the original run. This mode does not require `CODEX_HOME`, a model, or the paid
execution confirmation because it never starts an authenticated workflow subprocess. Version and
command-shape probes still run against the supplied binaries. Finalize revalidates all 60
scenario evidence files and digests plus the commit, product, binaries, resolved CLI versions,
fixtures, mapped artifact, runner, and App evidence before retaining the evaluator.

Only `unmeasured -> measured` is permitted. The input must already be a complete, lossless 60-run
matrix, and the evaluator must bind the same commit and product version and report exactly 60
evaluated scenarios. Measured evidence cannot be replaced; incomplete, modified, rebound, or
data-loss-bearing evidence fails closed. See `docs/compatibility.md` for the exact command.

`codexApp.current` must reference a retained sanitized `codex_app_verification` JSON file with a
non-empty build, ISO timestamp, status, path, and matching SHA-256. A `degraded` current result is
recorded truthfully but is not release-eligible. `codexApp.previous` has the same retained-file
shape when obtainable. If the prior build is genuinely not obtainable, it may instead contain
`status: "unavailable"`, a non-empty `reason`, and `checkedAt`. Resume reopens every passed or
degraded App evidence file and rejects missing, symlinked, modified, or identity-mismatched bytes.

The protected release environment pins the entire tarball with
`LIVE_COMPATIBILITY_BUNDLE_SHA256` and downloads it only from
`LIVE_COMPATIBILITY_BUNDLE_URL`. Passing individual evidence hashes therefore cannot substitute
files into a different matrix without changing the reviewed bundle digest. The workflow rejects
absolute, parent-traversing, symlink, and other non-regular archive entries before extraction.
