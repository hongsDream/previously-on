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
    "seriousStaleApplications": 0
  }
}
```

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

`codexApp.current` must contain `status: "passed"`, a non-empty `build`, and an evidence digest.
`codexApp.previous` has the same shape when obtainable. If the prior build is genuinely not
obtainable, it may instead contain `status: "unavailable"`, a non-empty `reason`, and `checkedAt`.

The protected release environment pins the entire tarball with
`LIVE_COMPATIBILITY_BUNDLE_SHA256` and downloads it only from
`LIVE_COMPATIBILITY_BUNDLE_URL`. Passing individual evidence hashes therefore cannot substitute
files into a different matrix without changing the reviewed bundle digest. The workflow rejects
absolute, parent-traversing, symlink, and other non-regular archive entries before extraction.
